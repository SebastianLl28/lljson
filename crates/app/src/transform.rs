//! Document text transforms (format, minify, sort keys): pure functions over `core`
//! that the window runs on a background thread.

use lljson_core::{Document, FormatOptions, Indent, format, minify};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    Format(Indent),
    Minify,
    /// Sorts the keys of every object and formats with 2 spaces.
    SortKeys,
}

impl Transform {
    pub fn format_from_param(param: &str) -> Option<Transform> {
        let indent = match param {
            "2" => Indent::Spaces(2),
            "4" => Indent::Spaces(4),
            "tab" => Indent::Tab,
            _ => return None,
        };
        Some(Transform::Format(indent))
    }

    pub fn apply(self, doc: &Document) -> String {
        match self {
            Transform::Format(indent) => format(
                doc,
                &FormatOptions {
                    indent,
                    sort_keys: false,
                },
            ),
            Transform::Minify => minify(doc),
            Transform::SortKeys => format(
                doc,
                &FormatOptions {
                    indent: Indent::Spaces(2),
                    sort_keys: true,
                },
            ),
        }
    }

    pub fn done_message(self) -> &'static str {
        match self {
            Transform::Format(Indent::Spaces(2)) => "Formatted with 2 spaces",
            Transform::Format(Indent::Spaces(4)) => "Formatted with 4 spaces",
            Transform::Format(_) => "Formatted",
            Transform::Minify => "Minified",
            Transform::SortKeys => "Keys sorted",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lljson_core::{ParseOptions, parse};

    fn doc(text: &str) -> Document {
        parse(text, &ParseOptions::default())
            .expect("valid JSON")
            .document
    }

    #[test]
    fn parses_format_params() {
        assert_eq!(
            Transform::format_from_param("2"),
            Some(Transform::Format(Indent::Spaces(2)))
        );
        assert_eq!(
            Transform::format_from_param("4"),
            Some(Transform::Format(Indent::Spaces(4)))
        );
        assert_eq!(
            Transform::format_from_param("tab"),
            Some(Transform::Format(Indent::Tab))
        );
        assert_eq!(Transform::format_from_param("8"), None);
        assert_eq!(Transform::format_from_param(""), None);
    }

    #[test]
    fn formats_with_the_requested_indent() {
        let d = doc(r#"{"a":[1,{"b":2}]}"#);
        let two = Transform::Format(Indent::Spaces(2)).apply(&d);
        assert_eq!(
            two,
            "{\n  \"a\": [\n    1,\n    {\n      \"b\": 2\n    }\n  ]\n}"
        );
        let tab = Transform::Format(Indent::Tab).apply(&d);
        assert!(tab.contains("\n\t\"a\""), "{tab:?}");
        let four = Transform::Format(Indent::Spaces(4)).apply(&d);
        assert!(four.contains("\n    \"a\""), "{four:?}");
    }

    #[test]
    fn minifies() {
        let d = doc("{ \"a\" : [ 1 , 2 ] ,\n \"b\" : \"x y\" }");
        assert_eq!(Transform::Minify.apply(&d), r#"{"a":[1,2],"b":"x y"}"#);
    }

    #[test]
    fn sorts_keys_at_every_level_and_keeps_array_order() {
        let d = doc(r#"{"b":1,"a":{"z":[3,1,2],"y":null}}"#);
        assert_eq!(
            Transform::SortKeys.apply(&d),
            "{\n  \"a\": {\n    \"y\": null,\n    \"z\": [\n      3,\n      1,\n      2\n    ]\n  },\n  \"b\": 1\n}"
        );
    }

    #[test]
    fn results_parse_back_to_the_same_structure() {
        let text = r#"{"k":"é\né","n":1.50e3,"e":{},"l":[]}"#;
        let d = doc(text);
        for t in [
            Transform::Format(Indent::Spaces(4)),
            Transform::Minify,
            Transform::SortKeys,
        ] {
            let again = doc(&t.apply(&d));
            assert_eq!(again.node_count(), d.node_count(), "{t:?}");
        }
        // Minify keeps the original text of strings and numbers.
        assert_eq!(Transform::Minify.apply(&d), text);
    }

    #[test]
    fn scalars_and_empty_containers() {
        assert_eq!(Transform::Minify.apply(&doc(" 42 ")), "42");
        assert_eq!(Transform::SortKeys.apply(&doc("[]")), "[]");
        assert_eq!(Transform::Format(Indent::Tab).apply(&doc("{}")), "{}");
    }
}
