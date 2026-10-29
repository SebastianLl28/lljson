# Viewer Tab — Phase 11 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Viewer tab next to Compare with a plain editable text editor (type, empty, paste, open file, drag and drop, copy all), and measure its editing latency on the 1M-node file.

**Architecture:** `adw::ViewStack` + `adw::ViewSwitcher` in `Window`; Compare content moves into the first page untouched, the Viewer is a new `crates/app/src/viewer/` module (`mod.rs` page, `editor.rs` text widget, `file.rs` GTK-free background file reading). No `core` changes in this phase. No new dependencies.

**Tech Stack:** Rust 2024, gtk4 0.11.5, libadwaita 0.9.2, async-channel 2.5.0 (all already pinned).

**Spec:** `docs/superpowers/specs/2026-10-28-viewer-tab-design.md` (Phase 11 section).

## Global Constraints

- Every task ends with `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` all green, the task ticked in `TASKS.md` (`[ ]` → `[x]`), and one commit.
- Commit messages: English, `phase 11: short description`, **no `Co-Authored-By` or any trailer**.
- No `unwrap()`/`expect()` outside tests; no `unsafe`; no new dependencies.
- Never block the UI thread: disk reads run on a background thread and return through `async-channel` + `glib::MainContext::spawn_local`.
- Compare tab behavior must be unchanged (no regression in its actions, shortcuts, session restore).
- GTK API doubts: check docs.rs for `gtk4 0.11.5` / `libadwaita 0.9.2` before guessing.
- Do not start Phase 12 without the user's confirmation.

## Review Focus

- **Ctrl+V / Ctrl+O / Ctrl+F in the Viewer** must edit text or open a Viewer file, never trigger the Compare actions bound to the same keys (`ACCELS` in `window.rs`).
- **Switching tabs** back and forth keeps the Viewer text and the Compare panes/state intact, and Compare actions regain their previous enabled state.
- **Open a file with invalid UTF-8, or a missing/unreadable file:** toast with the reason, editor text unchanged.
- **Open an empty file or a file with a UTF-8 BOM:** empty file gives an empty editor without error; BOM is not shown in the text.
- **Two opens in quick succession, or drop of several files / a non-file / a directory:** only the latest open applies; extra files give a toast; nothing panics.

---

## File Structure

- Create `crates/app/src/viewer/mod.rs` — `ViewerPage`: builds toolbar + editor, owns actions' logic, open/drop/copy wiring.
- Create `crates/app/src/viewer/editor.rs` — `Editor`: `GtkTextView` in a `GtkScrolledWindow`, text get/set.
- Create `crates/app/src/viewer/file.rs` — GTK-free: decode bytes, read a file, spawn a background read.
- Modify `crates/app/src/main.rs` — `mod viewer;`.
- Modify `crates/app/src/window.rs` — tab stack, switcher, per-page header/summary/actions handling, viewer accelerators.
- Modify `crates/app/src/devtools.rs` — `LLJSON_TAB`, `LLJSON_VIEWER_FILE`, latency probe (Task 4).
- Modify `crates/app/src/style.css` / `style-dark.css` only if the editor needs a monospace class not already available.
- Modify `BENCHMARKS.md`, `TASKS.md`.

---

### Task 1: Compare / Viewer tab switcher

**Files:**
- Create: `crates/app/src/viewer/mod.rs` (placeholder page)
- Modify: `crates/app/src/main.rs`, `crates/app/src/window.rs`, `crates/app/src/devtools.rs`

**Interfaces:**
- Produces: `viewer::ViewerPage::new() -> Rc<ViewerPage>`; `ViewerPage::root(&self) -> &gtk::Widget` (page content, a placeholder label "Viewer" for now).
- Produces: `Window` fields `stack: adw::ViewStack` (pages named `"compare"` and `"viewer"`), `viewer: Rc<ViewerPage>`.
- Produces: devtools env `LLJSON_TAB=viewer|compare` selects the tab before the screenshot.

