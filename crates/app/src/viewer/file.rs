//! Background reading of a text file for the Viewer. Independent of GTK.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;

use super::prepare::prepare;

const BOM: &[u8] = b"\xEF\xBB\xBF";

#[derive(Debug)]
pub enum ReadError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    NotUtf8,
    /// The worker thread could not be created or ended without sending a result.
    Interrupted,
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::Io { path, source } => {
                let name = path.file_name().unwrap_or(path.as_os_str());
                write!(f, "Could not read {}: {source}", name.to_string_lossy())
            }
            ReadError::NotUtf8 => write!(f, "The file is not valid UTF-8 text"),
            ReadError::Interrupted => write!(f, "Reading was interrupted unexpectedly"),
        }
    }
}

impl std::error::Error for ReadError {}

pub type ReadResult = Result<String, ReadError>;

/// UTF-8 text without a leading byte order mark.
pub fn decode(mut bytes: Vec<u8>) -> Result<String, FromUtf8Error> {
    if bytes.starts_with(BOM) {
        bytes.drain(..BOM.len());
    }
    String::from_utf8(bytes)
}

/// Blocking read on the current thread.
pub fn read_text(path: &Path) -> ReadResult {
    let bytes = std::fs::read(path).map_err(|source| ReadError::Io {
        path: path.to_owned(),
        source,
    })?;
    decode(bytes).map_err(|_| ReadError::NotUtf8)
}

/// Reads the file on a background thread and prepares the text for the editor (valid JSON
/// is pretty-printed); the receiver yields exactly one result.
pub fn spawn_read(path: PathBuf) -> async_channel::Receiver<ReadResult> {
    spawn(move || read_text(&path).map(prepare))
}

/// Prepares text that is already in memory (dropped text) on a background thread.
pub fn spawn_prepare(text: String) -> async_channel::Receiver<ReadResult> {
    spawn(move || Ok(prepare(text)))
}

fn spawn(job: impl FnOnce() -> ReadResult + Send + 'static) -> async_channel::Receiver<ReadResult> {
    let (sender, receiver) = async_channel::bounded(1);
    let worker = sender.clone();
    let spawned = std::thread::Builder::new()
        .name("lljson-read".to_owned())
        .spawn(move || {
            // A dropped receiver (stale request) just discards the result.
            let _ = worker.send_blocking(job());
        });
    if spawned.is_err() {
        let _ = sender.try_send(Err(ReadError::Interrupted));
    }
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp(name: &str, bytes: &[u8]) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("lljson-viewer-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).expect("write");
        path
    }

    #[test]
    fn decode_strips_bom() {
        assert_eq!(decode(b"\xEF\xBB\xBF{}".to_vec()).expect("decode"), "{}");
    }

    #[test]
    fn decode_keeps_plain_text() {
        assert_eq!(decode(b"[1, 2]".to_vec()).expect("decode"), "[1, 2]");
    }

    #[test]
    fn decode_rejects_invalid_utf8() {
        assert!(decode(vec![0xFF, 0xFE, b'a']).is_err());
    }

    #[test]
    fn read_text_reports_missing_file() {
        let err = read_text(Path::new("/no/such/viewer.json")).expect_err("should fail");
        assert!(matches!(err, ReadError::Io { .. }));
        assert_eq!(
            err.to_string(),
            "Could not read viewer.json: No such file or directory (os error 2)"
        );
    }

    #[test]
    fn read_text_reads_empty_file() {
        let path = temp("empty.json", b"");
        assert_eq!(read_text(&path).expect("read"), "");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn read_text_reports_non_utf8_file() {
        let path = temp("binary.json", &[0xFF, 0xFE, 0x00]);
        let err = read_text(&path).expect_err("should fail");
        assert!(matches!(err, ReadError::NotUtf8));
        assert_eq!(err.to_string(), "The file is not valid UTF-8 text");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn spawn_read_delivers_one_result() {
        let path = temp("one.json", b"[true]");
        let receiver = spawn_read(path.clone());
        assert_eq!(
            receiver.recv_blocking().expect("result").expect("read"),
            "[\n  true\n]"
        );
        assert!(receiver.recv_blocking().is_err(), "only one result");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn spawn_read_formats_valid_json() {
        let path = temp("minified.json", br#"{"a":1}"#);
        let text = spawn_read(path.clone())
            .recv_blocking()
            .expect("result")
            .expect("read");
        assert_eq!(text, "{\n  \"a\": 1\n}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn spawn_prepare_formats_text_and_keeps_invalid_text() {
        let formatted = spawn_prepare("[1]".to_owned())
            .recv_blocking()
            .expect("result")
            .expect("prepared");
        assert_eq!(formatted, "[\n  1\n]");
        let kept = spawn_prepare("[1".to_owned())
            .recv_blocking()
            .expect("result")
            .expect("prepared");
        assert_eq!(kept, "[1");
    }
}
