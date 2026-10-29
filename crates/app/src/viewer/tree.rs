//! Tree of the Viewer: a virtualized `GtkListView` over the flat rows from `core`. Only the
//! visible rows have widgets; their text is generated when they are bound.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use std::sync::Arc;

use lljson_core::{Document, Kind, TreeLabel, TreeRows, tree_text};

use super::validate::Tree;
use crate::view::{RowsModel, token_class};

const INDENT_PX: i32 = 16;
/// Indentation stops growing past this depth.
const MAX_INDENT_DEPTH: u16 = 200;

#[derive(Default)]
struct TreeData {
    document: Option<Arc<Document>>,
    rows: TreeRows,
}

pub struct TreeView {
    pub root: gtk::ScrolledWindow,
    model: RowsModel,
    data: Rc<RefCell<TreeData>>,
}

impl TreeView {
    pub fn new() -> TreeView {
        let model = RowsModel::new();
        let data = Rc::new(RefCell::new(TreeData::default()));
        let list = gtk::ListView::new(
            Some(gtk::NoSelection::new(Some(model.clone()))),
            Some(factory(Rc::clone(&data))),
        );
        let root = gtk::ScrolledWindow::builder()
            .child(&list)
            .hexpand(true)
            .vexpand(true)
            .build();
        TreeView { root, model, data }
    }

    /// Replaces the displayed tree.
    pub fn show(&self, tree: Tree) {
        let len = tree.rows.len();
        {
            let mut data = self.data.borrow_mut();
            data.document = Some(tree.document);
            data.rows = tree.rows;
        }
        // The borrow is released: the view requests the new rows right away.
        self.model.splice(0, self.model.len(), len, len);
    }

    pub fn clear(&self) {
        {
            let mut data = self.data.borrow_mut();
            data.document = None;
            data.rows = TreeRows::default();
        }
        self.model.splice(0, self.model.len(), 0, 0);
    }
}

/// Short symbol standing for a node type, drawn in the color of its value.
fn type_icon(kind: Kind) -> &'static str {
    match kind {
        Kind::Object => "{}",
        Kind::Array => "[]",
        Kind::String => "“”",
        Kind::Number => "#",
        Kind::Bool => "◑",
        Kind::Null => "∅",
    }
}

struct RowWidgets {
    root: gtk::Box,
    icon: gtk::Label,
    key: gtk::Label,
    value: gtk::Label,
}

impl RowWidgets {
    fn new() -> RowWidgets {
        let label = || {
            gtk::Label::builder()
                .xalign(0.0)
                .single_line_mode(true)
                .build()
        };
        let icon = label();
        icon.set_width_chars(3);
        icon.set_xalign(0.5);
        let key = label();
        let value = label();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["json-row"])
            .build();
        root.append(&icon);
        root.append(&key);
        root.append(&value);
        RowWidgets {
            root,
            icon,
            key,
            value,
        }
    }

    fn from_root(root: gtk::Box) -> Option<RowWidgets> {
        let icon = root.first_child()?;
        let key = icon.next_sibling()?;
        let value = key.next_sibling()?;
        Some(RowWidgets {
            icon: icon.downcast().ok()?,
            key: key.downcast().ok()?,
            value: value.downcast().ok()?,
            root,
        })
    }
}

fn factory(data: Rc<RefCell<TreeData>>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            item.set_child(Some(&RowWidgets::new().root));
        }
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
        bind(&widgets, &data.borrow(), item.position() as usize);
    });
    factory
}

fn bind(widgets: &RowWidgets, data: &TreeData, position: usize) {
    for label in [&widgets.icon, &widgets.key, &widgets.value] {
        label.set_label("");
    }
    let (Some(row), Some(doc)) = (data.rows.get(position), data.document.as_deref()) else {
        return;
    };
    widgets
        .root
        .set_margin_start(i32::from(row.depth.min(MAX_INDENT_DEPTH)) * INDENT_PX);
    let text = tree_text(doc, row);
    let value_class = token_class(text.value_kind);
    widgets.icon.set_label(type_icon(text.kind));
    widgets.icon.set_css_classes(&[value_class]);
    // Containers read `users [3]`; scalars `name: "x"`.
    let separator = if text.length.is_some() { " " } else { ": " };
    match &text.label {
        TreeLabel::Root => {}
        TreeLabel::Key(key) => {
            widgets.key.set_label(&format!("{key}{separator}"));
            widgets.key.set_css_classes(&["json-key"]);
        }
        TreeLabel::Index(index) => {
            widgets.key.set_label(&format!("[{index}]{separator}"));
            widgets.key.set_css_classes(&["dim-label"]);
        }
    }
    widgets.value.set_label(&text.value);
    widgets.value.set_css_classes(&["json-value", value_class]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_has_a_distinct_icon() {
        let kinds = [
            Kind::Object,
            Kind::Array,
            Kind::String,
            Kind::Number,
            Kind::Bool,
            Kind::Null,
        ];
        let mut icons: Vec<&str> = kinds.into_iter().map(type_icon).collect();
        icons.sort_unstable();
        icons.dedup();
        assert_eq!(icons.len(), kinds.len());
    }
}
