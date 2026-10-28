//! Carga de documentos en un hilo de fondo: lectura de disco o stdin y parseo.
//! El resultado vuelve al hilo de la UI por un `async_channel`. No depende de GTK.

use std::fmt;
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonlens_core::{Document, Mode, ParseError, ParseOptions, Source, parse};

/// Origen de un documento.
#[derive(Debug)]
pub enum Input {
    File(PathBuf),
    Stdin,
    /// Texto ya en memoria (portapapeles, arrastrar texto). `name` se muestra en el panel.
    Text {
        name: String,
        text: String,
    },
}

impl Input {
    /// Nombre para mostrar en el panel y en los errores.
    pub fn name(&self) -> String {
        match self {
            Input::File(path) => path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into(),
            ),
            Input::Stdin => "Entrada estándar".to_owned(),
            Input::Text { name, .. } => name.clone(),
        }
    }
}

/// Documento cargado.
#[derive(Debug)]
pub struct Loaded {
    pub name: String,
    /// Compartido con el hilo que construye las filas de la vista.
    pub document: Arc<Document>,
    /// Comentarios y comas finales aceptados en modo tolerante.
    pub fixes: usize,
    /// Tiempo de lectura más parseo.
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
    /// El hilo de fondo no pudo crearse o terminó sin enviar el resultado.
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
            LoadError::Io { source, .. } => write!(f, "No se pudo leer: {source}"),
            LoadError::Parse { error, .. } => write!(f, "JSON no válido en la {error}"),
            LoadError::Interrupted { .. } => write!(f, "La carga se interrumpió inesperadamente"),
        }
    }
}

impl std::error::Error for LoadError {}

pub type LoadResult = Result<Loaded, LoadError>;

/// Lee y parsea en el hilo actual (bloqueante). Se acepta JSONC (modo tolerante).
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

/// Carga en un hilo de fondo. El receptor entrega exactamente un resultado.
pub fn spawn_load(input: Input) -> async_channel::Receiver<LoadResult> {
    let (sender, receiver) = async_channel::bounded(1);
    let name = input.name();
    let worker = sender.clone();
    let spawned = std::thread::Builder::new()
        .name("jsonlens-carga".to_owned())
        .spawn(move || {
            // Si el receptor ya no existe (panel cerrado), el resultado se descarta.
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
            name: "prueba".to_owned(),
            text: t.to_owned(),
        }
    }

    #[test]
    fn loads_text() {
        let loaded = load(text(r#"{"a":[1,2]}"#)).expect("cargar");
        assert_eq!(loaded.name, "prueba");
        assert_eq!(loaded.document.node_count(), 4);
        assert_eq!(loaded.fixes, 0);
    }

    #[test]
    fn accepts_jsonc() {
        let loaded = load(text("[1, // uno\n 2,]")).expect("cargar");
        assert_eq!(loaded.fixes, 2);
    }

    #[test]
    fn reports_parse_errors_with_position() {
        let err = load(text("{\n  \"a\": }")).expect_err("debe fallar");
        assert!(matches!(err, LoadError::Parse { .. }));
        assert_eq!(err.name(), "prueba");
        assert!(err.to_string().contains("línea 2"), "{err}");
    }

    #[test]
    fn reports_missing_files() {
        let err = load(Input::File("/no/existe.json".into())).expect_err("debe fallar");
        assert!(matches!(err, LoadError::Io { .. }));
        assert_eq!(err.name(), "existe.json");
    }

    #[test]
    fn loads_files() {
        let path = std::env::temp_dir().join(format!("jsonlens-app-{}.json", std::process::id()));
        std::fs::write(&path, "[true]").expect("escribir");
        let loaded = load(Input::File(path.clone())).expect("cargar");
        assert_eq!(loaded.document.node_count(), 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn spawn_load_delivers_one_result() {
        let receiver = spawn_load(text("[1,2,3]"));
        let loaded = receiver
            .recv_blocking()
            .expect("resultado")
            .expect("cargar");
        assert_eq!(loaded.document.node_count(), 4);
        assert!(receiver.recv_blocking().is_err(), "solo un resultado");
    }
}
