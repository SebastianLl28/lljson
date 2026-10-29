//! `lljson-cli`: semantic diff, format, minify and test JSON generation.
//! A file named `-` is read from standard input.

use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use lljson_core::{
    Change, ChangeKind, Changes, DiffOptions, Document, FormatOptions, GenerateOptions, IgnoreRule,
    Indent, Mode, NodeId, ParseOptions, Source, Summary, diff, format, format_node, generate,
    parse,
};

const PREVIEW_CHARS: usize = 120;

#[derive(Parser)]
#[command(
    name = "lljson-cli",
    version,
    about = "Semantic JSON diff and formatting"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compare two documents (exit 0 if equal, 1 if different, 2 on error)
    Diff(DiffArgs),
    /// Format a document with indentation
    Format(FormatArgs),
    /// Minify a document
    Minify(MinifyArgs),
    /// Generate a deterministic test document (array of 12-node records)
    ///
    /// With the same seed, `--modify`/`--type-change`/`--remove`/`--add` produce a
    /// variant with exactly those changes (compare with `--array-key id`).
    Generate(GenerateArgs),
}

#[derive(Args)]
struct InputArgs {
    /// Accept `//` and `/* */` comments and trailing commas (JSONC)
    #[arg(long)]
    tolerant: bool,
}

#[derive(Args)]
struct DiffArgs {
    /// Left document (`-` for standard input)
    left: PathBuf,
    /// Right document (`-` for standard input)
    right: PathBuf,
    /// Match array elements by this key (e.g. `id`)
    #[arg(long, value_name = "KEY")]
    array_key: Option<String>,
    /// Key (`timestamp`) or path (`$.users[*].updatedAt`) to skip; repeatable
    #[arg(long, value_name = "RULE")]
    ignore: Vec<String>,
    /// Print nothing; only set the exit code
    #[arg(short, long)]
    quiet: bool,
    #[command(flatten)]
    input: InputArgs,
}

#[derive(Args)]
struct FormatArgs {
    /// Document (`-` or omitted for standard input)
    #[arg(default_value = "-")]
    file: PathBuf,
    /// Spaces per level
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(0..=16))]
    indent: u8,
    /// Indent with tabs
    #[arg(long, conflicts_with = "indent")]
    tab: bool,
    /// Sort the keys of every object
    #[arg(long)]
    sort_keys: bool,
    #[command(flatten)]
    input: InputArgs,
}

#[derive(Args)]
struct MinifyArgs {
    /// Document (`-` or omitted for standard input)
    #[arg(default_value = "-")]
    file: PathBuf,
    /// Sort the keys of every object
    #[arg(long)]
    sort_keys: bool,
    #[command(flatten)]
    input: InputArgs,
}

#[derive(Args)]
struct GenerateArgs {
    /// Approximate node count of the base document (rounded to whole records)
    #[arg(long, default_value_t = 1_000_000)]
    nodes: usize,
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// Records with a modified `name`
    #[arg(long, default_value_t = 0, value_name = "N")]
    modify: usize,
    /// Records whose `active` changes from boolean to string
    #[arg(long, default_value_t = 0, value_name = "N")]
    type_change: usize,
    /// Records removed
    #[arg(long, default_value_t = 0, value_name = "N")]
    remove: usize,
    /// New records inserted
    #[arg(long, default_value_t = 0, value_name = "N")]
    add: usize,
    /// No whitespace (default: indented with 2 spaces)
    #[arg(long)]
    compact: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(code) => code,
        Err(err) if is_broken_pipe(&err) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("lljson-cli: {err:#}");
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
        Command::Generate(args) => {
            let options = GenerateOptions {
                nodes: args.nodes,
                seed: args.seed,
                indent: if args.compact {
                    Indent::None
                } else {
                    Indent::default()
                },
                changes: Changes {
                    modified: args.modify,
                    type_changed: args.type_change,
                    removed: args.remove,
                    added: args.add,
                },
            };
            writeln!(out, "{}", generate(&options)?)?;
            ExitCode::SUCCESS
        }
    };
    out.flush()?;
    Ok(code)
}

fn run_diff(args: &DiffArgs, out: &mut impl Write) -> Result<ExitCode> {
    if is_stdin(&args.left) && is_stdin(&args.right) {
        bail!("only one of the two documents can be read from standard input");
    }
    let ignore = args
        .ignore
        .iter()
        .map(|rule| IgnoreRule::parse(rule).with_context(|| format!("--ignore rule {rule:?}")))
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
            .unwrap_or_else(|_| Err(anyhow::anyhow!("the parse thread panicked")));
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
            .context("could not read standard input")?;
        Source::Owned(bytes)
    } else {
        Source::from_file(path).with_context(|| format!("could not open {}", path.display()))?
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

/// One line per change: `+` added, `-` removed, `~` modified, `!` type change.
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
        // The diff engine always fills in the nodes that each kind requires.
        _ => bail!("change without nodes: {change:?}"),
    }
    Ok(())
}

/// Minified value truncated to [`PREVIEW_CHARS`] characters.
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
        return "No differences".to_owned();
    }
    let mut text = format!(
        "{}, {}, {}",
        plural(summary.added, "added", "added"),
        plural(summary.removed, "removed", "removed"),
        plural(summary.modified, "modified", "modified"),
    );
    if summary.type_changed > 0 {
        let types = plural(summary.type_changed, "type change", "type changes");
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

/// `lljson-cli … | head` closes the pipe early; that is not an error.
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
            .expect("valid JSON")
            .document
    }

    #[test]
    fn describe_counts() {
        assert_eq!(describe(Summary::default()), "No differences");
        let summary = Summary {
            added: 1,
            removed: 2,
            modified: 3,
            type_changed: 1,
        };
        assert_eq!(
            describe(summary),
            "1 added, 2 removed, 3 modified (1 type change)"
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
            write_change(&mut out, &left, &right, change).expect("write");
        }
        let text = String::from_utf8(out).expect("UTF-8");
        assert!(text.contains("~ $.a: 1 → 2\n"), "{text}");
        assert!(text.contains("! $.b: \"1\" → 1\n"), "{text}");
        assert!(text.contains("- $.c: true\n"), "{text}");
        assert!(text.contains("+ $.d: null\n"), "{text}");
    }
}
