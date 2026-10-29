//! Lista plana de filas visibles para la vista lado a lado.
//!
//! Cada fila alinea un nodo del documento izquierdo con uno del derecho, o deja un hueco
//! de relleno en el lado que no lo tiene. Los hijos se emparejan con [`pair_children`]
//! (como en el diff), así que los valores emparejados quedan en la misma fila en ambos
//! paneles. La vista virtualizada solo consulta las filas visibles.
//!
//! Expandir o colapsar modifica únicamente esta lista (un `splice`), nunca el árbol.
//! La construcción es iterativa: la profundidad no está limitada por la pila del hilo.

use std::borrow::Cow;

use crate::diff::{DiffOptions, DiffResult, DiffStatus, pair_children};
use crate::document::{Document, Kind, Node, NodeId};

/// Caracteres máximos del valor de un string en una fila.
pub const MAX_VALUE_CHARS: usize = 1000;

/// Qué muestra una fila para un nodo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RowKind {
    /// Escalar, contenedor vacío o contenedor colapsado: una sola fila.
    Leaf,
    /// Apertura de un contenedor expandido (`"clave": {`).
    Open,
    /// Cierre de un contenedor expandido (`}`).
    Close,
}

/// Lo que muestra un lado de una fila.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub node: NodeId,
    pub kind: RowKind,
}

/// Una fila alineada. Ocupa 12 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    left: NodeId,
    right: NodeId,
    left_kind: RowKind,
    right_kind: RowKind,
    depth: u16,
}

impl Row {
    fn new(left: NodeId, right: NodeId, kind: RowKind, depth: u16) -> Row {
        let side_kind = |id: NodeId| if id.is_none() { RowKind::Leaf } else { kind };
        Row {
            left,
            right,
            left_kind: side_kind(left),
            right_kind: side_kind(right),
            depth,
        }
    }

    /// Contenido del lado izquierdo, o `None` si es una fila de relleno en ese lado.
    pub fn left(&self) -> Option<Cell> {
        (!self.left.is_none()).then_some(Cell {
            node: self.left,
            kind: self.left_kind,
        })
    }

    /// Contenido del lado derecho, o `None` si es una fila de relleno en ese lado.
    pub fn right(&self) -> Option<Cell> {
        (!self.right.is_none()).then_some(Cell {
            node: self.right,
            kind: self.right_kind,
        })
    }

    /// Profundidad de anidamiento (0 para la raíz). Se satura en `u16::MAX`.
    pub fn depth(&self) -> u16 {
        self.depth
    }

    fn is_open(&self) -> bool {
        self.left_kind == RowKind::Open || self.right_kind == RowKind::Open
    }
}

/// Sentido de la búsqueda de diferencias.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// Fila de una diferencia encontrada por [`Rows::find_change`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangeStop {
    pub position: usize,
    /// La búsqueda dio la vuelta al principio o al final de la lista.
    pub wrapped: bool,
}

/// El estado marca el subárbol completo (sus descendientes tienen el mismo estado).
fn fills_subtree(status: DiffStatus) -> bool {
    matches!(
        status,
        DiffStatus::Added | DiffStatus::Removed | DiffStatus::TypeChanged
    )
}

/// Cambio en la lista tras expandir o colapsar: en `position` se quitaron `removed` filas
/// y se pusieron `added` (lo que necesita `GListModel::items_changed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Splice {
    pub position: usize,
    pub removed: usize,
    pub added: usize,
}

/// Filas visibles de uno o dos documentos.
///
/// Los métodos que reciben los documentos deben recibir siempre los mismos con los que
/// se construyó la lista.
#[derive(Debug, Default)]
pub struct Rows {
    rows: Vec<Row>,
    collapsed_left: Vec<bool>,
    collapsed_right: Vec<bool>,
    options: DiffOptions,
}

enum Task {
    Visit(NodeId, NodeId, u16),
    Close(NodeId, NodeId, u16),
}

