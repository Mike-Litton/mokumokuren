//! TypeScript adapter — produces [`StructuredFacts`] from a `.ts` /
//! `.tsx` body via tree-sitter.
//!
//! Walker approach: one full-tree walk extracts imports, exports,
//! and per-function metrics in a single pass. Type-density numbers
//! are gathered along the way.

use crate::adapter::LanguageAdapter;
use crate::facts::{
    template_for, ExportFact, ExportKind, FunctionFact, ImportFact, StructuredFacts, TypeDensity,
};
use crate::ts::parse_for;
use std::path::Path;
use tree_sitter::Node;

#[derive(Debug, Default)]
pub struct TsAdapter;

impl LanguageAdapter for TsAdapter {
    fn extensions(&self) -> &'static [&'static str] {
        &["ts", "tsx", "js", "jsx"]
    }

    fn extract(&self, path: &Path, body: &str) -> Option<StructuredFacts> {
        let tree = parse_for(path, body)?;
        let root = tree.root_node();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();

        let mut facts = StructuredFacts::default();
        let mut scope: Vec<String> = Vec::new();
        walk(root, body.as_bytes(), &stem, &mut scope, &mut facts);
        Some(facts)
    }
}

fn walk(
    node: Node<'_>,
    src: &[u8],
    stem: &str,
    scope: &mut Vec<String>,
    facts: &mut StructuredFacts,
) {
    match node.kind() {
        "import_statement" => {
            if let Some(import) = parse_import(node, src) {
                facts.imports.push(import);
            }
            // Don't descend further: nested imports aren't a thing,
            // and the children would otherwise be re-walked for
            // type-density — but inside an import clause every
            // identifier is "typed" trivially, which would skew
            // the density numbers.
            return;
        }
        "export_statement" => {
            if let Some(export) = parse_export(node, src, stem) {
                facts.exports.push(export);
            }
            // Fall through: an export wraps a function/class whose
            // body still needs walking for COMPLEXITY metrics.
        }
        "function_declaration" | "method_definition" => {
            if let Some(func) = parse_function(node, src, scope) {
                facts.functions.push(func);
            }
        }
        // `class_declaration` covers `class Foo {}`; `class` covers
        // anonymous class expressions (`const x = class { … }`). Both
        // push their name (or `<anon>`) onto the scope stack so any
        // nested `method_definition` can compose its qualified name
        // with the enclosing class. Nested classes compose with `::`
        // (`Outer::Inner::method`), the natural extension of the
        // single-class case.
        "class_declaration" | "class" => {
            let class_name = name_field(node, src).unwrap_or_else(|| "<anon>".to_owned());
            scope.push(class_name);
            update_type_density(node, src, &mut facts.type_density);
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk(child, src, stem, scope, facts);
            }
            scope.pop();
            return;
        }
        _ => {}
    }

    // Type-density tally on this node. Only counts at value-binding
    // sites (parameters and variable declarators) to keep the metric
    // legible: "of N value bindings, how many are typed?"
    update_type_density(node, src, &mut facts.type_density);

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, src, stem, scope, facts);
    }
}

fn parse_import(node: Node<'_>, src: &[u8]) -> Option<ImportFact> {
    // import_statement → import_clause? "from" string
    // The `source` field on `import_statement` points at the string
    // literal (one of the few well-named fields in the TS grammar).
    let source_node = node.child_by_field_name("source")?;
    let source = string_literal_text(source_node, src)?;
    let mut symbols = Vec::new();
    if let Some(clause) = first_child_kind(node, "import_clause") {
        collect_import_symbols(clause, src, &mut symbols);
    }
    Some(ImportFact { source, symbols })
}

