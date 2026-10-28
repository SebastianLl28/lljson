//! Herramientas de desarrollo (solo en builds de debug).
//!
//! `JSONLENS_SCREENSHOT=captura.png` renderiza la ventana a PNG y cierra la app, para
//! revisar la interfaz sin capturar el escritorio. `JSONLENS_SCREENSHOT_DELAY_MS` fija la
//! espera antes de la captura (1500 ms por defecto). `JSONLENS_TOGGLE=1,4` expande o
//! colapsa esas filas (en orden) 500 ms antes de la captura.

use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;

use crate::window::Window;

pub fn schedule_screenshot(app_window: &Rc<Window>) {
    let window = &app_window.window;
    let Some(path) = std::env::var_os("JSONLENS_SCREENSHOT") else {
        return;
    };
    let delay: u64 = std::env::var("JSONLENS_SCREENSHOT_DELAY_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(1500);
    if let Ok(list) = std::env::var("JSONLENS_TOGGLE") {
        let positions: Vec<u32> = list
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        let weak = Rc::downgrade(app_window);
        glib::timeout_add_local_once(
            Duration::from_millis(delay.saturating_sub(500)),
            move || {
                if let Some(app_window) = weak.upgrade() {
                    for position in positions {
                        let start = Instant::now();
                        app_window.toggle(position);
                        eprintln!("fila {position}: {:?}", start.elapsed());
                    }
                }
            },
        );
    }
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
