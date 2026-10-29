//! What survives between runs: the color scheme and the last JSON loaded in each pane.
//! Plain files under the user's config/data directories; independent of GTK widgets.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::loader::Input;
use crate::pane::Side;

/// Larger documents are not restored: writing them would cost more than it helps.
const MAX_SAVED_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    System,
    Light,
    Dark,
}

impl Scheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Scheme::System => "system",
            Scheme::Light => "light",
            Scheme::Dark => "dark",
        }
    }

    pub fn parse(text: &str) -> Scheme {
        match text.trim() {
            "light" => Scheme::Light,
            "dark" => Scheme::Dark,
            _ => Scheme::System,
        }
    }
}

fn config_dir() -> PathBuf {
    gtk::glib::user_config_dir().join("lljson")
}

fn data_dir() -> PathBuf {
    gtk::glib::user_data_dir().join("lljson")
}

pub fn load_scheme() -> Scheme {
    read_scheme(&config_dir())
}

pub fn save_scheme(scheme: Scheme) -> io::Result<()> {
    write_scheme(&config_dir(), scheme)
}

/// The document saved for `side`, ready to load again.
pub fn restore(side: Side) -> Option<Input> {
    read_pane(&data_dir(), side.name())
}

/// Saves a copy of the pane's source text; called from a background thread.
pub fn save_pane(side: Side, name: &str, bytes: &[u8]) -> io::Result<()> {
    write_pane(&data_dir(), side.name(), name, bytes)
}

fn read_scheme(dir: &Path) -> Scheme {
    fs::read_to_string(dir.join("color-scheme")).map_or(Scheme::System, |t| Scheme::parse(&t))
}

fn write_scheme(dir: &Path, scheme: Scheme) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    write_atomic(&dir.join("color-scheme"), scheme.as_str().as_bytes())
}

fn read_pane(dir: &Path, side: &str) -> Option<Input> {
    let text = fs::read_to_string(dir.join(format!("{side}.json"))).ok()?;
    let name = fs::read_to_string(dir.join(format!("{side}.name")))
        .ok()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Last session".to_owned());
    Some(Input::Text { name, text })
}

fn write_pane(dir: &Path, side: &str, name: &str, bytes: &[u8]) -> io::Result<()> {
    let text_path = dir.join(format!("{side}.json"));
    if bytes.len() > MAX_SAVED_BYTES {
        // A stale copy would restore a different document than the one on screen.
        return match fs::remove_file(&text_path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    fs::create_dir_all(dir)?;
    write_atomic(&dir.join(format!("{side}.name")), name.as_bytes())?;
    write_atomic(&text_path, bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lljson-session-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn scheme_round_trips() {
        let dir = temp_dir("scheme");
        for scheme in [Scheme::Light, Scheme::Dark, Scheme::System] {
            write_scheme(&dir, scheme).unwrap();
            assert_eq!(read_scheme(&dir), scheme);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_or_garbage_scheme_is_system() {
        let dir = temp_dir("garbage");
        assert_eq!(read_scheme(&dir), Scheme::System);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("color-scheme"), "purple").unwrap();
        assert_eq!(read_scheme(&dir), Scheme::System);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pane_round_trips() {
        let dir = temp_dir("pane");
        write_pane(&dir, "left", "Pasted text", br#"{"a": 1}"#).unwrap();
        match read_pane(&dir, "left") {
            Some(Input::Text { name, text }) => {
                assert_eq!(name, "Pasted text");
                assert_eq!(text, r#"{"a": 1}"#);
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(read_pane(&dir, "right").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversized_document_drops_the_stale_copy() {
        let dir = temp_dir("big");
        write_pane(&dir, "left", "small", b"[1]").unwrap();
        let big = vec![b' '; MAX_SAVED_BYTES + 1];
        write_pane(&dir, "left", "big", &big).unwrap();
        assert!(read_pane(&dir, "left").is_none());
        // Removing a copy that never existed is not an error.
        write_pane(&dir, "right", "big", &big).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
