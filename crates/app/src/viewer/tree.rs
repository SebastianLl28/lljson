//! Tree of the Viewer: a virtualized `GtkListView` over the flat rows from `core`. Only the
//! visible rows have widgets; their text is generated when they are bound.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk::{gdk, glib};
use gtk::prelude::*;
use lljson_core::{Document, Kind, TreeLabel, TreeRows, tree_text};

use super::validate::Tree;
use crate::nodecopy::{self, Copy};
use crate::view::{RowsModel, token_class};

const INDENT_PX: i32 = 16;
/// Indentation stops growing past this depth.
const MAX_INDENT_DEPTH: u16 = 200;

const EXPANDED_ICON: &str = "pan-down-symbolic";
const COLLAPSED_ICON: &str = "pan-end-symbolic";

/// Opens the context menu of the row at a position, at a point of the given widget.
type OnMenu = Rc<dyn Fn(u32, &gtk::Widget, f64, f64)>;

#[derive(Default)]
struct TreeData {
    document: Option<Arc<Document>>,
    rows: TreeRows,
}

/// Rebuilds of the whole row list (expand all, collapse all).
#[derive(Clone, Copy)]
enum Rebuild {
    ExpandAll,
    CollapseAll,
}

pub struct TreeView {
    pub root: gtk::Box,
    model: RowsModel,
    data: Rc<RefCell<TreeData>>,
}

impl TreeView {
    /// `notify` shows a short message to the user (copy confirmations).
    pub fn new(notify: nodecopy::Notify) -> TreeView {
        let model = RowsModel::new();
        let data = Rc::new(RefCell::new(TreeData::default()));
        let on_toggle: Rc<dyn Fn(u32)> = {
            let (model, data) = (model.clone(), Rc::clone(&data));
            Rc::new(move |position| toggle(&model, &data, position as usize))
        };
        let on_menu: OnMenu = {
            let data = Rc::clone(&data);
            Rc::new(move |position, widget, x, y| {
                show_menu(&data, &notify, position as usize, widget, x, y);
            })
        };
        let list = gtk::ListView::new(
            Some(gtk::NoSelection::new(Some(model.clone()))),
            Some(factory(Rc::clone(&data), Rc::clone(&on_toggle), on_menu)),
        );
        // Double click or Enter on a row.
        list.connect_activate(move |_, position| on_toggle(position));
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hexpand(true)
            .vexpand(true)
            .build();
        let view = TreeView {
            root: gtk::Box::new(gtk::Orientation::Vertical, 0),
            model,
            data,
        };
        view.root.append(&view.toolbar());
        view.root.append(&scrolled);
        view
    }

