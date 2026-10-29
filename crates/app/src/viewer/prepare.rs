//! Text preparation before it reaches the editor: valid JSON is pretty-printed so that
//! minified documents do not become a single huge line (which `GtkTextView` lays out
//! super-linearly). Runs on a background thread; independent of GTK.

use lljson_core::{FormatOptions, Indent, Mode, ParseOptions, Source, format, parse};

/// Pretty-prints `text` with 2 spaces when it is valid strict JSON. Anything else (invalid,
/// empty, JSONC with comments or trailing commas) is returned untouched.
pub fn prepare(text: String) -> String {
    let options = ParseOptions { mode: Mode::Strict };
    match parse(Source::from(text.as_str()), &options) {
        Ok(parsed) => format(
            &parsed.document,
            &FormatOptions {
                indent: Indent::Spaces(2),
                sort_keys: false,
            },
        ),
        Err(_) => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minified_json_is_indented_with_two_spaces() {
        assert_eq!(
            prepare(r#"{"a":[1,2]}"#.to_owned()),
            "{\n  \"a\": [\n    1,\n    2\n  ]\n}"
        );
    }

    #[test]
    fn already_indented_json_is_unchanged() {
        let text = "{\n  \"a\": 1\n}".to_owned();
        assert_eq!(prepare(text.clone()), text);
    }

    #[test]
    fn invalid_json_is_unchanged() {
        for text in ["{\"a\": ", "{\"a\":}", "not json", "[1,2"] {
            assert_eq!(prepare(text.to_owned()), text);
        }
    }

    #[test]
    fn jsonc_is_unchanged_so_comments_survive() {
        for text in ["[1, // one\n 2]", "{\"a\": 1,}"] {
            assert_eq!(prepare(text.to_owned()), text);
        }
    }

    #[test]
    fn empty_and_blank_text_are_unchanged() {
        for text in ["", "  \n\t"] {
            assert_eq!(prepare(text.to_owned()), text);
        }
    }

    #[test]
    fn scalars_are_kept_exactly() {
        assert_eq!(prepare("  1.50e3 ".to_owned()), "1.50e3");
        assert_eq!(prepare("\"é\"".to_owned()), "\"é\"");
    }
}
