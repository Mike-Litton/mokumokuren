//! Findings format primitive — the unified text + JSON shape every
//! event-driven subcommand (`review`, `pre-edit`, `drift`,
//! `session-summary`) emits. Layer-labeled, terse, line-by-line.
//!
//! Orthogonality tag: protects **agent mode** (the JSON contract LLM
//! harnesses parse) and **human mode** (one-line-per-finding text the
//! reviewer scans).

use mokumokuren::output::findings::{render_json, render_text, Finding, Layer, Severity};
use serde_json::Value;

#[test]
fn info_finding_rendered_with_bracket_marker_for_low_priority_distinction() {
    // Info renders with a `[info]` text marker, not the `ⓘ` glyph:
    // the glyph tokenized too close to `⚠`, and bracketed prefixes
    // match mmk's other low-priority forms (`[no actionable signal]`,
    // `[low-confidence n=N]`). The marker lets agents recognise
    // low-priority severity from the line itself, without
    // inspecting the JSON envelope's `severity` field.
    let findings = vec![Finding::new(
        Layer::Complexity,
        Severity::Info,
        "src/foo.ts::moveCardTo: 47 LOC (+1 vs HEAD), directory median 22 LOC (ratio 2.1)"
            .to_string(),
    )];

    let mut buf = Vec::new();
    render_text(&mut buf, &findings).unwrap();
    let out = String::from_utf8(buf).unwrap();

    let body_line = out
        .lines()
        .find(|l| l.contains("moveCardTo"))
        .expect("rendered line missing");
    assert!(
        body_line.starts_with("  [info] "),
        "info finding must be prefixed with `[info]` for tokenizer-friendly distinction, got: {body_line}"
    );
}

#[test]
fn finding_renders_text_one_line_per_finding() {
    let findings = vec![Finding::new(
        Layer::Hotspot,
        Severity::Warn,
        "core/a.rs ranks #2 (top-20 hotspot)".to_string(),
    )];

    let mut buf = Vec::new();
    render_text(&mut buf, &findings).unwrap();
    let out = String::from_utf8(buf).unwrap();

    let body_lines: Vec<&str> = out
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.ends_with(':'))
        .collect();
    assert_eq!(
        body_lines.len(),
        1,
        "one finding should render as one body line, got: {out}"
    );
    let line = body_lines[0];
    assert!(
        line.starts_with("  ⚠"),
        "warn finding must be prefixed with ⚠ (stable severity glyph), got: {line}"
    );
    assert!(
        line.contains("core/a.rs ranks #2 (top-20 hotspot)"),
        "message must round-trip into the rendered line, got: {line}"
    );
}

#[test]
fn findings_serialize_to_json_with_stable_keys() {
    let findings = vec![
        Finding::new(Layer::Hotspot, Severity::Warn, "h".to_string()),
        Finding::new(Layer::Coupling, Severity::Info, "c".to_string()),
        Finding::new(Layer::Drift, Severity::Ok, "d".to_string()),
        Finding::new(Layer::Budget, Severity::Warn, "b".to_string()),
    ];

    let mut buf = Vec::new();
    render_json(&mut buf, &findings).unwrap();
    let v: Value = serde_json::from_slice(&buf).expect("findings render valid JSON");

    let arr = v.as_array().expect("findings JSON is a flat array");
    assert_eq!(arr.len(), 4, "all findings present in JSON");

    for (i, entry) in arr.iter().enumerate() {
        let obj = entry
            .as_object()
            .unwrap_or_else(|| panic!("findings[{i}] should be an object, got {entry:?}"));
        let keys: std::collections::BTreeSet<&str> = obj.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["id", "layer", "message", "severity"]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
            "findings[{i}] must have exactly {{layer, severity, message, id}} keys, got {keys:?}"
        );
        assert!(
            entry["id"].is_null(),
            "findings without an explain id must serialize id as null (explicit absence), got {:?}",
            entry["id"],
        );
    }

    assert_eq!(arr[0]["layer"], "hotspot");
    assert_eq!(arr[0]["severity"], "warn");
    assert_eq!(arr[0]["message"], "h");
    assert_eq!(arr[1]["layer"], "coupling");
    assert_eq!(arr[1]["severity"], "info");
    assert_eq!(arr[2]["layer"], "drift");
    assert_eq!(arr[2]["severity"], "ok");
    assert_eq!(arr[3]["layer"], "budget");
}

