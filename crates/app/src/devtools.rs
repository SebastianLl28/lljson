//! Herramientas de desarrollo: activas en builds de debug o con la feature `devtools`
//! (`cargo run --release -p jsonlens --features devtools`). Se controlan con variables
//! de entorno:
//!
//! - `JSONLENS_TRACE=1`: tiempos desde el arranque del proceso (primer frame, documentos
//!   cargados, filas listas).
//! - `JSONLENS_SCREENSHOT=captura.png`: renderiza la ventana a PNG y cierra la app.
//!   `JSONLENS_SCREENSHOT_DELAY_MS` fija la espera (1500 ms por defecto).
//! - `JSONLENS_TOGGLE=1,4`: expande o colapsa esas filas 500 ms antes de la captura.
//! - `JSONLENS_GOTO=n,n,p`: activa «diferencia siguiente» (`n`) o «anterior» (`p`) en orden,
//!   400 ms antes de la captura.
//! - `JSONLENS_ACTIONS=win.hide-equal,…`: activa esas acciones de la ventana 800 ms antes
//!   de la captura.
//! - `JSONLENS_ARRAY_KEY=id` y `JSONLENS_IGNORE=regla;regla`: opciones de comparación.
//! - `JSONLENS_COPY=lado:fila:path|value`: copia la ruta o el valor y escribe el
//!   portapapeles. `JSONLENS_NODE_MENU=lado:fila`: abre el menú contextual de una fila.
//! - `JSONLENS_CLIPBOARD_FILE=a.json:b.json`: compara `a.json` con el portapapeles (`b.json`).
//! - `JSONLENS_SCROLL=px`: desplaza el panel izquierdo 300 ms antes de la captura.
//! - `JSONLENS_SCROLL_TEST=segundos`: cuando las filas están listas, hace scroll continuo
//!   la mitad del tiempo y saltos aleatorios la otra mitad, mide la duración de cada
//!   frame y la memoria, y cierra la app.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;

use crate::nodecopy::Copy;
use crate::pane::Side;
use crate::window::Window;

/// Velocidad del scroll continuo, en píxeles por segundo.
const SCROLL_SPEED: f64 = 3000.0;

static START: OnceLock<Instant> = OnceLock::new();

thread_local! {
    static ON_ROWS_READY: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

/// Marca el inicio del proceso (lo antes posible en `main`).
pub fn mark_start() {
    let _ = START.set(Instant::now());
}

/// Escribe un evento con el tiempo transcurrido desde el arranque si `JSONLENS_TRACE=1`.
pub fn trace(event: &str) {
    if std::env::var_os("JSONLENS_TRACE").is_some() {
        let ms = START
            .get()
            .map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0);
        eprintln!("[{ms:8.1} ms] {event}");
    }
}

/// La ventana avisa cuando las filas de los documentos actuales están en la vista.
pub fn rows_ready(rows: usize) {
    trace(&format!("filas listas: {rows}"));
    if let Some(f) = ON_ROWS_READY.with(|f| f.borrow_mut().take()) {
        f();
    }
}

pub fn install(app_window: &Rc<Window>) {
    let window = &app_window.window;
    window.add_tick_callback(|_, _| {
        trace("primer frame");
        glib::ControlFlow::Break
    });
    if let Some(seconds) = std::env::var("JSONLENS_SCROLL_TEST")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
    {
        let weak = Rc::downgrade(app_window);
        ON_ROWS_READY.with(|f| {
            f.replace(Some(Box::new(move || {
                if let Some(app_window) = weak.upgrade() {
                    scroll_test(&app_window, seconds);
                }
            })));
        });
    }
    let array_key = std::env::var("JSONLENS_ARRAY_KEY").unwrap_or_default();
    let ignore = std::env::var("JSONLENS_IGNORE").unwrap_or_default();
    if !array_key.is_empty() || !ignore.is_empty() {
        app_window.set_options_text(crate::options::OptionsText {
            array_key,
            ignore: ignore.replace(';', "\n"),
        });
    }
    // `JSONLENS_CLIPBOARD_FILE=a.json:b.json`: copia `b.json` al portapapeles y lo compara
    // con `a.json` como lo haría «comparar portapapeles con archivo».
    if let Ok(spec) = std::env::var("JSONLENS_CLIPBOARD_FILE")
        && let Some((file, clip)) = spec.split_once(':')
    {
        let text = std::fs::read_to_string(clip).unwrap_or_default();
        app_window.window.clipboard().set_text(&text);
        app_window.compare_clipboard_with_path(std::path::Path::new(file));
    }
    schedule_screenshot(app_window);
}

