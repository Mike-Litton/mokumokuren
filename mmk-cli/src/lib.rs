//! Mokumokuren CLI entry point. Both `mmk` and `mokumokuren` binaries call
//! [`run()`].

use clap::Parser;
use std::io::{self, Write};
use std::process::ExitCode;

pub mod args;
pub mod commands;
pub mod dedup;
pub mod hook;
pub mod monotonic;
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

    // Hook envelope is read once, before subcommand dispatch. Only
    // review / pre-edit consume it (those are the hooked subcommands
    // — analyze / drift / eval / session-summary are all explicit
    // user commands). On parse failure we surface loudly: silently
    // falling back to argv mode would aim mmk at the wrong path.
    let envelope = match hook::read_envelope_from_stdin() {
        Ok(env) => env,
        Err(e) => {
            let _ = writeln!(err, "error: {e:#}");
            return ExitCode::FAILURE;
        }
    };

    // Verdict::GateTriggered → exit 2 (distinct from 1 = mmk error).
    // 2 is conventional for "policy failure" (lint exit codes,
    // pre-commit). Lets CI distinguish "mmk crashed" from "mmk found
    // an issue and you asked to gate on it."
    let result = match cli.command {
        args::Command::Analyze(a) => {
            commands::analyze::run(&a, &mut out, &mut err).map(|()| Verdict::Ok)
        }
        args::Command::SessionSummary(a) => commands::session::run(&a, &mut out, &mut err),
        args::Command::Review(a) => {
            commands::review::run(&a, envelope.as_ref(), &mut out, &mut err)
        }
        args::Command::PreEdit(a) => {
            commands::pre_edit::run(&a, envelope.as_ref(), &mut out, &mut err)
        }
        args::Command::Drift(a) => {
            commands::drift::run(&a, &mut out, &mut err).map(|()| Verdict::Ok)
        }
        args::Command::Audit(a) => commands::audit::run(&a, &mut out, &mut err),
        args::Command::Init(a) => commands::init::run(&a, &mut out, &mut err).map(|()| Verdict::Ok),
        args::Command::Eval(a) => commands::eval::run(&a, &mut out, &mut err).map(|()| Verdict::Ok),
        args::Command::Cache(a) => {
            commands::cache::run(&a, &mut out, &mut err).map(|()| Verdict::Ok)
        }
        args::Command::Explain(a) => {
            commands::explain::run(&a, &mut out, &mut err).map(|()| Verdict::Ok)
        }
        args::Command::Sensors(a) => {
            commands::sensors::run(&a, &mut out, &mut err).map(|()| Verdict::Ok)
        }
    };

    match result {
        Ok(Verdict::Ok) => ExitCode::SUCCESS,
        Ok(Verdict::GateTriggered) => ExitCode::from(2),
        Err(e) => {
            let _ = writeln!(err, "error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Outcome of a subcommand run. Most commands always return `Ok`;
/// review/pre-edit/session may return `GateTriggered` when the user
/// passed `--gate <warn|error>` and a matching finding fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    GateTriggered,
}
