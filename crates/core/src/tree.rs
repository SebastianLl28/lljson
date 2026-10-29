//! Flat list of visible rows for the single-document tree viewer.
//!
//! One row per node, in document order, with its nesting depth. Construction is iterative,
//! so depth is not limited by the thread stack.

use crate::document::{Document, NodeId};

/// A visible node (8 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeRow {
    pub node: NodeId,
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
        let mut stack = vec![(doc.root(), 0u16)];
        while let Some((node, depth)) = stack.pop() {
            rows.push(TreeRow { node, depth });
            let child_depth = depth.saturating_add(1);
            let first = stack.len();
            stack.extend(doc.children(node).map(|child| (child, child_depth)));
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
    fn row_is_eight_bytes() {
        assert_eq!(size_of::<TreeRow>(), 8);
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
}
