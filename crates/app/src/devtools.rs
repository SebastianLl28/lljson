//! Dev tooling, enabled in debug builds or with the `devtools` feature. Env vars:
//!
//! - `LLJSON_TAB=compare|viewer`: select the tab before anything else.
//! - `LLJSON_TRACE=1`: timings since process start.
//! - `LLJSON_SCREENSHOT=out.png`: render the window to PNG and quit;
//!   `LLJSON_SCREENSHOT_DELAY_MS` sets the wait (default 1500).
//! - `LLJSON_TOGGLE=1,4`: expand/collapse those rows before the screenshot.
//! - `LLJSON_GOTO=n,n,p`: next (`n`) / previous (`p`) difference, in order.
//! - `LLJSON_ACTIONS=win.hide-equal,...`: activate window actions (`action::param@ms`).
//! - `LLJSON_PRINT_CLIPBOARD=1`: print the clipboard just before the screenshot.
//! - `LLJSON_SEARCH=text`: open the search bar with that text (`LLJSON_SEARCH_SIDE=0|1`,
//!   `LLJSON_SEARCH_SCOPE=0|1|2`, `LLJSON_SEARCH_MODE=0|1|2`); `LLJSON_SEARCH_OTHER=text`
//!   also searches the opposite pane. Activate `win.search-next-left|right` with
//!   `LLJSON_ACTIONS` to step through results.
//! - `LLJSON_ARRAY_KEY=id`, `LLJSON_IGNORE=rule;rule`: comparison options.
//! - `LLJSON_COPY=side:row:path|value`: copy a path or value and print the clipboard.
//! - `LLJSON_NODE_MENU=side:row`: open a row's context menu.
//! - `LLJSON_CLIPBOARD_FILE=a.json:b.json`: compare `a.json` with `b.json` as the clipboard.
//! - `LLJSON_SCROLL=px`: scroll the left pane before the screenshot.
//! - `LLJSON_SCROLL_TEST=secs`: once rows are ready, scroll continuously then jump
//!   randomly, report frame times and memory, and quit.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;

use crate::nodecopy::Copy;
use crate::pane::Side;
use crate::window::Window;

const SCROLL_SPEED: f64 = 3000.0;

static START: OnceLock<Instant> = OnceLock::new();

thread_local! {
    static ON_ROWS_READY: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

pub fn mark_start() {
    let _ = START.set(Instant::now());
}

pub fn trace(event: &str) {
    if std::env::var_os("LLJSON_TRACE").is_some() {
        let ms = START
            .get()
            .map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0);
        eprintln!("[{ms:8.1} ms] {event}");
    }
}

pub fn rows_ready(rows: usize) {
    trace(&format!("rows ready: {rows}"));
    if let Some(f) = ON_ROWS_READY.with(|f| f.borrow_mut().take()) {
        f();
    }
}

