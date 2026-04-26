//! `mmk init`: write a starter `mokumokuren.toml` in CWD. With
//! `--profile <NAME>`, write the matching ecosystem-tuned profile
//! (js-ts, rust, python, go) instead of the generic starter.

use anyhow::{bail, Context, Result};
use std::io::Write;

use crate::args::InitArgs;

const STARTER: &str = include_str!("../../profiles/default.toml");
const PROFILE_JS_TS: &str = include_str!("../../profiles/js-ts.toml");
const PROFILE_RUST: &str = include_str!("../../profiles/rust.toml");
const PROFILE_PYTHON: &str = include_str!("../../profiles/python.toml");
const PROFILE_GO: &str = include_str!("../../profiles/go.toml");

const KNOWN_PROFILES: &[&str] = &["default", "js-ts", "rust", "python", "go"];

pub fn run<O: Write, E: Write>(args: &InitArgs, stdout: &mut O, _stderr: &mut E) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let target = cwd.join("mokumokuren.toml");

    if target.exists() && !args.force {
        bail!(
            "{} already exists; pass --force to overwrite",
            target.display()
        );
    }

    let body = match args.profile.as_deref() {
        None | Some("default") => STARTER,
        Some("js-ts") => PROFILE_JS_TS,
        Some("rust") => PROFILE_RUST,
        Some("python") => PROFILE_PYTHON,
        Some("go") => PROFILE_GO,
        Some(other) => bail!(
            "unknown profile {other:?}; available: {}",
            KNOWN_PROFILES.join(", ")
        ),
    };

    std::fs::write(&target, body)
        .with_context(|| format!("failed to write {}", target.display()))?;
    writeln!(stdout, "wrote {}", target.display())?;
    Ok(())
}
