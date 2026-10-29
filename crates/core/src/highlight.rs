//! Syntax highlighting of one line of JSON text. Stateless, so any line can be highlighted
//! without looking at the ones before it: a JSON string never spans lines. The one exception is
//! a `/* */` comment (tolerant mode) that spans lines; only the part on each line is marked as
//! a comment when it opens on that line.

use std::ops::Range;

/// What a piece of text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    Key,
    String,
    Number,
    Bool,
    Null,
    Punctuation,
    Comment,
}

/// A highlighted byte range of the line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub range: Range<usize>,
    pub syntax: Syntax,
}

/// Highlights `line` (without its line break). Text that is not valid JSON is highlighted as
/// far as it can be told apart; unknown bytes are left unmarked. Never panics.
pub fn highlight_line(line: &str) -> Vec<Highlight> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        match bytes[i] {
            b'"' => {
                i = string_end(bytes, i);
                let syntax = if is_followed_by_colon(bytes, i) {
                    Syntax::Key
                } else {
                    Syntax::String
                };
                out.push(Highlight {
                    range: start..i,
                    syntax,
                });
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                out.push(Highlight {
                    range: start..bytes.len(),
                    syntax: Syntax::Comment,
                });
                break;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = find(bytes, i + 2, b"*/").map_or(bytes.len(), |at| at + 2);
                out.push(Highlight {
                    range: start..i,
                    syntax: Syntax::Comment,
                });
            }
            b'{' | b'}' | b'[' | b']' | b',' | b':' => {
                i += 1;
                out.push(Highlight {
                    range: start..i,
                    syntax: Syntax::Punctuation,
                });
            }
            b'-' | b'0'..=b'9' => {
                while i < bytes.len()
                    && matches!(bytes[i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
                {
                    i += 1;
                }
                out.push(Highlight {
                    range: start..i,
                    syntax: Syntax::Number,
                });
            }
            b'a'..=b'z' | b'A'..=b'Z' => {
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let syntax = match &bytes[start..i] {
                    b"true" | b"false" => Some(Syntax::Bool),
                    b"null" => Some(Syntax::Null),
                    _ => None,
                };
                if let Some(syntax) = syntax {
                    out.push(Highlight {
                        range: start..i,
                        syntax,
                    });
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// Index just past the closing quote of the string opening at `open`, or the end of the line
/// if it is not closed.
fn string_end(bytes: &[u8], open: usize) -> usize {
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn is_followed_by_colon(bytes: &[u8], from: usize) -> bool {
    bytes
        .get(from..)
        .and_then(|rest| rest.iter().find(|b| !b.is_ascii_whitespace()))
        == Some(&b':')
}

fn find(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|at| at + from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text:kind` of each highlight, to read the expectations at a glance.
    fn spans(line: &str) -> Vec<(String, Syntax)> {
        highlight_line(line)
            .into_iter()
            .map(|h| (line[h.range].to_owned(), h.syntax))
            .collect()
    }

    fn s(text: &str, syntax: Syntax) -> (String, Syntax) {
        (text.to_owned(), syntax)
    }

    #[test]
    fn member_with_each_value_type() {
        assert_eq!(
            spans(r#"  "a": "x","#),
            [
                s("\"a\"", Syntax::Key),
                s(":", Syntax::Punctuation),
                s("\"x\"", Syntax::String),
                s(",", Syntax::Punctuation),
            ]
        );
        assert_eq!(
            spans(r#""n": -1.5e+3"#),
            [
                s("\"n\"", Syntax::Key),
                s(":", Syntax::Punctuation),
                s("-1.5e+3", Syntax::Number),
            ]
        );
        assert_eq!(
            spans("[true, false, null]"),
            [
                s("[", Syntax::Punctuation),
                s("true", Syntax::Bool),
                s(",", Syntax::Punctuation),
                s("false", Syntax::Bool),
                s(",", Syntax::Punctuation),
                s("null", Syntax::Null),
                s("]", Syntax::Punctuation),
            ]
        );
    }

    #[test]
    fn key_may_have_space_before_the_colon() {
        assert_eq!(spans(r#""a" :"#)[0], s("\"a\"", Syntax::Key));
    }

    #[test]
    fn escaped_quotes_do_not_end_a_string() {
        assert_eq!(spans(r#""a\"b\\""#), [s(r#""a\"b\\""#, Syntax::String)]);
    }

    #[test]
    fn unterminated_string_runs_to_the_end() {
        assert_eq!(spans(r#"  "abc"#), [s("\"abc", Syntax::String)]);
        assert_eq!(spans("\"abc\\"), [s("\"abc\\", Syntax::String)]);
    }

    #[test]
    fn comments() {
        assert_eq!(
            spans("1 // note"),
            [s("1", Syntax::Number), s("// note", Syntax::Comment)]
        );
        assert_eq!(
            spans("/* a */ 2"),
            [s("/* a */", Syntax::Comment), s("2", Syntax::Number)]
        );
        assert_eq!(spans("/* open"), [s("/* open", Syntax::Comment)]);
        // A slash alone is not a comment.
        assert_eq!(spans("/"), []);
    }

    #[test]
    fn braces_inside_strings_are_not_punctuation() {
        assert_eq!(spans(r#""{[,]}""#), [s(r#""{[,]}""#, Syntax::String)]);
    }

    #[test]
    fn unknown_words_and_unicode_are_left_alone() {
        assert_eq!(spans("nope"), []);
        assert_eq!(spans("é ü"), []);
        assert_eq!(
            spans(r#""é": "日本""#),
            [
                s("\"é\"", Syntax::Key),
                s(":", Syntax::Punctuation),
                s("\"日本\"", Syntax::String),
            ]
        );
    }

    #[test]
    fn empty_and_blank_lines() {
        assert_eq!(spans(""), []);
        assert_eq!(spans("   \t"), []);
    }

    #[test]
    fn ranges_are_valid_char_boundaries() {
        let line = r#"{"é":"üé", "n": [1, 2.5e10, "日"], /* ç */ "b": true} // ñ"#;
        for h in highlight_line(line) {
            assert!(line.is_char_boundary(h.range.start));
            assert!(line.is_char_boundary(h.range.end));
            assert!(h.range.start < h.range.end);
        }
    }
}
