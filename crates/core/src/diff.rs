//! Motor de diff semántico entre dos documentos.
//! Los subárboles con el mismo hash se consideran iguales y no se recorren.
//! Es iterativo (pila explícita): la profundidad no está limitada por la pila del hilo.

use std::hash::Hash;

use ahash::AHashMap;

use crate::document::{Document, Kind, Node, NodeId};

/// Estado de un nodo tras el diff.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DiffStatus {
    /// Igual en ambos lados (o dentro de un subárbol igual).
    #[default]
    Equal,
    /// Solo existe en el documento derecho (el subárbol completo).
    Added,
    /// Solo existe en el documento izquierdo (el subárbol completo).
    Removed,
    /// Escalar con distinto valor.
    Modified,
    /// Contenedor con alguna diferencia entre sus descendientes.
    Contains,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
}

/// Una diferencia concreta, con el nodo implicado en cada documento.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    pub left: Option<NodeId>,
    pub right: Option<NodeId>,
}

#[derive(Clone, Debug, Default)]
pub struct DiffOptions {
    /// Clave para emparejar los elementos de los arrays (p. ej. `"id"`). Se aplica a los
    /// arrays cuyos elementos, en ambos lados, son todos objetos con esa clave; el resto
    /// se compara por índice.
    pub array_key: Option<String>,
}

/// Resultado del diff: estado de cada nodo, emparejamientos y lista de cambios.
#[derive(Debug)]
pub struct DiffResult {
    left_status: Vec<DiffStatus>,
    right_status: Vec<DiffStatus>,
    left_match: Vec<NodeId>,
    right_match: Vec<NodeId>,
    changes: Vec<Change>,
}

impl DiffResult {
    fn new(left_len: usize, right_len: usize) -> DiffResult {
        DiffResult {
            left_status: vec![DiffStatus::Equal; left_len],
            right_status: vec![DiffStatus::Equal; right_len],
            left_match: vec![NodeId::NONE; left_len],
            right_match: vec![NodeId::NONE; right_len],
            changes: Vec::new(),
        }
    }

    pub fn left_status(&self, id: NodeId) -> DiffStatus {
        self.left_status
            .get(id.index())
            .copied()
            .unwrap_or_default()
    }

    pub fn right_status(&self, id: NodeId) -> DiffStatus {
        self.right_status
            .get(id.index())
            .copied()
            .unwrap_or_default()
    }

    /// Nodo derecho emparejado con un nodo izquierdo. Solo se registra para los nodos que
    /// el diff recorrió; dentro de subárboles iguales se usa [`pair_children`] bajo demanda.
    pub fn left_match(&self, id: NodeId) -> Option<NodeId> {
        some(self.left_match.get(id.index()).copied())
    }

    /// Nodo izquierdo emparejado con un nodo derecho (ver [`DiffResult::left_match`]).
    pub fn right_match(&self, id: NodeId) -> Option<NodeId> {
        some(self.right_match.get(id.index()).copied())
    }

    /// Diferencias en orden de documento.
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    pub fn is_equal(&self) -> bool {
        self.changes.is_empty()
    }
}

fn some(id: Option<NodeId>) -> Option<NodeId> {
    id.filter(|id| !id.is_none())
}

/// Compara dos documentos.
pub fn diff(left: &Document, right: &Document, options: &DiffOptions) -> DiffResult {
    let mut differ = Differ {
        left,
        right,
        options,
        result: DiffResult::new(left.node_count(), right.node_count()),
        stack: Vec::new(),
    };
    differ.run();
    differ.result
}

