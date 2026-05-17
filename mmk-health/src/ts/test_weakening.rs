//! TEST_WEAKENING — net erosion of a test file's strength.
//!
//! Targets the agent-self-validation failure mode documented in
//! arXiv:2503.15223 *"Are 'Solved Issues' in SWE-bench Really Solved
//! Correctly?"* — agents passing CI by weakening the tests rather
//! than fixing the implementation. The shape: a working-tree diff
//! adds `.skip` / `.only` decorators, deletes `expect()` assertions,
//! inserts `jest.mock` / `vi.mock` calls, sprinkles
//! `@ts-expect-error` / `@ts-ignore`, or removes whole
//! `it` / `test` / `describe` cases.
//!
//! Like EVASION the detector compares working tree against HEAD;
//! only files whose names match the test-file convention (.test.*,
//! .spec.*) are eligible — impl edits never fire here. A finding
//! fires when at least one erosion axis strictly worsens; the
//! per-axis counts surface in `HealthFindingDetail::TestWeakening`
//! so consumers can branch on which axis tipped without re-parsing.

use crate::ts::{parse_for, test_pair};
use crate::{HealthFinding, HealthFindingDetail, HealthPattern};
use std::path::Path;
use tree_sitter::Node;

/// Detect net erosion of test strength in `working_body` relative
/// to `head_body`.
///
/// `head_body == None` means a new file (no HEAD blob to compare
/// against). New test files don't fire — there's no "weakening"
/// without a baseline, and a brand-new weak test is a different
/// failure mode (test_pair / coverage tooling).
#[must_use]
pub fn detect(
    subject: &Path,
    head_body: Option<&str>,
    working_body: &str,
) -> Vec<HealthFinding> {
    if !is_test_file(subject) {
        return Vec::new();
    }
    let Some(head_body) = head_body else {
        return Vec::new();
    };
    let head = count_axes(subject, head_body);
    let working = count_axes(subject, working_body);

    let skips_added = working.skips.saturating_sub(head.skips);
    let assertions_lost = head.assertions.saturating_sub(working.assertions);
    let mocks_added = working.mocks.saturating_sub(head.mocks);
    let ts_suppressions_added = working
        .ts_suppressions
        .saturating_sub(head.ts_suppressions);
    let tests_removed = head.test_cases.saturating_sub(working.test_cases);

    if skips_added == 0
        && assertions_lost == 0
        && mocks_added == 0
        && ts_suppressions_added == 0
        && tests_removed == 0
    {
        return Vec::new();
    }

    vec![HealthFinding {
        pattern: HealthPattern::TestWeakening,
        subject: subject.to_path_buf(),
        related: Vec::new(),
        detail: Some(HealthFindingDetail::TestWeakening {
            skips_added,
            assertions_lost,
            mocks_added,
            ts_suppressions_added,
            tests_removed,
        }),
    }]
}

#[derive(Debug, Default, Clone, Copy)]
struct Axes {
    /// Calls to skip / only / xit / xtest / xdescribe.
    skips: u32,
    /// `expect(...)` invocations.
    assertions: u32,
    /// `jest.mock(...)` / `vi.mock(...)` / standalone `mock(...)` calls.
    mocks: u32,
    /// `@ts-expect-error` / `@ts-ignore` comment markers.
    ts_suppressions: u32,
    /// `it(...)` / `test(...)` / `describe(...)` invocations
    /// (counts the test cases / suites; a deletion here means an
    /// entire case was dropped).
    test_cases: u32,
}

fn count_axes(subject: &Path, body: &str) -> Axes {
    let Some(tree) = parse_for(subject, body) else {
        return Axes::default();
    };
    let src = body.as_bytes();
    let mut axes = Axes::default();
    walk(tree.root_node(), src, &mut axes);
    axes.ts_suppressions = count_ts_suppressions(body);
    axes
}

fn walk(node: Node<'_>, src: &[u8], axes: &mut Axes) {
    if node.kind() == "call_expression" {
        classify_call(node, src, axes);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, src, axes);
    }
}

fn classify_call(call: Node<'_>, src: &[u8], axes: &mut Axes) {
    let Some(callee) = call.child_by_field_name("function") else {
        return;
    };
    match callee.kind() {
        "identifier" => {
            if let Ok(name) = callee.utf8_text(src) {
                match name {
                    "expect" => axes.assertions = axes.assertions.saturating_add(1),
                    "it" | "test" | "describe" => {
                        axes.test_cases = axes.test_cases.saturating_add(1);
                    }
                    "xit" | "xtest" | "xdescribe" => {
                        axes.skips = axes.skips.saturating_add(1);
                    }
                    _ => {}
                }
            }
        }
        "member_expression" => {
            classify_member_call(callee, src, axes);
        }
        _ => {}
    }
}

fn classify_member_call(callee: Node<'_>, src: &[u8], axes: &mut Axes) {
    let Some(object) = callee.child_by_field_name("object") else {
        return;
    };
    let Some(property) = callee.child_by_field_name("property") else {
        return;
    };
    let Ok(prop_name) = property.utf8_text(src) else {
        return;
    };
    let object_name = if object.kind() == "identifier" {
        object.utf8_text(src).ok()
    } else {
        None
    };
    if matches!(object_name, Some("it" | "test" | "describe"))
        && matches!(prop_name, "skip" | "only")
    {
        axes.skips = axes.skips.saturating_add(1);
        // Keep test_cases steady so an `it(...)` → `it.skip(...)`
        // edit reads as skips=+1 only, not skips=+1 / tests=−1.
        axes.test_cases = axes.test_cases.saturating_add(1);
        return;
    }
    if matches!(object_name, Some("jest" | "vi")) && prop_name == "mock" {
        axes.mocks = axes.mocks.saturating_add(1);
    }
}

