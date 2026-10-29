//! Viewer tab: edit and inspect a single JSON document.

mod editor;

use std::rc::Rc;

use adw::prelude::*;

use self::editor::Editor;

pub struct ViewerPage {
    root: gtk::Widget,
}

impl ViewerPage {
    pub fn new() -> Rc<ViewerPage> {
        let editor = Editor::new();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&editor.root);
        Rc::new(ViewerPage {
            root: root.upcast(),
        })
    }

    pub fn root(&self) -> &gtk::Widget {
        &self.root
    }
}
