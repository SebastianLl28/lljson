# CLAUDE.md — Working rules for lljson

This file defines how work is done in this repository. It is mandatory in every session.

## At the start of each session
1. Read `SPEC.md` (what the app is, mandatory architecture, performance goals and pinned versions).
2. Read `TASKS.md` and identify the current phase and the next unchecked task.
3. Briefly summarize where the project stands before starting work.

## Workflow
- Work on **one phase at a time**. Do not start the next phase without my explicit confirmation.
- Within a phase, proceed task by task.
- When finishing each task, these must pass without errors:
  - `cargo build --workspace`
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
- Mark the task as completed in `TASKS.md` (`[ ]` → `[x]`).
- Make a **git commit when finishing each task**, with a descriptive message
  (suggested format: `phase N: short description`).
- Commit messages are written in English with no Co-Authored-By trailer.
- If a task cannot be completed, do not mark it: explain what is missing and why.

## Architecture
- Follow the **"Mandatory architecture"** section of `SPEC.md` to the letter.
- If you think something in the architecture or the stack should change, **ask me first**
  and explain the reason with data (benchmarks, API limitations, etc.).
- `crates/core` must not depend on GTK or on any UI library.
- **Never block the UI thread.** Parsing, diffing, formatting of large files and disk reads
  run on background threads; results return to the main thread through a channel
  (`async-channel` + `glib::MainContext::spawn_local`).
- **Never create one widget per JSON node.** The UI always uses virtualized views
  (`GtkListView`/`GtkColumnView` with a custom `GListModel`).

## Code quality
- No `unwrap()` or `expect()` in app or library code. Errors are propagated
  with `Result` and, in the app, shown to the user (toast, banner or dialog).
  Allowed exception: tests, benchmarks and cases whose impossibility is proven
  and documented with a comment.
- No `unsafe` unless it is essential (e.g. `memmap2`), and always with a `// SAFETY:` comment
  justifying the invariant.
- All new code in `core` comes with unit tests, including edge cases.

## Dependencies
- Use **only** the versions pinned in the dependency table of `SPEC.md`.
- Declare dependencies in `[workspace.dependencies]` of the root `Cargo.toml` and
  reference them from each crate with `workspace = true`. `Cargo.lock` is versioned.
- Do not add new dependencies without asking me.
- If a gtk4-rs / libadwaita API does not compile, consult the documentation for **that exact
  version** on docs.rs (e.g. `https://docs.rs/gtk4/0.11.5/gtk4/`) before guessing.

## Performance
- The performance goals in `SPEC.md` are requirements, not suggestions.
- If a goal is not met, tell me with the benchmark numbers (criterion), the machine
  and the file used, and propose options before changing the architecture.
