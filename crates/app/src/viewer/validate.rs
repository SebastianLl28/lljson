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
    Valid,
    Invalid(ParseError),
}

/// Parses `text` as strict JSON.
pub fn validate(text: String) -> Validation {
    if text.trim().is_empty() {
        return Validation::Empty;
    }
    let options = ParseOptions { mode: Mode::Strict };
    match parse(Source::from(text), &options) {
        Ok(_) => Validation::Valid,
        Err(err) => Validation::Invalid(err),
    }
}

/// Parses on a background thread; the receiver yields exactly one result, or closes without
/// one if the thread could not be created.
pub fn spawn_validate(text: String) -> async_channel::Receiver<Validation> {
    let (sender, receiver) = async_channel::bounded(1);
    // A failed spawn drops the sender with the closure, which closes the channel: the
    // caller sees no result and keeps the previous status.
    let _ = std::thread::Builder::new()
        .name("lljson-validate".to_owned())
        .spawn(move || {
            // A dropped receiver (stale request) just discards the result.
            let _ = sender.send_blocking(validate(text));
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

    pub fn is_current(&self, id: u64) -> bool {
        self.0.get() == id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lljson_core::ErrorKind;

    #[test]
    fn blank_text_is_empty_not_an_error() {
        for text in ["", "  ", "\n\t \r\n"] {
            assert_eq!(validate(text.to_owned()), Validation::Empty);
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
            assert_eq!(validate(text.to_owned()), Validation::Valid, "{text}");
        }
    }

    #[test]
    fn invalid_documents_report_line_and_column() {
        let Validation::Invalid(err) = validate("{\n  \"a\": 1,\n  x\n}".to_owned()) else {
            panic!("expected an error");
        };
        assert_eq!((err.line, err.column), (3, 3));
        assert!(matches!(err.kind, ErrorKind::UnexpectedChar { .. }));
    }

    #[test]
    fn truncated_text_is_an_error_at_the_end() {
        let Validation::Invalid(err) = validate("[1, 2".to_owned()) else {
            panic!("expected an error");
        };
        assert!(matches!(err.kind, ErrorKind::UnexpectedEnd { .. }));
    }

    #[test]
    fn comments_and_trailing_commas_are_errors_in_strict_mode() {
        for text in ["[1, // one\n 2]", "{\"a\": 1,}"] {
            assert!(matches!(validate(text.to_owned()), Validation::Invalid(_)));
        }
    }

    #[test]
    fn spawn_validate_delivers_one_result() {
        let receiver = spawn_validate("[1]".to_owned());
        assert_eq!(receiver.recv_blocking(), Ok(Validation::Valid));
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
}
