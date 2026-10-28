//! Tests del hash semántico por subárbol.

use jsonlens_core::{Document, Mode, ParseOptions, parse};

fn doc(text: &str) -> Document {
    let options = ParseOptions {
        mode: Mode::Tolerant,
    };
    match parse(text, &options) {
        Ok(parsed) => parsed.document,
        Err(e) => panic!("{text:?} debería parsear: {e}"),
    }
}

fn root_hash(text: &str) -> u64 {
    let d = doc(text);
    d.hash(d.root()).unwrap_or_default()
}

fn assert_same(a: &str, b: &str) {
    assert_eq!(root_hash(a), root_hash(b), "{a} debería ser igual a {b}");
}

fn assert_differ(a: &str, b: &str) {
    assert_ne!(
        root_hash(a),
        root_hash(b),
        "{a} debería ser distinto de {b}"
    );
}

#[test]
fn key_order_does_not_matter() {
    assert_same(r#"{"a": 1, "b": 2}"#, r#"{"b": 2, "a": 1}"#);
    assert_same(
        r#"{"x": {"p": [1, 2], "q": null}, "y": true}"#,
        r#"{"y": true, "x": {"q": null, "p": [1, 2]}}"#,
    );
}

#[test]
fn array_order_matters() {
    assert_differ("[1, 2]", "[2, 1]");
    assert_differ("[[1], [2]]", "[[2], [1]]");
    assert_differ("[1, 1, 2]", "[1, 2, 2]");
}

#[test]
fn whitespace_comments_and_trailing_commas_do_not_matter() {
    assert_same(
        r#"{"a":[1,2],"b":"x"}"#,
        "{\n  // comentario\n  \"a\": [ 1, 2, ],\n  \"b\": \"x\", /* fin */\n}",
    );
}

#[test]
fn keys_stay_bound_to_their_values() {
    assert_differ(r#"{"a": 1, "b": 2}"#, r#"{"a": 2, "b": 1}"#);
    assert_differ(r#"{"a": {"b": 1}}"#, r#"{"b": {"a": 1}}"#);
    assert_differ(r#"{"a": 1}"#, r#"{"b": 1}"#);
}

#[test]
fn types_are_part_of_the_hash() {
    let values = [
        "null",
        "false",
        "true",
        "0",
        r#""""#,
        r#""0""#,
        "[]",
        "{}",
        r#""null""#,
        "[null]",
        r#"{"": null}"#,
        r#""true""#,
        "1",
        r#""1""#,
    ];
    for (i, a) in values.iter().enumerate() {
        for b in &values[i + 1..] {
            assert_differ(a, b);
        }
    }
}

#[test]
fn numbers_compare_by_value() {
    assert_same("1", "1.0");
    assert_same("[1, 2.50]", "[1e0, 25e-1]");
    assert_same("0", "-0");
    assert_differ("12345678901234567890", "12345678901234567891");
    assert_differ("1", "1.0000001");
}

#[test]
fn strings_compare_decoded() {
    assert_same(r#""A""#, r#""\u0041""#);
    assert_same(r#""ñ/""#, r#""\u00f1\/""#);
    assert_same(r#""😀""#, r#""\ud83d\ude00""#);
    assert_same(r#"{"clave": 1}"#, r#"{"cl\u0061ve": 1}"#);
    assert_differ(r#""a""#, r#""A""#);
}

#[test]
fn empty_containers_and_counts() {
    assert_differ("[]", "[[]]");
    assert_differ("{}", r#"{"a": {}}"#);
    assert_differ(r#"{"a": 1}"#, r#"{"a": 1, "a": 1}"#);
}

#[test]
fn subtree_hashes_match_across_documents() {
    let left = doc(r#"{"meta": {"v": 1}, "data": [10, {"id": 7, "tags": ["x"]}]}"#);
    let right = doc(r#"{"data": [11, {"tags": ["x"], "id": 7}], "meta": {"v": 2}}"#);
    let find = |d: &Document, key: &str| {
        d.children(d.root())
            .find(|&c| d.key(c).as_deref() == Some(key))
            .and_then(|c| d.hash(c))
    };
    assert_ne!(find(&left, "meta"), find(&right, "meta"));
    assert_ne!(find(&left, "data"), find(&right, "data"));
    let second = |d: &Document| {
        let data = d
            .children(d.root())
            .find(|&c| d.key(c).as_deref() == Some("data"));
        data.and_then(|a| d.children(a).nth(1))
            .and_then(|n| d.hash(n))
    };
    assert_eq!(second(&left), second(&right));
}
