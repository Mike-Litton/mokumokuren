//! EVASION — non-top-level broad-catch handler addition.
//!
//! Targets the *"evasive repairs with try-except blocks"* failure
//! mode named in arXiv:2509.13941 *(An Empirical Study on Failures in
//! Automated Issue Solving)* and corroborated by FSE 2025
//! *Suppressed Static Analysis Warnings* (broad-except = 18.4% of
//! Python suppressions across 46 projects). The shape: an LLM-edited
//! patch wraps the failing call site in `try { ... } catch {}` /
//! `catch (e) {}` / `catch (e: any) {}` to silence the failure
//! rather than fix it.
//!
//! The detector compares working tree against HEAD: only the
//! *addition* (working count > head count) fires a finding.
//! Module-level handlers are filtered out — those are typically
//! legitimate top-level error boundaries; the pathological case is
//! a swallowed error inside business logic.

use crate::ts::parse_for;
use crate::{HealthFinding, HealthPattern};
use std::path::Path;
use tree_sitter::Node;

/// Detect newly-added broad non-top-level catch handlers in
/// `working_body` relative to `head_body`.
///
/// `head_body == None` means a new file: a finding fires when the
/// working tree has at least one broad non-top-level handler.
#[must_use]
pub fn detect(subject: &Path, head_body: Option<&str>, working_body: &str) -> Vec<HealthFinding> {
    let working_count = count_broad_non_top_level(subject, working_body);
    let head_count = head_body.map_or(0, |h| count_broad_non_top_level(subject, h));
    if working_count > head_count {
        vec![HealthFinding {
            pattern: HealthPattern::BroadException,
            subject: subject.to_path_buf(),
            related: Vec::new(),
        }]
    } else {
        Vec::new()
    }
}

fn count_broad_non_top_level(subject: &Path, body: &str) -> u32 {
    let Some(tree) = parse_for(subject, body) else {
        return 0;
    };
    let src = body.as_bytes();
    let mut count = 0u32;
    walk(tree.root_node(), src, &mut count);
    count
}

fn walk(node: Node<'_>, src: &[u8], count: &mut u32) {
    if node.kind() == "catch_clause" && is_broad(node, src) && !is_top_level(node) {
        *count = count.saturating_add(1);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, src, count);
    }
}

/// A catch is "broad" when it suppresses arbitrary errors:
///   - empty body (no statements inside the `statement_block`), OR
///   - no parameter at all (TS/JS `try { ... } catch { ... }`), OR
///   - parameter typed as `any | unknown | Error` (TS-specific
///     surface — JS catches don't carry annotations, so this branch
///     stays inert for `.js` / `.jsx`).
fn is_broad(node: Node<'_>, src: &[u8]) -> bool {
    if catch_body_empty(node) {
        return true;
    }
    if !has_catch_parameter(node) {
        // `catch { ... }` shape — TS/JS allow it; broad by definition.
        return true;
    }
    if catch_param_type_is_broad(node, src) {
        return true;
    }
    false
}

/// Whether the `catch_clause` has a parameter at all. Field-name
/// access via tree-sitter is grammar-version-sensitive; matching by
/// node kind across direct children avoids that fragility.
fn has_catch_parameter(catch_node: Node<'_>) -> bool {
    let mut cursor = catch_node.walk();
    let result = catch_node
        .children(&mut cursor)
        .any(|c| matches!(c.kind(), "identifier" | "array_pattern" | "object_pattern"));
    result
}

/// Look at the catch parameter's UTF-8 text and decide whether the
/// declared type is broad. Substring-based on the parenthesized
/// parameter region — robust across grammar variations that wrap
/// the type in different node kinds.
fn catch_param_type_is_broad(catch_node: Node<'_>, src: &[u8]) -> bool {
    let Ok(catch_text) = catch_node.utf8_text(src) else {
        return false;
    };
    // Slice between the first `(` and matching `)`.
    let Some(open) = catch_text.find('(') else {
        return false;
    };
    let Some(close) = catch_text[open..].find(')') else {
        return false;
    };
    let param = &catch_text[open + 1..open + close];
    let Some(colon) = param.find(':') else {
        return false;
    };
    let type_text = param[colon + 1..].trim();
    matches!(type_text, "any" | "unknown" | "Error")
}

fn catch_body_empty(catch_node: Node<'_>) -> bool {
    // `catch_clause` has a `body` field pointing at a
    // `statement_block`. An empty block has no `_statement` children.
    let Some(body) = catch_node.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    let has_statement = body.children(&mut cursor).any(|c| c.is_named());
    !has_statement
}

