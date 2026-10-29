# SPEC.md — lljson

## Vision
A native Linux tool that brings together what is separate today: semantic JSON diff
(like `jd`), a visual side-by-side view (like Meld) and formatting/transformation (like DevToys).
Offline, instant startup and smooth with huge files (up to 1 million nodes)
without freezing.

## Stack
- **Language:** Rust, edition 2024. Minimum toolchain: **Rust 1.92** (required by `gtk4` 0.11).
- **UI:** GTK4 + libadwaita (gtk4-rs). Plan B if the UI gets too complicated: egui.
- **Parsing:** custom parser (`crates/core/src/parser.rs`) that builds the arena directly
  and supports tolerant mode. **Decision made in Phase 1** with the benchmark
  `crates/core/benches/parse.rs` (1,008,001 nodes, 18.9 MiB, 12 threads, release profile, 2026-10-28):

  | Option | Time | Arena? | Tolerant? |
  |---|---|---|---|
  | lljson strict | ~44 ms | yes | n/a |
  | lljson tolerant | ~40 ms | yes | yes |
  | serde_json → `Value` | ~182 ms | no (needs 2nd pass) | no |
  | simd-json → `BorrowedValue` | ~89 ms | no (needs 2nd pass) | no |
  | simd-json → tape | ~26 ms | no (needs 2nd pass) | no |

  The simd-json tape is faster, but it does not provide the arena or tolerant mode and it decodes
  strings by modifying the buffer (incompatible with ranges over the original and with the
  read-only mmap). The custom parser meets the goal (< 1 s) with a margin of ~20×.
  serde_json and simd-json remain only as dev-dependencies (reference tests and benchmark).
- **Subtree hash:** xxh3 (`xxhash-rust`), deterministic and fast.
  `ahash` for internal hash tables (e.g. matching array elements by key).
- **Large file reading:** memmap2.
- **jq filter (post-MVP):** jaq (pure-Rust jq implementation).
- **Benchmarks:** criterion.
- **Cargo workspace with three crates:**
  - `crates/core` → library: parser, tree, hash, formatter, diff. No UI dependencies.
  - `crates/cli`  → terminal binary (`lljson-cli`) that uses `core` (tests and scripts).
  - `crates/app`  → GTK4 app (`lljson`) that uses `core`.

## Dependency versions (pinned)
Verified on crates.io on **2026-10-28** (latest stable version of each crate).
They are declared in `[workspace.dependencies]` and `Cargo.lock` is versioned.
They are not updated without approval.

| Crate | Version | Use | Crate(s) |
|---|---|---|---|
| `gtk4` | 0.11.5 | UI (requires Rust ≥ 1.92) | app |
| `libadwaita` | 0.9.2 | GNOME widgets and styling (depends on gtk4 ^0.11) | app |
| `glib` | 0.22.10 | Main loop, GObject subclasses | app |
| `gio` | 0.22.10 | `GListModel`, files, clipboard | app |
| `serde` | 1.0.229 | Serialization (options, config) | core, cli |
| `serde_json` | 1.0.154 | Reference in tests and benchmark (dev) | core |
| `simd-json` | 0.19.0 | Comparison in the parsing benchmark (dev) | core |
| `xxhash-rust` | 0.8.19 | xxh3 subtree hash (feature `xxh3`) | core |
| `ahash` | 0.8.12 | Fast HashMap for matching by key | core |
| `memmap2` | 0.9.12 | Memory-mapped reading of large files | core |
| `thiserror` | 2.0.21 | Error types in `core` | core |
| `regex` | 1.13.1 | Regular-expression search (linear time, Unicode) | core |
| `anyhow` | 1.0.104 | Error handling in binaries | cli, app |
| `clap` | 4.6.7 | Command-line arguments (feature `derive`) | cli, app |
| `async-channel` | 2.5.0 | Background thread → UI thread channel | app |
| `criterion` | 0.8.2 | Benchmarks (dev-dependency) | core |
| `jaq-core` | 3.1.2 | jq filter (post-MVP) | core |
| `jaq-std` | 3.0.4 | jaq standard library (post-MVP) | core |
| `jaq-json` | 2.0.3 | JSON values for jaq (post-MVP) | core |
| `egui` / `eframe` | 0.37.0 | Only if plan B is activated | app |

**Detected system libraries** (CachyOS/Arch): `gtk4` 4.24.1 and `libadwaita` 1.10.0.
Recommended compatibility features: `gtk4` with `v4_18` and `libadwaita` with `v1_7` as a
minimum (raise them only if a specific API is needed), so that the Flatpak works on
not-so-recent GNOME runtimes.

