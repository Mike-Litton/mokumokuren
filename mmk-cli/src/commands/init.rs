//! `mmk init`: write a starter `mokumokuren.toml` in CWD.

use anyhow::{bail, Context, Result};
use std::io::Write;

use crate::args::InitArgs;

const STARTER: &str = include_str!("../../starter_config.toml");

pub fn run<O: Write, E: Write>(args: &InitArgs, stdout: &mut O, _stderr: &mut E) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let target = cwd.join("mokumokuren.toml");

    if target.exists() && !args.force {
        bail!(
            "{} already exists; pass --force to overwrite",
            target.display()
        );
    }

    std::fs::write(&target, STARTER)
        .with_context(|| format!("failed to write {}", target.display()))?;
    writeln!(stdout, "wrote {}", target.display())?;
    Ok(())
}
