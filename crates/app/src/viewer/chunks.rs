//! Splits text into slices for incremental insertion into the editor, so that no single
//! insertion blocks the UI thread for long. Independent of GTK.

/// Slices of at most `max` bytes (a single character wider than `max` is its own slice),
/// ending after a newline when one fits and never inside a UTF-8 character.
pub fn chunks(text: &str, max: usize) -> Chunks<'_> {
    Chunks { rest: text, max }
}

pub struct Chunks<'a> {
    rest: &'a str,
    max: usize,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        if self.rest.is_empty() {
            return None;
        }
        let mut end = self.max;
        if self.rest.len() <= end {
            end = self.rest.len();
        } else {
            end = self.rest.floor_char_boundary(end);
            if let Some(newline) = self.rest[..end].rfind('\n') {
                end = newline + 1;
            } else if end == 0 {
                end = self.rest.chars().next().map_or(0, char::len_utf8);
            }
        }
        let (chunk, rest) = self.rest.split_at(end);
        self.rest = rest;
        Some(chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(text: &str, max: usize) -> Vec<&str> {
        chunks(text, max).collect()
    }

    #[test]
    fn empty_text_has_no_chunks() {
        assert!(collect("", 8).is_empty());
    }

    #[test]
    fn short_text_is_one_chunk() {
        assert_eq!(collect("abc\ndef", 64), ["abc\ndef"]);
    }

    #[test]
    fn splits_after_the_last_newline_that_fits() {
        assert_eq!(collect("aaa\nbbb\nccc\nddd", 9), ["aaa\nbbb\n", "ccc\nddd"]);
    }

    #[test]
    fn lines_longer_than_the_limit_split_anywhere() {
        assert_eq!(collect("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn never_splits_a_utf8_character() {
        // "é" is 2 bytes, "€" 3 bytes, "😀" 4 bytes.
        let text = "é€😀é€😀é€😀";
        for max in 1..12 {
            let parts = collect(text, max);
            assert_eq!(parts.concat(), text, "max {max}");
            assert!(parts.iter().all(|p| !p.is_empty()), "max {max}");
        }
    }

    #[test]
    fn chunks_concatenate_to_the_original_and_respect_the_limit() {
        let text = "line one\nline two\n\nline four with more text\nx\n";
        for max in [5, 9, 17, 100] {
            let parts = collect(text, max);
            assert_eq!(parts.concat(), text, "max {max}");
            // A character wider than `max` may exceed it; ASCII text never does.
            assert!(parts.iter().all(|p| p.len() <= max), "max {max}");
        }
    }
}