## MVP features

### Comparison
- **Semantic diff:** ignores key order and whitespace; compares structure and values.
- **Side-by-side view** with synchronized scrolling and aligned rows (empty padding rows
  where one side has content that the other does not).
- **Colors:** added (green), removed (red), modified (yellow), highlighting the exact value.
- **Type change** (`"1"` vs `1`) detected as an explicit difference.
- **Arrays** compared by index or by a configurable key (e.g. `"id"`).
- **Ignore paths/keys**, configurable (e.g. `timestamp`, `updatedAt`).
- **Navigate between differences:** next/previous with keyboard shortcuts.
- **Collapse equal sections;** expand/collapse nodes.
- **Summary bar:** N added, N removed, N modified.

### Input
- Paste text into each pane, open files, drag and drop.
- Open from the terminal: `lljson a.json b.json` (and `-` to read from stdin).
- Compare the clipboard against a file.
- **Tolerant mode:** accepts trailing commas and comments (JSONC), with an "auto-fix" option.
- Clear syntax errors with line and column.

### Formatting
- Format (configurable indentation), minify, sort keys.

### Utilities
- Copy a node's path (e.g. `$.users[3].email`).
- Copy a node's value.
- Light/dark theme following the system.

### Viewer tab
A second tab for a single document: editable text with live validation (line/column error and
hint), auto-format on paste, and a virtualized tree viewer with type icons, lengths and indices.
Details in `docs/superpowers/specs/2026-10-28-viewer-tab-design.md`.

## Post-MVP features
- jq/JSONPath filter applied to both sides before comparing (flagship feature).
- Export the diff as JSON Patch (RFC 6902) and Merge Patch (RFC 7396).
- Unescape JSON embedded in strings.
- NDJSON and JSON5.
- JSON ↔ YAML ↔ TOML ↔ CSV conversion.
- Move changes from one side to the other (merge).
- Automatic reload if the file changes on disk.
- Command palette (Ctrl+K).
- Configurable numeric tolerance (`1.0` vs `1.0000001`).
- "Compare with…" integration in Nautilus.

## Mandatory architecture (key to performance)
1. **Arena tree:** nodes live in a flat `Vec<Node>` and are referenced by index
   (`u32`), with no `Box`/`Rc` per node. Strings are stored as ranges over the original
   buffer whenever possible (they are only copied if they contain escapes).
2. **Per-subtree hash computed during parsing.** In the diff, if two subtrees have
   the same hash they are considered equal and are not traversed. An object's hash does not
   depend on the order of its keys.
3. **Parsing and diff on background threads.** The UI thread is **never** blocked.
   The UI shows the formatted JSON immediately and applies the diff colors when done,
   with a progress indicator if the operation takes longer than ~150 ms.
4. **Virtualized view:** the UI consumes a flat list of visible rows
   `[node, depth, diff_state, aligned_row]`. `GtkListView`/`GtkColumnView` is used
   with a custom `GListModel`. **Creating one widget per node is forbidden.**
5. **Text on demand:** the text of each row is generated when it is displayed (in the `bind` of the
   row factory), not all up front.
6. **Collapse/expand** modifies only the flat list; it does not rebuild the tree.
7. **Large files** are read with `memmap2`.

### Reference data schema (indicative)
```text
Document { source: Buffer (mmap or String), nodes: Vec<Node>, root: NodeId }
Node     { kind: Null|Bool|Number|String|Array|Object, parent: NodeId,
           first_child: NodeId, next_sibling: NodeId, key: Option<Span>,
           value: Span, hash: u64, line: u32 }
DiffResult { left_status: Vec<DiffStatus>, right_status: Vec<DiffStatus>,
             pairs: Vec<(Option<NodeId>, Option<NodeId>)>, summary: Summary }
Row      { node: NodeId, depth: u16, status: DiffStatus, aligned_row: u32 }
```

## Performance goals
Reference: a file of **1 million nodes**, ordinary machine (4+ cores, SSD, 16 GB RAM).

| Metric | Goal |
|---|---|
| App startup | < 300 ms |
| Parsing | < 1 s |
| Diff of two nearly identical files | < 500 ms |
| Scrolling | smooth at 60 fps |
| Memory with both files loaded | < 500 MB |

Measurements are done with criterion (core) and with documented manual measurements (app).
