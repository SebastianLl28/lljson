//! Opciones de comparación: clave para emparejar los elementos de los arrays y reglas
//! para ignorar claves o rutas. El diálogo es solo la capa de GTK sobre [`parse`].

use std::rc::Rc;

use adw::prelude::*;
use lljson_core::{DiffOptions, IgnoreRule};

/// Lo que el usuario escribió en el diálogo (se conserva para volver a mostrarlo tal cual).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OptionsText {
    pub array_key: String,
    pub ignore: String,
}

/// Interpreta el texto del diálogo. Devuelve todos los errores a la vez, uno por línea
/// inválida, para que el usuario pueda corregirlos de una vez.
pub fn parse(text: &OptionsText) -> Result<DiffOptions, Vec<String>> {
    let array_key = text.array_key.trim();
    let mut ignore = Vec::new();
    let mut errors = Vec::new();
    for (index, line) in text.ignore.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match IgnoreRule::parse(line) {
            Ok(rule) => ignore.push(rule),
            Err(err) => errors.push(format!("Línea {}: {err}", index + 1)),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(DiffOptions {
        array_key: (!array_key.is_empty()).then(|| array_key.to_owned()),
        ignore,
    })
}

/// Muestra el diálogo con `current` y llama a `on_apply` con las opciones válidas.
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    current: &OptionsText,
    on_apply: impl Fn(OptionsText, DiffOptions) + 'static,
) {
    let key_row = adw::EntryRow::builder()
        .title("Clave de los elementos de arrays (p. ej. id)")
        .text(&current.array_key)
        .build();
    let keys = adw::PreferencesGroup::builder()
        .title("Arrays")
        .description(
            "Empareja los elementos por esta clave en lugar de por posición. \
             Vacío: por posición.",
        )
        .build();
    keys.add(&key_row);

    let rules = gtk::TextView::builder()
        .monospace(true)
        .accepts_tab(false)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(10)
        .right_margin(10)
        .build();
    rules.buffer().set_text(&current.ignore);
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&rules)
        .min_content_height(140)
        .css_classes(["card", "options-rules"])
        .build();
    let ignore = adw::PreferencesGroup::builder()
        .title("Ignorar")
        .description(
            "Una regla por línea: una clave a cualquier profundidad (timestamp) \
             o una ruta ($.users[*].updatedAt).",
        )
        .build();
    ignore.add(&scrolled);

    let errors = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .css_classes(["error"])
        .build();
    let page = adw::PreferencesPage::new();
    page.add(&keys);
    page.add(&ignore);
    let error_group = adw::PreferencesGroup::new();
    error_group.add(&errors);
    page.add(&error_group);

    let apply = gtk::Button::builder()
        .label("Aplicar")
        .css_classes(["suggested-action"])
        .build();
    let header = adw::HeaderBar::new();
    header.pack_end(&apply);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title("Opciones de comparación")
        .content_width(480)
        .content_height(560)
        .child(&view)
        .default_widget(&apply)
        .build();

    let on_apply = Rc::new(on_apply);
    let key_row_enter = key_row.clone();
    let submit = {
        let dialog = dialog.clone();
        move || {
            let buffer = rules.buffer();
            let text = OptionsText {
                array_key: key_row.text().into(),
                ignore: buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .into(),
            };
            match parse(&text) {
                Ok(options) => {
                    on_apply(text, options);
                    dialog.close();
                }
                Err(messages) => {
                    errors.set_label(&messages.join("\n"));
                    errors.set_visible(true);
                }
            }
        }
    };
    let submit = Rc::new(submit);
    let on_click = Rc::clone(&submit);
    apply.connect_clicked(move |_| on_click());
    key_row_enter.connect_entry_activated(move |_| submit());
    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(array_key: &str, ignore: &str) -> OptionsText {
        OptionsText {
            array_key: array_key.to_owned(),
            ignore: ignore.to_owned(),
        }
    }

    #[test]
    fn empty_text_gives_default_options() {
        let options = parse(&text("", "")).expect("válido");
        assert_eq!(options.array_key, None);
        assert!(options.ignore.is_empty());
        let options = parse(&text("   ", "\n  \n")).expect("válido");
        assert_eq!(options.array_key, None);
        assert!(options.ignore.is_empty());
    }

    #[test]
    fn trims_the_key_and_parses_keys_and_paths() {
        let options =
            parse(&text(" id ", "timestamp\n  $.users[*].updatedAt \n\n")).expect("válido");
        assert_eq!(options.array_key.as_deref(), Some("id"));
        assert_eq!(options.ignore.len(), 2);
        assert_eq!(options.ignore[0], IgnoreRule::Key("timestamp".to_owned()));
        assert!(matches!(options.ignore[1], IgnoreRule::Path(_)));
    }

    #[test]
    fn reports_every_invalid_line_with_its_number() {
        let errors = parse(&text("id", "ok\n$.a[\n\n$..b[x]")).expect_err("inválido");
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].starts_with("Línea 2:"), "{errors:?}");
        assert!(errors[1].starts_with("Línea 4:"), "{errors:?}");
    }
}
