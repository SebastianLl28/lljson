//! Mapping between positions in the source text and nodes, for the editor ↔ tree sync.

use std::ops::Range;

use crate::document::{Document, Kind, NodeId};

/// Start offset of every line of a text, with the same line breaks as `GtkTextBuffer`: LF, CR,
/// CRLF and U+2029. (A lone U+2028 is not a break.)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineIndex {
    starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(text: &str) -> LineIndex {
        let bytes = text.as_bytes();
        let mut starts = vec![0];
        let mut i = 0;
        while i < bytes.len() {
            let width = match bytes[i] {
                b'\r' if bytes.get(i + 1) == Some(&b'\n') => 2,
                b'\n' | b'\r' => 1,
                // U+2029 is E2 80 A9.
                0xE2 if bytes[i..].starts_with(&[0xE2, 0x80, 0xA9]) => 3,
                _ => {
                    i += 1;
                    continue;
                }
            };
            i += width;
            starts.push(u32::try_from(i).unwrap_or(u32::MAX));
        }
        LineIndex { starts }
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// Line (0-based) and byte within it of `offset`; offsets past the end land on the last line.
    pub fn position(&self, offset: usize) -> (usize, usize) {
        let line = self
            .starts
            .partition_point(|&start| start as usize <= offset)
            .saturating_sub(1);
        (line, offset - self.starts[line] as usize)
    }

    /// Offset of byte `column` of `line`; `None` if the line does not exist.
    pub fn offset(&self, line: usize, column: usize) -> Option<usize> {
        self.starts.get(line).map(|&start| start as usize + column)
    }
}

impl Document {
    /// The text a node occupies: its key (with the opening quote) through the end of its value
    /// (with the closing quote of a string). `None` if `id` is not part of the document.
    pub fn node_range(&self, id: NodeId) -> Option<Range<usize>> {
        let node = self.node(id)?;
        let quote = usize::from(node.kind() == Kind::String);
        let value = node.value_span().range();
        let start = match node.key_span() {
            Some(key) => key.range().start.saturating_sub(1),
            None => value.start.saturating_sub(quote),
        };
        Some(start..value.end + quote)
    }

    /// The innermost node whose text contains `offset`. An offset between two members, in
    /// whitespace or on a comma, belongs to the container around them; one just past the end of
    /// a node (the cursor after its last character) belongs to that node. `None` outside the
    /// root value.
    pub fn node_at_offset(&self, offset: usize) -> Option<NodeId> {
        let here = self.innermost_at(offset);
        let before = offset.checked_sub(1).and_then(|o| self.innermost_at(o));
        // Preorder: the later node is the deeper one, or the one that starts later.
        here.max(before)
    }

