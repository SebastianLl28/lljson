//! Integration tests for the diff engine.

use lljson_core::{
    ChangeKind, DiffOptions, DiffResult, DiffStatus, Document, IgnoreRule, Mode, NodeId,
    ParseOptions, diff, parse,
};
use serde_json::{Value, json};

fn doc(text: &str) -> Document {
    let options = ParseOptions {
        mode: Mode::Tolerant,
    };
    match parse(text, &options) {
        Ok(parsed) => parsed.document,
        Err(e) => panic!("{text:?} should parse: {e}"),
    }
}

fn sample() -> Value {
    json!({
        "version": 3,
        "meta": {"updatedAt": "2026-10-28", "tags": ["a", "b"]},
        "users": [
            {"id": 1, "name": "Ana", "roles": ["admin"], "active": true, "score": 9.5},
            {"id": 2, "name": "Luis", "roles": [], "active": false, "score": null},
            {"id": 3, "name": "Eva", "roles": ["dev", "ops"], "active": true, "score": 7}
        ],
        "settings": {"theme": "dark", "limits": {"max": 100, "min": 0}}
    })
}

/// Checks that the change list and the node statuses agree.
fn check_consistency(left: &Document, right: &Document, result: &DiffResult) {
    for change in result.changes() {
        let (l_status, r_status) = match change.kind {
            ChangeKind::Added => (None, Some(DiffStatus::Added)),
            ChangeKind::Removed => (Some(DiffStatus::Removed), None),
            ChangeKind::Modified => (Some(DiffStatus::Modified), Some(DiffStatus::Modified)),
            ChangeKind::TypeChanged => {
                (Some(DiffStatus::TypeChanged), Some(DiffStatus::TypeChanged))
            }
        };
        assert_eq!(change.left.is_some(), l_status.is_some());
        assert_eq!(change.right.is_some(), r_status.is_some());
        if let (Some(l), Some(expected)) = (change.left, l_status) {
            assert_eq!(result.left_status(l), expected);
            check_ancestors(left, l, |id| result.left_status(id));
        }
        if let (Some(r), Some(expected)) = (change.right, r_status) {
            assert_eq!(result.right_status(r), expected);
            check_ancestors(right, r, |id| result.right_status(id));
        }
    }
    let root_status = result.left_status(left.root());
    if result.is_equal() {
        assert_eq!(root_status, DiffStatus::Equal);
    } else {
        assert!(matches!(
            root_status,
            DiffStatus::Contains | DiffStatus::Modified | DiffStatus::TypeChanged
        ));
    }
}

fn check_ancestors(doc: &Document, id: NodeId, status: impl Fn(NodeId) -> DiffStatus) {
    let mut current = doc.node(id).and_then(|n| n.parent());
    while let Some(ancestor) = current {
        assert_eq!(
            status(ancestor),
            DiffStatus::Contains,
            "{}",
            doc.path(ancestor)
        );
        current = doc.node(ancestor).and_then(|n| n.parent());
    }
}

/// Applies `mutate` to a copy of `sample()` and returns the changed paths.
fn diff_after(mutate: impl FnOnce(&mut Value), options: &DiffOptions) -> Vec<(ChangeKind, String)> {
    let original = sample();
    let mut changed = original.clone();
    mutate(&mut changed);
    let left = doc(&serde_json::to_string_pretty(&original).unwrap_or_default());
    let right = doc(&serde_json::to_string(&changed).unwrap_or_default());
    let result = diff(&left, &right, options);
    check_consistency(&left, &right, &result);
    result
        .changes()
        .iter()
        .map(|c| {
            let path = match (c.left, c.right) {
                (Some(l), _) => left.path(l),
                (None, Some(r)) => right.path(r),
                (None, None) => String::new(),
            };
            (c.kind, path)
        })
        .collect()
}

fn set(pointer: &str, value: Value) -> impl FnOnce(&mut Value) {
    let pointer = pointer.to_owned();
    move |root: &mut Value| {
        if let Some(slot) = root.pointer_mut(&pointer) {
            *slot = value;
        }
    }
}

#[test]
fn identical_and_reformatted_documents_are_equal() {
    let text = serde_json::to_string(&sample()).unwrap_or_default();
    let reordered = r#"{"settings": {"limits": {"min": 0, "max": 100.0}, "theme": "dark"},
        "users": [
            {"score": 9.50, "active": true, "roles": ["admin"], "name": "Ana", "id": 1},
            {"id": 2, "name": "Luis", "roles": [], "active": false, "score": null},
            {"id": 3, "name": "Eva", "roles": ["dev", "ops",], "active": true, "score": 7e0},
        ],
        // comment
        "meta": {"tags": ["a", "b"], "updatedAt": "2026-10-28"}, "version": 3}"#;
    let (left, right) = (doc(&text), doc(reordered));
    let result = diff(&left, &right, &DiffOptions::default());
    assert!(result.is_equal(), "{:?}", result.changes());
    check_consistency(&left, &right, &result);
}

