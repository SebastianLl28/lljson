//! Semantic diff between two documents.
//! Subtrees with equal hashes are treated as equal and skipped. Iterative, so depth is
//! not limited by the thread stack.

use std::hash::Hash;

use ahash::AHashMap;

use crate::document::{Document, Kind, Node, NodeId};
use crate::path::{IgnoreRule, PatternSegment};

/// Status of a node after the diff.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DiffStatus {
    #[default]
    Equal,
    /// Only in the right document (whole subtree).
    Added,
    /// Only in the left document (whole subtree).
    Removed,
    /// Scalar with a different value.
    Modified,
    /// Different type on each side (e.g. `"1"` and `1`); marks the whole subtree.
    TypeChanged,
    /// Excluded by an [`IgnoreRule`] (whole subtree).
    Ignored,
    /// Container with some difference among its descendants.
    Contains,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
    TypeChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    pub left: Option<NodeId>,
    pub right: Option<NodeId>,
}

/// Change counts. An added or removed subtree counts as one change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub added: usize,
    pub removed: usize,
    /// Includes type changes.
    pub modified: usize,
    /// Subset of `modified`.
    pub type_changed: usize,
}

impl Summary {
    pub fn total(&self) -> usize {
        self.added + self.removed + self.modified
    }
}

#[derive(Clone, Debug, Default)]
pub struct DiffOptions {
    /// Key used to pair array elements (e.g. `"id"`). Applies only to arrays whose
    /// elements, on both sides, are all objects with that key; others are paired by index.
    pub array_key: Option<String>,
    pub ignore: Vec<IgnoreRule>,
}

#[derive(Debug)]
pub struct DiffResult {
    left_status: Vec<DiffStatus>,
    right_status: Vec<DiffStatus>,
    left_match: Vec<NodeId>,
    right_match: Vec<NodeId>,
    changes: Vec<Change>,
    summary: Summary,
}

impl DiffResult {
    fn new(left_len: usize, right_len: usize) -> DiffResult {
        DiffResult {
            left_status: vec![DiffStatus::Equal; left_len],
            right_status: vec![DiffStatus::Equal; right_len],
            left_match: vec![NodeId::NONE; left_len],
            right_match: vec![NodeId::NONE; right_len],
            changes: Vec::new(),
            summary: Summary::default(),
        }
    }

    pub fn left_status(&self, id: NodeId) -> DiffStatus {
        self.left_status
            .get(id.index())
            .copied()
            .unwrap_or_default()
    }

    pub fn right_status(&self, id: NodeId) -> DiffStatus {
        self.right_status
            .get(id.index())
            .copied()
            .unwrap_or_default()
    }

    /// Right node paired with a left node. Only recorded for nodes the diff visited; inside
    /// equal subtrees use [`pair_children`] on demand.
    pub fn left_match(&self, id: NodeId) -> Option<NodeId> {
        some(self.left_match.get(id.index()).copied())
    }

    /// Left node paired with a right node (see [`DiffResult::left_match`]).
    pub fn right_match(&self, id: NodeId) -> Option<NodeId> {
        some(self.right_match.get(id.index()).copied())
    }

    /// Changes in document order.
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    pub fn summary(&self) -> Summary {
        self.summary
    }

    pub fn is_equal(&self) -> bool {
        self.changes.is_empty()
    }
}

fn some(id: Option<NodeId>) -> Option<NodeId> {
    id.filter(|id| !id.is_none())
}

pub fn diff(left: &Document, right: &Document, options: &DiffOptions) -> DiffResult {
    let mut differ = Differ {
        left,
        right,
        options,
        result: DiffResult::new(left.node_count(), right.node_count()),
        stack: Vec::new(),
        paths: Vec::new(),
        track_paths: options
            .ignore
            .iter()
            .any(|rule| matches!(rule, IgnoreRule::Path(_))),
    };
    differ.run();
    differ.result
}

