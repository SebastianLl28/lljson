//! Tree of the Viewer: a virtualized `GtkListView` over the flat rows from `core`. Only the
//! visible rows have widgets; their text is generated when they are bound.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use lljson_core::{Cell, Document, RowKind, TreeRows, cell_text};
use std::sync::Arc;

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

struct RowWidgets {
    root: gtk::Box,
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
        let key = label();
        key.add_css_class("json-key");
        let value = label();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["json-row"])
            .build();
        root.append(&key);
        root.append(&value);
        RowWidgets { root, key, value }
    }

    fn from_root(root: gtk::Box) -> Option<RowWidgets> {
        let key = root.first_child()?;
        let value = key.next_sibling()?;
        Some(RowWidgets {
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
    widgets.key.set_label("");
    widgets.value.set_label("");
    let (Some(row), Some(doc)) = (data.rows.get(position), data.document.as_deref()) else {
        return;
    };
    widgets
        .root
        .set_margin_start(i32::from(row.depth.min(MAX_INDENT_DEPTH)) * INDENT_PX);
    let text = cell_text(
        doc,
        Cell {
            node: row.node,
            kind: RowKind::Leaf,
        },
    );
    if let Some(key) = &text.key {
        widgets.key.set_label(&format!("{key}: "));
    }
    widgets.value.set_label(&text.value);
    widgets
        .value
        .set_css_classes(&["json-value", token_class(text.value_kind)]);
}