    fn innermost_at(&self, offset: usize) -> Option<NodeId> {
        // Nodes are in preorder, so their starts only grow: the last one starting at or before
        // `offset` is the deepest candidate, and its ancestors are the others.
        let start_of = |index: usize| {
            self.node_range(NodeId::from_index(index))
                .map_or(usize::MAX, |range| range.start)
        };
        let (mut low, mut high) = (0, self.node_count());
        while low < high {
            let middle = low + (high - low) / 2;
            if start_of(middle) <= offset {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let at = low;
        let mut current = NodeId::from_index(at.checked_sub(1)?);
        loop {
            if self.node_range(current)?.contains(&offset) {
                return Some(current);
            }
            current = self.node(current)?.parent()?;
        }
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

    fn path_at(d: &Document, text: &str, needle: &str) -> String {
        let offset = text.find(needle).expect("needle");
        d.node_at_offset(offset)
            .map_or_else(|| "none".to_owned(), |id| d.path(id))
    }

    fn range_of<'a>(d: &Document, text: &'a str, path: &str) -> &'a str {
        let id = (0..d.node_count())
            .map(NodeId::from_index)
            .find(|&id| d.path(id) == path)
            .expect("path");
        &text[d.node_range(id).expect("range")]
    }

    #[test]
    fn line_index_positions() {
        let index = LineIndex::new("ab\ncd\n\ne");
        assert_eq!(index.line_count(), 4);
        assert_eq!(index.position(0), (0, 0));
        assert_eq!(index.position(2), (0, 2));
        assert_eq!(index.position(3), (1, 0));
        assert_eq!(index.position(6), (2, 0));
        assert_eq!(index.position(7), (3, 0));
        assert_eq!(index.position(99), (3, 92));
        assert_eq!(index.offset(1, 1), Some(4));
        assert_eq!(index.offset(4, 0), None);
    }

    #[test]
    fn line_index_follows_the_text_buffer_line_breaks() {
        assert_eq!(LineIndex::new("a\r\nb").line_count(), 2);
        assert_eq!(LineIndex::new("a\rb").line_count(), 2);
        assert_eq!(LineIndex::new("a\r\n\r\nb").line_count(), 3);
        assert_eq!(LineIndex::new("a\u{2029}b").line_count(), 2);
        // A line separator is not a paragraph break.
        assert_eq!(LineIndex::new("a\u{2028}b").line_count(), 1);
        assert_eq!(LineIndex::new("").line_count(), 1);
        assert_eq!(LineIndex::new("é\nü").position(3), (1, 0));
    }

    #[test]
    fn line_and_offset_round_trip() {
        let text = "{\n  \"a\": [1,\r\n 2]\n}";
        let index = LineIndex::new(text);
        for offset in 0..=text.len() {
            let (line, column) = index.position(offset);
            assert_eq!(index.offset(line, column), Some(offset));
        }
    }

    #[test]
    fn ranges_cover_key_and_value() {
        let text = r#"{"a": "x", "bb": [1, 2], "c": 3.5}"#;
        let d = doc(text);
        assert_eq!(range_of(&d, text, "$"), text);
        assert_eq!(range_of(&d, text, "$.a"), r#""a": "x""#);
        assert_eq!(range_of(&d, text, "$.bb"), r#""bb": [1, 2]"#);
        assert_eq!(range_of(&d, text, "$.bb[1]"), "2");
        assert_eq!(range_of(&d, text, "$.c"), r#""c": 3.5"#);
    }

    #[test]
    fn offsets_map_to_the_innermost_node() {
        let text = r#"{"a": "x", "bb": [10, 20], "c": null}"#;
        let d = doc(text);
        assert_eq!(path_at(&d, text, "\"a\""), "$.a");
        assert_eq!(path_at(&d, text, "x\""), "$.a");
        assert_eq!(path_at(&d, text, "bb"), "$.bb");
        assert_eq!(path_at(&d, text, "10"), "$.bb[0]");
        assert_eq!(path_at(&d, text, "20"), "$.bb[1]");
        assert_eq!(path_at(&d, text, "null"), "$.c");
        assert_eq!(path_at(&d, text, "{"), "$");
    }

    #[test]
    fn gaps_belong_to_the_container() {
        let text = r#"{"a": 1, "b": [1, 2]}"#;
        let d = doc(text);
        // The space after the comma, inside the root; the one inside the array.
        assert_eq!(path_at(&d, text, " \"b\""), "$");
        assert_eq!(path_at(&d, text, " 2"), "$.b");
    }

    #[test]
    fn the_end_of_a_node_still_belongs_to_it() {
        let scalar = doc("  42  ");
        let path = |offset| scalar.node_at_offset(offset).map(|id| scalar.path(id));
        assert_eq!(path(2), Some("$".to_owned()));
        // Just past the last character of the value.
        assert_eq!(path(4), Some("$".to_owned()));
        assert_eq!(path(5), None);
        let array = doc("[12, 3]");
        assert_eq!(
            array.node_at_offset(3).map(|id| array.path(id)),
            Some("$[0]".to_owned())
        );
    }

    #[test]
    fn outside_the_root_value_is_none() {
        let d = doc("  [1]  ");
        assert_eq!(d.node_at_offset(0), None);
        assert_eq!(d.node_at_offset(100), None);
        assert!(d.node_at_offset(2).is_some());
    }

    #[test]
    fn unicode_and_escapes_use_byte_offsets() {
        let text = r#"{"é": "日\"本", "z": 1}"#;
        let d = doc(text);
        assert_eq!(path_at(&d, text, "本"), "$.é");
        assert_eq!(path_at(&d, text, "\"z\""), "$.z");
    }

    #[test]
    fn deeply_nested_documents_do_not_overflow() {
        let depth = 20_000;
        let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        let d = doc(&text);
        let innermost = d.node_at_offset(depth).expect("innermost");
        assert_eq!(d.kind(innermost), Some(Kind::Number));
        assert_eq!(d.node_at_offset(0), Some(d.root()));
    }

    #[test]
    fn range_of_a_foreign_node_is_none() {
        let d = doc("1");
        assert_eq!(d.node_range(NodeId::from_index(9)), None);
    }
}