/// Pairs the children of two containers of the same kind: objects by key (duplicates in
/// order of appearance), arrays by index or by [`DiffOptions::array_key`].
/// Output follows the left container's order; a right-only child goes after the pair of
/// its right predecessor (and after any removed children following that pair).
pub fn pair_children<'a>(
    left: &'a Document,
    l: NodeId,
    right: &'a Document,
    r: NodeId,
    options: &DiffOptions,
) -> Vec<(Option<NodeId>, Option<NodeId>)> {
    let left_children: Vec<NodeId> = left.children(l).collect();
    let right_children: Vec<NodeId> = right.children(r).collect();
    let matches = match (left.kind(l), right.kind(r)) {
        // Common case: same keys in the same order. Pair by position and skip the hash map.
        (Some(Kind::Object), Some(Kind::Object))
            if left_children.len() == right_children.len()
                && left_children
                    .iter()
                    .zip(&right_children)
                    .all(|(&a, &b)| left.key(a) == right.key(b)) =>
        {
            (0..left_children.len()).map(Some).collect()
        }
        (Some(Kind::Object), Some(Kind::Object)) => match_by(
            left_children
                .iter()
                .map(|&c| left.key(c).unwrap_or_default()),
            right_children
                .iter()
                .map(|&c| right.key(c).unwrap_or_default()),
        ),
        (Some(Kind::Array), Some(Kind::Array)) => {
            let keyed = options.array_key.as_deref().and_then(|key| {
                let left_keys = element_keys(left, &left_children, key)?;
                let right_keys = element_keys(right, &right_children, key)?;
                Some(match_by(left_keys.into_iter(), right_keys.into_iter()))
            });
            keyed.unwrap_or_else(|| {
                (0..left_children.len())
                    .map(|i| (i < right_children.len()).then_some(i))
                    .collect()
            })
        }
        _ => vec![None; left_children.len()],
    };
    merge(&left_children, &right_children, &matches)
}

fn element_keys(doc: &Document, elements: &[NodeId], key: &str) -> Option<Vec<u64>> {
    elements
        .iter()
        .map(|&element| {
            if doc.kind(element)? != Kind::Object {
                return None;
            }
            let member = doc
                .children(element)
                .find(|&c| doc.key(c).as_deref() == Some(key))?;
            doc.hash(member)
        })
        .collect()
}

/// For each left element, the index of the right one with the same key; duplicates pair
/// in order of appearance.
fn match_by<K: Hash + Eq>(
    left: impl Iterator<Item = K>,
    right: impl Iterator<Item = K>,
) -> Vec<Option<usize>> {
    const END: usize = usize::MAX;
    let right: Vec<K> = right.collect();
    // Per-key linked list (head in the map, successors in `next`): no allocation per key.
    let mut head: AHashMap<K, usize> = AHashMap::with_capacity(right.len());
    let mut next = vec![END; right.len()];
    for (j, key) in right.into_iter().enumerate().rev() {
        if let Some(slot) = next.get_mut(j) {
            *slot = head.insert(key, j).unwrap_or(END);
        }
    }
    left.map(|key| {
        let slot = head.get_mut(&key)?;
        let j = *slot;
        if j == END {
            return None;
        }
        *slot = next.get(j).copied().unwrap_or(END);
        Some(j)
    })
    .collect()
}

