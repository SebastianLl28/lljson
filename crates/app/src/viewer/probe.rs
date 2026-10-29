//! Editing-latency probe of the Viewer (dev tooling, see `devtools`): loads a file and
//! reports the main-thread cost of opening it, the keystroke latency at the start, middle
//! and end of the text, scrolling smoothness and memory, then quits.
//!
//! "Frozen" times are measured until the second frame clock tick after the change: the first
//! tick belongs to the frame that lays out and draws the change, the second one starts only
//! when that frame is done.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::glib;

use super::validate::Validation;
use super::{ViewerPage, file};
use crate::devtools;

const KEYSTROKES: usize = 20;
const SCROLL_SECONDS: f64 = 3.0;
const SCROLL_SPEED: f64 = 3000.0;

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Resolves once the window's frame clock has ticked `count` times.
async fn frames(window: &adw::ApplicationWindow, count: u32) {
    let (sender, receiver) = async_channel::bounded(1);
    let seen = Cell::new(0);
    window.add_tick_callback(move |_, _| {
        seen.set(seen.get() + 1);
        if seen.get() >= count {
            let _ = sender.try_send(());
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    let _ = receiver.recv().await;
}

fn validation_label(validation: &Validation) -> &'static str {
    match validation {
        Validation::Empty => "empty",
        Validation::Valid => "valid",
        Validation::Invalid { .. } => "invalid",
    }
}

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples.get(samples.len() / 2).copied().unwrap_or(0.0)
}

pub fn run(page: &Rc<ViewerPage>, window: &adw::ApplicationWindow, path: PathBuf) {
    let page = Rc::clone(page);
    let window = window.clone();
    glib::MainContext::default().spawn_local(async move {
        let started = Instant::now();
        let text = match file::spawn_read(path).recv().await {
            Ok(Ok(text)) => text,
            _ => {
                eprintln!("viewer probe: could not read the file");
                window.close();
                return;
            }
        };
        eprintln!(
            "read + decode (background): {:.1} MiB in {:.1} ms",
            text.len() as f64 / (1024.0 * 1024.0),
            ms(started.elapsed())
        );

        let expected = text.len();
        let intervals = Rc::new(RefCell::new(Vec::new()));
        let loading = Rc::new(Cell::new(true));
        let last = Cell::new(None::<i64>);
        let (collected, still_loading) = (Rc::clone(&intervals), Rc::clone(&loading));
        window.add_tick_callback(move |_, clock| {
            let now = clock.frame_time();
            if let Some(previous) = last.replace(Some(now)) {
                collected
                    .borrow_mut()
                    .push((now - previous) as f64 / 1000.0);
            }
            if still_loading.get() {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
        let start = Instant::now();
        let done = page.editor.set_text(text);
        let _ = done.recv().await;
        let total = ms(start.elapsed());
        loading.set(false);
        frames(&window, 2).await;
        let matches = page.editor.text().len() == expected;
        eprintln!(
            "open (chunked): fully inserted in {total:.1} ms, {}; text length preserved: {matches}",
            if intervals.borrow().is_empty() {
                "no frames drawn while loading".to_owned()
            } else {
                let mut sorted = intervals.borrow().clone();
                sorted.sort_by(f64::total_cmp);
                format!(
                    "{} frames while loading, worst frame interval {:.1} ms",
                    sorted.len(),
                    sorted.last().copied().unwrap_or(0.0)
                )
            }
        );

        let validation_start = Instant::now();
        page.schedule_validation();
        while page.validation().is_none() {
            glib::timeout_future(Duration::from_millis(5)).await;
        }
        eprintln!(
            "live validation (debounce + background parse): {:?} in {:.1} ms",
            page.validation().as_deref().map(validation_label),
            ms(validation_start.elapsed())
        );

        let view = page.editor.view.clone();
        let buffer = view.buffer();
        let total = buffer.char_count();
        for (label, offset) in [("start", 0), ("middle", total / 2), ("end", total)] {
            let mut samples = Vec::with_capacity(KEYSTROKES);
            for _ in 0..KEYSTROKES {
                buffer.place_cursor(&buffer.iter_at_offset(offset));
                view.scroll_mark_onscreen(&buffer.get_insert());
                frames(&window, 2).await;
                let start = Instant::now();
                buffer.insert_at_cursor("x");
                frames(&window, 2).await;
                samples.push(ms(start.elapsed()));
            }
            eprintln!(
                "keystroke at {label}: median {:.1} ms, max {:.1} ms ({KEYSTROKES} samples)",
                median(&mut samples),
                samples.iter().copied().fold(0.0, f64::max)
            );
        }

        let scrolled = &page.editor.root;
        let (vertical, horizontal) = (scrolled.vadjustment(), scrolled.hadjustment());
        let range = |a: &gtk::Adjustment| a.upper() - a.page_size();
        let (adjustment, axis) = if range(&vertical) >= range(&horizontal) {
            (vertical, "vertical")
        } else {
            (horizontal, "horizontal (single long line)")
        };
        let max = range(&adjustment).max(0.0);
        let intervals = Rc::new(RefCell::new(Vec::new()));
        let (sender, receiver) = async_channel::bounded(1);
        let first = Cell::new(None::<i64>);
        let last = Cell::new(None::<i64>);
        let collected = Rc::clone(&intervals);
        window.add_tick_callback(move |_, clock| {
            let now = clock.frame_time();
            let begin = *first.get().get_or_insert(now);
            first.set(Some(begin));
            if let Some(previous) = last.replace(Some(now)) {
                collected
                    .borrow_mut()
                    .push((now - previous) as f64 / 1000.0);
            }
            let elapsed = (now - begin) as f64 / 1e6;
            if elapsed >= SCROLL_SECONDS {
                let _ = sender.try_send(());
                return glib::ControlFlow::Break;
            }
            adjustment.set_value((elapsed * SCROLL_SPEED).min(max));
            glib::ControlFlow::Continue
        });
        let _ = receiver.recv().await;
        devtools::report(&format!("scroll ({axis})"), &intervals.borrow());
        devtools::report_memory();
        window.close();
    });
}
