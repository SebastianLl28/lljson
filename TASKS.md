# TASKS.md — lljson work plan

Mark each task with `[x]` when completed (build, test and clippy green + commit).
Do not advance to the next phase without confirmation.

## Phase 0 — Setup
- [x] Verify/install system dependencies: Rust toolchain ≥ 1.92 (rustup),
      gtk4 and libadwaita development packages, pkgconf. Detect the distribution and give me the command.
- [x] Create the Cargo workspace with the `core`, `cli` and `app` crates (edition 2024,
      `[workspace.dependencies]` with the versions from SPEC.md, `.gitignore`, versioned `Cargo.lock`).
- [x] "Hello world" libadwaita app that opens a window (confirm that everything compiles).

## Phase 1 — Core: parser and tree
- [x] Arena tree structure (`Vec<Node>`, `u32` indices, strings as ranges).
- [x] Strict parser.
- [x] Tolerant mode (trailing commas, `//` and `/* */` comments) and "auto-fix" option.
- [x] Errors with line and column.
- [x] Choose the parsing strategy (serde_json / simd-json / custom) with a preliminary benchmark.
- [x] Unit tests with normal and edge cases (empty, deep nesting,
      unicode and escapes, large numbers, truncated files).

## Phase 2 — Hash and formatter
- [x] Per-subtree hash (xxh3) during parsing, independent of key order.
- [x] Format (configurable indentation).
- [x] Minify.
- [x] Sort keys.
- [x] Tests.

## Phase 3 — Diff engine
- [x] Semantic diff using hashes to skip equal subtrees.
- [x] Arrays by index and by configurable key.
- [x] Type change detection.
- [x] Ignore paths.
- [x] Diff result with summary (added/removed/modified).
- [x] Tests.

## Phase 4 — CLI and benchmarks
- [x] CLI: `lljson-cli diff a.json b.json`, `format`, `minify`, stdin support (`-`).
- [x] Test JSON generator (1 million nodes, with few-changes variants).
- [x] Benchmarks with criterion (parsing, hash, diff, formatting, memory).
- [x] Report results against the performance goals.

## Phase 5 — App: structure
- [x] libadwaita window with two side-by-side panes.
- [x] Open files, paste, drag and drop.
- [x] Terminal arguments (`lljson a.json b.json`, `-` for stdin).
- [x] Parsing on a background thread with a progress indicator (> ~150 ms).

## Phase 6 — App: virtualized view
- [x] Flat list of rows + custom `GListModel` + `GtkListView`.
- [x] Syntax colors.
- [x] Expand/collapse (modifying only the flat list).
- [x] Synchronized scrolling and aligned rows between panes.
- [x] Test with the 1-million-node file (smoothness and memory).

## Phase 7 — App: visual diff
- [x] Diff colors per value.
- [x] Summary bar.
- [x] Next/previous difference navigation with shortcuts.
- [x] Collapse equal sections.

## Phase 8 — App: MVP utilities
- [x] Configure array key and ignore paths.
- [x] Copy a node's path and value.
- [x] Compare clipboard against file.
- [x] Format/minify/sort buttons.
- [x] System light/dark theme.

## Phase 9 — Packaging
- [x] `.desktop` file, icon, AppStream metainfo.
- [ ] Flatpak manifest and local test build.
- [ ] README with screenshots and instructions.

## Phase 10 — Post-MVP (in the order I choose)
- [ ] jq/JSONPath filter applied to both sides before comparing (jaq).
- [ ] Export diff as JSON Patch (RFC 6902) and Merge Patch (RFC 7396).
- [ ] Unescape JSON embedded in strings.
- [ ] NDJSON and JSON5 support.
- [ ] JSON ↔ YAML ↔ TOML ↔ CSV conversion.
- [ ] Move changes from one side to the other (merge).
- [x] Search keys and values per pane (ignores case and accents; exact and regex options).
- [ ] Automatic reload if the file changes on disk.
- [ ] Command palette (Ctrl+K).
- [ ] Configurable numeric tolerance.
- [ ] "Compare with…" integration in Nautilus.

## Phase 11 — Viewer tab: tabs and basic editor
Spec: `docs/superpowers/specs/2026-10-28-viewer-tab-design.md`
- [x] Tab switcher (Compare / Viewer); Compare unchanged.
- [x] Viewer editor: editable text, can be emptied and pasted into.
- [x] Open file, drag and drop, copy all.
- [x] Measure editing latency with the 1M-node file and report numbers.
- [x] Format valid JSON off the UI thread when opening a file or dropping text (long-line fix).
- [x] Insert opened text in chunks from idle callbacks (no UI freeze on large files).
- [x] Re-measure the Viewer with the 1M-node files and update `BENCHMARKS.md`.

## Phase 12 — Viewer tab: live validation
- [x] Debounced background parse with generation counter.
- [x] Status bar with valid / error `line:col`.
- [x] Error hints in `core` (with tests) and error mark in the editor.
- [x] Tolerant mode toggle with auto-fix.

## Phase 13 — Viewer tab: formatting
- [x] Auto-format on paste when the content is valid JSON.
- [x] Format, minify and sort-keys buttons and shortcuts (background thread for large input).

## Phase 14 — Viewer tab: tree viewer
- [x] Virtualized flat-row model from the parsed document.
- [x] Type icons, container length and array indices.
- [x] Expand/collapse per node, expand all, collapse all.

## Phase 15 — Viewer tab: utilities
- [x] Copy node path and value.
- [x] Search over keys and values.

## Phase 16 — Viewer tab: editor polish
- [x] Line numbers and syntax colors.
- [ ] Two-way sync between tree selection and text cursor (offset↔node mapping in `core`, with tests).
