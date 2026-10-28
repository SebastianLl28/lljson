//! Benchmark del formateador con ~1 millón de nodos.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use jsonlens_core::{FormatOptions, Indent, ParseOptions, format, minify, parse};

/// Array de registros con ~12 nodos cada uno, minificado y con claves desordenadas.
fn generate(records: usize) -> String {
    let mut out = String::from("[");
    for i in 0..records {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            r#"{{"name":"usuario_{i}","id":{i},"tags":["beta","alpha"],"score":{}.5,"active":true,"email":"u{i}@example.com","address":{{"zip":null,"city":"Ciudad {}"}}}}"#,
            i % 1000,
            i % 97
        ));
    }
    out.push(']');
    out
}

fn bench(c: &mut Criterion) {
    let text = generate(84_000);
    let doc = match parse(text.as_str(), &ParseOptions::default()) {
        Ok(parsed) => parsed.document,
        Err(e) => panic!("el JSON generado debería ser válido: {e}"),
    };
    eprintln!(
        "entrada: {} nodos, {:.1} MiB",
        doc.node_count(),
        text.len() as f64 / 1048576.0
    );

    let mut group = c.benchmark_group("format_1m_nodos");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(8));
    group.throughput(Throughput::Bytes(text.len() as u64));
    group.bench_function("formatear_2_espacios", |b| {
        b.iter(|| format(black_box(&doc), &FormatOptions::default()))
    });
    group.bench_function("minificar", |b| b.iter(|| minify(black_box(&doc))));
    let sorted = FormatOptions {
        indent: Indent::Spaces(2),
        sort_keys: true,
    };
    group.bench_function("ordenar_claves", |b| {
        b.iter(|| format(black_box(&doc), &sorted))
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
