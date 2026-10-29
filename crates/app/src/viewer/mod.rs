//! Viewer tab: edit and inspect a single JSON document.

mod chunks;
mod editor;
mod file;
mod prepare;
#[cfg(any(debug_assertions, feature = "devtools"))]
pub mod probe;
mod reformat;
mod status;
mod tree;
mod validate;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use lljson_core::{Document, LineIndex, Mode, NodeId};

use crate::transform::Transform;

use self::editor::Editor;
use self::reformat::spawn_reformat;
use self::status::StatusBar;
use self::tree::TreeView;
use self::validate::{Checked, DEBOUNCE_MS, Generation, Validation, spawn_autofix, spawn_validate};

/// Whether a paste replaces the whole text: the buffer is empty or the selection spans it
/// (offsets in characters).
fn covers_all(total: i32, selection: Option<(i32, i32)>) -> bool {
    total == 0 || selection.is_some_and(|(start, end)| start <= 0 && end >= total)
}

pub struct ViewerPage {
    root: adw::ToastOverlay,
    editor: Editor,
    status: StatusBar,
    tree: TreeView,
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
    /// What the tree shows, to map between tree nodes and editor positions.
    sync: RefCell<Option<SyncState>>,
    /// The editor text is the one the tree was built from (no edit since).
    synced: Cell<bool>,
    /// A cursor → tree update is already queued.
    cursor_queued: Cell<bool>,
}

