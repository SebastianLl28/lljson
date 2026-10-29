//! Memory with two ~1 million node documents loaded and compared.
//!
//! criterion only measures time, so this harness-less benchmark reads the process RSS from
//! `/proc/self/status` (Linux only) at each stage: mmap, parse, diff. Also prints the
//! theoretical size of each structure. Run with `cargo bench -p lljson-core --bench memory`.

use std::path::{Path, PathBuf};

use lljson_core::{
    Changes, DiffOptions, Document, GenerateOptions, Node, ParseOptions, Source, diff, generate,
    parse,
};

const MIB: f64 = 1024.0 * 1024.0;

/// Value in KiB of a `/proc/self/status` field (e.g. `VmRSS`, `VmHWM`).
fn status_kib(field: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix(field)?.strip_prefix(':'))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
}

fn rss_mib() -> f64 {
    status_kib("VmRSS").map_or(f64::NAN, |kib| kib as f64 / 1024.0)
}

fn write_document(dir: &Path, name: &str, changes: Changes) -> PathBuf {
    let options = GenerateOptions {
        changes,
        ..GenerateOptions::default()
    };
    let text = generate(&options).expect("generate the document");
    let path = dir.join(name);
    std::fs::write(&path, text).expect("write the document");
    path
}

fn load(path: &Path) -> Document {
    let source = Source::from_file(path).expect("open the document");
    match parse(source, &ParseOptions::default()) {
        Ok(parsed) => parsed.document,
        Err(e) => panic!("generated JSON should be valid: {e}"),
    }
}

fn main() {
    // `cargo test --benches` runs benchmarks in test mode; nothing to test here.
    if std::env::args().any(|arg| arg == "--test") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("lljson-memory-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp directory");
    let left_path = write_document(&dir, "a.json", Changes::default());
    let right_path = write_document(
        &dir,
        "b.json",
        Changes {
            modified: 10,
            ..Changes::default()
        },
    );

    // Reset the peak (`VmHWM`) so file generation is not counted.
    let _ = std::fs::write("/proc/self/clear_refs", "5");
    let start = rss_mib();
    let left = load(&left_path);
    let right = load(&right_path);
    let parsed = rss_mib();
    let result = diff(&left, &right, &DiffOptions::default());
    let diffed = rss_mib();
    let peak = status_kib("VmHWM").map_or(f64::NAN, |kib| kib as f64 / 1024.0);

    let source = (left.source_bytes().len() + right.source_bytes().len()) as f64 / MIB;
    let nodes = left.node_count() + right.node_count();
    let arena = (nodes * size_of::<Node>()) as f64 / MIB;
    println!("documents: 2 x {} nodes", left.node_count());
    println!("theoretical: text {source:.1} MiB (mmap) + arena {arena:.1} MiB");
    println!("  Node size: {} bytes", size_of::<Node>());
    println!("RSS initial:        {start:8.1} MiB");
    println!(
        "RSS after parse:   {parsed:8.1} MiB (+{:.1})",
        parsed - start
    );
    println!(
        "RSS after diff:    {diffed:8.1} MiB (+{:.1})",
        diffed - parsed
    );
    println!("peak since start: {peak:6.1} MiB");
    println!("changes: {:?}", result.summary());

    let _ = std::fs::remove_dir_all(dir);
}
