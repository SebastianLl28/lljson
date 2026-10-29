//! Status bar of the Viewer: one line telling whether the text is valid JSON.

use gtk::prelude::*;

use super::validate::Validation;

/// Libadwaita style class that colors the message.
const STATE_CLASSES: [&str; 2] = ["success", "error"];

pub struct StatusBar {
    pub root: gtk::Box,
    message: gtk::Label,
    hint: gtk::Label,
}

/// What the status bar says about a validation outcome.
#[derive(Debug, PartialEq, Eq)]
struct Status {
    message: String,
    hint: Option<String>,
    /// Style class that colors the message; `None` for the neutral look.
    class: Option<&'static str>,
}

fn label(css_class: &str) -> gtk::Label {
    gtk::Label::builder()
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .selectable(true)
        .css_classes([css_class])
        .build()
}

impl StatusBar {
    pub fn new() -> StatusBar {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(12)
            .margin_end(12)
            .build();
        let message = label("dim-label");
        let hint = label("dim-label");
        hint.set_visible(false);
        root.append(&message);
        root.append(&hint);
        let bar = StatusBar {
            root,
            message,
            hint,
        };
        bar.show(&Validation::Empty);
        bar
    }

    pub fn show(&self, validation: &Validation) {
        let status = describe(validation);
        self.message.set_text(&status.message);
        for state in STATE_CLASSES {
            self.message.remove_css_class(state);
        }
        match status.class {
            Some(class) => {
                self.message.remove_css_class("dim-label");
                self.message.add_css_class(class);
            }
            None => self.message.add_css_class("dim-label"),
        }
        self.hint.set_visible(status.hint.is_some());
        self.hint
            .set_text(status.hint.as_deref().unwrap_or_default());
    }
}

fn describe(validation: &Validation) -> Status {
    match validation {
        Validation::Empty => Status {
            message: "Empty".to_owned(),
            hint: None,
            class: None,
        },
        Validation::Valid => Status {
            message: "Valid JSON".to_owned(),
            hint: None,
            class: Some("success"),
        },
        Validation::Invalid { error, hint, .. } => Status {
            message: format!(
                "Line {}, column {}: {}",
                error.line, error.column, error.kind
            ),
            hint: hint.as_ref().map(|hint| format!("Hint: {hint}")),
            class: Some("error"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewer::validate::validate;

    #[test]
    fn empty_is_neutral() {
        let status = describe(&validate("  ".to_owned()));
        assert_eq!((status.message.as_str(), status.class), ("Empty", None));
        assert_eq!(status.hint, None);
    }

    #[test]
    fn valid_is_success() {
        let status = describe(&validate("[1]".to_owned()));
        assert_eq!(
            (status.message.as_str(), status.class),
            ("Valid JSON", Some("success"))
        );
        assert_eq!(status.hint, None);
    }

    #[test]
    fn invalid_reports_position_reason_and_hint() {
        let status = describe(&validate("{\n  \"a\": 1,\n  x\n}".to_owned()));
        assert_eq!(
            status.message,
            "Line 3, column 3: unexpected character 'x': expected a quoted key"
        );
        assert_eq!(
            status.hint.as_deref(),
            Some("Hint: Object keys must be quoted with double quotes")
        );
        assert_eq!(status.class, Some("error"));
    }

    #[test]
    fn truncated_text_reports_the_end() {
        let status = describe(&validate("[1, 2".to_owned()));
        assert!(
            status
                .message
                .starts_with("Line 1, column 6: unexpected end of text"),
            "{}",
            status.message
        );
        assert_eq!(
            status.hint.as_deref(),
            Some("Hint: Unclosed `[` opened at line 1: add the closing `]`")
        );
    }

    #[test]
    fn error_without_advice_has_no_hint() {
        assert_eq!(describe(&validate("[-]".to_owned())).hint, None);
    }
}
