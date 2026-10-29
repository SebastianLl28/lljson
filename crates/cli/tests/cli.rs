//! Tests for the `lljson-cli` binary: arguments, stdin, output and exit codes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn run(args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lljson-cli"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn lljson-cli");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait for lljson-cli")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("UTF-8")
}

/// Per-test temp file (tests run in parallel).
fn temp_file(name: &str, text: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("lljson-cli-{}-{name}", std::process::id()));
    std::fs::write(&path, text).expect("write temp file");
    path
}

fn arg(path: &Path) -> &str {
    path.to_str().expect("UTF-8 path")
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
    let text = "[1, 2, // two\n]";
    let strict = run(&["minify"], text);
    assert_eq!(strict.status.code(), Some(2));
    assert!(
        stderr(&strict).contains("<stdin>: line 1"),
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
    assert_eq!(stdout(&output), "No differences\n");
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
         1 added, 1 removed, 2 modified (1 type change)\n"
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

    let missing = run(&["diff", "/no/such.json", "-"], "{}");
    assert_eq!(missing.status.code(), Some(2));
    assert!(stderr(&missing).contains("could not open /no/such.json"));

    let bad_rule = run(&["diff", "--ignore", "$.a[", "/no/such.json", "-"], "{}");
    assert_eq!(bad_rule.status.code(), Some(2));
    assert!(
        stderr(&bad_rule).contains("--ignore"),
        "{}",
        stderr(&bad_rule)
    );
}

#[test]
fn generate_base_and_variant() {
    let base = run(&["generate", "--nodes", "1201"], "");
    assert!(base.status.success(), "{}", stderr(&base));
    let base_path = temp_file("gen-base.json", &stdout(&base));
    let variant = run(
        &[
            "generate",
            "--nodes",
            "1201",
            "--compact",
            "--modify",
            "2",
            "--type-change",
            "1",
            "--remove",
            "1",
            "--add",
            "3",
        ],
        "",
    );
    assert!(variant.status.success(), "{}", stderr(&variant));
    assert!(!stdout(&variant).trim_end().contains('\n'));

    let output = run(&["diff", "-q", arg(&base_path), "-"], &stdout(&base));
    assert_eq!(output.status.code(), Some(0));
    let output = run(
        &["diff", "--array-key", "id", arg(&base_path), "-"],
        &stdout(&variant),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output).ends_with("3 added, 1 removed, 3 modified (1 type change)\n"),
        "{}",
        stdout(&output)
    );
    let _ = std::fs::remove_file(base_path);
}

#[test]
fn generate_too_many_changes() {
    let output = run(&["generate", "--nodes", "13", "--remove", "2"], "");
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("2 changes"), "{}", stderr(&output));
}
