//! Coste del hash semántico (xxh3) con ~1 millón de nodos.
//!
//! El hash se calcula dentro del parser (arquitectura obligatoria, punto 2) y no se puede
//! desactivar, así que aquí se estima su peso repitiendo el mismo trabajo sobre el arena ya
//! construido: xxh3 de cada string decodificado, número y clave, y la combinación de los
//! contenedores (miembros sumados, elementos encadenados). Los números se hashean tal cual
//! (sin forma canónica), así que la cifra es una cota inferior aproximada.
//! También se mide comparar dos documentos completos por el hash de la raíz.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use jsonlens_core::{Document, GenerateOptions, Kind, NodeId, ParseOptions, generate, parse};
use xxhash_rust::xxh3::xxh3_64_with_seed;

const TAG_NULL: u64 = 1;
const TAG_FALSE: u64 = 2;
const TAG_TRUE: u64 = 3;
const TAG_NUMBER: u64 = 4;
const TAG_STRING: u64 = 5;
const TAG_ARRAY: u64 = 6;
const TAG_OBJECT: u64 = 7;
const TAG_MEMBER: u64 = 8;

/// Recalcula un hash por nodo, de abajo arriba (en el arena los hijos van detrás del padre).
fn rehash(doc: &Document, hashes: &mut [u64]) -> u64 {
    for i in (0..doc.node_count()).rev() {
        let id = NodeId::from_index(i);
        let h = match doc.kind(id) {
            Some(Kind::Null) | None => xxh3_64_with_seed(&[], TAG_NULL),
            Some(Kind::Bool) => {
                let tag = if doc.bool(id) == Some(true) {
                    TAG_TRUE
                } else {
                    TAG_FALSE
                };
                xxh3_64_with_seed(&[], tag)
            }
            Some(Kind::Number) => {
                xxh3_64_with_seed(doc.number(id).unwrap_or("").as_bytes(), TAG_NUMBER)
            }
            Some(Kind::String) => {
                let text = doc.string(id).unwrap_or_default();
                xxh3_64_with_seed(text.as_bytes(), TAG_STRING)
            }
            Some(Kind::Array) => {
                let mut acc = TAG_ARRAY;
                let mut count = 0u32;
                for child in doc.children(id) {
                    acc = xxh3_64_with_seed(&hashes[child.index()].to_le_bytes(), acc);
                    count += 1;
                }
                xxh3_64_with_seed(&count.to_le_bytes(), acc)
            }
            Some(Kind::Object) => {
                let mut sum = 0u64;
                let mut count = 0u32;
                for child in doc.children(id) {
                    let key = doc.key(child).unwrap_or_default();
                    let member =
                        xxh3_64_with_seed(key.as_bytes(), hashes[child.index()] ^ TAG_MEMBER);
                    sum = sum.wrapping_add(member);
                    count += 1;
                }
                let mut bytes = [0u8; 12];
                bytes[..8].copy_from_slice(&sum.to_le_bytes());
                bytes[8..].copy_from_slice(&count.to_le_bytes());
                xxh3_64_with_seed(&bytes, TAG_OBJECT)
            }
        };
        hashes[i] = h;
    }
    hashes.first().copied().unwrap_or_default()
}

fn bench(c: &mut Criterion) {
    let text = generate(&GenerateOptions::default()).expect("generar el documento");
    let parse_doc = || match parse(text.as_str(), &ParseOptions::default()) {
        Ok(parsed) => parsed.document,
        Err(e) => panic!("el JSON generado debería ser válido: {e}"),
    };
    let doc = parse_doc();
    let other = parse_doc();
    let mut hashes = vec![0u64; doc.node_count()];
    eprintln!("nodos: {}", doc.node_count());

    let mut group = c.benchmark_group("hash_1m_nodos");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(8));
    group.bench_function("recalcular_hashes_estimado", |b| {
        b.iter(|| rehash(black_box(&doc), &mut hashes))
    });
    group.bench_function("comparar_raices", |b| {
        b.iter(|| black_box(&doc).hash(doc.root()) == black_box(&other).hash(other.root()))
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