/// Empareja los hijos de dos contenedores del mismo tipo: los objetos por clave
/// (las claves repetidas, por orden de aparición) y los arrays por índice o por
/// [`DiffOptions::array_key`].
/// El orden de salida es el del contenedor izquierdo; cada hijo que solo existe en el
/// derecho se coloca detrás del par de su predecesor en el derecho (y detrás de los
/// hijos eliminados que sigan a ese par).
pub fn pair_children<'a>(
    left: &'a Document,
    l: NodeId,
    right: &'a Document,
    r: NodeId,
    options: &DiffOptions,
) -> Vec<(Option<NodeId>, Option<NodeId>)> {
    let left_children: Vec<NodeId> = left.children(l).collect();
    let right_children: Vec<NodeId> = right.children(r).collect();
    let matches = match (left.kind(l), right.kind(r)) {
        (Some(Kind::Object), Some(Kind::Object)) => match_by(
            left_children
                .iter()
                .map(|&c| left.key(c).unwrap_or_default()),
            right_children
                .iter()
                .map(|&c| right.key(c).unwrap_or_default()),
        ),
        (Some(Kind::Array), Some(Kind::Array)) => {
            let keyed = options.array_key.as_deref().and_then(|key| {
                let left_keys = element_keys(left, &left_children, key)?;
                let right_keys = element_keys(right, &right_children, key)?;
                Some(match_by(left_keys.into_iter(), right_keys.into_iter()))
            });
            keyed.unwrap_or_else(|| {
                (0..left_children.len())
                    .map(|i| (i < right_children.len()).then_some(i))
                    .collect()
            })
        }
        _ => vec![None; left_children.len()],
    };
    merge(&left_children, &right_children, &matches)
}

/// Hash del valor de `key` en cada elemento, o `None` si algún elemento no es un
/// objeto con esa clave.
fn element_keys(doc: &Document, elements: &[NodeId], key: &str) -> Option<Vec<u64>> {
    elements
        .iter()
        .map(|&element| {
            if doc.kind(element)? != Kind::Object {
                return None;
            }
            let member = doc
                .children(element)
                .find(|&c| doc.key(c).as_deref() == Some(key))?;
            doc.hash(member)
        })
        .collect()
}

/// Para cada elemento izquierdo, el índice del derecho con la misma clave.
/// Las claves repetidas se emparejan por orden de aparición.
fn match_by<K: Hash + Eq>(
    left: impl Iterator<Item = K>,
    right: impl Iterator<Item = K>,
) -> Vec<Option<usize>> {
    const END: usize = usize::MAX;
    let right: Vec<K> = right.collect();
    // Lista enlazada por clave (cabeza en el mapa, siguientes en `next`) sin una
    // asignación por clave.
    let mut head: AHashMap<K, usize> = AHashMap::with_capacity(right.len());
    let mut next = vec![END; right.len()];
    for (j, key) in right.into_iter().enumerate().rev() {
        if let Some(slot) = next.get_mut(j) {
            *slot = head.insert(key, j).unwrap_or(END);
        }
    }
    left.map(|key| {
        let slot = head.get_mut(&key)?;
        let j = *slot;
        if j == END {
            return None;
        }
        *slot = next.get(j).copied().unwrap_or(END);
        Some(j)
    })
    .collect()
}

/// Combina los hijos emparejados en una secuencia alineada.
fn merge(
    left: &[NodeId],
    right: &[NodeId],
    matches: &[Option<usize>],
) -> Vec<(Option<NodeId>, Option<NodeId>)> {
    let mut matched_from = vec![None; right.len()];
    for (i, m) in matches.iter().enumerate() {
        if let Some(slot) = m.and_then(|j| matched_from.get_mut(j)) {
            *slot = Some(i);
        }
    }
    // (posición en el izquierdo delante de la que va, índice en el derecho)
    let mut inserts: Vec<(usize, usize)> = Vec::new();
    let mut anchor = 0;
    for (j, from) in matched_from.iter().enumerate() {
        match from {
            Some(i) => anchor = i + 1,
            None => inserts.push((anchor, j)),
        }
    }
    inserts.sort_by_key(|&(position, _)| position);

    let mut out = Vec::with_capacity(left.len() + inserts.len());
    let mut pending = inserts.into_iter().peekable();
    for i in 0..=left.len() {
        // Los eliminados consecutivos van antes que los añadidos en esa posición.
        let unmatched_left = matches.get(i).is_some_and(Option::is_none);
        while let Some((_, j)) = pending.next_if(|&(position, _)| position <= i && !unmatched_left)
        {
            out.push((None, right.get(j).copied()));
        }
        if let Some(&l) = left.get(i) {
            let r = matches
                .get(i)
                .copied()
                .flatten()
                .and_then(|j| right.get(j).copied());
            out.push((Some(l), r));
        }
    }
    out
}

