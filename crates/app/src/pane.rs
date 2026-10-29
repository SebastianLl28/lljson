//! One document pane: title/actions bar and a stack of states (empty, loading, rows, error).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use lljson_core::Document;

use crate::loader::{self, Input, LoadError, LoadResult, Loaded};
use crate::session;
use crate::view::RowsModel;

/// Loads slower than this show the progress indicator.
const PROGRESS_DELAY: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }

    pub fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

pub struct Pane {
    side: Side,
    pub root: gtk::Box,
    title: gtk::Label,
    stats: gtk::Label,
    stack: gtk::Stack,
    pub list: gtk::ListView,
    pub scrolled: gtk::ScrolledWindow,
    error: adw::StatusPage,
    loading_label: gtk::Label,
    document: RefCell<Option<Loaded>>,
    /// Id of the latest load request; results from older loads are discarded.
    generation: Cell<u64>,
    loading: Cell<bool>,
    on_changed: RefCell<Option<Box<dyn Fn()>>>,
}

impl Pane {
    pub fn new(side: Side, model: &RowsModel, factory: gtk::SignalListItemFactory) -> Rc<Pane> {
        let open_action = format!("win.open-{}", side.name());
        let paste_action = format!("win.paste-{}", side.name());
        let shift = if side == Side::Right { "Shift+" } else { "" };

        let title = gtk::Label::builder()
            .label("No document")
            .hexpand(true)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["heading"])
            .build();
        let stats = gtk::Label::builder()
            .css_classes(["dim-label", "caption"])
            .build();
        let open = gtk::Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text(format!("Open file (Ctrl+{shift}O)"))
            .action_name(open_action.as_str())
            .build();
        let paste = gtk::Button::builder()
            .icon_name("edit-paste-symbolic")
            .tooltip_text(format!("Paste from clipboard (Ctrl+{shift}V)"))
            .action_name(paste_action.as_str())
            .build();
        let format = gio::Menu::new();
        for (label, param) in [
            ("Format with 2 spaces", "2"),
            ("Format with 4 spaces", "4"),
            ("Format with tabs", "tab"),
        ] {
            let item = gio::MenuItem::new(Some(label), None);
            item.set_action_and_target_value(
                Some(&format!("win.format-{}", side.name())),
                Some(&param.to_variant()),
            );
            format.append_item(&item);
        }
        let transform = gio::Menu::new();
        transform.append(Some("Minify"), Some(&format!("win.minify-{}", side.name())));
        transform.append(
            Some("Sort keys"),
            Some(&format!("win.sort-{}", side.name())),
        );
        let export = gio::Menu::new();
        export.append(
            Some("Copy document text"),
            Some(&format!("win.copy-text-{}", side.name())),
        );
        let menu = gio::Menu::new();
        menu.append_section(None, &format);
        menu.append_section(None, &transform);
        menu.append_section(None, &export);
        let format_button = gtk::MenuButton::builder()
            .icon_name("format-justify-left-symbolic")
            .tooltip_text("Format, minify, sort keys or copy the text")
            .menu_model(&menu)
            .build();
        let bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .css_classes(["toolbar"])
            .build();
        bar.append(&title);
        bar.append(&stats);
        bar.append(&format_button);
        bar.append(&open);
        bar.append(&paste);

        let list = gtk::ListView::builder()
            .model(&gtk::NoSelection::new(Some(model.clone())))
            .factory(&factory)
            .css_classes(["json-view", "monospace"])
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .hexpand(true)
            .build();
        let error = adw::StatusPage::builder()
            .icon_name("dialog-error-symbolic")
            .css_classes(["error"])
            .build();
        let loading_label = gtk::Label::builder()
            .css_classes(["title-4"])
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        let loading = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .margin_start(12)
            .margin_end(12)
            .build();
        let spinner = adw::Spinner::builder()
            .width_request(48)
            .height_request(48)
            .build();
        loading.append(&spinner);
        loading.append(&loading_label);

