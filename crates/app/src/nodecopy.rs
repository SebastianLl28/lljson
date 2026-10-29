//! Qué se copia al portapapeles de un nodo (ruta o valor) y el menú contextual de las
//! filas. El texto se calcula con funciones puras; el menú es solo la capa de GTK.

use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gdk, glib};
use jsonlens_core::{Document, FormatOptions, Kind, NodeId, format_node};

/// Valor de un nodo como texto para pegar en otro sitio: los strings sin comillas y con
/// los escapes resueltos; los escalares tal como están en el original; los contenedores
/// como JSON formateado con 2 espacios.
pub fn value_text(doc: &Document, id: NodeId) -> String {
    match doc.kind(id) {
        Some(Kind::String) => doc.string(id).map(String::from).unwrap_or_default(),
        Some(Kind::Array | Kind::Object) => format_node(doc, id, &FormatOptions::default()),
        Some(_) => doc.raw(id).to_owned(),
        None => String::new(),
    }
}

/// Ruta JSONPath de un nodo (`$.users[3].email`).
pub fn path_text(doc: &Document, id: NodeId) -> String {
    doc.path(id)
}

/// Qué acción del menú se eligió.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Copy {
    Path,
    Value,
}

/// Resultado de copiar, para que la ventana lo comunique al usuario.
pub type Notify = Rc<dyn Fn(&str)>;

/// Copia la ruta o el valor de `node`. Los contenedores se formatean en un hilo de fondo
/// porque pueden ser enormes; el resto se copia al instante.
pub fn copy(
    clipboard: &gdk::Clipboard,
    doc: &Arc<Document>,
    node: NodeId,
    what: Copy,
    notify: &Notify,
) {
    match what {
        Copy::Path => {
            let path = path_text(doc, node);
            clipboard.set_text(&path);
            notify(&format!("Ruta copiada: {}", preview(&path)));
        }
        Copy::Value if doc.kind(node).is_some_and(Kind::is_container) => {
            let doc = Arc::clone(doc);
            copy_in_background(
                clipboard,
                notify,
                move || value_text(&doc, node),
                "Valor copiado",
            );
        }
        Copy::Value => {
            clipboard.set_text(&value_text(doc, node));
            notify("Valor copiado");
        }
    }
}

/// Copia el texto que produce `produce` (calculado en un hilo de fondo, por si es enorme) y
/// avisa con `done` al terminar.
pub fn copy_in_background(
    clipboard: &gdk::Clipboard,
    notify: &Notify,
    produce: impl FnOnce() -> String + Send + 'static,
    done: &'static str,
) {
    let (sender, receiver) = async_channel::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("jsonlens-copiar".to_owned())
        .spawn(move || {
            let _ = sender.send_blocking(produce());
        });
    if let Err(err) = spawned {
        notify(&format!("No se pudo copiar: {err}"));
        return;
    }
    let clipboard = clipboard.clone();
    let notify = Rc::clone(notify);
    glib::MainContext::default().spawn_local(async move {
        match receiver.recv().await {
            Ok(text) => {
                clipboard.set_text(&text);
                notify(done);
            }
            Err(_) => notify("No se pudo copiar"),
        }
    });
}

/// Texto completo de un documento tal como está cargado (tras formatear, minificar u
/// ordenar es el resultado de esa operación).
pub fn document_text(doc: &Document) -> String {
    String::from_utf8_lossy(doc.source_bytes()).into_owned()
}

/// Menú contextual de una fila, anclado en `(x, y)` de `parent`.
pub fn show_menu(parent: &gtk::Widget, x: f64, y: f64, on_choose: impl Fn(Copy) + 'static) {
    let on_choose = Rc::new(on_choose);
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .halign(gtk::Align::Start)
        .css_classes(["menu"])
        .build();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for (label, what) in [("Copiar ruta", Copy::Path), ("Copiar valor", Copy::Value)] {
        let button = gtk::Button::builder()
            .label(label)
            .css_classes(["flat"])
            .halign(gtk::Align::Fill)
            .build();
        if let Some(child) = button.child().and_downcast::<gtk::Label>() {
            child.set_xalign(0.0);
        }
        let popover = popover.clone();
        let on_choose = Rc::clone(&on_choose);
        button.connect_clicked(move |_| {
            popover.popdown();
            on_choose(what);
        });
        list.append(&button);
    }
    popover.set_child(Some(&list));
    popover.set_parent(parent);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    // Un popover con padre propio debe desanclarse al cerrarse.
    popover.connect_closed(|popover| {
        let popover = popover.clone();
        glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
}

/// Recorta un texto largo para mostrarlo en un aviso.
fn preview(text: &str) -> String {
    const MAX: usize = 60;
    if text.chars().count() <= MAX {
        return text.to_owned();
    }
    let head: String = text.chars().take(MAX).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonlens_core::{ParseOptions, parse};

    fn doc(text: &str) -> Document {
        parse(text, &ParseOptions::default())
            .expect("JSON válido")
            .document
    }

    fn find(doc: &Document, path: &str) -> NodeId {
        (0..doc.node_count())
            .map(NodeId::from_index)
            .find(|&id| doc.path(id) == path)
            .expect("ruta existente")
    }

    #[test]
    fn strings_are_copied_without_quotes_and_decoded() {
        let d = doc(r#"{"a":"x\ny é \"q\""}"#);
        assert_eq!(value_text(&d, find(&d, "$.a")), "x\ny é \"q\"");
    }

    #[test]
    fn scalars_keep_their_original_text() {
        let d = doc(r#"{"n":1.50e3,"t":true,"z":null,"big":12345678901234567890}"#);
        assert_eq!(value_text(&d, find(&d, "$.n")), "1.50e3");
        assert_eq!(value_text(&d, find(&d, "$.t")), "true");
        assert_eq!(value_text(&d, find(&d, "$.z")), "null");
        assert_eq!(value_text(&d, find(&d, "$.big")), "12345678901234567890");
    }

    #[test]
    fn containers_are_formatted() {
        let d = doc(r#"{"o":{"a":[1,2]},"e":[]}"#);
        assert_eq!(
            value_text(&d, find(&d, "$.o")),
            "{\n  \"a\": [\n    1,\n    2\n  ]\n}"
        );
        assert_eq!(value_text(&d, find(&d, "$.e")), "[]");
    }

    #[test]
    fn document_text_is_the_loaded_source() {
        let text = " {\"a\" : [1,\n2]} ";
        assert_eq!(document_text(&doc(text)), text);
    }

    #[test]
    fn invalid_node_gives_empty_text() {
        let d = doc("1");
        assert_eq!(value_text(&d, NodeId::from_index(1000)), "");
    }

    #[test]
    fn paths_use_jsonpath() {
        let d = doc(r#"{"users":[{"email":"a"},{"a b":1}]}"#);
        assert_eq!(
            path_text(&d, find(&d, "$.users[0].email")),
            "$.users[0].email"
        );
        assert_eq!(
            path_text(&d, find(&d, "$.users[1][\"a b\"]")),
            "$.users[1][\"a b\"]"
        );
        assert_eq!(path_text(&d, d.root()), "$");
    }

    #[test]
    fn preview_truncates_long_text() {
        assert_eq!(preview("$.a"), "$.a");
        let long = "x".repeat(100);
        assert_eq!(preview(&long).chars().count(), 61);
    }
}
