//! Panel de un documento: barra con su nombre y acciones, y una pila con sus estados.

use adw::prelude::*;

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
}

pub struct Pane {
    pub root: gtk::Box,
    title: gtk::Label,
}

impl Pane {
    pub fn new(side: Side) -> Pane {
        let open_action = format!("win.open-{}", side.name());
        let paste_action = format!("win.paste-{}", side.name());

        let title = gtk::Label::builder()
            .label("Sin documento")
            .hexpand(true)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["heading"])
            .build();
        let open = gtk::Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text("Abrir archivo")
            .action_name(open_action.as_str())
            .build();
        let paste = gtk::Button::builder()
            .icon_name("edit-paste-symbolic")
            .tooltip_text("Pegar del portapapeles")
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

        let stack = gtk::Stack::builder()
            .vexpand(true)
            .hexpand(true)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        stack.add_named(&empty_page(&open_action, &paste_action), Some("empty"));

        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(280)
            .build();
        root.append(&bar);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&stack);

        Pane { root, title }
    }

    /// Nombre del documento mostrado en la barra del panel.
    pub fn title(&self) -> String {
        self.title.label().into()
    }
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
