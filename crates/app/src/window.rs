//! Main window: two side-by-side panes sharing one list of aligned rows, which is built
//! on a background thread.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use lljson_core::{
    Cell as RowCell, DiffOptions, Direction, Document, Indent, NodeId, Query, RowSide, Rows,
    Splice, diff,
};

use crate::loader::Input;
use crate::nodecopy::{self, Copy};
use crate::options::{self, OptionsText};
use crate::pane::{Pane, Side};
use crate::search::{SearchState, count_text};
use crate::session;
use crate::summary::{State, SummaryBar};
use crate::theme::{self, THEMES};
use crate::transform::Transform;
use crate::view::{self, Comparison, RowsModel, to_u32};
use crate::viewer::ViewerPage;

const DEFAULT_WIDTH: i32 = 1200;
const DEFAULT_HEIGHT: i32 = 760;
const PAGE_COMPARE: &str = "compare";
const PAGE_VIEWER: &str = "viewer";

pub const ACCELS: &[(&str, &[&str])] = &[
    ("win.open-left", &["<Control>o"]),
    ("win.open-right", &["<Control><Shift>o"]),
    ("win.paste-left", &["<Control>v"]),
    ("win.paste-right", &["<Control><Shift>v"]),
    ("win.next-change", &["F3", "<Alt>Down"]),
    ("win.prev-change", &["<Shift>F3", "<Alt>Up"]),
    ("win.hide-equal", &["<Control>h"]),
    ("win.diff-options", &["<Control>comma"]),
    ("win.compare-clipboard", &["<Control><Shift>k"]),
    ("win.search", &["<Control>f"]),
    ("win.search-next", &["<Control>g"]),
    ("win.search-prev", &["<Control><Shift>g"]),
];

/// Shortcuts of the Viewer, active only while its tab is visible.
pub const VIEWER_ACCELS: &[(&str, &[&str])] = &[
    ("win.viewer-open", &["<Control>o"]),
    ("win.viewer-copy-all", &["<Control><Shift>c"]),
    ("win.viewer-format", &["<Control><Shift>f"]),
    ("win.viewer-minify", &["<Control><Shift>m"]),
    ("win.viewer-sort-keys", &["<Control><Shift>s"]),
    ("win.viewer-search", &["<Control>f"]),
    ("win.viewer-search-next", &["<Control>g"]),
    ("win.viewer-search-prev", &["<Control><Shift>g"]),
];

pub struct Window {
    pub window: adw::ApplicationWindow,
    stack: adw::ViewStack,
    viewer: Rc<ViewerPage>,
    /// Header widgets that only make sense for the comparison.
    compare_header: Vec<gtk::Widget>,
    toasts: adw::ToastOverlay,
    summary: SummaryBar,
    left: Rc<Pane>,
    right: Rc<Pane>,
    comparison: Rc<RefCell<Comparison>>,
    left_model: RowsModel,
    right_model: RowsModel,
    /// Id of the latest row-build request; stale results are dropped.
    rows_generation: Cell<u64>,
    change_cursor: Cell<Option<usize>>,
    hide_equal: Cell<bool>,
    diff_options: RefCell<DiffOptions>,
    options_text: RefCell<OptionsText>,
    /// Id of the latest search request of each pane; stale results are dropped.
    search_generation: [Cell<u64>; 2],
    /// Pane whose search `Ctrl+G` steps through: the one used last.
    active_search: Cell<Side>,
}

impl Window {
    pub fn new(app: &adw::Application) -> Rc<Window> {
        let this = Rc::new_cyclic(|weak: &Weak<Window>| Window::build(app, weak));
        this.setup();
        this
    }

