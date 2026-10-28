//! `jsonlens-cli`: diff semántico, formateo y minificado de JSON desde la terminal.
//! Un archivo `-` se lee de la entrada estándar.

use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use jsonlens_core::{
    Change, ChangeKind, DiffOptions, Document, FormatOptions, IgnoreRule, Indent, Mode, NodeId,
    ParseOptions, Source, Summary, diff, format, format_node, parse,
};

/// Caracteres máximos de un valor en la salida del diff.
const PREVIEW_CHARS: usize = 120;

#[derive(Parser)]
#[command(
    name = "jsonlens-cli",
    version,
    about = "Diff semántico y formateo de JSON"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compara dos documentos. Sale con 0 si son iguales, 1 si difieren y 2 si hay un error.
    Diff(DiffArgs),
    /// Formatea un documento con indentación.
    Format(FormatArgs),
    /// Minifica un documento (sin espacios ni saltos de línea).
    Minify(MinifyArgs),
}

#[derive(Args)]
struct InputArgs {
    /// Acepta comentarios `//` y `/* */` y comas finales (JSONC).
    #[arg(long)]
    tolerant: bool,
}

#[derive(Args)]
struct DiffArgs {
    /// Documento izquierdo (`-` para la entrada estándar).
    left: PathBuf,
    /// Documento derecho (`-` para la entrada estándar).
    right: PathBuf,
    /// Empareja los elementos de los arrays por esta clave (p. ej. `id`).
    #[arg(long, value_name = "CLAVE")]
    array_key: Option<String>,
    /// Clave (`timestamp`) o ruta (`$.users[*].updatedAt`) que no se compara. Repetible.
    #[arg(long, value_name = "REGLA")]
    ignore: Vec<String>,
    /// No escribe nada: solo el código de salida.
    #[arg(short, long)]
    quiet: bool,
    #[command(flatten)]
    input: InputArgs,
}

#[derive(Args)]
struct FormatArgs {
    /// Documento (`-` o vacío para la entrada estándar).
    #[arg(default_value = "-")]
    file: PathBuf,
    /// Espacios por nivel.
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(0..=16))]
    indent: u8,
    /// Indenta con tabuladores.
    #[arg(long, conflicts_with = "indent")]
    tab: bool,
    /// Ordena las claves de cada objeto.
    #[arg(long)]
    sort_keys: bool,
    #[command(flatten)]
    input: InputArgs,
}

