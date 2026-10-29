//! Transformaciones de texto de un documento: formatear, minificar y ordenar claves.
//! Son funciones puras sobre `core`; la ventana las ejecuta en un hilo de fondo.

use lljson_core::{Document, FormatOptions, Indent, format, minify};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    Format(Indent),
    Minify,
    /// Ordena las claves de cada objeto y formatea con 2 espacios.
    SortKeys,
}

impl Transform {
    /// Interpreta el parámetro de la acción de formatear: `2`, `4` o `tab`.
    pub fn format_from_param(param: &str) -> Option<Transform> {
        let indent = match param {
            "2" => Indent::Spaces(2),
            "4" => Indent::Spaces(4),
            "tab" => Indent::Tab,
            _ => return None,
        };
        Some(Transform::Format(indent))
    }

    /// Texto del documento tras la transformación.
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

    /// Mensaje para el usuario cuando termina.
    pub fn done_message(self) -> &'static str {
        match self {
            Transform::Format(Indent::Spaces(2)) => "Formateado con 2 espacios",
            Transform::Format(Indent::Spaces(4)) => "Formateado con 4 espacios",
            Transform::Format(_) => "Formateado",
            Transform::Minify => "Minificado",
            Transform::SortKeys => "Claves ordenadas",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lljson_core::{ParseOptions, parse};

    fn doc(text: &str) -> Document {
        parse(text, &ParseOptions::default())
            .expect("JSON válido")
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
        // Minificar conserva el texto original de strings y números.
        assert_eq!(Transform::Minify.apply(&d), text);
    }

    #[test]
    fn scalars_and_empty_containers() {
        assert_eq!(Transform::Minify.apply(&doc(" 42 ")), "42");
        assert_eq!(Transform::SortKeys.apply(&doc("[]")), "[]");
        assert_eq!(Transform::Format(Indent::Tab).apply(&doc("{}")), "{}");
    }
}
