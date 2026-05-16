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
///
/// `log_identifiers` controls which object identifiers count as a
/// "logger" for the log-and-swallow predicate (`logger.warn(...)`
/// alone in a catch body without a rethrow). Defaults are propagated
/// from `mmk-config::HealthTsCfg` at the CLI boundary; pure detector
/// callers (tests, future programmatic users) supply their own.
#[must_use]
pub fn detect(
    subject: &Path,
    head_body: Option<&str>,
    working_body: &str,
    log_identifiers: &[String],
) -> Vec<HealthFinding> {
    let working_count = count_broad_non_top_level(subject, working_body, log_identifiers);
    let head_count = head_body.map_or(0, |h| {
        count_broad_non_top_level(subject, h, log_identifiers)
    });
    if working_count > head_count {
        vec![HealthFinding {
            pattern: HealthPattern::BroadException,
            subject: subject.to_path_buf(),
            related: Vec::new(),
            detail: None,
        }]
    } else {
        Vec::new()
    }
}

pub(crate) fn count_broad_non_top_level(
    subject: &Path,
    body: &str,
    log_identifiers: &[String],
) -> u32 {
    let Some(tree) = parse_for(subject, body) else {
        return 0;
    };
    let src = body.as_bytes();
    let mut count = 0u32;
    walk(tree.root_node(), src, log_identifiers, &mut count);
    count
}

/// Locate every broad non-top-level catch handler and return its
/// `(line, column)` start position (1-based line, 1-based column).
/// Used by the `BroadCatchDebt` static-mode detector.
#[must_use]
pub(crate) fn collect_broad_non_top_level_with_locations(
    subject: &Path,
    body: &str,
    log_identifiers: &[String],
) -> Vec<(usize, usize)> {
    let Some(tree) = parse_for(subject, body) else {
        return Vec::new();
    };
    let src = body.as_bytes();
    let mut out = Vec::new();
    walk_collect(tree.root_node(), src, log_identifiers, &mut out);
    out
}

fn walk(node: Node<'_>, src: &[u8], log_identifiers: &[String], count: &mut u32) {
    if node.kind() == "catch_clause" && is_broad(node, src, log_identifiers) && !is_top_level(node)
    {
        *count = count.saturating_add(1);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, src, log_identifiers, count);
    }
}

fn walk_collect(
    node: Node<'_>,
    src: &[u8],
    log_identifiers: &[String],
    out: &mut Vec<(usize, usize)>,
) {
    if node.kind() == "catch_clause" && is_broad(node, src, log_identifiers) && !is_top_level(node)
    {
        let pos = node.start_position();
        out.push((pos.row + 1, pos.column + 1));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_collect(child, src, log_identifiers, out);
    }
}

/// A catch is "broad" when it suppresses arbitrary errors:
///   - empty body (no statements inside the `statement_block`), OR
///   - no parameter at all (TS/JS `try { ... } catch { ... }`), OR
///   - parameter typed as `any | unknown | Error` (TS-specific
///     surface — JS catches don't carry annotations, so this branch
///     stays inert for `.js` / `.jsx`), OR
///   - body is exclusively log calls on a configured log identifier
///     (the dominant TypeScript log-and-swallow shape, e.g.
///     `catch (e) { logger.warn(e); }`).
fn is_broad(node: Node<'_>, src: &[u8], log_identifiers: &[String]) -> bool {
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
    if is_log_and_swallow(node, src, log_identifiers) {
        return true;
    }
    false
}

/// Recognize the dominant TypeScript log-and-swallow shape:
/// `catch (e) { logger.warn(e); }` — body is one or more
/// expression-statements that are member calls on a configured log
/// identifier, with no `throw` / `return` of an error. Empty bodies
/// and rethrows are handled by the other predicates.
fn is_log_and_swallow(catch_node: Node<'_>, src: &[u8], log_identifiers: &[String]) -> bool {
    if log_identifiers.is_empty() {
        return false;
    }
    let Some(body) = catch_node.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    let mut had_log_call = false;
    for stmt in body.children(&mut cursor) {
        if !stmt.is_named() {
            continue;
        }
        // Anything other than an expression-statement disqualifies:
        // throw/return/if/etc. all break the simple log-and-swallow
        // shape we target. (Throws and returns specifically would
        // make the catch non-swallowing.) Inside expression statements
        // we further require the call to be a member-call on a
        // configured log identifier.
        if stmt.kind() != "expression_statement"
            || !is_log_call_expression_statement(stmt, src, log_identifiers)
        {
            return false;
        }
        had_log_call = true;
    }
    had_log_call
}

