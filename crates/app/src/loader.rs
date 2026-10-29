//! Background document loading (disk or stdin read plus parse), delivered over an
//! `async_channel`. Independent of GTK.

use std::fmt;
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lljson_core::{Document, Mode, ParseError, ParseOptions, Source, parse};

#[derive(Debug)]
pub enum Input {
    File(PathBuf),
    Stdin,
    /// Text already in memory (clipboard, dropped text); `name` is shown in the pane.
    Text {
        name: String,
        text: String,
    },
}

impl Input {
    pub fn name(&self) -> String {
        match self {
            Input::File(path) => path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into(),
            ),
            Input::Stdin => "Standard input".to_owned(),
            Input::Text { name, .. } => name.clone(),
        }
    }
}

#[derive(Debug)]
pub struct Loaded {
    pub name: String,
    /// Shared with the thread that builds the view rows.
    pub document: Arc<Document>,
    /// Comments and trailing commas accepted in tolerant mode.
    pub fixes: usize,
    /// Read plus parse time.
    pub elapsed: Duration,
}

#[derive(Debug)]
pub enum LoadError {
    Io {
        name: String,
        source: io::Error,
    },
    Parse {
        name: String,
        error: ParseError,
    },
    /// The worker thread could not be created or ended without sending a result.
    Interrupted {
        name: String,
    },
}

impl LoadError {
    pub fn name(&self) -> &str {
        match self {
            LoadError::Io { name, .. }
            | LoadError::Parse { name, .. }
            | LoadError::Interrupted { name } => name,
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io { source, .. } => write!(f, "Could not read: {source}"),
            LoadError::Parse { error, .. } => write!(f, "Invalid JSON at {error}"),
            LoadError::Interrupted { .. } => write!(f, "Loading was interrupted unexpectedly"),
        }
    }
}

impl std::error::Error for LoadError {}

pub type LoadResult = Result<Loaded, LoadError>;

/// Blocking read and parse on the current thread; accepts JSONC (tolerant mode).
pub fn load(input: Input) -> LoadResult {
    let start = Instant::now();
    let name = input.name();
    let source = match input {
        Input::File(path) => Source::from_file(&path),
        Input::Stdin => {
            let mut bytes = Vec::new();
            io::stdin()
                .lock()
                .read_to_end(&mut bytes)
                .map(|_| Source::Owned(bytes))
        }
        Input::Text { text, .. } => Ok(Source::from(text)),
    };
    let source = source.map_err(|source| LoadError::Io {
        name: name.clone(),
        source,
    })?;
    let options = ParseOptions {
        mode: Mode::Tolerant,
    };
    let parsed = parse(source, &options).map_err(|error| LoadError::Parse {
        name: name.clone(),
        error,
    })?;
    Ok(Loaded {
        name,
        fixes: parsed.fixes.len(),
        document: Arc::new(parsed.document),
        elapsed: start.elapsed(),
    })
}

/// Loads on a background thread; the receiver yields exactly one result.
pub fn spawn_load(input: Input) -> async_channel::Receiver<LoadResult> {
    let (sender, receiver) = async_channel::bounded(1);
    let name = input.name();
    let worker = sender.clone();
    let spawned = std::thread::Builder::new()
        .name("lljson-load".to_owned())
        .spawn(move || {
            // A dropped receiver (pane closed) just discards the result.
            let _ = worker.send_blocking(load(input));
        });
    if spawned.is_err() {
        let _ = sender.try_send(Err(LoadError::Interrupted { name }));
    }
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Input {
        Input::Text {
            name: "sample".to_owned(),
            text: t.to_owned(),
        }
    }

    #[test]
    fn loads_text() {
        let loaded = load(text(r#"{"a":[1,2]}"#)).expect("load");
        assert_eq!(loaded.name, "sample");
        assert_eq!(loaded.document.node_count(), 4);
        assert_eq!(loaded.fixes, 0);
    }

    #[test]
    fn accepts_jsonc() {
        let loaded = load(text("[1, // one\n 2,]")).expect("load");
        assert_eq!(loaded.fixes, 2);
    }

    #[test]
    fn reports_parse_errors_with_position() {
        let err = load(text("{\n  \"a\": }")).expect_err("should fail");
        assert!(matches!(err, LoadError::Parse { .. }));
        assert_eq!(err.name(), "sample");
        assert!(err.to_string().contains("line 2"), "{err}");
    }

    #[test]
    fn reports_missing_files() {
        let err = load(Input::File("/no/such.json".into())).expect_err("should fail");
        assert!(matches!(err, LoadError::Io { .. }));
        assert_eq!(err.name(), "such.json");
    }

    #[test]
    fn loads_files() {
        let path = std::env::temp_dir().join(format!("lljson-app-{}.json", std::process::id()));
        std::fs::write(&path, "[true]").expect("write");
        let loaded = load(Input::File(path.clone())).expect("load");
        assert_eq!(loaded.document.node_count(), 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn spawn_load_delivers_one_result() {
        let receiver = spawn_load(text("[1,2,3]"));
        let loaded = receiver.recv_blocking().expect("result").expect("load");
        assert_eq!(loaded.document.node_count(), 4);
        assert!(receiver.recv_blocking().is_err(), "only one result");
    }
}
