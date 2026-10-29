//! lljson: compare and format JSON (GTK4 + libadwaita).

mod args;
#[cfg(any(debug_assertions, feature = "devtools"))]
mod devtools;
mod loader;
mod nodecopy;
mod options;
mod pane;
mod session;
mod summary;
mod theme;
mod transform;
mod view;
mod window;

use std::cell::Cell;

use adw::prelude::*;
use clap::{CommandFactory, Parser};
use gtk::{gio, glib};

use crate::pane::Side;

const APP_ID: &str = "io.github.sebastianll28.Lljson";

fn main() -> glib::ExitCode {
    #[cfg(any(debug_assertions, feature = "devtools"))]
    devtools::mark_start();
    let (left, right) = match args::Args::parse().into_inputs() {
        Ok(inputs) => inputs,
        Err(message) => args::Args::command()
            .error(clap::error::ErrorKind::ArgumentConflict, message)
            .exit(),
    };

    // NON_UNIQUE: `lljson a b` must always open its own window, and `-` must read
    // this process's stdin (a remote instance could not).
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    for (action, accels) in window::ACCELS {
        app.set_accels_for_action(action, accels);
    }
    let inputs = Cell::new(Some((left, right)));
    app.connect_activate(move |app| {
        view::install_styles();
        let window = window::Window::new(app);
        let (left, right) = inputs.take().unwrap_or((None, None));
        // Panes not given on the command line come back as they were last time.
        if let Some(input) = left.or_else(|| session::restore(Side::Left)) {
            window.load(Side::Left, input);
        }
        if let Some(input) = right.or_else(|| session::restore(Side::Right)) {
            window.load(Side::Right, input);
        }
        window.window.present();
        #[cfg(any(debug_assertions, feature = "devtools"))]
        devtools::install(&window);
    });
    // Hide the arguments from GTK; clap already handled them.
    let program: Vec<String> = std::env::args().take(1).collect();
    app.run_with_args(&program)
}
