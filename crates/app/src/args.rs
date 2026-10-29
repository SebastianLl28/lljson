//! Argumentos de terminal: `lljson [IZQUIERDO] [DERECHO]`, con `-` para stdin.

use std::path::PathBuf;

use clap::Parser;

use crate::loader::Input;

#[derive(Debug, Parser)]
#[command(name = "lljson", version, about = "Compara y formatea JSON")]
pub struct Args {
    /// Documento del panel izquierdo (`-` para la entrada estándar).
    #[arg(value_name = "IZQUIERDO")]
    pub left: Option<PathBuf>,
    /// Documento del panel derecho (`-` para la entrada estándar).
    #[arg(value_name = "DERECHO")]
    pub right: Option<PathBuf>,
}

impl Args {
    /// Documentos a cargar en cada panel. Falla si los dos piden la entrada estándar.
    pub fn into_inputs(self) -> Result<(Option<Input>, Option<Input>), String> {
        let is_stdin = |p: &Option<PathBuf>| p.as_deref() == Some("-".as_ref());
        if is_stdin(&self.left) && is_stdin(&self.right) {
            return Err(
                "solo uno de los dos documentos puede leerse de la entrada estándar".into(),
            );
        }
        Ok((self.left.map(input), self.right.map(input)))
    }
}

fn input(path: PathBuf) -> Input {
    if path.as_os_str() == "-" {
        Input::Stdin
    } else {
        Input::File(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(args: &[&str]) -> Result<(Option<Input>, Option<Input>), String> {
        let args = Args::try_parse_from(std::iter::once("lljson").chain(args.iter().copied()))
            .map_err(|e| e.to_string())?;
        args.into_inputs()
    }

    #[test]
    fn no_arguments() {
        let (left, right) = inputs(&[]).expect("válido");
        assert!(left.is_none() && right.is_none());
    }

    #[test]
    fn two_files() {
        let (left, right) = inputs(&["a.json", "b.json"]).expect("válido");
        assert!(matches!(left, Some(Input::File(p)) if p.as_os_str() == "a.json"));
        assert!(matches!(right, Some(Input::File(p)) if p.as_os_str() == "b.json"));
    }

    #[test]
    fn one_file() {
        let (left, right) = inputs(&["a.json"]).expect("válido");
        assert!(matches!(left, Some(Input::File(_))));
        assert!(right.is_none());
    }

    #[test]
    fn stdin_on_either_side() {
        let (left, right) = inputs(&["-", "b.json"]).expect("válido");
        assert!(matches!(left, Some(Input::Stdin)));
        assert!(matches!(right, Some(Input::File(_))));
        let (_, right) = inputs(&["a.json", "-"]).expect("válido");
        assert!(matches!(right, Some(Input::Stdin)));
    }

    #[test]
    fn stdin_twice_is_an_error() {
        assert!(inputs(&["-", "-"]).is_err());
    }

    #[test]
    fn too_many_arguments_is_an_error() {
        assert!(inputs(&["a", "b", "c"]).is_err());
    }
}