    fn build(app: &adw::Application, weak: &Weak<Window>) -> Window {
        let comparison = Rc::new(RefCell::new(Comparison::default()));
        let weak_toggle = weak.clone();
        let on_toggle: Rc<dyn Fn(u32)> = Rc::new(move |position| {
            if let Some(this) = weak_toggle.upgrade() {
                this.toggle(position);
            }
        });
        let weak_menu = weak.clone();
        let on_context: view::ContextMenu = Rc::new(move |side, position, widget, x, y| {
            if let Some(this) = weak_menu.upgrade() {
                this.show_node_menu(side, position, widget, x, y);
            }
        });
        let left_model = RowsModel::new();
        let right_model = RowsModel::new();
        let left = Pane::new(
            Side::Left,
            &left_model,
            view::factory(
                Side::Left,
                Rc::clone(&comparison),
                Rc::clone(&on_toggle),
                Rc::clone(&on_context),
            ),
        );
        let right = Pane::new(
            Side::Right,
            &right_model,
            view::factory(
                Side::Right,
                Rc::clone(&comparison),
                Rc::clone(&on_toggle),
                Rc::clone(&on_context),
            ),
        );
        for pane in [&left, &right] {
            let on_toggle = Rc::clone(&on_toggle);
            pane.list
                .connect_activate(move |_, position| on_toggle(position));
        }

        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&left.root)
            .end_child(&right.root)
            .resize_start_child(true)
            .resize_end_child(true)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .wide_handle(true)
            .position(DEFAULT_WIDTH / 2)
            .build();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&paned));

        let viewer = ViewerPage::new();
        let stack = adw::ViewStack::new();
        stack.add_titled_with_icon(&toasts, Some(PAGE_COMPARE), "Compare", "view-dual-symbolic");
        stack.add_titled_with_icon(
            viewer.root(),
            Some(PAGE_VIEWER),
            "Viewer",
            "text-x-generic-symbolic",
        );
        let switcher = adw::ViewSwitcher::builder()
            .stack(&stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .build();
        let header = adw::HeaderBar::builder().title_widget(&switcher).build();
        let mut compare_header: Vec<gtk::Widget> = Vec::new();
        let menu = gio::Menu::new();
        menu.append(
            Some("Compare clipboard with file…"),
            Some("win.compare-clipboard"),
        );
        let themes = gio::Menu::new();
        let stock = gio::Menu::new();
        let palettes = gio::Menu::new();
        for theme in THEMES {
            let section = if theme.stock { &stock } else { &palettes };
            section.append(
                Some(theme.label),
                Some(&format!("win.theme('{}')", theme.id)),
            );
        }
        themes.append_section(None, &stock);
        themes.append_section(None, &palettes);
        menu.append_submenu(Some("Theme"), &themes);
        header.pack_end(
            &gtk::MenuButton::builder()
                .icon_name("open-menu-symbolic")
                .menu_model(&menu)
                .primary(true)
                .tooltip_text("Main menu")
                .build(),
        );
        let options_button = gtk::Button::builder()
            .icon_name("preferences-system-symbolic")
            .action_name("win.diff-options")
            .tooltip_text("Comparison options (Ctrl+,)")
            .build();
        header.pack_end(&options_button);
        compare_header.push(options_button.upcast());
        let hide_equal_button = gtk::ToggleButton::builder()
            .icon_name("view-conceal-symbolic")
            .action_name("win.hide-equal")
            .tooltip_text("Hide equal sections (Ctrl+H)")
            .build();
        header.pack_start(&hide_equal_button);
        compare_header.push(hide_equal_button.upcast());
        for (icon, action, tooltip) in [
            (
                "go-up-symbolic",
                "win.prev-change",
                "Previous difference (Shift+F3)",
            ),
            (
                "go-down-symbolic",
                "win.next-change",
                "Next difference (F3)",
            ),
        ] {
            let button = gtk::Button::builder()
                .icon_name(icon)
                .action_name(action)
                .tooltip_text(tooltip)
                .build();
            header.pack_start(&button);
            compare_header.push(button.upcast());
        }
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        let summary = SummaryBar::new();
        view.set_content(Some(&stack));
        view.add_bottom_bar(&summary.root);

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("lljson")
            .default_width(DEFAULT_WIDTH)
            .default_height(DEFAULT_HEIGHT)
            .width_request(600)
            .height_request(400)
            .content(&view)
            .build();

        Window {
            window,
            stack,
            viewer,
            compare_header,
            toasts,
            summary,
            left,
            right,
            comparison,
            left_model,
            right_model,
            rows_generation: Cell::new(0),
            change_cursor: Cell::new(None),
            hide_equal: Cell::new(false),
            diff_options: RefCell::new(DiffOptions::default()),
            options_text: RefCell::new(OptionsText::default()),
            search_generation: [Cell::new(0), Cell::new(0)],
            active_search: Cell::new(Side::Left),
        }
    }

    fn setup(self: &Rc<Self>) {
        let this = self;
        for side in [Side::Left, Side::Right] {
            this.setup_actions(side);
            this.setup_drop(side);
            let weak = Rc::downgrade(this);
            this.pane(side).connect_changed(move || {
                if let Some(this) = weak.upgrade() {
                    this.update_title();
                    this.update_transform_actions();
                    this.rebuild_rows();
                }
            });
        }
        this.setup_navigation();
        this.setup_hide_equal();
        this.setup_options();
        this.setup_compare_clipboard();
        this.setup_theme();
        this.setup_search();
        this.setup_viewer_actions();
        this.setup_tabs();
        this.update_transform_actions();
        this.update_title();
        this.sync_scroll();

        // Actions only hold weak references; this keeps the state alive until the window closes.
        let keep_alive = RefCell::new(Some(Rc::clone(this)));
        this.window.connect_close_request(move |_| {
            keep_alive.take();
            glib::Propagation::Proceed
        });
    }

    fn setup_navigation(self: &Rc<Self>) {
        for (name, direction) in [
            ("next-change", Direction::Forward),
            ("prev-change", Direction::Backward),
        ] {
            let action = gio::SimpleAction::new(name, None);
            action.set_enabled(false);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(this) = weak.upgrade() {
                    this.go_to_change(direction);
                }
            });
            self.window.add_action(&action);
        }
    }

    fn setup_search(self: &Rc<Self>) {
        let open = gio::SimpleAction::new("search", None);
        let weak = Rc::downgrade(self);
        open.connect_activate(move |_, _| {
            if let Some(this) = weak.upgrade() {
                this.open_search();
            }
        });
        self.window.add_action(&open);
        // Ctrl+G and Ctrl+Shift+G act on the pane searched last.
        for (name, direction) in [
            ("search-next", Direction::Forward),
            ("search-prev", Direction::Backward),
        ] {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(this) = weak.upgrade() {
                    this.step_match(this.active_search.get(), direction);
                }
            });
            self.window.add_action(&action);
        }
        self.setup_search_side(Side::Left);
        self.setup_search_side(Side::Right);
    }

    /// Wires one pane's search box; the two panes never share state.
    fn setup_search_side(self: &Rc<Self>, side: Side) {
        for (name, direction) in [
            ("search-next", Some(Direction::Forward)),
            ("search-prev", Some(Direction::Backward)),
            ("search-close", None),
        ] {
            let action = gio::SimpleAction::new(&format!("{name}-{}", side.name()), None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(this) = weak.upgrade() {
                    match direction {
                        Some(direction) => this.step_match(side, direction),
                        None => this.close_search(side),
                    }
                }
            });
            self.window.add_action(&action);
        }
        let ui = &self.pane(side).search;
        let weak = Rc::downgrade(self);
        ui.entry.connect_search_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.run_search(side, true);
            }
        });
        let weak = Rc::downgrade(self);
        ui.entry.connect_activate(move |_| {
            if let Some(this) = weak.upgrade() {
                this.step_match(side, Direction::Forward);
            }
        });
        // Capture phase: the entry would otherwise also treat Shift+Enter as "activate".
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let enter = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter);
            match weak.upgrade() {
                Some(this) if enter && state.contains(gdk::ModifierType::SHIFT_MASK) => {
                    this.step_match(side, Direction::Backward);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        ui.entry.add_controller(keys);
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        focus.connect_contains_focus_notify(move |focus| {
            if let Some(this) = weak.upgrade()
                && focus.contains_focus()
            {
                this.active_search.set(side);
            }
        });
        ui.entry.add_controller(focus);
        let weak = Rc::downgrade(self);
        ui.connect_options_changed(move || {
            if let Some(this) = weak.upgrade() {
                this.run_search(side, true);
            }
        });
        let weak = Rc::downgrade(self);
        ui.root.connect_reveal_child_notify(move |root| {
            if let Some(this) = weak.upgrade()
                && !root.reveals_child()
            {
                this.clear_search(side);
            }
        });
        let weak = Rc::downgrade(self);
        ui.entry.connect_stop_search(move |_| {
            if let Some(this) = weak.upgrade() {
                this.close_search(side);
            }
        });
    }

    /// Shows the search box of the pane that has the focus (the left one if none).
    fn open_search(self: &Rc<Self>) {
        let focus = RootExt::focus(&self.window);
        let inside = |side: Side| {
            focus
                .as_ref()
                .is_some_and(|w| w.is_ancestor(&self.pane(side).root))
        };
        let side = if inside(Side::Right) {
            Side::Right
        } else if inside(Side::Left) {
            Side::Left
        } else {
            self.active_search.get()
        };
        self.open_search_in(side);
    }

    fn open_search_in(self: &Rc<Self>, side: Side) {
        self.active_search.set(side);
        let ui = &self.pane(side).search;
        ui.entry
            .set_placeholder_text(Some(&format!("Search in {}…", self.pane(side).title())));
        ui.set_open(true);
        ui.entry.grab_focus();
        ui.entry.select_region(0, -1);
        if !ui.entry.text().is_empty() {
            self.run_search(side, true);
        }
    }

    fn close_search(&self, side: Side) {
        self.pane(side).search.set_open(false);
        self.pane(side).list.grab_focus();
    }

    /// Searches `side`'s document on a background thread; with `jump`, the first result is
    /// shown when it arrives.
    fn run_search(self: &Rc<Self>, side: Side, jump: bool) {
        let generation = &self.search_generation[side.index()];
        let id = generation.get() + 1;
        generation.set(id);
        let ui = &self.pane(side).search;
        let text = ui.entry.text().to_string();
        let query = match Query::new(&text, ui.mode(), ui.scope()) {
            Ok(query) => query,
            Err(err) => {
                ui.set_error(Some(&err.to_string()));
                ui.set_count("Invalid regex");
                self.reset_matches(side);
                return;
            }
        };
        ui.set_error(None);
        let document = self.pane(side).document();
        let Some(document) = document.filter(|_| !text.is_empty()) else {
            ui.set_count("");
            self.reset_matches(side);
            return;
        };
        let (sender, receiver) = async_channel::bounded(1);
        let spawned = std::thread::Builder::new()
            .name("lljson-search".to_owned())
            .spawn(move || {
                let _ = sender.send_blocking(query.find(&document));
            });
        if let Err(err) = spawned {
            self.toast(&format!("Could not start the search: {err}"));
            return;
        }
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Ok(matches) = receiver.recv().await else {
                return;
            };
            if let Some(this) = weak.upgrade()
                && this.search_generation[side.index()].get() == id
            {
                this.set_matches(side, matches, jump);
            }
        });
    }

    fn set_matches(self: &Rc<Self>, side: Side, matches: Vec<NodeId>, jump: bool) {
        let total = matches.len();
        {
            let mut comparison = self.comparison.borrow_mut();
            let search = comparison.search_mut(side);
            search.matches = matches;
            search.current = None;
        }
        self.pane(side).search.set_count(&count_text(None, total));
        self.refresh_visible();
        if jump && total > 0 {
            self.go_to_match(side, 0);
        }
    }

    fn reset_matches(&self, side: Side) {
        self.comparison.borrow_mut().search_mut(side).clear();
        self.refresh_visible();
    }

    fn clear_search(&self, side: Side) {
        let generation = &self.search_generation[side.index()];
        generation.set(generation.get() + 1);
        self.reset_matches(side);
    }

    fn step_match(self: &Rc<Self>, side: Side, direction: Direction) {
        let (current, total) = {
            let comparison = self.comparison.borrow();
            let search = comparison.search(side);
            (search.current, search.matches.len())
        };
        if total == 0 {
            return;
        }
        let index = match (direction, current) {
            (Direction::Forward, None) => 0,
            (Direction::Forward, Some(i)) => (i + 1) % total,
            (Direction::Backward, None) => total - 1,
            (Direction::Backward, Some(i)) => (i + total - 1) % total,
        };
        self.go_to_match(side, index);
    }

    /// Makes result `index` of `side` the current one: expands what hides it and scrolls to it.
    fn go_to_match(self: &Rc<Self>, side: Side, index: usize) {
        let (node, total) = {
            let comparison = self.comparison.borrow();
            let search = comparison.search(side);
            (search.matches.get(index).copied(), search.matches.len())
        };
        let Some(node) = node else {
            return;
        };
        let reveal = {
            let mut comparison = self.comparison.borrow_mut();
            comparison.search_mut(side).current = Some(index);
            let Comparison {
                left, right, rows, ..
            } = &mut *comparison;
            let row_side = match side {
                Side::Left => RowSide::Left,
                Side::Right => RowSide::Right,
            };
            rows.reveal(row_side, node, left.as_deref(), right.as_deref())
        };
        self.pane(side)
            .search
            .set_count(&count_text(Some(index), total));
        let Some(reveal) = reveal else {
            self.toast("Could not show that result");
            return;
        };
        for (splice, len) in &reveal.splices {
            self.change_cursor
                .set(self.change_cursor.get().map(|c| shift_cursor(c, *splice)));
            for model in [&self.left_model, &self.right_model] {
                model.splice(splice.position, splice.removed, splice.added, *len);
            }
        }
        self.refresh_visible();
        // After layout: the new rows have no height yet, so positions would be off.
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(this) = weak.upgrade() {
                this.scroll_to_centered(reveal.position);
            }
        });
    }

    /// Rebinds the rows on screen (and a few around them) so highlights follow the search.
    fn refresh_visible(&self) {
        let adjustment = self.left.scrolled.vadjustment();
        let len = self.comparison.borrow().rows.len();
        if len == 0 || adjustment.upper() <= 0.0 {
            return;
        }
        let row_height = adjustment.upper() / len as f64;
        let first = ((adjustment.value() / row_height) as usize).saturating_sub(4);
        let count = ((adjustment.page_size() / row_height) as usize + 10).min(len - first.min(len));
        for model in [&self.left_model, &self.right_model] {
            model.splice(first, count, count, len);
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools"))]
    pub fn search_for_test(self: &Rc<Self>, text: &str, side: Side, scope: u32, mode: u32) {
        let ui = &self.pane(side).search;
        ui.select_options(scope, mode);
        ui.set_open(true);
        ui.entry.set_text(text);
    }

    fn setup_hide_equal(self: &Rc<Self>) {
        let action = gio::SimpleAction::new_stateful("hide-equal", None, &false.to_variant());
        action.set_enabled(false);
        let weak = Rc::downgrade(self);
        action.connect_activate(move |action, _| {
            let Some(this) = weak.upgrade() else {
                return;
            };
            let hide = !this.hide_equal.get();
            this.hide_equal.set(hide);
            action.set_state(&hide.to_variant());
            this.rebuild_rows();
        });
        self.window.add_action(&action);
    }

    fn setup_theme(self: &Rc<Self>) {
        let saved = theme::by_id(&session::load_theme());
        theme::apply(saved);
        let action = gio::SimpleAction::new_stateful(
            "theme",
            Some(glib::VariantTy::STRING),
            &saved.id.to_variant(),
        );
        let weak = Rc::downgrade(self);
        action.connect_activate(move |action, param| {
            let Some(value) = param.and_then(|p| p.str()) else {
                return;
            };
            let chosen = theme::by_id(value);
            theme::apply(chosen);
            action.set_state(&chosen.id.to_variant());
            if let Some(this) = weak.upgrade() {
                this.viewer.refresh_colors();
            }
            if let Err(err) = session::save_theme(chosen.id)
                && let Some(this) = weak.upgrade()
            {
                this.toast(&format!("Could not save the theme: {err}"));
            }
        });
        self.window.add_action(&action);
    }

    fn setup_compare_clipboard(self: &Rc<Self>) {
        let action = gio::SimpleAction::new("compare-clipboard", None);
        let weak = Rc::downgrade(self);
        action.connect_activate(move |_, _| {
            if let Some(this) = weak.upgrade() {
                this.compare_clipboard_with_file();
            }
        });
        self.window.add_action(&action);
    }

    fn setup_options(self: &Rc<Self>) {
        let action = gio::SimpleAction::new("diff-options", None);
        let weak = Rc::downgrade(self);
        action.connect_activate(move |_, _| {
            let Some(this) = weak.upgrade() else {
                return;
            };
            let current = this.options_text.borrow().clone();
            let weak = Rc::downgrade(&this);
            options::present(&this.window, &current, move |text, options| {
                let Some(this) = weak.upgrade() else {
                    return;
                };
                if *this.options_text.borrow() == text {
                    return;
                }
                this.options_text.replace(text);
                this.diff_options.replace(options);
                this.rebuild_rows();
            });
        });
        self.window.add_action(&action);
    }

    fn set_hide_equal_enabled(&self, enabled: bool) {
        if let Some(action) = self
            .window
            .lookup_action("hide-equal")
            .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
        {
            action.set_enabled(enabled);
        }
    }

    fn set_navigation_enabled(&self, enabled: bool) {
        for name in ["next-change", "prev-change"] {
            if let Some(action) = self
                .window
                .lookup_action(name)
                .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
            {
                action.set_enabled(enabled);
            }
        }
    }

    /// Wraps around at the ends and expands a collapsed container holding the difference.
    fn go_to_change(&self, direction: Direction) {
        let found = {
            let comparison = self.comparison.borrow();
            let Some(diff) = &comparison.diff else {
                return;
            };
            comparison.rows.find_change(
                self.change_cursor.get(),
                direction,
                comparison.left.as_deref(),
                comparison.right.as_deref(),
                diff,
            )
        };
        let Some(stop) = found else {
            self.toast("No differences");
            return;
        };
        self.change_cursor.set(Some(stop.position));
        if self
            .comparison
            .borrow()
            .is_collapsed_with_changes(stop.position)
        {
            self.toggle(to_u32(stop.position));
        }
        #[cfg(any(debug_assertions, feature = "devtools"))]
        crate::devtools::trace(&format!(
            "difference at row {} (wrapped: {})",
            stop.position, stop.wrapped
        ));
        self.scroll_to_centered(stop.position);
        if stop.wrapped {
            self.toast(match direction {
                Direction::Forward => "Last difference: wrapping to the first",
                Direction::Backward => "First difference: wrapping to the last",
            });
        }
    }

    /// `GtkListView` only scrolls a row to the nearest edge, so ask to reveal a row half a
    /// page away on the side we move towards. Does nothing if the row is already visible.
    fn scroll_to_centered(&self, position: usize) {
        let adjustment = self.left.scrolled.vadjustment();
        let len = self.comparison.borrow().rows.len().max(1);
        // All rows have the same height, so the mean height gives a row position.
        let row_height = if adjustment.upper() > 0.0 {
            adjustment.upper() / len as f64
        } else {
            1.0
        };
        let half = (adjustment.page_size() / 2.0 / row_height) as usize;
        let top = position as f64 * row_height;
        let anchor = if top < adjustment.value() {
            position.saturating_sub(half)
        } else if top + row_height > adjustment.value() + adjustment.page_size() {
            (position + half).min(len - 1)
        } else {
            return;
        };
        self.left
            .list
            .scroll_to(to_u32(anchor), gtk::ListScrollFlags::NONE, None);
    }

    /// Both lists have identical row heights, so copying the value is enough.
    fn sync_scroll(&self) {
        let syncing = Rc::new(Cell::new(false));
        let left = self.left.scrolled.vadjustment();
        let right = self.right.scrolled.vadjustment();
        for (from, to) in [(&left, &right), (&right, &left)] {
            let to = to.clone();
            let syncing = Rc::clone(&syncing);
            from.connect_value_changed(move |from| {
                if !syncing.replace(true) {
                    to.set_value(from.value());
                    syncing.set(false);
                }
            });
        }
    }

    fn rebuild_rows(self: &Rc<Self>) {
        let generation = self.rows_generation.get() + 1;
        self.rows_generation.set(generation);
        let left = self.left.document();
        let right = self.right.document();
        let hide_equal = self.hide_equal.get();
        let options = self.diff_options.borrow().clone();
        let (sender, receiver) = async_channel::bounded(1);
        let spawned = std::thread::Builder::new()
            .name("lljson-rows".to_owned())
            .spawn(move || {
                let diff = match (left.as_deref(), right.as_deref()) {
                    (Some(l), Some(r)) => Some(diff(l, r, &options)),
                    _ => None,
                };
                let rows = match (&diff, left.as_deref(), right.as_deref()) {
                    (Some(d), Some(l), Some(r)) if hide_equal => {
                        Rows::with_equal_collapsed(l, r, options, d)
                    }
                    _ => Rows::new(left.as_deref(), right.as_deref(), options),
                };
                let _ = sender.send_blocking(Comparison {
                    left,
                    right,
                    rows,
                    diff,
                    search: [SearchState::default(), SearchState::default()],
                });
            });
        if let Err(err) = spawned {
            self.toast(&format!("Could not prepare the view: {err}"));
            return;
        }
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Ok(comparison) = receiver.recv().await else {
                return;
            };
            let Some(this) = weak.upgrade() else {
                return;
            };
            if this.rows_generation.get() != generation {
                return;
            }
            let len = comparison.rows.len();
            let state = comparison
                .diff
                .as_ref()
                .map_or(State::Incomplete, |d| State::Compared(d.summary()));
            // The borrow ends before notifying the models: the view re-reads rows during
            // `items_changed`.
            let old = this.comparison.replace(comparison);
            // Removing millions of rows with the view attached takes seconds in GTK:
            // detach the model, resize it, then reattach.
            for (pane, model) in [
                (&this.left, &this.left_model),
                (&this.right, &this.right_model),
            ] {
                pane.list.set_model(None::<&gtk::SelectionModel>);
                model.splice(0, model.len(), len, len);
                pane.list
                    .set_model(Some(&gtk::NoSelection::new(Some(model.clone()))));
            }
            this.summary.update(state);
            this.change_cursor.set(None);
            this.set_hide_equal_enabled(matches!(state, State::Compared(_)));
            this.set_navigation_enabled(matches!(state, State::Compared(s) if s.total() > 0));
            this.left.show_list();
            this.right.show_list();
            for side in [Side::Left, Side::Right] {
                if this.pane(side).search.is_open() {
                    this.run_search(side, false);
                }
            }
            #[cfg(any(debug_assertions, feature = "devtools"))]
            crate::devtools::rows_ready(len);
            // Free the previous documents only after the view is updated.
            drop(old);
        });
    }

    #[cfg(any(debug_assertions, feature = "devtools"))]
    pub fn set_options_text(self: &Rc<Self>, text: OptionsText) {
        match options::parse(&text) {
            Ok(options) => {
                self.options_text.replace(text);
                self.diff_options.replace(options);
                self.rebuild_rows();
            }
            Err(errors) => self.toast(&errors.join(" · ")),
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools"))]
    pub fn left_vadjustment(&self) -> gtk::Adjustment {
        self.left.scrolled.vadjustment()
    }

    /// `None` for filler rows.
    fn node_at(&self, side: Side, position: u32) -> Option<(RowCell, Arc<Document>)> {
        let comparison = self.comparison.borrow();
        let cell = comparison.cell(side, position as usize)?;
        let doc = match side {
            Side::Left => comparison.left.clone(),
            Side::Right => comparison.right.clone(),
        }?;
        Some((cell, doc))
    }

    pub fn copy_node(self: &Rc<Self>, side: Side, position: u32, what: Copy) {
        let Some((cell, doc)) = self.node_at(side, position) else {
            return;
        };
        let weak = Rc::downgrade(self);
        let notify: nodecopy::Notify = Rc::new(move |message| {
            if let Some(this) = weak.upgrade() {
                this.toast(message);
            }
        });
        nodecopy::copy(&self.window.clipboard(), &doc, cell.node, what, &notify);
    }

    pub fn show_node_menu(
        self: &Rc<Self>,
        side: Side,
        position: u32,
        widget: &gtk::Widget,
        x: f64,
        y: f64,
    ) {
        // A filler row has no node on this side.
        if self.node_at(side, position).is_none() {
            return;
        }
        let weak = Rc::downgrade(self);
        nodecopy::show_menu(widget, x, y, move |what: Copy| {
            if let Some(this) = weak.upgrade() {
                this.copy_node(side, position, what);
            }
        });
    }

    #[cfg(any(debug_assertions, feature = "devtools"))]
    pub fn list(&self, side: Side) -> gtk::ListView {
        self.pane(side).list.clone()
    }

    pub fn toggle(&self, position: u32) {
        let (splice, len) = {
            let mut comparison = self.comparison.borrow_mut();
            let Comparison {
                left, right, rows, ..
            } = &mut *comparison;
            let splice = rows.toggle(position as usize, left.as_deref(), right.as_deref());
            (splice, rows.len())
        };
        if let Some(s) = splice {
            self.change_cursor
                .set(self.change_cursor.get().map(|c| shift_cursor(c, s)));
            for model in [&self.left_model, &self.right_model] {
                model.splice(s.position, s.removed, s.added, len);
            }
        }
    }

    pub fn load(&self, side: Side, input: Input) {
        self.pane(side).load(input);
    }

    fn pane(&self, side: Side) -> &Rc<Pane> {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    fn setup_transforms(self: &Rc<Self>, side: Side) {
        let format = gio::SimpleAction::new(
            &format!("format-{}", side.name()),
            Some(glib::VariantTy::STRING),
        );
        let weak = Rc::downgrade(self);
        format.connect_activate(move |_, param| {
            let transform = param
                .and_then(|p| p.str())
                .and_then(Transform::format_from_param);
            if let (Some(this), Some(transform)) = (weak.upgrade(), transform) {
                this.transform(side, transform);
            }
        });
        self.window.add_action(&format);
        for (name, transform) in [("minify", Transform::Minify), ("sort", Transform::SortKeys)] {
            let action = gio::SimpleAction::new(&format!("{name}-{}", side.name()), None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(this) = weak.upgrade() {
                    this.transform(side, transform);
                }
            });
            self.window.add_action(&action);
        }
    }

    fn setup_copy_text(self: &Rc<Self>, side: Side) {
        let action = gio::SimpleAction::new(&format!("copy-text-{}", side.name()), None);
        let weak = Rc::downgrade(self);
        action.connect_activate(move |_, _| {
            let Some(this) = weak.upgrade() else {
                return;
            };
            let Some(doc) = this.pane(side).document() else {
                return;
            };
            let weak_toast = Rc::downgrade(&this);
            let notify: nodecopy::Notify = Rc::new(move |message| {
                if let Some(this) = weak_toast.upgrade() {
                    this.toast(message);
                }
            });
            nodecopy::copy_in_background(
                &this.window.clipboard(),
                &notify,
                move || nodecopy::document_text(&doc),
                "Document text copied",
            );
        });
        self.window.add_action(&action);
    }

    fn update_transform_actions(&self) {
        for side in [Side::Left, Side::Right] {
            let enabled = self.pane(side).document().is_some();
            for name in ["format", "minify", "sort", "copy-text"] {
                if let Some(action) = self
                    .window
                    .lookup_action(&format!("{name}-{}", side.name()))
                    .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
                {
                    action.set_enabled(enabled);
                }
            }
        }
    }

    /// The text is generated on a background thread and reloaded normally; the result is
    /// dropped if the pane received another document meanwhile.
    pub fn transform(self: &Rc<Self>, side: Side, transform: Transform) {
        let pane = Rc::clone(self.pane(side));
        let Some(doc) = pane.document() else {
            return;
        };
        let name = pane.title();
        let (sender, receiver) = async_channel::bounded(1);
        let worker = Arc::clone(&doc);
        let spawned = std::thread::Builder::new()
            .name("lljson-format".to_owned())
            .spawn(move || {
                let _ = sender.send_blocking(transform.apply(&worker));
            });
        if let Err(err) = spawned {
            self.toast(&format!("Could not run the operation: {err}"));
            return;
        }
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Ok(text) = receiver.recv().await else {
                return;
            };
            let Some(this) = weak.upgrade() else {
                return;
            };
            if !pane.document().is_some_and(|d| Arc::ptr_eq(&d, &doc)) {
                return;
            }
            pane.load(Input::Text { name, text });
            this.toast(transform.done_message());
        });
    }

    fn setup_actions(self: &Rc<Self>, side: Side) {
        self.setup_transforms(side);
        self.setup_copy_text(side);
        let open = gio::SimpleAction::new(&format!("open-{}", side.name()), None);
        let weak = Rc::downgrade(self);
        open.connect_activate(move |_, _| {
            if let Some(this) = weak.upgrade() {
                this.open_file(side);
            }
        });
        self.window.add_action(&open);

        let paste = gio::SimpleAction::new(&format!("paste-{}", side.name()), None);
        let weak = Rc::downgrade(self);
        paste.connect_activate(move |_, _| {
            if let Some(this) = weak.upgrade() {
                this.paste(side);
            }
        });
        self.window.add_action(&paste);
    }

    fn open_file(self: &Rc<Self>, side: Side) {
        self.choose_file(move |this, file| this.load_file(side, &file));
    }

    fn choose_file(self: &Rc<Self>, on_file: impl FnOnce(&Rc<Window>, gio::File) + 'static) {
        let dialog = json_file_dialog();

        let window = self.window.clone();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = dialog.open_future(Some(&window)).await;
            let Some(this) = weak.upgrade() else {
                return;
            };
            match result {
                Ok(file) => on_file(&this, file),
                Err(err) if err.matches(gtk::DialogError::Dismissed) => {}
                Err(err) => this.toast(&format!("Could not open the file: {err}")),
            }
        });
    }

    /// The clipboard is read before opening the chooser to report missing text right away.
    fn compare_clipboard_with_file(self: &Rc<Self>) {
        self.select_tab(PAGE_COMPARE);
        self.read_clipboard(|this, text| {
            this.choose_file(move |this, file| this.compare_text_with_file(text, &file));
        });
    }

    fn read_clipboard(self: &Rc<Self>, then: impl FnOnce(&Rc<Window>, String) + 'static) {
        let clipboard = self.window.clipboard();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = clipboard.read_text_future().await;
            let Some(this) = weak.upgrade() else {
                return;
            };
            match result {
                Ok(Some(text)) => then(&this, text.into()),
                Ok(None) => this.toast("The clipboard contains no text"),
                Err(err) => this.toast(&format!("Could not read the clipboard: {err}")),
            }
        });
    }

    fn compare_text_with_file(&self, text: String, file: &gio::File) {
        self.load_file(Side::Left, file);
        self.load(
            Side::Right,
            Input::Text {
                name: "Clipboard".to_owned(),
                text,
            },
        );
    }

    #[cfg(any(debug_assertions, feature = "devtools"))]
    pub fn compare_clipboard_with_path(self: &Rc<Self>, path: &std::path::Path) {
        let file = gio::File::for_path(path);
        self.read_clipboard(move |this, text| this.compare_text_with_file(text, &file));
    }

    fn load_file(&self, side: Side, file: &gio::File) {
        match file.path() {
            Some(path) => self.load(side, Input::File(path)),
            None => self.toast("Only local files can be opened"),
        }
    }

    fn paste(self: &Rc<Self>, side: Side) {
        let clipboard = self.window.clipboard();
        let weak: Weak<Window> = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = clipboard.read_text_future().await;
            let Some(this) = weak.upgrade() else {
                return;
            };
            match result {
                Ok(Some(text)) => this.load(
                    side,
                    Input::Text {
                        name: "Clipboard".to_owned(),
                        text: text.into(),
                    },
                ),
                Ok(None) => this.toast("The clipboard contains no text"),
                Err(err) => this.toast(&format!("Could not read the clipboard: {err}")),
            }
        });
    }

    /// Two dropped files go to both panes: the second one to the other pane.
    fn setup_drop(self: &Rc<Self>, side: Side) {
        let target = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY);
        target.set_types(&[gdk::FileList::static_type(), String::static_type()]);
        let weak = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            weak.upgrade()
                .is_some_and(|this| this.drop_value(side, value))
        });
        self.pane(side).root.add_controller(target);
    }

    fn drop_value(&self, side: Side, value: &glib::Value) -> bool {
        if let Ok(list) = value.get::<gdk::FileList>() {
            let files = list.files();
            match files.as_slice() {
                [] => return false,
                [file] => self.load_file(side, file),
                [first, second, rest @ ..] => {
                    self.load_file(side, first);
                    self.load_file(side.other(), second);
                    if !rest.is_empty() {
                        self.toast("Only the first two files are compared");
                    }
                }
            }
            true
        } else if let Ok(text) = value.get::<String>() {
            self.load(
                side,
                Input::Text {
                    name: "Dropped text".to_owned(),
                    text,
                },
            );
            true
        } else {
            false
        }
    }

    fn toast(&self, message: &str) {
        self.toasts.add_toast(adw::Toast::new(message));
    }

    fn update_title(&self) {
        let title = format!(
            "lljson — {} ↔ {}",
            self.pane(Side::Left).title(),
            self.pane(Side::Right).title()
        );
        self.window.set_title(Some(&title));
    }

    fn setup_viewer_actions(self: &Rc<Self>) {
        let open = gio::SimpleAction::new("viewer-open", None);
        let weak = Rc::downgrade(self);
        open.connect_activate(move |_, _| {
            if let Some(this) = weak.upgrade() {
                this.viewer.open_dialog(&this.window);
            }
        });
        self.window.add_action(&open);
        let copy = gio::SimpleAction::new("viewer-copy-all", None);
        let weak = Rc::downgrade(self);
        copy.connect_activate(move |_, _| {
            if let Some(this) = weak.upgrade() {
                this.viewer.copy_all();
            }
        });
        self.window.add_action(&copy);
        for (name, step) in [
            ("viewer-search", None),
            ("viewer-search-next", Some(true)),
            ("viewer-search-prev", Some(false)),
        ] {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(this) = weak.upgrade() {
                    match step {
                        Some(forward) => this.viewer.step_search(forward),
                        None => this.viewer.open_search(),
                    }
                }
            });
            self.window.add_action(&action);
        }
        for (name, transform) in [
            ("viewer-format", Transform::Format(Indent::Spaces(2))),
            ("viewer-minify", Transform::Minify),
            ("viewer-sort-keys", Transform::SortKeys),
        ] {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(this) = weak.upgrade() {
                    this.viewer.reformat(transform);
                }
            });
            self.window.add_action(&action);
        }
    }

    #[cfg(any(debug_assertions, feature = "devtools"))]
    pub fn open_in_viewer(&self, path: std::path::PathBuf) {
        self.select_tab(PAGE_VIEWER);
        self.viewer.open_path(path);
    }

    #[cfg(any(debug_assertions, feature = "devtools"))]
    pub fn probe_viewer(&self, path: std::path::PathBuf) {
        self.select_tab(PAGE_VIEWER);
        crate::viewer::probe::run(&self.viewer, &self.window, path);
    }

    fn setup_tabs(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.stack.connect_visible_child_name_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.apply_page();
            }
        });
        self.apply_page();
    }

    fn viewer_visible(&self) -> bool {
        self.stack.visible_child_name().as_deref() == Some(PAGE_VIEWER)
    }

    /// Shows only the header widgets, summary bar and shortcuts of the visible page. The
    /// comparison shortcuts are released while the Viewer is shown so that keys such as
    /// `Ctrl+V` reach its text editor; the comparison actions themselves stay untouched.
    fn apply_page(&self) {
        let viewer = self.viewer_visible();
        for widget in &self.compare_header {
            widget.set_visible(!viewer);
        }
        self.summary.root.set_visible(!viewer);
        if let Some(app) = self.window.application() {
            for (action, accels) in ACCELS {
                let accels: &[&str] = if viewer { &[] } else { accels };
                app.set_accels_for_action(action, accels);
            }
            for (action, accels) in VIEWER_ACCELS {
                let accels: &[&str] = if viewer { accels } else { &[] };
                app.set_accels_for_action(action, accels);
            }
        }
    }

    /// Selects the `compare` or `viewer` tab; unknown names are ignored.
    pub fn select_tab(&self, name: &str) {
        if matches!(name, PAGE_COMPARE | PAGE_VIEWER) {
            self.stack.set_visible_child_name(name);
        }
    }
}

