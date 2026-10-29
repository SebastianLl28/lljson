//! Search over keys and values of a [`Document`]. Matching is parallel over chunks of the
//! arena, so it stays fast with millions of nodes.

use std::thread;

use regex::{Regex, RegexBuilder};
use thiserror::Error;

use crate::document::{Document, Kind, NodeId};

/// Below this many nodes the cost of spawning threads outweighs the gain.
const PARALLEL_MIN_NODES: usize = 50_000;
const MAX_THREADS: usize = 8;

/// What the query is compared against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// Keys and scalar values.
    #[default]
    Both,
    Keys,
    /// Scalar values only (containers have no value text).
    Values,
}

/// How the query text is interpreted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MatchMode {
    /// Substring that ignores case and accents (`cafe` finds `Café`).
    #[default]
    Normal,
    /// Substring that respects case and accents.
    Exact,
    /// Regular expression, case-sensitive unless it says otherwise (`(?i)`).
    Regex,
}

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("invalid regular expression: {0}")]
    InvalidRegex(String),
}

enum Matcher {
    /// Query already folded (lowercase, no accents).
    Folded(String),
    Exact(String),
    Regex(Regex),
}

/// A compiled search. Cheap to share between threads.
pub struct Query {
    matcher: Option<Matcher>,
    scope: Scope,
}

impl Query {
    /// An empty `text` matches nothing. Fails only for an invalid regular expression.
    pub fn new(text: &str, mode: MatchMode, scope: Scope) -> Result<Query, SearchError> {
        let matcher = if text.is_empty() {
            None
        } else {
            Some(match mode {
                MatchMode::Normal => Matcher::Folded(fold(text)),
                MatchMode::Exact => Matcher::Exact(text.to_owned()),
                MatchMode::Regex => Matcher::Regex(
                    RegexBuilder::new(text)
                        .build()
                        .map_err(|e| SearchError::InvalidRegex(e.to_string()))?,
                ),
            })
        };
        Ok(Query { matcher, scope })
    }

    /// Matching nodes in document order. A node matches through its key or, if it is a
    /// scalar, its value text (strings decoded, numbers and literals as written).
    pub fn find(&self, doc: &Document) -> Vec<NodeId> {
        if self.matcher.is_none() {
            return Vec::new();
        }
        let total = doc.nodes().len();
        let threads = thread::available_parallelism().map_or(1, |n| n.get().min(MAX_THREADS));
        if total < PARALLEL_MIN_NODES || threads < 2 {
            return self.find_in(doc, 0..total);
        }
        let chunk = total.div_ceil(threads);
        thread::scope(|scope| {
            let mut parts = Vec::with_capacity(threads);
            for start in (0..total).step_by(chunk) {
                let range = start..(start + chunk).min(total);
                let spawned = thread::Builder::new()
                    .name("lljson-search".to_owned())
                    .spawn_scoped(scope, {
                        let range = range.clone();
                        move || self.find_in(doc, range)
                    });
                // If a thread cannot be created, this chunk is searched right here.
                parts.push(spawned.map_err(|_| range));
            }
            parts
                .into_iter()
                .flat_map(|part| match part {
                    Ok(handle) => handle.join().unwrap_or_default(),
                    Err(range) => self.find_in(doc, range),
                })
                .collect()
        })
    }

    fn find_in(&self, doc: &Document, range: std::ops::Range<usize>) -> Vec<NodeId> {
        range
            .map(NodeId::from_index)
            .filter(|&id| self.node_matches(doc, id))
            .collect()
    }

    fn node_matches(&self, doc: &Document, id: NodeId) -> bool {
        if self.scope != Scope::Values
            && let Some(key) = doc.key(id)
            && self.text_matches(&key)
        {
            return true;
        }
        if self.scope == Scope::Keys {
            return false;
        }
        match doc.kind(id) {
            Some(Kind::String) => doc.string(id).is_some_and(|s| self.text_matches(&s)),
            Some(Kind::Number) => doc.number(id).is_some_and(|n| self.text_matches(n)),
            Some(Kind::Bool) => doc
                .bool(id)
                .is_some_and(|b| self.text_matches(if b { "true" } else { "false" })),
            Some(Kind::Null) => self.text_matches("null"),
            _ => false,
        }
    }

