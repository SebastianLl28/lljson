//! Ventana principal: dos paneles lado a lado. Cada panel se llena abriendo un archivo,
//! pegando del portapapeles o arrastrando archivos o texto sobre él. Ambos paneles
//! muestran la misma lista de filas alineadas, que se construye en un hilo de fondo.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use jsonlens_core::{DiffOptions, Direction, Rows, Splice, diff};

use crate::loader::Input;
use crate::pane::{Pane, Side};
use crate::summary::{State, SummaryBar};
use crate::view::{self, Comparison, RowsModel, to_u32};

const DEFAULT_WIDTH: i32 = 1200;
const DEFAULT_HEIGHT: i32 = 760;

/// Atajos de las acciones de la ventana.
pub const ACCELS: &[(&str, &[&str])] = &[
    ("win.open-left", &["<Control>o"]),
    ("win.open-right", &["<Control><Shift>o"]),
    ("win.paste-left", &["<Control>v"]),
    ("win.paste-right", &["<Control><Shift>v"]),
    ("win.next-change", &["F3", "<Alt>Down"]),
    ("win.prev-change", &["<Shift>F3", "<Alt>Up"]),
];

pub struct Window {
    pub window: adw::ApplicationWindow,
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    summary: SummaryBar,
    left: Rc<Pane>,
    right: Rc<Pane>,
    comparison: Rc<RefCell<Comparison>>,
    left_model: RowsModel,
    right_model: RowsModel,
    /// Identifica la última construcción de filas pedida.
    rows_generation: Cell<u64>,
    /// Fila de la última diferencia a la que se navegó (para ir a la siguiente o anterior).
    change_cursor: Cell<Option<usize>>,
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
        let left_model = RowsModel::new();
        let right_model = RowsModel::new();
        let left = Pane::new(
            Side::Left,
            &left_model,
            view::factory(Side::Left, Rc::clone(&comparison), Rc::clone(&on_toggle)),
        );
        let right = Pane::new(
            Side::Right,
            &right_model,
            view::factory(Side::Right, Rc::clone(&comparison), Rc::clone(&on_toggle)),
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

        let title = adw::WindowTitle::new("jsonlens", "");
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        for (icon, action, tooltip) in [
            (
                "go-up-symbolic",
                "win.prev-change",
                "Diferencia anterior (Mayús+F3)",
            ),
            (
                "go-down-symbolic",
                "win.next-change",
                "Diferencia siguiente (F3)",
            ),
        ] {
            header.pack_start(
                &gtk::Button::builder()
                    .icon_name(icon)
                    .action_name(action)
                    .tooltip_text(tooltip)
                    .build(),
            );
        }
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        let summary = SummaryBar::new();
        view.set_content(Some(&toasts));
        view.add_bottom_bar(&summary.root);

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("jsonlens")
            .default_width(DEFAULT_WIDTH)
            .default_height(DEFAULT_HEIGHT)
            .width_request(600)
            .height_request(400)
            .content(&view)
            .build();

        Window {
            window,
            title,
            toasts,
            summary,
            left,
            right,
            comparison,
            left_model,
            right_model,
            rows_generation: Cell::new(0),
            change_cursor: Cell::new(None),
        }
    }

    /// Conecta las acciones y señales; se llama una vez, ya dentro de un `Rc`.
    fn setup(self: &Rc<Self>) {
        let this = self;
        for side in [Side::Left, Side::Right] {
            this.setup_actions(side);
            this.setup_drop(side);
            let weak = Rc::downgrade(this);
            this.pane(side).connect_changed(move || {
                if let Some(this) = weak.upgrade() {
                    this.update_title();
                    this.rebuild_rows();
                }
            });
        }
        this.setup_navigation();
        this.update_title();
        this.sync_scroll();

        // Las acciones solo guardan referencias débiles: la ventana de GTK mantiene vivo
        // este estado hasta que se cierra.
        let keep_alive = RefCell::new(Some(Rc::clone(this)));
        this.window.connect_close_request(move |_| {
            keep_alive.take();
            glib::Propagation::Proceed
        });
    }

    /// Acciones de navegación entre diferencias; desactivadas hasta que haya alguna.
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

