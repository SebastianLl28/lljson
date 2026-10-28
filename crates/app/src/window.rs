//! Ventana principal: dos paneles lado a lado.

use std::rc::Rc;

use crate::pane::{Pane, Side};

const DEFAULT_WIDTH: i32 = 1200;
const DEFAULT_HEIGHT: i32 = 760;

pub struct Window {
    pub window: adw::ApplicationWindow,
    title: adw::WindowTitle,
    left: Pane,
    right: Pane,
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

        let title = adw::WindowTitle::new("jsonlens", "");
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&paned));

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
            left,
            right,
        });
        this.update_title();
        this
    }

    fn pane(&self, side: Side) -> &Pane {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
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
