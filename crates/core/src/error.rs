#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ErrorKind {
    #[error("text is not valid UTF-8")]
    InvalidUtf8,
    #[error("document is too large (maximum 4 GiB)")]
    TooLarge,
    #[error("unexpected end of text: expected {expected}")]
    UnexpectedEnd { expected: &'static str },
    #[error("unexpected character {found:?}: expected {expected}")]
    UnexpectedChar { found: char, expected: &'static str },
    #[error("invalid literal: expected `{expected}`")]
    InvalidLiteral { expected: &'static str },
    #[error("invalid number")]
    InvalidNumber,
    #[error("invalid number: leading zeros are not allowed")]
    LeadingZero,
    #[error("unterminated string")]
    UnterminatedString,
    #[error("unescaped control character inside a string")]
    ControlCharacter,
    #[error("invalid escape sequence")]
    InvalidEscape,
    #[error("invalid \\u sequence: expected 4 hexadecimal digits")]
    InvalidUnicodeEscape,
    #[error("invalid \\u sequence: unpaired UTF-16 surrogate")]
    LoneSurrogate,
    #[error("trailing comma not allowed in strict mode")]
    TrailingComma,
    #[error("comment not allowed in strict mode")]
    CommentNotAllowed,
    #[error("unterminated comment")]
    UnterminatedComment,
    #[error("extra content after the JSON value")]
    TrailingContent,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("line {line}, column {column}: {kind}")]
pub struct ParseError {
    pub kind: ErrorKind,
    /// Byte offset from the start of the text.
    pub offset: usize,
    /// 1-based line.
    pub line: u32,
    /// 1-based column, counted in characters (not bytes).
    pub column: u32,
}

impl ParseError {
    /// Line/column are computed here, so the text is only scanned on error.
    pub(crate) fn new(kind: ErrorKind, offset: usize, bytes: &[u8]) -> ParseError {
        let (line, column) = line_column(bytes, offset);
        ParseError {
            kind,
            offset,
            line,
            column,
        }
    }
}

impl ParseError {
    /// Short advice on how to fix the error, derived from its kind, the character found and
    /// the brackets still open at the failure point. `source` must be the text that was parsed.
    /// Only scans the text up to the error, so it is meant to be called on demand.
    pub fn hint(&self, source: &[u8]) -> Option<String> {
        match &self.kind {
            ErrorKind::UnexpectedEnd { .. } => Some(match innermost_open(source, self.offset) {
                Some(open) => unclosed(source, open),
                None => "The text ends before the value is complete".to_owned(),
            }),
            ErrorKind::UnexpectedChar { found, expected } => {
                unexpected_char_hint(source, self.offset, *found, expected)
            }
            ErrorKind::InvalidLiteral { .. } => {
                Some("Literals are lowercase: `true`, `false` and `null`".to_owned())
            }
            ErrorKind::LeadingZero => Some("Remove the extra zeros before the number".to_owned()),
            ErrorKind::UnterminatedString => {
                Some("Close the string with a double quote `\"`".to_owned())
            }
            ErrorKind::ControlCharacter => {
                Some("Write line breaks and tabs inside strings as `\\n` and `\\t`".to_owned())
            }
            ErrorKind::InvalidEscape => Some(
                "Valid escapes are `\\\"`, `\\\\`, `\\/`, `\\b`, `\\f`, `\\n`, `\\r`, `\\t` and `\\uXXXX`"
                    .to_owned(),
            ),
            ErrorKind::InvalidUnicodeEscape => {
                Some("`\\u` must be followed by exactly 4 hexadecimal digits".to_owned())
            }
            ErrorKind::LoneSurrogate => Some(
                "A UTF-16 surrogate `\\uD800`-`\\uDFFF` must be followed by its pair `\\uXXXX`"
                    .to_owned(),
            ),
            ErrorKind::TrailingComma => {
                Some("Remove the comma before the closing bracket, or enable tolerant mode".to_owned())
            }
            ErrorKind::CommentNotAllowed => {
                Some("Remove the comment, or enable tolerant mode".to_owned())
            }
            ErrorKind::UnterminatedComment => Some("Close the comment with `*/`".to_owned()),
            ErrorKind::TrailingContent => Some(
                "A document holds a single value: wrap several values in an array, or remove the extra text"
                    .to_owned(),
            ),
            ErrorKind::InvalidUtf8 | ErrorKind::TooLarge | ErrorKind::InvalidNumber => None,
        }
    }
}

fn unexpected_char_hint(
    source: &[u8],
    offset: usize,
    found: char,
    expected: &str,
) -> Option<String> {
    if matches!(found, '}' | ']') && expected != "a value" {
        let open = innermost_open(source, offset)?;
        let closer = closer_of(open.byte);
        return (found != closer).then(|| {
            format!(
                "`{closer}` is expected to close the `{}` opened at line {}, but found `{found}`",
                open.byte as char,
                open.line(source)
            )
        });
    }
    if expected.starts_with("',' or") {
        return starts_value(found).then(|| "A comma `,` is missing before this value".to_owned());
    }
    match expected {
        "a quoted key" if found == '\'' => {
            Some("JSON uses double quotes `\"` for keys, not single quotes".to_owned())
        }
        "a quoted key" if found.is_alphabetic() || found == '_' => {
            Some("Object keys must be quoted with double quotes".to_owned())
        }
        "':'" => Some("Keys and values are separated by a colon `:`".to_owned()),
        "a value" => match found {
            '\'' => Some("JSON uses double quotes `\"` for strings, not single quotes".to_owned()),
            ',' | '}' | ']' => Some("A value is missing before this character".to_owned()),
            '+' | '.' => Some("Numbers cannot start with `+` or `.`".to_owned()),
            c if c.is_alphabetic() => Some(
                "Not a JSON value: use `true`, `false`, `null`, a number or a quoted string"
                    .to_owned(),
            ),
            _ => None,
        },
        _ => None,
    }
}

fn starts_value(c: char) -> bool {
    matches!(c, '"' | '-' | '{' | '[' | 't' | 'f' | 'n') || c.is_ascii_digit()
}

fn closer_of(open: u8) -> char {
    if open == b'{' { '}' } else { ']' }
}

/// An opening bracket that has not been closed yet.
#[derive(Clone, Copy)]
struct Open {
    byte: u8,
    offset: usize,
}

impl Open {
    fn line(self, source: &[u8]) -> u32 {
        line_column(source, self.offset).0
    }
}

fn unclosed(source: &[u8], open: Open) -> String {
    format!(
        "Unclosed `{}` opened at line {}: add the closing `{}`",
        open.byte as char,
        open.line(source),
        closer_of(open.byte)
    )
}

/// Innermost bracket open before `end`, skipping the contents of strings.
fn innermost_open(source: &[u8], end: usize) -> Option<Open> {
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for (offset, &byte) in source.iter().enumerate().take(end) {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => stack.push(Open { byte, offset }),
            b'}' | b']' => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack.last().copied()
}

// Column counts UTF-8 characters: continuation bytes (`10xxxxxx`) are skipped.
fn line_column(bytes: &[u8], offset: usize) -> (u32, u32) {
    let before = bytes.get(..offset).unwrap_or(bytes);
    let line_start = before
        .iter()
        .rposition(|&c| c == b'\n')
        .map_or(0, |i| i + 1);
    let line = before.iter().filter(|&&c| c == b'\n').count() + 1;
    let column = before[line_start..]
        .iter()
        .filter(|&&c| c & 0xC0 != 0x80)
        .count()
        + 1;
    (saturate(line), saturate(column))
}

fn saturate(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_character() {
        assert_eq!(line_column(b"abc", 0), (1, 1));
    }

    #[test]
    fn counts_lines_and_columns() {
        let text = b"{\n  \"a\": 1,\n  x\n}";
        let offset = text.iter().position(|&c| c == b'x').unwrap_or_default();
        assert_eq!(line_column(text, offset), (3, 3));
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        let text = "[\"ñandú\", ?]";
        let offset = text.find('?').unwrap_or_default();
        assert_eq!(line_column(text.as_bytes(), offset), (1, 11));
    }

    #[test]
    fn crlf_and_end_of_text() {
        let text = b"[1,\r\n2";
        assert_eq!(line_column(text, text.len()), (2, 2));
    }

    #[test]
    fn message_includes_position() {
        let e = ParseError::new(ErrorKind::TrailingContent, 2, b"1\n2");
        assert_eq!(
            e.to_string(),
            "line 2, column 1: extra content after the JSON value"
        );
    }

    fn hint_for(text: &str) -> Option<String> {
        let err = crate::parse(text, &crate::ParseOptions::default()).expect_err("should fail");
        err.hint(text.as_bytes())
    }

    fn tolerant_hint_for(text: &str) -> Option<String> {
        let options = crate::ParseOptions {
            mode: crate::Mode::Tolerant,
        };
        let err = crate::parse(text, &options).expect_err("should fail");
        err.hint(text.as_bytes())
    }

    #[test]
    fn unclosed_brace_names_the_bracket_and_its_line() {
        assert_eq!(
            hint_for("{\n  \"a\": [1, 2]").as_deref(),
            Some("Unclosed `{` opened at line 1: add the closing `}`")
        );
        assert_eq!(
            hint_for("{\"a\": [1,\n 2").as_deref(),
            Some("Unclosed `[` opened at line 1: add the closing `]`")
        );
    }

    #[test]
    fn brackets_inside_strings_are_not_counted() {
        assert_eq!(
            hint_for("[\"{[\\\"\", 1").as_deref(),
            Some("Unclosed `[` opened at line 1: add the closing `]`")
        );
    }

    #[test]
    fn extra_text_after_a_value() {
        assert_eq!(
            hint_for("\"a\" ,").as_deref(),
            Some(
                "A document holds a single value: wrap several values in an array, or remove the extra text"
            )
        );
    }

    #[test]
    fn missing_comma() {
        let hint = Some("A comma `,` is missing before this value".to_owned());
        assert_eq!(hint_for("[1 2]"), hint);
        assert_eq!(hint_for("{\"a\": 1 \"b\": 2}"), hint);
        assert_eq!(hint_for("[\"a\" \"b\"]"), hint);
        assert_eq!(hint_for("[{} {}]"), hint);
    }

    #[test]
    fn mismatched_closing_bracket() {
        assert_eq!(
            hint_for("{\"a\": [1, 2}").as_deref(),
            Some("`]` is expected to close the `[` opened at line 1, but found `}`")
        );
        assert_eq!(
            hint_for("[1,\n{\"a\": 1]").as_deref(),
            Some("`}` is expected to close the `{` opened at line 2, but found `]`")
        );
    }

    #[test]
    fn unquoted_and_single_quoted_keys() {
        assert_eq!(
            hint_for("{a: 1}").as_deref(),
            Some("Object keys must be quoted with double quotes")
        );
        assert_eq!(
            hint_for("{'a': 1}").as_deref(),
            Some("JSON uses double quotes `\"` for keys, not single quotes")
        );
    }

    #[test]
    fn missing_colon() {
        assert_eq!(
            hint_for("{\"a\" 1}").as_deref(),
            Some("Keys and values are separated by a colon `:`")
        );
    }

    #[test]
    fn invalid_values() {
        assert_eq!(
            hint_for("['a']").as_deref(),
            Some("JSON uses double quotes `\"` for strings, not single quotes")
        );
        assert_eq!(
            hint_for("{\"a\":}").as_deref(),
            Some("A value is missing before this character")
        );
        assert_eq!(
            hint_for("[1,,2]").as_deref(),
            Some("A value is missing before this character")
        );
        assert_eq!(
            hint_for("[undefined]").as_deref(),
            Some("Not a JSON value: use `true`, `false`, `null`, a number or a quoted string")
        );
        assert_eq!(
            hint_for("[+1]").as_deref(),
            Some("Numbers cannot start with `+` or `.`")
        );
    }

    #[test]
    fn strict_mode_extensions_suggest_tolerant_mode() {
        assert_eq!(
            hint_for("[1,]").as_deref(),
            Some("Remove the comma before the closing bracket, or enable tolerant mode")
        );
        assert_eq!(
            hint_for("[1 // one\n]").as_deref(),
            Some("Remove the comment, or enable tolerant mode")
        );
    }

    #[test]
    fn string_and_number_errors() {
        assert_eq!(
            hint_for("[\"abc").as_deref(),
            Some("Close the string with a double quote `\"`")
        );
        assert_eq!(
            hint_for("[\"a\nb\"]").as_deref(),
            Some("Write line breaks and tabs inside strings as `\\n` and `\\t`")
        );
        assert!(hint_for("[\"\\x\"]").is_some_and(|h| h.starts_with("Valid escapes")));
        assert!(hint_for("[\"\\u12\"]").is_some_and(|h| h.contains("4 hexadecimal digits")));
        assert_eq!(
            hint_for("[012]").as_deref(),
            Some("Remove the extra zeros before the number")
        );
        assert_eq!(
            hint_for("[True]").as_deref(),
            Some("Not a JSON value: use `true`, `false`, `null`, a number or a quoted string")
        );
        assert_eq!(
            hint_for("[tru]").as_deref(),
            Some("Literals are lowercase: `true`, `false` and `null`")
        );
    }

    #[test]
    fn lone_surrogate() {
        assert!(hint_for("[\"\\ud800\"]").is_some_and(|h| h.contains("surrogate")));
    }

    #[test]
    fn extra_content_and_unterminated_comment() {
        assert!(
            hint_for("{} {}").is_some_and(|h| h.starts_with("A document holds a single value"))
        );
        assert_eq!(
            tolerant_hint_for("[1 /* open").as_deref(),
            Some("Close the comment with `*/`")
        );
    }

    #[test]
    fn errors_without_advice_return_none() {
        assert_eq!(hint_for("[-]"), None);
        assert_eq!(hint_for("[1 :]"), None);
    }

    #[test]
    fn empty_and_deep_input_do_not_panic() {
        let _ = hint_for("");
        let deep = "[".repeat(10_000);
        assert_eq!(
            hint_for(&deep).as_deref(),
            Some("Unclosed `[` opened at line 1: add the closing `]`")
        );
    }
}