impl Rows {
    /// Construye la lista con todo expandido. `options` decide cómo se emparejan los
    /// elementos de los arrays (por índice o por clave).
    pub fn new(left: Option<&Document>, right: Option<&Document>, options: DiffOptions) -> Rows {
        let mut rows = Rows {
            rows: Vec::new(),
            collapsed_left: vec![false; left.map_or(0, Document::node_count)],
            collapsed_right: vec![false; right.map_or(0, Document::node_count)],
            options,
        };
        let l = left.map_or(NodeId::NONE, Document::root);
        let r = right.map_or(NodeId::NONE, Document::root);
        if !l.is_none() || !r.is_none() {
            rows.rows = rows.emit(left, right, l, r, 0);
        }
        rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<Row> {
        self.rows.get(index).copied()
    }

    /// `Some(true)` si la fila abre un contenedor expandido, `Some(false)` si muestra un
    /// contenedor no vacío colapsado y `None` si no se puede expandir ni colapsar.
    pub fn is_expanded(
        &self,
        index: usize,
        left: Option<&Document>,
        right: Option<&Document>,
    ) -> Option<bool> {
        let row = self.rows.get(index)?;
        if row.is_open() {
            return Some(true);
        }
        let collapsed = |cell: Option<Cell>, doc: Option<&Document>| {
            cell.is_some_and(|c| c.kind == RowKind::Leaf && is_non_empty_container(doc, c.node))
        };
        (collapsed(row.left(), left) || collapsed(row.right(), right)).then_some(false)
    }

    /// Expande o colapsa el contenedor de la fila `index`. Las filas de cierre y las que
    /// no muestran un contenedor no vacío no cambian (`None`).
    pub fn toggle(
        &mut self,
        index: usize,
        left: Option<&Document>,
        right: Option<&Document>,
    ) -> Option<Splice> {
        match self.is_expanded(index, left, right)? {
            true => self.collapse(index),
            false => Some(self.expand(index, left, right)),
        }
    }

    fn collapse(&mut self, index: usize) -> Option<Splice> {
        let row = *self.rows.get(index)?;
        let close = index
            + 1
            + self
                .rows
                .get(index + 1..)?
                .iter()
                .position(|r| r.depth <= row.depth)?;
        if let Some(c) = row.left().filter(|c| c.kind == RowKind::Open) {
            set(&mut self.collapsed_left, c.node, true);
        }
        if let Some(c) = row.right().filter(|c| c.kind == RowKind::Open) {
            set(&mut self.collapsed_right, c.node, true);
        }
        let leaf = Row::new(row.left, row.right, RowKind::Leaf, row.depth);
        self.rows.splice(index..=close, [leaf]);
        Some(Splice {
            position: index,
            removed: close - index + 1,
            added: 1,
        })
    }

    fn expand(
        &mut self,
        index: usize,
        left: Option<&Document>,
        right: Option<&Document>,
    ) -> Splice {
        let row = self.rows[index];
        set(&mut self.collapsed_left, row.left, false);
        set(&mut self.collapsed_right, row.right, false);
        let new_rows = self.emit(left, right, row.left, row.right, row.depth);
        let added = new_rows.len();
        self.rows.splice(index..=index, new_rows);
        Splice {
            position: index,
            removed: 1,
            added,
        }
    }

    /// Busca la diferencia más cercana a `from` (`None`: desde el principio al avanzar o
    /// desde el final al retroceder) y da la vuelta si no hay más en esa dirección.
    ///
    /// Un subárbol añadido, eliminado o con cambio de tipo cuenta como una sola parada
    /// (su primera fila), y un contenedor colapsado con cambios dentro también.
    /// Devuelve `None` si no hay ninguna diferencia visible. Recorre la lista en tiempo
    /// lineal; `diff` debe ser el resultado de comparar `left` con `right`.
    pub fn find_change(
        &self,
        from: Option<usize>,
        direction: Direction,
        left: Option<&Document>,
        right: Option<&Document>,
        diff: &DiffResult,
    ) -> Option<ChangeStop> {
        let len = self.rows.len();
        if len == 0 {
            return None;
        }
        // Posición virtual de partida y paso; fuera de `0..len` significa que dio la vuelta.
        let (start, step): (i64, i64) = match (direction, from) {
            (Direction::Forward, Some(p)) => (p as i64 + 1, 1),
            (Direction::Forward, None) => (0, 1),
            (Direction::Backward, Some(p)) => (p as i64 - 1, -1),
            (Direction::Backward, None) => (len as i64 - 1, -1),
        };
        (0..len as i64).find_map(|k| {
            let virtual_index = start + k * step;
            let position = virtual_index.rem_euclid(len as i64) as usize;
            let row = self.rows.get(position)?;
            let is_change = is_change_start(row.left(), left, |n| diff.left_status(n))
                || is_change_start(row.right(), right, |n| diff.right_status(n));
            is_change.then_some(ChangeStop {
                position,
                wrapped: !(0..len as i64).contains(&virtual_index),
            })
        })
    }

    /// Filas del par `(l, r)` y sus descendientes según el estado de colapso actual.
    fn emit(
        &self,
        left: Option<&Document>,
        right: Option<&Document>,
        l: NodeId,
        r: NodeId,
        depth: u16,
    ) -> Vec<Row> {
        let mut out = Vec::new();
        let mut stack = vec![Task::Visit(l, r, depth)];
        while let Some(task) = stack.pop() {
            let (l, r, depth) = match task {
                Task::Close(l, r, depth) => {
                    out.push(Row::new(l, r, RowKind::Close, depth));
                    continue;
                }
                Task::Visit(l, r, depth) => (l, r, depth),
            };
            let lk = open_kind(left, l, &self.collapsed_left);
            let rk = open_kind(right, r, &self.collapsed_right);
            if !l.is_none() && !r.is_none() && lk != rk && (lk.is_some() || rk.is_some()) {
                // No se pueden alinear línea a línea (p. ej. objeto frente a array):
                // primero el subárbol izquierdo y después el derecho.
                stack.push(Task::Visit(NodeId::NONE, r, depth));
                stack.push(Task::Visit(l, NodeId::NONE, depth));
                continue;
            }
            if lk.or(rk).is_none() {
                out.push(Row::new(l, r, RowKind::Leaf, depth));
                continue;
            }
            out.push(Row::new(l, r, RowKind::Open, depth));
            stack.push(Task::Close(l, r, depth));
            let pairs = match (
                left.filter(|_| !l.is_none()),
                right.filter(|_| !r.is_none()),
            ) {
                (Some(ld), Some(rd)) => pair_children(ld, l, rd, r, &self.options),
                (Some(ld), None) => ld.children(l).map(|c| (Some(c), None)).collect(),
                (None, Some(rd)) => rd.children(r).map(|c| (None, Some(c))).collect(),
                (None, None) => Vec::new(),
            };
            let child_depth = depth.saturating_add(1);
            for (a, b) in pairs.into_iter().rev() {
                stack.push(Task::Visit(
                    a.unwrap_or(NodeId::NONE),
                    b.unwrap_or(NodeId::NONE),
                    child_depth,
                ));
            }
        }
        out
    }
}

/// La celda es el comienzo de una diferencia: un cambio que no está dentro de otro
/// subárbol ya marcado, o un contenedor colapsado con cambios dentro.
fn is_change_start(
    cell: Option<Cell>,
    doc: Option<&Document>,
    status: impl Fn(NodeId) -> DiffStatus,
) -> bool {
    let Some(cell) = cell.filter(|c| c.kind != RowKind::Close) else {
        return false;
    };
    match status(cell.node) {
        DiffStatus::Contains => cell.kind == RowKind::Leaf,
        DiffStatus::Added
        | DiffStatus::Removed
        | DiffStatus::Modified
        | DiffStatus::TypeChanged => !doc
            .and_then(|d| d.node(cell.node))
            .and_then(Node::parent)
            .is_some_and(|parent| fills_subtree(status(parent))),
        DiffStatus::Equal | DiffStatus::Ignored => false,
    }
}

/// Tipo del contenedor si se muestra expandido (no vacío y no colapsado).
fn open_kind(doc: Option<&Document>, id: NodeId, collapsed: &[bool]) -> Option<Kind> {
    if !is_non_empty_container(doc, id) || collapsed.get(id.index()).copied().unwrap_or(false) {
        return None;
    }
    doc?.kind(id)
}

fn is_non_empty_container(doc: Option<&Document>, id: NodeId) -> bool {
    doc.and_then(|d| d.node(id))
        .is_some_and(|n| n.kind().is_container() && n.child_count() > 0)
}

fn set(flags: &mut [bool], id: NodeId, value: bool) {
    if let Some(flag) = flags.get_mut(id.index()) {
        *flag = value;
    }
}

/// Categoría de un fragmento de texto, para los colores de sintaxis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Punctuation,
    String,
    Number,
    Bool,
    Null,
}

