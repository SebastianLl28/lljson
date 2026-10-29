//! Plain text editor of the Viewer: a `GtkTextView` that accepts anything the user types or pastes.

pub struct Editor {
    pub root: gtk::ScrolledWindow,
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
        Editor { root }
    }
}
