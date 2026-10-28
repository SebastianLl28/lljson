//! Vista virtualizada: un `GListModel` propio sobre la lista plana de filas de `core`
//! y la fábrica que genera el texto de cada fila al mostrarse (en el `bind`).
//! Solo existen widgets para las filas visibles; `GtkListView` los recicla.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use jsonlens_core::{Cell, Document, Rows, TokenKind, cell_text};

use crate::pane::Side;

/// Sangría por nivel de anidamiento, en píxeles.
const INDENT_PX: i32 = 16;
/// A partir de esta profundidad la sangría deja de crecer.
const MAX_INDENT_DEPTH: u16 = 200;

const STYLE: &str = include_str!("style.css");
const STYLE_DARK: &str = include_str!("style-dark.css");

/// Carga los estilos de la vista y cambia la paleta de sintaxis con el tema del sistema.
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

/// Documentos que se muestran y sus filas alineadas (construidas para esos documentos).
#[derive(Default)]
pub struct Comparison {
    pub left: Option<Arc<Document>>,
    pub right: Option<Arc<Document>>,
    pub rows: Rows,
}

impl Comparison {
    fn document(&self, side: Side) -> Option<&Document> {
        match side {
            Side::Left => self.left.as_deref(),
            Side::Right => self.right.as_deref(),
        }
    }

    fn cell(&self, side: Side, position: usize) -> Option<Cell> {
        let row = self.rows.get(position)?;
        match side {
            Side::Left => row.left(),
            Side::Right => row.right(),
        }
    }

    /// `Some(expandido)` si la fila muestra un contenedor que se puede expandir o colapsar.
    fn is_expanded(&self, position: usize) -> Option<bool> {
        self.rows
            .is_expanded(position, self.left.as_deref(), self.right.as_deref())
    }
}

mod imp {
    use std::cell::Cell;

    use super::*;

    /// Solo guarda el número de filas: los elementos se crean bajo demanda y el
    /// contenido de cada fila se lee de [`Comparison`] en el `bind`.
    #[derive(Default)]
    pub struct RowsModel {
        pub len: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RowsModel {
        const NAME: &'static str = "JsonlensRowsModel";
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

    /// Avisa a la vista de que en `position` se quitaron `removed` filas y se pusieron
    /// `added`. Debe llamarse sin ningún préstamo activo de [`Comparison`], porque la
    /// vista vuelve a pedir las filas de inmediato.
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

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Widgets de una fila, creados una vez por fila visible y reutilizados.
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

    /// Recupera los widgets a partir de la raíz creada por [`RowWidgets::new`].
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
        self.root.set_margin_start(0);
    }
}

/// Fábrica de filas de un lado. `on_toggle` recibe la posición de la fila a expandir o
/// colapsar (clic en el triángulo).
pub fn factory(
    side: Side,
    comparison: Rc<RefCell<Comparison>>,
    on_toggle: Rc<dyn Fn(u32)>,
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
        // Hueco de relleno: el otro panel tiene contenido en esta fila.
        if position < comparison.rows.len() {
            widgets.root.add_css_class("json-filler");
        }
        return;
    };
    let depth = comparison.rows.get(position).map_or(0, |r| r.depth());
    widgets
        .root
        .set_margin_start(i32::from(depth.min(MAX_INDENT_DEPTH)) * INDENT_PX);

    if let Some(expanded) = comparison.is_expanded(position) {
        widgets.expander.set_opacity(1.0);
        widgets.expander.set_can_target(true);
        widgets.expander.set_icon_name(Some(if expanded {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        }));
    }
    let text = cell_text(doc, cell);
    if let Some(key) = &text.key {
        widgets.key.set_label(&format!("{key}: "));
    }
    widgets.value.set_label(&text.value);
    widgets
        .value
        .set_css_classes(&[token_class(text.value_kind)]);
    if text.comma {
        widgets.comma.set_label(",");
    }
    if let Some(summary) = &text.summary {
        widgets.summary.set_label(summary);
    }
}
