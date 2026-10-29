//! Plain text editor of the Viewer: a `GtkTextView` that accepts anything the user types or pastes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use lljson_core::{Syntax, highlight_line};

use super::chunks::chunks;
use super::validate::Mark;

/// Between redrawing (`PRIORITY_REDRAW`) and the text view's background line layout (+5): frames
/// are drawn between slices, but the slices do not wait behind the layout of the text so far.
const SLICE_PRIORITY: i32 = gdk::PRIORITY_REDRAW as i32 + 1;

/// Text inserted per idle callback: about 4 ms of layout work, well under one frame.
const CHUNK_BYTES: usize = 256 * 1024;

/// Lines longer than this are not highlighted: a minified file is a single huge line and
/// tokenizing it on every scroll would stall the window.
const MAX_HIGHLIGHT_LINE_BYTES: usize = 4096;

/// Lines past the first visible one that are ever considered visible.
const MAX_VISIBLE_LINES: i32 = 200;

/// Most line-number labels ever created: more lines than this never fit on screen.
const MAX_GUTTER_LABELS: usize = 300;

/// Lines highlighted above and below the visible ones, so that small scrolls find them done.
const HIGHLIGHT_MARGIN_LINES: i32 = 30;

/// Syntax kinds, the tag drawn for each and the CSS class the color comes from.
const SYNTAX_STYLES: [(Syntax, &str, &str); 7] = [
    (Syntax::Key, "syn-key", "json-key"),
    (Syntax::String, "syn-string", "json-string"),
    (Syntax::Number, "syn-number", "json-number"),
    (Syntax::Bool, "syn-bool", "json-bool"),
    (Syntax::Null, "syn-null", "json-null"),
    (Syntax::Punctuation, "syn-punct", "json-punct"),
    (Syntax::Comment, "syn-comment", "dim-label"),
];

pub struct Editor {
    /// Line numbers and the text, side by side.
    pub root: gtk::Box,
    pub(super) scrolled: gtk::ScrolledWindow,
    pub(super) view: gtk::TextView,
    gutter: Gutter,
    /// A highlight pass is already queued.
    highlight_queued: Rc<Cell<bool>>,
    /// Id of the latest `set_text`; an insertion in progress stops when it is no longer current.
    load: Rc<Cell<u64>>,
    /// An error mark is applied, so the next edit has to clear it.
    marked: Cell<bool>,
}

