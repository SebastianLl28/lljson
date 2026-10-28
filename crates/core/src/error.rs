//! Errores de parseo.

/// Motivo de un error de parseo. Los mensajes están pensados para mostrarse al usuario.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ErrorKind {
    #[error("el texto no es UTF-8 válido")]
    InvalidUtf8,
    #[error("el documento es demasiado grande (máximo 4 GiB)")]
    TooLarge,
    #[error("fin inesperado del texto: se esperaba {expected}")]
    UnexpectedEnd { expected: &'static str },
    #[error("carácter inesperado {found:?}: se esperaba {expected}")]
    UnexpectedChar { found: char, expected: &'static str },
    #[error("literal inválido: se esperaba `{expected}`")]
    InvalidLiteral { expected: &'static str },
    #[error("número inválido")]
    InvalidNumber,
    #[error("número inválido: no se permiten ceros a la izquierda")]
    LeadingZero,
    #[error("string sin cerrar")]
    UnterminatedString,
    #[error("carácter de control sin escapar dentro de un string")]
    ControlCharacter,
    #[error("secuencia de escape inválida")]
    InvalidEscape,
    #[error("secuencia \\u inválida: se esperaban 4 dígitos hexadecimales")]
    InvalidUnicodeEscape,
    #[error("secuencia \\u inválida: surrogate UTF-16 sin pareja")]
    LoneSurrogate,
    #[error("coma final no permitida en modo estricto")]
    TrailingComma,
    #[error("comentario no permitido en modo estricto")]
    CommentNotAllowed,
    #[error("comentario sin cerrar")]
    UnterminatedComment,
    #[error("contenido extra después del valor JSON")]
    TrailingContent,
}

/// Error de parseo con su posición en el texto original.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("línea {line}, columna {column}: {kind}")]
pub struct ParseError {
    pub kind: ErrorKind,
    /// Posición en bytes desde el inicio del texto.
    pub offset: usize,
    /// Línea, empezando en 1.
    pub line: u32,
    /// Columna en caracteres (no bytes), empezando en 1.
    pub column: u32,
}

impl ParseError {
    /// Crea el error calculando línea y columna. Solo se recorre el texto en caso de error.
    pub(crate) fn new(kind: ErrorKind, offset: usize, bytes: &[u8]) -> ParseError {
        let (line, column) = line_column(bytes, offset);
        ParseError {
            kind,
            offset,
            line,
            column,
        }
    }
}

/// Línea y columna (1-based) de un desplazamiento en bytes. La columna cuenta
/// caracteres UTF-8: se ignoran los bytes de continuación (`10xxxxxx`).
fn line_column(bytes: &[u8], offset: usize) -> (u32, u32) {
    let before = bytes.get(..offset).unwrap_or(bytes);
    let line_start = before
        .iter()
        .rposition(|&c| c == b'\n')
        .map_or(0, |i| i + 1);
    let line = before.iter().filter(|&&c| c == b'\n').count() + 1;
    let column = before[line_start..]
        .iter()
        .filter(|&&c| c & 0xC0 != 0x80)
        .count()
        + 1;
    (saturate(line), saturate(column))
}

fn saturate(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_character() {
        assert_eq!(line_column(b"abc", 0), (1, 1));
    }

    #[test]
    fn counts_lines_and_columns() {
        let text = b"{\n  \"a\": 1,\n  x\n}";
        let offset = text.iter().position(|&c| c == b'x').unwrap_or_default();
        assert_eq!(line_column(text, offset), (3, 3));
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        let text = "[\"ñandú\", ?]";
        let offset = text.find('?').unwrap_or_default();
        assert_eq!(line_column(text.as_bytes(), offset), (1, 11));
    }

    #[test]
    fn crlf_and_end_of_text() {
        let text = b"[1,\r\n2";
        assert_eq!(line_column(text, text.len()), (2, 2));
    }

    #[test]
    fn message_includes_position() {
        let e = ParseError::new(ErrorKind::TrailingContent, 2, b"1\n2");
        assert_eq!(
            e.to_string(),
            "línea 2, columna 1: contenido extra después del valor JSON"
        );
    }
}