        let stack = gtk::Stack::builder()
            .vexpand(true)
            .hexpand(true)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        stack.add_named(&empty_page(&open_action, &paste_action), Some("empty"));
        stack.add_named(&loading, Some("loading"));
        stack.add_named(&scrolled, Some("list"));
        stack.add_named(&error, Some("error"));

        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(280)
            .build();
        root.append(&bar);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&stack);

        Rc::new(Pane {
            side,
            root,
            title,
            stats,
            stack,
            list,
            scrolled,
            error,
            loading_label,
            document: RefCell::new(None),
            generation: Cell::new(0),
            loading: Cell::new(false),
            on_changed: RefCell::new(None),
        })
    }

    pub fn title(&self) -> String {
        self.title.label().into()
    }

    pub fn document(&self) -> Option<Arc<Document>> {
        self.document
            .borrow()
            .as_ref()
            .map(|loaded| Arc::clone(&loaded.document))
    }

    pub fn show_list(&self) {
        if self.document.borrow().is_some() {
            self.stack.set_visible_child_name("list");
        }
    }

    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        self.on_changed.replace(Some(Box::new(f)));
    }

    /// Loads in the background; a new load invalidates the previous one.
    pub fn load(self: &Rc<Self>, input: Input) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.loading.set(true);
        let name = input.name();
        let receiver = loader::spawn_load(input);

        let pane: Weak<Pane> = Rc::downgrade(self);
        let label = format!("Loading {name}…");
        glib::timeout_add_local_once(PROGRESS_DELAY, move || {
            if let Some(pane) = pane.upgrade()
                && pane.loading.get()
                && pane.generation.get() == generation
            {
                pane.loading_label.set_label(&label);
                pane.stack.set_visible_child_name("loading");
            }
        });

        let pane: Weak<Pane> = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = receiver.recv().await;
            let Some(pane) = pane.upgrade() else {
                return;
            };
            if pane.generation.get() != generation {
                return;
            }
            // The channel only closes without a result if the load thread panicked.
            let result = result.unwrap_or_else(|_| Err(LoadError::Interrupted { name }));
            pane.finish(result);
        });
    }

    /// Keeps a copy of the document so the next run starts with it.
    fn persist(&self, loaded: &Loaded) {
        let side = self.side;
        let name = loaded.name.clone();
        let document = Arc::clone(&loaded.document);
        std::thread::spawn(move || {
            if let Err(err) = session::save_pane(side, &name, document.source_bytes()) {
                eprintln!("lljson: could not save the {} pane: {err}", side.name());
            }
        });
    }

    fn finish(&self, result: LoadResult) {
        self.loading.set(false);
        match result {
            Ok(loaded) => {
                #[cfg(any(debug_assertions, feature = "devtools"))]
                crate::devtools::trace(&format!(
                    "document loaded: {} ({} ms)",
                    loaded.name,
                    loaded.elapsed.as_millis()
                ));
                // The list is shown once the window finishes building the rows.
                self.title.set_label(&loaded.name);
                self.title.set_tooltip_text(Some(&describe(&loaded)));
                self.stats.set_label(&format!(
                    "{} nodes · {}",
                    group_digits(loaded.document.node_count()),
                    format_bytes(loaded.document.source_bytes().len())
                ));
                self.persist(&loaded);
                self.document.replace(Some(loaded));
            }
            Err(err) => {
                self.title.set_label(err.name());
                self.title.set_tooltip_text(None);
                self.stats.set_label("");
                self.error
                    .set_title(&format!("Could not load {}", err.name()));
                self.error.set_description(Some(&err.to_string()));
                self.stack.set_visible_child_name("error");
                self.document.replace(None);
            }
        }
        if let Some(f) = self.on_changed.borrow().as_ref() {
            f();
        }
    }
}

fn describe(loaded: &Loaded) -> String {
    let mut text = format!("Read and parsed in {} ms", loaded.elapsed.as_millis());
    if loaded.fixes > 0 {
        text.push_str(&format!(
            "\nJSONC: {} comments or trailing commas accepted",
            loaded.fixes
        ));
    }
    text
}

/// Human-readable size: `512 B`, `3.4 KiB`, `20.7 MiB`.
pub fn format_bytes(bytes: usize) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Groups thousands with thin spaces: `1 000 009`.
pub fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 * 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{202F}');
        }
        out.push(c);
    }
    out
}

fn empty_page(open_action: &str, paste_action: &str) -> adw::StatusPage {
    let open = gtk::Button::builder()
        .label("Open…")
        .action_name(open_action)
        .css_classes(["pill", "suggested-action"])
        .build();
    let paste = gtk::Button::builder()
        .label("Paste")
        .action_name(paste_action)
        .css_classes(["pill"])
        .build();
    let buttons = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .halign(gtk::Align::Center)
        .build();
    buttons.append(&open);
    buttons.append(&paste);
    adw::StatusPage::builder()
        .icon_name("document-open-symbolic")
        .title("Drop a JSON here")
        .description("or open it from a file or paste it from the clipboard")
        .child(&buttons)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_are_human_readable() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(21_705_000), "20.7 MiB");
        assert_eq!(format_bytes(usize::MAX), format_bytes(usize::MAX));
    }

    #[test]
    fn digits_are_grouped() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1\u{202F}000");
        assert_eq!(group_digits(1_000_009), "1\u{202F}000\u{202F}009");
    }

    #[test]
    fn sides() {
        assert_eq!(Side::Left.other(), Side::Right);
        assert_eq!(Side::Right.name(), "right");
    }
}