    fn toolbar(&self) -> gtk::Box {
        let bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .css_classes(["toolbar"])
            .build();
        for (label, tooltip, rebuild) in [
            ("Expand all", "Show every node", Rebuild::ExpandAll),
            (
                "Collapse all",
                "Hide everything below the first level",
                Rebuild::CollapseAll,
            ),
        ] {
            let button = gtk::Button::builder()
                .label(label)
                .tooltip_text(tooltip)
                .build();
            let (model, data) = (self.model.clone(), Rc::clone(&self.data));
            button.connect_clicked(move |_| rebuild_all(&model, &data, rebuild));
            bar.append(&button);
        }
        bar
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

/// Expands or collapses the container at `position`; other rows are left alone.
fn toggle(model: &RowsModel, data: &RefCell<TreeData>, position: usize) {
    let splice = {
        let mut data = data.borrow_mut();
        let TreeData { document, rows } = &mut *data;
        let Some(document) = document.as_deref() else {
            return;
        };
        rows.toggle(position, document).map(|s| (s, rows.len()))
    };
    // The borrow is released: the view requests the changed rows right away.
    if let Some((splice, len)) = splice {
        model.splice(splice.position, splice.removed, splice.added, len);
    }
}

/// Right-click menu of the row at `position`: copy its path or its value.
fn show_menu(
    data: &RefCell<TreeData>,
    notify: &nodecopy::Notify,
    position: usize,
    widget: &gtk::Widget,
    x: f64,
    y: f64,
) {
    let (node, doc) = {
        let data = data.borrow();
        let (Some(row), Some(doc)) = (data.rows.get(position), data.document.clone()) else {
            return;
        };
        (row.node, doc)
    };
    let clipboard = widget.clipboard();
    let notify = Rc::clone(notify);
    nodecopy::show_menu(widget, x, y, move |what: Copy| {
        nodecopy::copy(&clipboard, &doc, node, what, &notify);
    });
}

/// Rebuilds the whole row list on a background thread (it can hold a million rows) and swaps
/// it in, unless another document was shown in the meantime.
fn rebuild_all(model: &RowsModel, data: &Rc<RefCell<TreeData>>, rebuild: Rebuild) {
    let Some(document) = data.borrow().document.clone() else {
        return;
    };
    let (sender, receiver) = async_channel::bounded(1);
    let worker = Arc::clone(&document);
    let spawned = std::thread::Builder::new()
        .name("lljson-tree".to_owned())
        .spawn(move || {
            let mut rows = TreeRows::default();
            match rebuild {
                Rebuild::ExpandAll => rows.expand_all(&worker),
                Rebuild::CollapseAll => rows.collapse_all(&worker),
            }
            let _ = sender.send_blocking(rows);
        });
    if spawned.is_err() {
        return;
    }
    let (model, data) = (model.clone(), Rc::clone(data));
    glib::MainContext::default().spawn_local(async move {
        let Ok(rows) = receiver.recv().await else {
            return;
        };
        let (old, new) = {
            let mut data = data.borrow_mut();
            if !data
                .document
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &document))
            {
                return;
            }
            let old = data.rows.len();
            data.rows = rows;
            (old, data.rows.len())
        };
        model.splice(0, old, new, new);
    });
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
    expander: gtk::Image,
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
        let expander = gtk::Image::builder()
            .icon_name(EXPANDED_ICON)
            .pixel_size(12)
            .css_classes(["row-expander"])
            .build();
        let icon = label();
        icon.set_width_chars(3);
        icon.set_xalign(0.5);
        let key = label();
        let value = label();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["json-row"])
            .build();
        root.append(&expander);
        root.append(&icon);
        root.append(&key);
        root.append(&value);
        RowWidgets {
            root,
            expander,
            icon,
            key,
            value,
        }
    }

    fn from_root(root: gtk::Box) -> Option<RowWidgets> {
        let expander = root.first_child()?;
        let icon = expander.next_sibling()?;
        let key = icon.next_sibling()?;
        let value = key.next_sibling()?;
        Some(RowWidgets {
            expander: expander.downcast().ok()?,
            icon: icon.downcast().ok()?,
            key: key.downcast().ok()?,
            value: value.downcast().ok()?,
            root,
        })
    }

    fn clear(&self) {
        for label in [&self.icon, &self.key, &self.value] {
            label.set_label("");
        }
        self.expander.set_opacity(0.0);
        self.expander.set_can_target(false);
        self.expander.set_margin_start(0);
    }
}

fn factory(
    data: Rc<RefCell<TreeData>>,
    on_toggle: Rc<dyn Fn(u32)>,
    on_menu: OnMenu,
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
        let secondary = gtk::GestureClick::builder()
            .button(gdk::BUTTON_SECONDARY)
            .build();
        let weak_item = item.downgrade();
        let on_menu = Rc::clone(&on_menu);
        let row = widgets.root.clone();
        secondary.connect_pressed(move |gesture, _, x, y| {
            if let Some(item) = weak_item.upgrade() {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                on_menu(item.position(), row.upcast_ref(), x, y);
            }
        });
        widgets.root.add_controller(secondary);
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
        bind(&widgets, &data.borrow(), item.position() as usize);
    });
    factory
}

fn bind(widgets: &RowWidgets, data: &TreeData, position: usize) {
    widgets.clear();
    let (Some(row), Some(doc)) = (data.rows.get(position), data.document.as_deref()) else {
        return;
    };
    widgets
        .expander
        .set_margin_start(i32::from(row.depth.min(MAX_INDENT_DEPTH)) * INDENT_PX);
    if let Some(expanded) = data.rows.is_expanded(position, doc) {
        widgets.expander.set_opacity(1.0);
        widgets.expander.set_can_target(true);
        widgets.expander.set_icon_name(Some(if expanded {
            EXPANDED_ICON
        } else {
            COLLAPSED_ICON
        }));
    }
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
