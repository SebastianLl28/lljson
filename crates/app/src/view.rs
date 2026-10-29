//! Virtualized view: a custom `GListModel` over the flat row list from `core`, plus the
//! factory that fills each row on `bind`. Only visible rows have widgets.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use lljson_core::{Cell, DiffResult, DiffStatus, Document, RowKind, Rows, TokenKind, cell_text};

use crate::pane::Side;
use crate::search::SearchState;

const INDENT_PX: i32 = 16;
/// Indentation stops growing past this depth.
const MAX_INDENT_DEPTH: u16 = 200;

const STYLE: &str = include_str!("style.css");
const STYLE_DARK: &str = include_str!("style-dark.css");

pub fn install_styles() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let base = gtk::CssProvider::new();
    base.load_from_string(STYLE);
    gtk::style_context_add_provider_for_display(
        &display,
        &base,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let dark = gtk::CssProvider::new();
    let manager = adw::StyleManager::default();
    gtk::style_context_add_provider_for_display(
        &display,
        &dark,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
    let apply = move |manager: &adw::StyleManager| {
        dark.load_from_string(if manager.is_dark() { STYLE_DARK } else { "" });
    };
    apply(&manager);
    manager.connect_dark_notify(apply);
}

fn token_class(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::Punctuation => "json-punct",
        TokenKind::String => "json-string",
        TokenKind::Number => "json-number",
        TokenKind::Bool => "json-bool",
        TokenKind::Null => "json-null",
    }
}

const DIFF_ROW_CLASSES: [&str; 5] = [
    "diff-added",
    "diff-removed",
    "diff-modified",
    "diff-type-changed",
    "diff-ignored",
];

/// Collapsed equal section: dimmed to tell it apart from sections with changes.
const EQUAL_SECTION_CLASS: &str = "diff-equal-section";

const SEARCH_MATCH_CLASS: &str = "search-match";
const SEARCH_CURRENT_CLASS: &str = "search-current";

/// `Contains` containers are not painted: the changes show on their children.
fn diff_class(status: DiffStatus) -> Option<&'static str> {
    match status {
        DiffStatus::Added => Some("diff-added"),
        DiffStatus::Removed => Some("diff-removed"),
        DiffStatus::Modified => Some("diff-modified"),
        DiffStatus::TypeChanged => Some("diff-type-changed"),
        DiffStatus::Ignored => Some("diff-ignored"),
        DiffStatus::Equal | DiffStatus::Contains => None,
    }
}

/// Displayed documents and their aligned rows. `diff` only exists with two documents.
#[derive(Default)]
pub struct Comparison {
    pub left: Option<Arc<Document>>,
    pub right: Option<Arc<Document>>,
    pub rows: Rows,
    pub diff: Option<DiffResult>,
    /// Indexed by [`Side::index`]: each pane has its own search.
    pub search: [SearchState; 2],
}

impl Comparison {
    pub fn search(&self, side: Side) -> &SearchState {
        &self.search[side.index()]
    }

    pub fn search_mut(&mut self, side: Side) -> &mut SearchState {
        &mut self.search[side.index()]
    }

    fn document(&self, side: Side) -> Option<&Document> {
        match side {
            Side::Left => self.left.as_deref(),
            Side::Right => self.right.as_deref(),
        }
    }

    pub fn cell(&self, side: Side, position: usize) -> Option<Cell> {
        let row = self.rows.get(position)?;
        match side {
            Side::Left => row.left(),
            Side::Right => row.right(),
        }
    }

    fn status(&self, side: Side, cell: Cell) -> DiffStatus {
        self.diff
            .as_ref()
            .map_or(DiffStatus::Equal, |d| match side {
                Side::Left => d.left_status(cell.node),
                Side::Right => d.right_status(cell.node),
            })
    }

    /// Collapsed container with hidden differences inside.
    pub fn is_collapsed_with_changes(&self, position: usize) -> bool {
        let (Some(diff), Some(row)) = (&self.diff, self.rows.get(position)) else {
            return false;
        };
        self.is_expanded(position) == Some(false)
            && (row
                .left()
                .is_some_and(|c| diff.left_status(c.node) == DiffStatus::Contains)
                || row
                    .right()
                    .is_some_and(|c| diff.right_status(c.node) == DiffStatus::Contains))
    }