fn merge(
    left: &[NodeId],
    right: &[NodeId],
    matches: &[Option<usize>],
) -> Vec<(Option<NodeId>, Option<NodeId>)> {
    let mut matched_from = vec![None; right.len()];
    for (i, m) in matches.iter().enumerate() {
        if let Some(slot) = m.and_then(|j| matched_from.get_mut(j)) {
            *slot = Some(i);
        }
    }
    // (left position to insert before, right index)
    let mut inserts: Vec<(usize, usize)> = Vec::new();
    let mut anchor = 0;
    for (j, from) in matched_from.iter().enumerate() {
        match from {
            Some(i) => anchor = i + 1,
            None => inserts.push((anchor, j)),
        }
    }
    inserts.sort_by_key(|&(position, _)| position);

    let mut out = Vec::with_capacity(left.len() + inserts.len());
    let mut pending = inserts.into_iter().peekable();
    for i in 0..=left.len() {
        // Consecutive removed children come before the added ones at that position.
        let unmatched_left = matches.get(i).is_some_and(Option::is_none);
        while let Some((_, j)) = pending.next_if(|&(position, _)| position <= i && !unmatched_left)
        {
            out.push((None, right.get(j).copied()));
        }
        if let Some(&l) = left.get(i) {
            let r = matches
                .get(i)
                .copied()
                .flatten()
                .and_then(|j| right.get(j).copied());
            out.push((Some(l), r));
        }
    }
    out
}

struct Work {
    left: Option<NodeId>,
    right: Option<NodeId>,
    /// Entry in `Differ::paths` for the parent container (`NO_PATH` for the root).
    parent_path: u32,
}

const NO_PATH: u32 = u32::MAX;

/// Visited container, used to rebuild the path for `$...` rules. Stores one side's node
/// (the left if present).
struct PathEntry {
    parent: u32,
    side: Side,
    node: NodeId,
}

#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

struct Differ<'a> {
    left: &'a Document,
    right: &'a Document,
    options: &'a DiffOptions,
    result: DiffResult,
    stack: Vec<Work>,
    paths: Vec<PathEntry>,
    /// Paths are only recorded when there are `$...` rules.
    track_paths: bool,
}

