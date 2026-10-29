//! Search box widgets and the search state the row view reads for highlighting.

use adw::prelude::*;
use lljson_core::{MatchMode, NodeId, Scope};

use crate::pane::{Side, group_digits};

/// Matches of one pane's search, in document order.
#[derive(Default)]
pub struct SearchState {
    pub matches: Vec<NodeId>,
    pub current: Option<usize>,
}

impl SearchState {
    pub fn is_match(&self, node: NodeId) -> bool {
        self.matches.binary_search(&node).is_ok()
    }

    pub fn is_current(&self, node: NodeId) -> bool {
        self.current
            .and_then(|i| self.matches.get(i))
            .is_some_and(|&n| n == node)
    }

    pub fn clear(&mut self) {
        self.matches.clear();
        self.current = None;
    }
}

/// Text for the result counter next to the search box.
pub fn count_text(current: Option<usize>, total: usize) -> String {
    match (current, total) {
        (_, 0) => "No results".to_owned(),
        (Some(i), n) => format!("{} of {}", group_digits(i + 1), group_digits(n)),
        (None, n) => format!("{} results", group_digits(n)),
    }
}

/// The search box: a small card that floats over the top-right of a pane, like an editor's
/// find widget.
pub struct SearchUi {
    pub root: gtk::Revealer,
    pub entry: gtk::SearchEntry,
    scope: gtk::DropDown,
    mode: gtk::DropDown,
    count: gtk::Label,
}

impl SearchUi {
    /// The buttons act on the pane's own search through the `*-left` / `*-right` actions.
    pub fn new(side: Side) -> SearchUi {
        let name = side.name();
        SearchUi::with_actions(
            &format!("win.search-next-{name}"),
            &format!("win.search-prev-{name}"),
            &format!("win.search-close-{name}"),
        )
    }

    /// Same box, with the buttons wired to the given previous / next / close actions.
    pub fn with_actions(next: &str, prev: &str, close: &str) -> SearchUi {
        let entry = gtk::SearchEntry::builder()
            .hexpand(true)
            .width_chars(6)
            .placeholder_text("Search…")
            .build();
        let scope = gtk::DropDown::from_strings(&["Both", "Keys", "Values"]);
        scope.set_tooltip_text(Some("Search keys and values, only keys, or only values"));
        scope.set_hexpand(true);
        let mode = gtk::DropDown::from_strings(&["Normal", "Exact", "Regex"]);
        mode.set_tooltip_text(Some(
            "Normal ignores case and accents · Exact respects them · Regex is a regular expression",
        ));
        let count = gtk::Label::builder()
            .width_chars(7)
            .xalign(1.0)
            .css_classes(["dim-label", "numeric"])
            .build();
        let button = |icon: &str, action: &str, tooltip: &str| {
            gtk::Button::builder()
                .icon_name(icon)
                .action_name(action)
                .tooltip_text(tooltip)
                .css_classes(["flat"])
                .build()
        };
        let top = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .build();
        top.append(&entry);
        top.append(&count);
        top.append(&button(
            "go-up-symbolic",
            prev,
            "Previous result (Shift+Enter)",
        ));
        top.append(&button(
            "go-down-symbolic",
            next,
            "Next result (Enter)",
        ));
        top.append(&button(
            "window-close-symbolic",
            close,
            "Close (Esc)",
        ));
        let options = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        options.append(&scope);
        options.append(&mode);
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(["search-box"])
            .build();
        card.append(&top);
        card.append(&options);
        let root = gtk::Revealer::builder()
            .child(&card)
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(6)
            .margin_end(18)
            .margin_start(6)
            .margin_bottom(6)
            .build();
        SearchUi {
            root,
            entry,
            scope,
            mode,
            count,
        }
    }

    pub fn is_open(&self) -> bool {
        self.root.reveals_child()
    }

    pub fn set_open(&self, open: bool) {
        self.root.set_reveal_child(open);
    }

    pub fn scope(&self) -> Scope {
        match self.scope.selected() {
            1 => Scope::Keys,
            2 => Scope::Values,
            _ => Scope::Both,
        }
    }

    pub fn mode(&self) -> MatchMode {
        match self.mode.selected() {
            1 => MatchMode::Exact,
            2 => MatchMode::Regex,
            _ => MatchMode::Normal,
        }
    }

    pub fn connect_options_changed(&self, f: impl Fn() + Clone + 'static) {
        let g = f.clone();
        self.scope.connect_selected_notify(move |_| g());
        self.mode.connect_selected_notify(move |_| f());
    }

    pub fn set_count(&self, text: &str) {
        self.count.set_label(text);
    }

    /// Marks the entry as holding a bad regular expression.
    pub fn set_error(&self, error: Option<&str>) {
        if error.is_some() {
            self.entry.add_css_class("error");
        } else {
            self.entry.remove_css_class("error");
        }
        self.entry.set_tooltip_text(error);
    }

    #[cfg(any(debug_assertions, feature = "devtools"))]
    pub fn select_options(&self, scope: u32, mode: u32) {
        self.scope.set_selected(scope);
        self.mode.set_selected(mode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(matches: &[usize], current: Option<usize>) -> SearchState {
        SearchState {
            matches: matches.iter().map(|&i| NodeId::from_index(i)).collect(),
            current,
        }
    }

    #[test]
    fn match_lookup() {
        let s = state(&[1, 5, 9], Some(1));
        assert!(s.is_match(NodeId::from_index(5)));
        assert!(!s.is_match(NodeId::from_index(6)));
    }

    #[test]
    fn current_is_the_selected_match() {
        let s = state(&[1, 5, 9], Some(1));
        assert!(s.is_current(NodeId::from_index(5)));
        assert!(!s.is_current(NodeId::from_index(1)));
        assert!(!state(&[1], None).is_current(NodeId::from_index(1)));
    }

    #[test]
    fn clear_forgets_everything() {
        let mut s = state(&[1, 2], Some(0));
        s.clear();
        assert!(s.matches.is_empty() && s.current.is_none());
    }

    #[test]
    fn counter_text() {
        assert_eq!(count_text(None, 0), "No results");
        assert_eq!(count_text(Some(0), 0), "No results");
        assert_eq!(count_text(Some(2), 27), "3 of 27");
        assert_eq!(count_text(None, 1200), "1\u{202F}200 results");
    }
}
