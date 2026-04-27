//! Round-trip tests on the TS adapter — TS body to
//! [`StructuredFacts`]. Pin `template_stem` normalisation since
//! STRUCTURE depends on it for cross-sibling templates.

use mmk_health::{adapter, ExportKind};
use std::path::Path;

#[test]
fn extract_returns_some_for_ts_path() {
    let body = "import { foo } from './x';\nexport function Foo() {}\n";
    let facts = adapter::extract(Path::new("a.ts"), body).expect("ts adapter");
    assert_eq!(facts.imports.len(), 1);
    assert_eq!(facts.exports.len(), 1);
}

#[test]
fn template_stem_normalises_award_dialog() {
    // The doc claim: `award.tsx` exporting `CreateAwardDialog`
    // must produce template_stem `Create*Dialog`.
    let body = "export function CreateAwardDialog() { return null; }\n";
    let facts = adapter::extract(Path::new("award.tsx"), body).expect("ts adapter");
    assert_eq!(facts.exports.len(), 1);
    let e = &facts.exports[0];
    assert_eq!(e.name, "CreateAwardDialog");
    assert_eq!(e.template_stem, "Create*Dialog");
    assert_eq!(e.kind, ExportKind::Function);
}

#[test]
fn extract_returns_none_for_unsupported_ext() {
    // Rust adapter is a stub; without a real AST it returns None
    // and the dispatch propagates None — sensors fall back to
    // line-scan or refuse to fire.
    assert!(adapter::extract(Path::new("a.rs"), "fn main(){}").is_none());
    assert!(adapter::extract(Path::new("a.py"), "x = 1").is_none());
}

#[test]
fn extract_with_imports_uses_linescan_for_rust() {
    let body = "use foo::bar::Baz;\n";
    let facts = adapter::extract_with_imports(Path::new("a.rs"), body)
        .expect("linescan should yield at least one import");
    assert_eq!(facts.imports.len(), 1);
    assert_eq!(facts.imports[0].source, "foo::bar::Baz");
}

#[test]
fn nested_function_max_nesting_depth_climbs() {
    let body = r"
function outer() {
  for (const x of xs) {
    if (x) {
      while (true) {
        try {
          break;
        } catch (e) { return; }
      }
    }
  }
}
";
    let facts = adapter::extract(Path::new("a.ts"), body).expect("ts adapter");
    let outer = facts
        .functions
        .iter()
        .find(|f| f.name == "outer")
        .expect("outer fn");
    // body=1, for=2, if=3, while=4, try=5
    assert!(
        outer.max_nesting_depth >= 5,
        "expected ≥5; got {}",
        outer.max_nesting_depth
    );
}
