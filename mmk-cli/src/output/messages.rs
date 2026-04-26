//! Pure formatters for the human-readable bodies of every finding
//! type. One function per shape; no I/O.
//!
//! The wording rules these encode are:
//! - factual, not suggestive (no "expected", no "consider", no
//!   editorial like "likely a sweep");
//! - terse — severity glyph + brevity carry the "this matters"
//!   signal, not editorial tails like "pre-edit consulted";
//! - no algorithm names or config tokens (`Wilson`, `min_sample_size`)
//!   in the human surface — those stay in `--format json`;
//! - K of N raw count, never the percentage — the percentage frames a
//!   small-n estimate as confident when it isn't.
//!
//! Negative oracles in `mmk-cli/tests/messages.rs` lock these
//! invariants so a future format-string tweak can't regress them.

use std::path::Path;

/// `<subject> edited; <partner> co-edited K of N prior commits, not in diff`
///
/// The review-mode COUPLING body. Names the partner that *exists* and
/// is *not* in the diff — the reader infers the implication.
#[must_use]
pub fn coupling_review_missed(subject: &Path, partner: &Path, k: u32, n: u32) -> String {
    format!(
        "{} edited; {} co-edited {k} of {n} prior commits, not in diff",
        subject.display(),
        partner.display(),
    )
}

/// `<subject> co-edited with <partner> in K of N prior commits`
///
/// The pre-edit COUPLING body. Pre-edit fires before any change, so
/// the wording states the historical fact without implying a missed
/// edit.
#[must_use]
pub fn coupling_pre_edit(subject: &Path, partner: &Path, k: u32, n: u32) -> String {
    format!(
        "{} co-edited with {} in {k} of {n} prior commits",
        subject.display(),
        partner.display(),
    )
}

/// `<path>: rank #R of top-T`
#[must_use]
pub fn hotspot(path: &Path, rank: u32, top: usize) -> String {
    format!("{}: rank #{rank} of top-{top}", path.display())
}

/// `diff touches A files; cap M[, analysis suppressed]`
///
/// `suppressed = true` is the bulk-self-filter path: the diff itself
/// was so big that hotspot/coupling analysis was skipped. The trailing
/// clause is the *only* signal the reader gets that something was
/// dropped — the "likely a sweep" editorial is gone.
#[must_use]
pub fn budget_files(actual: u32, max: u32, suppressed: bool) -> String {
    let tail = if suppressed {
        ", analysis suppressed"
    } else {
        ""
    };
    format!("diff touches {actual} files; cap {max}{tail}")
}

/// `diff is A lines; cap M[, analysis suppressed]`
#[must_use]
pub fn budget_lines(actual: u64, max: u64, suppressed: bool) -> String {
    let tail = if suppressed {
        ", analysis suppressed"
    } else {
        ""
    };
    format!("diff is {actual} lines; cap {max}{tail}")
}

/// `<path>: climbed K of N transitions; latest rank #R`
#[must_use]
pub fn drift(
    path: &Path,
    climb_transitions: u32,
    total_transitions: u32,
    latest_rank: u32,
) -> String {
    format!(
        "{}: climbed {climb_transitions} of {total_transitions} transitions; latest rank #{latest_rank}",
        path.display()
    )
}

/// `<subject>: action-registration; precedents: <Y>, <Z>`
#[must_use]
pub fn health_registration<P: AsRef<Path>>(subject: &Path, related: &[P]) -> String {
    format!(
        "{}: action-registration; precedents: {}",
        subject.display(),
        join_paths(related),
    )
}

/// `<subject>: service-decl; consumers: <Y>, <Z>`
#[must_use]
pub fn health_service<P: AsRef<Path>>(subject: &Path, related: &[P]) -> String {
    format!(
        "{}: service-decl; consumers: {}",
        subject.display(),
        join_paths(related),
    )
}

/// `<subject>: test partner <Y> not in diff`
#[must_use]
pub fn health_test_pair<P: AsRef<Path>>(subject: &Path, related: &[P]) -> String {
    format!(
        "{}: test partner {} not in diff",
        subject.display(),
        join_paths(related),
    )
}

/// `<path>: no signal (N commits in W-day window[, rank #R])` —
/// or `<path>: new file (no history)` when `n_commits == 0`.
///
/// The pre-edit fall-through when no other layer fires. Lets the
/// agent distinguish "mmk was consulted but had nothing to say" from
/// "mmk wasn't run." The zero-commit branch distinguishes
/// "untouched in window" (which has history elsewhere) from "doesn't
/// exist in history at all" — wording that conflates the two
/// misleads agents working in greenfield slices.
#[must_use]
pub fn quiet_file(path: &Path, n_commits: u32, window_days: u32, rank: Option<u32>) -> String {
    if n_commits == 0 {
        return format!("{}: new file (no history)", path.display());
    }
    let rank_clause = rank.map_or_else(String::new, |r| format!(", rank #{r}"));
    format!(
        "{}: no signal ({n_commits} commits in {window_days}-day window{rank_clause})",
        path.display(),
    )
}

/// `diff is K of N new files; history priors don't apply`
///
/// Emitted when the working-tree diff is mostly greenfield — the
/// HOTSPOT/COUPLING/DRIFT layers structurally have nothing to say
/// about paths the historical analyzer hasn't seen. Acknowledging
/// the silence as positive information beats letting the agent
/// guess whether mmk just decided to be quiet.
#[must_use]
pub fn greenfield_signal(new_count: usize, total: usize) -> String {
    format!("diff is {new_count} of {total} new files; history priors don't apply")
}

/// `K of N commits dropped (>F files or >L lines)` — session budget.
///
/// Fires when the per-commit bulk filter dropped commits inside the
/// session window. `dropped` is the count filtered; `total` is the
/// full revwalk count (`commits_seen`) so the reader sees the rate,
/// not just the absolute.
#[must_use]
pub fn session_budget(dropped: u64, total: u64, max_files: u32, max_lines: u32) -> String {
    format!("{dropped} of {total} commits dropped (>{max_files} files or >{max_lines} lines)")
}

/// `session is L lines across N commits; cap B` — session aggregate.
#[must_use]
pub fn session_overrun(session_lines: u64, session_n: u32, budget: u64) -> String {
    format!("session is {session_lines} lines across {session_n} commits; cap {budget}")
}

fn join_paths<P: AsRef<Path>>(paths: &[P]) -> String {
    paths
        .iter()
        .map(|p| p.as_ref().display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