/// Par pendiente de comparar (o un hijo que solo existe en un lado).
struct Work {
    left: Option<NodeId>,
    right: Option<NodeId>,
}

struct Differ<'a> {
    left: &'a Document,
    right: &'a Document,
    options: &'a DiffOptions,
    result: DiffResult,
    stack: Vec<Work>,
}

impl Differ<'_> {
    fn run(&mut self) {
        self.stack.push(Work {
            left: Some(self.left.root()),
            right: Some(self.right.root()),
        });
        while let Some(work) = self.stack.pop() {
            match (work.left, work.right) {
                (Some(l), Some(r)) => self.compare(l, r),
                (Some(l), None) => self.removed(l),
                (None, Some(r)) => self.added(r),
                (None, None) => {}
            }
        }
    }

    fn compare(&mut self, l: NodeId, r: NodeId) {
        if let Some(slot) = self.result.left_match.get_mut(l.index()) {
            *slot = r;
        }
        if let Some(slot) = self.result.right_match.get_mut(r.index()) {
            *slot = l;
        }
        let (Some(ln), Some(rn)) = (self.left.node(l), self.right.node(r)) else {
            return;
        };
        if ln.hash() == rn.hash() {
            return;
        }
        if ln.kind().is_container() && ln.kind() == rn.kind() {
            let pairs = pair_children(self.left, l, self.right, r, self.options);
            let works = pairs
                .into_iter()
                .rev()
                .map(|(left, right)| Work { left, right });
            self.stack.extend(works);
            return;
        }
        set(&mut self.result.left_status, l, DiffStatus::Modified);
        set(&mut self.result.right_status, r, DiffStatus::Modified);
        self.record(ChangeKind::Modified, Some(l), Some(r));
        self.mark_contains(ln.parent(), rn.parent());
    }

    fn removed(&mut self, l: NodeId) {
        fill_subtree(
            &mut self.result.left_status,
            self.left,
            l,
            DiffStatus::Removed,
        );
        self.record(ChangeKind::Removed, Some(l), None);
        let parent = self.left.node(l).and_then(Node::parent);
        let right_parent = parent.and_then(|p| self.result.left_match(p));
        self.mark_contains(parent, right_parent);
    }

    fn added(&mut self, r: NodeId) {
        fill_subtree(
            &mut self.result.right_status,
            self.right,
            r,
            DiffStatus::Added,
        );
        self.record(ChangeKind::Added, None, Some(r));
        let parent = self.right.node(r).and_then(Node::parent);
        let left_parent = parent.and_then(|p| self.result.right_match(p));
        self.mark_contains(left_parent, parent);
    }

    fn record(&mut self, kind: ChangeKind, left: Option<NodeId>, right: Option<NodeId>) {
        self.result.changes.push(Change { kind, left, right });
    }

    /// Marca como `Contains` los contenedores desde `left`/`right` hasta la raíz.
    fn mark_contains(&mut self, left: Option<NodeId>, right: Option<NodeId>) {
        mark_ancestors(&mut self.result.left_status, self.left, left);
        mark_ancestors(&mut self.result.right_status, self.right, right);
    }
}

fn set(status: &mut [DiffStatus], id: NodeId, value: DiffStatus) {
    if let Some(slot) = status.get_mut(id.index()) {
        *slot = value;
    }
}

fn fill_subtree(status: &mut [DiffStatus], doc: &Document, id: NodeId, value: DiffStatus) {
    if let Some(range) = status.get_mut(id.index()..doc.subtree_end(id)) {
        range.fill(value);
    }
}

