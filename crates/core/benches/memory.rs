//! Memoria con dos documentos de ~1 millón de nodos cargados y comparados.
//!
//! criterion solo mide tiempo, así que este benchmark (sin harness) mide el RSS del proceso
//! leyendo `/proc/self/status` (solo Linux) en cada etapa: abrir los archivos con `memmap2`,
//! parsear y hacer el diff. Muestra también el tamaño teórico de cada estructura.
//! Ejecutar con `cargo bench -p jsonlens-core --bench memory`.

use std::path::{Path, PathBuf};

use jsonlens_core::{
    Changes, DiffOptions, Document, GenerateOptions, Node, ParseOptions, Source, diff, generate,
    parse,
};

const MIB: f64 = 1024.0 * 1024.0;

/// Valor en KiB de un campo de `/proc/self/status` (p. ej. `VmRSS`, `VmHWM`).
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
    let text = generate(&options).expect("generar el documento");
    let path = dir.join(name);
    std::fs::write(&path, text).expect("escribir el documento");
    path
}

fn load(path: &Path) -> Document {
    let source = Source::from_file(path).expect("abrir el documento");
    match parse(source, &ParseOptions::default()) {
        Ok(parsed) => parsed.document,
        Err(e) => panic!("el JSON generado debería ser válido: {e}"),
    }
}

fn main() {
    // `cargo test --benches` ejecuta los benchmarks en modo prueba: aquí no hay nada que probar.
    if std::env::args().any(|arg| arg == "--test") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("jsonlens-memory-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("crear directorio temporal");
    let left_path = write_document(&dir, "a.json", Changes::default());
    let right_path = write_document(
        &dir,
        "b.json",
        Changes {
            modified: 10,
            ..Changes::default()
        },
    );

    // Reinicia el pico (`VmHWM`) para no contar la generación de los archivos.
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
    println!("documentos: 2 × {} nodos", left.node_count());
    println!("teórico: texto {source:.1} MiB (mmap) + arena {arena:.1} MiB");
    println!("  tamaño de Node: {} bytes", size_of::<Node>());
    println!("RSS inicial:        {start:8.1} MiB");
    println!(
        "RSS tras parsear:   {parsed:8.1} MiB (+{:.1})",
        parsed - start
    );
    println!(
        "RSS tras el diff:   {diffed:8.1} MiB (+{:.1})",
        diffed - parsed
    );
    println!("pico desde el inicio: {peak:6.1} MiB");
    println!("cambios: {:?}", result.summary());

    let _ = std::fs::remove_dir_all(dir);
}
