//! Deterministic generator of test documents (benchmarks and performance tests).
//! Produces an array of [`NODES_PER_RECORD`]-node records and, with the same seed, variants
//! with an exact number of known changes.

use std::fmt::Write;

use crate::format::Indent;

/// Nodes per record: the object, 5 scalars, `tags` (2 elements) and `address` (2 members).
pub const NODES_PER_RECORD: usize = 12;

const TAGS: &[&str] = &[
    "alpha",
    "beta",
    "gamma",
    "ñandú",
    "new\\nline",
    "\\\"double\\\" quotes",
    "emoji 😀",
    "\\u00e9t\\u00e9",
];

const CITIES: &[&str] = &[
    "Lima",
    "Bogotá",
    "Ciudad de México",
    "Montevideo",
    "São Paulo",
];

/// Changes of a variant relative to the base document. Each change hits a different record,
/// spread evenly across the document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// Changes the value of `name` (string to string).
    pub modified: usize,
    /// Changes `active` from boolean to string (`true` to `"true"`).
    pub type_changed: usize,
    /// Removes the record.
    pub removed: usize,
    /// Inserts a new record (with a new `id`) after the chosen one.
    pub added: usize,
}

impl Changes {
    pub fn total(&self) -> usize {
        self.modified + self.type_changed + self.removed + self.added
    }
}

#[derive(Clone, Debug)]
pub struct GenerateOptions {
    /// Approximate node count of the base document; rounded up to whole records.
    pub nodes: usize,
    /// Same seed and `nodes` yield the same base document.
    pub seed: u64,
    pub indent: Indent,
    pub changes: Changes,
}