impl Differ<'_> {
    fn run(&mut self) {
        self.stack.push(Work {
            left: Some(self.left.root()),
            right: Some(self.right.root()),
            parent_path: NO_PATH,
        });
        while let Some(work) = self.stack.pop() {
            if self.is_ignored(&work) {
                if let Some(l) = work.left {
                    fill_subtree(
                        &mut self.result.left_status,
                        self.left,
                        l,
                        DiffStatus::Ignored,
                    );
                }
                if let Some(r) = work.right {
                    fill_subtree(
                        &mut self.result.right_status,
                        self.right,
                        r,
                        DiffStatus::Ignored,
                    );
                }
                continue;
            }
            match (work.left, work.right) {
                (Some(l), Some(r)) => self.compare(l, r, work.parent_path),
                (Some(l), None) => self.removed(l),
                (None, Some(r)) => self.added(r),
                (None, None) => {}
            }
        }
    }

    fn compare(&mut self, l: NodeId, r: NodeId, parent_path: u32) {
        if let Some(slot) = self.result.left_match.get_mut(l.index()) {
            *slot = r;
        }
        if let Some(slot) = self.result.right_match.get_mut(r.index()) {
            *slot = l;
        }
        let (Some(ln), Some(rn)) = (self.left.node(l), self.right.node(r)) else {
            return;
        };
        if ln.hash() == rn.hash() {
            return;
        }
        if ln.kind() != rn.kind() {
            fill_subtree(
                &mut self.result.left_status,
                self.left,
                l,
                DiffStatus::TypeChanged,
            );
            fill_subtree(
                &mut self.result.right_status,
                self.right,
                r,
                DiffStatus::TypeChanged,
            );
            self.record(ChangeKind::TypeChanged, Some(l), Some(r));
            self.mark_contains(ln.parent(), rn.parent());
            return;
        }
        if ln.kind().is_container() {
            let path = if self.track_paths {
                let index = u32::try_from(self.paths.len()).unwrap_or(NO_PATH);
                self.paths.push(PathEntry {
                    parent: parent_path,
                    side: Side::Left,
                    node: l,
                });
                index
            } else {
                NO_PATH
            };
            let pairs = pair_children(self.left, l, self.right, r, self.options);
            let works = pairs.into_iter().rev().map(|(left, right)| Work {
                left,
                right,
                parent_path: path,
            });
            self.stack.extend(works);
            return;
        }
        set(&mut self.result.left_status, l, DiffStatus::Modified);
        set(&mut self.result.right_status, r, DiffStatus::Modified);
        self.record(ChangeKind::Modified, Some(l), Some(r));
        self.mark_contains(ln.parent(), rn.parent());
    }

    fn removed(&mut self, l: NodeId) {
        fill_subtree(
            &mut self.result.left_status,
            self.left,
            l,
            DiffStatus::Removed,
        );
        self.record(ChangeKind::Removed, Some(l), None);
        let parent = self.left.node(l).and_then(Node::parent);
        let right_parent = parent.and_then(|p| self.result.left_match(p));
        self.mark_contains(parent, right_parent);
    }

    fn added(&mut self, r: NodeId) {
        fill_subtree(
            &mut self.result.right_status,
            self.right,
            r,
            DiffStatus::Added,
        );
        self.record(ChangeKind::Added, None, Some(r));
        let parent = self.right.node(r).and_then(Node::parent);
        let left_parent = parent.and_then(|p| self.result.right_match(p));
        self.mark_contains(left_parent, parent);
    }

    fn is_ignored(&self, work: &Work) -> bool {
        if self.options.ignore.is_empty() {
            return false;
        }
        let current = match (work.left, work.right) {
            (Some(l), _) => (Side::Left, l),
            (None, Some(r)) => (Side::Right, r),
            (None, None) => return false,
        };
        let mut chain: Option<Vec<(Side, NodeId)>> = None;
        self.options.ignore.iter().any(|rule| match rule {
            IgnoreRule::Key(name) => self.key(current).as_deref() == Some(name.as_str()),
            IgnoreRule::Path(pattern) => {
                let chain = chain.get_or_insert_with(|| self.chain(current, work.parent_path));
                pattern.len() == chain.len()
                    && pattern
                        .iter()
                        .zip(chain.iter())
                        .all(|(segment, &node)| self.segment_matches(segment, node))
            }
        })
    }

    /// Nodes from the first level below the root down to `current` (root is not a segment).
    fn chain(&self, current: (Side, NodeId), parent_path: u32) -> Vec<(Side, NodeId)> {
        let mut chain = vec![current];
        let mut next = parent_path;
        while let Some(entry) = self.paths.get(next as usize) {
            chain.push((entry.side, entry.node));
            next = entry.parent;
        }
        chain.pop();
        chain.reverse();
        chain
    }

    fn segment_matches(&self, segment: &PatternSegment, (side, node): (Side, NodeId)) -> bool {
        let key = self.key((side, node));
        match segment {
            PatternSegment::Key(name) => key.as_deref() == Some(name.as_str()),
            PatternSegment::AnyKey => key.is_some(),
            PatternSegment::AnyIndex => key.is_none(),
            PatternSegment::Index(index) => {
                key.is_none() && self.doc(side).child_position(node) == Some(*index)
            }
        }
    }

    fn doc(&self, side: Side) -> &Document {
        match side {
            Side::Left => self.left,
            Side::Right => self.right,
        }
    }

    fn key(&self, (side, node): (Side, NodeId)) -> Option<std::borrow::Cow<'_, str>> {
        self.doc(side).key(node)
    }

    fn record(&mut self, kind: ChangeKind, left: Option<NodeId>, right: Option<NodeId>) {
        let summary = &mut self.result.summary;
        match kind {
            ChangeKind::Added => summary.added += 1,
            ChangeKind::Removed => summary.removed += 1,
            ChangeKind::Modified => summary.modified += 1,
            ChangeKind::TypeChanged => {
                summary.modified += 1;
                summary.type_changed += 1;
            }
        }
        self.result.changes.push(Change { kind, left, right });
    }

    fn mark_contains(&mut self, left: Option<NodeId>, right: Option<NodeId>) {
        mark_ancestors(&mut self.result.left_status, self.left, left);
        mark_ancestors(&mut self.result.right_status, self.right, right);
    }
}

