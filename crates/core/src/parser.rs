//! Parser JSON que construye el arena directamente, sin árbol intermedio.
//! Es iterativo (pila explícita): la profundidad de anidamiento no está limitada
//! por la pila del hilo.

use crate::document::{Document, Kind, Node, NodeId, Span, flags};
use crate::error::{ErrorKind, ParseError};
use crate::source::Source;

/// Los rangos se guardan como `u32`.
const MAX_SOURCE_LEN: usize = u32::MAX as usize;
const BOM: &[u8] = b"\xEF\xBB\xBF";

/// Modo de parseo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// JSON estándar (RFC 8259).
    #[default]
    Strict,
    /// Acepta además comentarios `//` y `/* */` y comas finales (JSONC).
    Tolerant,
}

#[derive(Clone, Debug, Default)]
pub struct ParseOptions {
    pub mode: Mode,
}

/// Construcción no estándar aceptada en modo tolerante.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixKind {
    LineComment,
    BlockComment,
    TrailingComma,
}

/// Fragmento del texto original que habría que quitar para obtener JSON estándar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fix {
    pub kind: FixKind,
    pub span: Span,
}

/// Resultado de un parseo correcto.
#[derive(Debug)]
pub struct Parsed {
    pub document: Document,
    /// Construcciones no estándar encontradas (solo en modo tolerante).
    pub fixes: Vec<Fix>,
}

impl Parsed {
    /// Texto original sin comentarios ni comas finales ("autocorregir").
    /// `None` si el documento ya era JSON estándar.
    pub fn autocorrected(&self) -> Option<String> {
        if self.fixes.is_empty() {
            return None;
        }
        let text = std::str::from_utf8(self.document.source_bytes()).ok()?;
        let mut spans: Vec<Span> = self.fixes.iter().map(|f| f.span).collect();
        spans.sort_by_key(|s| s.start);
        let mut out = String::with_capacity(text.len());
        let mut cursor = 0;
        for span in spans {
            out.push_str(text.get(cursor..span.start as usize)?);
            cursor = span.end as usize;
        }
        out.push_str(text.get(cursor..)?);
        Some(out)
    }
}

/// Parsea un documento JSON.
pub fn parse(source: impl Into<Source>, options: &ParseOptions) -> Result<Parsed, ParseError> {
    let source = source.into();
    let tolerant = options.mode == Mode::Tolerant;
    let (nodes, root, fixes) = Parser::new(source.as_bytes(), tolerant)?.run()?;
    Ok(Parsed {
        document: Document::from_parts(source, nodes, root),
        fixes,
    })
}

/// Contenedor abierto en la pila del parser.
struct Frame {
    id: NodeId,
    last_child: NodeId,
    child_count: u32,
    object: bool,
    /// Ya se leyó al menos un elemento (el siguiente debe ir precedido de `,`).
    has_value: bool,
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    tolerant: bool,
    nodes: Vec<Node>,
    stack: Vec<Frame>,
    fixes: Vec<Fix>,
}

