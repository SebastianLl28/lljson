//! Tree of the Viewer: a virtualized `GtkListView` over the flat rows from `core`. Only the
//! visible rows have widgets; their text is generated when they are bound.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use lljson_core::{Document, Kind, Query, TreeLabel, TreeRows, tree_text};

use super::validate::Tree;
use crate::nodecopy::{self, Copy};
use crate::search::{SearchState, SearchUi, count_text};
use crate::view::{RowsModel, token_class};

const INDENT_PX: i32 = 16;
/// Indentation stops growing past this depth.
const MAX_INDENT_DEPTH: u16 = 200;

const SEARCH_MATCH_CLASS: &str = "search-match";
const SEARCH_CURRENT_CLASS: &str = "search-current";

const EXPANDED_ICON: &str = "pan-down-symbolic";
const COLLAPSED_ICON: &str = "pan-end-symbolic";

/// Opens the context menu of the row at a position, at a point of the given widget.
type OnMenu = Rc<dyn Fn(u32, &gtk::Widget, f64, f64)>;

#[derive(Default)]
struct TreeData {
    document: Option<Arc<Document>>,
    rows: TreeRows,
    search: SearchState,
}

/// Rebuilds of the whole row list (expand all, collapse all).
#[derive(Clone, Copy)]
enum Rebuild {
    ExpandAll,
    CollapseAll,
}

/// Handles shared by the widgets' signal handlers.
#[derive(Clone)]
struct Ctx {
    model: RowsModel,
    data: Rc<RefCell<TreeData>>,
    list: gtk::ListView,
    search: Rc<SearchUi>,
    /// Id of the latest search request; results of older ones are dropped.
    generation: Rc<Cell<u64>>,
    notify: nodecopy::Notify,
}

