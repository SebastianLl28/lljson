//! Núcleo de jsonlens: parser, árbol en arena, hash, formateador y diff.
//! Sin dependencias de UI.

/// Versión de la librería.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_package() {
        assert_eq!(version(), "0.1.0");
    }
}
