//! `mmk init` writes a starter `mokumokuren.toml` in the current
//! directory. The file should be valid TOML, parse cleanly through
//! `ConfigFile::load_from_path`, and contain commented-out examples
//! covering the common ecosystem cases (translations, vendored,
//! lockfiles, generated, engine assets) so users can uncomment what
//! applies to their repo without a separate cheat sheet trip.

mod common;

use mmk_config::ConfigFile;
use mokumokuren::args::InitArgs;
use serial_test::serial;
use std::path::Path;
use tempfile::TempDir;

fn run_init_in(dir: &Path, args: InitArgs) -> (Result<(), anyhow::Error>, Vec<u8>, Vec<u8>) {
    common::with_cwd(dir, |so, se| {
        mokumokuren::commands::init::run(&args, so, se)
    })
}

#[serial(cwd)]
#[test]
fn init_creates_mokumokuren_toml_in_cwd() {
    let dir = TempDir::new().unwrap();
    let (res, stdout, _) = run_init_in(
        dir.path(),
        InitArgs {
            force: false,
            profile: None,
        },
    );
    res.expect("init should succeed");

    let target = dir.path().join("mokumokuren.toml");
    assert!(target.exists(), "expected file to be created");
    let stdout = String::from_utf8(stdout).unwrap();
    assert!(
        stdout.contains("mokumokuren.toml"),
        "stdout should mention the path: {stdout}"
    );
}

#[serial(cwd)]
#[test]
fn init_output_parses_as_a_valid_config_file() {
    let dir = TempDir::new().unwrap();
    let (res, _, _) = run_init_in(
        dir.path(),
        InitArgs {
            force: false,
            profile: None,
        },
    );
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

#[serial(cwd)]
#[test]
fn init_starter_mentions_common_ecosystem_patterns() {
    let dir = TempDir::new().unwrap();
    let (res, _, _) = run_init_in(
        dir.path(),
        InitArgs {
            force: false,
            profile: None,
        },
    );
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

#[serial(cwd)]
#[test]
fn init_refuses_to_overwrite_without_force() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "ignore = [\"existing/**\"]\n",
    )
    .unwrap();

    let (res, _, _) = run_init_in(
        dir.path(),
        InitArgs {
            force: false,
            profile: None,
        },
    );
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

#[serial(cwd)]
#[test]
fn init_with_js_ts_profile_writes_expected_keys() {
    let dir = TempDir::new().unwrap();
    let (res, _, _) = run_init_in(
        dir.path(),
        InitArgs {
            force: false,
            profile: Some("js-ts".into()),
        },
    );
    res.expect("js-ts profile should write");
    let body = std::fs::read_to_string(dir.path().join("mokumokuren.toml")).unwrap();
    // Snapshot-style: pin signal patterns the profile must contain
    // so a regression in the profile content fails this test.
    for needle in [
        "node_modules",
        "**/package.json",
        "**/Fastfile",
        "[coupling]",
        "confidence_threshold = 0.20",
        "min_sample_size = 1",
        "[health.ts]",
        "test_pair",
        // Lockfiles must be globified so nested workspace lockfiles
        // are caught (bare `yarn.lock` only matches at repo root).
        "**/yarn.lock",
        "**/package-lock.json",
        "**/pnpm-lock.yaml",
        // Yarn 4 vendors its release binary under .yarn/.
        ".yarn/**",
        // Auto-generated release notes shouldn't surface as required
        // co-edits.
        "**/CHANGELOG.md",
        // Role patterns ship as an active block, not commented-out:
        // users see what's exempted without reading mmk source.
        "[sensor.structure]",
        "role_patterns",
        "*Barrel",
    ] {
        assert!(
            body.contains(needle),
            "js-ts profile missing {needle:?}; body was:\n{body}"
        );
    }
    // Negative pin: the role_patterns block must be active, not
    // commented out. Find the line that declares the block and
    // assert it doesn't start with `#`.
    let active = body
        .lines()
        .find(|l| l.trim_start().starts_with("[sensor.structure]"))
        .expect("expected [sensor.structure] block in js-ts profile");
    assert!(
        !active.trim_start().starts_with('#'),
        "[sensor.structure] block must be active, not commented out"
    );
}

#[serial(cwd)]
#[test]
fn init_with_rust_profile_writes_minimal_config() {
    let dir = TempDir::new().unwrap();
    let (res, _, _) = run_init_in(
        dir.path(),
        InitArgs {
            force: false,
            profile: Some("rust".into()),
        },
    );
    res.expect("rust profile should write");
    let body = std::fs::read_to_string(dir.path().join("mokumokuren.toml")).unwrap();
    assert!(body.contains("Cargo.lock"));
    assert!(body.contains("[coupling]"));
}

#[serial(cwd)]
#[test]
fn init_unknown_profile_is_an_error() {
    let dir = TempDir::new().unwrap();
    let (res, _, _) = run_init_in(
        dir.path(),
        InitArgs {
            force: false,
            profile: Some("not-a-real-profile".into()),
        },
    );
    let err = res.expect_err("unknown profile must error");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("not-a-real-profile") && msg.contains("js-ts"),
        "error should name the bad profile and list available ones: {msg}"
    );
}

#[serial(cwd)]
#[test]
fn init_force_overwrites_existing_file() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "ignore = [\"existing/**\"]\n",
    )
    .unwrap();

    let (res, _, _) = run_init_in(
        dir.path(),
        InitArgs {
            force: true,
            profile: None,
        },
    );
    res.expect("--force should succeed");
    let body = std::fs::read_to_string(dir.path().join("mokumokuren.toml")).unwrap();
    assert!(
        !body.contains("existing/**"),
        "old contents should be replaced"
    );
}
