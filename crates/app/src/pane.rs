//! Panel de un documento: barra con su nombre y acciones, y una pila con sus estados
//! (vacío, cargando, cargado, error). La carga ocurre en un hilo de fondo (ver `loader`).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

use crate::loader::{self, Input, LoadError, LoadResult, Loaded};

/// Si una carga tarda más que esto, se muestra el indicador de progreso.
const PROGRESS_DELAY: Duration = Duration::from_millis(150);

/// Lado del panel. Da nombre a sus acciones (`win.open-left`, `win.paste-right`…).
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
    pub root: gtk::Box,
    title: gtk::Label,
    stack: gtk::Stack,
    info: adw::StatusPage,
    error: adw::StatusPage,
    loading_label: gtk::Label,
    document: RefCell<Option<Loaded>>,
    /// Identifica la última carga pedida: los resultados de cargas anteriores se descartan.
    generation: Cell<u64>,
    loading: Cell<bool>,
    /// Se llama cuando cambia el documento o su nombre.
    on_changed: RefCell<Option<Box<dyn Fn()>>>,
}

impl Pane {
    pub fn new(side: Side) -> Rc<Pane> {
        let open_action = format!("win.open-{}", side.name());
        let paste_action = format!("win.paste-{}", side.name());
        let shift = if side == Side::Right { "Mayús+" } else { "" };

        let title = gtk::Label::builder()
            .label("Sin documento")
            .hexpand(true)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["heading"])
            .build();
        let open = gtk::Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text(format!("Abrir archivo (Ctrl+{shift}O)"))
            .action_name(open_action.as_str())
            .build();
        let paste = gtk::Button::builder()
            .icon_name("edit-paste-symbolic")
            .tooltip_text(format!("Pegar del portapapeles (Ctrl+{shift}V)"))
            .action_name(paste_action.as_str())
            .build();
        let bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .css_classes(["toolbar"])
            .build();
        bar.append(&title);
        bar.append(&open);
        bar.append(&paste);

        // Provisional hasta la Fase 6 (vista virtualizada): resumen del documento cargado.
        let info = adw::StatusPage::builder()
            .icon_name("emblem-ok-symbolic")
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
        stack.add_named(&info, Some("loaded"));
        stack.add_named(&error, Some("error"));

        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(280)
            .build();
        root.append(&bar);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&stack);

        Rc::new(Pane {
            root,
            title,
            stack,
            info,
            error,
            loading_label,
            document: RefCell::new(None),
            generation: Cell::new(0),
            loading: Cell::new(false),
            on_changed: RefCell::new(None),
        })
    }

    /// Nombre del documento mostrado en la barra del panel.
    pub fn title(&self) -> String {
        self.title.label().into()
    }

    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        self.on_changed.replace(Some(Box::new(f)));
    }

    /// Carga un documento en segundo plano. Una carga nueva invalida la anterior.
    /// Mientras tanto se sigue mostrando el contenido actual; si la carga tarda más de
    /// [`PROGRESS_DELAY`], se muestra el indicador de progreso.
    pub fn load(self: &Rc<Self>, input: Input) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.loading.set(true);
        let name = input.name();
        let receiver = loader::spawn_load(input);

        let pane: Weak<Pane> = Rc::downgrade(self);
        let label = format!("Cargando {name}…");
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
            // El canal solo se cierra sin resultado si el hilo de carga entró en pánico.
            let result = result.unwrap_or_else(|_| Err(LoadError::Interrupted { name }));
            pane.finish(result);
        });
    }

    fn finish(&self, result: LoadResult) {
        self.loading.set(false);
        match result {
            Ok(loaded) => {
                self.title.set_label(&loaded.name);
                self.info.set_title(&format!(
                    "{} nodos",
                    group_digits(loaded.document.node_count())
                ));
                self.info.set_description(Some(&describe(&loaded)));
                self.stack.set_visible_child_name("loaded");
                self.document.replace(Some(loaded));
            }
            Err(err) => {
                self.title.set_label(err.name());
                self.error
                    .set_title(&format!("No se pudo cargar {}", err.name()));
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
    let bytes = loaded.document.source_bytes().len();
    let mut text = format!(
        "{} · leído y analizado en {} ms",
        format_bytes(bytes),
        loaded.elapsed.as_millis()
    );
    if loaded.fixes > 0 {
        text.push_str(&format!(
            "\nJSONC: {} comentarios o comas finales aceptados",
            loaded.fixes
        ));
    }
    text
}

/// Tamaño legible con coma decimal: `512 B`, `3,4 KiB`, `20,7 MiB`.
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
        format!("{value:.1} {}", UNITS[unit]).replace('.', ",")
    }
}

/// Separa los miles con espacios finos: `1 000 009`.
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
        .label("Abrir…")
        .action_name(open_action)
        .css_classes(["pill", "suggested-action"])
        .build();
    let paste = gtk::Button::builder()
        .label("Pegar")
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
        .title("Arrastra un JSON aquí")
        .description("o ábrelo desde un archivo o pégalo del portapapeles")
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
        assert_eq!(format_bytes(1024), "1,0 KiB");
        assert_eq!(format_bytes(21_705_000), "20,7 MiB");
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