    /// `Some(expanded)` if the row is an expandable container.
    fn is_expanded(&self, position: usize) -> Option<bool> {
        self.rows
            .is_expanded(position, self.left.as_deref(), self.right.as_deref())
    }
}

mod imp {
    use std::cell::Cell;

    use super::*;

    /// Only stores the row count; items are created on demand and read from [`Comparison`].
    #[derive(Default)]
    pub struct RowsModel {
        pub len: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RowsModel {
        const NAME: &'static str = "LljsonRowsModel";
        type Type = super::RowsModel;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for RowsModel {}

    impl ListModelImpl for RowsModel {
        fn item_type(&self) -> glib::Type {
            glib::BoxedAnyObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.len.get()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            (position < self.len.get()).then(|| glib::BoxedAnyObject::new(position).upcast())
        }
    }
}

glib::wrapper! {
    pub struct RowsModel(ObjectSubclass<imp::RowsModel>) @implements gio::ListModel;
}

impl RowsModel {
    pub fn new() -> RowsModel {
        glib::Object::new()
    }

    /// Must be called with no active borrow of [`Comparison`]: the view re-requests
    /// rows immediately.
    pub fn splice(&self, position: usize, removed: usize, added: usize, len: usize) {
        self.imp().len.set(to_u32(len));
        self.items_changed(to_u32(position), to_u32(removed), to_u32(added));
    }

    pub fn len(&self) -> usize {
        self.imp().len.get() as usize
    }
}

impl Default for RowsModel {
    fn default() -> RowsModel {
        RowsModel::new()
    }
}

pub fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

struct RowWidgets {
    root: gtk::Box,
    expander: gtk::Image,
    key: gtk::Label,
    value: gtk::Label,
    comma: gtk::Label,
    summary: gtk::Label,
}

impl RowWidgets {
    fn new() -> RowWidgets {
        let label = || {
            gtk::Label::builder()
                .xalign(0.0)
                .single_line_mode(true)
                .build()
        };
        let expander = gtk::Image::builder()
            .icon_name("pan-down-symbolic")
            .pixel_size(12)
            .css_classes(["row-expander"])
            .build();
        let key = label();
        key.add_css_class("json-key");
        let value = label();
        let comma = label();
        comma.add_css_class("json-punct");
        let summary = label();
        summary.add_css_class("dim-label");
        summary.set_margin_start(12);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .hexpand(true)
            .css_classes(["json-row"])
            .build();
        root.append(&expander);
        root.append(&key);
        root.append(&value);
        root.append(&comma);
        root.append(&summary);
        RowWidgets {
            root,
            expander,
            key,
            value,
            comma,
            summary,
        }
    }

    fn from_root(root: gtk::Box) -> Option<RowWidgets> {
        let expander = root.first_child()?;
        let key = expander.next_sibling()?;
        let value = key.next_sibling()?;
        let comma = value.next_sibling()?;
        let summary = comma.next_sibling()?;
        Some(RowWidgets {
            expander: expander.downcast().ok()?,
            key: key.downcast().ok()?,
            value: value.downcast().ok()?,
            comma: comma.downcast().ok()?,
            summary: summary.downcast().ok()?,
            root,
        })
    }

