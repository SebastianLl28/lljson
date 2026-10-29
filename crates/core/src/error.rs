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
}
