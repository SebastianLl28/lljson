//! Live validation of the Viewer text: parsing on a background thread and the counter that
//! discards stale results. Independent of GTK.

use std::cell::Cell;

use lljson_core::{Mode, ParseError, ParseOptions, Source, parse};

/// Quiet time after the last edit before the text is parsed.
pub const DEBOUNCE_MS: u64 = 150;

#[derive(Debug, PartialEq, Eq)]
pub enum Validation {
    /// Nothing but whitespace: not an error, there is just nothing to validate.
    Empty,
    /// `fixes` counts comments and trailing commas accepted only because of tolerant mode.
    Valid { fixes: usize },
    Invalid {
        error: ParseError,
        /// How to fix it, when `core` has advice for this kind of error.
        hint: Option<String>,
        mark: Mark,
    },
}

/// Where to underline the error in the editor: a single character, located by its line
/// (0-based) and its byte range within that line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    pub line: u32,
    pub start: u32,
    pub end: u32,
}

/// The character to underline for an error at byte `offset`. An error past the end of the text
/// or in whitespace points at the last visible character before it, since there is nothing
/// to underline at the error itself.
pub fn mark_for(text: &str, offset: usize) -> Mark {
    let offset = text.floor_char_boundary(offset.min(text.len()));
    let start = match text[offset..].chars().next() {
        Some(c) if !c.is_whitespace() => offset,
        _ => text[..offset]
            .char_indices()
            .rev()
            .find(|(_, c)| !c.is_whitespace())
            .map_or(offset, |(i, _)| i),
    };
    let width = text[start..].chars().next().map_or(0, char::len_utf8);
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let line = text[..start].bytes().filter(|&b| b == b'\n').count();
    let saturate = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    Mark {
        line: saturate(line),
        start: saturate(start - line_start),
        end: saturate(start - line_start + width),
    }
}

/// Parses `text` in the given mode.
pub fn validate(text: String, mode: Mode) -> Validation {
    if text.trim().is_empty() {
        return Validation::Empty;
    }
    let options = ParseOptions { mode };
    match parse(Source::from(text.as_str()), &options) {
        Ok(parsed) => Validation::Valid {
            fixes: parsed.fixes.len(),
        },
        Err(error) => Validation::Invalid {
            hint: error.hint(text.as_bytes()),
            mark: mark_for(&text, error.offset),
            error,
        },
    }
}

/// Parses on a background thread; the receiver yields exactly one result, or closes without
/// one if the thread could not be created.
pub fn spawn_validate(text: String, mode: Mode) -> async_channel::Receiver<Validation> {
    let (sender, receiver) = async_channel::bounded(1);
    // A failed spawn drops the sender with the closure, which closes the channel: the
    // caller sees no result and keeps the previous status.
    let _ = std::thread::Builder::new()
        .name("lljson-validate".to_owned())
        .spawn(move || {
            // A dropped receiver (stale request) just discards the result.
            let _ = sender.send_blocking(validate(text, mode));
        });
    receiver
}

/// Text without comments or trailing commas, or `None` if it is not valid tolerant JSON or
/// already standard.
pub fn autofix(text: String) -> Option<String> {
    let options = ParseOptions {
        mode: Mode::Tolerant,
    };
    parse(Source::from(text), &options).ok()?.autocorrected()
}

/// Runs [`autofix`] on a background thread; the receiver yields exactly one result, or closes
/// without one if the thread could not be created.
pub fn spawn_autofix(text: String) -> async_channel::Receiver<Option<String>> {
    let (sender, receiver) = async_channel::bounded(1);
    let _ = std::thread::Builder::new()
        .name("lljson-autofix".to_owned())
        .spawn(move || {
            let _ = sender.send_blocking(autofix(text));
        });
    receiver
}

/// Id of the latest validation request; results of older ones are stale.
#[derive(Default)]
pub struct Generation(Cell<u64>);

impl Generation {
    /// Starts a new request, invalidating every earlier one.
    pub fn advance(&self) -> u64 {
        let id = self.0.get().wrapping_add(1);
        self.0.set(id);
        id
    }

    pub fn current(&self) -> u64 {
        self.0.get()
    }

