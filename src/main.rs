use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

use docrev::adapter::json_comment_store::{self, JsonCommentStore};
use docrev::adapter::terminal_frontend::TerminalFrontend;
use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::comments;
use docrev::app::dump::dump;
use docrev::app::viewer::{self, Viewer};
use docrev::infra::terminal;
use docrev::ui::theme::Theme;
use docrev::ui::{comment_list, table};

#[derive(Parser)]
#[command(name = "docrev", version, about)]
struct Cli {
    /// Open a document (.xlsx) in the TUI viewer
    file: Option<PathBuf>,
    /// Viewer colors: `sheets` or `terminal` [env: DOCREV_THEME]
    #[arg(long, value_parser = parse_theme)]
    theme: Option<Theme>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Print a sheet as a plain-text table
    Dump {
        /// Path to the document (.xlsx)
        file: PathBuf,
        /// Sheet name to print (defaults to the first sheet)
        #[arg(long)]
        sheet: Option<String>,
    },
    /// Read and write review comments (built for AI agents)
    Comment {
        #[command(subcommand)]
        action: CommentAction,
    },
}

#[derive(Subcommand)]
enum CommentAction {
    /// List comment threads
    List {
        file: PathBuf,
        /// Machine-readable output (sidecar schema)
        #[arg(long)]
        json: bool,
        /// Only threads that are not resolved
        #[arg(long)]
        unresolved: bool,
        /// Only threads whose root comment is by this author
        #[arg(long)]
        author: Option<String>,
        /// Only threads on this sheet
        #[arg(long)]
        sheet: Option<String>,
    },
    /// Start a new thread on a cell
    Add {
        file: PathBuf,
        /// Target cell, e.g. "Sheet1!B3"
        #[arg(long)]
        cell: String,
        #[arg(long)]
        body: String,
        #[arg(long, default_value = "agent")]
        author: String,
    },
    /// Reply to an existing thread
    Reply {
        file: PathBuf,
        /// Thread id (from `comment list --json`)
        #[arg(long)]
        thread: String,
        #[arg(long)]
        body: String,
        #[arg(long, default_value = "agent")]
        author: String,
    },
    /// Mark a thread as resolved
    Resolve {
        file: PathBuf,
        /// Thread id (from `comment list --json`)
        #[arg(long)]
        thread: String,
    },
}

fn parse_theme(value: &str) -> Result<Theme, String> {
    value
        .parse()
        .map_err(|e: docrev::ui::theme::UnknownTheme| e.to_string())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // an explicit flag wins over the environment, which wins over the default
    let theme = cli.theme.unwrap_or_else(Theme::from_env);
    match (cli.command, cli.file) {
        (Some(Command::Dump { file, sheet }), _) => run_dump(&file, sheet.as_deref()),
        (Some(Command::Comment { action }), _) => run_comment(action),
        (None, Some(file)) => run_viewer(&file, theme),
        (None, None) => {
            let _ = Cli::command().print_help();
            ExitCode::FAILURE
        }
    }
}

fn run_comment(action: CommentAction) -> ExitCode {
    let document = match &action {
        CommentAction::List { file, .. }
        | CommentAction::Add { file, .. }
        | CommentAction::Reply { file, .. }
        | CommentAction::Resolve { file, .. } => file.clone(),
    };
    if !document.exists() {
        return fail(&format!("document not found: {}", document.display()));
    }
    let mut store = JsonCommentStore::for_document(&document);
    match action {
        CommentAction::List {
            json,
            unresolved,
            author,
            sheet,
            ..
        } => {
            let filter = comments::Filter {
                unresolved_only: unresolved,
                author: author.as_deref(),
                sheet: sheet.as_deref(),
            };
            if json {
                // agents get each thread's cell content attached, so a batch
                // of comments is actionable without reading the sheets
                return match comments::list_with_context(&XlsxSource, &store, &document, &filter) {
                    Ok(items) => {
                        print_json(json_comment_store::threads_with_context_to_json(&items))
                    }
                    Err(e) => fail(&e),
                };
            }
            match comments::list(&store, &filter) {
                Ok(threads) => print_stdout(&comment_list::render(&threads)),
                Err(e) => fail(&e),
            }
        }
        CommentAction::Add {
            cell, body, author, ..
        } => match comments::add(&XlsxSource, &mut store, &document, &cell, &body, &author) {
            Ok(thread) => print_json(json_comment_store::thread_to_json(&thread)),
            Err(e) => fail(&e),
        },
        CommentAction::Reply {
            thread,
            body,
            author,
            ..
        } => match comments::reply(&mut store, &thread, &body, &author) {
            Ok(thread) => print_json(json_comment_store::thread_to_json(&thread)),
            Err(e) => fail(&e),
        },
        CommentAction::Resolve { thread, .. } => match comments::resolve(&mut store, &thread) {
            Ok(thread) => print_json(json_comment_store::thread_to_json(&thread)),
            Err(e) => fail(&e),
        },
    }
}

fn print_json(rendered: Result<String, docrev::app::error::StoreError>) -> ExitCode {
    match rendered {
        Ok(json) => print_stdout(&format!("{json}\n")),
        Err(e) => fail(&e),
    }
}

fn run_dump(file: &Path, sheet: Option<&str>) -> ExitCode {
    match dump(&XlsxSource, file, sheet) {
        Ok(view) => print_stdout(&table::render(&view.sheet, view.position, view.total)),
        Err(e) => fail(&e),
    }
}

/// All stdout goes through here: a downstream pipe closing early
/// (e.g. `| head`, `| jq -e`) is not an error.
fn print_stdout(text: &str) -> ExitCode {
    match io::stdout().write_all(text.as_bytes()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn run_viewer(file: &Path, theme: Theme) -> ExitCode {
    let store = JsonCommentStore::for_document(file);
    let viewer = match Viewer::open(&XlsxSource, Box::new(store), file) {
        Ok(viewer) => viewer,
        Err(e) => return fail(&e),
    };
    let terminal = match terminal::init() {
        Ok(terminal) => terminal,
        Err(e) => return fail(&e),
    };
    let result = viewer::run(viewer, &mut TerminalFrontend::new(terminal, theme));
    terminal::restore();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn fail(error: &dyn std::fmt::Display) -> ExitCode {
    eprintln!("error: {error}");
    ExitCode::FAILURE
}
