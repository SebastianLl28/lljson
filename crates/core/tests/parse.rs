//! Tests de integración del parser: comparación con serde_json como referencia,
//! invariantes del arena y casos extremos.

use jsonlens_core::{Document, ErrorKind, Kind, Mode, NodeId, ParseOptions, Parsed, Source, parse};
use serde_json::Value;

fn strict() -> ParseOptions {
    ParseOptions::default()
}

fn tolerant() -> ParseOptions {
    ParseOptions {
        mode: Mode::Tolerant,
    }
}

fn parse_ok(text: &str, options: &ParseOptions) -> Parsed {
    match parse(text, options) {
        Ok(parsed) => parsed,
        Err(e) => panic!("{text:?} debería parsear: {e}"),
    }
}

/// Convierte un nodo a `serde_json::Value` (recursivo: solo para documentos poco profundos).
fn to_value(doc: &Document, id: NodeId) -> Value {
    match doc.kind(id) {
        Some(Kind::Null) | None => Value::Null,
        Some(Kind::Bool) => Value::Bool(doc.bool(id).unwrap_or_default()),
        Some(Kind::Number) => {
            serde_json::from_str(doc.number(id).unwrap_or("null")).unwrap_or(Value::Null)
        }
        Some(Kind::String) => Value::String(doc.string(id).unwrap_or_default().into_owned()),
        Some(Kind::Array) => Value::Array(doc.children(id).map(|c| to_value(doc, c)).collect()),
        Some(Kind::Object) => Value::Object(
            doc.children(id)
                .map(|c| {
                    (
                        doc.key(c).unwrap_or_default().into_owned(),
                        to_value(doc, c),
                    )
                })
                .collect(),
        ),
    }
}

/// Comprueba las invariantes estructurales del arena.
fn check_invariants(doc: &Document) {
    let root = doc.root();
    assert_eq!(doc.node(root).and_then(|n| n.parent()), None);
    for (index, node) in doc.nodes().iter().enumerate() {
        let id = NodeId::from_index(index);
        let children: Vec<_> = doc.children(id).collect();
        assert_eq!(
            children.len() as u32,
            node.child_count(),
            "child_count del nodo {index}"
        );
        assert!(node.kind().is_container() || children.is_empty());
        let mut previous = index;
        for child in children {
            let child_node = doc.node(child).expect("hijo existente");
            assert_eq!(child_node.parent(), Some(id));
            assert!(child.index() > previous, "los hijos van en preorden");
            previous = child.index();
            assert_eq!(child_node.key_span().is_some(), node.kind() == Kind::Object);
        }
    }
}

const VALID: &[&str] = &[
    "0",
    "-0",
    "-0.0",
    "1.5e-10",
    "123456789012345678901234567890",
    // `1e400` es JSON válido pero serde_json lo rechaza (no cabe en f64);
    // se cubre en `numbers_keep_their_exact_text`.
    r#""""#,
    r#""texto""#,
    "true",
    "false",
    "null",
    "[]",
    "{}",
    "  \r\n\t [ \r\n ] \n",
    "[[[[[]]]]]",
    r#"[1, "dos", 3.0, true, false, null, [], {}]"#,
    r#"{"a": {"b": {"c": [1, {"d": null}]}}}"#,
    r#"{"": "clave vacía", " ": "espacio"}"#,
    r#"{"ñandú": "ü", "中文": "日本語", "emoji": "😀🎉"}"#,
    r#"{"esc\u0061pe": "\u00e9\u4e2d\ud83d\ude00", "nul": "\u0000"}"#,
    r#"["\"", "\\", "\/", "\b", "\f", "\n", "\r", "\t"]"#,
    r#"["// no es comentario", "/* tampoco */"]"#,
    "{\r\n  \"crlf\": [1,\r\n 2]\r\n}",
];

#[test]
fn matches_serde_json_on_valid_documents() {
    for text in VALID {
        for options in [strict(), tolerant()] {
            let parsed = parse_ok(text, &options);
            check_invariants(&parsed.document);
            assert!(parsed.fixes.is_empty(), "{text:?} no tiene correcciones");
            let expected: Value = match serde_json::from_str(text) {
                Ok(v) => v,
                Err(e) => panic!("serde_json debería aceptar {text:?}: {e}"),
            };
            let doc = &parsed.document;
            assert_eq!(to_value(doc, doc.root()), expected, "{text:?}");
        }
    }
}

#[test]
fn numbers_keep_their_exact_text() {
    for text in ["123456789012345678901234567890", "1e400", "-0.0", "1.50"] {
        let parsed = parse_ok(text, &strict());
        let doc = &parsed.document;
        assert_eq!(doc.number(doc.root()), Some(text));
    }
}

