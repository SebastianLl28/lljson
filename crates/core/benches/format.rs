//! Formatter benchmark with ~1 million nodes.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use lljson_core::{
    FormatOptions, GenerateOptions, Indent, ParseOptions, format, generate, minify, parse,
};

fn bench(c: &mut Criterion) {
    // Minified input, so formatting cannot benefit from reusing the original indentation.
    let options = GenerateOptions {
        indent: Indent::None,
        ..GenerateOptions::default()
    };
    let text = generate(&options).expect("generate the document");
    let doc = match parse(text.as_str(), &ParseOptions::default()) {
        Ok(parsed) => parsed.document,
        Err(e) => panic!("generated JSON should be valid: {e}"),
    };
    eprintln!(
        "input: {} nodes, {:.1} MiB",
        doc.node_count(),
        text.len() as f64 / 1048576.0
    );

    let mut group = c.benchmark_group("format_1m_nodes");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(8));
    group.throughput(Throughput::Bytes(text.len() as u64));
    group.bench_function("format_2_spaces", |b| {
        b.iter(|| format(black_box(&doc), &FormatOptions::default()))
    });
    group.bench_function("minify", |b| b.iter(|| minify(black_box(&doc))));
    let sorted = FormatOptions {
        indent: Indent::Spaces(2),
        sort_keys: true,
    };
    group.bench_function("sort_keys", |b| b.iter(|| format(black_box(&doc), &sorted)));
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