/// A `catch_clause` is "top-level" when no enclosing
/// function/method/arrow ancestor sits between it and the program
/// root. Module-level error boundaries are legitimate; the
/// pathological case is a swallowed error inside business logic.
fn is_top_level(catch_node: Node<'_>) -> bool {
    let mut cur = catch_node.parent();
    while let Some(node) = cur {
        match node.kind() {
            "function_declaration"
            | "method_definition"
            | "arrow_function"
            | "function_expression"
            | "function"
            | "generator_function"
            | "generator_function_declaration" => return false,
            "program" => return true,
            _ => {}
        }
        cur = node.parent();
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ts_subject() -> PathBuf {
        PathBuf::from("src/foo.ts")
    }

    #[test]
    fn empty_catch_body_added_in_function_fires() {
        let head = "function f() { try { g(); } catch (e) { throw e; } }";
        let working = "function f() { try { g(); } catch (e) {} }";
        let f = detect(&ts_subject(), Some(head), working);
        assert_eq!(f.len(), 1, "added empty catch body should fire; got {f:?}");
    }

    #[test]
    fn rethrow_preserved_does_not_fire() {
        let head = "function f() { try { g(); } catch (e) { log(e); throw e; } }";
        let working = head;
        let f = detect(&ts_subject(), Some(head), working);
        assert!(f.is_empty(), "rethrow keeps catch non-broad; got {f:?}");
    }

    #[test]
    fn top_level_broad_catch_does_not_fire() {
        // No enclosing function: module-level handler is legitimate.
        let working = "try { boot(); } catch {}\n";
        let f = detect(&ts_subject(), None, working);
        assert!(
            f.is_empty(),
            "top-level broad handler is legitimate; got {f:?}"
        );
    }

    #[test]
    fn function_local_broad_catch_fires_on_new_file() {
        let working = "function f() { try { g(); } catch {} }";
        let f = detect(&ts_subject(), None, working);
        assert_eq!(
            f.len(),
            1,
            "new file with broad in-function catch; got {f:?}"
        );
    }

    #[test]
    fn relocation_with_unchanged_count_does_not_fire() {
        // 1 broad handler in HEAD (function f), 1 broad handler in
        // working (function g). Net delta = 0 → no finding.
        let head = "function f() { try { g(); } catch {} } function h() {}";
        let working = "function f() {} function h() { try { g(); } catch {} }";
        let f = detect(&ts_subject(), Some(head), working);
        assert!(
            f.is_empty(),
            "relocation-only must not fire (count unchanged); got {f:?}"
        );
    }

    #[test]
    fn new_file_no_broad_catch_does_not_fire() {
        let working = "function f() { return 1; }";
        let f = detect(&ts_subject(), None, working);
        assert!(f.is_empty(), "new file with no catches; got {f:?}");
    }

    #[test]
    fn typed_any_catch_fires() {
        let head = "function f() { try { g(); } catch (e: number) { throw e; } }";
        let working = "function f() { try { g(); } catch (e: any) { throw e; } }";
        let f = detect(&ts_subject(), Some(head), working);
        assert_eq!(f.len(), 1, "any-typed catch is broad; got {f:?}");
    }

    #[test]
    fn typed_unknown_catch_fires() {
        let working = "function f() { try { g(); } catch (e: unknown) { log(e); } }";
        let f = detect(&ts_subject(), None, working);
        assert_eq!(f.len(), 1, "unknown-typed catch is broad; got {f:?}");
    }

    #[test]
    fn no_parameter_catch_fires() {
        let working = "function f() { try { g(); } catch { /* swallow */ } }";
        let f = detect(&ts_subject(), None, working);
        assert_eq!(f.len(), 1, "no-parameter catch is broad; got {f:?}");
    }

    #[test]
    fn jsx_subject_uses_tsx_grammar() {
        // .tsx with JSX content: must parse via LANGUAGE_TSX. If the
        // dispatch silently fell back to LANGUAGE_TYPESCRIPT, the
        // JSX would error and this finding would be missed.
        let working = "function App() { try { f(); } catch {} return <div />; }";
        let subject = PathBuf::from("src/App.tsx");
        let f = detect(&subject, None, working);
        assert_eq!(
            f.len(),
            1,
            "tsx with JSX must be parsed correctly; got {f:?}"
        );
    }
}