fn collect_import_symbols(node: Node<'_>, src: &[u8], out: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                if let Ok(name) = child.utf8_text(src) {
                    out.push(name.to_owned());
                }
            }
            "namespace_import" => {
                // `* as name` — pull the trailing identifier.
                let mut sub = child.walk();
                for sub_child in child.children(&mut sub) {
                    if sub_child.kind() == "identifier" {
                        if let Ok(name) = sub_child.utf8_text(src) {
                            out.push(name.to_owned());
                        }
                    }
                }
            }
            "named_imports" => {
                let mut sub = child.walk();
                for spec in child.children(&mut sub) {
                    if spec.kind() == "import_specifier" {
                        // name field is the imported name; alias
                        // field is the local binding. Convention
                        // is to track the imported name — that's
                        // what STRUCTURE compares across siblings.
                        let name_node = spec
                            .child_by_field_name("name")
                            .or_else(|| first_child_kind(spec, "identifier"));
                        if let Some(n) = name_node {
                            if let Ok(name) = n.utf8_text(src) {
                                out.push(name.to_owned());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn parse_export(node: Node<'_>, src: &[u8], stem: &str) -> Option<ExportFact> {
    // `export const X = …`, `export function X() {}`, `export class X {}`,
    // `export type X = …`. The wrapped declaration tells us which.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name) = name_field(child, src) {
                    return Some(make_export(name, ExportKind::Function, stem));
                }
            }
            "class_declaration" => {
                if let Some(name) = name_field(child, src) {
                    return Some(make_export(name, ExportKind::Class, stem));
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                // Pull the first declarator's name (`export const X = …`).
                let mut sub = child.walk();
                for decl in child.children(&mut sub) {
                    if decl.kind() == "variable_declarator" {
                        if let Some(name) = name_field(decl, src) {
                            return Some(make_export(name, ExportKind::Const, stem));
                        }
                    }
                }
            }
            "type_alias_declaration" | "interface_declaration" => {
                if let Some(name) = name_field(child, src) {
                    return Some(make_export(name, ExportKind::Type, stem));
                }
            }
            _ => {}
        }
    }
    None
}

fn make_export(name: String, kind: ExportKind, stem: &str) -> ExportFact {
    let template_stem = template_for(stem, &name);
    ExportFact {
        name,
        kind,
        template_stem,
    }
}

fn name_field(node: Node<'_>, src: &[u8]) -> Option<String> {
    let n = node.child_by_field_name("name")?;
    n.utf8_text(src).ok().map(str::to_owned)
}

fn parse_function(node: Node<'_>, src: &[u8], scope: &[String]) -> Option<FunctionFact> {
    let name = name_field(node, src)?;
    let body = node
        .child_by_field_name("body")
        .or_else(|| first_child_kind(node, "statement_block"))?;
    let start_line = node.start_position().row;
    let end_line = body.end_position().row;
    let loc = u32::try_from(end_line - start_line + 1).unwrap_or(u32::MAX);
    let max_nesting_depth = max_nesting(body, 1);
    let qualified_name = if scope.is_empty() {
        name.clone()
    } else {
        format!("{}::{name}", scope.join("::"))
    };
    Some(FunctionFact {
        name,
        qualified_name,
        loc,
        max_nesting_depth,
    })
}

/// Recursively find the deepest `if/for/while/switch/try` nest.
/// `depth` is the depth of `node`; control-flow children push
/// depth+1.
fn max_nesting(node: Node<'_>, depth: u32) -> u32 {
    let mut deepest = depth;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let child_depth = if is_control_flow(child.kind()) {
            depth + 1
        } else {
            depth
        };
        let sub = max_nesting(child, child_depth);
        if sub > deepest {
            deepest = sub;
        }
    }
    deepest
}

fn is_control_flow(kind: &str) -> bool {
    matches!(
        kind,
        "if_statement"
            | "for_statement"
            | "for_in_statement"
            | "for_of_statement"
            | "while_statement"
            | "do_statement"
            | "switch_statement"
            | "try_statement"
            | "catch_clause"
    )
}

fn update_type_density(node: Node<'_>, src: &[u8], density: &mut TypeDensity) {
    // We tally at two value-binding sites: function parameters and
    // top-level variable declarators. Counting every expression
    // would inflate the denominator with synthetic nodes; counting
    // only bindings tracks the legible "is this name typed" question.
    match node.kind() {
        "required_parameter" | "optional_parameter" | "variable_declarator" => {
            density.total_value_exprs = density.total_value_exprs.saturating_add(1);
            if let Some(ann) = node.child_by_field_name("type") {
                density.typed_value_exprs = density.typed_value_exprs.saturating_add(1);
                update_any_unknown(ann, src, density);
            }
        }
        _ => {}
    }
}

fn update_any_unknown(node: Node<'_>, src: &[u8], density: &mut TypeDensity) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // tree-sitter-typescript represents `any` / `unknown` as a
        // `predefined_type` node containing the literal text. Only
        // count at the outer wrapper to avoid double-counting when
        // the grammar exposes both the wrapper and an inner literal.
        if child.kind() == "predefined_type" {
            if let Ok(text) = child.utf8_text(src) {
                match text {
                    "any" => density.any_count = density.any_count.saturating_add(1),
                    "unknown" => density.unknown_count = density.unknown_count.saturating_add(1),
                    _ => {}
                }
            }
        } else {
            update_any_unknown(child, src, density);
        }
    }
}