/// True when the `expression_statement` wraps a `call_expression`
/// whose callee is a member access (`<obj>.<method>(...)`) and `<obj>`
/// is one of the configured log identifiers.
fn is_log_call_expression_statement(
    stmt: Node<'_>,
    src: &[u8],
    log_identifiers: &[String],
) -> bool {
    let Some(call) = first_named_child(stmt) else {
        return false;
    };
    if call.kind() != "call_expression" {
        return false;
    }
    let Some(callee) = call.child_by_field_name("function") else {
        return false;
    };
    if callee.kind() != "member_expression" {
        return false;
    }
    let Some(object) = callee.child_by_field_name("object") else {
        return false;
    };
    if object.kind() != "identifier" {
        return false;
    }
    let Ok(text) = object.utf8_text(src) else {
        return false;
    };
    log_identifiers.iter().any(|id| id == text)
}

fn first_named_child(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    let result = node.children(&mut cursor).find(tree_sitter::Node::is_named);
    result
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

    fn default_log_ids() -> Vec<String> {
        vec!["logger".into(), "log".into(), "console".into()]
    }

    #[test]
    fn empty_catch_body_added_in_function_fires() {
        let head = "function f() { try { g(); } catch (e) { throw e; } }";
        let working = "function f() { try { g(); } catch (e) {} }";
        let f = detect(&ts_subject(), Some(head), working, &default_log_ids());
        assert_eq!(f.len(), 1, "added empty catch body should fire; got {f:?}");
    }

    #[test]
    fn rethrow_preserved_does_not_fire() {
        let head = "function f() { try { g(); } catch (e) { handleError(e); throw e; } }";
        let working = head;
        let f = detect(&ts_subject(), Some(head), working, &default_log_ids());
        assert!(f.is_empty(), "rethrow keeps catch non-broad; got {f:?}");
    }

    #[test]
    fn top_level_broad_catch_does_not_fire() {
        // No enclosing function: module-level handler is legitimate.
        let working = "try { boot(); } catch {}\n";
        let f = detect(&ts_subject(), None, working, &default_log_ids());
        assert!(
            f.is_empty(),
            "top-level broad handler is legitimate; got {f:?}"
        );
    }

    #[test]
    fn function_local_broad_catch_fires_on_new_file() {
        let working = "function f() { try { g(); } catch {} }";
        let f = detect(&ts_subject(), None, working, &default_log_ids());
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
        let f = detect(&ts_subject(), Some(head), working, &default_log_ids());
        assert!(
            f.is_empty(),
            "relocation-only must not fire (count unchanged); got {f:?}"
        );
    }

    #[test]
    fn new_file_no_broad_catch_does_not_fire() {
        let working = "function f() { return 1; }";
        let f = detect(&ts_subject(), None, working, &default_log_ids());
        assert!(f.is_empty(), "new file with no catches; got {f:?}");
    }

    #[test]
    fn typed_any_catch_fires() {
        let head = "function f() { try { g(); } catch (e: number) { throw e; } }";
        let working = "function f() { try { g(); } catch (e: any) { throw e; } }";
        let f = detect(&ts_subject(), Some(head), working, &default_log_ids());
        assert_eq!(f.len(), 1, "any-typed catch is broad; got {f:?}");
    }

    #[test]
    fn typed_unknown_catch_fires() {
        let working = "function f() { try { g(); } catch (e: unknown) { handleError(e); } }";
        let f = detect(&ts_subject(), None, working, &default_log_ids());
        assert_eq!(f.len(), 1, "unknown-typed catch is broad; got {f:?}");
    }

    #[test]
    fn no_parameter_catch_fires() {
        let working = "function f() { try { g(); } catch { /* swallow */ } }";
        let f = detect(&ts_subject(), None, working, &default_log_ids());
        assert_eq!(f.len(), 1, "no-parameter catch is broad; got {f:?}");
    }

    #[test]
    fn jsx_subject_uses_tsx_grammar() {
        // .tsx with JSX content: must parse via LANGUAGE_TSX. If the
        // dispatch silently fell back to LANGUAGE_TYPESCRIPT, the
        // JSX would error and this finding would be missed.
        let working = "function App() { try { f(); } catch {} return <div />; }";
        let subject = PathBuf::from("src/App.tsx");
        let f = detect(&subject, None, working, &default_log_ids());
        assert_eq!(
            f.len(),
            1,
            "tsx with JSX must be parsed correctly; got {f:?}"
        );
    }

    // ---- v0.12 log-and-swallow extension --------------------------

    #[test]
    fn log_and_swallow_warn_fires() {
        let working = "function f() { try { g(); } catch (e) { logger.warn(e); } }";
        let f = detect(&ts_subject(), None, working, &default_log_ids());
        assert_eq!(f.len(), 1, "logger.warn-only catch is broad; got {f:?}");
    }

    #[test]
    fn log_and_swallow_error_fires() {
        let working = "function f() { try { g(); } catch (e) { logger.error('msg', e); } }";
        let f = detect(&ts_subject(), None, working, &default_log_ids());
        assert_eq!(f.len(), 1, "logger.error-only catch is broad; got {f:?}");
    }

    #[test]
    fn log_and_swallow_console_fires() {
        let working = "function f() { try { g(); } catch (err) { console.warn(err); } }";
        let f = detect(&ts_subject(), None, working, &default_log_ids());
        assert_eq!(f.len(), 1, "console.warn-only catch is broad; got {f:?}");
    }

    #[test]
    fn log_and_swallow_multi_call_fires() {
        let working =
            "function f() { try { g(); } catch (e) { logger.warn(e); logger.info('also'); } }";
        let f = detect(&ts_subject(), None, working, &default_log_ids());
        assert_eq!(f.len(), 1, "multi-call log-only catch is broad; got {f:?}");
    }

    #[test]
    fn log_then_rethrow_does_not_fire() {
        let working = "function f() { try { g(); } catch (e) { logger.warn(e); throw e; } }";
        let f = detect(&ts_subject(), None, working, &default_log_ids());
        assert!(f.is_empty(), "rethrow keeps catch non-broad; got {f:?}");
    }

    #[test]
    fn log_then_throw_new_does_not_fire() {
        let working =
            "function f() { try { g(); } catch (e) { logger.warn(e); throw new Error('x'); } }";
        let f = detect(&ts_subject(), None, working, &default_log_ids());
        assert!(f.is_empty(), "throw new keeps catch non-broad; got {f:?}");
    }

    #[test]
    fn non_logger_call_does_not_fire() {
        let working = "function f() { try { g(); } catch (e) { handleError(e); } }";
        let f = detect(&ts_subject(), None, working, &default_log_ids());
        assert!(
            f.is_empty(),
            "custom handler call is not log-and-swallow; got {f:?}"
        );
    }

    #[test]
    fn custom_log_identifier_via_config() {
        let working = "function f() { try { g(); } catch (e) { myLog.warn(e); } }";
        let ids = vec!["myLog".to_string()];
        let f = detect(&ts_subject(), None, working, &ids);
        assert_eq!(f.len(), 1, "custom log identifier fires; got {f:?}");
    }

    #[test]
    fn delta_semantics_unchanged() {
        // Two log-and-swallow handlers in HEAD; same in working.
        // The delta predicate must not retroactively trigger.
        let head = "function f() { try { g(); } catch (e) { logger.warn(e); } }\n\
                    function h() { try { g(); } catch (e) { logger.error(e); } }";
        let working = head;
        let f = detect(&ts_subject(), Some(head), working, &default_log_ids());
        assert!(
            f.is_empty(),
            "pre-existing log-and-swallow must not fire (delta=0); got {f:?}"
        );
    }
}
