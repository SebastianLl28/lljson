use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use memmap2::Mmap;

/// Files at least this large are memory-mapped instead of copied.
const MMAP_THRESHOLD: u64 = 1024 * 1024;

/// Original document bytes; nodes store spans over them.
#[derive(Debug)]
pub enum Source {
    Owned(Vec<u8>),
    Mapped(Mmap),
}

impl Source {
    pub fn from_file(path: &Path) -> io::Result<Source> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        if len < MMAP_THRESHOLD {
            let mut bytes = Vec::with_capacity(len as usize);
            file.read_to_end(&mut bytes)?;
            return Ok(Source::Owned(bytes));
        }
        // SAFETY: the map is read-only and lives inside `Source`. If another process
        // modifies or truncates the file while mapped, contents may change or access may
        // fail; this is the risk accepted in SPEC.md (mandatory architecture, point 7)
        // to avoid copying huge files.
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
        std::fs::write(&path, bytes).expect("write temp file");
        path
    }

    #[test]
    fn small_file_is_copied() {
        let path = temp_file("small.json", b"[1,2]");
        let source = Source::from_file(&path).expect("open file");
        assert!(matches!(source, Source::Owned(_)));
        assert_eq!(source.as_bytes(), b"[1,2]");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn large_file_is_mapped() {
        let bytes = vec![b' '; MMAP_THRESHOLD as usize + 1];
        let path = temp_file("large.json", &bytes);
        let source = Source::from_file(&path).expect("open file");
        assert!(matches!(source, Source::Mapped(_)));
        assert_eq!(source.as_bytes().len(), bytes.len());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(Source::from_file(Path::new("/does/not/exist/lljson.json")).is_err());
    }
}