    fn clear(&self) {
        for label in [&self.key, &self.value, &self.comma, &self.summary] {
            label.set_label("");
        }
        self.expander.set_opacity(0.0);
        self.expander.set_can_target(false);
        self.root.remove_css_class("json-filler");
        self.expander.set_margin_start(0);
        self.value.set_css_classes(&[]);
        for class in DIFF_ROW_CLASSES.into_iter().chain([
            EQUAL_SECTION_CLASS,
            SEARCH_MATCH_CLASS,
            SEARCH_CURRENT_CLASS,
        ]) {
            self.root.remove_css_class(class);
        }
    }
}

/// Right click on a row: side, position, row widget and coordinates within it.
pub type ContextMenu = Rc<dyn Fn(Side, u32, &gtk::Widget, f64, f64)>;

pub fn factory(
    side: Side,
    comparison: Rc<RefCell<Comparison>>,
    on_toggle: Rc<dyn Fn(u32)>,
    on_context: ContextMenu,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let widgets = RowWidgets::new();
        let click = gtk::GestureClick::new();
        let weak_item = item.downgrade();
        let on_toggle = Rc::clone(&on_toggle);
        click.connect_released(move |gesture, _, _, _| {
            if let Some(item) = weak_item.upgrade() {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                on_toggle(item.position());
            }
        });
        widgets.expander.add_controller(click);
        let menu = gtk::GestureClick::builder()
            .button(gtk::gdk::BUTTON_SECONDARY)
            .build();
        let weak_item = item.downgrade();
        let weak_root = widgets.root.downgrade();
        let on_context = Rc::clone(&on_context);
        menu.connect_pressed(move |gesture, _, x, y| {
            if let (Some(item), Some(root)) = (weak_item.upgrade(), weak_root.upgrade()) {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                on_context(side, item.position(), root.upcast_ref(), x, y);
            }
        });
        widgets.root.add_controller(menu);
        item.set_activatable(true);
        item.set_child(Some(&widgets.root));
    });
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(widgets) = item
            .child()
            .and_then(|c| c.downcast::<gtk::Box>().ok())
            .and_then(RowWidgets::from_root)
        else {
            return;
        };
        bind(
            &widgets,
            side,
            &comparison.borrow(),
            item.position() as usize,
        );
    });
    factory
}

fn bind(widgets: &RowWidgets, side: Side, comparison: &Comparison, position: usize) {
    widgets.clear();
    let (Some(cell), Some(doc)) = (comparison.cell(side, position), comparison.document(side))
    else {
        // Filler gap: the other pane has content on this row.
        if position < comparison.rows.len() {
            widgets.root.add_css_class("json-filler");
        }
        return;
    };
    let depth = comparison.rows.get(position).map_or(0, |r| r.depth());
    widgets
        .expander
        .set_margin_start(i32::from(depth.min(MAX_INDENT_DEPTH)) * INDENT_PX);

    if let Some(expanded) = comparison.is_expanded(position) {
        widgets.expander.set_opacity(1.0);
        widgets.expander.set_can_target(true);
        if !expanded
            && comparison.diff.is_some()
            && comparison.status(side, cell) == DiffStatus::Equal
        {
            widgets.root.add_css_class(EQUAL_SECTION_CLASS);
        }
        widgets.expander.set_icon_name(Some(if expanded {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        }));
    }
    // A container's closing row repeats its node: only the opening row is highlighted.
    let search = comparison.search(side);
    if cell.kind != RowKind::Close && search.is_match(cell.node) {
        widgets.root.add_css_class(SEARCH_MATCH_CLASS);
        if search.is_current(cell.node) {
            widgets.root.add_css_class(SEARCH_CURRENT_CLASS);
        }
    }
    let text = cell_text(doc, cell);
    if let Some(key) = &text.key {
        widgets.key.set_label(&format!("{key}: "));
    }
    widgets.value.set_label(&text.value);
    let mut value_classes = vec!["json-value", token_class(text.value_kind)];
    if let Some(class) = diff_class(comparison.status(side, cell)) {
        widgets.root.add_css_class(class);
        value_classes.push(class);
    }
    widgets.value.set_css_classes(&value_classes);
    if text.comma {
        widgets.comma.set_label(",");
    }
    if let Some(summary) = &text.summary {
        widgets.summary.set_label(summary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_painted_status_has_a_declared_class() {
        let painted = [
            DiffStatus::Added,
            DiffStatus::Removed,
            DiffStatus::Modified,
            DiffStatus::TypeChanged,
            DiffStatus::Ignored,
        ];
        for status in painted {
            let class = diff_class(status).expect("painted status");
            assert!(DIFF_ROW_CLASSES.contains(&class), "{class}");
        }
        assert_eq!(DIFF_ROW_CLASSES.len(), painted.len());
    }

    #[test]
    fn equal_rows_and_containers_with_changes_are_not_painted() {
        assert_eq!(diff_class(DiffStatus::Equal), None);
        assert_eq!(diff_class(DiffStatus::Contains), None);
    }
}