/// The file chooser used to open JSON in either tab.
pub fn json_file_dialog() -> gtk::FileDialog {
    let json = gtk::FileFilter::new();
    json.set_name(Some("JSON"));
    json.add_mime_type("application/json");
    json.add_suffix("json");
    json.add_suffix("jsonc");
    let all = gtk::FileFilter::new();
    all.set_name(Some("All files"));
    all.add_pattern("*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&json);
    filters.append(&all);
    gtk::FileDialog::builder()
        .title("Open JSON")
        .modal(true)
        .filters(&filters)
        .default_filter(&json)
        .build()
}

/// Rows that disappear in a splice end up pointing at the toggled row.
fn shift_cursor(cursor: usize, splice: Splice) -> usize {
    if cursor < splice.position + splice.removed.min(1) {
        cursor
    } else if cursor >= splice.position + splice.removed {
        cursor + splice.added - splice.removed
    } else {
        splice.position
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn splice(position: usize, removed: usize, added: usize) -> Splice {
        Splice {
            position,
            removed,
            added,
        }
    }

    #[test]
    fn cursor_before_the_splice_does_not_move() {
        assert_eq!(shift_cursor(2, splice(5, 1, 10)), 2);
        assert_eq!(shift_cursor(5, splice(5, 1, 10)), 5);
    }

    #[test]
    fn cursor_after_the_splice_shifts_by_the_difference() {
        assert_eq!(shift_cursor(6, splice(5, 1, 10)), 15);
        assert_eq!(shift_cursor(20, splice(5, 10, 1)), 11);
    }

    #[test]
    fn cursor_inside_collapsed_rows_goes_to_the_collapsed_row() {
        assert_eq!(shift_cursor(7, splice(5, 10, 1)), 5);
        assert_eq!(shift_cursor(14, splice(5, 10, 1)), 5);
    }
}
