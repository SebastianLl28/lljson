//! Plain text editor of the Viewer: a `GtkTextView` that accepts anything the user types or pastes.

use gtk::prelude::*;

pub struct Editor {
    pub root: gtk::ScrolledWindow,
    view: gtk::TextView,
}

impl Editor {
    pub fn new() -> Editor {
        let view = gtk::TextView::builder()
            .monospace(true)
            .wrap_mode(gtk::WrapMode::None)
            .accepts_tab(true)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(12)
            .right_margin(12)
            .build();
        let root = gtk::ScrolledWindow::builder()
            .child(&view)
            .hexpand(true)
            .vexpand(true)
            .build();
        Editor { root, view }
    }

    pub fn text(&self) -> String {
        let buffer = self.view.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), true)
            .into()
    }

    pub fn set_text(&self, text: &str) {
        self.view.buffer().set_text(text);
    }
}