fn count_ts_suppressions(body: &str) -> u32 {
    let mut count: u32 = 0;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("//") && !trimmed.starts_with("/*") && !trimmed.starts_with("*") {
            continue;
        }
        if trimmed.contains("@ts-expect-error") || trimmed.contains("@ts-ignore") {
            count = count.saturating_add(1);
        }
    }
    count
}

/// Inverse selector to `test_pair::is_implementation_file`:
/// test_weakening's subject is the test itself, not its impl partner.
fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(test_pair::is_test_file_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ts_test_subject() -> PathBuf {
        PathBuf::from("src/foo.test.ts")
    }

    #[test]
    fn skip_decorator_added_fires() {
        let head = "describe('f', () => { it('works', () => { expect(1).toBe(1); }); });";
        let working = "describe('f', () => { it.skip('works', () => { expect(1).toBe(1); }); });";
        let f = detect(&ts_test_subject(), Some(head), working);
        assert_eq!(f.len(), 1, "added .skip should fire; got {f:?}");
    }

    #[test]
    fn assertion_count_decreased_fires() {
        let head = "test('f', () => { expect(a).toBe(1); expect(b).toBe(2); });";
        let working = "test('f', () => { expect(a).toBe(1); });";
        let f = detect(&ts_test_subject(), Some(head), working);
        assert_eq!(f.len(), 1, "lost assertion should fire; got {f:?}");
    }

    #[test]
    fn mock_added_fires() {
        let head = "test('f', () => { expect(g()).toBe(1); });";
        let working =
            "jest.mock('../dep');\ntest('f', () => { expect(g()).toBe(1); });";
        let f = detect(&ts_test_subject(), Some(head), working);
        assert_eq!(f.len(), 1, "jest.mock added should fire; got {f:?}");
    }

    #[test]
    fn ts_expect_error_added_fires() {
        let head = "test('f', () => { expect(g()).toBe(1); });";
        let working =
            "test('f', () => {\n  // @ts-expect-error wonky\n  expect(g()).toBe(1);\n});";
        let f = detect(&ts_test_subject(), Some(head), working);
        assert_eq!(f.len(), 1, "@ts-expect-error added should fire; got {f:?}");
    }

    #[test]
    fn test_case_removed_fires() {
        let head = "describe('f', () => { it('a', () => { expect(1).toBe(1); }); it('b', () => { expect(2).toBe(2); }); });";
        let working = "describe('f', () => { it('a', () => { expect(1).toBe(1); }); });";
        let f = detect(&ts_test_subject(), Some(head), working);
        assert_eq!(f.len(), 1, "removed test case should fire; got {f:?}");
    }

    #[test]
    fn new_test_file_does_not_fire() {
        // No HEAD baseline → nothing to weaken against; this is a
        // different failure mode (test_pair / coverage gap).
        let working = "test('f', () => {});";
        let f = detect(&ts_test_subject(), None, working);
        assert!(f.is_empty(), "new test file must not fire; got {f:?}");
    }

    #[test]
    fn impl_file_edits_do_not_fire() {
        // Only test files are subjects.
        let impl_subject = PathBuf::from("src/foo.ts");
        let head = "export function f() { return 1; }";
        let working = "export function f() { return 2; }";
        let f = detect(&impl_subject, Some(head), working);
        assert!(f.is_empty(), "impl-file edit must not fire; got {f:?}");
    }

    #[test]
    fn unchanged_test_does_not_fire() {
        let body = "test('f', () => { expect(1).toBe(1); });";
        let f = detect(&ts_test_subject(), Some(body), body);
        assert!(f.is_empty(), "unchanged test must not fire; got {f:?}");
    }

    #[test]
    fn additions_alone_do_not_fire() {
        // Adding tests / assertions is a strengthening, not a
        // weakening — must stay silent.
        let head = "test('a', () => { expect(1).toBe(1); });";
        let working = "test('a', () => { expect(1).toBe(1); expect(2).toBe(2); });\n\
                       test('b', () => { expect(3).toBe(3); });";
        let f = detect(&ts_test_subject(), Some(head), working);
        assert!(
            f.is_empty(),
            "additions-only diff must not fire; got {f:?}"
        );
    }

    #[test]
    fn vi_mock_added_fires() {
        let head = "test('f', () => { expect(g()).toBe(1); });";
        let working =
            "vi.mock('../dep');\ntest('f', () => { expect(g()).toBe(1); });";
        let f = detect(&ts_test_subject(), Some(head), working);
        assert_eq!(f.len(), 1, "vi.mock added should fire; got {f:?}");
    }

    #[test]
    fn xit_swap_fires() {
        let head = "describe('f', () => { it('works', () => { expect(1).toBe(1); }); });";
        let working = "describe('f', () => { xit('works', () => { expect(1).toBe(1); }); });";
        let f = detect(&ts_test_subject(), Some(head), working);
        assert_eq!(f.len(), 1, "xit substitution should fire; got {f:?}");
    }

    #[test]
    fn jsx_test_subject_uses_tsx_grammar() {
        // .test.tsx with JSX in a test body: must parse via LANGUAGE_TSX.
        let head =
            "test('renders', () => { expect(<div />).toBeTruthy(); expect(<span />).toBeTruthy(); });";
        let working = "test('renders', () => { expect(<div />).toBeTruthy(); });";
        let subject = PathBuf::from("src/App.test.tsx");
        let f = detect(&subject, Some(head), working);
        assert_eq!(f.len(), 1, "tsx test must be parsed correctly; got {f:?}");
    }
}