impl<'a> Parser<'a> {
    fn new(bytes: &'a [u8], tolerant: bool) -> Result<Parser<'a>, ParseError> {
        if bytes.len() > MAX_SOURCE_LEN {
            return Err(ParseError::new(ErrorKind::TooLarge, 0, &[]));
        }
        if let Err(e) = std::str::from_utf8(bytes) {
            return Err(ParseError::new(
                ErrorKind::InvalidUtf8,
                e.valid_up_to(),
                bytes,
            ));
        }
        Ok(Parser {
            bytes,
            pos: if bytes.starts_with(BOM) { BOM.len() } else { 0 },
            tolerant,
            // Estimación barata para evitar la mayoría de realocaciones.
            nodes: Vec::with_capacity(bytes.len() / 16),
            stack: Vec::new(),
            fixes: Vec::new(),
        })
    }

    fn run(mut self) -> Result<(Vec<Node>, NodeId, Vec<Fix>), ParseError> {
        self.skip_ws()?;
        self.value(Span::EMPTY, 0)?;
        while let Some(top) = self.stack.last() {
            let (object, has_value) = (top.object, top.has_value);
            let close = if object { b'}' } else { b']' };
            self.skip_ws()?;
            if has_value {
                match self.peek() {
                    Some(b',') => {
                        let comma = self.pos;
                        self.pos += 1;
                        self.skip_ws()?;
                        if self.peek() == Some(close) {
                            if !self.tolerant {
                                return Err(self.error(ErrorKind::TrailingComma, comma));
                            }
                            self.fixes.push(Fix {
                                kind: FixKind::TrailingComma,
                                span: Span::new(comma, comma + 1),
                            });
                            self.close_container();
                            continue;
                        }
                    }
                    Some(c) if c == close => {
                        self.close_container();
                        continue;
                    }
                    _ => {
                        let expected = if object { "',' o '}'" } else { "',' o ']'" };
                        return Err(self.unexpected(expected));
                    }
                }
            } else if self.peek() == Some(close) {
                self.close_container();
                continue;
            }

            let (key, key_flags) = if object {
                self.member_key()?
            } else {
                (Span::EMPTY, 0)
            };
            if let Some(top) = self.stack.last_mut() {
                top.has_value = true;
            }
            self.value(key, key_flags)?;
        }
        self.skip_ws()?;
        if self.pos < self.bytes.len() {
            return Err(self.error(ErrorKind::TrailingContent, self.pos));
        }
        Ok((self.nodes, NodeId(0), self.fixes))
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn error(&self, kind: ErrorKind, offset: usize) -> ParseError {
        ParseError::new(kind, offset, self.bytes)
    }

    /// Error en la posición actual: fin del texto o carácter inesperado.
    fn unexpected(&self, expected: &'static str) -> ParseError {
        let kind = match self.char_at(self.pos) {
            Some(found) => ErrorKind::UnexpectedChar { found, expected },
            None => ErrorKind::UnexpectedEnd { expected },
        };
        self.error(kind, self.pos)
    }

    fn char_at(&self, pos: usize) -> Option<char> {
        let first = *self.bytes.get(pos)?;
        let width = match first {
            0x00..=0x7F => 1,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            _ => 4,
        };
        let bytes = self.bytes.get(pos..pos + width)?;
        std::str::from_utf8(bytes).ok()?.chars().next()
    }

    fn skip_ws(&mut self) -> Result<(), ParseError> {
        loop {
            match self.peek() {
                Some(b' ' | b'\n' | b'\r' | b'\t') => self.pos += 1,
                Some(b'/') if matches!(self.bytes.get(self.pos + 1), Some(b'/' | b'*')) => {
                    self.comment()?;
                }
                _ => return Ok(()),
            }
        }
    }

    /// Consume un comentario que empieza en la posición actual (solo modo tolerante).
    fn comment(&mut self) -> Result<(), ParseError> {
        let start = self.pos;
        if !self.tolerant {
            return Err(self.error(ErrorKind::CommentNotAllowed, start));
        }
        let body = self.bytes.get(start + 2..).unwrap_or_default();
        let (kind, end) = if self.bytes.get(start + 1) == Some(&b'/') {
            // El salto de línea no forma parte del comentario.
            let len = body.iter().position(|&c| c == b'\n').unwrap_or(body.len());
            (FixKind::LineComment, start + 2 + len)
        } else {
            let Some(len) = body.windows(2).position(|w| w == b"*/") else {
                return Err(self.error(ErrorKind::UnterminatedComment, start));
            };
            (FixKind::BlockComment, start + 2 + len + 2)
        };
        self.fixes.push(Fix {
            kind,
            span: Span::new(start, end),
        });
        self.pos = end;
        Ok(())
    }

    /// Lee `"clave" :` y deja la posición al inicio del valor.
    fn member_key(&mut self) -> Result<(Span, u8), ParseError> {
        if self.peek() != Some(b'"') {
            return Err(self.unexpected("una clave entre comillas"));
        }
        let (key, escaped) = self.string()?;
        self.skip_ws()?;
        if self.peek() != Some(b':') {
            return Err(self.unexpected("':'"));
        }
        self.pos += 1;
        self.skip_ws()?;
        let key_flags = flags::HAS_KEY | if escaped { flags::KEY_ESCAPED } else { 0 };
        Ok((key, key_flags))
    }

    /// Lee un valor. Los contenedores quedan abiertos en la pila.
    fn value(&mut self, key: Span, key_flags: u8) -> Result<(), ParseError> {
        let start = self.pos;
        match self.peek() {
            Some(c @ (b'{' | b'[')) => {
                let object = c == b'{';
                let kind = if object { Kind::Object } else { Kind::Array };
                let id = self.push_node(kind, key_flags, key, Span::new(start, start))?;
                self.pos += 1;
                self.stack.push(Frame {
                    id,
                    last_child: NodeId::NONE,
                    child_count: 0,
                    object,
                    has_value: false,
                });
            }
            Some(b'"') => {
                let (span, escaped) = self.string()?;
                let value_flags = if escaped { flags::VALUE_ESCAPED } else { 0 };
                self.push_node(Kind::String, key_flags | value_flags, key, span)?;
            }
            Some(b't') => {
                self.literal("true")?;
                let span = Span::new(start, self.pos);
                self.push_node(Kind::Bool, key_flags | flags::TRUE, key, span)?;
            }
            Some(b'f') => {
                self.literal("false")?;
                self.push_node(Kind::Bool, key_flags, key, Span::new(start, self.pos))?;
            }
            Some(b'n') => {
                self.literal("null")?;
                self.push_node(Kind::Null, key_flags, key, Span::new(start, self.pos))?;
            }
            Some(b'-' | b'0'..=b'9') => {
                let span = self.number()?;
                self.push_node(Kind::Number, key_flags, key, span)?;
            }
            _ => return Err(self.unexpected("un valor")),
        }
        Ok(())
    }

    /// Añade un nodo al arena y lo enlaza como último hijo del contenedor abierto.
    fn push_node(
        &mut self,
        kind: Kind,
        flags: u8,
        key: Span,
        value: Span,
    ) -> Result<NodeId, ParseError> {
        let index = self.nodes.len();
        if index >= NodeId::NONE.index() {
            return Err(self.error(ErrorKind::TooLarge, self.pos));
        }
        let id = NodeId(index as u32);
        let parent = match self.stack.last_mut() {
            Some(frame) => {
                let link = if frame.last_child.is_none() {
                    self.nodes
                        .get_mut(frame.id.index())
                        .map(|n| &mut n.first_child)
                } else {
                    self.nodes
                        .get_mut(frame.last_child.index())
                        .map(|n| &mut n.next_sibling)
                };
                if let Some(link) = link {
                    *link = id;
                }
                frame.last_child = id;
                frame.child_count += 1;
                frame.id
            }
            None => NodeId::NONE,
        };
        self.nodes.push(Node {
            kind,
            flags,
            parent,
            first_child: NodeId::NONE,
            next_sibling: NodeId::NONE,
            child_count: 0,
            key,
            value,
        });
        Ok(id)
    }

    /// Consume `}` o `]` y cierra el contenedor de la cima de la pila.
    fn close_container(&mut self) {
        self.pos += 1;
        if let Some(frame) = self.stack.pop()
            && let Some(node) = self.nodes.get_mut(frame.id.index())
        {
            node.value.end = self.pos as u32;
            node.child_count = frame.child_count;
        }
    }

    fn literal(&mut self, word: &'static str) -> Result<(), ParseError> {
        let end = self.pos + word.len();
        if self.bytes.get(self.pos..end) != Some(word.as_bytes()) {
            return Err(self.error(ErrorKind::InvalidLiteral { expected: word }, self.pos));
        }
        self.pos = end;
        Ok(())
    }

    /// Lee un string empezando en `"`. Devuelve el rango sin comillas y si tiene escapes.
    fn string(&mut self) -> Result<(Span, bool), ParseError> {
        let open = self.pos;
        let start = open + 1;
        let mut i = start;
        let mut escaped = false;
        loop {
            match self.bytes.get(i) {
                Some(b'"') => break,
                Some(b'\\') => {
                    escaped = true;
                    i = self.escape(i)?;
                }
                Some(0x00..=0x1F) => return Err(self.error(ErrorKind::ControlCharacter, i)),
                Some(_) => i += 1,
                None => return Err(self.error(ErrorKind::UnterminatedString, open)),
            }
        }
        self.pos = i + 1;
        Ok((Span::new(start, i), escaped))
    }

    /// Valida la secuencia de escape que empieza en `i` y devuelve la posición siguiente.
    fn escape(&self, i: usize) -> Result<usize, ParseError> {
        match self.bytes.get(i + 1) {
            Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => Ok(i + 2),
            Some(b'u') => {
                let unit = self
                    .hex4(i + 2)
                    .ok_or_else(|| self.error(ErrorKind::InvalidUnicodeEscape, i))?;
                match unit {
                    0xD800..=0xDBFF => {
                        let paired = self.bytes.get(i + 6..i + 8) == Some(b"\\u")
                            && matches!(self.hex4(i + 8), Some(0xDC00..=0xDFFF));
                        if paired {
                            Ok(i + 12)
                        } else {
                            Err(self.error(ErrorKind::LoneSurrogate, i))
                        }
                    }
                    0xDC00..=0xDFFF => Err(self.error(ErrorKind::LoneSurrogate, i)),
                    _ => Ok(i + 6),
                }
            }
            Some(_) => Err(self.error(ErrorKind::InvalidEscape, i)),
            None => Err(self.error(ErrorKind::UnterminatedString, i)),
        }
    }

    fn hex4(&self, at: usize) -> Option<u32> {
        let digits = self.bytes.get(at..at + 4)?;
        digits
            .iter()
            .try_fold(0, |acc, &c| Some(acc * 16 + char::from(c).to_digit(16)?))
    }

    /// Lee un número según la gramática de RFC 8259.
    fn number(&mut self) -> Result<Span, ParseError> {
        let start = self.pos;
        let mut i = start;
        if self.bytes.get(i) == Some(&b'-') {
            i += 1;
        }
        match self.bytes.get(i) {
            Some(b'0') => {
                i += 1;
                if matches!(self.bytes.get(i), Some(b'0'..=b'9')) {
                    return Err(self.error(ErrorKind::LeadingZero, start));
                }
            }
            Some(b'1'..=b'9') => i = self.digits(i + 1),
            _ => return Err(self.error(ErrorKind::InvalidNumber, start)),
        }
        if self.bytes.get(i) == Some(&b'.') {
            let first = i + 1;
            i = self.digits(first);
            if i == first {
                return Err(self.error(ErrorKind::InvalidNumber, start));
            }
        }
        if matches!(self.bytes.get(i), Some(b'e' | b'E')) {
            i += 1;
            if matches!(self.bytes.get(i), Some(b'+' | b'-')) {
                i += 1;
            }
            let first = i;
            i = self.digits(first);
            if i == first {
                return Err(self.error(ErrorKind::InvalidNumber, start));
            }
        }
        self.pos = i;
        Ok(Span::new(start, i))
    }

    fn digits(&self, mut i: usize) -> usize {
        while matches!(self.bytes.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strict() -> ParseOptions {
        ParseOptions::default()
    }

    fn tolerant() -> ParseOptions {
        ParseOptions {
            mode: Mode::Tolerant,
        }
    }

    fn doc(text: &str) -> Document {
        match parse(text, &strict()) {
            Ok(parsed) => parsed.document,
            Err(e) => panic!("{text:?} debería parsear: {e}"),
        }
    }

    fn err(text: &str) -> ParseError {
        match parse(text, &strict()) {
            Ok(_) => panic!("{text:?} debería fallar"),
            Err(e) => e,
        }
    }

    #[test]
    fn scalars() {
        let d = doc(" 42 ");
        assert_eq!(d.number(d.root()), Some("42"));
        let d = doc(r#""a\nb""#);
        assert_eq!(d.string(d.root()).as_deref(), Some("a\nb"));
        let d = doc("true");
        assert_eq!(d.bool(d.root()), Some(true));
        let d = doc("false");
        assert_eq!(d.bool(d.root()), Some(false));
        let d = doc("null");
        assert_eq!(d.kind(d.root()), Some(Kind::Null));
    }

    #[test]
    fn nested_structure() {
        let d = doc(r#"{"a": [1, {"b": null}], "c": "x"}"#);
        let root = d.root();
        assert_eq!(d.node_count(), 6);
        assert_eq!(d.node(root).map(Node::child_count), Some(2));
        let members: Vec<_> = d.children(root).collect();
        assert_eq!(d.key(members[0]).as_deref(), Some("a"));
        assert_eq!(d.kind(members[0]), Some(Kind::Array));
        assert_eq!(d.raw(members[0]), r#"[1, {"b": null}]"#);
        let items: Vec<_> = d.children(members[0]).collect();
        assert_eq!(items.len(), 2);
        assert_eq!(d.key(items[0]), None);
        assert_eq!(d.raw(items[1]), r#"{"b": null}"#);
        assert_eq!(d.key(members[1]).as_deref(), Some("c"));
        assert_eq!(d.raw(root), r#"{"a": [1, {"b": null}], "c": "x"}"#);
    }

    #[test]
    fn empty_containers() {
        let d = doc("[ ]");
        assert_eq!(d.children(d.root()).count(), 0);
        let d = doc("{}");
        assert_eq!(d.node(d.root()).map(Node::child_count), Some(0));
    }

    #[test]
    fn numbers() {
        for ok in ["0", "-0", "12", "-3.25", "1e10", "1E+2", "2.5e-3"] {
            let d = doc(ok);
            assert_eq!(d.number(d.root()), Some(ok));
        }
        assert_eq!(err("01").kind, ErrorKind::LeadingZero);
        for bad in ["-", "1.", ".5", "1e", "1e+", "+1"] {
            let kind = err(bad).kind;
            assert!(
                matches!(
                    kind,
                    ErrorKind::InvalidNumber | ErrorKind::UnexpectedChar { .. }
                ),
                "{bad}: {kind:?}"
            );
        }
    }

    #[test]
    fn strict_rejects_comments_and_trailing_commas() {
        assert_eq!(err("[1,]").kind, ErrorKind::TrailingComma);
        assert_eq!(err("[1,]").offset, 2);
        assert_eq!(err(r#"{"a":1,}"#).kind, ErrorKind::TrailingComma);
        assert_eq!(err("// x\n1").kind, ErrorKind::CommentNotAllowed);
        assert_eq!(err("[1 /* x */]").kind, ErrorKind::CommentNotAllowed);
    }

    #[test]
    fn structural_errors() {
        assert_eq!(
            err("").kind,
            ErrorKind::UnexpectedEnd {
                expected: "un valor"
            }
        );
        assert_eq!(
            err("[1 2]").kind,
            ErrorKind::UnexpectedChar {
                found: '2',
                expected: "',' o ']'"
            }
        );
        assert_eq!(
            err("{1:2}").kind,
            ErrorKind::UnexpectedChar {
                found: '1',
                expected: "una clave entre comillas"
            }
        );
        assert_eq!(
            err(r#"{"a" 1}"#).kind,
            ErrorKind::UnexpectedChar {
                found: '1',
                expected: "':'"
            }
        );
        assert_eq!(
            err("[1").kind,
            ErrorKind::UnexpectedEnd {
                expected: "',' o ']'"
            }
        );
        assert_eq!(err("1 2").kind, ErrorKind::TrailingContent);
        assert_eq!(
            err("tru").kind,
            ErrorKind::InvalidLiteral { expected: "true" }
        );
        assert_eq!(
            err("[,1]").kind,
            ErrorKind::UnexpectedChar {
                found: ',',
                expected: "un valor"
            }
        );
    }

    #[test]
    fn string_errors() {
        assert_eq!(err(r#""abc"#).kind, ErrorKind::UnterminatedString);
        assert_eq!(err("\"a\nb\"").kind, ErrorKind::ControlCharacter);
        assert_eq!(err(r#""\x""#).kind, ErrorKind::InvalidEscape);
        assert_eq!(err(r#""\u12g4""#).kind, ErrorKind::InvalidUnicodeEscape);
        assert_eq!(err(r#""\ud800""#).kind, ErrorKind::LoneSurrogate);
        assert_eq!(err(r#""\udc00""#).kind, ErrorKind::LoneSurrogate);
    }

    #[test]
    fn invalid_utf8_reports_offset() {
        let e = match parse(b"[\"a\xFF\"]".to_vec(), &strict()) {
            Ok(_) => panic!("debería fallar"),
            Err(e) => e,
        };
        assert_eq!(
            (e.kind, e.offset, e.line, e.column),
            (ErrorKind::InvalidUtf8, 3, 1, 4)
        );
    }

    #[test]
    fn errors_report_line_and_column() {
        let e = err("{\n  \"nombre\": \"Ana\",\n  \"edad\": 3x\n}");
        assert_eq!((e.line, e.column), (3, 12));
        assert_eq!(
            e.to_string(),
            "línea 3, columna 12: carácter inesperado 'x': se esperaba ',' o '}'"
        );
        let e = err("[\n  1,\n  2,\n]");
        assert_eq!((e.kind, e.line, e.column), (ErrorKind::TrailingComma, 3, 4));
        let e = err("{\"a\": [1, 2}");
        assert_eq!((e.line, e.column), (1, 12));
    }

    #[test]
    fn bom_is_skipped() {
        let d = doc("\u{feff}[1]");
        assert_eq!(d.children(d.root()).count(), 1);
    }

    fn lenient(text: &str) -> Parsed {
        match parse(text, &tolerant()) {
            Ok(parsed) => parsed,
            Err(e) => panic!("{text:?} debería parsear en modo tolerante: {e}"),
        }
    }

    #[test]
    fn tolerant_accepts_jsonc() {
        let text = "// cabecera\n{\n  \"a\": [1, 2,], /* nota */\n  \"b\": {\"c\": 3,},\n}\n";
        let parsed = lenient(text);
        let d = &parsed.document;
        let members: Vec<_> = d.children(d.root()).collect();
        assert_eq!(members.len(), 2);
        assert_eq!(d.children(members[0]).count(), 2);
        let kinds: Vec<_> = parsed.fixes.iter().map(|f| f.kind).collect();
        assert_eq!(
            kinds,
            vec![
                FixKind::LineComment,
                FixKind::TrailingComma,
                FixKind::BlockComment,
                FixKind::TrailingComma,
                FixKind::TrailingComma,
            ]
        );
        assert_eq!(d.text(parsed.fixes[0].span), "// cabecera");
        assert_eq!(d.text(parsed.fixes[2].span), "/* nota */");
    }

    #[test]
    fn autocorrect_produces_strict_json() {
        let text = "[1, /* x */ 2, // y\n 3,]";
        let parsed = lenient(text);
        let fixed = parsed.autocorrected();
        assert_eq!(fixed.as_deref(), Some("[1,  2, \n 3]"));
        let reparsed = parse(fixed.unwrap_or_default(), &strict());
        assert!(reparsed.is_ok());
    }

    #[test]
    fn autocorrect_is_none_for_standard_json() {
        assert_eq!(lenient("[1, 2]").autocorrected(), None);
        assert!(lenient("[1, 2]").fixes.is_empty());
    }

    #[test]
    fn tolerant_still_rejects_real_errors() {
        let e = |text: &str| match parse(text, &tolerant()) {
            Ok(_) => panic!("{text:?} debería fallar"),
            Err(e) => e.kind,
        };
        assert_eq!(
            e("[1,,2]"),
            ErrorKind::UnexpectedChar {
                found: ',',
                expected: "un valor"
            }
        );
        assert_eq!(
            e("[,]"),
            ErrorKind::UnexpectedChar {
                found: ',',
                expected: "un valor"
            }
        );
        assert_eq!(e("/* sin cerrar"), ErrorKind::UnterminatedComment);
    }

    #[test]
    fn tolerant_accepts_comment_after_value() {
        let parsed = lenient("[1] // fin");
        assert_eq!(parsed.autocorrected().as_deref(), Some("[1] "));
    }
}
