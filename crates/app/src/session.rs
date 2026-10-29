//! What survives between runs: the theme and the last JSON loaded in each pane.
//! Plain files under the user's config/data directories; independent of GTK widgets.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::loader::Input;
use crate::pane::Side;
use crate::theme;

/// Larger documents are not restored: writing them would cost more than it helps.
const MAX_SAVED_BYTES: usize = 32 * 1024 * 1024;

fn config_dir() -> PathBuf {
    gtk::glib::user_config_dir().join("lljson")
}

fn data_dir() -> PathBuf {
    gtk::glib::user_data_dir().join("lljson")
}

pub fn load_theme() -> String {
    read_theme(&config_dir())
}

pub fn save_theme(id: &str) -> io::Result<()> {
    write_theme(&config_dir(), id)
}

/// The document saved for `side`, ready to load again.
pub fn restore(side: Side) -> Option<Input> {
    read_pane(&data_dir(), side.name())
}

/// Saves a copy of the pane's source text; called from a background thread.
pub fn save_pane(side: Side, name: &str, bytes: &[u8]) -> io::Result<()> {
    write_pane(&data_dir(), side.name(), name, bytes)
}

/// The saved theme id; unknown ids are resolved by `theme::by_id`.
fn read_theme(dir: &Path) -> String {
    fs::read_to_string(dir.join("theme"))
        .map(|t| t.trim().to_owned())
        .unwrap_or_else(|_| theme::DEFAULT_ID.to_owned())
}

fn write_theme(dir: &Path, id: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    write_atomic(&dir.join("theme"), id.as_bytes())
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
    fn theme_round_trips() {
        let dir = temp_dir("theme");
        write_theme(&dir, "synthwave").unwrap();
        assert_eq!(read_theme(&dir), "synthwave");
        write_theme(&dir, "dark").unwrap();
        assert_eq!(read_theme(&dir), "dark");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_theme_is_the_default() {
        let dir = temp_dir("missing");
        assert_eq!(read_theme(&dir), theme::DEFAULT_ID);
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
