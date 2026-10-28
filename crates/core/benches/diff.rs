//! Benchmark del diff con ~1 millón de nodos por documento.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use jsonlens_core::{DiffOptions, Document, ParseOptions, diff, parse};

/// Array de registros (~12 nodos cada uno). `modify` cambia el nombre de esos registros;
/// `insert_first` añade un registro al principio (desplaza los índices).
fn generate(records: usize, modify: &[usize], insert_first: bool) -> String {
    let mut out = String::from("[");
    if insert_first {
        out.push_str(r#"{"id":-1,"name":"nuevo","tags":[],"score":0,"active":false,"email":"","address":{"city":"","zip":null}},"#);
    }
    for i in 0..records {
        if i > 0 {
            out.push(',');
        }
        let name = if modify.contains(&i) {
            format!("cambiado_{i}")
        } else {
            format!("usuario_{i}")
        };
        out.push_str(&format!(
            r#"{{"id":{i},"name":"{name}","tags":["alpha","beta"],"score":{}.5,"active":true,"email":"u{i}@example.com","address":{{"city":"Ciudad {}","zip":null}}}}"#,
            i % 1000,
            i % 97
        ));
    }
    out.push(']');
    out
}

fn doc(text: &str) -> Document {
    match parse(text, &ParseOptions::default()) {
        Ok(parsed) => parsed.document,
        Err(e) => panic!("el JSON generado debería ser válido: {e}"),
    }
}

fn bench(c: &mut Criterion) {
    const RECORDS: usize = 84_000;
    let modified: Vec<usize> = (0..10).map(|k| k * 8_000 + 123).collect();
    let base_text = generate(RECORDS, &[], false);
    let base = doc(&base_text);
    let same = doc(&base_text);
    let few = doc(&generate(RECORDS, &modified, false));
    let shifted = doc(&generate(RECORDS, &[], true));
    eprintln!("nodos por documento: {}", base.node_count());

    let defaults = DiffOptions::default();
    let keyed = DiffOptions {
        array_key: Some("id".to_owned()),
        ..DiffOptions::default()
    };
    for (name, right, options) in [
        ("identicos", &same, &defaults),
        ("10_cambios", &few, &defaults),
        ("insercion_al_inicio_por_indice", &shifted, &defaults),
        ("insercion_al_inicio_por_clave", &shifted, &keyed),
    ] {
        let changes = diff(&base, right, options).changes().len();
        eprintln!("{name}: {changes} cambios");
    }

    let mut group = c.benchmark_group("diff_1m_nodos");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(8));
    group.bench_function("identicos", |b| {
        b.iter(|| diff(black_box(&base), &same, &defaults))
    });
    group.bench_function("10_cambios", |b| {
        b.iter(|| diff(black_box(&base), &few, &defaults))
    });
    group.bench_function("insercion_al_inicio_por_indice", |b| {
        b.iter(|| diff(black_box(&base), &shifted, &defaults))
    });
    group.bench_function("insercion_al_inicio_por_clave", |b| {
        b.iter(|| diff(black_box(&base), &shifted, &keyed))
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