    fn text_matches(&self, text: &str) -> bool {
        match &self.matcher {
            Some(Matcher::Folded(needle)) => contains_folded(text, needle),
            Some(Matcher::Exact(needle)) => text.contains(needle.as_str()),
            Some(Matcher::Regex(regex)) => regex.is_match(text),
            None => false,
        }
    }
}

/// Lowercases and strips accents (Latin letters, plus combining marks).
fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        for lower in c.to_lowercase() {
            match fold_char(lower) {
                Some(Some(base)) => out.push(base),
                Some(None) => {}
                None => out.push(lower),
            }
        }
    }
    out
}

/// `Some(Some(base))`: accented letter; `Some(None)`: combining mark to drop; `None`: keep.
fn fold_char(c: char) -> Option<Option<char>> {
    let base = match c {
        'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' | 'ŧ' => 't',
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        '\u{300}'..='\u{36f}' => return Some(None),
        _ => return None,
    };
    Some(Some(base))
}

/// `needle` must already be folded.
fn contains_folded(haystack: &str, needle: &str) -> bool {
    if haystack.is_ascii() {
        ascii_contains(haystack.as_bytes(), needle.as_bytes())
    } else {
        fold(haystack).contains(needle)
    }
}

/// Case-insensitive search of a lowercase ASCII `needle`.
fn ascii_contains(haystack: &[u8], needle: &[u8]) -> bool {
    let Some(&first) = needle.first() else {
        return true;
    };
    if haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|window| {
        window[0].to_ascii_lowercase() == first
            && window
                .iter()
                .zip(needle)
                .all(|(a, b)| a.to_ascii_lowercase() == *b)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{ParseOptions, parse};

    fn doc(text: &str) -> Document {
        parse(text, &ParseOptions::default())
            .expect("valid JSON")
            .document
    }

    /// Raw text of each match, to keep assertions readable.
    fn hits(doc: &Document, text: &str, mode: MatchMode, scope: Scope) -> Vec<String> {
        Query::new(text, mode, scope)
            .expect("valid query")
            .find(doc)
            .into_iter()
            .map(|id| match doc.key(id) {
                Some(key) => format!("{key}={}", doc.raw(id)),
                None => doc.raw(id).to_owned(),
            })
            .collect()
    }

    #[test]
    fn finds_keys_and_values_by_default() {
        let d = doc(r#"{"name": "Ana", "city": "Name City", "n": 5}"#);
        assert_eq!(
            hits(&d, "name", MatchMode::Normal, Scope::Both),
            ["name=Ana", "city=Name City"]
        );
    }

    #[test]
    fn scope_limits_where_it_looks() {
        let d = doc(r#"{"name": "Ana", "city": "Name City"}"#);
        assert_eq!(
            hits(&d, "name", MatchMode::Normal, Scope::Keys),
            ["name=Ana"]
        );
        assert_eq!(
            hits(&d, "name", MatchMode::Normal, Scope::Values),
            ["city=Name City"]
        );
    }

    #[test]
    fn normal_ignores_case_and_accents_both_ways() {
        let d = doc(r#"{"café": "Árbol", "CAFE": "ñandú"}"#);
        assert_eq!(hits(&d, "CAFÉ", MatchMode::Normal, Scope::Keys).len(), 2);
        assert_eq!(hits(&d, "arbol", MatchMode::Normal, Scope::Values).len(), 1);
        assert_eq!(hits(&d, "NANDU", MatchMode::Normal, Scope::Values).len(), 1);
    }

    #[test]
    fn normal_handles_decomposed_accents() {
        // "e" followed by U+0301 (combining acute).
        let d = doc("{\"k\": \"cafe\u{301}\"}");
        assert_eq!(hits(&d, "cafe", MatchMode::Normal, Scope::Values).len(), 1);
    }

    #[test]
    fn exact_respects_case_and_accents() {
        let d = doc(r#"{"a": "Café", "b": "cafe"}"#);
        assert_eq!(
            hits(&d, "Café", MatchMode::Exact, Scope::Values),
            ["a=Café"]
        );
        assert!(hits(&d, "CAFE", MatchMode::Exact, Scope::Values).is_empty());
        assert_eq!(hits(&d, "cafe", MatchMode::Exact, Scope::Values).len(), 1);
    }

    #[test]
    fn regex_matches_and_is_case_sensitive_by_default() {
        let d = doc(r#"{"id": "AB-123", "x": "ab-9"}"#);
        assert_eq!(
            hits(&d, r"^[A-Z]{2}-\d+$", MatchMode::Regex, Scope::Values).len(),
            1
        );
        assert_eq!(
            hits(&d, r"(?i)^ab-\d+$", MatchMode::Regex, Scope::Values).len(),
            2
        );
    }

    #[test]
    fn invalid_regex_is_an_error() {
        assert!(matches!(
            Query::new("(", MatchMode::Regex, Scope::Both),
            Err(SearchError::InvalidRegex(_))
        ));
    }

    #[test]
    fn empty_query_matches_nothing() {
        let d = doc(r#"{"a": 1}"#);
        assert!(hits(&d, "", MatchMode::Normal, Scope::Both).is_empty());
        assert!(hits(&d, "", MatchMode::Regex, Scope::Both).is_empty());
    }

    #[test]
    fn searches_numbers_literals_and_decoded_escapes() {
        let d = doc(r#"{"n": 12.5, "t": true, "f": false, "z": null, "e": "café \"q\""}"#);
        assert_eq!(
            hits(&d, "12.5", MatchMode::Normal, Scope::Values),
            ["n=12.5"]
        );
        assert_eq!(
            hits(&d, "true", MatchMode::Normal, Scope::Values),
            ["t=true"]
        );
        assert_eq!(
            hits(&d, "null", MatchMode::Normal, Scope::Values),
            ["z=null"]
        );
        assert_eq!(
            hits(&d, "café \"q", MatchMode::Exact, Scope::Values).len(),
            1
        );
    }

    #[test]
    fn containers_match_by_key_only() {
        let d = doc(r#"{"users": [{"id": 1}], "other": {"users": 2}}"#);
        let found = hits(&d, "users", MatchMode::Normal, Scope::Both);
        assert_eq!(found.len(), 2);
        assert!(hits(&d, "users", MatchMode::Normal, Scope::Values).is_empty());
    }

    #[test]
    fn escaped_keys_are_decoded() {
        let d = doc(r#"{"café": 1}"#);
        assert_eq!(hits(&d, "café", MatchMode::Exact, Scope::Keys).len(), 1);
    }

    #[test]
    fn results_are_in_document_order() {
        let d = doc(r#"[{"a": "x"}, {"a": "x"}, "x"]"#);
        let ids = Query::new("x", MatchMode::Normal, Scope::Values)
            .expect("valid")
            .find(&d);
        assert_eq!(ids.len(), 3);
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn parallel_search_matches_sequential() {
        let items: Vec<String> = (0..30_000)
            .map(|i| format!(r#"{{"id": {i}, "name": "Item Ñandú {i}"}}"#))
            .collect();
        let d = doc(&format!("[{}]", items.join(",")));
        assert!(d.nodes().len() >= PARALLEL_MIN_NODES);
        let query = Query::new("nandu 2999", MatchMode::Normal, Scope::Both).expect("valid");
        let parallel = query.find(&d);
        let sequential = query.find_in(&d, 0..d.nodes().len());
        assert_eq!(parallel, sequential);
        assert!(!parallel.is_empty());
    }

    #[test]
    fn folding_table() {
        assert_eq!(fold("ÁÉÍÓÚ ÜÑ çÇ"), "aeiou un cc");
        assert_eq!(fold("Hello, World 123"), "hello, world 123");
    }
}