fn string_literal_text(node: Node<'_>, src: &[u8]) -> Option<String> {
    // `string` node contains a string_fragment child with the
    // unquoted text. Falling back to the raw text minus quotes
    // covers cases where the grammar surfaces the fragment
    // differently across versions.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string_fragment" {
            if let Ok(text) = child.utf8_text(src) {
                return Some(text.to_owned());
            }
        }
    }
    let raw = node.utf8_text(src).ok()?;
    let trimmed = raw.trim_matches(|c| c == '"' || c == '\'' || c == '`');
    Some(trimmed.to_owned())
}

fn first_child_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    let result = node
        .children(&mut cursor)
        .find(|child| child.kind() == kind);
    result
}

#[cfg(test)]
mod tests {
    use super::TsAdapter;
    use crate::adapter::LanguageAdapter;
    use std::path::Path;

    #[test]
    fn extracts_named_import() {
        let body = "import { foo, bar as Baz } from \"./mod\";\n";
        let f = TsAdapter
            .extract(Path::new("a.ts"), body)
            .expect("ts parse");
        assert_eq!(f.imports.len(), 1);
        assert_eq!(f.imports[0].source, "./mod");
        assert_eq!(f.imports[0].symbols, vec!["foo", "bar"]);
    }

    #[test]
    fn extracts_default_import() {
        let body = "import React from 'react';\n";
        let f = TsAdapter
            .extract(Path::new("a.ts"), body)
            .expect("ts parse");
        assert_eq!(f.imports.len(), 1);
        assert_eq!(f.imports[0].source, "react");
        assert_eq!(f.imports[0].symbols, vec!["React"]);
    }

    #[test]
    fn export_function_template_replaces_pascal_stem() {
        // Locks the doc claim: file `award.tsx` exporting
        // `CreateAwardDialog` produces template_stem `Create*Dialog`.
        let body = "export function CreateAwardDialog() { return null; }\n";
        let f = TsAdapter
            .extract(Path::new("award.tsx"), body)
            .expect("ts parse");
        assert_eq!(f.exports.len(), 1);
        let e = &f.exports[0];
        assert_eq!(e.name, "CreateAwardDialog");
        assert_eq!(e.template_stem, "Create*Dialog");
    }

    #[test]
    fn export_const_arrow_captures_kind_const() {
        let body = "export const Foo = () => 1;\n";
        let f = TsAdapter
            .extract(Path::new("foo.ts"), body)
            .expect("ts parse");
        assert_eq!(f.exports.len(), 1);
        assert_eq!(f.exports[0].name, "Foo");
    }

    #[test]
    fn function_loc_and_nesting_computed() {
        let body = r"
function deep() {
  if (a) {
    if (b) {
      return 1;
    }
  }
  return 0;
}
";
        let f = TsAdapter
            .extract(Path::new("a.ts"), body)
            .expect("ts parse");
        assert_eq!(f.functions.len(), 1);
        let fun = &f.functions[0];
        assert_eq!(fun.name, "deep");
        // body depth=1, outer if=2, inner if=3
        assert_eq!(fun.max_nesting_depth, 3);
        assert!(fun.loc >= 7);
    }

    #[test]
    fn qualified_name_distinguishes_methods_across_classes() {
        // Two classes each with a `constructor` — bare name collides;
        // qualified_name disambiguates so the COMPLEXITY HEAD-baseline
        // filter can match the right baseline per method.
        let body = "class Outer { constructor() { return 1; } }\n\
                    class Inner { constructor() { return 2; } }\n";
        let f = TsAdapter
            .extract(Path::new("a.ts"), body)
            .expect("ts parse");
        let names: Vec<&str> = f
            .functions
            .iter()
            .map(|fun| fun.qualified_name.as_str())
            .collect();
        assert!(
            names.contains(&"Outer::constructor"),
            "expected Outer::constructor; got: {names:?}"
        );
        assert!(
            names.contains(&"Inner::constructor"),
            "expected Inner::constructor; got: {names:?}"
        );
    }

    #[test]
    fn qualified_name_for_top_level_function_is_bare() {
        let body = "function topLevel() { return 1; }\n";
        let f = TsAdapter
            .extract(Path::new("a.ts"), body)
            .expect("ts parse");
        assert_eq!(f.functions.len(), 1);
        assert_eq!(f.functions[0].name, "topLevel");
        assert_eq!(f.functions[0].qualified_name, "topLevel");
    }

    #[test]
    fn type_density_counts_typed_params_and_any() {
        let body = "function f(a: number, b, c: any) { return a; }\n";
        let f = TsAdapter
            .extract(Path::new("a.ts"), body)
            .expect("ts parse");
        assert_eq!(f.type_density.total_value_exprs, 3);
        assert_eq!(f.type_density.typed_value_exprs, 2);
        assert_eq!(f.type_density.any_count, 1);
    }
}
