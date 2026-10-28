//! jsonlens: compara y formatea JSON (GTK4 + libadwaita).

mod args;
mod loader;
mod pane;
mod window;

use std::cell::Cell;

use adw::prelude::*;
use clap::{CommandFactory, Parser};
use gtk::{gio, glib};

use crate::pane::Side;

const APP_ID: &str = "io.github.jsonlens.JsonLens";

fn main() -> glib::ExitCode {
    // `clap` termina el proceso con `--help`, `--version` o argumentos inválidos.
    let (left, right) = match args::Args::parse().into_inputs() {
        Ok(inputs) => inputs,
        Err(message) => args::Args::command()
            .error(clap::error::ErrorKind::ArgumentConflict, message)
            .exit(),
    };

    // Cada ejecución es una instancia propia: así `jsonlens a b` siempre abre su ventana
    // y `-` lee la entrada estándar de este proceso (una instancia remota no podría).
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    for (action, accel) in window::ACCELS {
        app.set_accels_for_action(action, &[accel]);
    }
    let inputs = Cell::new(Some((left, right)));
    app.connect_activate(move |app| {
        let window = window::Window::new(app);
        if let Some((left, right)) = inputs.take() {
            if let Some(input) = left {
                window.load(Side::Left, input);
            }
            if let Some(input) = right {
                window.load(Side::Right, input);
            }
        }
        window.window.present();
    });
    // GTK no debe interpretar los argumentos: ya los procesó `clap`.
    let program: Vec<String> = std::env::args().take(1).collect();
    app.run_with_args(&program)
}
