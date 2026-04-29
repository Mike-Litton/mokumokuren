//! Locks in: the TSX grammar (not the non-TSX TypeScript grammar)
//! is selected for `.tsx` and `.jsx` files. Pre-v0.7 every `.tsx`
//! was parsed by `LANGUAGE_TYPESCRIPT`, which silently degraded on
//! JSX — the test below would have failed.

use mmk_health::adapter::extract;
use std::path::PathBuf;

const TSX_BODY: &str = "
export function App() {
  return <div className=\"hello\" />;
}
";

const JS_BODY: &str = "
export function helper() {
  return 42;
}
";

#[test]
fn tsx_with_jsx_extracts_export() {
    let f = extract(&PathBuf::from("src/App.tsx"), TSX_BODY)
        .expect("TsAdapter should produce facts for tsx with JSX");
    assert!(
        !f.exports.is_empty(),
        "exports must be picked up via the TSX grammar; got {:?}",
        f.exports
    );
    assert_eq!(f.exports[0].name, "App");
}

#[test]
fn jsx_file_extension_extracts_export() {
    // Adapter now claims `.js` and `.jsx` as well; the dispatcher
    // routes `.jsx` to LANGUAGE_TSX so the JSX parses correctly.
    let f = extract(&PathBuf::from("src/App.jsx"), TSX_BODY)
        .expect("TsAdapter should produce facts for jsx files");
    assert!(
        !f.exports.is_empty(),
        "exports must be picked up via the TSX grammar on .jsx; got {:?}",
        f.exports
    );
    assert_eq!(f.exports[0].name, "App");
}

#[test]
fn js_file_extension_extracts_export() {
    let f = extract(&PathBuf::from("src/helper.js"), JS_BODY)
        .expect("TsAdapter should produce facts for js files");
    assert!(
        !f.exports.is_empty(),
        "exports must be picked up on .js; got {:?}",
        f.exports
    );
    assert_eq!(f.exports[0].name, "helper");
}
