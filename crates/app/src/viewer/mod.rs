//! Viewer tab: edit and inspect a single JSON document.

mod chunks;
mod editor;
mod file;
mod prepare;
#[cfg(any(debug_assertions, feature = "devtools"))]
pub mod probe;

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use self::editor::Editor;

pub struct ViewerPage {
    root: adw::ToastOverlay,
    editor: Editor,
    /// Id of the latest file read; results from older reads are discarded.
    generation: Cell<u64>,
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
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&bar);
        content.append(&editor.root);
        let root = adw::ToastOverlay::new();
        root.set_child(Some(&content));

        let this = Rc::new(ViewerPage {
            root,
            editor,
            generation: Cell::new(0),
        });
        this.setup_drop();
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
                    let _ = this.editor.set_text(text);
                }
                Err(err) => this.toast(&err.to_string()),
            }
        });
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