- [ ] **Step 1: Add `mod viewer;` and the placeholder `ViewerPage`.**
- [ ] **Step 2: In `Window::build`, wrap the existing `toasts` overlay content as the `"compare"` page of an `adw::ViewStack`; add a `"viewer"` page; put an `adw::ViewSwitcher` (wide policy) as the header title widget, replacing `adw::WindowTitle` in the header. Keep the Compare subtitle (`update_title`) reachable: show it as the Compare page's title only if it can be kept without clutter; otherwise drop the subtitle and note it in the commit description.**
- [ ] **Step 3: Compare-only widgets (hide-equal toggle, prev/next buttons, options button, compare menu entry, `summary.root` bottom bar) are hidden while the Viewer is visible, shown again for Compare (`stack.connect_visible_child_name_notify`).**
- [ ] **Step 4: Compare window actions (`open-*`, `paste-*`, `next/prev-change`, `hide-equal`, `diff-options`, `compare-clipboard`, `search*`, transforms, copy-text) are disabled while the Viewer is visible, and on return restored to their prior state by calling the existing refresh methods (`update_transform_actions`, `set_navigation_enabled`, `set_hide_equal_enabled`) rather than blindly enabling.**
- [ ] **Step 5: Devtools: `LLJSON_TAB=viewer` switches the stack before `schedule_screenshot` runs; document it in the module header comment.**
- [ ] **Step 6: Verify.** Run `cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`. Then `LLJSON_TAB=viewer LLJSON_SCREENSHOT=$SCRATCH/viewer.png cargo run -p lljson` and `LLJSON_SCREENSHOT=$SCRATCH/compare.png cargo run -p lljson -- a.json b.json` (any two small JSON files); view both PNGs: switcher visible, Compare identical to before, Viewer shows the placeholder without Compare buttons/summary bar.
- [ ] **Step 7: Tick Task 1 in `TASKS.md` (Phase 11, "Tab switcher") and commit** `phase 11: add Compare/Viewer tab switcher`.

---

### Task 2: Viewer editor

**Files:**
- Create: `crates/app/src/viewer/editor.rs`
- Modify: `crates/app/src/viewer/mod.rs`, `crates/app/src/window.rs` (only the accel/enable wiring below)

**Interfaces:**
- Produces: `Editor::new() -> Editor` with `pub root: gtk::ScrolledWindow`, `pub view: gtk::TextView`, `fn text(&self) -> String`, `fn set_text(&self, text: &str)`, `fn is_empty(&self) -> bool`.
- Consumes: `ViewerPage::root` from Task 1 (now a vertical box: editor fills it).

- [ ] **Step 1: Implement `Editor`.** Monospace (`view.set_monospace(true)`), no wrapping (`WrapMode::None`), left/right/top/bottom margins matching the pane look, `accepts_tab` true, hexpand/vexpand, undo enabled (default `GtkTextBuffer` undo).
- [ ] **Step 2: Wire it as the content of `ViewerPage`.**
- [ ] **Step 3: Resolve the shortcut conflict.** With the Viewer visible and the editor focused, `Ctrl+V` must paste into the text, `Ctrl+C/X/Z/A` behave natively, and `Ctrl+F` does nothing yet (Compare search is disabled). If disabled Compare actions still swallow the key (GTK capture-phase app accelerators), fix by giving `ViewerPage` its own capture-phase `GtkShortcutController` or by removing the accelerators from the app while the Viewer is visible (`set_accels_for_action` with `&[]`, restoring `ACCELS` on return). Choose whichever works and note it in a code comment.
- [ ] **Step 4: Verify manually** (run `cargo run -p lljson`, Viewer tab): type text, select all and delete (editor becomes empty), paste a JSON from the clipboard, undo/redo; switch to Compare and back: text preserved; Compare `Ctrl+V` still pastes into the left pane.
- [ ] **Step 5: Build/test/clippy green, tick the "Viewer editor" task in `TASKS.md`, commit** `phase 11: add Viewer text editor`.

---

### Task 3: Open file, drag and drop, copy all

**Files:**
- Create: `crates/app/src/viewer/file.rs`
- Modify: `crates/app/src/viewer/mod.rs`, `crates/app/src/window.rs` (actions + `ACCELS`), `crates/app/src/devtools.rs`

**Interfaces:**
- Produces in `file.rs` (no GTK):
  - `enum ReadError { Io { path: PathBuf, source: io::Error }, NotUtf8 { path: PathBuf }, Interrupted }` implementing `Display` (`"Could not read: …"`, `"The file is not valid UTF-8 text"`) and `Error`.
  - `fn decode(bytes: Vec<u8>) -> Result<String, std::string::FromUtf8Error>` — strips a leading UTF-8 BOM.
  - `fn read_text(path: &Path) -> Result<String, ReadError>`.
  - `fn spawn_read(path: PathBuf) -> async_channel::Receiver<Result<String, ReadError>>` — one result, same pattern as `loader::spawn_load`.
- Produces in `ViewerPage`: `fn open_path(self: &Rc<Self>, path: PathBuf)` (background read, then `Editor::set_text`; a generation counter drops stale results), `fn copy_all(&self)`, `fn open_dialog(self: &Rc<Self>)`.
- Produces actions: `win.viewer-open` (`<Control>o`), `win.viewer-copy-all` (`<Control><Shift>c`), enabled only while the Viewer is visible.
- Produces: devtools env `LLJSON_VIEWER_FILE=path` opens that file into the Viewer at startup (also switches to the Viewer tab).

