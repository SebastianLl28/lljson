//! Texto de entrada de un documento: en memoria o mapeado desde disco.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use memmap2::Mmap;

/// A partir de este tamaño los archivos se mapean en memoria en lugar de copiarse.
const MMAP_THRESHOLD: u64 = 1024 * 1024;

/// Bytes originales de un documento. Los nodos guardan rangos sobre ellos.
#[derive(Debug)]
pub enum Source {
    Owned(Vec<u8>),
    Mapped(Mmap),
}

impl Source {
    /// Abre un archivo. Los grandes se leen con `memmap2`, los pequeños se copian.
    pub fn from_file(path: &Path) -> io::Result<Source> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        if len < MMAP_THRESHOLD {
            let mut bytes = Vec::with_capacity(len as usize);
            file.read_to_end(&mut bytes)?;
            return Ok(Source::Owned(bytes));
        }
        // SAFETY: el mapa es de solo lectura y vive dentro de `Source`. Si otro
        // proceso modifica o trunca el archivo mientras está mapeado, el contenido
        // puede cambiar o el acceso fallar; es el riesgo aceptado en SPEC.md
        // (arquitectura obligatoria, punto 7) a cambio de no copiar archivos enormes.
        let map = unsafe { Mmap::map(&file)? };
        Ok(Source::Mapped(map))
    }

    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Source::Owned(bytes) => bytes,
            Source::Mapped(map) => map,
        }
    }
}

impl From<Vec<u8>> for Source {
    fn from(bytes: Vec<u8>) -> Source {
        Source::Owned(bytes)
    }
}

impl From<String> for Source {
    fn from(text: String) -> Source {
        Source::Owned(text.into_bytes())
    }
}

impl From<&str> for Source {
    fn from(text: &str) -> Source {
        Source::Owned(text.as_bytes().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("lljson-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).expect("escribir archivo temporal");
        path
    }

    #[test]
    fn small_file_is_copied() {
        let path = temp_file("small.json", b"[1,2]");
        let source = Source::from_file(&path).expect("abrir archivo");
        assert!(matches!(source, Source::Owned(_)));
        assert_eq!(source.as_bytes(), b"[1,2]");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn large_file_is_mapped() {
        let bytes = vec![b' '; MMAP_THRESHOLD as usize + 1];
        let path = temp_file("large.json", &bytes);
        let source = Source::from_file(&path).expect("abrir archivo");
        assert!(matches!(source, Source::Mapped(_)));
        assert_eq!(source.as_bytes().len(), bytes.len());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(Source::from_file(Path::new("/no/existe/lljson.json")).is_err());
    }
}
