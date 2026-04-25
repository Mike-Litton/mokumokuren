//! Mokumokuren CLI entry point. Both `mmk` and `mokumokuren` binaries call
//! [`run()`].

use clap::Parser;
use std::io::{self, Write};
use std::process::ExitCode;

pub mod args;
pub mod commands;
pub mod output;

/// Parse `std::env::args` and dispatch. Prints any error to stderr and
/// returns a non-zero `ExitCode` for the caller's `main()`.
#[must_use]
pub fn run() -> ExitCode {
    let cli = args::Cli::parse();
    let stdout = io::stdout();
    let stderr = io::stderr();
    let mut out = stdout.lock();
    let mut err = stderr.lock();

    let result = match cli.command {
        args::Command::Analyze(a) => commands::analyze::run(&a, &mut out, &mut err),
        args::Command::Session(a) => commands::session::run(&a, &mut out, &mut err),
        args::Command::Init(a) => commands::init::run(&a, &mut out, &mut err),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let _ = writeln!(err, "error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
