//! Flat list of visible rows for the single-document tree viewer.
//!
//! One row per node, in document order, with its nesting depth. Construction is iterative,
//! so depth is not limited by the thread stack.

use std::borrow::Cow;

use crate::document::{Document, Kind, NodeId};
use crate::rows::{Cell, RowKind, TokenKind, cell_text};

/// A visible node (12 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeRow {
    pub node: NodeId,
    /// Position among its siblings (0 for the root), kept here so that showing an array
    /// index never has to count siblings.
    pub index: u32,
    /// Nesting depth (0 for the root); saturates at `u16::MAX`.
    pub depth: u16,
}

#[derive(Debug, Default)]
pub struct TreeRows {
    rows: Vec<TreeRow>,
}

impl TreeRows {
    /// Every node of `doc` is visible.
    pub fn new(doc: &Document) -> TreeRows {
        let mut rows = Vec::with_capacity(doc.node_count());
        let mut stack = vec![(doc.root(), 0u32, 0u16)];
        while let Some((node, index, depth)) = stack.pop() {
            rows.push(TreeRow { node, index, depth });
            let child_depth = depth.saturating_add(1);
            let first = stack.len();
            stack.extend(
                (0..)
                    .zip(doc.children(node))
                    .map(|(index, child)| (child, index, child_depth)),
            );
            // Popping takes the last element first: reverse to visit children in order.
            stack[first..].reverse();
        }
        TreeRows { rows }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<TreeRow> {
        self.rows.get(index).copied()
    }
}

/// What precedes the value of a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeLabel {
    /// The root has neither key nor index.
    Root,
    /// Key exactly as in the source (escapes kept), without quotes.
    Key(String),
    /// Position of an array element.
    Index(u32),
}

/// Text of a tree row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeText<'a> {
    pub label: TreeLabel,
    pub kind: Kind,
    /// Scalar as written (long strings truncated), or `{n}` / `[n]` for a container.
    pub value: Cow<'a, str>,
    pub value_kind: TokenKind,
    /// Number of members or elements of a container.
    pub length: Option<u32>,
}

