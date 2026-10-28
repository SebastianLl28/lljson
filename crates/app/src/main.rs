use adw::prelude::*;
use gtk::glib;

const APP_ID: &str = "io.github.jsonlens.JsonLens";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_window);
    app.run()
}

fn build_window(app: &adw::Application) {
    let status = adw::StatusPage::builder()
        .icon_name("text-x-generic-symbolic")
        .title("jsonlens")
        .description(format!("Hola mundo · core {}", jsonlens_core::version()))
        .vexpand(true)
        .build();

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&status));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("jsonlens")
        .default_width(900)
        .default_height(600)
        .content(&view)
        .build();
    window.present();
}
