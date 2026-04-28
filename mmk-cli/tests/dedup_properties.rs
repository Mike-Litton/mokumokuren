//! Property tests for `mokumokuren::dedup` — order-independence and
//! TTL/HEAD/hash gate behaviour.
//!
//! `tests/dedup.rs` covers explicit truth-table cases. The properties
//! here lock the load-bearing invariants of `hash_findings`
//! (orientation-independent, deterministic) and `should_suppress`
//! (every gate is necessary, none sufficient on its own).

use mokumokuren::dedup::{hash_findings, should_suppress, DedupRecord};
use mokumokuren::output::findings::{Finding, Layer, Severity};
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::sample::Index;

fn layer_strategy() -> impl Strategy<Value = Layer> {
    prop_oneof![
        Just(Layer::Hotspot),
        Just(Layer::Coupling),
        Just(Layer::Cohesion),
        Just(Layer::Drift),
        Just(Layer::Budget),
        Just(Layer::Health),
        Just(Layer::Structure),
        Just(Layer::Complexity),
        Just(Layer::Anchor),
    ]
}

fn severity_strategy() -> impl Strategy<Value = Severity> {
    prop_oneof![
        Just(Severity::Warn),
        Just(Severity::Info),
        Just(Severity::Ok),
    ]
}

fn finding_strategy() -> impl Strategy<Value = Finding> {
    (layer_strategy(), severity_strategy(), "[a-z0-9 .:_]{1,40}")
        .prop_map(|(layer, severity, message)| Finding::new(layer, severity, message))
}

fn findings_strategy() -> impl Strategy<Value = Vec<Finding>> {
    vec(finding_strategy(), 0..20)
}

proptest! {
    /// `hash_findings` is order-independent. Sorting is the explicit
    /// pre-step; any future change that drops the sort would break
    /// the dedup contract on real agent traces (which emit the same
    /// finding-set in different orders depending on which sensor
    /// finished first).
    ///
    /// `swaps` is a vector of proptest `Index` values; each one
    /// picks a position to swap with the i-th element in a
    /// Fisher-Yates pass. Empty `swaps` ⇒ original order; non-empty
    /// ⇒ a real permutation. This avoids the prior hand-rolled
    /// shuffle's vacuous-identity case (single `j` value reused
    /// across iterations).
    #[test]
    fn hash_findings_is_order_independent(
        findings in findings_strategy(),
        swaps in vec(any::<Index>(), 0..32),
    ) {
        let h_canonical = hash_findings(&findings);
        let mut shuffled = findings.clone();
        let n = shuffled.len();
        if n > 1 {
            for (i, idx) in swaps.iter().enumerate() {
                let pos_a = i % n;
                let pos_b = idx.index(n);
                shuffled.swap(pos_a, pos_b);
            }
        }
        let h_shuffled = hash_findings(&shuffled);
        prop_assert_eq!(
            h_canonical, h_shuffled,
            "hash drifted under permutation: original={:?}, shuffled={:?}",
            findings.iter().map(|f| (&f.layer, &f.severity, &f.message)).collect::<Vec<_>>(),
            shuffled.iter().map(|f| (&f.layer, &f.severity, &f.message)).collect::<Vec<_>>(),
        );
    }

    /// Reverse-order is the simplest non-identity permutation. Catches
    /// the boundary case where the more-elaborate shuffle could miss
    /// a "every element moved" scenario.
    #[test]
    fn hash_findings_invariant_under_reverse(mut findings in findings_strategy()) {
        let h_forward = hash_findings(&findings);
        findings.reverse();
        let h_reverse = hash_findings(&findings);
        prop_assert_eq!(h_forward, h_reverse);
    }

    /// `hash_findings` is deterministic across calls on the same
    /// input. The `RandomState::with_seeds(0,0,0,0)` constructor
    /// hard-codes the seed; any future swap to default `RandomState`
    /// would silently break this and bypass dedup on every fire.
    #[test]
    fn hash_findings_is_deterministic(findings in findings_strategy()) {
        let h1 = hash_findings(&findings);
        let h2 = hash_findings(&findings);
        prop_assert_eq!(h1, h2);
    }

    /// `should_suppress` requires every gate. Drop any one — hash,
    /// HEAD, or TTL — and the function must return false. All three
    /// must hold; a regression that loosened the conjunction (any
    /// one of three sufficient) would silently re-suppress findings
    /// the agent should re-see.
    #[test]
    fn should_suppress_requires_every_gate(
        prior_hash in any::<u64>(),
        prior_head in "[a-f0-9]{6,40}",
        prior_ts in 0i64..1_000_000,
        ttl in 1i64..86_400,
        offset in 0i64..1_000,
    ) {
        let prior = DedupRecord {
            findings_hash: prior_hash,
            head_sha: prior_head.clone(),
            emitted_at: prior_ts,
        };
        let now = prior_ts + offset;

        // All match + within TTL ⇒ suppressed.
        if offset < ttl {
            prop_assert!(
                should_suppress(prior_hash, &prior_head, Some(&prior), now, ttl),
                "all-match within TTL should suppress",
            );
        }
        // Different hash ⇒ not suppressed.
        prop_assert!(
            !should_suppress(prior_hash.wrapping_add(1), &prior_head, Some(&prior), now, ttl),
            "differing hash must flush suppression",
        );
        // Different HEAD ⇒ not suppressed.
        prop_assert!(
            !should_suppress(prior_hash, "ffff_different_head", Some(&prior), now, ttl),
            "differing HEAD must flush suppression",
        );
        // Past TTL ⇒ not suppressed.
        prop_assert!(
            !should_suppress(prior_hash, &prior_head, Some(&prior), prior_ts + ttl, ttl),
            "exactly at TTL must re-emit (boundary is exclusive)",
        );
        // No prior ⇒ not suppressed.
        prop_assert!(
            !should_suppress(prior_hash, &prior_head, None, now, ttl),
            "no prior record must never suppress",
        );
    }
}
