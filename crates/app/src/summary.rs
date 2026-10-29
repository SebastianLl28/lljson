//! Barra resumen inferior: cuántos valores se añadieron, eliminaron o modificaron.

use adw::prelude::*;
use jsonlens_core::Summary;

/// Qué sabe la barra sobre la comparación actual.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Falta algún documento: no hay nada que comparar.
    Incomplete,
    Compared(Summary),
}

/// Texto de un contador con su plural: `1 añadido`, `3 añadidos`.
pub fn count_text(n: usize, singular: &str, plural: &str) -> String {
    format!("{n} {}", if n == 1 { singular } else { plural })
}

pub struct SummaryBar {
    pub root: gtk::Box,
    message: gtk::Label,
    added: gtk::Label,
    removed: gtk::Label,
    modified: gtk::Label,
}

impl SummaryBar {
    pub fn new() -> SummaryBar {
        let chip = |class: &str| {
            gtk::Label::builder()
                .css_classes(["summary-chip", class])
                .build()
        };
        let message = gtk::Label::builder().css_classes(["dim-label"]).build();
        let added = chip("summary-added");
        let removed = chip("summary-removed");
        let modified = chip("summary-modified");
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .css_classes(["summary-bar"])
            .build();
        for widget in [&message, &added, &removed, &modified] {
            root.append(widget);
        }
        let bar = SummaryBar {
            root,
            message,
            added,
            removed,
            modified,
        };
        bar.update(State::Incomplete);
        bar
    }

    pub fn update(&self, state: State) {
        let counts = match state {
            State::Incomplete => {
                self.message
                    .set_label("Abre dos documentos para compararlos");
                None
            }
            State::Compared(s) if s.total() == 0 => {
                self.message.set_label("Sin diferencias");
                None
            }
            State::Compared(s) => Some(s),
        };
        self.message.set_visible(counts.is_none());
        for chip in [&self.added, &self.removed, &self.modified] {
            chip.set_visible(counts.is_some());
        }
        if let Some(s) = counts {
            self.added
                .set_label(&count_text(s.added, "añadido", "añadidos"));
            self.removed
                .set_label(&count_text(s.removed, "eliminado", "eliminados"));
            self.modified
                .set_label(&count_text(s.modified, "modificado", "modificados"));
            self.modified.set_tooltip_text(
                (s.type_changed > 0)
                    .then(|| {
                        count_text(
                            s.type_changed,
                            "cambio de tipo incluido",
                            "cambios de tipo incluidos",
                        )
                    })
                    .as_deref(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_text_handles_plurals() {
        assert_eq!(count_text(0, "añadido", "añadidos"), "0 añadidos");
        assert_eq!(count_text(1, "añadido", "añadidos"), "1 añadido");
        assert_eq!(count_text(2, "añadido", "añadidos"), "2 añadidos");
        assert_eq!(
            count_text(1_000_000, "eliminado", "eliminados"),
            "1000000 eliminados"
        );
    }
}
