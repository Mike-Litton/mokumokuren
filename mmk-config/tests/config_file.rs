//! Loading + parsing of `mokumokuren.toml`. The schema is intentionally
//! tiny (just `ignore = [...]`) — the bar is not extensibility but
//! "user types four lines of TOML and the noisy paths go away."

use mmk_config::ConfigFile;
use std::path::Path;
use tempfile::TempDir;

fn write_toml(dir: &Path, body: &str) -> std::path::PathBuf {
    let p = dir.join("mokumokuren.toml");
    std::fs::write(&p, body).expect("write toml");
    p
}

#[test]
fn loads_explicit_ignore_list() {
    let dir = TempDir::new().unwrap();
    let p = write_toml(
        dir.path(),
        r#"
ignore = ["po/**", "Documentation/**"]
"#,
    );
    let cfg = ConfigFile::load_from_path(&p).expect("load");
    assert_eq!(
        cfg.ignore,
        vec!["po/**".to_string(), "Documentation/**".to_string()]
    );
}

#[test]
fn empty_file_is_valid_and_yields_default() {
    let dir = TempDir::new().unwrap();
    let p = write_toml(dir.path(), "");
    let cfg = ConfigFile::load_from_path(&p).expect("empty file should parse");
    assert!(cfg.ignore.is_empty());
}

#[test]
fn missing_ignore_field_yields_default() {
    let dir = TempDir::new().unwrap();
    // Valid TOML, just no ignore key.
    let p = write_toml(dir.path(), "# nothing here yet\n");
    let cfg = ConfigFile::load_from_path(&p).expect("ok");
    assert!(cfg.ignore.is_empty());
}

#[test]
fn malformed_toml_is_a_clear_error() {
    let dir = TempDir::new().unwrap();
    let p = write_toml(dir.path(), "this is not = valid = toml = at all\n[broken");
    let err = ConfigFile::load_from_path(&p).unwrap_err();
    let msg = format!("{err:#}");
    // Both the source path and a TOML-parser hint should be present so
    // the user can find the file and the offending location.
    assert!(
        msg.contains("mokumokuren.toml"),
        "error should name the file: {msg}"
    );
}

#[test]
fn unknown_keys_are_a_clear_error() {
    // Strict parsing: a misspelled key shouldn't silently no-op.
    let dir = TempDir::new().unwrap();
    let p = write_toml(dir.path(), "ignores = [\"po/**\"]\n"); // typo: ignores vs ignore
    let err = ConfigFile::load_from_path(&p).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.to_lowercase().contains("unknown") || msg.to_lowercase().contains("ignores"),
        "error should call out the typo: {msg}"
    );
}

#[test]
fn missing_path_yields_io_error() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("does-not-exist.toml");
    let err = ConfigFile::load_from_path(&p).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("does-not-exist.toml"),
        "error should mention the missing path: {msg}"
    );
}

#[test]
fn default_is_an_empty_ignore_list() {
    let cfg = ConfigFile::default();
    assert!(cfg.ignore.is_empty());
}

#[test]
fn loads_blast_radius_threshold() {
    let dir = TempDir::new().unwrap();
    let p = write_toml(
        dir.path(),
        r"
[blast_radius]
threshold = 0.25
",
    );
    let cfg = ConfigFile::load_from_path(&p).expect("load");
    let br = cfg
        .blast_radius
        .expect("blast_radius block should be parsed");
    assert!((br.threshold - 0.25).abs() < 1e-12);
}

#[test]
fn missing_blast_radius_block_yields_none() {
    let cfg = ConfigFile::default();
    assert!(
        cfg.blast_radius.is_none(),
        "default ConfigFile must not synthesize a blast_radius block"
    );
}

#[test]
fn loads_coupling_v0_4_fields() {
    let dir = TempDir::new().unwrap();
    let p = write_toml(
        dir.path(),
        r#"
[coupling]
confidence_threshold = 0.25
min_sample_size = 8
ignore_partners = ["**/CHANGELOG.md"]
"#,
    );
    let cfg = ConfigFile::load_from_path(&p).expect("load");
    let cp = cfg.coupling.expect("coupling block parsed");
    assert!((cp.confidence_threshold.unwrap() - 0.25).abs() < 1e-12);
    assert_eq!(cp.min_sample_size, Some(8));
    assert_eq!(cp.ignore_partners, vec!["**/CHANGELOG.md".to_string()]);
}

#[test]
fn loads_health_ts_block() {
    let dir = TempDir::new().unwrap();
    let p = write_toml(
        dir.path(),
        r#"
[health.ts]
enabled = true
patterns = ["test_pair"]
"#,
    );
    let cfg = ConfigFile::load_from_path(&p).expect("load");
    let h = cfg.health.expect("health block parsed");
    let ts = h.ts.expect("ts subblock parsed");
    assert_eq!(ts.enabled, Some(true));
    assert_eq!(ts.patterns, Some(vec!["test_pair".to_string()]));
}
