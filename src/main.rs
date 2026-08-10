use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use docrev::adapter::xlsx_source::XlsxSource;
use docrev::app::dump::dump;
use docrev::ui::table;

#[derive(Parser)]
#[command(name = "docrev", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
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
    match cli.command {
        Command::Dump { file, sheet } => match dump(&XlsxSource, &file, sheet.as_deref()) {
            Ok(view) => {
                let rendered = table::render(&view.sheet, view.position, view.total);
                match io::stdout().write_all(rendered.as_bytes()) {
                    Ok(()) => ExitCode::SUCCESS,
                    // downstream pipe closed early (e.g. `| head`) — not an error
                    Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("error: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
    }
}
