//! Format, minify and sort keys on the Viewer text, on a background thread. Independent of GTK.

use lljson_core::{Mode, ParseError, ParseOptions, Source, parse};

use crate::transform::Transform;

/// Result of a transform: the new text, or why the text could not be parsed.
pub type Reformatted = Result<String, ParseError>;

/// Parses `text` in `mode` and applies `transform`. Tolerant mode drops comments and trailing
/// commas along the way, since the output is standard JSON.
pub fn reformat(text: String, transform: Transform, mode: Mode) -> Reformatted {
    let parsed = parse(Source::from(text), &ParseOptions { mode })?;
    Ok(transform.apply(&parsed.document))
}

/// The receiver yields exactly one result, or closes without one if the thread could not be
/// created.
pub fn spawn_reformat(
    text: String,
    transform: Transform,
    mode: Mode,
) -> async_channel::Receiver<Reformatted> {
    let (sender, receiver) = async_channel::bounded(1);
    let _ = std::thread::Builder::new()
        .name("lljson-reformat".to_owned())
        .spawn(move || {
            // A dropped receiver (stale request) just discards the result.
            let _ = sender.send_blocking(reformat(text, transform, mode));
        });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use lljson_core::{ErrorKind, Indent};

    fn run(text: &str, transform: Transform) -> Reformatted {
        reformat(text.to_owned(), transform, Mode::Strict)
    }

    #[test]
    fn formats_with_the_chosen_indent() {
        assert_eq!(
            run(r#"{"a":[1]}"#, Transform::Format(Indent::Spaces(2))).as_deref(),
            Ok("{\n  \"a\": [\n    1\n  ]\n}")
        );
    }

    #[test]
    fn minifies() {
        assert_eq!(
            run("{\n  \"a\": [ 1, 2 ]\n}", Transform::Minify).as_deref(),
            Ok(r#"{"a":[1,2]}"#)
        );
    }

    #[test]
    fn sorts_keys_at_every_level() {
        assert_eq!(
            run(r#"{"b":{"d":1,"c":2},"a":0}"#, Transform::SortKeys).as_deref(),
            Ok("{\n  \"a\": 0,\n  \"b\": {\n    \"c\": 2,\n    \"d\": 1\n  }\n}")
        );
    }

    #[test]
    fn invalid_text_reports_the_parse_error() {
        let err = run("[1,", Transform::Minify).expect_err("should fail");
        assert!(matches!(err.kind, ErrorKind::UnexpectedEnd { .. }));
    }

    #[test]
    fn empty_text_is_an_error() {
        assert!(run("", Transform::Minify).is_err());
    }

    #[test]
    fn strict_mode_rejects_comments_and_tolerant_mode_drops_them() {
        let text = "[1, // one\n 2,]";
        assert!(run(text, Transform::Minify).is_err());
        assert_eq!(
            reformat(text.to_owned(), Transform::Minify, Mode::Tolerant).as_deref(),
            Ok("[1,2]")
        );
    }

    #[test]
    fn spawn_reformat_delivers_one_result() {
        let receiver = spawn_reformat("[ 1 ]".to_owned(), Transform::Minify, Mode::Strict);
        assert_eq!(receiver.recv_blocking(), Ok(Ok("[1]".to_owned())));
        assert!(receiver.recv_blocking().is_err(), "only one result");
    }
}
