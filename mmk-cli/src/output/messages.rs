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

/// `diff touches A files; cap M[, analysis suppressed]; large diffs concentrate rollback risk and slow review`
///
/// `suppressed = true` is the bulk-self-filter path: the diff itself
/// was so big that hotspot/coupling analysis was skipped. The
/// indicator tail names the implication (rollback risk + review
/// slowdown) — the agent decides what to do; the message doesn't
/// prescribe.
#[must_use]
pub fn budget_files(actual: u32, max: u32, suppressed: bool) -> String {
    let tail = if suppressed {
        ", analysis suppressed"
    } else {
        ""
    };
    format!(
        "diff touches {actual} files; cap {max}{tail}; large diffs concentrate rollback risk and slow review"
    )
}

/// `diff is A lines; cap M[, analysis suppressed]; large diffs concentrate rollback risk and slow review`
#[must_use]
pub fn budget_lines(actual: u64, max: u64, suppressed: bool) -> String {
    let tail = if suppressed {
        ", analysis suppressed"
    } else {
        ""
    };
    format!(
        "diff is {actual} lines; cap {max}{tail}; large diffs concentrate rollback risk and slow review"
    )
}

/// `diff at A_f of M_f files, A_l of M_l lines (P% of cap); approaching review cap`
///
/// Continuous-feedback ramp surface, fired at 50–74% (Info) and
/// 75–99% (Warn). The tail differentiates the tiers: at "approaching"
/// it's neutral wording; at "near" it adds "approaching review cap"
/// to mark the decision point. Locks in the visibility-of-climbing-
/// meter that the prior binary-fire-at-100% lacked.
#[must_use]
pub fn budget_ramp(
    files: u32,
    max_files: u32,
    lines: u64,
    max_lines: u64,
    near_cap: bool,
) -> String {
    let pct = (peak_pct(files, max_files, lines, max_lines)).round() as u32;
    let tail = if near_cap {
        "; approaching review cap"
    } else {
        ""
    };
    format!(
        "diff at {files} of {max_files} files, {lines} of {max_lines} lines ({pct}% of cap){tail}"
    )
}

fn peak_pct(files: u32, max_files: u32, lines: u64, max_lines: u64) -> f64 {
    let max_files = max_files.max(1);
    let max_lines = max_lines.max(1);
    let r_f = f64::from(files) / f64::from(max_files);
    let r_l = lines as f64 / max_lines as f64;
    100.0 * r_f.max(r_l)
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

/// Empty-session nudge.
///
/// Surfaced when `session_commits.len() == 0` (typically
/// `--base HEAD` or a fresh branch with no commits since base).
/// Without it the agent reads "0 files" as silent failure; with it
/// they're pointed at the right subcommand for uncommitted work.
#[must_use]
pub const fn session_empty_nudge() -> &'static str {
    "session contains 0 commits since the resolved base; for uncommitted \
     working-tree review, use `mmk review` instead"
}

