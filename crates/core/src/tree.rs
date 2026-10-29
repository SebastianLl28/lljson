//! Flat list of visible rows for the single-document tree viewer.
//!
//! One row per node, in document order, with its nesting depth. Construction is iterative,
//! so depth is not limited by the thread stack.

use std::borrow::Cow;

use crate::document::{Document, Kind, NodeId};
use crate::rows::{Cell, RowKind, Splice, TokenKind, cell_text};

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

/// Result of [`TreeRows::reveal`].
#[derive(Debug, PartialEq, Eq)]
pub struct TreeReveal {
    /// Row of the revealed node once the splices are applied.
    pub position: usize,
    /// Each splice with the row count after applying it, in the order to apply them.
    pub splices: Vec<(Splice, usize)>,
}

/// Visible rows of one document. Methods taking a document must always receive the one the
/// list was built with.
#[derive(Debug, Default)]
pub struct TreeRows {
    rows: Vec<TreeRow>,
    /// Per node: its children are hidden. Kept when an ancestor collapses, so that expanding
    /// the ancestor again restores the state below it.
    collapsed: Vec<bool>,
}

impl TreeRows {
    /// Every node of `doc` is visible.
    pub fn new(doc: &Document) -> TreeRows {
        TreeRows::rebuilt(doc, vec![false; doc.node_count()])
    }