pub struct TreeView {
    pub root: gtk::Box,
    ctx: Ctx,
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
            let notify = Rc::clone(&notify);
            Rc::new(move |position, widget, x, y| {
                show_menu(&data, &notify, position as usize, widget, x, y);
            })
        };
        let list = gtk::ListView::builder()
            .model(&gtk::NoSelection::new(Some(model.clone())))
            .factory(&factory(Rc::clone(&data), Rc::clone(&on_toggle), on_menu))
            .css_classes(["tree-view"])
            .build();
        // Double click or Enter on a row.
        list.connect_activate(move |_, position| on_toggle(position));
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list)
            .hexpand(true)
            .vexpand(true)
            .build();
        let search = Rc::new(SearchUi::with_actions(
            "treesearch.next",
            "treesearch.prev",
            "treesearch.close",
        ));
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&scrolled));
        overlay.add_overlay(&search.root);
        overlay.set_clip_overlay(&search.root, true);
        let ctx = Ctx {
            model,
            data,
            list,
            search,
            generation: Rc::new(Cell::new(0)),
            notify,
        };
        let view = TreeView {
            root: gtk::Box::new(gtk::Orientation::Vertical, 0),
            ctx,
        };
        view.root.append(&view.toolbar());
        view.root.append(&overlay);
        view.setup_search();
        view
    }

    /// Wires the search box; the actions live on the tree so that the box can use them.
    fn setup_search(&self) {
        let ctx = &self.ctx;
        let group = gio::SimpleActionGroup::new();
        for (name, forward) in [("next", Some(true)), ("prev", Some(false)), ("close", None)] {
            let action = gio::SimpleAction::new(name, None);
            let ctx = ctx.clone();
            action.connect_activate(move |_, _| match forward {
                Some(forward) => step_match(&ctx, forward),
                None => close_search(&ctx),
            });
            group.add_action(&action);
        }
        self.root.insert_action_group("treesearch", Some(&group));
        let entry = &ctx.search.entry;
        let c = ctx.clone();
        entry.connect_search_changed(move |_| run_search(&c, true));
        let c = ctx.clone();
        entry.connect_activate(move |_| step_match(&c, true));
        // Capture phase: the entry would otherwise also treat Shift+Enter as "activate".
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let c = ctx.clone();
        keys.connect_key_pressed(move |_, key, _, state| {
            let enter = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter);
            if enter && state.contains(gdk::ModifierType::SHIFT_MASK) {
                step_match(&c, false);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        entry.add_controller(keys);
        let c = ctx.clone();
        ctx.search
            .connect_options_changed(move || run_search(&c, true));
        let c = ctx.clone();
        ctx.search.root.connect_reveal_child_notify(move |root| {
            if !root.reveals_child() {
                clear_search(&c);
            }
        });
        let c = ctx.clone();
        entry.connect_stop_search(move |_| close_search(&c));
    }

    /// Shows the search box and focuses it.
    pub fn open_search(&self) {
        let ui = &self.ctx.search;
        ui.entry.set_placeholder_text(Some("Search in the tree…"));
        ui.set_open(true);
        ui.entry.grab_focus();
        ui.entry.select_region(0, -1);
        if !ui.entry.text().is_empty() {
            run_search(&self.ctx, true);
        }
    }

    /// Goes to the next (or previous) result.
    pub fn step_search(&self, forward: bool) {
        step_match(&self.ctx, forward);
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
            let (model, data) = (self.ctx.model.clone(), Rc::clone(&self.ctx.data));
            button.connect_clicked(move |_| rebuild_all(&model, &data, rebuild));
            bar.append(&button);
        }
        let toggle = gtk::ToggleButton::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text("Search keys and values (Ctrl+F)")
            .build();
        self.ctx
            .search
            .root
            .bind_property("reveal-child", &toggle, "active")
            .bidirectional()
            .sync_create()
            .build();
        bar.append(&toggle);
        bar
    }

    /// Replaces the displayed tree.
    pub fn show(&self, tree: Tree) {
        let len = tree.rows.len();
        {
            let mut data = self.ctx.data.borrow_mut();
            data.document = Some(tree.document);
            data.rows = tree.rows;
            // Node ids belong to the old document.
            data.search.clear();
        }
        let ctx = &self.ctx;
        // The borrow is released: the view requests the new rows right away.
        ctx.model.splice(0, ctx.model.len(), len, len);
        if ctx.search.is_open() {
            run_search(ctx, false);
        }
    }

    pub fn clear(&self) {
        {
            let mut data = self.ctx.data.borrow_mut();
            data.document = None;
            data.rows = TreeRows::default();
            data.search.clear();
        }
        self.ctx.generation.set(self.ctx.generation.get() + 1);
        self.ctx.search.set_count("");
        self.ctx.model.splice(0, self.ctx.model.len(), 0, 0);
    }
}

