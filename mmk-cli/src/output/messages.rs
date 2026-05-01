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

use std::path::{Path, PathBuf};

/// Canonical prefix on every "designed quiet case, not uncertainty" surface.
///
/// v0.8 scattered six distinct wordings ("no signal", "new file",
/// "history priors don't apply", "session contains 0 commits", …) —
/// agents in calibration variously misread these as "the sensor
/// failed" or "I need to re-run." Pinning a single scannable prefix
/// lets the agent read all six as the same outcome shape; the
/// per-case detail follows the prefix unchanged.
///
/// Trailing space is intentional — every consumer composes
/// `{NO_SIGNAL_PREFIX}{reason}`.
pub const NO_SIGNAL_PREFIX: &str = "[no actionable signal] ";

/// `<subject> edited; <partner> co-edited K of N prior commits, not in diff[ [low-confidence n=N]]`
///
/// The review-mode COUPLING body. Names the partner that *exists* and
/// is *not* in the diff — the reader infers the implication.
///
/// `wilson_lower` and the cfg pair are read by [`coupling_confidence_suffix`]
/// to append a `[low-confidence n=N]` suffix on fires that gated through
/// near the floor; high-confidence fires read clean (no suffix).
#[must_use]
pub fn coupling_review_missed(
    subject: &Path,
    partner: &Path,
    k: u32,
    n: u32,
    wilson_lower: f64,
    min_sample_size: u32,
    confidence_threshold: f64,
) -> String {
    let suffix = coupling_confidence_suffix(n, wilson_lower, min_sample_size, confidence_threshold);
    format!(
        "{} edited; {} co-edited {k} of {n} prior commits, not in diff{suffix}",
        subject.display(),
        partner.display(),
    )
}

/// `<subject> co-edited with <partner> in K of N prior commits[ [low-confidence n=N]]`
///
/// The pre-edit COUPLING body. Pre-edit fires before any change, so
/// the wording states the historical fact without implying a missed
/// edit. Same low-confidence suffix logic as [`coupling_review_missed`].
#[must_use]
pub fn coupling_pre_edit(
    subject: &Path,
    partner: &Path,
    k: u32,
    n: u32,
    wilson_lower: f64,
    min_sample_size: u32,
    confidence_threshold: f64,
) -> String {
    let suffix = coupling_confidence_suffix(n, wilson_lower, min_sample_size, confidence_threshold);
    format!(
        "{} co-edited with {} in {k} of {n} prior commits{suffix}",
        subject.display(),
        partner.display(),
    )
}

/// Two-tier COUPLING confidence: low-confidence band gets an explicit
/// `[low-confidence n=N]` suffix; everything above stays silent so the
/// agent reads silence as "high confidence." Two tiers (not three)
/// keep the surface scannable — three-tier (low/medium/strong) was
/// considered and rejected as over-fitting to one experiment.
///
/// A fire is "low-confidence" when:
/// - `n` is at the floor (`≤ min_sample_size + 1`), or
/// - `wilson_lower` sits in the band `[confidence_threshold,
///   2 × confidence_threshold)` — i.e. it cleared the gate but is
///   closest to it.
///
/// Both branches catch the case the cohort flagged: a fire that
/// gated through but where the agent shouldn't override silently.
fn coupling_confidence_suffix(
    n: u32,
    wilson_lower: f64,
    min_sample_size: u32,
    confidence_threshold: f64,
) -> String {
    let small_sample = n <= min_sample_size + 1;
    let near_threshold = wilson_lower < 2.0 * confidence_threshold;
    if small_sample || near_threshold {
        format!(" [low-confidence n={n}]")
    } else {
        String::new()
    }
}

/// `<path>: rank #R of top-T`
#[must_use]
pub fn hotspot(path: &Path, rank: u32, top: usize) -> String {
    format!("{}: rank #{rank} of top-{top}", path.display())
}

