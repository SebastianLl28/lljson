//! Núcleo de jsonlens: parser, árbol en arena, hash, formateador, diff y generador de pruebas.
//! Sin dependencias de UI.

mod decode;
mod diff;
mod document;
mod error;
mod format;
mod generate;
mod hash;
mod parser;
mod path;
mod source;

pub use diff::{
    Change, ChangeKind, DiffOptions, DiffResult, DiffStatus, Summary, diff, pair_children,
};
pub use document::{Children, Document, Kind, Node, NodeId, Span};
pub use error::{ErrorKind, ParseError};
pub use format::{FormatOptions, Indent, format, format_node, minify};
pub use generate::{
    Changes, GenerateError, GenerateOptions, NODES_PER_RECORD, generate, record_count,
};
pub use parser::{Fix, FixKind, Mode, ParseOptions, Parsed, parse};
pub use path::{IgnoreRule, PatternError, PatternSegment};
pub use source::Source;

/// Versión de la librería.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_package() {
        assert_eq!(version(), "0.1.0");
    }
}
