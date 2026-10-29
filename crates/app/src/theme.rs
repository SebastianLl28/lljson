//! Selectable themes: the three system-driven ones plus named palettes that repaint the
//! window, the JSON view and the syntax colors. A palette is turned into CSS that overrides
//! libadwaita's named colors, so every widget follows it.

use std::cell::RefCell;

pub const DEFAULT_ID: &str = "system";

/// Colors of a named theme, as `#rrggbb`.
pub struct Palette {
    /// Palettes are forced to a light or dark libadwaita base.
    pub dark: bool,
    pub window: &'static str,
    pub view: &'static str,
    pub header: &'static str,
    pub text: &'static str,
    pub accent: &'static str,
    pub key: &'static str,
    pub string: &'static str,
    pub number: &'static str,
    pub boolean: &'static str,
    pub null: &'static str,
}

pub struct Theme {
    pub id: &'static str,
    pub label: &'static str,
    /// Listed apart from the named palettes in the menu.
    pub stock: bool,
    /// `None` only for "Follow system", which leaves colors to the system and the shell.
    pub palette: Option<Palette>,
}

pub const THEMES: &[Theme] = &[
    Theme {
        id: "system",
        label: "Follow system",
        stock: true,
        palette: None,
    },
    Theme {
        id: "light",
        label: "Light",
        stock: true,
        palette: Some(Palette {
            dark: false,
            window: "#fafafb",
            view: "#ffffff",
            header: "#ebebed",
            text: "#2e3436",
            accent: "#3584e4",
            key: "#1a5fb4",
            string: "#813d9c",
            number: "#c64600",
            boolean: "#0d7377",
            null: "#77767b",
        }),
    },
    Theme {
        id: "dark",
        label: "Dark",
        stock: true,
        palette: Some(Palette {
            dark: true,
            window: "#242424",
            view: "#1e1e1e",
            header: "#303030",
            text: "#ffffff",
            accent: "#3584e4",
            key: "#99c1f1",
            string: "#dc8add",
            number: "#ffa348",
            boolean: "#6fd3c7",
            null: "#9a9996",
        }),
    },
    Theme {
        id: "midnight",
        label: "Midnight",
        stock: false,
        palette: Some(Palette {
            dark: true,
            window: "#0b1020",
            view: "#0f1630",
            header: "#17214a",
            text: "#e6e9ff",
            accent: "#5b8cff",
            key: "#7aa2ff",
            string: "#c792ea",
            number: "#ffb86c",
            boolean: "#4fd6c8",
            null: "#7f87b5",
        }),
    },
    Theme {
        id: "synthwave",
        label: "Synthwave",
        stock: false,
        palette: Some(Palette {
            dark: true,
            window: "#1a1033",
            view: "#241447",
            header: "#3a1f7a",
            text: "#f4e9ff",
            accent: "#ff2e97",
            key: "#36f9f6",
            string: "#ff7edb",
            number: "#ff9e64",
            boolean: "#b893ff",
            null: "#8d86c4",
        }),
    },
    Theme {
        id: "dracula",
        label: "Dracula",
        stock: false,
        palette: Some(Palette {
            dark: true,
            window: "#282a36",
            view: "#21222c",
            header: "#3b3e52",
            text: "#f8f8f2",
            accent: "#bd93f9",
            key: "#8be9fd",
            string: "#ff79c6",
            number: "#ffb86c",
            boolean: "#bd93f9",
            null: "#7283b5",
        }),
    },
    Theme {
        id: "ocean",
        label: "Ocean",
        stock: false,
        palette: Some(Palette {
            dark: true,
            window: "#08242b",
            view: "#0a2e36",
            header: "#0f4753",
            text: "#dff6f5",
            accent: "#14b8c4",
            key: "#5ad1e6",
            string: "#ffb3c1",
            number: "#ffa26b",
            boolean: "#9fe3d0",
            null: "#6fa0a7",
        }),
    },
    Theme {
        id: "sunset",
        label: "Sunset",
        stock: false,
        palette: Some(Palette {
            dark: true,
            window: "#2a1320",
            view: "#331828",
            header: "#53213f",
            text: "#ffe9e0",
            accent: "#ff7a59",
            key: "#ffa38a",
            string: "#f4a6e6",
            number: "#ffcf7a",
            boolean: "#8fd3ff",
            null: "#b08ea2",
        }),
    },
    Theme {
        id: "nord",
        label: "Nord",
        stock: false,
        palette: Some(Palette {
            dark: true,
            window: "#2e3440",
            view: "#272c36",
            header: "#434c5e",
            text: "#eceff4",
            accent: "#88c0d0",
            key: "#81a1c1",
            string: "#b48ead",
            number: "#d08770",
            boolean: "#8fbcbb",
            null: "#7b88a1",
        }),
    },
    Theme {
        id: "paper",
        label: "Paper",
        stock: false,
        palette: Some(Palette {
            dark: false,
            window: "#f1e7cf",
            view: "#fbf6e9",
            header: "#e3d3ae",
            text: "#3b3228",
            accent: "#b5651d",
            key: "#1d5fa8",
            string: "#8a3f8f",
            number: "#b5470c",
            boolean: "#0b7a75",
            null: "#8a7f6d",
        }),
    },
    Theme {
        id: "bubblegum",
        label: "Bubblegum",
        stock: false,
        palette: Some(Palette {
            dark: false,
            window: "#ffd9ec",
            view: "#fff4fa",
            header: "#ffb3d9",
            text: "#3a1030",
            accent: "#e5167f",
            key: "#5b3bd6",
            string: "#c2107a",
            number: "#d9480f",
            boolean: "#0a8f8f",
            null: "#8d6a82",
        }),
    },
];

