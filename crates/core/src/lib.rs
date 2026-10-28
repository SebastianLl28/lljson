//! Núcleo de jsonlens: parser, árbol en arena, hash, formateador y diff.
//! Sin dependencias de UI.

mod decode;
mod diff;
mod document;
mod error;
mod format;
mod hash;
mod parser;
mod source;

pub use diff::{Change, ChangeKind, DiffOptions, DiffResult, DiffStatus, diff, pair_children};
pub use document::{Children, Document, Kind, Node, NodeId, Span};
pub use error::{ErrorKind, ParseError};
pub use format::{FormatOptions, Indent, format, format_node, minify};
pub use parser::{Fix, FixKind, Mode, ParseOptions, Parsed, parse};
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
