//! lljson core: parser, arena tree, hash, formatter, diff, view rows, search and test generator.
//! No UI dependencies.

mod decode;
mod diff;
mod document;
mod error;
mod format;
mod generate;
mod hash;
mod parser;
mod path;
mod rows;
mod search;
mod source;
mod tree;

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
pub use rows::{
    Cell, CellText, ChangeStop, Direction, MAX_VALUE_CHARS, Reveal, Row, RowKind, RowSide, Rows,
    Splice, TokenKind, cell_text,
};
pub use search::{MatchMode, Query, Scope, SearchError};
pub use source::Source;
pub use tree::{TreeLabel, TreeRow, TreeRows, TreeText, tree_text};

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
