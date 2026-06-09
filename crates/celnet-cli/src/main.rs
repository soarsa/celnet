//! Celnet operator/quant CLI (`celnet`) — price vanillas, build smiles, price
//! exotics, and inspect FX conventions from the terminal (work-stream WS-I).
//!
//! `main` is intentionally thin: it parses the [`cli::Cli`] command tree and hands
//! off to [`cli::dispatch`], which converts arguments and invokes the per-command
//! core functions in [`price`], [`surface`], [`exotic`], [`convention`], and
//! [`risk`]. The local-compute cores route every number through the underlying
//! Celnet crates, and the networked [`risk`] (`risk aggregate`/`drill`/`positions`/
//! `limits`) and `stream` commands route every number through the typed
//! `celnet-client` SDK against a running edge — the SAME contract the GUI and Excel
//! add-in consume — so the CLI adds no pricing/aggregation of its own (four-client
//! parity).

#![forbid(unsafe_code)]

mod args;
mod basket;
mod cli;
mod convention;
mod exotic;
mod linear;
mod price;
mod risk;
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
