//! Diff benchmark with ~1 million nodes per document (from the `core` generator).

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
    let text = generate(&options).expect("generate the document");
    match parse(text, &ParseOptions::default()) {
        Ok(parsed) => parsed.document,
        Err(e) => panic!("generated JSON should be valid: {e}"),
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
    // One record inserted near the start shifts every index.
    let shifted = doc(Changes {
        added: 1,
        ..Changes::default()
    });
    eprintln!("nodes per document: {}", base.node_count());

    let defaults = DiffOptions::default();
    let keyed = DiffOptions {
        array_key: Some("id".to_owned()),
        ..DiffOptions::default()
    };
    let ignoring = DiffOptions {
        ignore: vec![
            IgnoreRule::Key("score".to_owned()),
            IgnoreRule::parse("$[*].address.zip").expect("valid rule"),
        ],
        ..DiffOptions::default()
    };
    let cases = [
        ("identical", &same, &defaults),
        ("10_changes", &few, &defaults),
        ("10_mixed_changes_by_key", &mixed, &keyed),
        ("10_changes_ignoring_paths", &few, &ignoring),
        ("insertion_at_start_by_index", &shifted, &defaults),
        ("insertion_at_start_by_key", &shifted, &keyed),
    ];
    for (name, right, options) in cases {
        let summary = diff(&base, right, options).summary();
        eprintln!("{name}: {summary:?}");
    }

    let mut group = c.benchmark_group("diff_1m_nodes");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(8));
    for (name, right, options) in cases {
        group.bench_function(name, |b| b.iter(|| diff(black_box(&base), right, options)));
    }
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
