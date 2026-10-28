//! Benchmark preliminar de estrategias de parseo (Fase 1).
//! Compara el parser propio (que construye el arena) con serde_json y simd-json.

use std::fmt::Write;
use std::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use jsonlens_core::{Mode, ParseOptions, parse};

/// Genera un array de registros con ~12 nodos cada uno, indentado con 2 espacios.
fn generate(records: usize) -> String {
    let mut out = String::from("[\n");
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in 0..records {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let score = (seed >> 40) as f64 / 1000.0;
        let active = seed & 1 == 0;
        let sep = if i + 1 == records { "" } else { "," };
        let _ = write!(
            out,
            "  {{\n    \"id\": {i},\n    \"name\": \"usuario_{i}\",\n    \"email\": \"u{i}@example.com\",\n    \"active\": {active},\n    \"score\": {score},\n    \"tags\": [\"alpha\", \"beta\"],\n    \"address\": {{\n      \"city\": \"Ciudad \\\"{}\\\"\",\n      \"zip\": null\n    }}\n  }}{sep}\n",
            i % 97
        );
    }
    out.push(']');
    out
}

fn bench(c: &mut Criterion) {
    let text = generate(84_000);
    let nodes = match parse(text.as_str(), &ParseOptions::default()) {
        Ok(parsed) => parsed.document.node_count(),
        Err(e) => panic!("el JSON generado debería ser válido: {e}"),
    };
    eprintln!(
        "entrada: {} nodos, {:.1} MiB, arena: {:.1} MiB",
        nodes,
        text.len() as f64 / 1048576.0,
        (nodes * std::mem::size_of::<jsonlens_core::Node>()) as f64 / 1048576.0
    );

    let mut group = c.benchmark_group("parse_1m_nodos");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));
    group.throughput(Throughput::Bytes(text.len() as u64));

    group.bench_function("jsonlens_estricto", |b| {
        b.iter(|| parse(black_box(text.as_str()), &ParseOptions::default()))
    });
    let tolerant = ParseOptions {
        mode: Mode::Tolerant,
    };
    group.bench_function("jsonlens_tolerante", |b| {
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
