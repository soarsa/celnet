//! Celnet operator/quant CLI (`celnet`) — price vanillas, build smiles, price
//! exotics, and inspect FX conventions from the terminal (work-stream WS-I).
//!
//! `main` is intentionally thin: it parses the [`cli::Cli`] command tree and hands
//! off to [`cli::dispatch`], which converts arguments and invokes the per-command
//! core functions in [`price`], [`surface`], [`exotic`], and [`convention`]. Those
//! cores route every number through the underlying Celnet crates, so the CLI adds
//! no pricing logic of its own.

#![forbid(unsafe_code)]

mod args;
mod cli;
mod convention;
mod exotic;
mod price;
mod surface;
mod tenor;

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = cli::Cli::parse();
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    match cli::dispatch(cli, &mut handle) {
        Ok(()) => {
            let _ = handle.flush();
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = handle.flush();
            let mut stderr = std::io::stderr();
            let _ = writeln!(stderr, "error: {e}");
            ExitCode::FAILURE
        }
    }
}
