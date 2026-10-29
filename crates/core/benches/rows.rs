//! Benchmark of the flat row list with ~1 million nodes per document.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use lljson_core::{
    Changes, DiffOptions, Direction, Document, GenerateOptions, ParseOptions, Rows, diff, generate,
    parse,
};

fn doc(changes: Changes) -> Document {
    let options = GenerateOptions {
        changes,
        ..GenerateOptions::default()
    };
    let text = generate(&options).expect("generate the document");
    match parse(text, &ParseOptions::default()) {
        Ok(parsed) => parsed.document,
        Err(e) => panic!("generated JSON should be valid: {e}"),
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
        "nodes per document: {}, rows: {} ({:.1} MiB)",
        left.node_count(),
        rows.len(),
        (rows.len() * 12) as f64 / 1048576.0
    );

    let mut group = c.benchmark_group("rows_1m_nodes");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function("build_one_document", |b| {
        b.iter(|| Rows::new(black_box(l), None, DiffOptions::default()))
    });
    group.bench_function("build_two_documents", |b| {
        b.iter(|| Rows::new(black_box(l), r, DiffOptions::default()))
    });
    // Worst case of "next difference": scan every row without finding one.
    let same = diff(&left, &left, &DiffOptions::default());
    let same_rows = Rows::new(Some(&left), Some(&left), DiffOptions::default());
    group.bench_function("find_change_full_scan", |b| {
        b.iter(|| {
            same_rows.find_change(
                black_box(Some(0)),
                Direction::Forward,
                Some(&left),
                Some(&left),
                &same,
            )
        })
    });
    // Realistic case: 10 spread-out changes, stop to stop.
    let changed = diff(&left, &right, &DiffOptions::default());
    group.bench_function("find_change_next", |b| {
        b.iter(|| rows.find_change(black_box(Some(0)), Direction::Forward, l, r, &changed))
    });
    group.bench_function("collapse_root", |b| {
        b.iter_batched(
            || Rows::new(l, r, DiffOptions::default()),
            |mut rows| rows.toggle(0, l, r),
            BatchSize::LargeInput,
        )
    });
    group.bench_function("expand_root", |b| {
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
    group.bench_function("collapse_middle_record", |b| {
        b.iter_batched(
            || Rows::new(l, r, DiffOptions::default()),
            // Row 0 is the root; each record takes 15 rows (open, 5 scalars, `tags` and
            // `address` at 4 rows each, close).
            |mut rows| rows.toggle(1 + 15 * (left.node_count() / 24), l, r),
            BatchSize::LargeInput,
        )
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
