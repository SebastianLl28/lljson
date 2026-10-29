# Viewer tab — design

Date: 2026-10-28
Status: draft, pending user review

## Goal
Add a second tab, **Viewer**, next to the existing **Compare** tab. It is a single-document
editor and viewer in the spirit of jsonviewer.stack.hu and jsonformatter.org/json-viewer:
free text editing on one side, a typed tree view on the other. The Compare tab is not modified.

Success criteria:
- Typing, deleting everything and pasting all work in the editor.
- Pasting valid JSON formats it immediately.
- Invalid JSON shows where it fails (line, column) with a hint.
- The tree shows type icons, container lengths and array indices, and stays smooth on large input.
- Every rule in `CLAUDE.md` still holds (no UI-thread blocking, virtualized views, no `unwrap`).

## Layout
`AdwViewSwitcher` / `AdwViewStack` with two pages. The Viewer page is a horizontal split:
- **Left:** toolbar (open, copy all, format, minify, sort keys, tolerant toggle), a
  `GtkTextView` editor, and a status bar (valid / error `line:col` + hint).
- **Right:** tree view (`GtkListView`, virtualized) with a search bar and expand/collapse actions.

## Architecture
- `crates/core` gets everything that is not UI: error hints, and a node↔text-offset mapping
  (byte offset → `NodeId` and `NodeId` → span). It already provides parsing, tolerant mode,
  formatting, minify, sort keys, path building and search. All new `core` code ships with unit tests.
- `crates/app` gets a `viewer/` module (editor, status bar, tree model, tab page). The tree reuses
  the existing flat-row + custom `GListModel` approach from the Compare panes; it never creates
  a widget per node.
- Validation flow: editor change → debounce (~150 ms) → background thread parses the text →
  result returns through `async-channel` + `glib::MainContext::spawn_local` → UI updates status
  bar, error mark and tree. A newer edit cancels/ignores stale results via a generation counter.
- No new dependencies. The editor is a plain `GtkTextView`.

## Phases
Each phase is delivered task by task; each task ends with a green `cargo build`, `cargo test`,
`cargo clippy -- -D warnings`, a `TASKS.md` update and one commit (`phase N: ...`).

### Phase 11 — Tabs and basic editor
- Tab switcher with Compare and Viewer; Compare behaves exactly as before.
- Viewer editor: editable `GtkTextView`, can be emptied and pasted into.
- Open file, drag and drop, copy all.
- Measure editing latency with the 1M-node file and report numbers before going further.

### Phase 12 — Live validation
- Debounced background parse with generation counter.
- Status bar: "Valid JSON" or `line:col` error message.
- Error mark in the text buffer at the failure point, plus a hint in `core`
  (e.g. missing comma, unclosed brace, unexpected end of input).
- Tolerant mode toggle (comments, trailing commas) with auto-fix.

### Phase 13 — Formatting
- Auto-format on paste when the pasted content is valid JSON.
- Format, minify and sort-keys buttons and shortcuts, running on a background thread for large input.

### Phase 14 — Tree viewer
- Virtualized flat-row model built from the parsed document.
- Icons per type (object, array, string, number, boolean, null).
- Container length (`users [3]`, `{5}`) and array indices (`[0]`, `[1]`).
- Expand/collapse per node, expand all, collapse all.

### Phase 15 — Viewer utilities
- Copy node path (`$.users[3].email`) and value (reusing `nodecopy.rs`).
- Search over keys and values (reusing the `core` search engine and `search.rs` patterns).

### Phase 16 — Editor polish
- Line numbers and syntax colors in the editor (`GtkTextView` tags). If performance or code
  complexity is unacceptable, propose `GtkSourceView` with data **before** adding it.
- Two-way sync: selecting a tree node selects its text span; moving the text cursor selects the node.

## Error handling
Parse errors never panic and are never `unwrap`ped; they become status-bar state. File read
failures show a toast/banner. Stale background results are dropped.

## Testing
- `core`: unit tests for hints (each error class, edge cases such as empty input, truncated input,
  deep nesting) and for offset↔node mapping.
- `app`: logic that does not need GTK (generation counter, row building) is kept in plain
  functions with unit tests; UI behavior is checked manually and noted in `TASKS.md` when relevant.

## Performance
Same goals as `SPEC.md`. The Viewer must not block the UI thread on a 1M-node document; typing
latency and tree scrolling are measured in Phase 11 and Phase 14 and reported with numbers.
If a goal is missed, options are proposed before any architecture change.

## Out of scope (later)
Editing values in the tree, node detail panel, jq filter, format conversion.
