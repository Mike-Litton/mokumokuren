//! Direct tests for `mmk_git::binary` — the gate that decides
//! whether a blob enters the metrics pipeline at all.
//!
//! `is_binary` mirrors git's NUL-byte heuristic over the first 8 KiB.
//! `count_lines` runs on whatever passed `is_binary == false`. Both
//! are short enough to inspect by eye but load-bearing: a regression
//! in `is_binary` would either flood the metrics with binary noise
//! (false negative) or silently drop legitimate text (false
//! positive).

use mmk_git::binary::{count_lines, is_binary};
use proptest::collection::vec;
use proptest::prelude::*;
use rstest::rstest;

// ---- is_binary -----------------------------------------------------------

#[rstest]
#[case::empty(&[][..], false)]
#[case::pure_ascii(b"hello world\n", false)]
#[case::utf8_bom(&[0xEF, 0xBB, 0xBF, b'h', b'i'], false)]
#[case::high_bytes_no_nul(&[0xFF, 0xFE, 0xFD, b'a'], false)]
#[case::nul_byte_mid_string(&[b'a', 0u8, b'b'], true)]
#[case::nul_at_start(&[0u8, b'a'], true)]
#[case::nul_at_end(&[b'a', 0u8], true)]
fn is_binary_examples(#[case] body: &[u8], #[case] expected: bool) {
    assert_eq!(
        is_binary(body),
        expected,
        "is_binary({body:?}) expected {expected}",
    );
}

#[test]
fn is_binary_only_inspects_first_8k() {
    // Sniff window is 8 KiB. Place the NUL just past the window so a
    // future tweak that inflates the window past 8 KiB (or shrinks
    // below) trips this test.
    let mut body = vec![b'a'; 8 * 1024 + 1];
    body[8 * 1024] = 0;
    assert!(
        !is_binary(&body),
        "NUL beyond the 8 KiB window must NOT be detected (matches git's heuristic)"
    );

    let mut just_in = vec![b'a'; 8 * 1024];
    just_in[8 * 1024 - 1] = 0;
    assert!(
        is_binary(&just_in),
        "NUL at index 8191 (last byte of window) must be detected"
    );
}

// ---- count_lines ---------------------------------------------------------

#[rstest]
#[case::empty(&b""[..], 0)]
#[case::single_no_newline(&b"hello"[..], 1)]
#[case::single_trailing_newline(&b"hello\n"[..], 1)]
#[case::two_trailing_newline(&b"a\nb\n"[..], 2)]
#[case::two_no_trailing(&b"a\nb"[..], 2)]
#[case::three_with_blanks(&b"a\n\n\n"[..], 3)]
fn count_lines_examples(#[case] body: &[u8], #[case] expected: u32) {
    assert_eq!(count_lines(body), expected, "count_lines({body:?})");
}

// ---- properties ----------------------------------------------------------

proptest! {
    /// Bytes past the 8-KiB sniff window do not affect `is_binary`.
    /// Always uses a full-window head so the load-bearing assertion
    /// fires on every generated case (an earlier version conditioned
    /// on `head.len() == 8 KiB` and effectively ran the trivial
    /// direction on most inputs).
    #[test]
    fn is_binary_ignores_bytes_past_sniff_limit(
        tail in vec(0u8..=255, 1..1024),
    ) {
        // Fixed full-window head, no NUL bytes — the sniff must
        // report `false` on the head alone.
        let head: Vec<u8> = vec![b'a'; 8 * 1024];
        prop_assert!(!is_binary(&head));

        // Appending arbitrary bytes (potentially containing NULs)
        // must not flip the result, because the sniff stops at 8 KiB.
        let with_tail: Vec<u8> = head.iter().chain(tail.iter()).copied().collect();
        prop_assert!(
            !is_binary(&with_tail),
            "tail bytes (including NULs) past the 8 KiB sniff limit must not flip is_binary",
        );
    }

    /// `count_lines` is bounded by `bytes.len() + 1`. Each line
    /// requires at least one byte; the +1 is the no-trailing-newline
    /// "phantom" line. Locks the saturating-add behaviour against a
    /// regression that miscounted on degenerate input.
    #[test]
    fn count_lines_bounded_by_byte_length(body in vec(0u8..255, 0..2_000)) {
        let n = count_lines(&body);
        prop_assert!(
            u64::from(n) <= body.len() as u64 + 1,
            "count_lines = {n} exceeds bytes.len()+1 = {}",
            body.len() + 1,
        );
    }

    /// Empty body ⇒ exactly 0 lines. Non-empty body ⇒ ≥ 1 line.
    /// Pins the boundary that the docs document and downstream LOC
    /// metrics depend on.
    #[test]
    fn count_lines_zero_iff_empty(body in vec(0u8..255, 0..200)) {
        let n = count_lines(&body);
        if body.is_empty() {
            prop_assert_eq!(n, 0);
        } else {
            prop_assert!(n >= 1, "non-empty body produced {n} lines");
        }
    }

    /// Adding a trailing newline to a non-empty no-trailing-newline
    /// body never changes the line count: the trailing newline is
    /// what the "phantom" line was already accounting for. Generates
    /// the body in canonical no-trailing-newline form via prop_filter
    /// — a `pop()`-then-`push('\n')` shape silently mishandles
    /// multi-`\n` tails (one pop is not enough) and empties the body
    /// for `[b'\n']` (count differs because empty is the documented
    /// 0 case), and proptest had been finding both shrunk inputs.
    #[test]
    fn count_lines_invariant_on_terminator(
        body in vec(0u8..255, 1..200)
            .prop_filter(
                "body must be non-empty and not end with newline",
                |b| !b.is_empty() && b.last() != Some(&b'\n'),
            ),
    ) {
        let count_without = count_lines(&body);
        let mut with_terminator = body;
        with_terminator.push(b'\n');
        let count_with = count_lines(&with_terminator);
        prop_assert_eq!(
            count_without, count_with,
            "terminator should not change the line count",
        );
    }
}
