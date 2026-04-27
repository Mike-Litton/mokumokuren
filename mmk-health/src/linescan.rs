//! Imports-only fallback for languages without a real AST adapter.
//!
//! Strictly weaker than tree-sitter — captures the import *source*
//! (module specifier), never imported symbols, never exports, never
//! function shape. Used by STRUCTURE's directory-import-aggregation
//! slice on Rust / Python / Go in v0.5; COMPLEXITY refuses to fire
//! without a real AST.
//!
//! The patterns live in this one place so the truth-table tests
//! pin them per-language without each call site re-deriving the
//! prefix logic.

use std::path::Path;

/// Extract import sources (the quoted module path / `use`-path /
/// dotted module name) from `body`, dispatched by `path` extension.
///
/// Returns sources in source order; empty when the language is
/// unsupported or no imports were found. No deduplication — the
/// caller decides whether duplicate imports matter.
#[must_use]
pub fn extract_imports(path: &Path, body: &str) -> Vec<String> {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return Vec::new();
    };
    match ext.to_lowercase().as_str() {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => extract_js_ts(body),
        "rs" => extract_rust(body),
        "py" => extract_python(body),
        "go" => extract_go(body),
        _ => Vec::new(),
    }
}

fn extract_js_ts(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in body.lines() {
        let line = raw.trim_start();
        // `import 'x'`, `import x from 'y'`, `import { … } from 'y'`,
        // `export { … } from 'y'`. The `from '…'` clause is the
        // anchor — pulling between its quotes covers all forms.
        if line.starts_with("import ")
            || line.starts_with("import(")
            || (line.starts_with("export ") && line.contains(" from "))
        {
            if let Some(s) = quoted_string(line) {
                out.push(s);
                continue;
            }
        }
        // CommonJS `require('x')` — lines starting with `const x = require('…')`
        // or similar.
        if let Some(idx) = line.find("require(") {
            let after = &line[idx + "require(".len()..];
            if let Some(s) = quoted_string(after) {
                out.push(s);
            }
        }
    }
    out
}

fn extract_rust(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in body.lines() {
        let line = raw.trim_start();
        let after = if let Some(rest) = line.strip_prefix("pub use ") {
            rest
        } else if let Some(rest) = line.strip_prefix("use ") {
            rest
        } else {
            continue;
        };
        // `use foo::bar::Baz;`, `use foo::bar::{Baz, Qux};`
        // The path token is everything up to `::{`, `;`, or first
        // whitespace.
        let stop = after
            .find("::{")
            .or_else(|| after.find(';'))
            .or_else(|| after.find(char::is_whitespace))
            .unwrap_or(after.len());
        let path = after[..stop].trim_end_matches(':').trim();
        if !path.is_empty() {
            out.push(path.to_owned());
        }
    }
    out
}

fn extract_python(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in body.lines() {
        let line = raw.trim_start();
        if let Some(rest) = line.strip_prefix("from ") {
            // `from foo.bar import Baz`
            if let Some(end) = rest.find(" import ") {
                let module = rest[..end].trim();
                if !module.is_empty() {
                    out.push(module.to_owned());
                }
            }
        } else if let Some(rest) = line.strip_prefix("import ") {
            // `import foo.bar` or `import foo as f` — first token
            // before whitespace / `,` / `as`.
            let stop = rest
                .find(|c: char| c.is_whitespace() || c == ',')
                .unwrap_or(rest.len());
            let module = rest[..stop].trim();
            if !module.is_empty() {
                out.push(module.to_owned());
            }
        }
    }
    out
}

fn extract_go(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_block = false;
    for raw in body.lines() {
        let line = raw.trim();
        if !in_block {
            if line.starts_with("import (") {
                in_block = true;
                continue;
            }
            if let Some(rest) = line.strip_prefix("import ") {
                if let Some(s) = quoted_string(rest) {
                    out.push(s);
                }
            }
            continue;
        }
        if line.starts_with(')') {
            in_block = false;
            continue;
        }
        if let Some(s) = quoted_string(line) {
            out.push(s);
        }
    }
    out
}

/// Pull the contents of the *first* quoted string (single or double
/// quote) out of `s`. Used by the JS/TS and Go scanners — both anchor
/// imports on a quoted module specifier.
fn quoted_string(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let q = bytes[i];
        if q == b'\'' || q == b'"' || q == b'`' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != q {
                j += 1;
            }
            if j < bytes.len() {
                return std::str::from_utf8(&bytes[start..j])
                    .ok()
                    .map(str::to_owned);
            }
            return None;
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::extract_imports;
    use std::path::Path;

    #[test]
    fn ts_named_import_captures_source() {
        let body = r#"import { foo } from "./foo";
import bar from 'bar';
import 'side-effect';
"#;
        let got = extract_imports(Path::new("a.ts"), body);
        assert_eq!(got, vec!["./foo", "bar", "side-effect"]);
    }

    #[test]
    fn ts_export_from_captures_source() {
        let body = "export { x } from \"./x\";\n";
        let got = extract_imports(Path::new("a.ts"), body);
        assert_eq!(got, vec!["./x"]);
    }

    #[test]
    fn ts_require_captures_source() {
        let body = "const fs = require('fs');\n";
        let got = extract_imports(Path::new("a.js"), body);
        assert_eq!(got, vec!["fs"]);
    }

    #[test]
    fn rust_use_captures_path_to_first_brace() {
        let body = "use foo::bar::Baz;\nuse other::{One, Two};\n";
        let got = extract_imports(Path::new("a.rs"), body);
        assert_eq!(got, vec!["foo::bar::Baz", "other"]);
    }

    #[test]
    fn rust_pub_use_also_captured() {
        let body = "pub use crate::types::Thing;\n";
        let got = extract_imports(Path::new("a.rs"), body);
        assert_eq!(got, vec!["crate::types::Thing"]);
    }

    #[test]
    fn python_import_and_from_captured() {
        let body = "import os\nfrom foo.bar import Baz\nimport pkg as p\n";
        let got = extract_imports(Path::new("a.py"), body);
        assert_eq!(got, vec!["os", "foo.bar", "pkg"]);
    }

    #[test]
    fn go_block_import_captured() {
        let body = "package x\nimport (\n  \"fmt\"\n  \"os\"\n)\n";
        let got = extract_imports(Path::new("a.go"), body);
        assert_eq!(got, vec!["fmt", "os"]);
    }

    #[test]
    fn go_single_import_captured() {
        let body = "package x\nimport \"fmt\"\n";
        let got = extract_imports(Path::new("a.go"), body);
        assert_eq!(got, vec!["fmt"]);
    }

    #[test]
    fn unsupported_extension_returns_empty() {
        let body = "anything";
        let got = extract_imports(Path::new("a.txt"), body);
        assert!(got.is_empty());
    }
}