    fn rebuilt(doc: &Document, collapsed: Vec<bool>) -> TreeRows {
        let root = TreeRow {
            node: doc.root(),
            index: 0,
            depth: 0,
        };
        let mut rows = Vec::with_capacity(doc.node_count());
        emit(doc, root, &collapsed, &mut rows);
        TreeRows { rows, collapsed }
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

    /// `Some(true)` if the row shows its children, `Some(false)` if they are hidden, `None`
    /// if it has none (scalar or empty container).
    pub fn is_expanded(&self, index: usize, doc: &Document) -> Option<bool> {
        let node = self.rows.get(index)?.node;
        has_children(doc, node).then(|| !self.collapsed.get(node.index()).copied().unwrap_or(false))
    }

    /// Expands or collapses the container at row `index` by splicing the list (the row itself
    /// included); `None` if the row has no children.
    pub fn toggle(&mut self, index: usize, doc: &Document) -> Option<Splice> {
        let row = self.rows.get(index).copied()?;
        let expanded = self.is_expanded(index, doc)?;
        let flag = self.collapsed.get_mut(row.node.index())?;
        *flag = expanded;
        if expanded {
            let end = doc.subtree_end(row.node);
            let removed = self.rows[index + 1..]
                .iter()
                .take_while(|r| r.node.index() < end)
                .count();
            self.rows.drain(index + 1..=index + removed);
            // The toggled row is part of the splice so that its marker is redrawn.
            Some(Splice {
                position: index,
                removed: removed + 1,
                added: 1,
            })
        } else {
            let mut shown = Vec::new();
            emit(doc, row, &self.collapsed, &mut shown);
            let added = shown.len();
            self.rows.splice(index..=index, shown);
            Some(Splice {
                position: index,
                removed: 1,
                added,
            })
        }
    }

    /// Row showing `node`, if it is visible.
    pub fn position_of(&self, node: NodeId) -> Option<usize> {
        // Rows are in document order, which is node order.
        let at = self.rows.partition_point(|r| r.node.index() < node.index());
        self.rows.get(at).filter(|r| r.node == node).map(|_| at)
    }

    /// Expands every collapsed ancestor of `node` so that it has a row. `None` if `node` is
    /// not part of `doc`.
    pub fn reveal(&mut self, node: NodeId, doc: &Document) -> Option<TreeReveal> {
        doc.node(node)?;
        let mut chain = Vec::new();
        let mut current = doc.node(node)?.parent();
        while let Some(ancestor) = current {
            chain.push(ancestor);
            current = doc.node(ancestor)?.parent();
        }
        let mut splices = Vec::new();
        // Outermost first: an inner ancestor only has a row once the outer ones are open.
        for ancestor in chain.into_iter().rev() {
            if !self
                .collapsed
                .get(ancestor.index())
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            let at = self.position_of(ancestor)?;
            if let Some(splice) = self.toggle(at, doc) {
                splices.push((splice, self.rows.len()));
            }
        }
        let position = self.position_of(node)?;
        Some(TreeReveal { position, splices })
    }

    /// Shows every node. The whole list changes, so callers replace it entirely.
    pub fn expand_all(&mut self, doc: &Document) {
        *self = TreeRows::new(doc);
    }

    /// Hides everything below the root's children: the root stays expanded so that the first
    /// level remains visible. The whole list changes, so callers replace it entirely.
    pub fn collapse_all(&mut self, doc: &Document) {
        let mut collapsed = vec![true; doc.node_count()];
        if let Some(flag) = collapsed.get_mut(doc.root().index()) {
            *flag = false;
        }
        *self = TreeRows::rebuilt(doc, collapsed);
    }
}

fn has_children(doc: &Document, node: NodeId) -> bool {
    doc.node(node)
        .is_some_and(|n| n.kind().is_container() && n.child_count() > 0)
}

/// Appends `row` and, unless collapsed, everything below it in document order.
fn emit(doc: &Document, row: TreeRow, collapsed: &[bool], out: &mut Vec<TreeRow>) {
    let mut stack = vec![row];
    while let Some(row) = stack.pop() {
        out.push(row);
        if collapsed.get(row.node.index()).copied().unwrap_or(false) {
            continue;
        }
        let depth = row.depth.saturating_add(1);
        let first = stack.len();
        stack.extend(
            (0..)
                .zip(doc.children(row.node))
                .map(|(index, node)| TreeRow { node, index, depth }),
        );
        // Popping takes the last element first: reverse to visit children in order.
        stack[first..].reverse();
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
    fn reveal_expands_every_collapsed_ancestor() {
        let d = doc(r#"{"a":{"b":{"c":1}},"d":2}"#);
        let mut rows = TreeRows::new(&d);
        rows.collapse_all(&d);
        assert_eq!(rows.len(), 3);
        let c = (0..d.node_count())
            .map(NodeId::from_index)
            .find(|&id| d.path(id) == "$.a.b.c")
            .expect("path");
        assert_eq!(rows.position_of(c), None);
        let reveal = rows.reveal(c, &d).expect("revealed");
        assert_eq!(reveal.splices.len(), 2);
        assert_eq!(reveal.splices.last().map(|s| s.1), Some(rows.len()));
        assert_eq!(rows.position_of(c), Some(reveal.position));
        assert_eq!(rows.len(), 5);
    }

    #[test]
    fn reveal_of_a_visible_node_changes_nothing() {
        let d = doc(r#"{"a":[1,2]}"#);
        let mut rows = TreeRows::new(&d);
        let target = NodeId::from_index(2);
        let reveal = rows.reveal(target, &d).expect("revealed");
        assert!(reveal.splices.is_empty());
        assert_eq!(reveal.position, 2);
        assert_eq!(rows.len(), 4);
    }

    #[test]
    fn reveal_of_a_foreign_node_is_none() {
        let d = doc("[1]");
        let mut rows = TreeRows::new(&d);
        assert_eq!(rows.reveal(NodeId::from_index(99), &d), None);
    }

    #[test]
    fn position_of_hidden_node_is_none() {
        let d = doc(r#"{"a":[1]}"#);
        let mut rows = TreeRows::new(&d);
        rows.collapse_all(&d);
        assert_eq!(rows.position_of(NodeId::from_index(2)), None);
        assert_eq!(rows.position_of(d.root()), Some(0));
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

    /// Raw text and depth of each visible row.
    fn lines(d: &Document, rows: &TreeRows) -> Vec<String> {
        (0..rows.len())
            .filter_map(|i| rows.get(i))
            .map(|r| {
                let text = tree_text(d, r);
                let label = match text.label {
                    TreeLabel::Root => String::new(),
                    TreeLabel::Key(k) => format!("{k}: "),
                    TreeLabel::Index(i) => format!("[{i}]: "),
                };
                format!("{}{label}{}", "  ".repeat(usize::from(r.depth)), text.value)
            })
            .collect()
    }

    const NESTED: &str = r#"{"a":{"b":[1,2],"c":3},"d":[],"e":4}"#;

    #[test]
    fn is_expanded_distinguishes_containers_from_leaves() {
        let d = doc(NESTED);
        let rows = TreeRows::new(&d);
        // rows: root, a, b, [0], [1], c, d, e
        assert_eq!(rows.is_expanded(0, &d), Some(true));
        assert_eq!(rows.is_expanded(2, &d), Some(true));
        assert_eq!(rows.is_expanded(3, &d), None, "scalar");
        assert_eq!(rows.is_expanded(6, &d), None, "empty container");
        assert_eq!(rows.is_expanded(99, &d), None, "no such row");
    }

    #[test]
    fn collapsing_hides_the_subtree_and_reports_the_splice() {
        let d = doc(NESTED);
        let mut rows = TreeRows::new(&d);
        let splice = rows.toggle(1, &d);
        assert_eq!(
            splice,
            Some(Splice {
                position: 1,
                removed: 5,
                added: 1
            })
        );
        assert_eq!(lines(&d, &rows), ["{3}", "  a: {2}", "  d: [0]", "  e: 4"]);
        assert_eq!(rows.is_expanded(1, &d), Some(false));
    }

    #[test]
    fn expanding_restores_the_rows_and_the_state_below() {
        let d = doc(NESTED);
        let mut rows = TreeRows::new(&d);
        let before = lines(&d, &rows);
        rows.toggle(2, &d); // collapse b
        rows.toggle(1, &d); // collapse a
        let splice = rows.toggle(1, &d); // expand a: b stays collapsed
        assert_eq!(
            splice,
            Some(Splice {
                position: 1,
                removed: 1,
                added: 3
            })
        );
        assert_eq!(
            lines(&d, &rows),
            [
                "{3}",
                "  a: {2}",
                "    b: [2]",
                "    c: 3",
                "  d: [0]",
                "  e: 4"
            ]
        );
        rows.toggle(2, &d); // expand b
        assert_eq!(lines(&d, &rows), before);
    }

    #[test]
    fn toggling_a_row_without_children_does_nothing() {
        let d = doc(NESTED);
        let mut rows = TreeRows::new(&d);
        let before = lines(&d, &rows);
        assert_eq!(rows.toggle(3, &d), None);
        assert_eq!(rows.toggle(6, &d), None);
        assert_eq!(rows.toggle(99, &d), None);
        assert_eq!(lines(&d, &rows), before);
    }

    #[test]
    fn collapsing_the_root_leaves_one_row() {
        let d = doc(NESTED);
        let mut rows = TreeRows::new(&d);
        rows.toggle(0, &d);
        assert_eq!(lines(&d, &rows), ["{3}"]);
        rows.toggle(0, &d);
        assert_eq!(rows.len(), d.node_count());
    }

    #[test]
    fn collapse_all_keeps_the_first_level_and_expand_all_shows_everything() {
        let d = doc(NESTED);
        let mut rows = TreeRows::new(&d);
        rows.collapse_all(&d);
        assert_eq!(lines(&d, &rows), ["{3}", "  a: {2}", "  d: [0]", "  e: 4"]);
        rows.toggle(1, &d);
        assert_eq!(
            lines(&d, &rows),
            [
                "{3}",
                "  a: {2}",
                "    b: [2]",
                "    c: 3",
                "  d: [0]",
                "  e: 4"
            ]
        );
        rows.expand_all(&d);
        assert_eq!(rows.len(), d.node_count());
    }

    #[test]
    fn collapse_all_on_a_scalar_keeps_its_row() {
        let d = doc("1");
        let mut rows = TreeRows::new(&d);
        rows.collapse_all(&d);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn array_indices_survive_collapsing_and_expanding() {
        let d = doc("[[1],[2],[3]]");
        let mut rows = TreeRows::new(&d);
        rows.toggle(1, &d);
        rows.toggle(1, &d);
        let indices: Vec<u32> = (0..rows.len())
            .filter_map(|i| rows.get(i))
            .map(|r| r.index)
            .collect();
        assert_eq!(indices, [0, 0, 0, 1, 0, 2, 0]);
    }

    #[test]
    fn toggling_deeper_than_the_depth_limit_still_works() {
        let levels = 70_000;
        let d = doc(&format!("{}{}", "[".repeat(levels), "]".repeat(levels)));
        let mut rows = TreeRows::new(&d);
        let splice = rows.toggle(1, &d);
        assert_eq!(splice.map(|s| s.removed), Some(levels - 1));
        assert_eq!(rows.len(), 2);
        rows.toggle(1, &d);
        assert_eq!(rows.len(), levels);
    }

    #[test]
    fn splices_match_the_row_list_changes() {
        let d = doc(r#"{"a":[{"x":1},{"y":[1,2]}],"b":{"c":[true]}}"#);
        let mut rows = TreeRows::new(&d);
        for index in [1, 2, 0, 0, 1, 3, 1] {
            let before = rows.len();
            if let Some(s) = rows.toggle(index, &d) {
                assert_eq!(before - s.removed + s.added, rows.len(), "row {index}");
            }
        }
    }
}