    /// Salta a la diferencia siguiente o anterior (dando la vuelta al llegar al final),
    /// expande un contenedor colapsado que la contenga y la deja a la vista en ambos paneles.
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
            self.toast("No hay diferencias");
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
            "diferencia en la fila {} (vuelta: {})",
            stop.position, stop.wrapped
        ));
        self.scroll_to_centered(stop.position);
        if stop.wrapped {
            self.toast(match direction {
                Direction::Forward => "Última diferencia: vuelve a la primera",
                Direction::Backward => "Primera diferencia: vuelve a la última",
            });
        }
    }

    /// Deja la fila `position` más o menos en el centro del panel (el otro panel la sigue).
    /// `GtkListView` solo sabe llevar una fila al borde más cercano, así que se pide ver
    /// una fila a media página de distancia, del lado hacia el que hay que moverse.
    /// Si la fila ya está a la vista no se mueve nada.
    fn scroll_to_centered(&self, position: usize) {
        let adjustment = self.left.scrolled.vadjustment();
        let len = self.comparison.borrow().rows.len().max(1);
        // Las filas miden lo mismo, así que la altura media da su posición.
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

    /// Sincroniza el scroll vertical de ambos paneles. Las dos listas tienen las mismas
    /// filas con la misma altura, así que basta con copiar la posición.
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

    /// Reconstruye las filas alineadas de los documentos actuales en un hilo de fondo.
    fn rebuild_rows(self: &Rc<Self>) {
        let generation = self.rows_generation.get() + 1;
        self.rows_generation.set(generation);
        let left = self.left.document();
        let right = self.right.document();
        let (sender, receiver) = async_channel::bounded(1);
        let spawned = std::thread::Builder::new()
            .name("jsonlens-filas".to_owned())
            .spawn(move || {
                let options = DiffOptions::default();
                let diff = match (left.as_deref(), right.as_deref()) {
                    (Some(l), Some(r)) => Some(diff(l, r, &options)),
                    _ => None,
                };
                let rows = Rows::new(left.as_deref(), right.as_deref(), options);
                let _ = sender.send_blocking(Comparison {
                    left,
                    right,
                    rows,
                    diff,
                });
            });
        if let Err(err) = spawned {
            self.toast(&format!("No se pudo preparar la vista: {err}"));
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
            // El préstamo termina antes de avisar a los modelos: la vista vuelve a leer
            // las filas durante `items_changed`.
            let old = this.comparison.replace(comparison);
            for model in [&this.left_model, &this.right_model] {
                model.splice(0, model.len(), len, len);
            }
            this.summary.update(state);
            this.change_cursor.set(None);
            this.set_navigation_enabled(matches!(state, State::Compared(s) if s.total() > 0));
            this.left.show_list();
            this.right.show_list();
            #[cfg(any(debug_assertions, feature = "devtools"))]
            crate::devtools::rows_ready(len);
            // Los documentos anteriores se liberan aquí, después de actualizar la vista.
            drop(old);
        });
    }

    /// Ajuste vertical del panel izquierdo (herramientas de desarrollo).
    #[cfg(any(debug_assertions, feature = "devtools"))]
    pub fn left_vadjustment(&self) -> gtk::Adjustment {
        self.left.scrolled.vadjustment()
    }

    /// Expande o colapsa la fila `position` en ambos paneles.
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

    /// Carga un documento en un panel (en segundo plano).
    pub fn load(&self, side: Side, input: Input) {
        self.pane(side).load(input);
    }

    fn pane(&self, side: Side) -> &Rc<Pane> {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    fn setup_actions(self: &Rc<Self>, side: Side) {
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
        let json = gtk::FileFilter::new();
        json.set_name(Some("JSON"));
        json.add_mime_type("application/json");
        json.add_suffix("json");
        json.add_suffix("jsonc");
        let all = gtk::FileFilter::new();
        all.set_name(Some("Todos los archivos"));
        all.add_pattern("*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&json);
        filters.append(&all);
        let dialog = gtk::FileDialog::builder()
            .title("Abrir JSON")
            .modal(true)
            .filters(&filters)
            .default_filter(&json)
            .build();

        let window = self.window.clone();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = dialog.open_future(Some(&window)).await;
            let Some(this) = weak.upgrade() else {
                return;
            };
            match result {
                Ok(file) => this.load_file(side, &file),
                Err(err) if err.matches(gtk::DialogError::Dismissed) => {}
                Err(err) => this.toast(&format!("No se pudo abrir el archivo: {err}")),
            }
        });
    }

    fn load_file(&self, side: Side, file: &gio::File) {
        match file.path() {
            Some(path) => self.load(side, Input::File(path)),
            None => self.toast("Solo se pueden abrir archivos locales"),
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
                        name: "Portapapeles".to_owned(),
                        text: text.into(),
                    },
                ),
                Ok(None) => this.toast("El portapapeles no contiene texto"),
                Err(err) => this.toast(&format!("No se pudo leer el portapapeles: {err}")),
            }
        });
    }

    /// Acepta archivos o texto. Si se sueltan dos archivos, el segundo va al otro panel.
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
                        self.toast("Solo se comparan los dos primeros archivos");
                    }
                }
            }
            true
        } else if let Ok(text) = value.get::<String>() {
            self.load(
                side,
                Input::Text {
                    name: "Texto arrastrado".to_owned(),
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

    /// Subtítulo con los nombres de ambos documentos.
    fn update_title(&self) {
        let subtitle = format!(
            "{} ↔ {}",
            self.pane(Side::Left).title(),
            self.pane(Side::Right).title()
        );
        self.title.set_subtitle(&subtitle);
    }
}

/// Nueva posición de una fila tras un expandir o colapsar: las filas que desaparecen
/// pasan a apuntar a la fila que se expandió o colapsó.
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