const ERROR_TAG: &str = "syntax-error";

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
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&view)
            .hexpand(true)
            .vexpand(true)
            .build();
        let gutter = Gutter::new();
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&gutter.root);
        root.append(&scrolled);
        let tag = gtk::TextTag::builder()
            .name(ERROR_TAG)
            .underline(gtk::pango::Underline::Error)
            .underline_rgba(&gdk::RGBA::new(0.88, 0.11, 0.14, 1.0))
            .background_rgba(&gdk::RGBA::new(0.88, 0.11, 0.14, 0.25))
            .build();
        view.buffer().tag_table().add(&tag);
        for (_, name, _) in SYNTAX_STYLES {
            view.buffer()
                .tag_table()
                .add(&gtk::TextTag::builder().name(name).build());
        }
        let editor = Editor {
            root,
            scrolled,
            view,
            gutter,
            highlight_queued: Rc::new(Cell::new(false)),
            load: Rc::new(Cell::new(0)),
            marked: Cell::new(false),
        };
        editor.connect_decorations();
        // The first pass needs the widgets realized (colors come from their style).
        let (view, gutter) = (editor.view.clone(), editor.gutter.clone());
        let queued = Rc::clone(&editor.highlight_queued);
        editor
            .view
            .connect_map(move |_| queue_highlight(&view, &gutter, &queued));
        let (view, gutter) = (editor.view.clone(), editor.gutter.clone());
        let queued = Rc::clone(&editor.highlight_queued);
        adw::StyleManager::default()
            .connect_dark_notify(move |_| queue_highlight(&view, &gutter, &queued));
        editor
    }

    /// Keeps line numbers and highlighting in step with scrolling, resizing and edits.
    fn connect_decorations(&self) {
        let adjustment = self.scrolled.vadjustment();
        let (view, gutter) = (self.view.clone(), self.gutter.clone());
        let queued = Rc::clone(&self.highlight_queued);
        adjustment.connect_value_changed(move |_| {
            gutter.update(&view);
            queue_highlight(&view, &gutter, &queued);
        });
        // The estimated height of the text keeps being refined while it is laid out: the numbers
        // follow, but highlighting does not (it would never settle).
        let (view, gutter) = (self.view.clone(), self.gutter.clone());
        adjustment.connect_changed(move |_| gutter.schedule(&view));
        let (view, gutter) = (self.view.clone(), self.gutter.clone());
        let queued = Rc::clone(&self.highlight_queued);
        self.view.buffer().connect_changed(move |_| {
            gutter.schedule(&view);
            queue_highlight(&view, &gutter, &queued);
        });
    }

    /// Colors may have changed (theme): repaint the highlighting.
    pub fn refresh_colors(&self) {
        queue_highlight(&self.view, &self.gutter, &self.highlight_queued);
    }

    /// Underlines the character of a validation error; replaces any previous mark.
    pub fn mark_error(&self, mark: &Mark) {
        self.clear_error();
        let buffer = self.view.buffer();
        let (Ok(line), Ok(start), Ok(end)) = (
            i32::try_from(mark.line),
            i32::try_from(mark.start),
            i32::try_from(mark.end),
        ) else {
            return;
        };
        if line >= buffer.line_count() {
            return;
        }
        let (Some(from), Some(to)) = (
            buffer.iter_at_line_index(line, start),
            buffer.iter_at_line_index(line, end),
        ) else {
            return;
        };
        buffer.apply_tag_by_name(ERROR_TAG, &from, &to);
        self.marked.set(true);
    }

    /// Removes the error mark, if any (cheap when there is none).
    pub fn clear_error(&self) {
        if self.marked.replace(false) {
            let buffer = self.view.buffer();
            buffer.remove_tag_by_name(ERROR_TAG, &buffer.start_iter(), &buffer.end_iter());
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

/// Highlights from an idle callback so that a burst of edits or scroll events costs one pass.
fn queue_highlight(view: &gtk::TextView, gutter: &Gutter, queued: &Rc<Cell<bool>>) {
    if queued.replace(true) {
        return;
    }
    let (view, gutter, queued) = (view.clone(), gutter.clone(), Rc::clone(queued));
    glib::idle_add_local_full(glib::Priority::from(SLICE_PRIORITY), move || {
        queued.set(false);
        highlight_visible(&view, &gutter);
        glib::ControlFlow::Break
    });
}

/// Colors the visible lines (and a margin around them); everything else keeps whatever it had.
fn highlight_visible(view: &gtk::TextView, gutter: &Gutter) {
    let buffer = view.buffer();
    let visible = view.visible_rect();
    let (top, _) = view.line_at_y(visible.y());
    let (bottom, _) = view.line_at_y(visible.y() + visible.height());
    let first = (top.line() - HIGHLIGHT_MARGIN_LINES).max(0);
    // However tall the view claims to be (before it is laid out), only so many lines fit.
    let last = (bottom.line().min(top.line() + MAX_VISIBLE_LINES) + HIGHLIGHT_MARGIN_LINES)
        .min(buffer.line_count() - 1);
    let Some(from) = buffer.iter_at_line(first) else {
        return;
    };
    let Some(mut to) = buffer.iter_at_line(last) else {
        return;
    };
    to.forward_to_line_end();
    for (_, name, class) in SYNTAX_STYLES {
        if let (Some(tag), Some(color)) = (buffer.tag_table().lookup(name), gutter.color_of(class))
            && tag.property::<Option<gdk::RGBA>>("foreground-rgba") != Some(color)
        {
            // A tag change invalidates the layout of the whole buffer: only when it differs.
            tag.set_foreground_rgba(Some(&color));
        }
        buffer.remove_tag_by_name(name, &from, &to);
    }
    for line in first..=last {
        let Some(start) = buffer.iter_at_line(line) else {
            continue;
        };
        let mut end = start;
        end.forward_to_line_end();
        let text = buffer.text(&start, &end, false);
        if text.len() > MAX_HIGHLIGHT_LINE_BYTES {
            continue;
        }
        for highlight in highlight_line(&text) {
            let Some(name) = SYNTAX_STYLES
                .iter()
                .find(|(syntax, ..)| *syntax == highlight.syntax)
                .map(|(_, name, _)| *name)
            else {
                continue;
            };
            let (Ok(a), Ok(b)) = (
                i32::try_from(highlight.range.start),
                i32::try_from(highlight.range.end),
            ) else {
                continue;
            };
            if let (Some(a), Some(b)) = (
                buffer.iter_at_line_index(line, a),
                buffer.iter_at_line_index(line, b),
            ) {
                buffer.apply_tag_by_name(name, &a, &b);
            }
        }
    }
}

/// Line numbers drawn next to the text: one label per visible line, moved on scroll. Only the
/// visible lines have widgets, however long the text is.
#[derive(Clone)]
struct Gutter {
    /// Only the spacer gives it a size: the labels, being overlays, never affect the layout
    /// (otherwise moving them would resize the editor, which moves them again).
    root: gtk::Overlay,
    spacer: gtk::Box,
    fixed: gtk::Fixed,
    labels: Rc<RefCell<Vec<gtk::Label>>>,
    /// Never shown: styled with the syntax classes so that their colors can be read.
    probes: Rc<Vec<(&'static str, gtk::Label)>>,
    width_chars: Rc<Cell<i32>>,
    /// An update is already queued.
    queued: Rc<Cell<bool>>,
    /// An update is running: laying out lines to place the numbers must not start another.
    updating: Rc<Cell<bool>>,
}

impl Gutter {
    fn new() -> Gutter {
        let fixed = gtk::Fixed::new();
        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let root = gtk::Overlay::builder()
            .css_classes(["line-gutter"])
            .overflow(gtk::Overflow::Hidden)
            .child(&spacer)
            .build();
        root.add_overlay(&fixed);
        let probes: Vec<_> = SYNTAX_STYLES
            .iter()
            .map(|(.., class)| {
                let label = gtk::Label::builder()
                    .css_classes([*class])
                    .opacity(0.0)
                    .can_target(false)
                    .build();
                fixed.put(&label, 0.0, 0.0);
                (*class, label)
            })
            .collect();
        Gutter {
            root,
            spacer,
            fixed,
            labels: Rc::new(RefCell::new(Vec::new())),
            probes: Rc::new(probes),
            width_chars: Rc::new(Cell::new(0)),
            queued: Rc::new(Cell::new(false)),
            updating: Rc::new(Cell::new(false)),
        }
    }

    fn color_of(&self, class: &str) -> Option<gdk::RGBA> {
        self.probes
            .iter()
            .find(|(c, _)| *c == class)
            .map(|(_, label)| label.color())
    }

    /// Repositions the labels once the current signal handlers are done.
    fn schedule(&self, view: &gtk::TextView) {
        if self.queued.replace(true) {
            return;
        }
        let (gutter, view) = (self.clone(), view.clone());
        glib::idle_add_local_full(glib::Priority::from(SLICE_PRIORITY), move || {
            gutter.queued.set(false);
            gutter.update(&view);
            glib::ControlFlow::Break
        });
    }

    /// Repositions the labels for the lines now on screen.
    fn update(&self, view: &gtk::TextView) {
        if self.updating.replace(true) {
            return;
        }
        self.place_labels(view);
        self.updating.set(false);
    }

    fn place_labels(&self, view: &gtk::TextView) {
        let buffer = view.buffer();
        let digits = digits(buffer.line_count());
        if digits != self.width_chars.get() {
            self.width_chars.set(digits);
            let char_width = view.create_pango_layout(Some("0")).pixel_size().0;
            self.spacer.set_width_request(char_width * digits + 20);
        }
        let visible = view.visible_rect();
        let height = visible.height();
        let (mut iter, _) = view.line_at_y(visible.y());
        let mut labels = self.labels.borrow_mut();
        let mut used = 0;
        loop {
            let (_, window_y) = view.buffer_to_window_coords(gtk::TextWindowType::Widget, 0, {
                let (y, _) = view.line_yrange(&iter);
                y
            });
            if window_y > height || used >= MAX_GUTTER_LABELS {
                break;
            }
            if used == labels.len() {
                let label = gtk::Label::builder()
                    .xalign(1.0)
                    .css_classes(["dim-label", "monospace", "numeric"])
                    .build();
                self.fixed.put(&label, 0.0, 0.0);
                labels.push(label);
            }
            let label = &labels[used];
            let text = (iter.line() + 1).to_string();
            if label.label() != text {
                label.set_label(&text);
            }
            label.set_width_request(self.spacer.width_request() - 8);
            label.set_visible(true);
            self.fixed.move_(label, 0.0, f64::from(window_y));
            used += 1;
            if !iter.forward_line() {
                break;
            }
        }
        for label in &labels[used..] {
            label.set_visible(false);
        }
    }
}

fn digits(lines: i32) -> i32 {
    lines.max(1).to_string().len() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gutter_width_follows_the_digits_of_the_line_count() {
        assert_eq!(digits(0), 1);
        assert_eq!(digits(1), 1);
        assert_eq!(digits(9), 1);
        assert_eq!(digits(10), 2);
        assert_eq!(digits(1_000_000), 7);
    }
}