impl Default for GenerateOptions {
    fn default() -> GenerateOptions {
        GenerateOptions {
            nodes: 1_000_000,
            seed: 1,
            indent: Indent::default(),
            changes: Changes::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GenerateError {
    #[error("requested {requested} changes, but the document only has {records} records")]
    TooManyChanges { requested: usize, records: usize },
}

/// Number of records in the base document for `nodes` nodes (plus the root array).
pub fn record_count(nodes: usize) -> usize {
    nodes.saturating_sub(1).div_ceil(NODES_PER_RECORD)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    None,
    Modify,
    TypeChange,
    Remove,
    Add,
}

/// Generates the document; with empty `changes` it is the base document.
pub fn generate(options: &GenerateOptions) -> Result<String, GenerateError> {
    let records = record_count(options.nodes);
    let edits = plan(records, options.changes, options.seed)?;
    let mut w = Writer::new(options.indent, records * 300);
    w.open('[');
    let mut first = true;
    let mut next_id = records;
    for (i, &edit) in edits.iter().enumerate() {
        if edit != Edit::Remove {
            w.item(first);
            first = false;
            write_record(&mut w, i, options.seed, edit);
        }
        if edit == Edit::Add {
            w.item(false);
            write_record(&mut w, next_id, options.seed, Edit::None);
            next_id += 1;
        }
    }
    w.close(']', first);
    Ok(w.out)
}

/// Assigns changes to evenly spread records, grouped by kind.
fn plan(records: usize, changes: Changes, seed: u64) -> Result<Vec<Edit>, GenerateError> {
    let total = changes.total();
    if total > records {
        return Err(GenerateError::TooManyChanges {
            requested: total,
            records,
        });
    }
    let mut edits = vec![Edit::None; records];
    if total == 0 {
        return Ok(edits);
    }
    let stride = records / total;
    // `stride >= 1` because `total <= records`; the remainder is below `stride`.
    let offset = (seed % stride as u64) as usize;
    let kinds = [
        (Edit::Modify, changes.modified),
        (Edit::TypeChange, changes.type_changed),
        (Edit::Remove, changes.removed),
        (Edit::Add, changes.added),
    ];
    let mut slot = 0;
    for (edit, count) in kinds {
        for _ in 0..count {
            if let Some(e) = edits.get_mut(slot * stride + offset) {
                *e = edit;
            }
            slot += 1;
        }
    }
    Ok(edits)
}

fn write_record(w: &mut Writer, id: usize, seed: u64, edit: Edit) {
    let r = splitmix64(seed.wrapping_add(id as u64));
    let pick = |list: &[&'static str], shift: u32| list[(r >> shift) as usize % list.len()];

    w.open('{');
    w.item(true);
    w.key("id");
    let _ = write!(w.out, "{id}");

    w.item(false);
    w.key("name");
    let suffix = if edit == Edit::Modify {
        "_modified"
    } else {
        ""
    };
    let _ = write!(w.out, "\"user_{id}{suffix}\"");

    w.item(false);
    w.key("email");
    let _ = write!(w.out, "\"u{id}@example.com\"");

    w.item(false);
    w.key("active");
    let active = r & 1 == 0;
    if edit == Edit::TypeChange {
        let _ = write!(w.out, "\"{active}\"");
    } else {
        let _ = write!(w.out, "{active}");
    }

    w.item(false);
    w.key("score");
    let score = (r >> 8) % 1_000_000;
    let _ = write!(w.out, "{}.{:02}", score / 100, score % 100);

    w.item(false);
    w.key("tags");
    w.open('[');
    w.item(true);
    let _ = write!(w.out, "\"{}\"", pick(TAGS, 24));
    w.item(false);
    let _ = write!(w.out, "\"{}\"", pick(TAGS, 32));
    w.close(']', false);

    w.item(false);
    w.key("address");
    w.open('{');
    w.item(true);
    w.key("city");
    let _ = write!(w.out, "\"{}\"", pick(CITIES, 40));
    w.item(false);
    w.key("zip");
    if r & 2 == 0 {
        w.out.push_str("null");
    } else {
        let _ = write!(w.out, "{}", 10_000 + (r >> 48) % 90_000);
    }
    w.close('}', false);

    w.close('}', false);
}

fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Writer with the same layout as the formatter.
struct Writer {
    out: String,
    unit: String,
    compact: bool,
    depth: usize,
}

impl Writer {
    fn new(indent: Indent, capacity: usize) -> Writer {
        let unit = match indent {
            Indent::Spaces(n) => " ".repeat(usize::from(n)),
            Indent::Tab => "\t".to_owned(),
            Indent::None => String::new(),
        };
        Writer {
            out: String::with_capacity(capacity),
            unit,
            compact: indent == Indent::None,
            depth: 0,
        }
    }

    fn open(&mut self, c: char) {
        self.out.push(c);
        self.depth += 1;
    }

    fn item(&mut self, first: bool) {
        if !first {
            self.out.push(',');
        }
        self.newline();
    }

    fn key(&mut self, key: &str) {
        let _ = write!(self.out, "\"{key}\":");
        if !self.compact {
            self.out.push(' ');
        }
    }

    fn close(&mut self, c: char, empty: bool) {
        self.depth -= 1;
        if !empty {
            self.newline();
        }
        self.out.push(c);
    }

    fn newline(&mut self) {
        if !self.compact {
            self.out.push('\n');
            for _ in 0..self.depth {
                self.out.push_str(&self.unit);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DiffOptions, Document, FormatOptions, ParseOptions, Summary, diff, format, parse};

    fn options(nodes: usize, changes: Changes) -> GenerateOptions {
        GenerateOptions {
            nodes,
            changes,
            ..GenerateOptions::default()
        }
    }

    fn doc(text: String) -> Document {
        parse(text, &ParseOptions::default())
            .expect("generated JSON must be valid")
            .document
    }

    fn by_id() -> DiffOptions {
        DiffOptions {
            array_key: Some("id".to_owned()),
            ..DiffOptions::default()
        }
    }

    #[test]
    fn node_count_rounds_up_to_whole_records() {
        let d = doc(generate(&options(1 + 12 * 100, Changes::default())).expect("generate"));
        assert_eq!(d.node_count(), 1 + 12 * 100);
        let d = doc(generate(&options(1 + 12 * 100 + 1, Changes::default())).expect("generate"));
        assert_eq!(d.node_count(), 1 + 12 * 101);
        assert_eq!(record_count(1_000_000), 83_334);
    }

    #[test]
    fn tiny_documents_are_empty_arrays() {
        for nodes in [0, 1] {
            let text = generate(&options(nodes, Changes::default())).expect("generate");
            assert_eq!(text, "[]");
        }
        let text = generate(&options(2, Changes::default())).expect("generate");
        assert_eq!(doc(text).node_count(), 13);
    }

    #[test]
    fn is_deterministic() {
        let a = generate(&options(5_000, Changes::default())).expect("generate");
        let b = generate(&options(5_000, Changes::default())).expect("generate");
        assert_eq!(a, b);
        let other = GenerateOptions {
            seed: 2,
            ..options(5_000, Changes::default())
        };
        assert_ne!(a, generate(&other).expect("generate"));
    }

    #[test]
    fn indent_matches_formatter() {
        for indent in [
            Indent::Spaces(2),
            Indent::Spaces(0),
            Indent::Tab,
            Indent::None,
        ] {
            let opts = GenerateOptions {
                indent,
                ..options(500, Changes::default())
            };
            let text = generate(&opts).expect("generate");
            let d = doc(text.clone());
            let formatted = format(
                &d,
                &FormatOptions {
                    indent,
                    sort_keys: false,
                },
            );
            assert_eq!(text, formatted, "{indent:?}");
        }
    }

    #[test]
    fn contains_escapes_and_unicode() {
        let d = doc(generate(&options(20_000, Changes::default())).expect("generate"));
        let strings: Vec<String> = (0..d.node_count())
            .filter_map(|i| d.string(crate::NodeId::from_index(i)))
            .map(|s| s.into_owned())
            .collect();
        assert!(strings.iter().any(|s| s == "new\nline"));
        assert!(strings.iter().any(|s| s == "\"double\" quotes"));
        assert!(strings.iter().any(|s| s == "été"));
        assert!(strings.iter().any(|s| s == "emoji 😀"));
    }

    #[test]
    fn variant_has_exactly_the_requested_changes() {
        let base = doc(generate(&options(12_001, Changes::default())).expect("generate"));
        let changes = Changes {
            modified: 7,
            type_changed: 3,
            removed: 2,
            added: 4,
        };
        let variant = doc(generate(&options(12_001, changes)).expect("generate"));
        let result = diff(&base, &variant, &by_id());
        assert_eq!(
            result.summary(),
            Summary {
                added: 4,
                removed: 2,
                modified: 10,
                type_changed: 3,
            }
        );
    }

    #[test]
    fn each_kind_alone() {
        let base = doc(generate(&options(6_001, Changes::default())).expect("generate"));
        let cases = [
            (
                Changes {
                    modified: 1,
                    ..Changes::default()
                },
                (0, 0, 1, 0),
            ),
            (
                Changes {
                    type_changed: 1,
                    ..Changes::default()
                },
                (0, 0, 1, 1),
            ),
            (
                Changes {
                    removed: 1,
                    ..Changes::default()
                },
                (0, 1, 0, 0),
            ),
            (
                Changes {
                    added: 1,
                    ..Changes::default()
                },
                (1, 0, 0, 0),
            ),
        ];
        for (changes, (added, removed, modified, type_changed)) in cases {
            let variant = doc(generate(&options(6_001, changes)).expect("generate"));
            let summary = diff(&base, &variant, &by_id()).summary();
            let expected = Summary {
                added,
                removed,
                modified,
                type_changed,
            };
            assert_eq!(summary, expected, "{changes:?}");
        }
    }

    #[test]
    fn every_record_can_change() {
        let records = 50;
        let nodes = 1 + 12 * records;
        let changes = Changes {
            modified: records,
            ..Changes::default()
        };
        let base = doc(generate(&options(nodes, Changes::default())).expect("generate"));
        let variant = doc(generate(&options(nodes, changes)).expect("generate"));
        assert_eq!(diff(&base, &variant, &by_id()).summary().modified, records);
    }

    #[test]
    fn too_many_changes_is_an_error() {
        let changes = Changes {
            removed: 101,
            ..Changes::default()
        };
        assert_eq!(
            generate(&options(1 + 12 * 100, changes)),
            Err(GenerateError::TooManyChanges {
                requested: 101,
                records: 100,
            })
        );
    }
}
