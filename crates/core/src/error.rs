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
#[error("byte {offset}: {kind}")]
pub struct ParseError {
    pub kind: ErrorKind,
    /// Posición en bytes desde el inicio del texto.
    pub offset: usize,
}

impl ParseError {
    pub(crate) fn new(kind: ErrorKind, offset: usize) -> ParseError {
        ParseError { kind, offset }
    }
}
