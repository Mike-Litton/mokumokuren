//! `mmk init` writes a starter `mokumokuren.toml` in the current
//! directory. The file should be valid TOML, parse cleanly through
//! `ConfigFile::load_from_path`, and contain commented-out examples
//! covering the common ecosystem cases (translations, vendored,
//! lockfiles, generated, engine assets) so users can uncomment what
//! applies to their repo without a separate cheat sheet trip.

mod common;

use common::CWD_LOCK;
use mmk_config::ConfigFile;
use mokumokuren::args::InitArgs;
use std::path::Path;
use tempfile::TempDir;

fn run_init_in(dir: &Path, args: InitArgs) -> (Result<(), anyhow::Error>, Vec<u8>, Vec<u8>) {
    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir).unwrap();
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = mokumokuren::commands::init::run(&args, &mut stdout, &mut stderr);
    std::env::set_current_dir(orig).unwrap();
    (res, stdout, stderr)
}

#[test]
fn init_creates_mokumokuren_toml_in_cwd() {
    let dir = TempDir::new().unwrap();
    let (res, stdout, _) = run_init_in(dir.path(), InitArgs { force: false });
    res.expect("init should succeed");

    let target = dir.path().join("mokumokuren.toml");
    assert!(target.exists(), "expected file to be created");
    let stdout = String::from_utf8(stdout).unwrap();
    assert!(
        stdout.contains("mokumokuren.toml"),
        "stdout should mention the path: {stdout}"
    );
}

#[test]
fn init_output_parses_as_a_valid_config_file() {
    let dir = TempDir::new().unwrap();
    let (res, _, _) = run_init_in(dir.path(), InitArgs { force: false });
    res.expect("init");
    let path = dir.path().join("mokumokuren.toml");
    let cfg =
        ConfigFile::load_from_path(&path).expect("starter file must round-trip through the loader");
    // Starter is all comments — concrete ignores commented out — so the
    // active list is empty. This is intentional: no decision is made for
    // the user.
    assert!(
        cfg.ignore.is_empty(),
        "starter should have no active ignores; user uncomments what applies"
    );
}

#[test]
fn init_starter_mentions_common_ecosystem_patterns() {
    let dir = TempDir::new().unwrap();
    let (res, _, _) = run_init_in(dir.path(), InitArgs { force: false });
    res.expect("init");
    let body = std::fs::read_to_string(dir.path().join("mokumokuren.toml")).unwrap();
    // Don't pin exact wording — pin that each common case shows up so
    // the file is genuinely useful as a copy-paste cheat sheet.
    for needle in [
        "*.po",              // translations
        "vendor",            // vendored
        "node_modules",      // vendored (JS)
        "Cargo.lock",        // lockfiles
        "package-lock.json", // lockfiles
        "target",            // generated
        "*.tscn",            // engine assets
    ] {
        assert!(
            body.contains(needle),
            "starter should mention {needle:?} as a copy-paste example"
        );
    }
}

#[test]
fn init_refuses_to_overwrite_without_force() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "ignore = [\"existing/**\"]\n",
    )
    .unwrap();

    let (res, _, _) = run_init_in(dir.path(), InitArgs { force: false });
    let err = res.expect_err("should refuse to clobber");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("--force") && msg.contains("mokumokuren.toml"),
        "error should explain how to overwrite: {msg}"
    );

    // And the original file is untouched.
    let body = std::fs::read_to_string(dir.path().join("mokumokuren.toml")).unwrap();
    assert!(body.contains("existing/**"));
}

#[test]
fn init_force_overwrites_existing_file() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "ignore = [\"existing/**\"]\n",
    )
    .unwrap();

    let (res, _, _) = run_init_in(dir.path(), InitArgs { force: true });
    res.expect("--force should succeed");
    let body = std::fs::read_to_string(dir.path().join("mokumokuren.toml")).unwrap();
    assert!(
        !body.contains("existing/**"),
        "old contents should be replaced"
    );
}
