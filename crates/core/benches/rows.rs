//! Benchmark de la lista plana de filas de la vista con ~1 millón de nodos por documento.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use jsonlens_core::{
    Changes, DiffOptions, Document, GenerateOptions, ParseOptions, Rows, generate, parse,
};

fn doc(changes: Changes) -> Document {
    let options = GenerateOptions {
        changes,
        ..GenerateOptions::default()
    };
    let text = generate(&options).expect("generar el documento");
    match parse(text, &ParseOptions::default()) {
        Ok(parsed) => parsed.document,
        Err(e) => panic!("el JSON generado debería ser válido: {e}"),
    }
}

fn bench(c: &mut Criterion) {
    let left = doc(Changes::default());
    let right = doc(Changes {
        modified: 10,
        ..Changes::default()
    });
    let (l, r) = (Some(&left), Some(&right));
    let rows = Rows::new(l, r, DiffOptions::default());
    eprintln!(
        "nodos por documento: {}, filas: {} ({:.1} MiB)",
        left.node_count(),
        rows.len(),
        (rows.len() * 12) as f64 / 1048576.0
    );

    let mut group = c.benchmark_group("filas_1m_nodos");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function("construir_un_documento", |b| {
        b.iter(|| Rows::new(black_box(l), None, DiffOptions::default()))
    });
    group.bench_function("construir_dos_documentos", |b| {
        b.iter(|| Rows::new(black_box(l), r, DiffOptions::default()))
    });
    group.bench_function("colapsar_raiz", |b| {
        b.iter_batched(
            || Rows::new(l, r, DiffOptions::default()),
            |mut rows| rows.toggle(0, l, r),
            BatchSize::LargeInput,
        )
    });
    group.bench_function("expandir_raiz", |b| {
        b.iter_batched(
            || {
                let mut rows = Rows::new(l, r, DiffOptions::default());
                rows.toggle(0, l, r);
                rows
            },
            |mut rows| rows.toggle(0, l, r),
            BatchSize::LargeInput,
        )
    });
    group.bench_function("colapsar_un_registro_en_medio", |b| {
        b.iter_batched(
            || Rows::new(l, r, DiffOptions::default()),
            // Fila 0: raíz; cada registro ocupa 15 filas (apertura, 5 escalares,
            // `tags` y `address` con 4 filas cada uno, y cierre).
            |mut rows| rows.toggle(1 + 15 * (left.node_count() / 24), l, r),
            BatchSize::LargeInput,
        )
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
