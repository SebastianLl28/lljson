//! Plain text editor of the Viewer: a `GtkTextView` that accepts anything the user types or pastes.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};

use super::chunks::chunks;

/// Between redrawing (`PRIORITY_REDRAW`) and the text view's background line layout (+5): frames
/// are drawn between slices, but the slices do not wait behind the layout of the text so far.
const SLICE_PRIORITY: i32 = gdk::PRIORITY_REDRAW as i32 + 1;

/// Text inserted per idle callback: about 4 ms of layout work, well under one frame.
const CHUNK_BYTES: usize = 256 * 1024;

pub struct Editor {
    pub root: gtk::ScrolledWindow,
    pub(super) view: gtk::TextView,
    /// Id of the latest `set_text`; an insertion in progress stops when it is no longer current.
    load: Rc<Cell<u64>>,
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
        Editor {
            root,
            view,
            load: Rc::new(Cell::new(0)),
        }
    }

    pub fn text(&self) -> String {
        let buffer = self.view.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), true)
            .into()
    }

    /// Replaces the text incrementally from idle callbacks so that large texts do not freeze
    /// the window. The editor is read-only until the last slice is in, and the replacement is
    /// not recorded for undo. The receiver yields `()` when done; a newer call cancels this one
    /// (its receiver then just closes).
    pub fn set_text(&self, text: String) -> async_channel::Receiver<()> {
        let id = self.load.get() + 1;
        self.load.set(id);
        let (sender, receiver) = async_channel::bounded(1);
        let buffer = self.view.buffer();
        // Read-only first, so that listeners can tell the clearing from a user edit.
        self.view.set_editable(false);
        buffer.begin_irreversible_action();
        buffer.set_text("");
        let view = self.view.clone();
        let load = Rc::clone(&self.load);
        let mut offset = 0;
        glib::idle_add_local_full(glib::Priority::from(SLICE_PRIORITY), move || {
            if load.get() != id {
                buffer.end_irreversible_action();
                return glib::ControlFlow::Break;
            }
            let Some(chunk) = chunks(&text[offset..], CHUNK_BYTES).next() else {
                buffer.end_irreversible_action();
                buffer.place_cursor(&buffer.start_iter());
                view.set_editable(true);
                let _ = sender.try_send(());
                return glib::ControlFlow::Break;
            };
            let mut end = buffer.end_iter();
            buffer.insert(&mut end, chunk);
            offset += chunk.len();
            glib::ControlFlow::Continue
        });
        receiver
    }
}
