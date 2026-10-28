//! Herramientas de desarrollo (solo en builds de debug).
//!
//! `JSONLENS_SCREENSHOT=captura.png` renderiza la ventana a PNG y cierra la app, para
//! revisar la interfaz sin capturar el escritorio. `JSONLENS_SCREENSHOT_DELAY_MS` fija la
//! espera antes de la captura (1500 ms por defecto).

use std::time::Duration;

use adw::prelude::*;
use gtk::glib;

pub fn schedule_screenshot(window: &adw::ApplicationWindow) {
    let Some(path) = std::env::var_os("JSONLENS_SCREENSHOT") else {
        return;
    };
    let delay = std::env::var("JSONLENS_SCREENSHOT_DELAY_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(1500);
    let window = window.clone();
    glib::timeout_add_local_once(Duration::from_millis(delay), move || {
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(
            &snapshot,
            f64::from(window.width()),
            f64::from(window.height()),
        );
        let texture = snapshot.to_node().and_then(|node| {
            let renderer = window.native()?.renderer()?;
            Some(renderer.render_texture(&node, None))
        });
        match texture.map(|t| t.save_to_png(&path)) {
            Some(Ok(())) => eprintln!("captura guardada en {}", path.to_string_lossy()),
            Some(Err(err)) => eprintln!("no se pudo guardar la captura: {err}"),
            None => eprintln!("no se pudo renderizar la ventana"),
        }
        window.close();
    });
}