const INVALID: &[&str] = &[
    "",
    "   ",
    "[",
    "]",
    "{",
    r#"{"a"}"#,
    r#"{"a":}"#,
    r#"{"a" "b"}"#,
    r#"{a: 1}"#,
    "{'a': 1}",
    "[1 2]",
    "[1,,2]",
    "[1,]",
    r#"{"a":1,}"#,
    "01",
    "1.",
    ".1",
    "+1",
    "-",
    "1e",
    "NaN",
    "Infinity",
    "tru",
    "nul",
    "True",
    r#""sin cerrar"#,
    "\"tab\tcrudo\"",
    r#""\x""#,
    r#""\u12""#,
    r#""\ud800""#,
    r#""\udc00x""#,
    "1 2",
    "[] []",
    "// comentario\n1",
    "/* c */ 1",
];

#[test]
fn rejects_what_serde_json_rejects() {
    for text in INVALID {
        assert!(
            serde_json::from_str::<Value>(text).is_err(),
            "serde_json acepta {text:?}"
        );
        let e = match parse(*text, &strict()) {
            Ok(_) => panic!("{text:?} debería fallar"),
            Err(e) => e,
        };
        assert!(e.offset <= text.len());
        assert!(e.line >= 1 && e.column >= 1);
    }
}

#[test]
fn deep_nesting_does_not_overflow_the_stack() {
    const DEPTH: usize = 200_000;
    let arrays = format!("{}{}", "[".repeat(DEPTH), "]".repeat(DEPTH));
    let parsed = parse_ok(&arrays, &strict());
    assert_eq!(parsed.document.node_count(), DEPTH);

    let objects = format!("{}null{}", r#"{"a":"#.repeat(DEPTH), "}".repeat(DEPTH));
    let parsed = parse_ok(&objects, &strict());
    assert_eq!(parsed.document.node_count(), DEPTH + 1);

    let unclosed = "[".repeat(DEPTH);
    let e = parse(unclosed, &strict()).err().map(|e| e.kind);
    assert_eq!(
        e,
        Some(ErrorKind::UnexpectedEnd {
            expected: "un valor"
        })
    );
}

const SAMPLE: &str = r#"{
  "usuarios": [
    {"id": 1, "nombre": "Ana", "tags": ["a", "b"], "activo": true},
    {"id": 2, "nombre": "Luis \"Lucho\"", "extra": null, "puntos": -12.5e3}
  ],
  "total": 2
}"#;

#[test]
fn every_truncated_prefix_fails_cleanly() {
    for end in 0..SAMPLE.len() {
        let Some(prefix) = SAMPLE.get(..end) else {
            continue;
        };
        for options in [strict(), tolerant()] {
            assert!(
                parse(prefix, &options).is_err(),
                "el prefijo de {end} bytes debería fallar"
            );
        }
    }
    assert!(parse(SAMPLE, &strict()).is_ok());
}

#[test]
fn corrupted_bytes_never_panic() {
    let replacements = [
        b'"', b'\\', b',', b':', b'{', b'}', b'[', b']', b'/', b'*', b'0', 0x00, 0xFF,
    ];
    for pos in 0..SAMPLE.len() {
        for &byte in &replacements {
            let mut bytes = SAMPLE.as_bytes().to_vec();
            bytes[pos] = byte;
            for options in [strict(), tolerant()] {
                if let Ok(parsed) = parse(bytes.clone(), &options) {
                    check_invariants(&parsed.document);
                }
            }
        }
    }
}

#[test]
fn tolerant_mode_round_trips_through_autocorrect() {
    let text = "{\n  // usuarios\n  \"a\": [1, 2, /* tres */ 3,],\n  \"b\": {\"c\": true,},\n}\n";
    assert!(parse(text, &strict()).is_err());
    let parsed = parse_ok(text, &tolerant());
    check_invariants(&parsed.document);
    assert_eq!(parsed.fixes.len(), 5);
    let fixed = parsed.autocorrected().unwrap_or_default();
    let reparsed = parse_ok(&fixed, &strict());
    let (a, b) = (&parsed.document, &reparsed.document);
    assert_eq!(to_value(a, a.root()), to_value(b, b.root()));
    assert_eq!(
        to_value(b, b.root()),
        serde_json::json!({"a": [1, 2, 3], "b": {"c": true}})
    );
}

#[test]
fn parses_large_file_through_mmap() {
    let mut text = String::from("[");
    for i in 0..60_000 {
        if i > 0 {
            text.push(',');
        }
        text.push_str(&format!(r#"{{"id":{i},"v":"valor {i}"}}"#));
    }
    text.push(']');
    assert!(text.len() > 1024 * 1024, "debe superar el umbral de mmap");
    let path = std::env::temp_dir().join(format!("jsonlens-mmap-{}.json", std::process::id()));
    std::fs::write(&path, &text).expect("escribir archivo temporal");
    let source = Source::from_file(&path).expect("abrir archivo");
    assert!(matches!(source, Source::Mapped(_)));
    let parsed = match parse(source, &strict()) {
        Ok(p) => p,
        Err(e) => panic!("debería parsear: {e}"),
    };
    let _ = std::fs::remove_file(&path);
    let doc = &parsed.document;
    assert_eq!(doc.node_count(), 1 + 60_000 * 3);
    check_invariants(doc);
    let last = doc.children(doc.root()).last().expect("último elemento");
    let value = doc.children(last).nth(1).expect("campo v");
    assert_eq!(doc.string(value).as_deref(), Some("valor 59999"));
}