pub fn install(app_window: &Rc<Window>) {
    let window = &app_window.window;
    if let Ok(tab) = std::env::var("LLJSON_TAB") {
        app_window.select_tab(&tab);
    }
    window.add_tick_callback(|_, _| {
        trace("first frame");
        glib::ControlFlow::Break
    });
    if let Some(seconds) = std::env::var("LLJSON_SCROLL_TEST")
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
    if let Ok(text) = std::env::var("LLJSON_SEARCH") {
        let number = |name: &str| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0)
        };
        let side = if number("LLJSON_SEARCH_SIDE") == 1 {
            Side::Right
        } else {
            Side::Left
        };
        app_window.search_for_test(
            &text,
            side,
            number("LLJSON_SEARCH_SCOPE"),
            number("LLJSON_SEARCH_MODE"),
        );
        if let Ok(other) = std::env::var("LLJSON_SEARCH_OTHER") {
            app_window.search_for_test(&other, side.other(), 0, 0);
        }
    }
    let array_key = std::env::var("LLJSON_ARRAY_KEY").unwrap_or_default();
    let ignore = std::env::var("LLJSON_IGNORE").unwrap_or_default();
    if !array_key.is_empty() || !ignore.is_empty() {
        app_window.set_options_text(crate::options::OptionsText {
            array_key,
            ignore: ignore.replace(';', "\n"),
        });
    }
    if let Ok(spec) = std::env::var("LLJSON_CLIPBOARD_FILE")
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
    let Some(path) = std::env::var_os("LLJSON_SCREENSHOT") else {
        return;
    };
    let delay: u64 = std::env::var("LLJSON_SCREENSHOT_DELAY_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(1500);
    if let Ok(list) = std::env::var("LLJSON_TOGGLE") {
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
                        eprintln!("row {position}: {:?}", start.elapsed());
                    }
                }
            },
        );
    }
    if let Ok(list) = std::env::var("LLJSON_GOTO") {
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
    if let Ok(list) = std::env::var("LLJSON_ACTIONS") {
        for entry in list.split(',') {
            let (entry, extra_ms) = match entry.trim().split_once('@') {
                Some((entry, ms)) => (entry.to_owned(), ms.parse().unwrap_or(0)),
                None => (entry.trim().to_owned(), 0),
            };
            let weak = Rc::downgrade(app_window);
            glib::timeout_add_local_once(
                Duration::from_millis(delay.saturating_sub(800) + extra_ms),
                move || {
                    let Some(app_window) = weak.upgrade() else {
                        return;
                    };
                    let (action, param) = match entry.split_once("::") {
                        Some((action, param)) => (action, Some(param.to_variant())),
                        None => (entry.as_str(), None),
                    };
                    let _ = WidgetExt::activate_action(&app_window.window, action, param.as_ref());
                },
            );
        }
    }
    if std::env::var_os("LLJSON_PRINT_CLIPBOARD").is_some() {
        let clipboard = app_window.window.clipboard();
        glib::timeout_add_local_once(
            Duration::from_millis(delay.saturating_sub(100)),
            move || {
                glib::MainContext::default().spawn_local(async move {
                    match clipboard.read_text_future().await {
                        Ok(text) => eprintln!("clipboard: {text:?}"),
                        Err(err) => eprintln!("clipboard: error {err}"),
                    }
                });
            },
        );
    }
    if let Ok(spec) = std::env::var("LLJSON_COPY") {
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
                            Ok(text) => eprintln!("clipboard: {text:?}"),
                            Err(err) => eprintln!("clipboard: error {err}"),
                        }
                    });
                });
            },
        );
    }
    if let Ok(spec) = std::env::var("LLJSON_NODE_MENU") {
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
    if let Some(px) = std::env::var("LLJSON_SCROLL")
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
            Some(Ok(())) => eprintln!("screenshot saved to {}", path.to_string_lossy()),
            Some(Err(err)) => eprintln!("could not save screenshot: {err}"),
            None => eprintln!("could not render window"),
        }
        window.close();
    });
}

fn scroll_test(app_window: &Rc<Window>, seconds: f64) {
    let adjustment = app_window.left_vadjustment();
    let first = Cell::new(None::<i64>);
    let last = Cell::new(None::<i64>);
    let smooth = RefCell::new(Vec::new());
    let jumps = RefCell::new(Vec::new());
    let seed = Cell::new(0x9E37_79B9_7F4A_7C15_u64);
    let window = app_window.window.clone();
    eprintln!(
        "scroll test: {seconds} s, content height {:.0} px",
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
                "final content height {:.0} px, window {:.0} px",
                adjustment.upper(),
                adjustment.page_size()
            );
            report("continuous scroll", &smooth.borrow());
            report("random jumps", &jumps.borrow());
            report_memory();
            window.close();
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

fn report(name: &str, intervals: &[f64]) {
    if intervals.is_empty() {
        eprintln!("{name}: no frames");
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
        "{name}: {} frames, {:.1} fps avg, interval p50 {:.1} ms, p95 {:.1} ms, \
         p99 {:.1} ms, max {:.1} ms, {slow} frames > 25 ms",
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
