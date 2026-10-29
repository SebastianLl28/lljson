//! Arena-based JSON tree: nodes live in a flat `Vec<Node>` and are referenced by index.
//! Strings and numbers are spans over the original text.

use std::borrow::Cow;
use std::ops::Range;

use crate::decode::unescape;
use crate::source::Source;

/// Index of a node in a [`Document`] arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub(crate) u32);

impl NodeId {
    /// Sentinel for "no node" (root's parent, no child, no sibling).
    pub(crate) const NONE: NodeId = NodeId(u32::MAX);

    /// Invalid if `index` does not fit in `u32`.
    pub fn from_index(index: usize) -> NodeId {
        NodeId(u32::try_from(index).unwrap_or(u32::MAX))
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn is_none(self) -> bool {
        self == Self::NONE
    }

    fn some(self) -> Option<NodeId> {
        if self.is_none() { None } else { Some(self) }
    }
}

/// Byte range `[start, end)` over the original text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub(crate) const EMPTY: Span = Span { start: 0, end: 0 };

    // The parser guarantees both ends fit in `u32`.
    pub(crate) fn new(start: usize, end: usize) -> Span {
        Span {
            start: start as u32,
            end: end as u32,
        }
    }

    pub fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }

    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start) as usize
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Kind {
    Null,
    Bool,
    Number,
    String,
    Array,
    Object,
}

impl Kind {
    pub fn is_container(self) -> bool {
        matches!(self, Kind::Array | Kind::Object)
    }
}

pub(crate) mod flags {
    /// Value of a `Bool` node.
    pub const TRUE: u8 = 1;
    /// The value string contains escape sequences.
    pub const VALUE_ESCAPED: u8 = 1 << 1;
    /// The key contains escape sequences.
    pub const KEY_ESCAPED: u8 = 1 << 2;
    /// The node is an object member and has a key.
    pub const HAS_KEY: u8 = 1 << 3;
}

/// Arena node. Children form a linked list (`first_child` -> `next_sibling`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node {
    pub(crate) kind: Kind,
    pub(crate) flags: u8,
    pub(crate) parent: NodeId,
    pub(crate) first_child: NodeId,
    pub(crate) next_sibling: NodeId,
    pub(crate) child_count: u32,
    /// Key content without quotes (empty if none).
    pub(crate) key: Span,
    /// Strings: content without quotes. Numbers: the literal. Containers: `{`/`[` through `}`/`]`.
    pub(crate) value: Span,
    /// Semantic subtree hash (see `hash.rs`).
    pub(crate) hash: u64,
}

impl Node {
    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn parent(&self) -> Option<NodeId> {
        self.parent.some()
    }

    pub fn first_child(&self) -> Option<NodeId> {
        self.first_child.some()
    }

    pub fn next_sibling(&self) -> Option<NodeId> {
        self.next_sibling.some()
    }

    pub fn child_count(&self) -> u32 {
        self.child_count
    }

    /// Key span without quotes, if the node is an object member.
    pub fn key_span(&self) -> Option<Span> {
        self.has(flags::HAS_KEY).then_some(self.key)
    }

    pub fn value_span(&self) -> Span {
        self.value
    }

    /// Semantic subtree hash: equal JSON values hash equal regardless of key order or formatting.
    pub fn hash(&self) -> u64 {
        self.hash
    }

    pub(crate) fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }
}

#[derive(Debug)]
pub struct Document {
    source: Source,
    nodes: Vec<Node>,
    root: NodeId,
}

impl Document {
    pub(crate) fn from_parts(source: Source, nodes: Vec<Node>, root: NodeId) -> Document {
        Document {
            source,
            nodes,
            root,
        }
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn kind(&self, id: NodeId) -> Option<Kind> {
        self.node(id).map(Node::kind)
    }

    pub fn hash(&self, id: NodeId) -> Option<u64> {
        self.node(id).map(Node::hash)
    }

    /// Exclusive end of a node's subtree. Nodes are in preorder, so the subtree is the
    /// contiguous arena range `id.index()..subtree_end(id)`.
    pub fn subtree_end(&self, id: NodeId) -> usize {
        let mut current = id;
        while let Some(node) = self.node(current) {
            if let Some(next) = node.next_sibling() {
                return next.index();
            }
            match node.parent() {
                Some(parent) => current = parent,
                None => break,
            }
        }
        self.nodes.len()
    }

    pub fn children(&self, id: NodeId) -> Children<'_> {
        let next = self.node(id).map_or(NodeId::NONE, |n| n.first_child);
        Children { doc: self, next }
    }

