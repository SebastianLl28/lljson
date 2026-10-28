//! Parser JSON que construye el arena directamente, sin árbol intermedio.
//! Es iterativo (pila explícita): la profundidad de anidamiento no está limitada
//! por la pila del hilo.

use crate::document::{Document, Kind, Node, NodeId, Span, flags};
use crate::error::{ErrorKind, ParseError};
use crate::source::Source;

/// Los rangos se guardan como `u32`.
const MAX_SOURCE_LEN: usize = u32::MAX as usize;
const BOM: &[u8] = b"\xEF\xBB\xBF";

/// Resultado de un parseo correcto.
#[derive(Debug)]
pub struct Parsed {
    pub document: Document,
}

/// Parsea un documento JSON estricto (RFC 8259).
pub fn parse(source: impl Into<Source>) -> Result<Parsed, ParseError> {
    let source = source.into();
    let (nodes, root) = Parser::new(source.as_bytes())?.run()?;
    Ok(Parsed {
        document: Document::from_parts(source, nodes, root),
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
    nodes: Vec<Node>,
    stack: Vec<Frame>,
}

impl<'a> Parser<'a> {
    fn new(bytes: &'a [u8]) -> Result<Parser<'a>, ParseError> {
        if bytes.len() > MAX_SOURCE_LEN {
            return Err(ParseError::new(ErrorKind::TooLarge, 0));
        }
        if let Err(e) = std::str::from_utf8(bytes) {
            return Err(ParseError::new(ErrorKind::InvalidUtf8, e.valid_up_to()));
        }
        Ok(Parser {
            bytes,
            pos: if bytes.starts_with(BOM) { BOM.len() } else { 0 },
            // Estimación barata para evitar la mayoría de realocaciones.
            nodes: Vec::with_capacity(bytes.len() / 16),
            stack: Vec::new(),
        })
    }

    fn run(mut self) -> Result<(Vec<Node>, NodeId), ParseError> {
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
                            return Err(self.error(ErrorKind::TrailingComma, comma));
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
        Ok((self.nodes, NodeId(0)))
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn error(&self, kind: ErrorKind, offset: usize) -> ParseError {
        ParseError::new(kind, offset)
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
                    return Err(self.error(ErrorKind::CommentNotAllowed, self.pos));
                }
                _ => return Ok(()),
            }
        }
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

    fn doc(text: &str) -> Document {
        match parse(text) {
            Ok(parsed) => parsed.document,
            Err(e) => panic!("{text:?} debería parsear: {e}"),
        }
    }

    fn err(text: &str) -> ParseError {
        match parse(text) {
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
        let e = match parse(b"[\"a\xFF\"]".to_vec()) {
            Ok(_) => panic!("debería fallar"),
            Err(e) => e,
        };
        assert_eq!(e, ParseError::new(ErrorKind::InvalidUtf8, 3));
    }

    #[test]
    fn bom_is_skipped() {
        let d = doc("\u{feff}[1]");
        assert_eq!(d.children(d.root()).count(), 1);
    }
}
