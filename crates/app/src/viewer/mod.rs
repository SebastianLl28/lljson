//! Viewer tab: edit and inspect a single JSON document.

mod chunks;
mod editor;
mod file;
mod prepare;
#[cfg(any(debug_assertions, feature = "devtools"))]
pub mod probe;
mod status;
mod validate;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use lljson_core::Mode;

use self::editor::Editor;
use self::status::StatusBar;
use self::validate::{DEBOUNCE_MS, Generation, Validation, spawn_autofix, spawn_validate};

pub struct ViewerPage {
    root: adw::ToastOverlay,
    editor: Editor,
    status: StatusBar,
    tolerant: gtk::ToggleButton,
    auto_fix: gtk::Button,
    /// Id of the latest file read; results from older reads are discarded.
    generation: Cell<u64>,
    /// Outcome of the latest finished validation; `None` until the first one.
    validation: RefCell<Option<Validation>>,
    /// Id of the latest validation request; results of older ones are dropped.
    validating: Generation,
    /// Pending debounce timer, cleared by the timer itself when it fires.
    debounce: RefCell<Option<glib::SourceId>>,
}

impl ViewerPage {
    pub fn new() -> Rc<ViewerPage> {
        let editor = Editor::new();
        let bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .css_classes(["toolbar"])
            .build();
        bar.append(
            &gtk::Button::builder()
                .icon_name("document-open-symbolic")
                .tooltip_text("Open file (Ctrl+O)")
                .action_name("win.viewer-open")
                .build(),
        );
        bar.append(
            &gtk::Button::builder()
                .icon_name("edit-copy-symbolic")
                .tooltip_text("Copy all the text (Ctrl+Shift+C)")
                .action_name("win.viewer-copy-all")
                .build(),
        );
        let tolerant = gtk::ToggleButton::builder()
            .label("Tolerant")
            .tooltip_text("Accept comments and trailing commas")
            .build();
        bar.append(&tolerant);
        let auto_fix = gtk::Button::builder()
            .label("Auto-fix")
            .tooltip_text("Remove comments and trailing commas")
            .sensitive(false)
            .build();
        bar.append(&auto_fix);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&bar);
        content.append(&editor.root);
        let status = StatusBar::new();
        content.append(&status.root);
        let root = adw::ToastOverlay::new();
        root.set_child(Some(&content));

