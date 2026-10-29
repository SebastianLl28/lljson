//! Status bar of the Viewer: one line telling whether the text is valid JSON.

use gtk::prelude::*;

use super::validate::Validation;

/// Libadwaita style class that colors the message.
const STATE_CLASSES: [&str; 2] = ["success", "error"];

pub struct StatusBar {
    pub root: gtk::Label,
}

impl StatusBar {
    pub fn new() -> StatusBar {
        let root = gtk::Label::builder()
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .selectable(true)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(12)
            .margin_end(12)
            .build();
        let bar = StatusBar { root };
        bar.show(&Validation::Empty);
        bar
    }

    pub fn show(&self, validation: &Validation) {
        let (message, class) = describe(validation);
        self.root.set_text(&message);
        for state in STATE_CLASSES {
            self.root.remove_css_class(state);
        }
        match class {
            Some(class) => {
                self.root.remove_css_class("dim-label");
                self.root.add_css_class(class);
            }
            None => self.root.add_css_class("dim-label"),
        }
    }
}

/// Message and style class (`None` for the neutral look) of a validation outcome.
fn describe(validation: &Validation) -> (String, Option<&'static str>) {
    match validation {
        Validation::Empty => ("Empty".to_owned(), None),
        Validation::Valid => ("Valid JSON".to_owned(), Some("success")),
        Validation::Invalid(err) => (
            format!("Line {}, column {}: {}", err.line, err.column, err.kind),
            Some("error"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewer::validate::validate;

    #[test]
    fn empty_is_neutral() {
        assert_eq!(
            describe(&validate("  ".to_owned())),
            ("Empty".to_owned(), None)
        );
    }

    #[test]
    fn valid_is_success() {
        assert_eq!(
            describe(&validate("[1]".to_owned())),
            ("Valid JSON".to_owned(), Some("success"))
        );
    }

    #[test]
    fn invalid_reports_position_and_reason() {
        let (message, class) = describe(&validate("{\n  \"a\": 1,\n  x\n}".to_owned()));
        assert_eq!(
            message,
            "Line 3, column 3: unexpected character 'x': expected a quoted key".to_owned()
        );
        assert_eq!(class, Some("error"));
    }

    #[test]
    fn truncated_text_reports_the_end() {
        let (message, class) = describe(&validate("[1, 2".to_owned()));
        assert!(
            message.starts_with("Line 1, column 6: unexpected end of text"),
            "{message}"
        );
        assert_eq!(class, Some("error"));
    }
}