/// `diff touches N files [(G gross, ignore_for_budget excluded G-N)]; cap M[, HOTSPOT/COUPLING skipped (partners co-touched by construction)]; large diffs concentrate rollback risk and slow review`
///
/// The `suppressed = true` tail names *what* mmk did not run and
/// *why*, instead of leaving the agent to infer it from "analysis
/// suppressed" jargon. The history-graph layers (HOTSPOT, COUPLING,
/// DRIFT, greenfield) are skipped on bulk diffs because their
/// signal collapses at scale: with hundreds of changed files,
/// every historical partner of every changed file is already in
/// the diff, so COUPLING's "missed partner" question is trivially
/// answered "no" and HOTSPOT degenerates to "every file is a
/// hotspot." Per-file sensors (STRUCTURE, COMPLEXITY) still run —
/// their cost and signal both scale per-file, not with diff size.
///
/// The rollback / review-slowdown tail is the empirical observation
/// from the SmartBear/Cisco case study (Cohen 2006), replicated under
/// controls in Jureczko 2020 (IET Software): review effectiveness
/// degrades past ~200 LOC and continues to degrade as size grows.
///
/// When `gross` is `Some(g)` and `g > actual`, the prose reports
/// both the net (post-`ignore_for_budget`) and gross totals so the
/// agent sees what was excluded — silent dropping was the failure
/// mode this calibration ships net-with-honest-gross to avoid.
#[must_use]
pub fn budget_files(actual: u32, max: u32, gross: Option<u32>, suppressed: bool) -> String {
    let tail = if suppressed {
        ", HOTSPOT/COUPLING skipped (partners co-touched by construction)"
    } else {
        ""
    };
    let gross_clause = match gross {
        Some(g) if g > actual => {
            format!(" ({g} gross, ignore_for_budget excluded {})", g - actual)
        }
        _ => String::new(),
    };
    format!(
        "diff touches {actual} files{gross_clause}; cap {max}{tail}; large diffs concentrate rollback risk and slow review"
    )
}

/// `diff is N lines [(G gross, ignore_for_budget excluded G-N)]; cap M[, HOTSPOT/COUPLING skipped (partners co-touched by construction)]; large diffs concentrate rollback risk and slow review`
///
/// Same shape and rationale as [`budget_files`]; the suppressed
/// tail names which layers were skipped and why so the agent reads
/// the silence on HOTSPOT/COUPLING as "didn't compute it because
/// the answer would be noise" rather than "all clear."
#[must_use]
pub fn budget_lines(actual: u64, max: u64, gross: Option<u64>, suppressed: bool) -> String {
    let tail = if suppressed {
        ", HOTSPOT/COUPLING skipped (partners co-touched by construction)"
    } else {
        ""
    };
    let gross_clause = match gross {
        Some(g) if g > actual => {
            format!(" ({g} gross, ignore_for_budget excluded {})", g - actual)
        }
        _ => String::new(),
    };
    format!(
        "diff is {actual} lines{gross_clause}; cap {max}{tail}; large diffs concentrate rollback risk and slow review"
    )
}