- [ ] **Step 1: Write failing tests in `file.rs` `#[cfg(test)]`:** `decode_strips_bom` (`b"\xEF\xBB\xBF{}"` → `"{}"`), `decode_keeps_plain_text`, `decode_rejects_invalid_utf8` (`vec![0xFF, 0xFE, b'a']` is an error), `read_text_reports_missing_file` (`ReadError::Io`), `read_text_reads_empty_file` (→ `""`), `read_text_reports_non_utf8_file` (→ `NotUtf8`), `spawn_read_delivers_one_result` (second `recv_blocking` is `Err`). Use `std::env::temp_dir()` + process id for files, as `loader.rs` tests do.
- [ ] **Step 2: Run `cargo test -p lljson file::` — expect FAIL (items undefined).**
- [ ] **Step 3: Implement the `file.rs` items above.**
- [ ] **Step 4: Run the tests — expect PASS.**
- [ ] **Step 5: Implement `ViewerPage` wiring.** Toolbar above the editor (`css_classes(["toolbar"])`) with an open button and a copy-all button (icons `document-open-symbolic`, `edit-copy-symbolic`, tooltips with the shortcuts), reusing the file filters of `Window::choose_file` (extract that dialog helper to a shared function if it avoids duplicating the filter code). Read errors and non-local files show a toast through the page's own `adw::ToastOverlay`. A `gtk::DropTarget` on the page accepts `gdk::FileList` (first file only; if more than one, toast "Only the first file is opened") and `String` (replaces the editor text).
- [ ] **Step 6: Register the actions and accelerators in `window.rs`; enable them only when the Viewer is visible (same mechanism as Task 1).**
- [ ] **Step 7: Verify manually:** open a valid file, an empty file, a UTF-8 BOM file, a binary file (toast, text unchanged), a missing path via drop; drop several files; open twice quickly (last wins); copy all and paste into another app. Screenshot with `LLJSON_VIEWER_FILE=<small.json> LLJSON_SCREENSHOT=$SCRATCH/viewer-file.png` and view it.
- [ ] **Step 8: Build/test/clippy green, tick the two tasks it covers in `TASKS.md` ("Viewer editor" already ticked; tick "Open file, drag and drop, copy all"), commit** `phase 11: open, drop and copy all in the Viewer`.

---

### Task 4: Measure editing latency on the 1M-node file

**Files:**
- Modify: `crates/app/src/devtools.rs`, `BENCHMARKS.md`, `TASKS.md`

**Interfaces:**
- Consumes: `LLJSON_VIEWER_FILE` and `ViewerPage`/`Editor` from Task 3.
- Produces: devtools env `LLJSON_VIEWER_LATENCY=1`: after the file is loaded into the Viewer, report to stderr (and quit) the numbers below.

Measurements, each for three inputs (generated with `lljson-cli generate`, see `BENCHMARKS.md` for the 1M variant): the 1M-node file indented (~20.7 MiB), the same minified (single ~13.7 MiB line), and a ~100k-node indented file.

1. Main-thread time of `Editor::set_text` and time until the next frame is drawn after it (how long the UI is frozen when a file is opened).
2. Keystroke latency: insert one character at the start, middle and end of the buffer, each timed until the next frame tick (median of 20).
3. Scroll smoothness inside the editor (reuse the frame-time reporting of `scroll_test`).
4. RSS after loading (reuse `report_memory`).

- [ ] **Step 1: Implement the probe in `devtools.rs`** (debug or `devtools` feature only, like the rest), reusing `report`/`report_memory`.
- [ ] **Step 2: Run it in release with the `devtools` feature** (`cargo run --release -p lljson --features devtools`), on the same machine as the existing numbers, for the three inputs.
- [ ] **Step 3: Add a "Viewer editor (Phase 11)" section to `BENCHMARKS.md`** with the machine, files and a table of results.
- [ ] **Step 4: Conclude in that section.** If opening or typing freezes the UI noticeably (rule of thumb: a frame over ~100 ms on open, or over ~50 ms per keystroke) the goal is not met: per `CLAUDE.md`, do not change the architecture. Stop and report the numbers to the user with options (e.g. a size threshold above which the Viewer opens read-only or asks first, chunked loading, `GtkSourceView`, deferring large files to the Compare tab) and wait for their decision. If it is fine, say so with the numbers.
- [ ] **Step 5: Build/test/clippy green, tick the last Phase 11 task in `TASKS.md` only if the measurement is done and reported, commit** `phase 11: measure Viewer editing latency on a 1M-node file`.
- [ ] **Step 6: Stop. Summarize Phase 11 for the user and wait for confirmation before Phase 12.**
