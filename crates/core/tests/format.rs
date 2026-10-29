//! Tests de integración del formateador: idempotencia, equivalencia semántica y
//! coherencia con el hash.

use lljson_core::{Document, FormatOptions, Indent, Mode, ParseOptions, format, minify, parse};
use serde_json::Value;

const CORPUS: &[&str] = &[
    "0",
    "-0.0",
    "123456789012345678901234567890",
    r#""texto con \"comillas\" y \\ barra""#,
    "true",
    "null",
    "[]",
    "{}",
    "[[[[[]]]]]",
    r#"[1, "dos", 3.0, true, false, null, [], {}]"#,
    r#"{"a": {"b": {"c": [1, {"d": null}]}}}"#,
    r#"{"": "clave vacía", "ñandú": "ü", "中文": "日本語", "emoji": "😀"}"#,
    r#"{"z": 1, "a": [3, 2, 1], "m": {"y": true, "b": false}}"#,
    "{\r\n  \"crlf\": [1,\r\n 2]\r\n}",
];

fn doc(text: &str) -> Document {
    let options = ParseOptions {
        mode: Mode::Tolerant,
    };
    match parse(text, &options) {
        Ok(parsed) => parsed.document,
        Err(e) => panic!("{text:?} debería parsear: {e}"),
    }
}

fn all_options() -> Vec<FormatOptions> {
    let mut options = Vec::new();
    for indent in [
        Indent::Spaces(2),
        Indent::Spaces(4),
        Indent::Spaces(0),
        Indent::Tab,
        Indent::None,
    ] {
        for sort_keys in [false, true] {
            options.push(FormatOptions { indent, sort_keys });
        }
    }
    options
}

fn root_hash(d: &Document) -> Option<u64> {
    d.hash(d.root())
}

#[test]
fn output_is_valid_json_with_the_same_value() {
    for text in CORPUS {
        let original = doc(text);
        let expected: Value = serde_json::from_str(text).unwrap_or(Value::Null);
        for options in all_options() {
            let out = format(&original, &options);
            let value: Value = match serde_json::from_str(&out) {
                Ok(v) => v,
                Err(e) => panic!("salida inválida para {text:?} con {options:?}: {e}\n{out}"),
            };
            assert_eq!(value, expected, "{text:?} con {options:?}");
            assert_eq!(
                root_hash(&doc(&out)),
                root_hash(&original),
                "{text:?} con {options:?}"
            );
        }
    }
}

#[test]
fn formatting_is_idempotent() {
    for text in CORPUS {
        for options in all_options() {
            let once = format(&doc(text), &options);
            let twice = format(&doc(&once), &options);
            assert_eq!(once, twice, "{text:?} con {options:?}");
        }
    }
}

#[test]
fn minify_ignores_previous_formatting() {
    for text in CORPUS {
        let direct = minify(&doc(text));
        for options in all_options().into_iter().filter(|o| !o.sort_keys) {
            let reformatted = format(&doc(text), &options);
            assert_eq!(
                minify(&doc(&reformatted)),
                direct,
                "{text:?} con {options:?}"
            );
        }
    }
}

#[test]
fn sorting_makes_key_order_irrelevant() {
    let options = FormatOptions {
        sort_keys: true,
        ..FormatOptions::default()
    };
    let a = format(
        &doc(r#"{"b": {"y": 1, "x": 2}, "a": [ {"d": 1, "c": 2} ]}"#),
        &options,
    );
    let b = format(
        &doc(r#"{"a": [{"c": 2, "d": 1}], "b": {"x": 2, "y": 1}}"#),
        &options,
    );
    assert_eq!(a, b);
}

#[test]
fn tolerant_input_formats_to_strict_json() {
    let text = "// config\n{\n  \"a\": [1, 2,], /* nota */\n  \"b\": {\"c\": true,},\n}\n";
    let out = minify(&doc(text));
    assert_eq!(out, r#"{"a":[1,2],"b":{"c":true}}"#);
    assert!(parse(out, &ParseOptions::default()).is_ok());
}