/// Texto de un lado de una fila: `clave: valor,  resumen`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellText<'a> {
    /// Clave entre comillas, tal como está en el original (con sus escapes).
    pub key: Option<String>,
    pub value: Cow<'a, str>,
    pub value_kind: TokenKind,
    /// Lleva coma detrás (tiene un hermano siguiente).
    pub comma: bool,
    /// Resumen de un contenedor colapsado (`3 claves`, `1 elemento`).
    pub summary: Option<String>,
}

/// Genera el texto de una celda. Los strings largos se recortan a [`MAX_VALUE_CHARS`].
pub fn cell_text(doc: &Document, cell: Cell) -> CellText<'_> {
    let Some(node) = doc.node(cell.node) else {
        return CellText {
            key: None,
            value: Cow::Borrowed(""),
            value_kind: TokenKind::Punctuation,
            comma: false,
            summary: None,
        };
    };
    let key = match cell.kind {
        RowKind::Close => None,
        _ => node
            .key_span()
            .map(|span| format!("\"{}\"", doc.text(span))),
    };
    let object = node.kind() == Kind::Object;
    let mut summary = None;
    let (value, value_kind) = match (cell.kind, node.kind()) {
        (RowKind::Open, _) => (
            Cow::Borrowed(if object { "{" } else { "[" }),
            TokenKind::Punctuation,
        ),
        (RowKind::Close, _) => (
            Cow::Borrowed(if object { "}" } else { "]" }),
            TokenKind::Punctuation,
        ),
        (RowKind::Leaf, Kind::Object | Kind::Array) => {
            let count = node.child_count() as usize;
            let text = match (object, count) {
                (true, 0) => "{}",
                (false, 0) => "[]",
                (true, _) => "{…}",
                (false, _) => "[…]",
            };
            if count > 0 {
                summary = Some(match (object, count) {
                    (true, 1) => "1 clave".to_owned(),
                    (true, n) => format!("{n} claves"),
                    (false, 1) => "1 elemento".to_owned(),
                    (false, n) => format!("{n} elementos"),
                });
            }
            (Cow::Borrowed(text), TokenKind::Punctuation)
        }
        (RowKind::Leaf, Kind::String) => {
            let raw = doc.raw(cell.node);
            let text = match raw.char_indices().nth(MAX_VALUE_CHARS) {
                Some((cut, _)) => format!("\"{}…", &raw[..cut]),
                None => format!("\"{raw}\""),
            };
            (Cow::Owned(text), TokenKind::String)
        }
        (RowKind::Leaf, Kind::Number) => (Cow::Borrowed(doc.raw(cell.node)), TokenKind::Number),
        (RowKind::Leaf, Kind::Bool) => (Cow::Borrowed(doc.raw(cell.node)), TokenKind::Bool),
        (RowKind::Leaf, Kind::Null) => (Cow::Borrowed("null"), TokenKind::Null),
    };
    CellText {
        key,
        value,
        value_kind,
        comma: cell.kind != RowKind::Open && node.next_sibling().is_some(),
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ParseOptions, parse};

    fn doc(text: &str) -> Document {
        parse(text, &ParseOptions::default())
            .expect("JSON válido")
            .document
    }

    /// Texto de cada fila: `izquierda | derecha` (`·` para el relleno).
    fn render(rows: &Rows, left: Option<&Document>, right: Option<&Document>) -> Vec<String> {
        let side = |cell: Option<Cell>, doc: Option<&Document>| match (cell, doc) {
            (Some(cell), Some(doc)) => {
                let t = cell_text(doc, cell);
                let key = t.key.map(|k| format!("{k}: ")).unwrap_or_default();
                let comma = if t.comma { "," } else { "" };
                format!("{key}{}{comma}", t.value)
            }
            _ => "·".to_owned(),
        };
        (0..rows.len())
            .filter_map(|i| rows.get(i))
            .map(|row| {
                let indent = "  ".repeat(usize::from(row.depth()));
                format!(
                    "{indent}{} | {}",
                    side(row.left(), left),
                    side(row.right(), right)
                )
            })
            .collect()
    }

    #[test]
    fn row_is_twelve_bytes() {
        assert_eq!(size_of::<Row>(), 12);
    }

    #[test]
    fn no_documents() {
        let rows = Rows::new(None, None, DiffOptions::default());
        assert!(rows.is_empty());
        assert_eq!(rows.get(0), None);
    }

    #[test]
    fn single_document() {
        let d = doc(r#"{"a":1,"b":[true,null],"c":{}}"#);
        let rows = Rows::new(Some(&d), None, DiffOptions::default());
        assert_eq!(
            render(&rows, Some(&d), None),
            [
                "{ | ·",
                "  \"a\": 1, | ·",
                "  \"b\": [ | ·",
                "    true, | ·",
                "    null | ·",
                "  ], | ·",
                "  \"c\": {} | ·",
                "} | ·",
            ]
        );
    }

    #[test]
    fn right_only() {
        let d = doc("[1]");
        let rows = Rows::new(None, Some(&d), DiffOptions::default());
        assert_eq!(render(&rows, None, Some(&d)), ["· | [", "  · | 1", "· | ]"]);
    }

    #[test]
    fn aligned_rows_with_filler() {
        let l = doc(r#"{"a":1,"b":2,"c":3}"#);
        let r = doc(r#"{"c":3,"a":10,"d":4}"#);
        let rows = Rows::new(Some(&l), Some(&r), DiffOptions::default());
        assert_eq!(
            render(&rows, Some(&l), Some(&r)),
            [
                "{ | {",
                "  \"a\": 1, | \"a\": 10,",
                "  \"b\": 2, | ·",
                // `d` va detrás de su predecesor en el derecho (`a`) y de los eliminados.
                "  · | \"d\": 4",
                "  \"c\": 3 | \"c\": 3,",
                "} | }",
            ]
        );
    }

    #[test]
    fn arrays_by_key() {
        let l = doc(r#"[{"id":1},{"id":2}]"#);
        let r = doc(r#"[{"id":2},{"id":1}]"#);
        let options = DiffOptions {
            array_key: Some("id".to_owned()),
            ..DiffOptions::default()
        };
        let rows = Rows::new(Some(&l), Some(&r), options);
        let text = render(&rows, Some(&l), Some(&r));
        assert_eq!(text[2], "    \"id\": 1 | \"id\": 1");
        assert_eq!(text[5], "    \"id\": 2 | \"id\": 2");
        assert_eq!(rows.len(), 8);
    }

    #[test]
    fn mismatched_containers_are_stacked() {
        let l = doc(r#"{"a":{"x":1}}"#);
        let r = doc(r#"{"a":[1]}"#);
        let rows = Rows::new(Some(&l), Some(&r), DiffOptions::default());
        assert_eq!(
            render(&rows, Some(&l), Some(&r)),
            [
                "{ | {",
                "  \"a\": { | ·",
                "    \"x\": 1 | ·",
                "  } | ·",
                "  · | \"a\": [",
                "    · | 1",
                "  · | ]",
                "} | }",
            ]
        );
    }

    #[test]
    fn container_against_scalar_is_stacked() {
        let l = doc(r#"[[1]]"#);
        let r = doc(r#"[1]"#);
        let rows = Rows::new(Some(&l), Some(&r), DiffOptions::default());
        assert_eq!(
            render(&rows, Some(&l), Some(&r)),
            [
                "[ | [",
                "  [ | ·",
                "    1 | ·",
                "  ] | ·",
                "  · | 1",
                "] | ]"
            ]
        );
    }

    #[test]
    fn scalars_of_different_type_share_a_row() {
        let l = doc(r#"["1"]"#);
        let r = doc(r#"[1]"#);
        let rows = Rows::new(Some(&l), Some(&r), DiffOptions::default());
        assert_eq!(render(&rows, Some(&l), Some(&r))[1], "  \"1\" | 1");
    }

    #[test]
    fn collapse_and_expand_round_trip() {
        let l = doc(r#"{"a":{"b":[1,2]},"c":3}"#);
        let r = doc(r#"{"a":{"b":[1]},"c":3}"#);
        let (ld, rd) = (Some(&l), Some(&r));
        let mut rows = Rows::new(ld, rd, DiffOptions::default());
        let original = render(&rows, ld, rd);
        assert_eq!(rows.is_expanded(1, ld, rd), Some(true));

        let splice = rows.toggle(1, ld, rd).expect("colapsar");
        assert_eq!(
            splice,
            Splice {
                position: 1,
                removed: original.len() - 3,
                added: 1
            }
        );
        assert_eq!(
            render(&rows, ld, rd),
            [
                "{ | {",
                "  \"a\": {…}, | \"a\": {…},",
                "  \"c\": 3 | \"c\": 3",
                "} | }"
            ]
        );
        assert_eq!(rows.is_expanded(1, ld, rd), Some(false));

        let splice = rows.toggle(1, ld, rd).expect("expandir");
        assert_eq!(splice.added, original.len() - 3);
        assert_eq!(render(&rows, ld, rd), original);
    }

    #[test]
    fn nested_collapse_is_kept_when_parent_expands() {
        let d = doc(r#"{"a":{"b":[1,2]}}"#);
        let mut rows = Rows::new(Some(&d), None, DiffOptions::default());
        rows.toggle(2, Some(&d), None).expect("colapsar b");
        rows.toggle(1, Some(&d), None).expect("colapsar a");
        rows.toggle(1, Some(&d), None).expect("expandir a");
        assert_eq!(
            render(&rows, Some(&d), None),
            [
                "{ | ·",
                "  \"a\": { | ·",
                "    \"b\": […] | ·",
                "  } | ·",
                "} | ·"
            ]
        );
    }

    #[test]
    fn collapse_root() {
        let d = doc("[1,2,3]");
        let mut rows = Rows::new(Some(&d), None, DiffOptions::default());
        let splice = rows.toggle(0, Some(&d), None).expect("colapsar");
        assert_eq!(splice.removed, 5);
        assert_eq!(rows.len(), 1);
        let text = cell_text(&d, rows.get(0).and_then(|r| r.left()).expect("celda"));
        assert_eq!(text.value, "[…]");
        assert_eq!(text.summary.as_deref(), Some("3 elementos"));
    }

    #[test]
    fn untoggleable_rows() {
        let d = doc(r#"{"a":1,"b":[]}"#);
        let mut rows = Rows::new(Some(&d), None, DiffOptions::default());
        for index in [1, 2, 3, 99] {
            assert_eq!(
                rows.is_expanded(index, Some(&d), None),
                None,
                "fila {index}"
            );
            assert_eq!(rows.toggle(index, Some(&d), None), None);
        }
    }

    #[test]
    fn expanding_mismatched_pair_stacks_it() {
        let l = doc(r#"{"a":{"x":1}}"#);
        let r = doc(r#"{"a":5}"#);
        let (ld, rd) = (Some(&l), Some(&r));
        let mut rows = Rows::new(ld, rd, DiffOptions::default());
        rows.toggle(1, ld, rd)
            .expect("colapsar el objeto izquierdo");
        assert_eq!(render(&rows, ld, rd)[1], "  \"a\": {…} | ·");
    }

    #[test]
    fn deep_nesting_does_not_overflow() {
        let depth = 100_000;
        let text = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        let d = doc(&text);
        let rows = Rows::new(Some(&d), None, DiffOptions::default());
        assert_eq!(rows.len(), 2 * depth - 1);
        let max = (0..rows.len())
            .filter_map(|i| rows.get(i))
            .map(|r| r.depth())
            .max();
        assert_eq!(max, Some(u16::MAX));
    }

    #[test]
    fn cell_text_values() {
        let d = doc(r#"{"k\n":"v","n":-1.5e3,"t":true,"z":null,"o":{"x":1}}"#);
        let texts: Vec<CellText> = d
            .children(d.root())
            .map(|id| {
                cell_text(
                    &d,
                    Cell {
                        node: id,
                        kind: RowKind::Leaf,
                    },
                )
            })
            .collect();
        assert_eq!(texts[0].key.as_deref(), Some("\"k\\n\""));
        assert_eq!(texts[0].value, "\"v\"");
        assert_eq!(texts[0].value_kind, TokenKind::String);
        assert!(texts[0].comma);
        assert_eq!(texts[1].value, "-1.5e3");
        assert_eq!(texts[1].value_kind, TokenKind::Number);
        assert_eq!(texts[2].value_kind, TokenKind::Bool);
        assert_eq!(texts[3].value_kind, TokenKind::Null);
        assert_eq!(texts[4].value, "{…}");
        assert_eq!(texts[4].summary.as_deref(), Some("1 clave"));
        assert!(!texts[4].comma);
    }

    #[test]
    fn open_and_close_text() {
        let d = doc(r#"[{"a":1},2]"#);
        let first = d.children(d.root()).next().expect("hijo");
        let open = cell_text(
            &d,
            Cell {
                node: first,
                kind: RowKind::Open,
            },
        );
        assert_eq!((open.value.as_ref(), open.comma), ("{", false));
        let close = cell_text(
            &d,
            Cell {
                node: first,
                kind: RowKind::Close,
            },
        );
        assert_eq!(
            (close.value.as_ref(), close.comma, close.key),
            ("}", true, None)
        );
    }

    #[test]
    fn long_strings_are_truncated() {
        let d = doc(&format!("[\"{}\"]", "é".repeat(5000)));
        let id = d.children(d.root()).next().expect("hijo");
        let text = cell_text(
            &d,
            Cell {
                node: id,
                kind: RowKind::Leaf,
            },
        );
        assert_eq!(text.value.chars().count(), MAX_VALUE_CHARS + 2);
        assert!(text.value.ends_with('…'));
    }

    /// Posiciones de todas las paradas dando la vuelta una vez, y si alguna dio la vuelta.
    fn stops(
        rows: &Rows,
        l: &Document,
        r: &Document,
        direction: Direction,
    ) -> (Vec<usize>, Vec<bool>) {
        let diff = crate::diff(l, r, &DiffOptions::default());
        let mut from = None;
        let (mut positions, mut wraps) = (Vec::new(), Vec::new());
        for _ in 0..4 {
            let Some(stop) = rows.find_change(from, direction, Some(l), Some(r), &diff) else {
                break;
            };
            positions.push(stop.position);
            wraps.push(stop.wrapped);
            from = Some(stop.position);
        }
        (positions, wraps)
    }

    #[test]
    fn find_change_visits_each_change_once_and_wraps() {
        let l = doc(r#"{"a":1,"b":2,"c":3}"#);
        let r = doc(r#"{"a":10,"b":2,"c":30,"d":{"k":[1,2]}}"#);
        let rows = Rows::new(Some(&l), Some(&r), DiffOptions::default());
        // Filas: 0 `{`, 1 a, 2 b, 3 c, 4 `d: {` (subárbol añadido: una sola parada), …
        let (forward, wraps) = stops(&rows, &l, &r, Direction::Forward);
        assert_eq!(forward, [1, 3, 4, 1]);
        assert_eq!(wraps, [false, false, false, true]);
        let (backward, wraps) = stops(&rows, &l, &r, Direction::Backward);
        assert_eq!(backward, [4, 3, 1, 4]);
        assert_eq!(wraps, [false, false, false, true]);
    }

    #[test]
    fn find_change_without_differences_or_rows() {
        let l = doc(r#"{"a":[1,2]}"#);
        let rows = Rows::new(Some(&l), Some(&l), DiffOptions::default());
        let diff = crate::diff(&l, &l, &DiffOptions::default());
        for direction in [Direction::Forward, Direction::Backward] {
            assert_eq!(
                rows.find_change(None, direction, Some(&l), Some(&l), &diff),
                None
            );
        }
        let empty = Rows::default();
        assert_eq!(
            empty.find_change(None, Direction::Forward, None, None, &diff),
            None
        );
    }

    #[test]
    fn find_change_single_change_returns_itself_after_wrapping() {
        let l = doc("[1,2,3]");
        let r = doc("[1,2,4]");
        let rows = Rows::new(Some(&l), Some(&r), DiffOptions::default());
        let diff = crate::diff(&l, &r, &DiffOptions::default());
        let stop = rows.find_change(Some(3), Direction::Forward, Some(&l), Some(&r), &diff);
        assert_eq!(
            stop,
            Some(ChangeStop {
                position: 3,
                wrapped: true
            })
        );
        let stop = rows.find_change(Some(0), Direction::Backward, Some(&l), Some(&r), &diff);
        assert_eq!(
            stop,
            Some(ChangeStop {
                position: 3,
                wrapped: true
            })
        );
    }

    #[test]
    fn find_change_stops_at_collapsed_containers_with_changes() {
        let l = doc(r#"{"o":{"x":1},"p":1}"#);
        let r = doc(r#"{"o":{"x":2},"p":1}"#);
        let mut rows = Rows::new(Some(&l), Some(&r), DiffOptions::default());
        assert_eq!(stops(&rows, &l, &r, Direction::Forward).0, [2; 4]);
        rows.toggle(1, Some(&l), Some(&r));
        assert_eq!(stops(&rows, &l, &r, Direction::Forward).0, [1; 4]);
    }

    #[test]
    fn find_change_type_change_counts_each_side_once() {
        let l = doc(r#"{"a":[1,2]}"#);
        let r = doc(r#"{"a":"s"}"#);
        let rows = Rows::new(Some(&l), Some(&r), DiffOptions::default());
        // 0 `{`, 1 `a: [` (solo izq.), 2 `1`, 3 `2`, 4 `]`, 5 `a: "s"` (solo der.), 6 `}`
        assert_eq!(stops(&rows, &l, &r, Direction::Forward).0, [1, 5, 1, 5]);
    }

    #[test]
    fn find_change_ignores_ignored_paths() {
        let l = doc(r#"{"t":1,"a":1}"#);
        let r = doc(r#"{"t":2,"a":1}"#);
        let options = DiffOptions {
            ignore: vec![crate::IgnoreRule::Key("t".to_owned())],
            ..DiffOptions::default()
        };
        let rows = Rows::new(Some(&l), Some(&r), options.clone());
        let diff = crate::diff(&l, &r, &options);
        assert_eq!(
            rows.find_change(None, Direction::Forward, Some(&l), Some(&r), &diff),
            None
        );
    }
}