fn join_paths<P: AsRef<Path>>(paths: &[P]) -> String {
    paths
        .iter()
        .map(|p| p.as_ref().display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

// ---- structure / complexity formatters --------------------------

/// Render a "shape" word like `*.tsx`, `*.test.ts`, `index.ts`.
fn shape_label(ext: &str, suffix: &str) -> String {
    if suffix == "__index__" {
        format!("index.{ext}")
    } else if suffix.is_empty() {
        format!("*.{ext}")
    } else {
        format!("*.{suffix}.{ext}")
    }
}

fn join_strings(items: &[String]) -> String {
    items.join(", ")
}

fn format_shown_imports(common: &[String], cap: usize, total: usize, majority_pct: u32) -> String {
    let shown = common.len().min(cap);
    let formatted = format!("{{{}}}", common[..shown].join(", "));
    if total <= cap {
        formatted
    } else {
        format!("{formatted} (showing {shown} of {total} ≥{majority_pct}%)")
    }
}

/// Bundle of structure-finding inputs for the pre-edit formatters.
///
/// Plain data — exists only to reduce the formatter call surface
/// from ten positional args to one. The fields are documented
/// where they're consumed.
#[derive(Debug, Clone)]
pub struct StructurePreEdit<'a> {
    pub path: &'a Path,
    pub dir: &'a Path,
    pub sibling_count: u32,
    pub shape_ext: &'a str,
    pub shape_suffix: &'a str,
    pub common_imports: &'a [String],
    pub total_common_imports: usize,
    pub cap: usize,
    pub majority_pct: u32,
    pub common_templates: &'a [String],
}

fn structure_pre_edit_render(prefix: &str, args: &StructurePreEdit<'_>) -> String {
    let shape = shape_label(args.shape_ext, args.shape_suffix);
    let imports = format_shown_imports(
        args.common_imports,
        args.cap,
        args.total_common_imports,
        args.majority_pct,
    );
    let template_clause = if args.common_templates.is_empty() {
        String::new()
    } else {
        format!(
            " and export template {}",
            join_strings(args.common_templates)
        )
    };
    format!(
        "{}: {prefix} in {}/; {} sibling {shape} files share imports {imports}{template_clause}",
        args.path.display(),
        args.dir.display(),
        args.sibling_count,
    )
}

/// `<P>: new file in <D>; K sibling <shape> files share imports {…}[ and export template …]`
#[must_use]
pub fn structure_pre_edit_new(args: &StructurePreEdit<'_>) -> String {
    structure_pre_edit_render("new file", args)
}

/// `<P>: existing file in <D>; K sibling <shape> files share imports {…}[ and export template …]`
#[must_use]
pub fn structure_pre_edit_existing(args: &StructurePreEdit<'_>) -> String {
    structure_pre_edit_render("existing file", args)
}

/// `<P>: missing N of M directory-common imports {…}[; not exporting expected …]`
#[must_use]
pub fn structure_review_divergent(
    path: &Path,
    missing_imports: &[String],
    total_common_imports: usize,
    missing_templates: &[String],
) -> String {
    let mut msg = if missing_imports.is_empty() {
        format!("{}:", path.display())
    } else {
        let n = missing_imports.len();
        format!(
            "{}: missing {n} of {total_common_imports} directory-common imports {{{}}}",
            path.display(),
            missing_imports.join(", "),
        )
    };
    if !missing_templates.is_empty() {
        if missing_imports.is_empty() {
            msg.push_str(" not exporting expected ");
        } else {
            msg.push_str("; not exporting expected ");
        }
        msg.push_str(&join_strings(missing_templates));
    }
    msg
}

/// `<P>: matches <D>/ convention (K sibling baseline)`
#[must_use]
pub fn structure_review_conforming(path: &Path, dir: &Path, sibling_count: u32) -> String {
    format!(
        "{}: matches {}/ convention ({sibling_count} sibling baseline)",
        path.display(),
        dir.display(),
    )
}

/// `<P>::<fn>: nesting N, directory median M (ratio R); correlates with elevated defect rate (Code Red 2022)`
///
/// Indicator wording: states the empirical implication, doesn't
/// prescribe a fix. The Tornhill & Borg 2022 study found Alert-class
/// code (which includes nested-logic smells in its Code Health
/// composite) shows 15× more defects than Healthy code, with a
/// medium-large effect size (Cohen's d = 0.73). The 15× headline
/// applies to the composite, not nesting alone — hence "elevated"
/// rather than a specific multiplier here.
#[must_use]
pub fn complexity_review_nesting(
    path: &Path,
    fn_name: &str,
    actual: u32,
    median: Option<u32>,
) -> String {
    let median_clause = median.map_or_else(
        || "directory median unknown".to_owned(),
        |m| {
            let ratio = if m == 0 {
                f64::INFINITY
            } else {
                f64::from(actual) / f64::from(m)
            };
            format!("directory median {m} (ratio {ratio:.1})")
        },
    );
    format!(
        "{}::{fn_name}: nesting {actual}, {median_clause}; correlates with elevated defect rate (Code Red 2022)",
        path.display(),
    )
}

/// `<P>::<fn>: N LOC, directory median M (ratio R); correlates with slower comprehension and issue resolution`
///
/// Indicator wording: ~60% of dev time is comprehension (Xia et al.
/// 2017, cited in Code Red); long functions are part of the smell
/// bundle that correlates with the 78–124% longer issue-resolution
/// time that Tornhill & Borg observed in Warning/Alert code.
#[must_use]
pub fn complexity_review_size(
    path: &Path,
    fn_name: &str,
    actual: u32,
    median: Option<u32>,
) -> String {
    let median_clause = median.map_or_else(
        || "directory median unknown".to_owned(),
        |m| {
            let ratio = if m == 0 {
                f64::INFINITY
            } else {
                f64::from(actual) / f64::from(m)
            };
            format!("directory median {m} LOC (ratio {ratio:.1})")
        },
    );
    format!(
        "{}::{fn_name}: {actual} LOC, {median_clause}; correlates with slower comprehension and issue resolution",
        path.display(),
    )
}
