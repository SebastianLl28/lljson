//! jsonlens: compara y formatea JSON (GTK4 + libadwaita).

mod loader;
mod pane;
mod window;

use adw::prelude::*;
use gtk::glib;

const APP_ID: &str = "io.github.jsonlens.JsonLens";

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    for (action, accel) in window::ACCELS {
        app.set_accels_for_action(action, &[accel]);
    }
    app.connect_activate(|app| {
        let window = window::Window::new(app);
        window.window.present();
    });
    app.run()
}
