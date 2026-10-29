//! Viewer tab: edit and inspect a single JSON document.

use std::rc::Rc;

use adw::prelude::*;

pub struct ViewerPage {
    root: gtk::Widget,
}

impl ViewerPage {
    pub fn new() -> Rc<ViewerPage> {
        let root = adw::StatusPage::builder()
            .icon_name("text-x-generic-symbolic")
            .title("Viewer")
            .build();
        Rc::new(ViewerPage {
            root: root.upcast(),
        })
    }

    pub fn root(&self) -> &gtk::Widget {
        &self.root
    }
}
