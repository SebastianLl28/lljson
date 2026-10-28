//! Ventana principal: dos paneles lado a lado. Cada panel se llena abriendo un archivo,
//! pegando del portapapeles o arrastrando archivos o texto sobre él.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::loader::Input;
use crate::pane::{Pane, Side};

const DEFAULT_WIDTH: i32 = 1200;
const DEFAULT_HEIGHT: i32 = 760;

/// Atajos de las acciones de la ventana.
pub const ACCELS: &[(&str, &str)] = &[
    ("win.open-left", "<Control>o"),
    ("win.open-right", "<Control><Shift>o"),
    ("win.paste-left", "<Control>v"),
    ("win.paste-right", "<Control><Shift>v"),
];

pub struct Window {
    pub window: adw::ApplicationWindow,
    title: adw::WindowTitle,
    toasts: adw::ToastOverlay,
    left: Rc<Pane>,
    right: Rc<Pane>,
}

impl Window {
    pub fn new(app: &adw::Application) -> Rc<Window> {
        let left = Pane::new(Side::Left);
        let right = Pane::new(Side::Right);

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
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&toasts));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("jsonlens")
            .default_width(DEFAULT_WIDTH)
            .default_height(DEFAULT_HEIGHT)
            .width_request(600)
            .height_request(400)
            .content(&view)
            .build();

        let this = Rc::new(Window {
            window,
            title,
            toasts,
            left,
            right,
        });
        for side in [Side::Left, Side::Right] {
            this.setup_actions(side);
            this.setup_drop(side);
            let weak = Rc::downgrade(&this);
            this.pane(side).connect_changed(move || {
                if let Some(this) = weak.upgrade() {
                    this.update_title();
                }
            });
        }
        this.update_title();

        // Las acciones solo guardan referencias débiles: la ventana de GTK mantiene vivo
        // este estado hasta que se cierra.
        let keep_alive = RefCell::new(Some(Rc::clone(&this)));
        this.window.connect_close_request(move |_| {
            keep_alive.take();
            glib::Propagation::Proceed
        });
        this
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
