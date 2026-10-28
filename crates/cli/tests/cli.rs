//! Tests del binario `jsonlens-cli`: argumentos, stdin, salida y códigos de salida.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn run(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_jsonlens-cli"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lanzar jsonlens-cli");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("escribir stdin");
    child.wait_with_output().expect("esperar a jsonlens-cli")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("UTF-8")
}

/// Archivo temporal propio de cada test (los tests corren en paralelo).
fn temp_file(name: &str, text: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("jsonlens-cli-{}-{name}", std::process::id()));
    std::fs::write(&path, text).expect("escribir archivo temporal");
    path
}

fn arg(path: &Path) -> &str {
    path.to_str().expect("ruta UTF-8")
}

#[test]
fn format_from_stdin() {
    let output = run(&["format"], r#"{"a":[1,2]}"#);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "{\n  \"a\": [\n    1,\n    2\n  ]\n}\n");
}

#[test]
fn format_options() {
    let output = run(&["format", "--tab", "--sort-keys", "-"], r#"{"b":1,"a":2}"#);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "{\n\t\"a\": 2,\n\t\"b\": 1\n}\n");

    let output = run(&["format", "--indent", "4"], r#"[1]"#);
    assert_eq!(stdout(&output), "[\n    1\n]\n");
}

#[test]
fn tab_and_indent_conflict() {
    let output = run(&["format", "--tab", "--indent", "4"], "[]");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn minify_file() {
    let path = temp_file("minify.json", "{ \"b\" : 1,\n  \"a\" : [ true ] }");
    let output = run(&["minify", "--sort-keys", arg(&path)], "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "{\"a\":[true],\"b\":1}\n");
    let _ = std::fs::remove_file(path);
}

#[test]
fn tolerant_input() {
    let text = "[1, 2, // dos\n]";
    let strict = run(&["minify"], text);
    assert_eq!(strict.status.code(), Some(2));
    assert!(
        stderr(&strict).contains("<stdin>: línea 1"),
        "{}",
        stderr(&strict)
    );

    let tolerant = run(&["minify", "--tolerant"], text);
    assert!(tolerant.status.success(), "{}", stderr(&tolerant));
    assert_eq!(stdout(&tolerant), "[1,2]\n");
}

#[test]
fn diff_equal_documents() {
    let path = temp_file("equal.json", r#"{"a":1,"b":[1,2]}"#);
    let output = run(&["diff", arg(&path), "-"], r#"{ "b": [1, 2], "a": 1 }"#);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "Sin diferencias\n");
    let _ = std::fs::remove_file(path);
}

#[test]
fn diff_reports_changes() {
    let left = temp_file("left.json", r#"{"a":1,"b":"1","c":true,"t":5}"#);
    let right = temp_file("right.json", r#"{"a":2,"b":1,"d":null,"t":6}"#);
    let output = run(&["diff", "--ignore", "t", arg(&left), arg(&right)], "");
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "~ $.a: 1 → 2\n\
         ! $.b: \"1\" → 1\n\
         - $.c: true\n\
         + $.d: null\n\
         1 añadido, 1 eliminado, 2 modificados (1 cambio de tipo)\n"
    );

    let quiet = run(&["diff", "-q", arg(&left), arg(&right)], "");
    assert_eq!(quiet.status.code(), Some(1));
    assert!(quiet.stdout.is_empty());
    let _ = std::fs::remove_file(left);
    let _ = std::fs::remove_file(right);
}

#[test]
fn diff_array_key() {
    let left = temp_file("key-left.json", r#"[{"id":1,"v":"a"},{"id":2,"v":"b"}]"#);
    let output = run(
        &["diff", "--array-key", "id", arg(&left), "-"],
        r#"[{"id":2,"v":"b"},{"id":1,"v":"a"}]"#,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let _ = std::fs::remove_file(left);
}

#[test]
fn diff_errors() {
    let both_stdin = run(&["diff", "-", "-"], "{}");
    assert_eq!(both_stdin.status.code(), Some(2));

    let missing = run(&["diff", "/no/existe.json", "-"], "{}");
    assert_eq!(missing.status.code(), Some(2));
    assert!(stderr(&missing).contains("no se pudo abrir /no/existe.json"));

    let bad_rule = run(&["diff", "--ignore", "$.a[", "/no/existe.json", "-"], "{}");
    assert_eq!(bad_rule.status.code(), Some(2));
    assert!(
        stderr(&bad_rule).contains("--ignore"),
        "{}",
        stderr(&bad_rule)
    );
}