fn set(status: &mut [DiffStatus], id: NodeId, value: DiffStatus) {
    if let Some(slot) = status.get_mut(id.index()) {
        *slot = value;
    }
}

fn fill_subtree(status: &mut [DiffStatus], doc: &Document, id: NodeId, value: DiffStatus) {
    if let Some(range) = status.get_mut(id.index()..doc.subtree_end(id)) {
        range.fill(value);
    }
}

/// Walks up from `current` marking `Contains`; stops at the first already marked node.
fn mark_ancestors(status: &mut [DiffStatus], doc: &Document, mut current: Option<NodeId>) {
    while let Some(id) = current {
        let Some(slot) = status.get_mut(id.index()) else {
            return;
        };
        if *slot == DiffStatus::Contains {
            return;
        }
        *slot = DiffStatus::Contains;
        current = doc.node(id).and_then(Node::parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{ParseOptions, parse};

    fn doc(text: &str) -> Document {
        match parse(text, &ParseOptions::default()) {
            Ok(parsed) => parsed.document,
            Err(e) => panic!("{text:?} should parse: {e}"),
        }
    }

    fn run(a: &str, b: &str) -> (Document, Document, DiffResult) {
        let (left, right) = (doc(a), doc(b));
        let result = diff(&left, &right, &DiffOptions::default());
        (left, right, result)
    }

    fn changes(a: &str, b: &str) -> Vec<(ChangeKind, String, String)> {
        let (left, right, result) = run(a, b);
        result
            .changes()
            .iter()
            .map(|c| {
                let l = c.left.map(|id| left.raw(id).to_owned()).unwrap_or_default();
                let r = c
                    .right
                    .map(|id| right.raw(id).to_owned())
                    .unwrap_or_default();
                (c.kind, l, r)
            })
            .collect()
    }

    fn change(kind: ChangeKind, l: &str, r: &str) -> (ChangeKind, String, String) {
        (kind, l.to_owned(), r.to_owned())
    }

    #[test]
    fn equal_documents() {
        let (left, right, result) = run(r#"{"a": [1, 2], "b": 1.0}"#, r#"{"b": 1, "a": [1, 2]}"#);
        assert!(result.is_equal());
        assert_eq!(result.left_status(left.root()), DiffStatus::Equal);
        assert_eq!(result.left_match(left.root()), Some(right.root()));
        // Equal subtree: not visited.
        let a = left.children(left.root()).next().unwrap_or(left.root());
        assert_eq!(result.left_match(a), None);
    }

    #[test]
    fn modified_added_and_removed_members() {
        let found = changes(
            r#"{"a": 1, "b": 2, "c": 3}"#,
            r#"{"a": 1, "b": 20, "d": 4}"#,
        );
        assert_eq!(
            found,
            vec![
                change(ChangeKind::Modified, "2", "20"),
                change(ChangeKind::Removed, "3", ""),
                change(ChangeKind::Added, "", "4"),
            ]
        );
    }

    #[test]
    fn arrays_by_index() {
        let found = changes("[1, 2, 3]", "[1, 5]");
        assert_eq!(
            found,
            vec![
                change(ChangeKind::Modified, "2", "5"),
                change(ChangeKind::Removed, "3", ""),
            ]
        );
        assert_eq!(
            changes("[1]", "[1, [2]]"),
            vec![change(ChangeKind::Added, "", "[2]")]
        );
    }

    fn keyed() -> DiffOptions {
        DiffOptions {
            array_key: Some("id".to_owned()),
            ..DiffOptions::default()
        }
    }

    fn keyed_changes(a: &str, b: &str) -> Vec<(ChangeKind, String, String)> {
        let (left, right) = (doc(a), doc(b));
        let result = diff(&left, &right, &keyed());
        result
            .changes()
            .iter()
            .map(|c| {
                let l = c.left.map(|id| left.raw(id).to_owned()).unwrap_or_default();
                let r = c
                    .right
                    .map(|id| right.raw(id).to_owned())
                    .unwrap_or_default();
                (c.kind, l, r)
            })
            .collect()
    }

    #[test]
    fn arrays_by_key_survive_insertions_and_reordering() {
        let a = r#"[{"id": 1, "v": "a"}, {"id": 2, "v": "b"}, {"id": 3, "v": "c"}]"#;
        let b = r#"[{"id": 0, "v": "z"}, {"id": 3, "v": "c"}, {"id": 1, "v": "A"}, {"id": 2, "v": "b"}]"#;
        assert_eq!(
            keyed_changes(a, b),
            vec![
                change(ChangeKind::Added, "", r#"{"id": 0, "v": "z"}"#),
                change(ChangeKind::Modified, "a", "A"),
            ]
        );
        // By index, inserting at the start shifts every element.
        assert_eq!(changes(a, b).len(), 7);
    }

    #[test]
    fn arrays_by_key_match_key_values_semantically() {
        let a = r#"[{"id": 1, "v": 1}, {"id": "1", "v": 2}]"#;
        let b = r#"[{"id": "1", "v": 2}, {"id": 1.0, "v": 3}]"#;
        assert_eq!(
            keyed_changes(a, b),
            vec![change(ChangeKind::Modified, "1", "3")]
        );
    }

    #[test]
    fn arrays_by_key_fall_back_to_index() {
        // An element without the key (or not an object) makes the array fall back to indices.
        let a = r#"[{"id": 1}, {"id": 2}]"#;
        let b = r#"[{"id": 2}, {"x": 1}]"#;
        // 1->2 modified; "id" removed and "x" added in the second element.
        assert_eq!(keyed_changes(a, b).len(), 3);
        assert_eq!(keyed_changes("[1, 2]", "[2, 1]").len(), 2);
    }

    #[test]
    fn arrays_by_key_handle_removals_and_duplicates() {
        let a = r#"[{"id": 1}, {"id": 1, "n": 2}, {"id": 2}]"#;
        let b = r#"[{"id": 1}, {"id": 1, "n": 3}]"#;
        assert_eq!(
            keyed_changes(a, b),
            vec![
                change(ChangeKind::Modified, "2", "3"),
                change(ChangeKind::Removed, r#"{"id": 2}"#, ""),
            ]
        );
    }

    #[test]
    fn type_changes_are_explicit() {
        let found = changes(
            r#"{"a": "1", "b": 1, "c": null, "d": [1], "e": true}"#,
            r#"{"a": 1, "b": 1.0, "c": false, "d": {"0": 1}, "e": "true"}"#,
        );
        assert_eq!(
            found,
            vec![
                change(ChangeKind::TypeChanged, "1", "1"),
                change(ChangeKind::TypeChanged, "null", "false"),
                change(ChangeKind::TypeChanged, "[1]", r#"{"0": 1}"#),
                change(ChangeKind::TypeChanged, "true", "true"),
            ]
        );
        // Same type with a different value is still a modification.
        assert_eq!(
            changes("true", "false"),
            vec![change(ChangeKind::Modified, "true", "false")]
        );
    }

    #[test]
    fn type_change_marks_whole_subtrees() {
        let (left, right, result) = run(r#"{"x": [1, [2]]}"#, r#"{"x": {"y": 1}}"#);
        let left_status: Vec<_> = (0..left.node_count())
            .map(|i| result.left_status(NodeId::from_index(i)))
            .collect();
        use DiffStatus::*;
        assert_eq!(
            left_status,
            vec![Contains, TypeChanged, TypeChanged, TypeChanged, TypeChanged]
        );
        assert_eq!(result.right_status(NodeId::from_index(2)), TypeChanged);
        assert_eq!(result.right_status(right.root()), Contains);
    }

    fn ignoring(rules: &[&str], a: &str, b: &str) -> (Document, Document, DiffResult) {
        let ignore = rules
            .iter()
            .map(|r| IgnoreRule::parse(r).unwrap_or_else(|e| panic!("{r}: {e}")))
            .collect();
        let options = DiffOptions {
            ignore,
            ..DiffOptions::default()
        };
        let (left, right) = (doc(a), doc(b));
        let result = diff(&left, &right, &options);
        (left, right, result)
    }

    fn changed_paths(result: &DiffResult, left: &Document, right: &Document) -> Vec<String> {
        result
            .changes()
            .iter()
            .map(|c| match (c.left, c.right) {
                (Some(l), _) => left.path(l),
                (None, Some(r)) => right.path(r),
                (None, None) => String::new(),
            })
            .collect()
    }

    #[test]
    fn ignores_keys_at_any_depth() {
        let a = r#"{"updatedAt": 1, "v": 1, "items": [{"updatedAt": 5, "n": 1}], "x": {"updatedAt": 0}}"#;
        let b = r#"{"updatedAt": 2, "v": 1, "items": [{"updatedAt": 6, "n": 2}], "x": {}}"#;
        let (left, right, result) = ignoring(&["updatedAt"], a, b);
        assert_eq!(changed_paths(&result, &left, &right), vec!["$.items[0].n"]);
        let updated = left.children(left.root()).next().unwrap_or(left.root());
        assert_eq!(result.left_status(updated), DiffStatus::Ignored);
        // "x" only has ignored differences: not marked Contains.
        let x = left.children(left.root()).nth(3).unwrap_or(left.root());
        assert_eq!(result.left_status(x), DiffStatus::Equal);
    }

    #[test]
    fn ignores_paths_with_wildcards() {
        let a = r#"{"meta": {"ts": 1, "id": 1}, "users": [{"seen": 1, "n": "a"}, {"seen": 2, "n": "b"}], "seen": 1}"#;
        let b = r#"{"meta": {"ts": 2, "id": 2}, "users": [{"seen": 9, "n": "a"}, {"seen": 8, "n": "B"}], "seen": 2}"#;
        let (left, right, result) = ignoring(&["$.meta.ts", "$.users[*].seen"], a, b);
        assert_eq!(
            changed_paths(&result, &left, &right),
            vec!["$.meta.id", "$.users[1].n", "$.seen"]
        );
        let (left, right, result) = ignoring(&["$.users[1]", "$.*.id"], a, b);
        assert_eq!(
            changed_paths(&result, &left, &right),
            vec!["$.meta.ts", "$.users[0].seen", "$.seen"]
        );
    }

    #[test]
    fn ignored_additions_and_removals_are_not_changes() {
        let (left, right, result) = ignoring(
            &["debug", "$.tmp"],
            r#"{"a": 1, "debug": {"x": 1}}"#,
            r#"{"a": 1, "tmp": [1, 2]}"#,
        );
        assert!(result.is_equal());
        assert_eq!(result.left_status(left.root()), DiffStatus::Equal);
        let tmp = right.children(right.root()).nth(1).unwrap_or(right.root());
        assert_eq!(result.right_status(tmp), DiffStatus::Ignored);
        assert_eq!(
            result.right_status(NodeId::from_index(right.node_count() - 1)),
            DiffStatus::Ignored
        );
    }

    #[test]
    fn ignore_inside_keyed_arrays() {
        let a = r#"[{"id": 1, "ts": 1}, {"id": 2, "ts": 1}]"#;
        let b = r#"[{"id": 2, "ts": 5}, {"id": 1, "ts": 7}]"#;
        let options = DiffOptions {
            array_key: Some("id".to_owned()),
            ignore: vec![IgnoreRule::Key("ts".to_owned())],
        };
        let result = diff(&doc(a), &doc(b), &options);
        assert!(result.is_equal());
    }

    #[test]
    fn summary_counts_changes() {
        let (_, _, result) = run(
            r#"{"a": 1, "b": "x", "c": [1, 2, 3], "d": {"e": 1}, "f": null}"#,
            r#"{"a": 2, "b": 1, "c": [1, 2], "d": {"e": 1, "g": [4, 5]}, "h": true}"#,
        );
        let expected = Summary {
            added: 2,
            removed: 2,
            modified: 2,
            type_changed: 1,
        };
        assert_eq!(result.summary(), expected);
        assert_eq!(result.summary().total(), result.changes().len());
        let (_, _, equal) = run("[1]", "[1.0]");
        assert_eq!(equal.summary(), Summary::default());
    }

    #[test]
    fn statuses_and_ancestors() {
        let (left, right, result) = run(
            r#"{"x": {"y": [1, {"z": true}]}, "k": {"old": [1, 2]}}"#,
            r#"{"x": {"y": [1, {"z": false}]}, "k": {}}"#,
        );
        let status: Vec<_> = (0..left.node_count())
            .map(|i| result.left_status(NodeId::from_index(i)))
            .collect();
        use DiffStatus::*;
        // root, x, y, 1, {z}, z, k, old, 1, 2
        assert_eq!(
            status,
            vec![
                Contains, Contains, Contains, Equal, Contains, Modified, Contains, Removed,
                Removed, Removed
            ]
        );
        assert_eq!(result.right_status(right.root()), Contains);
        assert_eq!(result.changes().len(), 2);
    }

    #[test]
    fn added_subtree_is_fully_marked() {
        let (_, right, result) = run("[]", r#"[{"a": [1, 2]}]"#);
        let statuses: Vec<_> = (1..right.node_count())
            .map(|i| result.right_status(NodeId::from_index(i)))
            .collect();
        assert!(statuses.iter().all(|&s| s == DiffStatus::Added));
        assert_eq!(result.right_status(right.root()), DiffStatus::Contains);
        assert_eq!(result.changes().len(), 1);
    }

    #[test]
    fn duplicate_keys_pair_in_order() {
        let found = changes(r#"{"a": 1, "a": 2}"#, r#"{"a": 1, "a": 3, "a": 4}"#);
        assert_eq!(
            found,
            vec![
                change(ChangeKind::Modified, "2", "3"),
                change(ChangeKind::Added, "", "4"),
            ]
        );
    }

    #[test]
    fn pairing_order_places_right_only_after_predecessor() {
        let (left, right) = (
            doc(r#"{"a": 1, "c": 3}"#),
            doc(r#"{"z": 0, "a": 1, "b": 2, "c": 3, "d": 4}"#),
        );
        let pairs = pair_children(
            &left,
            left.root(),
            &right,
            right.root(),
            &DiffOptions::default(),
        );
        let keys: Vec<_> = pairs
            .iter()
            .map(|(l, r)| {
                let side = |d: &Document, id: Option<NodeId>| {
                    id.and_then(|i| d.key(i)).unwrap_or_default().into_owned()
                };
                (side(&left, *l), side(&right, *r))
            })
            .collect();
        let expected = [("", "z"), ("a", "a"), ("", "b"), ("c", "c"), ("", "d")];
        let expected: Vec<_> = expected
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        assert_eq!(keys, expected);
    }

    #[test]
    fn deep_nesting_does_not_overflow_the_stack() {
        const DEPTH: usize = 100_000;
        let a = format!("{}1{}", r#"{"a":["#.repeat(DEPTH), "]}".repeat(DEPTH));
        let b = format!("{}2{}", r#"{"a":["#.repeat(DEPTH), "]}".repeat(DEPTH));
        let (_, _, result) = run(&a, &b);
        assert_eq!(result.changes().len(), 1);
    }
}
