//! Benchmark del diff con ~1 millón de nodos por documento (generador de `core`).

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use lljson_core::{
    Changes, DiffOptions, Document, GenerateOptions, IgnoreRule, ParseOptions, diff, generate,
    parse,
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
    let base = doc(Changes::default());
    let same = doc(Changes::default());
    let few = doc(Changes {
        modified: 10,
        ..Changes::default()
    });
    let mixed = doc(Changes {
        modified: 5,
        type_changed: 2,
        removed: 2,
        added: 1,
    });
    // Un solo registro insertado cerca del principio: desplaza todos los índices.
    let shifted = doc(Changes {
        added: 1,
        ..Changes::default()
    });
    eprintln!("nodos por documento: {}", base.node_count());

    let defaults = DiffOptions::default();
    let keyed = DiffOptions {
        array_key: Some("id".to_owned()),
        ..DiffOptions::default()
    };
    let ignoring = DiffOptions {
        ignore: vec![
            IgnoreRule::Key("score".to_owned()),
            IgnoreRule::parse("$[*].address.zip").expect("regla válida"),
        ],
        ..DiffOptions::default()
    };
    let cases = [
        ("identicos", &same, &defaults),
        ("10_cambios", &few, &defaults),
        ("10_cambios_mixtos_por_clave", &mixed, &keyed),
        ("10_cambios_ignorando_rutas", &few, &ignoring),
        ("insercion_al_inicio_por_indice", &shifted, &defaults),
        ("insercion_al_inicio_por_clave", &shifted, &keyed),
    ];
    for (name, right, options) in cases {
        let summary = diff(&base, right, options).summary();
        eprintln!("{name}: {summary:?}");
    }

    let mut group = c.benchmark_group("diff_1m_nodos");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(8));
    for (name, right, options) in cases {
        group.bench_function(name, |b| b.iter(|| diff(black_box(&base), right, options)));
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
