//! Parsing benchmark: the arena-building parser vs serde_json and simd-json.
//! Input: indented 1-million-node document from the `core` generator.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use lljson_core::{GenerateOptions, Mode, ParseOptions, generate, parse};

fn bench(c: &mut Criterion) {
    let text = generate(&GenerateOptions::default()).expect("generate the document");
    let nodes = match parse(text.as_str(), &ParseOptions::default()) {
        Ok(parsed) => parsed.document.node_count(),
        Err(e) => panic!("generated JSON should be valid: {e}"),
    };
    eprintln!(
        "input: {} nodes, {:.1} MiB, arena: {:.1} MiB",
        nodes,
        text.len() as f64 / 1048576.0,
        (nodes * std::mem::size_of::<lljson_core::Node>()) as f64 / 1048576.0
    );

    let mut group = c.benchmark_group("parse_1m_nodes");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));
    group.throughput(Throughput::Bytes(text.len() as u64));

    group.bench_function("lljson_strict", |b| {
        b.iter(|| parse(black_box(text.as_str()), &ParseOptions::default()))
    });
    let tolerant = ParseOptions {
        mode: Mode::Tolerant,
    };
    group.bench_function("lljson_tolerant", |b| {
        b.iter(|| parse(black_box(text.as_str()), &tolerant))
    });
    group.bench_function("serde_json_value", |b| {
        b.iter(|| serde_json::from_str::<serde_json::Value>(black_box(&text)))
    });
    group.bench_function("simd_json_borrowed_value", |b| {
        b.iter_batched(
            || text.as_bytes().to_vec(),
            |mut bytes| {
                simd_json::to_borrowed_value(&mut bytes)
                    .map(|v| {
                        black_box(&v);
                    })
                    .is_ok()
            },
            BatchSize::LargeInput,
        )
    });
    group.bench_function("simd_json_tape", |b| {
        b.iter_batched(
            || text.as_bytes().to_vec(),
            |mut bytes| {
                simd_json::to_tape(&mut bytes)
                    .map(|t| {
                        black_box(&t);
                    })
                    .is_ok()
            },
            BatchSize::LargeInput,
        )
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