/// `diff at N lines; review effectiveness degrades past ~M lines. Commit this slice before continuing.`
///
/// Absolute review-effectiveness floor (default 200 LOC). Fires Info
/// once the working-tree-vs-HEAD diff crosses the threshold while
/// still under 50% of the per-diff cap — the band the under-cap
/// ramp's 50% Approaching tier doesn't reach. The message anchors
/// to the behavior (review effectiveness degrades) rather than to a
/// paper citation an agent can't verify. The threshold's empirical
/// grounding lives in the docstring on `BulkCfg::review_quality_lines`
/// (Jureczko 2020 IET Software replication of the SmartBear/Cisco
/// review-rate findings).
#[must_use]
pub fn budget_review_quality(actual: u64, threshold: u64) -> String {
    format!(
        "diff at {actual} lines; review effectiveness degrades past ~{threshold} lines. Commit this slice before continuing."
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
pub fn health_registration(subject: &Path, related: &[PathBuf]) -> String {
    format!(
        "{}: action-registration; precedents: {}",
        subject.display(),
        join_paths(related),
    )
}

/// `<subject>: service-decl; consumers: <Y>, <Z>`
#[must_use]
pub fn health_service(subject: &Path, related: &[PathBuf]) -> String {
    format!(
        "{}: service-decl; consumers: {}",
        subject.display(),
        join_paths(related),
    )
}

/// `<subject>: test partner <Y> not in diff`
#[must_use]
pub fn health_test_pair(subject: &Path, related: &[PathBuf]) -> String {
    format!(
        "{}: test partner {} not in diff",
        subject.display(),
        join_paths(related),
    )
}

/// `<subject>: adds N broad exception handler[s] not in HEAD (catch
/// with empty body, no parameter, or any/unknown/Error type at
/// non-top-level)`
///
/// `delta` is the net working-vs-HEAD addition count. The wording
/// names the structural failure mode (broad-catch addition) without
/// editorializing — agents that read the message can decide whether
/// the addition is a genuine narrowing of scope or a swallow.
#[must_use]
pub fn health_broad_exception(subject: &Path, delta: u32) -> String {
    let plural = if delta == 1 { "" } else { "s" };
    format!(
        "{}: adds {delta} broad exception handler{plural} not in HEAD (catch with empty body, no parameter, or any/unknown/Error type at non-top-level)",
        subject.display(),
    )
}

/// Pre-edit fall-through when no other layer fires.
///
/// `commits_touching = 0` is *ambiguous*: it can mean the file is
/// truly new (not yet committed, absent from HEAD's tree) OR the
/// file lives in HEAD but every commit that ever touched it was
/// dropped from analysis (most commonly by the `bulk.max_files`
/// filter on workspace-grain repos where natural feature commits
/// run wider than the cap). Conflating those two states reads as
/// "you can edit freely, no historical risk" — which is true in
/// the first case and dangerous in the second. `present_in_head`
/// disambiguates so the wording matches the actual state.
///
/// Three messages — all share the canonical [`NO_SIGNAL_PREFIX`]:
/// - `[no actionable signal] <path>: new file (not yet in HEAD)`
/// - `[no actionable signal] <path>: present in HEAD but no analyzable history (file may be stale or prior touches were filtered as bulk commits)`
/// - `[no actionable signal] <path>: N commits in W-day window[, rank #R]`
#[must_use]
pub fn quiet_file(
    path: &Path,
    n_commits: u32,
    window_days: u32,
    rank: Option<u32>,
    present_in_head: bool,
) -> String {
    if n_commits == 0 {
        if present_in_head {
            return format!(
                "{NO_SIGNAL_PREFIX}{}: present in HEAD but no analyzable history \
                 (file may be stale or prior touches were filtered as bulk commits)",
                path.display()
            );
        }
        return format!(
            "{NO_SIGNAL_PREFIX}{}: new file (not yet in HEAD)",
            path.display()
        );
    }
    let rank_clause = rank.map_or_else(String::new, |r| format!(", rank #{r}"));
    format!(
        "{NO_SIGNAL_PREFIX}{}: {n_commits} commits in {window_days}-day window{rank_clause}",
        path.display(),
    )
}

/// `[no actionable signal] diff is K of N new files; history priors don't apply (greenfield)`
///
/// Emitted when the working-tree diff is mostly greenfield — the
/// HOTSPOT/COUPLING/DRIFT layers structurally have nothing to say
/// about paths the historical analyzer hasn't seen. Acknowledging
/// the silence as positive information beats letting the agent
/// guess whether mmk just decided to be quiet.
#[must_use]
pub fn greenfield_signal(new_count: usize, total: usize) -> String {
    format!(
        "{NO_SIGNAL_PREFIX}diff is {new_count} of {total} new files; \
         history priors don't apply (greenfield)"
    )
}

/// Diff-size summary attached to a "no findings" review surface.
///
/// `loc` is the total churn (`added + deleted`) across the diff —
/// matches the same accounting BUDGET uses, so the two surfaces stay
/// numerically consistent. The `+N LOC` rendering reads naturally
/// for the dominant additive case and stays unambiguous on rewrites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyDiffSummary {
    pub file_count: u32,
    pub loc: u32,
}

/// `[no actionable signal] no findings (HEAD <sha7>)` on a clean tree;
/// `[no actionable signal] no findings (N file[s], +M LOC vs HEAD <sha7>)`
/// when a real diff produced zero findings.
///
/// v0.8's text mode silently emitted nothing on either case — agents
/// across multiple cohort runs read silence as "did mmk run? did it
/// see my edits? did the gate fail open?" The diff-bearing form
/// closes that ambiguity: the agent sees mmk *did* see N files and
/// M lines, computed against a specific HEAD, and chose to surface
/// nothing. The sha7 matches the one already on hook-mode
/// `systemMessage` (`hook_json::dedup_message`) so the two surfaces
/// stay convergent.
#[must_use]
pub fn empty_review_line(sha7: &str, diff: Option<&EmptyDiffSummary>) -> String {
    diff.map_or_else(
        || format!("{NO_SIGNAL_PREFIX}no findings (HEAD {sha7})"),
        |d| {
            let plural = if d.file_count == 1 { "" } else { "s" };
            format!(
                "{NO_SIGNAL_PREFIX}no findings ({} file{plural}, +{} LOC vs HEAD {sha7})",
                d.file_count, d.loc,
            )
        },
    )
}

/// COHESION indicator: tangled-diff fingerprint.
///
/// Forms one of two prose shapes depending on `cluster_paths`:
///
/// - paths supplied → "diff spans N disjoint co-change clusters
///   (sizes s1, s2, …): {paths}; tangled diffs correlate with
///   elevated revert / review cost"
/// - paths empty (large clusters, blow up message length) →
///   summary form with sizes only
///
/// The "elevated revert / review cost" tail names the empirical
/// implication (Herzig & Zeller 2013 measured ~16 % of bug-fixes in
/// open-source projects carried unrelated changes; tangled commits
/// inflate review burden). The wording is descriptive, not
/// prescriptive — mmk doesn't tell the agent how to split the diff.
#[must_use]
pub fn cohesion_tangled(cluster_sizes: &[usize], cluster_paths: Option<&[Vec<String>]>) -> String {
    let n = cluster_sizes.len();
    let sizes_csv = cluster_sizes
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let detail = cluster_paths.map_or_else(String::new, |groups| {
        let pretty = groups
            .iter()
            .map(|g| format!("{{{}}}", g.join(", ")))
            .collect::<Vec<_>>()
            .join("; ");
        format!(": {pretty}")
    });
    format!(
        "diff spans {n} disjoint co-change clusters (sizes {sizes_csv}){detail}; \
         tangled diffs correlate with elevated revert / review cost"
    )
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
/// Carries the canonical [`NO_SIGNAL_PREFIX`] so the agent reads it
/// alongside the other fall-throughs as a designed quiet case, not
/// a missing-data state.
#[must_use]
pub const fn session_empty_nudge() -> &'static str {
    "[no actionable signal] session contains 0 commits since the resolved base; \
     for uncommitted working-tree review, use `mmk review` instead"
}

/// Empty-session window-suppression line.
///
/// Companion to [`session_empty_nudge`]: when the session is empty
/// the WINDOW ranking is dropped from output (locale `.po` files,
/// generated artifacts, etc. drown the ANCHOR nudge). Surface a
/// one-liner so consumers see the suppression as positive
/// information — `mmk analyze` is the right tool for pure
/// window-wide hotspots.
#[must_use]
pub const fn session_window_suppressed() -> &'static str {
    "WINDOW ranking suppressed: 0 commits in session — see `mmk analyze` \
     for window hotspots"
}