/// Expands or collapses the container at `position`; other rows are left alone.
fn toggle(model: &RowsModel, data: &RefCell<TreeData>, position: usize) {
    let splice = {
        let mut data = data.borrow_mut();
        let TreeData { document, rows, .. } = &mut *data;
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

fn close_search(ctx: &Ctx) {
    ctx.search.set_open(false);
    ctx.list.grab_focus();
}

/// Forgets the results (and any search still running) and redraws the highlighted rows.
fn clear_search(ctx: &Ctx) {
    ctx.generation.set(ctx.generation.get() + 1);
    reset_matches(ctx);
}

fn reset_matches(ctx: &Ctx) {
    ctx.data.borrow_mut().search.clear();
    refresh_visible(ctx);
}

/// Searches the shown document on a background thread; with `jump`, the first result is
/// shown when it arrives.
fn run_search(ctx: &Ctx, jump: bool) {
    let id = ctx.generation.get() + 1;
    ctx.generation.set(id);
    let ui = &ctx.search;
    let text = ui.entry.text().to_string();
    let query = match Query::new(&text, ui.mode(), ui.scope()) {
        Ok(query) => query,
        Err(err) => {
            ui.set_error(Some(&err.to_string()));
            ui.set_count("Invalid regex");
            reset_matches(ctx);
            return;
        }
    };
    ui.set_error(None);
    let document = ctx.data.borrow().document.clone();
    let Some(document) = document.filter(|_| !text.is_empty()) else {
        ui.set_count("");
        reset_matches(ctx);
        return;
    };
    let (sender, receiver) = async_channel::bounded(1);
    let worker = Arc::clone(&document);
    let spawned = std::thread::Builder::new()
        .name("lljson-search".to_owned())
        .spawn(move || {
            let _ = sender.send_blocking(query.find(&worker));
        });
    if let Err(err) = spawned {
        (ctx.notify)(&format!("Could not start the search: {err}"));
        return;
    }
    let ctx = ctx.clone();
    glib::MainContext::default().spawn_local(async move {
        let Ok(matches) = receiver.recv().await else {
            return;
        };
        let current = ctx
            .data
            .borrow()
            .document
            .as_ref()
            .is_some_and(|shown| Arc::ptr_eq(shown, &document));
        if ctx.generation.get() != id || !current {
            return;
        }
        let total = matches.len();
        {
            let mut data = ctx.data.borrow_mut();
            data.search.matches = matches;
            data.search.current = None;
        }
        ctx.search.set_count(&count_text(None, total));
        refresh_visible(&ctx);
        if jump && total > 0 {
            go_to_match(&ctx, 0);
        }
    });
}

fn step_match(ctx: &Ctx, forward: bool) {
    let (current, total) = {
        let data = ctx.data.borrow();
        (data.search.current, data.search.matches.len())
    };
    if total == 0 {
        return;
    }
    let index = match (forward, current) {
        (true, None) => 0,
        (true, Some(i)) => (i + 1) % total,
        (false, None) => total - 1,
        (false, Some(i)) => (i + total - 1) % total,
    };
    go_to_match(ctx, index);
}

/// Makes result `index` the current one: expands what hides it and scrolls to it.
fn go_to_match(ctx: &Ctx, index: usize) {
    let reveal = {
        let mut data = ctx.data.borrow_mut();
        let TreeData {
            document,
            rows,
            search,
        } = &mut *data;
        let (Some(document), Some(&node)) = (document.as_deref(), search.matches.get(index)) else {
            return;
        };
        search.current = Some(index);
        rows.reveal(node, document)
    };
    let total = ctx.data.borrow().search.matches.len();
    ctx.search.set_count(&count_text(Some(index), total));
    let Some(reveal) = reveal else {
        (ctx.notify)("Could not show that result");
        return;
    };
    // The borrow is released: the view requests the changed rows right away.
    for (splice, len) in &reveal.splices {
        ctx.model
            .splice(splice.position, splice.removed, splice.added, *len);
    }
    refresh_visible(ctx);
    let list = ctx.list.clone();
    let position = reveal.position as u32;
    // After layout: rows that were just added have no height yet.
    glib::idle_add_local_once(move || {
        list.scroll_to(position, gtk::ListScrollFlags::NONE, None);
    });
}

/// Rebinds the rows on screen (and a few around them) so highlights follow the search.
fn refresh_visible(ctx: &Ctx) {
    let Some(adjustment) = ctx.list.vadjustment() else {
        return;
    };
    let len = ctx.data.borrow().rows.len();
    if len == 0 || adjustment.upper() <= 0.0 {
        return;
    }
    let row_height = adjustment.upper() / len as f64;
    let first = ((adjustment.value() / row_height) as usize).saturating_sub(4);
    let count = ((adjustment.page_size() / row_height) as usize + 10).min(len - first.min(len));
    ctx.model.splice(first, count, count, len);
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
        self.root.remove_css_class(SEARCH_MATCH_CLASS);
        self.root.remove_css_class(SEARCH_CURRENT_CLASS);
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
    if data.search.is_match(row.node) {
        widgets.root.add_css_class(SEARCH_MATCH_CLASS);
        if data.search.is_current(row.node) {
            widgets.root.add_css_class(SEARCH_CURRENT_CLASS);
        }
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
