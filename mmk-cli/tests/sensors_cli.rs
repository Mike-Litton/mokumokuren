//! `mmk sensors {list,describe}` — discoverability surface tests.
//!
//! Asserts the JSON envelope shape that agent harnesses consume:
//! every catalog row has the same key set, so `jq` filters work
//! without per-row branching.

use mokumokuren::args::{Format, SensorsAction, SensorsArgs, SensorsDescribeArgs, SensorsListArgs};
use serde_json::Value;

fn run_sensors(args: SensorsArgs) -> Vec<u8> {
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    mokumokuren::commands::sensors::run(&args, &mut stdout, &mut stderr).expect("sensors run");
    stdout
}

fn list_json() -> Value {
    let stdout = run_sensors(SensorsArgs {
        action: SensorsAction::List(SensorsListArgs {
            format: Format::Json,
        }),
    });
    serde_json::from_slice(&stdout).expect("valid JSON")
}

fn describe_json(name: &str) -> Value {
    let stdout = run_sensors(SensorsArgs {
        action: SensorsAction::Describe(SensorsDescribeArgs {
            name: name.to_string(),
            format: Format::Json,
        }),
    });
    serde_json::from_slice(&stdout).expect("valid JSON")
}

#[test]
fn list_envelope_carries_schema_and_sensors_array() {
    let v = list_json();
    assert!(v["schema_version"].is_string(), "schema_version present");
    let sensors = v["sensors"].as_array().expect("sensors[] present");
    assert!(
        sensors.len() >= 11,
        "expected ≥11 catalog rows, got {}",
        sensors.len()
    );
    // Required keys on every row — agent harnesses rely on uniform
    // shape for `jq '.sensors[] | select(...)'`.
    for row in sensors {
        for key in [
            "name",
            "layer",
            "mode",
            "default_severity",
            "description",
            "commands",
            "since",
        ] {
            assert!(
                row.get(key).is_some(),
                "row missing required key `{key}`: {row}"
            );
        }
    }
}

#[test]
fn list_marks_broad_catch_debt_as_audit_only() {
    let v = list_json();
    let bcd = v["sensors"]
        .as_array()
        .expect("sensors")
        .iter()
        .find(|r| r["name"] == "HEALTH:broad_catch_debt")
        .expect("broad_catch_debt entry");
    assert_eq!(bcd["commands"], serde_json::json!(["audit"]));
    assert_eq!(bcd["mode"], "static");
    assert_eq!(bcd["default_severity"], "Info");
}

#[test]
fn describe_broad_exception_long_description_mentions_log_identifiers() {
    let v = describe_json("broad_exception");
    let long = v["long_description"]
        .as_str()
        .expect("long_description present");
    assert!(
        long.contains("log_identifiers"),
        "expected log_identifiers in long_description; got: {long}"
    );
}

#[test]
fn describe_broad_catch_debt_envelope_shape() {
    let v = describe_json("broad_catch_debt");
    assert_eq!(v["mode"], "static");
    assert_eq!(v["commands"], serde_json::json!(["audit"]));
    assert!(v["long_description"].is_string());
}

#[test]
fn describe_unknown_sensor_returns_error() {
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let res = mokumokuren::commands::sensors::run(
        &SensorsArgs {
            action: SensorsAction::Describe(SensorsDescribeArgs {
                name: "totally_made_up_sensor".to_string(),
                format: Format::Text,
            }),
        },
        &mut stdout,
        &mut stderr,
    );
    assert!(res.is_err(), "unknown sensor must surface an error");
}