fn join_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| p.display().to_string())
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

/// `<P>: role file in <D>/; divergence from K sibling <shape> files
/// is expected — confirm the role peers share the role convention
/// rather than the directory shape`
///
/// Emitted when STRUCTURE detects a role-pattern match (factory,
/// contribution, registration, …). Severity demotes to `Info`: the
/// divergence is structurally legitimate, but role-vs-role drift is
/// still worth the agent's attention. Wording is descriptive — names
/// the role status without prescribing a fix, since whether the
/// agent should align with role peers depends on context this
/// sensor doesn't see.
#[must_use]
pub fn structure_review_role(
    path: &Path,
    dir: &Path,
    sibling_count: u32,
    shape_ext: &str,
    shape_suffix: &str,
) -> String {
    let shape = shape_label(shape_ext, shape_suffix);
    format!(
        "{}: role file in {}/; divergence from {sibling_count} sibling {shape} files \
         is expected — confirm the role peers share the role convention rather than \
         the directory shape",
        path.display(),
        dir.display(),
    )
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

/// COMPLEXITY-nesting prose. See [`complexity_review_size`] for the
/// LOC variant; both share an internal `compose_complexity_prose`
/// helper.
///
/// Two output forms:
/// - `<P>::<fn>: nesting N exceeds cap C[, directory median M (ratio R)]; correlates with elevated defect rate (Code Red 2022)`
/// - `<P>::<fn>: nesting N, directory median M (ratio R); correlates with elevated defect rate (Code Red 2022)`
///
/// Names the absolute cap when it fired so the agent has a concrete
/// breach to act on (reduce nesting below `cap`). When the ratio
/// gate fired instead (absolute didn't), the median + ratio carry
/// the "outlier vs siblings" story. Either form is anchored on
/// code facts — never on mmk's missing-data state.
///
/// Empirical anchor: Tornhill & Borg 2022 found Alert-class code
/// (nested-logic smells included in the Code Health composite)
/// shows 15× more defects than Healthy code, with a medium-large
/// effect size (Cohen's d = 0.73). The 15× applies to the
/// composite, not nesting alone — hence "elevated" rather than a
/// specific multiplier.
#[must_use]
pub fn complexity_review_nesting(
    path: &Path,
    fn_name: &str,
    actual: u32,
    cap: u32,
    head_actual: Option<u32>,
    median: Option<u32>,
) -> String {
    let prose = compose_complexity_prose(actual, cap, head_actual, median, "nesting ", "");
    format!(
        "{}::{fn_name}: {prose}; correlates with elevated defect rate (Code Red 2022)",
        path.display(),
    )
}

/// COMPLEXITY-size prose; sibling of [`complexity_review_nesting`].
///
/// Two output forms:
/// - `<P>::<fn>: N LOC exceeds cap C[, directory median M LOC (ratio R)]; correlates with slower comprehension and issue resolution`
/// - `<P>::<fn>: N LOC, directory median M LOC (ratio R); correlates with slower comprehension and issue resolution`
///
/// Same wording shape as nesting: name the absolute cap when it
/// fired; otherwise lean on median + ratio. Empirical anchor:
/// ~60% of dev time is comprehension (Xia et al. 2017, cited in
/// Code Red); long functions are part of the smell bundle that
/// correlates with 78–124% longer issue-resolution time in
/// Warning/Alert code (Tornhill & Borg 2022).
#[must_use]
pub fn complexity_review_size(
    path: &Path,
    fn_name: &str,
    actual: u32,
    cap: u32,
    head_actual: Option<u32>,
    median: Option<u32>,
) -> String {
    let prose = compose_complexity_prose(actual, cap, head_actual, median, "", " LOC");
    format!(
        "{}::{fn_name}: {prose}; correlates with slower comprehension and issue resolution",
        path.display(),
    )
}

/// Shared body for both COMPLEXITY messages. `metric_prefix` is the
/// kind name placed *before* the value ("nesting " for nesting,
/// "" for LOC). `unit_suffix` is the unit placed *after* the value
/// (" LOC" for size, "" for nesting).
///
/// Picks the right wording based on which gate fired:
/// - `actual > cap` → absolute-cap breach is named; median is
///   appended only when available, as enrichment.
/// - else → ratio gate fired (median is always Some by construction
///   in this branch); use the existing "median (ratio)" phrasing.
fn compose_complexity_prose(
    actual: u32,
    cap: u32,
    head_actual: Option<u32>,
    median: Option<u32>,
    metric_prefix: &str,
    unit_suffix: &str,
) -> String {
    // `+N vs HEAD` clause: rendered when we have a HEAD baseline
    // AND the agent strictly worsened the metric. Tells the agent
    // *how much* of the breach they introduced — distinguishes "+3"
    // (small additions to an inherited problem) from "+60"
    // (substantial worsening). Inserted before the empirical
    // anchor; both forms (absolute and ratio) get the same delta
    // clause when applicable.
    let delta_clause = head_actual
        .filter(|h| actual > *h)
        .map(|h| format!(" (+{} vs HEAD)", actual - h))
        .unwrap_or_default();
    if actual > cap {
        let head = format!("{metric_prefix}{actual}{unit_suffix} exceeds cap {cap}{delta_clause}");
        if let Some(m) = median {
            let ratio = if m == 0 {
                f64::INFINITY
            } else {
                f64::from(actual) / f64::from(m)
            };
            return format!("{head}, directory median {m}{unit_suffix} (ratio {ratio:.1})");
        }
        head
    } else {
        // Ratio gate path: median is Some by construction (the
        // sensor only fires the ratio gate when median is
        // available). Defensive `unwrap_or` keeps prose intact if
        // the contract is ever violated.
        let m = median.unwrap_or(0);
        let ratio = if m == 0 {
            f64::INFINITY
        } else {
            f64::from(actual) / f64::from(m)
        };
        format!(
            "{metric_prefix}{actual}{unit_suffix}{delta_clause}, directory median {m}{unit_suffix} (ratio {ratio:.1})"
        )
    }
}
