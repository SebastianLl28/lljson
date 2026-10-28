//! Formateador: genera texto JSON a partir del arena.
//! Conserva el texto original de strings y números (escapes y precisión intactos).
//! Es iterativo: la profundidad no está limitada por la pila del hilo.

use crate::document::{Document, Kind, Node, NodeId};

/// Indentación del texto formateado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indent {
    Spaces(u8),
    Tab,
    /// Sin saltos de línea ni espacios (minificado).
    None,
}

impl Default for Indent {
    fn default() -> Indent {
        Indent::Spaces(2)
    }
}

#[derive(Clone, Debug, Default)]
pub struct FormatOptions {
    pub indent: Indent,
}

/// Formatea el documento completo.
pub fn format(doc: &Document, options: &FormatOptions) -> String {
    format_node(doc, doc.root(), options)
}

/// Minifica el documento completo: sin espacios ni saltos de línea.
pub fn minify(doc: &Document) -> String {
    format(
        doc,
        &FormatOptions {
            indent: Indent::None,
        },
    )
}

/// Formatea el subárbol de un nodo (sin su clave). Un id inválido produce `""`.
pub fn format_node(doc: &Document, id: NodeId, options: &FormatOptions) -> String {
    let unit = match options.indent {
        Indent::Spaces(n) => " ".repeat(usize::from(n)),
        Indent::Tab => "\t".to_owned(),
        Indent::None => String::new(),
    };
    let compact = options.indent == Indent::None;
    let estimate = doc.node(id).map_or(0, |n| n.value_span().len());
    let mut formatter = Formatter {
        doc,
        unit,
        compact,
        out: String::with_capacity(estimate + estimate / 2),
        stack: Vec::new(),
    };
    formatter.run(id);
    formatter.out
}

/// Contenedor abierto mientras se escriben sus hijos.
struct Frame {
    next: Option<NodeId>,
    /// Profundidad de los hijos.
    depth: usize,
    object: bool,
    first: bool,
}

struct Formatter<'a> {
    doc: &'a Document,
    unit: String,
    compact: bool,
    out: String,
    stack: Vec<Frame>,
}