#[derive(Args)]
struct MinifyArgs {
    /// Documento (`-` o vacío para la entrada estándar).
    #[arg(default_value = "-")]
    file: PathBuf,
    /// Ordena las claves de cada objeto.
    #[arg(long)]
    sort_keys: bool,
    #[command(flatten)]
    input: InputArgs,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(code) => code,
        Err(err) if is_broken_pipe(&err) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("jsonlens-cli: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run(command: Command) -> Result<ExitCode> {
    let stdout = io::stdout().lock();
    let mut out = BufWriter::new(stdout);
    let code = match command {
        Command::Diff(args) => run_diff(&args, &mut out)?,
        Command::Format(args) => {
            let indent = if args.tab {
                Indent::Tab
            } else {
                Indent::Spaces(args.indent)
            };
            let doc = load(&args.file, &args.input)?;
            write_document(&mut out, &doc, indent, args.sort_keys)?;
            ExitCode::SUCCESS
        }
        Command::Minify(args) => {
            let doc = load(&args.file, &args.input)?;
            write_document(&mut out, &doc, Indent::None, args.sort_keys)?;
            ExitCode::SUCCESS
        }
    };
    out.flush()?;
    Ok(code)
}

fn run_diff(args: &DiffArgs, out: &mut impl Write) -> Result<ExitCode> {
    if is_stdin(&args.left) && is_stdin(&args.right) {
        bail!("solo uno de los dos documentos puede leerse de la entrada estándar");
    }
    let ignore = args
        .ignore
        .iter()
        .map(|rule| IgnoreRule::parse(rule).with_context(|| format!("regla --ignore {rule:?}")))
        .collect::<Result<Vec<_>>>()?;
    let options = DiffOptions {
        array_key: args.array_key.clone(),
        ignore,
    };

    let (left, right) = std::thread::scope(|scope| {
        let right = scope.spawn(|| load(&args.right, &args.input));
        let left = load(&args.left, &args.input);
        let right = right
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("el hilo de parseo terminó con pánico")));
        (left, right)
    });
    let (left, right) = (left?, right?);

    let result = diff(&left, &right, &options);
    if !args.quiet {
        for change in result.changes() {
            write_change(out, &left, &right, change)?;
        }
        writeln!(out, "{}", describe(result.summary()))?;
    }
    Ok(if result.is_equal() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn load(path: &Path, input: &InputArgs) -> Result<Document> {
    let source = if is_stdin(path) {
        let mut bytes = Vec::new();
        io::stdin()
            .lock()
            .read_to_end(&mut bytes)
            .context("no se pudo leer la entrada estándar")?;
        Source::Owned(bytes)
    } else {
        Source::from_file(path).with_context(|| format!("no se pudo abrir {}", path.display()))?
    };
    let options = ParseOptions {
        mode: if input.tolerant {
            Mode::Tolerant
        } else {
            Mode::Strict
        },
    };
    let parsed = parse(source, &options).with_context(|| display_name(path))?;
    Ok(parsed.document)
}

fn write_document(
    out: &mut impl Write,
    doc: &Document,
    indent: Indent,
    sort_keys: bool,
) -> Result<()> {
    let text = format(doc, &FormatOptions { indent, sort_keys });
    writeln!(out, "{text}")?;
    Ok(())
}

/// Una línea por cambio: `+` añadido, `-` eliminado, `~` modificado, `!` cambio de tipo.
fn write_change(
    out: &mut impl Write,
    left: &Document,
    right: &Document,
    change: &Change,
) -> Result<()> {
    match (change.kind, change.left, change.right) {
        (ChangeKind::Added, _, Some(r)) => {
            writeln!(out, "+ {}: {}", right.path(r), preview(right, r))?;
        }
        (ChangeKind::Removed, Some(l), _) => {
            writeln!(out, "- {}: {}", left.path(l), preview(left, l))?;
        }
        (ChangeKind::Modified | ChangeKind::TypeChanged, Some(l), Some(r)) => {
            let mark = if change.kind == ChangeKind::Modified {
                '~'
            } else {
                '!'
            };
            let (before, after) = (preview(left, l), preview(right, r));
            writeln!(out, "{mark} {}: {before} → {after}", left.path(l))?;
        }
        // El motor de diff siempre rellena los nodos que corresponden a cada tipo.
        _ => bail!("cambio sin nodos: {change:?}"),
    }
    Ok(())
}

/// Valor minificado y recortado a [`PREVIEW_CHARS`] caracteres.
fn preview(doc: &Document, id: NodeId) -> String {
    let options = FormatOptions {
        indent: Indent::None,
        sort_keys: false,
    };
    let mut text = format_node(doc, id, &options);
    if let Some((cut, _)) = text.char_indices().nth(PREVIEW_CHARS) {
        text.truncate(cut);
        text.push('…');
    }
    text
}

fn describe(summary: Summary) -> String {
    if summary.total() == 0 {
        return "Sin diferencias".to_owned();
    }
    let mut text = format!(
        "{}, {}, {}",
        plural(summary.added, "añadido", "añadidos"),
        plural(summary.removed, "eliminado", "eliminados"),
        plural(summary.modified, "modificado", "modificados"),
    );
    if summary.type_changed > 0 {
        let types = plural(summary.type_changed, "cambio de tipo", "cambios de tipo");
        text.push_str(&format!(" ({types})"));
    }
    text
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn is_stdin(path: &Path) -> bool {
    path == Path::new("-")
}

fn display_name(path: &Path) -> String {
    if is_stdin(path) {
        "<stdin>".to_owned()
    } else {
        path.display().to_string()
    }
}

/// `jsonlens-cli … | head` cierra la tubería antes de tiempo: no es un error.
fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain()
        .filter_map(|cause| cause.downcast_ref::<io::Error>())
        .any(|e| e.kind() == io::ErrorKind::BrokenPipe)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        parse(text, &ParseOptions::default())
            .expect("JSON válido")
            .document
    }

    #[test]
    fn describe_counts() {
        assert_eq!(describe(Summary::default()), "Sin diferencias");
        let summary = Summary {
            added: 1,
            removed: 2,
            modified: 3,
            type_changed: 1,
        };
        assert_eq!(
            describe(summary),
            "1 añadido, 2 eliminados, 3 modificados (1 cambio de tipo)"
        );
    }

    #[test]
    fn preview_is_truncated_on_char_boundary() {
        let long = format!("[\"{}\"]", "ñ".repeat(500));
        let d = doc(&long);
        let text = preview(&d, d.root());
        assert_eq!(text.chars().count(), PREVIEW_CHARS + 1);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn preview_is_minified() {
        let d = doc("{ \"a\" : [ 1 , 2 ] }");
        assert_eq!(preview(&d, d.root()), r#"{"a":[1,2]}"#);
    }

    #[test]
    fn change_lines() {
        let left = doc(r#"{"a":1,"b":"1","c":true}"#);
        let right = doc(r#"{"a":2,"b":1,"d":null}"#);
        let result = diff(&left, &right, &DiffOptions::default());
        let mut out = Vec::new();
        for change in result.changes() {
            write_change(&mut out, &left, &right, change).expect("escribir");
        }
        let text = String::from_utf8(out).expect("UTF-8");
        assert!(text.contains("~ $.a: 1 → 2\n"), "{text}");
        assert!(text.contains("! $.b: \"1\" → 1\n"), "{text}");
        assert!(text.contains("- $.c: true\n"), "{text}");
        assert!(text.contains("+ $.d: null\n"), "{text}");
    }
}
