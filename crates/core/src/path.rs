//! Node paths (`$.users[3].email`) and path-ignore rules for the diff.

use std::fmt::Write;

use crate::decode::unescape;
use crate::document::{Document, NodeId};

impl Document {
    /// Walks the preceding siblings, so it is O(position).
    pub fn child_position(&self, id: NodeId) -> Option<usize> {
        let parent = self.node(id)?.parent()?;
        self.children(parent).position(|c| c == id)
    }

    /// JSONPath-style path, e.g. `$.users[3].email`; non-identifier keys use brackets: `$["a b"]`.
    pub fn path(&self, id: NodeId) -> String {
        let mut chain = Vec::new();
        let mut current = id;
        while let Some(parent) = self.node(current).and_then(|n| n.parent()) {
            chain.push(current);
            current = parent;
        }
        let mut out = String::from("$");
        for &node in chain.iter().rev() {
            match self.key(node) {
                Some(key) => push_key(&mut out, &key),
                None => {
                    let _ = write!(out, "[{}]", self.child_position(node).unwrap_or(0));
                }
            }
        }
        out
    }
}

fn push_key(out: &mut String, key: &str) {
    if is_identifier(key) {
        out.push('.');
        out.push_str(key);
        return;
    }
    out.push_str("[\"");
    for c in key.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push_str("\"]");
}

fn is_identifier(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IgnoreRule {
    /// Any member with this key at any depth (e.g. `timestamp`).
    Key(String),
    /// Path from the root (e.g. `$.users[*].updatedAt`).
    Path(Vec<PatternSegment>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternSegment {
    /// `.name` or `["name"]`.
    Key(String),
    /// `.*`: any key.
    AnyKey,
    /// `[3]`.
    Index(usize),
    /// `[*]`: any index.
    AnyIndex,
}

/// `position` is the 1-based column in the rule text.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PatternError {
    #[error("the rule is empty")]
    Empty,
    #[error("column {position}: {message}")]
    Invalid {
        position: usize,
        message: &'static str,
    },
}

impl IgnoreRule {
    /// A bare key name, or a path starting with `$`.
    pub fn parse(text: &str) -> Result<IgnoreRule, PatternError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(PatternError::Empty);
        }
        let Some(rest) = text.strip_prefix('$') else {
            return Ok(IgnoreRule::Key(text.to_owned()));
        };
        // 1-based column in `text` of position `i` of `rest`.
        let column = |i: usize| text.get(..1 + i).unwrap_or(text).chars().count() + 1;
        let invalid = |i: usize, message| PatternError::Invalid {
            position: column(i),
            message,
        };

        let mut segments = Vec::new();
        let mut i = 0;
        while let Some(&c) = rest.as_bytes().get(i) {
            match c {
                b'.' => {
                    let start = i + 1;
                    let tail = rest.get(start..).unwrap_or_default();
                    let end = tail.find(['.', '[']).map_or(rest.len(), |n| start + n);
                    let name = rest.get(start..end).unwrap_or_default();
                    if name.is_empty() {
                        return Err(invalid(i, "missing key name after '.'"));
                    }
                    segments.push(if name == "*" {
                        PatternSegment::AnyKey
                    } else {
                        PatternSegment::Key(name.to_owned())
                    });
                    i = end;
                }
                b'[' if rest.as_bytes().get(i + 1) == Some(&b'"') => {
                    let start = i + 2;
                    let mut j = start;
                    loop {
                        match rest.as_bytes().get(j) {
                            Some(b'\\') => j += 2,
                            Some(b'"') => break,
                            Some(_) => j += 1,
                            None => return Err(invalid(i, "unterminated quoted key")),
                        }
                    }
                    if rest.as_bytes().get(j + 1) != Some(&b']') {
                        return Err(invalid(j + 1, "expected ']'"));
                    }
                    let raw = rest.get(start..j).unwrap_or_default();
                    segments.push(PatternSegment::Key(unescape(raw).into_owned()));
                    i = j + 2;
                }
                b'[' => {
                    let tail = rest.get(i + 1..).unwrap_or_default();
                    let Some(len) = tail.find(']') else {
                        return Err(invalid(i, "missing ']'"));
                    };
                    let inner = tail.get(..len).unwrap_or_default().trim();
                    let segment = if inner == "*" {
                        PatternSegment::AnyIndex
                    } else {
                        match inner.parse() {
                            Ok(index) => PatternSegment::Index(index),
                            Err(_) => {
                                return Err(invalid(
                                    i + 1,
                                    "expected an index, '*' or a quoted key",
                                ));
                            }
                        }
                    };
                    segments.push(segment);
                    i += len + 2;
                }
                _ => return Err(invalid(i, "expected '.' or '['")),
            }
        }
        Ok(IgnoreRule::Path(segments))
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

    #[test]
    fn node_paths() {
        let d = doc(r#"{"users": [{"email": "a"}, {"e-mail": "b", "x\"y": 1, "ñ": [true]}]}"#);
        let paths: Vec<_> = (0..d.node_count())
            .map(|i| d.path(NodeId::from_index(i)))
            .collect();
        assert_eq!(
            paths,
            vec![
                "$",
                "$.users",
                "$.users[0]",
                "$.users[0].email",
                "$.users[1]",
                r#"$.users[1]["e-mail"]"#,
                r#"$.users[1]["x\"y"]"#,
                "$.users[1].ñ",
                "$.users[1].ñ[0]",
            ]
        );
        assert_eq!(d.child_position(d.root()), None);
    }

    #[test]
    fn parses_rules() {
        use PatternSegment::*;
        assert_eq!(
            IgnoreRule::parse(" timestamp "),
            Ok(IgnoreRule::Key("timestamp".into()))
        );
        assert_eq!(IgnoreRule::parse("$"), Ok(IgnoreRule::Path(vec![])));
        assert_eq!(
            IgnoreRule::parse(r#"$.users[*].meta["updated at"][2].*"#),
            Ok(IgnoreRule::Path(vec![
                Key("users".into()),
                AnyIndex,
                Key("meta".into()),
                Key("updated at".into()),
                Index(2),
                AnyKey,
            ]))
        );
        assert_eq!(
            IgnoreRule::parse(r#"$["a\"b"]"#),
            Ok(IgnoreRule::Path(vec![Key("a\"b".into())]))
        );
    }

    #[test]
    fn rejects_invalid_rules() {
        assert_eq!(IgnoreRule::parse("  "), Err(PatternError::Empty));
        let error = |text: &str| match IgnoreRule::parse(text) {
            Err(PatternError::Invalid { position, .. }) => position,
            other => panic!("{text:?} should fail: {other:?}"),
        };
        assert_eq!(error("$."), 2);
        assert_eq!(error("$.a..b"), 4);
        assert_eq!(error("$[x]"), 3);
        assert_eq!(error("$[1"), 2);
        assert_eq!(error(r#"$["a"x"#), 6);
        assert_eq!(error(r#"$["a"#), 2);
        assert_eq!(error("$a"), 2);
    }
}