/// Sube desde `current` marcando `Contains`; se detiene en el primero ya marcado.
fn mark_ancestors(status: &mut [DiffStatus], doc: &Document, mut current: Option<NodeId>) {
    while let Some(id) = current {
        let Some(slot) = status.get_mut(id.index()) else {
            return;
        };
        if *slot == DiffStatus::Contains {
            return;
        }
        *slot = DiffStatus::Contains;
        current = doc.node(id).and_then(Node::parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{ParseOptions, parse};

    fn doc(text: &str) -> Document {
        match parse(text, &ParseOptions::default()) {
            Ok(parsed) => parsed.document,
            Err(e) => panic!("{text:?} debería parsear: {e}"),
        }
    }

    fn run(a: &str, b: &str) -> (Document, Document, DiffResult) {
        let (left, right) = (doc(a), doc(b));
        let result = diff(&left, &right, &DiffOptions::default());
        (left, right, result)
    }

    /// Cambios como (tipo, texto izquierdo, texto derecho).
    fn changes(a: &str, b: &str) -> Vec<(ChangeKind, String, String)> {
        let (left, right, result) = run(a, b);
        result
            .changes()
            .iter()
            .map(|c| {
                let l = c.left.map(|id| left.raw(id).to_owned()).unwrap_or_default();
                let r = c
                    .right
                    .map(|id| right.raw(id).to_owned())
                    .unwrap_or_default();
                (c.kind, l, r)
            })
            .collect()
    }

    fn change(kind: ChangeKind, l: &str, r: &str) -> (ChangeKind, String, String) {
        (kind, l.to_owned(), r.to_owned())
    }

    #[test]
    fn equal_documents() {
        let (left, right, result) = run(r#"{"a": [1, 2], "b": 1.0}"#, r#"{"b": 1, "a": [1, 2]}"#);
        assert!(result.is_equal());
        assert_eq!(result.left_status(left.root()), DiffStatus::Equal);
        assert_eq!(result.left_match(left.root()), Some(right.root()));
        // Subárbol igual: no se recorre.
        let a = left.children(left.root()).next().unwrap_or(left.root());
        assert_eq!(result.left_match(a), None);
    }

    #[test]
    fn modified_added_and_removed_members() {
        let found = changes(
            r#"{"a": 1, "b": 2, "c": 3}"#,
            r#"{"a": 1, "b": 20, "d": 4}"#,
        );
        assert_eq!(
            found,
            vec![
                change(ChangeKind::Modified, "2", "20"),
                change(ChangeKind::Removed, "3", ""),
                change(ChangeKind::Added, "", "4"),
            ]
        );
    }

    #[test]
    fn arrays_by_index() {
        let found = changes("[1, 2, 3]", "[1, 5]");
        assert_eq!(
            found,
            vec![
                change(ChangeKind::Modified, "2", "5"),
                change(ChangeKind::Removed, "3", ""),
            ]
        );
        assert_eq!(
            changes("[1]", "[1, [2]]"),
            vec![change(ChangeKind::Added, "", "[2]")]
        );
    }

    fn keyed() -> DiffOptions {
        DiffOptions {
            array_key: Some("id".to_owned()),
        }
    }

    fn keyed_changes(a: &str, b: &str) -> Vec<(ChangeKind, String, String)> {
        let (left, right) = (doc(a), doc(b));
        let result = diff(&left, &right, &keyed());
        result
            .changes()
            .iter()
            .map(|c| {
                let l = c.left.map(|id| left.raw(id).to_owned()).unwrap_or_default();
                let r = c
                    .right
                    .map(|id| right.raw(id).to_owned())
                    .unwrap_or_default();
                (c.kind, l, r)
            })
            .collect()
    }

    #[test]
    fn arrays_by_key_survive_insertions_and_reordering() {
        let a = r#"[{"id": 1, "v": "a"}, {"id": 2, "v": "b"}, {"id": 3, "v": "c"}]"#;
        let b = r#"[{"id": 0, "v": "z"}, {"id": 3, "v": "c"}, {"id": 1, "v": "A"}, {"id": 2, "v": "b"}]"#;
        assert_eq!(
            keyed_changes(a, b),
            vec![
                change(ChangeKind::Added, "", r#"{"id": 0, "v": "z"}"#),
                change(ChangeKind::Modified, "a", "A"),
            ]
        );
        // Por índice, la inserción al principio desplaza todos los elementos.
        assert_eq!(changes(a, b).len(), 7);
    }

    #[test]
    fn arrays_by_key_match_key_values_semantically() {
        let a = r#"[{"id": 1, "v": 1}, {"id": "1", "v": 2}]"#;
        let b = r#"[{"id": "1", "v": 2}, {"id": 1.0, "v": 3}]"#;
        assert_eq!(
            keyed_changes(a, b),
            vec![change(ChangeKind::Modified, "1", "3")]
        );
    }

    #[test]
    fn arrays_by_key_fall_back_to_index() {
        // Un elemento sin la clave (o que no es objeto) hace que el array use índices.
        let a = r#"[{"id": 1}, {"id": 2}]"#;
        let b = r#"[{"id": 2}, {"x": 1}]"#;
        // 1→2 modificado; "id" eliminado y "x" añadido en el segundo elemento.
        assert_eq!(keyed_changes(a, b).len(), 3);
        assert_eq!(keyed_changes("[1, 2]", "[2, 1]").len(), 2);
    }

    #[test]
    fn arrays_by_key_handle_removals_and_duplicates() {
        let a = r#"[{"id": 1}, {"id": 1, "n": 2}, {"id": 2}]"#;
        let b = r#"[{"id": 1}, {"id": 1, "n": 3}]"#;
        assert_eq!(
            keyed_changes(a, b),
            vec![
                change(ChangeKind::Modified, "2", "3"),
                change(ChangeKind::Removed, r#"{"id": 2}"#, ""),
            ]
        );
    }

    #[test]
    fn statuses_and_ancestors() {
        let (left, right, result) = run(
            r#"{"x": {"y": [1, {"z": true}]}, "k": {"old": [1, 2]}}"#,
            r#"{"x": {"y": [1, {"z": false}]}, "k": {}}"#,
        );
        let status: Vec<_> = (0..left.node_count())
            .map(|i| result.left_status(NodeId::from_index(i)))
            .collect();
        use DiffStatus::*;
        // root, x, y, 1, {z}, z, k, old, 1, 2
        assert_eq!(
            status,
            vec![
                Contains, Contains, Contains, Equal, Contains, Modified, Contains, Removed,
                Removed, Removed
            ]
        );
        assert_eq!(result.right_status(right.root()), Contains);
        assert_eq!(result.changes().len(), 2);
    }

    #[test]
    fn added_subtree_is_fully_marked() {
        let (_, right, result) = run("[]", r#"[{"a": [1, 2]}]"#);
        let statuses: Vec<_> = (1..right.node_count())
            .map(|i| result.right_status(NodeId::from_index(i)))
            .collect();
        assert!(statuses.iter().all(|&s| s == DiffStatus::Added));
        assert_eq!(result.right_status(right.root()), DiffStatus::Contains);
        assert_eq!(result.changes().len(), 1);
    }

    #[test]
    fn duplicate_keys_pair_in_order() {
        let found = changes(r#"{"a": 1, "a": 2}"#, r#"{"a": 1, "a": 3, "a": 4}"#);
        assert_eq!(
            found,
            vec![
                change(ChangeKind::Modified, "2", "3"),
                change(ChangeKind::Added, "", "4"),
            ]
        );
    }

    #[test]
    fn pairing_order_places_right_only_after_predecessor() {
        let (left, right) = (
            doc(r#"{"a": 1, "c": 3}"#),
            doc(r#"{"z": 0, "a": 1, "b": 2, "c": 3, "d": 4}"#),
        );
        let pairs = pair_children(
            &left,
            left.root(),
            &right,
            right.root(),
            &DiffOptions::default(),
        );
        let keys: Vec<_> = pairs
            .iter()
            .map(|(l, r)| {
                let side = |d: &Document, id: Option<NodeId>| {
                    id.and_then(|i| d.key(i)).unwrap_or_default().into_owned()
                };
                (side(&left, *l), side(&right, *r))
            })
            .collect();
        let expected = [("", "z"), ("a", "a"), ("", "b"), ("c", "c"), ("", "d")];
        let expected: Vec<_> = expected
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        assert_eq!(keys, expected);
    }

    #[test]
    fn deep_nesting_does_not_overflow_the_stack() {
        const DEPTH: usize = 100_000;
        let a = format!("{}1{}", r#"{"a":["#.repeat(DEPTH), "]}".repeat(DEPTH));
        let b = format!("{}2{}", r#"{"a":["#.repeat(DEPTH), "]}".repeat(DEPTH));
        let (_, _, result) = run(&a, &b);
        assert_eq!(result.changes().len(), 1);
    }
}