/// The parsed text behind the tree.
struct SyncState {
    document: Arc<Document>,
    lines: Arc<LineIndex>,
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
        for (label, tooltip, action) in [
            (
                "Format",
                "Format with 2 spaces (Ctrl+Shift+F)",
                "win.viewer-format",
            ),
            (
                "Minify",
                "Remove all whitespace (Ctrl+Shift+M)",
                "win.viewer-minify",
            ),
            (
                "Sort keys",
                "Sort the keys of every object (Ctrl+Shift+S)",
                "win.viewer-sort-keys",
            ),
        ] {
            bar.append(
                &gtk::Button::builder()
                    .label(label)
                    .tooltip_text(tooltip)
                    .action_name(action)
                    .build(),
            );
        }
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
        let tree = TreeView::new({
            let root = root.clone();
            Rc::new(move |message| root.add_toast(adw::Toast::new(message)))
        });
        // Editor on the left, tree on the right; neither pane may be squeezed out.
        let split = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&content)
            .end_child(&tree.root)
            .resize_start_child(true)
            .resize_end_child(true)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .build();
        root.set_child(Some(&split));

        let this = Rc::new(ViewerPage {
            root,
            editor,
            status,
            tree,
            tolerant,
            auto_fix,
            generation: Cell::new(0),
            validation: RefCell::new(None),
            validating: Generation::default(),
            debounce: RefCell::new(None),
            sync: RefCell::new(None),
            synced: Cell::new(false),
            cursor_queued: Cell::new(false),
        });
        this.setup_sync();
        this.setup_drop();
        this.setup_validation();
        this.setup_paste();
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

    /// Pasting over the whole content (empty editor or everything selected) formats valid JSON
    /// before it goes in. Any other paste, and pasted text that is not valid JSON, takes the
    /// normal path so that it stays undoable and lands where the cursor is.
    fn setup_paste(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let bypass = Rc::new(Cell::new(false));
        self.editor.view.connect_paste_clipboard(move |view| {
            if bypass.get() {
                return;
            }
            let buffer = view.buffer();
            let selection = buffer
                .selection_bounds()
                .map(|(start, end)| (start.offset(), end.offset()));
            if !covers_all(buffer.char_count(), selection) {
                return;
            }
            view.stop_signal_emission_by_name("paste-clipboard");
            let (view, weak, bypass) = (view.clone(), weak.clone(), Rc::clone(&bypass));
            glib::MainContext::default().spawn_local(async move {
                let text = match view.clipboard().read_text_future().await {
                    Ok(Some(text)) => Some(text.to_string()),
                    _ => None,
                };
                let formatted = match (text.clone(), weak.upgrade()) {
                    (Some(text), Some(this)) => this.format_pasted(text).await,
                    _ => None,
                };
                match (formatted, weak.upgrade()) {
                    (Some(formatted), Some(this)) => this.replace_text(formatted),
                    _ => {
                        // Nothing to format: let GTK paste as usual.
                        bypass.set(true);
                        view.emit_by_name::<()>("paste-clipboard", &[]);
                        bypass.set(false);
                    }
                }
            });
        });
    }

    /// The pretty-printed form of `text` if it is valid JSON that is not already formatted;
    /// `None` also when a newer load made this paste stale.
    async fn format_pasted(self: &Rc<Self>, text: String) -> Option<String> {
        let id = self.generation.get() + 1;
        self.generation.set(id);
        let receiver = file::spawn_prepare(text.clone());
        let formatted = receiver.recv().await.ok()?.ok()?;
        (self.generation.get() == id && formatted != text).then_some(formatted)
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

    /// Applies `transform` to the text on a background thread. Invalid text is reported in a
    /// toast and left alone; so is a result for text that was edited in the meantime.
    pub fn reformat(self: &Rc<Self>, transform: Transform) {
        if !self.editor.view.is_editable() {
            return;
        }
        let text = self.editor.text();
        if text.trim().is_empty() {
            self.toast("Nothing to format");
            return;
        }
        let id = self.validating.current();
        let receiver = spawn_reformat(text.clone(), transform, self.mode());
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Ok(result) = receiver.recv().await else {
                return;
            };
            let Some(this) = weak.upgrade() else {
                return;
            };
            if !this.validating.is_current(id) {
                return;
            }
            match result {
                Ok(new) if new == text => this.toast("Already in that form"),
                Ok(new) => {
                    this.replace_text(new);
                    this.toast(transform.done_message());
                }
                Err(err) => this.toast(&format!("Cannot reformat invalid JSON: {err}")),
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

    /// Keeps the editor selection and the tree selection pointing at the same node.
    fn setup_sync(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.tree.connect_select(move |node| {
            if let Some(this) = weak.upgrade() {
                this.select_in_editor(node);
            }
        });
        let weak = Rc::downgrade(self);
        self.editor.connect_cursor_moved(move || {
            if let Some(this) = weak.upgrade() {
                this.queue_cursor_sync();
            }
        });
    }

    /// Selects the text of `node`. Nothing if the text was edited since the tree was built:
    /// the positions would be wrong.
    fn select_in_editor(&self, node: NodeId) {
        if !self.synced.get() {
            return;
        }
        let sync = self.sync.borrow();
        let Some(sync) = sync.as_ref() else {
            return;
        };
        let Some(range) = sync.document.node_range(node) else {
            return;
        };
        self.editor.select_range(
            sync.lines.position(range.start),
            sync.lines.position(range.end),
        );
    }

    /// Selects in the tree the node under the cursor, once the current edit has settled.
    fn queue_cursor_sync(self: &Rc<Self>) {
        if self.cursor_queued.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            let Some(this) = weak.upgrade() else {
                return;
            };
            this.cursor_queued.set(false);
            this.select_under_cursor();
        });
    }

    fn select_under_cursor(&self) {
        if !self.synced.get() || !self.editor.view.is_editable() {
            return;
        }
        let node = {
            let sync = self.sync.borrow();
            let Some(sync) = sync.as_ref() else {
                return;
            };
            let (line, column) = self.editor.cursor();
            sync.lines
                .offset(line, column)
                .and_then(|offset| sync.document.node_at_offset(offset))
        };
        if let Some(node) = node
            && self.tree.selected() != Some(node)
        {
            self.tree.select_node(node);
        }
    }

    fn cancel_validation(&self) {
        self.synced.set(false);
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
                let Ok(checked) = receiver.recv().await else {
                    return;
                };
                if let Some(this) = weak.upgrade()
                    && this.validating.is_current(id)
                {
                    let Checked {
                        validation: result,
                        tree,
                    } = checked;
                    // Invalid text keeps the last valid tree while the user fixes it.
                    match tree {
                        Some(tree) => {
                            *this.sync.borrow_mut() = Some(SyncState {
                                document: Arc::clone(&tree.document),
                                lines: Arc::clone(&tree.lines),
                            });
                            this.synced.set(true);
                            this.tree.show(tree);
                            this.queue_cursor_sync();
                        }
                        None if result == Validation::Empty => {
                            *this.sync.borrow_mut() = None;
                            this.tree.clear();
                        }
                        None => {}
                    }
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

    /// Moves the editor cursor like a click would.
    #[cfg(any(debug_assertions, feature = "devtools"))]
    pub fn set_cursor_for_test(&self, line: i32, column: i32) {
        let buffer = self.editor.view.buffer();
        if let Some(iter) = buffer.iter_at_line_index(line, column) {
            buffer.place_cursor(&iter);
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools"))]
    pub fn select_row_for_test(&self, position: u32) {
        self.tree.select_position_for_test(position);
    }

    /// The theme changed: repaint the syntax colors.
    pub fn refresh_colors(&self) {
        self.editor.refresh_colors();
    }

    pub fn open_search(&self) {
        self.tree.open_search();
    }

    pub fn step_search(&self, forward: bool) {
        self.tree.step_search(forward);
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

#[cfg(test)]
mod tests {
    use super::covers_all;

    #[test]
    fn paste_replaces_everything_when_empty_or_all_selected() {
        assert!(covers_all(0, None));
        assert!(covers_all(10, Some((0, 10))));
    }

    #[test]
    fn partial_selection_or_cursor_is_a_normal_paste() {
        assert!(!covers_all(10, None));
        assert!(!covers_all(10, Some((1, 10))));
        assert!(!covers_all(10, Some((0, 9))));
    }
}