impl Formatter<'_> {
    fn run(&mut self, id: NodeId) {
        self.value(id, 0);
        while let Some(frame) = self.stack.last_mut() {
            let depth = frame.depth;
            let Some(child) = frame.next else {
                let close = if frame.object { '}' } else { ']' };
                self.stack.pop();
                self.newline(depth.saturating_sub(1));
                self.out.push(close);
                continue;
            };
            frame.next = self.doc.node(child).and_then(Node::next_sibling);
            let first = std::mem::replace(&mut frame.first, false);
            if !first {
                self.out.push(',');
            }
            self.newline(depth);
            if let Some(key) = self.doc.node(child).and_then(Node::key_span) {
                self.out.push('"');
                self.out.push_str(self.doc.text(key));
                self.out.push_str(if self.compact { "\":" } else { "\": " });
            }
            self.value(child, depth);
        }
    }

    /// Escribe un escalar completo o abre un contenedor (sus hijos los escribe `run`).
    fn value(&mut self, id: NodeId, depth: usize) {
        let Some(node) = self.doc.node(id) else {
            return;
        };
        match node.kind() {
            Kind::String => {
                self.out.push('"');
                self.out.push_str(self.doc.text(node.value_span()));
                self.out.push('"');
            }
            Kind::Null | Kind::Bool | Kind::Number => {
                self.out.push_str(self.doc.text(node.value_span()));
            }
            Kind::Array | Kind::Object => {
                let object = node.kind() == Kind::Object;
                match node.first_child() {
                    None => self.out.push_str(if object { "{}" } else { "[]" }),
                    Some(first) => {
                        self.out.push(if object { '{' } else { '[' });
                        self.stack.push(Frame {
                            next: Some(first),
                            depth: depth + 1,
                            object,
                            first: true,
                        });
                    }
                }
            }
        }
    }

    fn newline(&mut self, depth: usize) {
        if self.compact {
            return;
        }
        self.out.push('\n');
        for _ in 0..depth {
            self.out.push_str(&self.unit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{Mode, ParseOptions, parse};

    fn doc(text: &str) -> Document {
        let options = ParseOptions {
            mode: Mode::Tolerant,
        };
        match parse(text, &options) {
            Ok(parsed) => parsed.document,
            Err(e) => panic!("{text:?} debería parsear: {e}"),
        }
    }

    fn pretty(text: &str) -> String {
        format(&doc(text), &FormatOptions::default())
    }

    #[test]
    fn formats_nested_structure() {
        let text = r#"{"a":[1,{"b":null}],"c":"x","d":{},"e":[]}"#;
        let expected = "{\n  \"a\": [\n    1,\n    {\n      \"b\": null\n    }\n  ],\n  \"c\": \"x\",\n  \"d\": {},\n  \"e\": []\n}";
        assert_eq!(pretty(text), expected);
    }

    #[test]
    fn scalars_and_empty_containers() {
        assert_eq!(pretty(" true "), "true");
        assert_eq!(pretty(r#" "s" "#), r#""s""#);
        assert_eq!(pretty("[ ]"), "[]");
        assert_eq!(pretty("{ }"), "{}");
    }

    #[test]
    fn custom_indentation() {
        let d = doc(r#"{"a":[1]}"#);
        let four = FormatOptions {
            indent: Indent::Spaces(4),
        };
        assert_eq!(format(&d, &four), "{\n    \"a\": [\n        1\n    ]\n}");
        let tab = FormatOptions {
            indent: Indent::Tab,
        };
        assert_eq!(format(&d, &tab), "{\n\t\"a\": [\n\t\t1\n\t]\n}");
    }

    #[test]
    fn keeps_original_text_of_strings_and_numbers() {
        let text = r#"{"k\n": "a\"b\/c", "n": 1.50e+3, "big": 123456789012345678901234567890}"#;
        let expected = "{\n  \"k\\n\": \"a\\\"b\\/c\",\n  \"n\": 1.50e+3,\n  \"big\": 123456789012345678901234567890\n}";
        assert_eq!(pretty(text), expected);
    }

    #[test]
    fn drops_comments_and_trailing_commas() {
        assert_eq!(pretty("[1, // uno\n 2, /* dos */]"), "[\n  1,\n  2\n]");
    }

    #[test]
    fn minifies() {
        let text =
            "{\n  \"a\": [ 1, {\"b\": null} ],\n  \"c\": \"x y\", \"d\": {}, \"e\": [] // fin\n}";
        assert_eq!(
            minify(&doc(text)),
            r#"{"a":[1,{"b":null}],"c":"x y","d":{},"e":[]}"#
        );
        assert_eq!(minify(&doc(" [ ] ")), "[]");
        assert_eq!(minify(&doc(" 1.0e2 ")), "1.0e2");
    }

    #[test]
    fn minified_output_reparses_to_the_same_value() {
        let text = "{\"u\": [ {\"id\": 1, \"t\": [\"a\", \"b\"]}, {\"x\": null} ], \"s\": \"\\n\"}";
        let original = doc(text);
        let minified = doc(&minify(&original));
        assert_eq!(
            original.hash(original.root()),
            minified.hash(minified.root())
        );
    }

    #[test]
    fn minifies_very_deep_documents() {
        const DEPTH: usize = 200_000;
        let text = format!("{}{}", r#"{"a":["#.repeat(DEPTH), "]}".repeat(DEPTH));
        assert_eq!(minify(&doc(&text)), text);
    }

    #[test]
    fn formats_a_subtree() {
        let d = doc(r#"{"a": {"b": [true]}}"#);
        let a = d.children(d.root()).next().unwrap_or(d.root());
        let out = format_node(&d, a, &FormatOptions::default());
        assert_eq!(out, "{\n  \"b\": [\n    true\n  ]\n}");
        assert_eq!(
            format_node(&d, NodeId::from_index(99), &FormatOptions::default()),
            ""
        );
    }

    #[test]
    fn output_reparses_to_the_same_value() {
        let text = r#"{"u":[{"id":1,"t":["a","b"],"ok":true},{"id":2,"x":null,"p":-1.5e3}],"n":2}"#;
        let original = doc(text);
        let formatted = doc(&pretty(text));
        assert_eq!(
            original.hash(original.root()),
            formatted.hash(formatted.root())
        );
    }

    #[test]
    fn deep_nesting_does_not_overflow_the_stack() {
        const DEPTH: usize = 2_000;
        let text = format!("{}{}", "[".repeat(DEPTH), "]".repeat(DEPTH));
        let out = pretty(&text);
        assert_eq!(out.matches('[').count(), DEPTH);
        assert_eq!(out.lines().count(), 2 * DEPTH - 1);
        assert!(out.contains("[]") && out.ends_with("\n]"));
    }
}