    pub fn is_current(&self, id: u64) -> bool {
        self.0.get() == id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strict(text: String) -> Validation {
        validate(text, Mode::Strict)
    }

    fn tolerant(text: &str) -> Validation {
        validate(text.to_owned(), Mode::Tolerant)
    }
    use lljson_core::ErrorKind;

    #[test]
    fn blank_text_is_empty_not_an_error() {
        for text in ["", "  ", "\n\t \r\n"] {
            assert_eq!(strict(text.to_owned()), Validation::Empty);
        }
    }

    #[test]
    fn valid_documents() {
        for text in [
            "{}",
            "[1, 2]",
            " \"é\" ",
            "null",
            "{\"a\": {\"b\": [true]}}",
        ] {
            assert_eq!(
                strict(text.to_owned()),
                Validation::Valid { fixes: 0 },
                "{text}"
            );
        }
    }

    #[test]
    fn invalid_documents_report_line_and_column() {
        let Validation::Invalid { error: err, .. } = strict("{\n  \"a\": 1,\n  x\n}".to_owned())
        else {
            panic!("expected an error");
        };
        assert_eq!((err.line, err.column), (3, 3));
        assert!(matches!(err.kind, ErrorKind::UnexpectedChar { .. }));
    }

    #[test]
    fn truncated_text_is_an_error_at_the_end() {
        let Validation::Invalid { error: err, .. } = strict("[1, 2".to_owned()) else {
            panic!("expected an error");
        };
        assert!(matches!(err.kind, ErrorKind::UnexpectedEnd { .. }));
    }

    #[test]
    fn comments_and_trailing_commas_are_errors_in_strict_mode() {
        for text in ["[1, // one\n 2]", "{\"a\": 1,}"] {
            assert!(matches!(
                strict(text.to_owned()),
                Validation::Invalid { .. }
            ));
        }
    }

    #[test]
    fn spawn_validate_delivers_one_result() {
        let receiver = spawn_validate("[1]".to_owned(), Mode::Strict);
        assert_eq!(receiver.recv_blocking(), Ok(Validation::Valid { fixes: 0 }));
        assert!(receiver.recv_blocking().is_err(), "only one result");
    }

    #[test]
    fn generation_only_the_latest_is_current() {
        let generation = Generation::default();
        let first = generation.advance();
        assert!(generation.is_current(first));
        let second = generation.advance();
        assert!(!generation.is_current(first));
        assert!(generation.is_current(second));
    }

    #[test]
    fn generation_wraps_without_panicking() {
        let generation = Generation(Cell::new(u64::MAX));
        let id = generation.advance();
        assert_eq!(id, 0);
        assert!(generation.is_current(id));
    }

    #[test]
    fn invalid_documents_carry_a_hint() {
        let Validation::Invalid { hint, .. } = strict("[1 2]".to_owned()) else {
            panic!("expected an error");
        };
        assert_eq!(
            hint.as_deref(),
            Some("A comma `,` is missing before this value")
        );
    }

    #[test]
    fn mark_points_at_the_offending_character() {
        let text = "{\n  \"a\": 1,\n  x\n}";
        let offset = text.find('x').unwrap_or_default();
        assert_eq!(
            mark_for(text, offset),
            Mark {
                line: 2,
                start: 2,
                end: 3
            }
        );
    }

    #[test]
    fn mark_at_the_end_falls_back_to_the_last_visible_character() {
        assert_eq!(
            mark_for("[1, 2", 5),
            Mark {
                line: 0,
                start: 4,
                end: 5
            }
        );
        assert_eq!(
            mark_for("[1,\n", 5),
            Mark {
                line: 0,
                start: 2,
                end: 3
            }
        );
        assert_eq!(
            mark_for("[\n  1,\n\n", 10),
            Mark {
                line: 1,
                start: 3,
                end: 4
            }
        );
    }

    #[test]
    fn mark_handles_multibyte_characters_and_odd_offsets() {
        assert_eq!(
            mark_for("[\"ñ\", ñ]", 7),
            Mark {
                line: 0,
                start: 7,
                end: 9
            }
        );
        // An offset inside a character snaps back to its start.
        assert_eq!(
            mark_for("ñ", 1),
            Mark {
                line: 0,
                start: 0,
                end: 2
            }
        );
        assert_eq!(
            mark_for("", 0),
            Mark {
                line: 0,
                start: 0,
                end: 0
            }
        );
        assert_eq!(
            mark_for("   ", 3),
            Mark {
                line: 0,
                start: 3,
                end: 3
            }
        );
        assert_eq!(
            mark_for("x", 99),
            Mark {
                line: 0,
                start: 0,
                end: 1
            }
        );
    }

    #[test]
    fn tolerant_mode_counts_what_it_forgives() {
        assert_eq!(
            tolerant("{\"a\": 1, // one\n \"b\": [2,],}"),
            Validation::Valid { fixes: 3 }
        );
        assert_eq!(tolerant("[1]"), Validation::Valid { fixes: 0 });
    }

    #[test]
    fn tolerant_mode_still_reports_real_errors() {
        assert!(matches!(tolerant("[1 2]"), Validation::Invalid { .. }));
        assert_eq!(tolerant("  "), Validation::Empty);
    }

    #[test]
    fn autofix_removes_comments_and_trailing_commas() {
        assert_eq!(
            autofix("{\"a\": 1, // one\n \"b\": [2,],}".to_owned()).as_deref(),
            Some("{\"a\": 1, \n \"b\": [2]}")
        );
        assert_eq!(autofix("/* c */ [1]".to_owned()).as_deref(), Some(" [1]"));
    }

    #[test]
    fn autofix_has_nothing_to_do_for_standard_or_broken_json() {
        assert_eq!(autofix("[1]".to_owned()), None);
        assert_eq!(autofix("[1 2]".to_owned()), None);
        assert_eq!(autofix(String::new()), None);
    }

    #[test]
    fn spawn_autofix_delivers_one_result() {
        let receiver = spawn_autofix("[1,]".to_owned());
        assert_eq!(receiver.recv_blocking(), Ok(Some("[1]".to_owned())));
        assert!(receiver.recv_blocking().is_err(), "only one result");
    }
}
