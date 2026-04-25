//! End-to-end: `mmk analyze` discovers `mokumokuren.toml` at the repo
//! root and applies its `ignore` list. CLI `--ignore` unions with file
//! ignores. `--config <path>` overrides discovery.

mod common;

use common::{commit_all, init_repo, write, CWD_LOCK, DAY};
use mokumokuren::args::{AnalyzeArgs, Format};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use tempfile::TempDir;

fn run_in(repo: &Path, args: AnalyzeArgs) -> (Vec<u8>, Vec<u8>) {
    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(repo).unwrap();
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = mokumokuren::commands::analyze::run(&args, &mut stdout, &mut stderr);
    std::env::set_current_dir(orig).unwrap();
    res.expect("analyze should succeed on fixture");
    (stdout, stderr)
}

fn default_args() -> AnalyzeArgs {
    AnalyzeArgs {
        since: "30days".into(),
        top: 50,
        format: Format::Json,
        ignores: Vec::new(),
        config: None,
        verbose: false,
    }
}

/// Two-commit repo with three files: `src/main.rs`, `vendored/lib.h`,
/// `po/sv.po`. All churn each commit so all three would otherwise rank.
fn build_multi_ecosystem_fixture(repo: &Path, now: i64) {
    init_repo(repo);
    write(repo, "src/main.rs", "fn main() {}\n");
    write(repo, "vendored/lib.h", "#define X 1\n");
    write(repo, "po/sv.po", "msgid \"\"\nmsgstr \"\"\n");
    commit_all(repo, "A: initial", now - 2 * DAY);

    write(repo, "src/main.rs", "fn main() { println!(); }\n");
    write(repo, "vendored/lib.h", "#define X 2\n");
    write(repo, "po/sv.po", "msgid \"hi\"\nmsgstr \"hej\"\n");
    commit_all(repo, "B: touch all", now - DAY);
}

fn paths_in(stdout: &[u8]) -> HashSet<String> {
    let v: Value = serde_json::from_slice(stdout).expect("valid JSON");
    v["files"]
        .as_array()
        .expect("files array")
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn no_config_file_includes_all_paths() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_multi_ecosystem_fixture(dir.path(), now);

    let (stdout, _) = run_in(dir.path(), default_args());
    let paths = paths_in(&stdout);
    assert!(paths.contains("src/main.rs"));
    assert!(
        paths.contains("vendored/lib.h"),
        "no config = no filtering; vendored/lib.h should rank: got {paths:?}"
    );
    assert!(paths.contains("po/sv.po"));
}

#[test]
fn repo_root_config_filters_listed_paths() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_multi_ecosystem_fixture(dir.path(), now);
    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "ignore = [\"vendored/**\", \"po/**\"]\n",
    )
    .unwrap();

    let (stdout, _) = run_in(dir.path(), default_args());
    let paths = paths_in(&stdout);
    assert_eq!(
        paths,
        HashSet::from(["src/main.rs".to_string()]),
        "config file should filter both vendored and po; got {paths:?}"
    );
}

#[test]
fn cli_ignore_unions_with_config_file() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_multi_ecosystem_fixture(dir.path(), now);
    // File handles po/, CLI handles vendored/.
    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "ignore = [\"po/**\"]\n",
    )
    .unwrap();

    let mut args = default_args();
    args.ignores.push("vendored/**".into());
    let (stdout, _) = run_in(dir.path(), args);
    let paths = paths_in(&stdout);
    assert_eq!(
        paths,
        HashSet::from(["src/main.rs".to_string()]),
        "union of file + CLI should drop both; got {paths:?}"
    );
}

#[test]
fn explicit_config_path_overrides_repo_discovery() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_multi_ecosystem_fixture(dir.path(), now);
    // Repo-root config would drop vendored — but the explicit one only
    // drops po. Explicit should win.
    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "ignore = [\"vendored/**\"]\n",
    )
    .unwrap();
    let alt = dir.path().join("alt.toml");
    std::fs::write(&alt, "ignore = [\"po/**\"]\n").unwrap();

    let mut args = default_args();
    args.config = Some(alt);
    let (stdout, _) = run_in(dir.path(), args);
    let paths = paths_in(&stdout);
    assert!(
        paths.contains("vendored/lib.h"),
        "explicit config should not include repo-root config's globs; got {paths:?}"
    );
    assert!(
        !paths.contains("po/sv.po"),
        "explicit config should drop po/; got {paths:?}"
    );
}

#[test]
fn explicit_missing_config_path_is_an_error() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_multi_ecosystem_fixture(dir.path(), now);

    let _g = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let orig = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();
    let mut args = default_args();
    args.config = Some(dir.path().join("does-not-exist.toml"));
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = mokumokuren::commands::analyze::run(&args, &mut stdout, &mut stderr);
    std::env::set_current_dir(orig).unwrap();
    let err = res.expect_err("missing explicit --config should error");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("does-not-exist.toml"),
        "error should name the missing path: {msg}"
    );
}

#[test]
fn verbose_reports_config_source_and_filter_impact() {
    let dir = TempDir::new().unwrap();
    let now: i64 = 1_700_000_000;
    build_multi_ecosystem_fixture(dir.path(), now);
    std::fs::write(
        dir.path().join("mokumokuren.toml"),
        "ignore = [\"vendored/**\", \"po/**\"]\n",
    )
    .unwrap();

    let mut args = default_args();
    args.verbose = true;
    let (_, stderr) = run_in(dir.path(), args);
    let text = String::from_utf8(stderr).unwrap();
    assert!(
        text.contains("mokumokuren.toml"),
        "verbose should name the loaded config: {text}"
    );
    assert!(
        text.contains("HEAD path"),
        "verbose should report ignored HEAD count: {text}"
    );
}