#[test]
fn findings_grouping_renders_layer_header_in_text_only() {
    let findings = vec![
        Finding::new(Layer::Hotspot, Severity::Warn, "h1".to_string()),
        Finding::new(Layer::Coupling, Severity::Info, "c1".to_string()),
        Finding::new(Layer::Hotspot, Severity::Info, "h2".to_string()),
    ];

    // Text mode: groups by layer, header per layer, body lines
    // indented under it.
    let mut tbuf = Vec::new();
    render_text(&mut tbuf, &findings).unwrap();
    let text = String::from_utf8(tbuf).unwrap();

    let hotspot_pos = text
        .find("HOTSPOT:")
        .expect("text mode must render layer header `HOTSPOT:`");
    let coupling_pos = text
        .find("COUPLING:")
        .expect("text mode must render layer header `COUPLING:`");
    assert!(text.contains("h1"));
    assert!(text.contains("h2"));
    assert!(text.contains("c1"));

    let h2_pos = text.find("h2").unwrap();
    assert!(
        h2_pos < coupling_pos,
        "h2 (also HOTSPOT) must group under HOTSPOT header, before COUPLING starts.\n\
         text:\n{text}"
    );
    assert!(hotspot_pos < h2_pos);

    // JSON mode: flat array, no layer-header strings.
    let mut jbuf = Vec::new();
    render_json(&mut jbuf, &findings).unwrap();
    let json_text = String::from_utf8(jbuf).unwrap();
    assert!(
        !json_text.contains("HOTSPOT:") && !json_text.contains("COUPLING:"),
        "JSON output must not emit text-mode layer headers, got: {json_text}"
    );
    let v: Value = serde_json::from_str(&json_text).unwrap();
    assert!(
        v.is_array(),
        "JSON findings render is a flat array, not a grouping object"
    );
}

#[test]
fn finding_with_id_renders_trailing_id_tag_in_text_mode() {
    // The `[id=…]` tag is the agent's join-key for `mmk explain
    // --finding <id>`. Rendered at the end of the line so a grep on
    // the message prose still works, and so the id never visually
    // competes with the severity glyph.
    let findings = vec![Finding::with_id(
        Layer::Coupling,
        Severity::Warn,
        "core/a.rs edited; core/b.rs co-edited 8 of 12 prior commits, not in diff".to_string(),
        "coupling:core/a.rs:core/b.rs".to_string(),
    )];
    let mut buf = Vec::new();
    render_text(&mut buf, &findings).unwrap();
    let out = String::from_utf8(buf).unwrap();
    let body = out
        .lines()
        .find(|l| l.contains("co-edited"))
        .expect("rendered line missing");
    assert!(
        body.ends_with(" [id=coupling:core/a.rs:core/b.rs]"),
        "id tag must sit at end of line, got: {body}"
    );
}

#[test]
fn finding_with_id_serializes_to_json_with_id_field() {
    let findings = vec![Finding::with_id(
        Layer::Coupling,
        Severity::Warn,
        "msg".to_string(),
        "coupling:a.rs:b.rs".to_string(),
    )];
    let mut buf = Vec::new();
    render_json(&mut buf, &findings).unwrap();
    let v: Value = serde_json::from_slice(&buf).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr[0]["id"], "coupling:a.rs:b.rs");
}

#[test]
fn empty_findings_render_nothing() {
    let mut tbuf = Vec::new();
    render_text(&mut tbuf, &[]).unwrap();
    assert!(tbuf.is_empty(), "empty findings -> empty text output");

    let mut jbuf = Vec::new();
    render_json(&mut jbuf, &[]).unwrap();
    let v: Value = serde_json::from_slice(&jbuf).expect("empty array is valid JSON");
    assert_eq!(v, Value::Array(vec![]));
}