    pub fn source_bytes(&self) -> &[u8] {
        self.source.as_bytes()
    }

    /// Returns `""` if the span is invalid.
    pub fn text(&self, span: Span) -> &str {
        self.source
            .as_bytes()
            .get(span.range())
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or("")
    }

    /// Raw value text (for strings: undecoded, without quotes).
    pub fn raw(&self, id: NodeId) -> &str {
        self.node(id).map_or("", |n| self.text(n.value))
    }

    pub fn key(&self, id: NodeId) -> Option<Cow<'_, str>> {
        let node = self.node(id)?;
        let span = node.key_span()?;
        Some(self.decoded(span, node.has(flags::KEY_ESCAPED)))
    }

    pub fn string(&self, id: NodeId) -> Option<Cow<'_, str>> {
        let node = self.node(id).filter(|n| n.kind == Kind::String)?;
        Some(self.decoded(node.value, node.has(flags::VALUE_ESCAPED)))
    }

    pub fn bool(&self, id: NodeId) -> Option<bool> {
        let node = self.node(id).filter(|n| n.kind == Kind::Bool)?;
        Some(node.has(flags::TRUE))
    }

    pub fn number(&self, id: NodeId) -> Option<&str> {
        let node = self.node(id).filter(|n| n.kind == Kind::Number)?;
        Some(self.text(node.value))
    }

    fn decoded(&self, span: Span, escaped: bool) -> Cow<'_, str> {
        let raw = self.text(span);
        if escaped {
            unescape(raw)
        } else {
            Cow::Borrowed(raw)
        }
    }
}

pub struct Children<'a> {
    doc: &'a Document,
    next: NodeId,
}

impl Iterator for Children<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let id = self.next.some()?;
        self.next = self.doc.node(id).map_or(NodeId::NONE, |n| n.next_sibling);
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(kind: Kind, parent: NodeId, value: Span) -> Node {
        Node {
            kind,
            flags: 0,
            parent,
            first_child: NodeId::NONE,
            next_sibling: NodeId::NONE,
            child_count: 0,
            key: Span::EMPTY,
            value,
            hash: 0,
        }
    }

    /// Hand-built arena for `{"a":1,"b\n":"x\ty"}`.
    fn sample() -> Document {
        let text = r#"{"a":1,"b\n":"x\ty"}"#;
        let root = NodeId(0);
        let mut obj = node(Kind::Object, NodeId::NONE, Span::new(0, text.len()));
        obj.first_child = NodeId(1);
        obj.child_count = 2;
        let mut a = node(Kind::Number, root, Span::new(5, 6));
        a.flags = flags::HAS_KEY;
        a.key = Span::new(2, 3);
        a.next_sibling = NodeId(2);
        let mut b = node(Kind::String, root, Span::new(14, 18));
        b.flags = flags::HAS_KEY | flags::KEY_ESCAPED | flags::VALUE_ESCAPED;
        b.key = Span::new(8, 11);
        Document::from_parts(Source::from(text), vec![obj, a, b], root)
    }

    #[test]
    fn node_stays_compact() {
        assert!(std::mem::size_of::<Node>() <= 48);
    }

    #[test]
    fn navigates_children_and_values() {
        let doc = sample();
        let kids: Vec<_> = doc.children(doc.root()).collect();
        assert_eq!(kids, vec![NodeId(1), NodeId(2)]);
        assert_eq!(doc.key(kids[0]).as_deref(), Some("a"));
        assert_eq!(doc.number(kids[0]), Some("1"));
        assert_eq!(doc.key(kids[1]).as_deref(), Some("b\n"));
        assert_eq!(doc.string(kids[1]).as_deref(), Some("x\ty"));
        assert_eq!(doc.node(kids[1]).and_then(Node::parent), Some(doc.root()));
        assert_eq!(doc.key(doc.root()), None);
    }

    #[test]
    fn subtree_ranges() {
        let doc = sample();
        assert_eq!(doc.subtree_end(doc.root()), 3);
        assert_eq!(doc.subtree_end(NodeId(1)), 2);
        assert_eq!(doc.subtree_end(NodeId(2)), 3);
    }

    #[test]
    fn invalid_ids_are_harmless() {
        let doc = sample();
        let bogus = NodeId(99);
        assert!(doc.node(bogus).is_none());
        assert_eq!(doc.children(bogus).count(), 0);
        assert_eq!(doc.raw(bogus), "");
        assert_eq!(doc.text(Span::new(5, 500)), "");
    }
}
