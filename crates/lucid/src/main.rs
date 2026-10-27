//! Lucid — a fast, modern viewer for Microsoft Help Container (`.mshc`) documentation.
//!
//! One binary, two faces:
//!
//! * the GUI (`lucid path/to/book.mshc`), and
//! * the headless commands used for automation, CI and context-sensitive help
//!   (`lucid search`, `lucid info`, `lucid toc`, `lucid extract`, `lucid demo`).
//!
//! Contract and exit codes: `docs/DESIGN.md` §6.

#![forbid(unsafe_code)]

mod app;
mod cli;
mod index_cache;
mod load;
mod quickopen;
mod render;
mod state;
mod theme;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match cli::dispatch(&args) {
        Ok(code) => code,
        Err(err) => {
            match err {
                cli::CliError::Usage(message) => {
                    eprintln!("lucid: {message}\n");
                    eprint!("{}", cli::USAGE);
                    ExitCode::from(2)
                }
                cli::CliError::Failed(message) => {
                    eprintln!("lucid: {message}");
                    ExitCode::from(1)
                }
                cli::CliError::NotFound(message) => {
                    eprintln!("lucid: {message}");
                    ExitCode::from(3)
                }
            }
        }
    }
}