        let this = Rc::new(ViewerPage {
            root,
            editor,
            status,
            tolerant,
            auto_fix,
            generation: Cell::new(0),
            validation: RefCell::new(None),
            validating: Generation::default(),
            debounce: RefCell::new(None),
        });
        this.setup_drop();
        this.setup_validation();
        this
    }

    pub fn root(&self) -> &gtk::Widget {
        self.root.upcast_ref()
    }

    /// Reads on a background thread and replaces the text; a newer request makes older
    /// results be dropped.
    pub fn open_path(self: &Rc<Self>, path: PathBuf) {
        self.show_when_ready(file::spawn_read(path));
    }

    fn show_when_ready(self: &Rc<Self>, receiver: async_channel::Receiver<file::ReadResult>) {
        let id = self.generation.get() + 1;
        self.generation.set(id);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = receiver.recv().await else {
                return;
            };
            let Some(this) = weak.upgrade() else {
                return;
            };
            if this.generation.get() != id {
                return;
            }
            match result {
                Ok(text) => {
                    this.replace_text(text);
                }
                Err(err) => this.toast(&err.to_string()),
            }
        });
    }

    /// Replaces the text; it is validated once all of it is in (inserting is done in slices).
    fn replace_text(self: &Rc<Self>, text: String) {
        let done = self.editor.set_text(text);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            if done.recv().await.is_ok()
                && let Some(this) = weak.upgrade()
            {
                this.schedule_validation();
            }
        });
    }

    fn mode(&self) -> Mode {
        if self.tolerant.is_active() {
            Mode::Tolerant
        } else {
            Mode::Strict
        }
    }

    /// Removes comments and trailing commas on a background thread and puts the result in the
    /// editor, unless the text was edited in the meantime.
    fn auto_fix(self: &Rc<Self>) {
        if !self.editor.view.is_editable() {
            return;
        }
        let id = self.validating.current();
        let receiver = spawn_autofix(self.editor.text());
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Ok(fixed) = receiver.recv().await else {
                return;
            };
            let Some(this) = weak.upgrade() else {
                return;
            };
            if !this.validating.is_current(id) {
                return;
            }
            match fixed {
                Some(text) => this.replace_text(text),
                None => this.toast("Nothing to fix"),
            }
        });
    }

    /// Edits restart the debounce timer; programmatic replacements (read-only editor) only
    /// cancel whatever is pending, since their own completion triggers the validation.
    fn setup_validation(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.tolerant.connect_toggled(move |_| {
            if let Some(this) = weak.upgrade() {
                this.schedule_validation();
            }
        });
        let weak = Rc::downgrade(self);
        self.auto_fix.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.auto_fix();
            }
        });
        let weak = Rc::downgrade(self);
        self.editor.view.buffer().connect_changed(move |_| {
            let Some(this) = weak.upgrade() else {
                return;
            };
            if this.editor.view.is_editable() {
                this.schedule_validation();
            } else {
                this.cancel_validation();
            }
        });
    }

    fn cancel_validation(&self) {
        // The mark would point at the wrong place once the text changes.
        self.editor.clear_error();
        self.auto_fix.set_sensitive(false);
        self.validating.advance();
        if let Some(timer) = self.debounce.take() {
            timer.remove();
        }
    }

    /// Parses the text on a background thread after the user stops typing.
    fn schedule_validation(self: &Rc<Self>) {
        self.cancel_validation();
        let id = self.validating.advance();
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(Duration::from_millis(DEBOUNCE_MS), move || {
            let Some(this) = weak.upgrade() else {
                return;
            };
            // The timer is gone once it fires; forget its id so it is not removed later.
            this.debounce.take();
            let receiver = spawn_validate(this.editor.text(), this.mode());
            let weak = Rc::downgrade(&this);
            glib::MainContext::default().spawn_local(async move {
                let Ok(result) = receiver.recv().await else {
                    return;
                };
                if let Some(this) = weak.upgrade()
                    && this.validating.is_current(id)
                {
                    this.status.show(&result);
                    this.auto_fix
                        .set_sensitive(matches!(result, Validation::Valid { fixes } if fixes > 0));
                    match &result {
                        Validation::Invalid { mark, .. } => this.editor.mark_error(mark),
                        _ => this.editor.clear_error(),
                    }
                    *this.validation.borrow_mut() = Some(result);
                }
            });
        });
        *self.debounce.borrow_mut() = Some(timer);
    }

    /// Latest validation outcome, for the probe.
    #[cfg(any(debug_assertions, feature = "devtools"))]
    fn validation(&self) -> Option<std::cell::Ref<'_, Validation>> {
        std::cell::Ref::filter_map(self.validation.borrow(), Option::as_ref).ok()
    }

    pub fn open_dialog(self: &Rc<Self>, window: &adw::ApplicationWindow) {
        let dialog = crate::window::json_file_dialog();
        let window = window.clone();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = dialog.open_future(Some(&window)).await;
            let Some(this) = weak.upgrade() else {
                return;
            };
            match result {
                Ok(file) => this.open_file(&file),
                Err(err) if err.matches(gtk::DialogError::Dismissed) => {}
                Err(err) => this.toast(&format!("Could not open the file: {err}")),
            }
        });
    }

    pub fn copy_all(&self) {
        self.root.clipboard().set_text(&self.editor.text());
        self.toast("Text copied");
    }

    fn open_file(self: &Rc<Self>, file: &gio::File) {
        match file.path() {
            Some(path) => self.open_path(path),
            None => self.toast("Only local files can be opened"),
        }
    }

    /// A dropped file is opened (only the first one); dropped text replaces the editor's.
    fn setup_drop(self: &Rc<Self>) {
        let target = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY);
        target.set_types(&[gdk::FileList::static_type(), String::static_type()]);
        // The text view has its own drop target that accepts the plain-text form of a dropped
        // file; capturing first makes files open instead of pasting their path.
        target.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            weak.upgrade().is_some_and(|this| this.drop_value(value))
        });
        self.root.add_controller(target);
    }

    fn drop_value(self: &Rc<Self>, value: &glib::Value) -> bool {
        if let Ok(list) = value.get::<gdk::FileList>() {
            let files = list.files();
            let Some(first) = files.first() else {
                return false;
            };
            if files.len() > 1 {
                self.toast("Only the first file is opened");
            }
            self.open_file(first);
            true
        } else if let Ok(text) = value.get::<String>() {
            self.show_when_ready(file::spawn_prepare(text));
            true
        } else {
            false
        }
    }

    fn toast(&self, message: &str) {
        self.root.add_toast(adw::Toast::new(message));
    }
}