/// Unknown ids (e.g. from a newer version's config) fall back to following the system.
pub fn by_id(id: &str) -> &'static Theme {
    let id = id.trim();
    THEMES.iter().find(|t| t.id == id).unwrap_or(&THEMES[0])
}

fn color_scheme(theme: &Theme) -> adw::ColorScheme {
    match &theme.palette {
        Some(p) if p.dark => adw::ColorScheme::ForceDark,
        Some(_) => adw::ColorScheme::ForceLight,
        None => adw::ColorScheme::Default,
    }
}

/// CSS for a theme: libadwaita named colors (both `@define-color` and the `--*` variables) plus the JSON syntax classes. Named themes also
/// get an accent line under the bars.
pub fn palette_css(theme: &Theme, p: &Palette) -> String {
    let Palette {
        window,
        view,
        header,
        text,
        accent,
        key,
        string,
        number,
        boolean,
        null,
        ..
    } = p;
    let bars = if theme.stock {
        String::new()
    } else {
        format!(
            "headerbar {{ box-shadow: 0 1px 0 alpha({accent}, 0.55); }}
.summary-bar {{ background-color: {header}; border-top: 1px solid alpha({accent}, 0.55); }}
"
        )
    };
    format!(
        "@define-color window_bg_color {window};
@define-color window_fg_color {text};
@define-color view_bg_color {view};
@define-color view_fg_color {text};
@define-color headerbar_bg_color {header};
@define-color headerbar_backdrop_color {window};
@define-color headerbar_fg_color {text};
@define-color card_bg_color {header};
@define-color card_fg_color {text};
@define-color popover_bg_color {header};
@define-color popover_fg_color {text};
@define-color dialog_bg_color {window};
@define-color dialog_fg_color {text};
@define-color accent_bg_color {accent};
@define-color accent_color {accent};
@define-color accent_fg_color #ffffff;
:root {{
  --window-bg-color: {window};
  --window-fg-color: {text};
  --view-bg-color: {view};
  --view-fg-color: {text};
  --headerbar-bg-color: {header};
  --headerbar-backdrop-color: {window};
  --headerbar-fg-color: {text};
  --card-bg-color: {header};
  --card-fg-color: {text};
  --popover-bg-color: {header};
  --popover-fg-color: {text};
  --dialog-bg-color: {window};
  --dialog-fg-color: {text};
  --accent-bg-color: {accent};
  --accent-color: {accent};
  --accent-fg-color: #ffffff;
}}
{bars}.json-key {{ color: {key}; }}
.json-string {{ color: {string}; }}
.json-number {{ color: {number}; }}
.json-bool {{ color: {boolean}; }}
.json-null {{ color: {null}; }}
"
    )
}

thread_local! {
    static PROVIDER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
}

/// Applies `theme` to the whole application.
pub fn apply(theme: &Theme) {
    adw::StyleManager::default().set_color_scheme(color_scheme(theme));
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    PROVIDER.with(|cell| {
        // Replacing the provider (instead of reloading its text) makes GTK drop the
        // previous theme's named colors.
        if let Some(old) = cell.borrow_mut().take() {
            gtk::style_context_remove_provider_for_display(&display, &old);
        }
        let Some(palette) = &theme.palette else {
            return;
        };
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&palette_css(theme, palette));
        // Above the user's own gtk-4.0/gtk.css (e.g. shell-generated colors), which would
        // otherwise repaint the chosen theme.
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER + 1,
        );
        cell.replace(Some(provider));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_hex(color: &str) -> bool {
        color.len() == 7
            && color.starts_with('#')
            && color[1..].chars().all(|c| c.is_ascii_hexdigit())
    }

    #[test]
    fn ids_are_unique_and_round_trip() {
        for (i, theme) in THEMES.iter().enumerate() {
            assert_eq!(by_id(theme.id).id, theme.id);
            assert!(THEMES[..i].iter().all(|t| t.id != theme.id));
        }
    }

    #[test]
    fn unknown_id_follows_the_system() {
        assert_eq!(by_id("purple").id, DEFAULT_ID);
        assert_eq!(by_id("").id, DEFAULT_ID);
        assert_eq!(by_id(" dark\n").id, "dark");
    }

    #[test]
    fn palettes_use_valid_hex_colors() {
        for palette in THEMES.iter().filter_map(|t| t.palette.as_ref()) {
            for color in [
                palette.window,
                palette.view,
                palette.header,
                palette.text,
                palette.accent,
                palette.key,
                palette.string,
                palette.number,
                palette.boolean,
                palette.null,
            ] {
                assert!(is_hex(color), "bad color {color}");
            }
        }
    }

    #[test]
    fn css_carries_the_palette() {
        let theme = by_id("synthwave");
        let css = palette_css(theme, theme.palette.as_ref().unwrap());
        assert!(css.contains("@define-color window_bg_color #1a1033;"));
        assert!(css.contains("--window-bg-color: #1a1033;"));
        assert!(css.contains(".json-key { color: #36f9f6; }"));
    }
}