/// Builds the text of a row; rows of another document give an empty text.
pub fn tree_text(doc: &Document, row: TreeRow) -> TreeText<'_> {
    let Some(node) = doc.node(row.node) else {
        return TreeText {
            label: TreeLabel::Root,
            kind: Kind::Null,
            value: Cow::Borrowed(""),
            value_kind: TokenKind::Punctuation,
            length: None,
        };
    };
    let in_array = node
        .parent()
        .and_then(|parent| doc.kind(parent))
        .is_some_and(|kind| kind == Kind::Array);
    let label = match node.key_span() {
        Some(span) => TreeLabel::Key(doc.text(span).to_owned()),
        None if in_array => TreeLabel::Index(row.index),
        None => TreeLabel::Root,
    };
    let (value, value_kind, length) = match node.kind() {
        Kind::Object => {
            let n = node.child_count();
            (
                Cow::Owned(format!("{{{n}}}")),
                TokenKind::Punctuation,
                Some(n),
            )
        }
        Kind::Array => {
            let n = node.child_count();
            (
                Cow::Owned(format!("[{n}]")),
                TokenKind::Punctuation,
                Some(n),
            )
        }
        _ => {
            let text = cell_text(
                doc,
                Cell {
                    node: row.node,
                    kind: RowKind::Leaf,
                },
            );
            (text.value, text.value_kind, None)
        }
    };
    TreeText {
        label,
        kind: node.kind(),
        value,
        value_kind,
        length,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ParseOptions, parse};

    fn doc(text: &str) -> Document {
        parse(text, &ParseOptions::default())
            .expect("valid JSON")
            .document
    }

    /// `key-or-raw@depth` of each row.
    fn render(doc: &Document, rows: &TreeRows) -> Vec<String> {
        (0..rows.len())
            .filter_map(|i| rows.get(i))
            .map(|row| {
                let label = doc
                    .key(row.node)
                    .map_or_else(|| doc.raw(row.node).to_owned(), |k| k.into_owned());
                format!("{label}@{}", row.depth)
            })
            .collect()
    }

    #[test]
    fn row_is_twelve_bytes() {
        assert_eq!(size_of::<TreeRow>(), 12);
    }

    #[test]
    fn scalar_root_is_a_single_row() {
        let d = doc("42");
        let rows = TreeRows::new(&d);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows.get(0).map(|r| (r.node, r.depth)), Some((d.root(), 0)));
        assert_eq!(rows.get(1), None);
    }

    #[test]
    fn nodes_appear_in_document_order_with_their_depth() {
        let d = doc(r#"{"a":1,"b":[true,{"c":null}],"d":{}}"#);
        let rows = TreeRows::new(&d);
        assert_eq!(
            render(&d, &rows),
            [
                "{\"a\":1,\"b\":[true,{\"c\":null}],\"d\":{}}@0",
                "a@1",
                "b@1",
                "true@2",
                "{\"c\":null}@2",
                "c@3",
                "d@1"
            ]
        );
    }

    #[test]
    fn one_row_per_node() {
        let d = doc(r#"[[],[[]],{"a":[1,2,3]}]"#);
        assert_eq!(TreeRows::new(&d).len(), d.node_count());
    }

    #[test]
    fn empty_containers_are_single_rows() {
        let d = doc("[[],{}]");
        let rows = TreeRows::new(&d);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.get(1).map(|r| r.depth), Some(1));
        assert_eq!(rows.get(2).map(|r| r.depth), Some(1));
    }

    #[test]
    fn default_has_no_rows() {
        let rows = TreeRows::default();
        assert!(rows.is_empty());
        assert_eq!(rows.get(0), None);
    }

    #[test]
    fn very_deep_nesting_does_not_overflow_and_depth_saturates() {
        let levels = 70_000;
        let text = format!("{}{}", "[".repeat(levels), "]".repeat(levels));
        let d = doc(&text);
        let rows = TreeRows::new(&d);
        assert_eq!(rows.len(), levels);
        assert_eq!(rows.get(1000).map(|r| r.depth), Some(1000));
        assert_eq!(rows.get(levels - 1).map(|r| r.depth), Some(u16::MAX));
    }

    fn texts(text: &str) -> Vec<TreeText<'static>> {
        let d = Box::leak(Box::new(doc(text)));
        let rows = TreeRows::new(d);
        (0..rows.len())
            .filter_map(|i| rows.get(i))
            .map(|row| tree_text(d, row))
            .collect()
    }

    fn summary(t: &TreeText<'_>) -> String {
        let label = match &t.label {
            TreeLabel::Root => "root".to_owned(),
            TreeLabel::Key(k) => format!("key {k}"),
            TreeLabel::Index(i) => format!("[{i}]"),
        };
        format!("{label} = {}", t.value)
    }

    #[test]
    fn sibling_index_is_recorded_for_every_row() {
        let d = doc(r#"[10,[20,30],{"a":1}]"#);
        let rows = TreeRows::new(&d);
        let indices: Vec<u32> = (0..rows.len())
            .filter_map(|i| rows.get(i))
            .map(|r| r.index)
            .collect();
        assert_eq!(indices, [0, 0, 1, 0, 1, 2, 0]);
    }

    #[test]
    fn objects_show_keys_and_length() {
        let t = texts(r#"{"users":[1,2,3],"meta":{"a":1,"b":2},"none":{}}"#);
        let lines: Vec<String> = t.iter().map(summary).collect();
        assert_eq!(
            lines,
            [
                "root = {3}",
                "key users = [3]",
                "[0] = 1",
                "[1] = 2",
                "[2] = 3",
                "key meta = {2}",
                "key a = 1",
                "key b = 2",
                "key none = {0}"
            ]
        );
    }

    #[test]
    fn lengths_are_set_only_for_containers() {
        let t = texts(r#"{"a":[],"b":{"c":1},"d":"x","e":null}"#);
        let lengths: Vec<Option<u32>> = t.iter().map(|t| t.length).collect();
        assert_eq!(lengths, [Some(4), Some(0), Some(1), None, None, None]);
        assert_eq!(t[1].value, "[0]");
    }

    #[test]
    fn kinds_and_value_kinds() {
        let t = texts(r#"[{},[],"s",1,true,null]"#);
        let kinds: Vec<Kind> = t.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            [
                Kind::Array,
                Kind::Object,
                Kind::Array,
                Kind::String,
                Kind::Number,
                Kind::Bool,
                Kind::Null
            ]
        );
        assert_eq!(t[3].value, "\"s\"");
        assert_eq!(t[3].value_kind, TokenKind::String);
        assert_eq!(t[4].value_kind, TokenKind::Number);
        assert_eq!(t[5].value_kind, TokenKind::Bool);
        assert_eq!(t[6].value_kind, TokenKind::Null);
    }

    #[test]
    fn keys_keep_source_escapes_and_unicode() {
        let t = texts(r#"{"a\nb":1,"ñ":2}"#);
        assert_eq!(t[1].label, TreeLabel::Key("a\\nb".to_owned()));
        assert_eq!(t[2].label, TreeLabel::Key("ñ".to_owned()));
    }

    #[test]
    fn scalar_root_has_no_label_and_no_length() {
        let t = texts("\"hi\"");
        assert_eq!(t[0].label, TreeLabel::Root);
        assert_eq!(t[0].length, None);
    }

    #[test]
    fn long_strings_are_truncated() {
        let long = "x".repeat(5000);
        let t = texts(&format!("\"{long}\""));
        assert!(t[0].value.chars().count() <= crate::MAX_VALUE_CHARS + 2);
    }

    #[test]
    fn rows_of_another_document_give_an_empty_text() {
        let big = doc("[1,2,3,4,5]");
        let small = doc("1");
        let rows = TreeRows::new(&big);
        let t = tree_text(&small, rows.get(5).expect("row"));
        assert_eq!((t.value.as_ref(), t.length), ("", None));
    }
}
