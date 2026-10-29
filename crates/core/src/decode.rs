use std::borrow::Cow;
use std::str::Chars;

/// Decodes JSON escapes (string given without quotes); borrows when there are none.
/// Invalid sequences (already rejected by the parser) become U+FFFD instead of failing.
pub(crate) fn unescape(raw: &str) -> Cow<'_, str> {
    if !raw.contains('\\') {
        return Cow::Borrowed(raw);
    }
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('u') => {
                out.push(unicode_escape(&mut chars).unwrap_or(char::REPLACEMENT_CHARACTER))
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    Cow::Owned(out)
}

fn unicode_escape(chars: &mut Chars<'_>) -> Option<char> {
    let unit = hex4(chars)?;
    if !(0xD800..=0xDBFF).contains(&unit) {
        return char::from_u32(unit);
    }
    let mut ahead = chars.clone();
    if ahead.next()? != '\\' || ahead.next()? != 'u' {
        return None;
    }
    let low = hex4(&mut ahead)?;
    if !(0xDC00..=0xDFFF).contains(&low) {
        return None;
    }
    *chars = ahead;
    char::from_u32(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00))
}

fn hex4(chars: &mut Chars<'_>) -> Option<u32> {
    let mut value = 0;
    for _ in 0..4 {
        value = value * 16 + chars.next()?.to_digit(16)?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_borrowed() {
        assert!(matches!(unescape("hello"), Cow::Borrowed("hello")));
    }

    #[test]
    fn simple_escapes() {
        assert_eq!(
            unescape(r#"a\"b\\c\/d\b\f\n\r\t"#),
            "a\"b\\c/d\u{8}\u{c}\n\r\t"
        );
    }

    #[test]
    fn unicode_escapes() {
        assert_eq!(unescape(r"\u00e1\u4E2D"), "á中");
        assert_eq!(unescape(r"\ud83d\ude00!"), "😀!");
    }

    #[test]
    fn broken_escapes_become_replacement_char() {
        assert_eq!(unescape(r"\ud83d"), "\u{fffd}");
        assert_eq!(unescape(r"\ude00x"), "\u{fffd}x");
        assert_eq!(unescape(r"\u12"), "\u{fffd}");
    }
}