fn schedule_screenshot(app_window: &Rc<Window>) {
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
    if let Ok(list) = std::env::var("JSONLENS_GOTO") {
        let steps: Vec<String> = list
            .split(',')
            .map(|p| {
                format!(
                    "win.{}-change",
                    if p.trim() == "p" { "prev" } else { "next" }
                )
            })
            .collect();
        let weak = Rc::downgrade(app_window);
        glib::timeout_add_local_once(
            Duration::from_millis(delay.saturating_sub(400)),
            move || {
                if let Some(app_window) = weak.upgrade() {
                    for step in steps {
                        let _ = WidgetExt::activate_action(&app_window.window, &step, None);
                    }
                }
            },
        );
    }
    if let Ok(list) = std::env::var("JSONLENS_ACTIONS") {
        let names: Vec<String> = list.split(',').map(|n| n.trim().to_owned()).collect();
        let weak = Rc::downgrade(app_window);
        glib::timeout_add_local_once(
            Duration::from_millis(delay.saturating_sub(800)),
            move || {
                if let Some(app_window) = weak.upgrade() {
                    for name in names {
                        let _ = WidgetExt::activate_action(&app_window.window, &name, None);
                    }
                }
            },
        );
    }
    if let Ok(spec) = std::env::var("JSONLENS_COPY") {
        let weak = Rc::downgrade(app_window);
        glib::timeout_add_local_once(
            Duration::from_millis(delay.saturating_sub(800)),
            move || {
                let Some(app_window) = weak.upgrade() else {
                    return;
                };
                let parts: Vec<&str> = spec.split(':').collect();
                let [side, position, what] = parts.as_slice() else {
                    return;
                };
                let side = if *side == "right" {
                    Side::Right
                } else {
                    Side::Left
                };
                let what = if *what == "path" {
                    Copy::Path
                } else {
                    Copy::Value
                };
                app_window.copy_node(side, position.parse().unwrap_or(0), what);
                let clipboard = app_window.window.clipboard();
                glib::timeout_add_local_once(Duration::from_millis(300), move || {
                    glib::MainContext::default().spawn_local(async move {
                        match clipboard.read_text_future().await {
                            Ok(text) => eprintln!("portapapeles: {text:?}"),
                            Err(err) => eprintln!("portapapeles: error {err}"),
                        }
                    });
                });
            },
        );
    }
    if let Ok(spec) = std::env::var("JSONLENS_NODE_MENU") {
        let weak = Rc::downgrade(app_window);
        glib::timeout_add_local_once(
            Duration::from_millis(delay.saturating_sub(800)),
            move || {
                let Some(app_window) = weak.upgrade() else {
                    return;
                };
                let Some((side, position)) = spec.split_once(':') else {
                    return;
                };
                let side = if side == "right" {
                    Side::Right
                } else {
                    Side::Left
                };
                let list = app_window.list(side);
                app_window.show_node_menu(
                    side,
                    position.parse().unwrap_or(0),
                    list.upcast_ref(),
                    150.0,
                    150.0,
                );
            },
        );
    }
    if let Some(px) = std::env::var("JSONLENS_SCROLL")
        .ok()
        .and_then(|px| px.parse::<f64>().ok())
    {
        let adjustment = app_window.left_vadjustment();
        glib::timeout_add_local_once(
            Duration::from_millis(delay.saturating_sub(300)),
            move || {
                adjustment.set_value(px);
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

/// Scroll continuo y luego saltos aleatorios; informa de la duración de los frames.
fn scroll_test(app_window: &Rc<Window>, seconds: f64) {
    let adjustment = app_window.left_vadjustment();
    let first = Cell::new(None::<i64>);
    let last = Cell::new(None::<i64>);
    let smooth = RefCell::new(Vec::new());
    let jumps = RefCell::new(Vec::new());
    let seed = Cell::new(0x9E37_79B9_7F4A_7C15_u64);
    let window = app_window.window.clone();
    eprintln!(
        "prueba de scroll: {seconds} s, altura del contenido {:.0} px",
        adjustment.upper()
    );
    window.add_tick_callback(move |window, clock| {
        let now = clock.frame_time();
        let start = *first.get().get_or_insert(now);
        first.set(Some(start));
        let elapsed = (now - start) as f64 / 1e6;
        if let Some(previous) = last.replace(Some(now)) {
            let interval = (now - previous) as f64 / 1000.0;
            if elapsed < seconds / 2.0 {
                smooth.borrow_mut().push(interval);
            } else {
                jumps.borrow_mut().push(interval);
            }
        }
        let max = adjustment.upper() - adjustment.page_size();
        if elapsed < seconds / 2.0 {
            adjustment.set_value((elapsed * SCROLL_SPEED).min(max));
        } else if elapsed < seconds {
            let next = seed
                .get()
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            seed.set(next);
            adjustment.set_value((next >> 11) as f64 / (1u64 << 53) as f64 * max);
        } else {
            eprintln!(
                "altura final del contenido {:.0} px, ventana {:.0} px",
                adjustment.upper(),
                adjustment.page_size()
            );
            report("scroll continuo", &smooth.borrow());
            report("saltos aleatorios", &jumps.borrow());
            report_memory();
            window.close();
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

fn report(name: &str, intervals: &[f64]) {
    if intervals.is_empty() {
        eprintln!("{name}: sin frames");
        return;
    }
    let mut sorted = intervals.to_vec();
    sorted.sort_by(f64::total_cmp);
    let pct = |p: f64| sorted[((sorted.len() - 1) as f64 * p).round() as usize];
    let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
    let slow = sorted
        .iter()
        .filter(|&&ms| ms > 1000.0 / 60.0 * 1.5)
        .count();
    eprintln!(
        "{name}: {} frames, {:.1} fps de media, intervalo p50 {:.1} ms, p95 {:.1} ms, \
         p99 {:.1} ms, máx {:.1} ms, {slow} frames > 25 ms",
        sorted.len(),
        1000.0 / mean,
        pct(0.5),
        pct(0.95),
        pct(0.99),
        pct(1.0),
    );
}

fn report_memory() {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for field in ["VmRSS", "VmHWM"] {
        if let Some(line) = status.lines().find(|l| l.starts_with(field)) {
            eprintln!("{line}");
        }
    }
}
