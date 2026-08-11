use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

use docrev::adapter::json_comment_store::JsonCommentStore;
use docrev::adapter::terminal_frontend::TerminalFrontend;
use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::dump::dump;
use docrev::app::viewer::{self, Viewer};
use docrev::infra::terminal;
use docrev::ui::table;

#[derive(Parser)]
#[command(
    name = "docrev",
    version,
    about,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    /// Open a document (.xlsx) in the TUI viewer
    file: Option<PathBuf>,
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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match (cli.command, cli.file) {
        (Some(Command::Dump { file, sheet }), _) => run_dump(&file, sheet.as_deref()),
        (None, Some(file)) => run_viewer(&file),
        (None, None) => {
            let _ = Cli::command().print_help();
            ExitCode::FAILURE
        }
    }
}

fn run_dump(file: &Path, sheet: Option<&str>) -> ExitCode {
    match dump(&XlsxSource, file, sheet) {
        Ok(view) => {
            let rendered = table::render(&view.sheet, view.position, view.total);
            match io::stdout().write_all(rendered.as_bytes()) {
                Ok(()) => ExitCode::SUCCESS,
                // downstream pipe closed early (e.g. `| head`) — not an error
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
                Err(e) => fail(&e),
            }
        }
        Err(e) => fail(&e),
    }
}

fn run_viewer(file: &Path) -> ExitCode {
    let store = JsonCommentStore::for_document(file);
    let viewer = match Viewer::open(&XlsxSource, &store, file) {
        Ok(viewer) => viewer,
        Err(e) => return fail(&e),
    };
    let terminal = match terminal::init() {
        Ok(terminal) => terminal,
        Err(e) => return fail(&e),
    };
    let result = viewer::run(viewer, &mut TerminalFrontend::new(terminal));
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