type Mutation = Box<dyn FnOnce(&mut Value)>;

#[test]
fn single_mutations_are_reported_at_the_right_path() {
    let defaults = DiffOptions::default();
    let cases: Vec<(Mutation, Vec<(ChangeKind, &str)>)> = vec![
        (
            Box::new(set("/version", json!(4))),
            vec![(ChangeKind::Modified, "$.version")],
        ),
        (
            Box::new(set("/users/1/name", json!("Lucho"))),
            vec![(ChangeKind::Modified, "$.users[1].name")],
        ),
        (
            Box::new(set("/users/2/score", json!("7"))),
            vec![(ChangeKind::TypeChanged, "$.users[2].score")],
        ),
        (
            Box::new(set("/settings/limits", json!([100, 0]))),
            vec![(ChangeKind::TypeChanged, "$.settings.limits")],
        ),
        (
            Box::new(|v: &mut Value| {
                if let Some(obj) = v.pointer_mut("/meta").and_then(Value::as_object_mut) {
                    obj.remove("tags");
                    obj.insert("author".into(), json!("me"));
                }
            }),
            vec![
                (ChangeKind::Removed, "$.meta.tags"),
                (ChangeKind::Added, "$.meta.author"),
            ],
        ),
        (
            Box::new(|v: &mut Value| {
                if let Some(roles) = v
                    .pointer_mut("/users/2/roles")
                    .and_then(Value::as_array_mut)
                {
                    roles.push(json!("qa"));
                }
            }),
            vec![(ChangeKind::Added, "$.users[2].roles[2]")],
        ),
    ];
    for (mutate, expected) in cases {
        let expected: Vec<_> = expected
            .into_iter()
            .map(|(k, p)| (k, p.to_owned()))
            .collect();
        assert_eq!(diff_after(mutate, &defaults), expected);
    }
}

#[test]
fn diff_is_symmetric() {
    let a = doc(r#"{"a": 1, "b": [1, 2, 3], "c": {"d": "x"}, "e": "1"}"#);
    let b = doc(r#"{"a": 2, "b": [1, 2], "c": {"d": "x", "f": null}, "e": 1}"#);
    let forward = diff(&a, &b, &DiffOptions::default()).summary();
    let backward = diff(&b, &a, &DiffOptions::default()).summary();
    assert_eq!(forward.added, backward.removed);
    assert_eq!(forward.removed, backward.added);
    assert_eq!(forward.modified, backward.modified);
    assert_eq!(forward.type_changed, backward.type_changed);
}

#[test]
fn keyed_arrays_and_ignore_rules_together() {
    let options = DiffOptions {
        array_key: Some("id".to_owned()),
        ignore: vec![
            IgnoreRule::parse("updatedAt").unwrap_or(IgnoreRule::Key(String::new())),
            IgnoreRule::parse("$.users[*].score").unwrap_or(IgnoreRule::Key(String::new())),
        ],
    };
    let changes = diff_after(
        |v| {
            if let Some(users) = v.pointer_mut("/users").and_then(Value::as_array_mut) {
                users.reverse();
                users.insert(0, json!({"id": 4, "name": "New"}));
            }
            if let Some(slot) = v.pointer_mut("/meta/updatedAt") {
                *slot = json!("2026-10-29");
            }
            if let Some(slot) = v.pointer_mut("/users/1/score") {
                *slot = json!(1);
            }
        },
        &options,
    );
    assert_eq!(changes, vec![(ChangeKind::Added, "$.users[0]".to_owned())]);
}

/// Array of records (~12 nodes each).
fn big(records: usize, modify: &[usize]) -> String {
    let mut out = String::from("[");
    for i in 0..records {
        if i > 0 {
            out.push(',');
        }
        let name = if modify.contains(&i) {
            format!("changed_{i}")
        } else {
            format!("u{i}")
        };
        out.push_str(&format!(
            r#"{{"id":{i},"name":"{name}","tags":["a","b"],"score":{i}.5,"ok":true,"addr":{{"city":"c{}","zip":null}}}}"#,
            i % 97
        ));
    }
    out.push(']');
    out
}

#[test]
fn large_nearly_identical_documents() {
    let left = doc(&big(50_000, &[]));
    let right = doc(&big(50_000, &[7, 25_000, 49_999]));
    let result = diff(&left, &right, &DiffOptions::default());
    let paths: Vec<_> = result
        .changes()
        .iter()
        .filter_map(|c| c.left.map(|l| left.path(l)))
        .collect();
    assert_eq!(paths, vec!["$[7].name", "$[25000].name", "$[49999].name"]);
    check_consistency(&left, &right, &result);
}
