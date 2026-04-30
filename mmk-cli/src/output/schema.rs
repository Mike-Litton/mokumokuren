//! Stable JSON schema version.
//!
//! Tracks the `mmk` minor release: every `0.9.x` build emits
//! `0.9.0`. Additive changes (new optional fields, new top-level
//! blocks) do not bump the schema version; renames, removals, type
//! changes, and semantic changes do.
//!
//! ## v0.9.0 — contract delta
//!
//! No JSON-shape changes. `findings[].message` content for
//! COMPLEXITY findings now carries class-qualified function
//! identity (`<P>::ClassName::methodName`) instead of bare names —
//! the field is the same string field, longer string. Fall-through
//! findings (the `quiet_file`, `greenfield_signal`,
//! `session_empty_nudge`, and clean-state lines) all gain the
//! canonical `[no actionable signal] ` prefix in their `message`
//! body; the rest of the message preserves the per-case detail.
//! COUPLING fires near the gate floor append a
//! ` [low-confidence n=N]` suffix to `message`. Reason: agents in
//! the v0.8 cohort variously misread the v0.8 wordings as missing
//! data and re-ran the command; the prefix gives a single scannable
//! token. v0.8 readers parse v0.9 output without modification.
//!
//! ## v0.8.0 — contract delta
//!
//! Additive: `analysis.window_truncation` block,
//! `sensor.structure.role_patterns`,
//! `sensor.complexity.delta_warn_pct` / `delta_warn_abs` in the
//! `config` echo. Severity field on `findings[]` shifts on COMPLEXITY
//! (delta-weighted) and STRUCTURE (role demotion). The v0.7
//! window-truncation `findings[]` entry on session-summary retires;
//! its data moves to the new metadata block. v0.7 readers parse v0.8
//! output without modification. See `CHANGELOG.md` for rationale.
//!
//! ## v0.7.0 — what's new
//!
//! Three additive changes — none break the existing shape; consumers
//! reading `0.6.x` parse v0.7 output without modification.
//!
//! - **`findings[].severity = "warn"` for `findings[].layer = "cohesion"`.**
//!   COHESION promotes from Info to Warn. Empirical grounding: the
//!   MSR 2026 mining-challenge paper *"LGTM! Characteristics of
//!   Auto-Merged LLM-based Agentic PRs"* (Canelas et al.) shows
//!   across the AIDev corpus that auto-merged PRs are smaller and
//!   more focused than non-auto-merged ones — peer-reviewed evidence
//!   for the direction COHESION already detects. Severity-only
//!   change — no shape change in the finding's structure. Consumers
//!   running `--gate warn` will see exit code 2 fire on tangled diffs.
//! - **Optional `cohesion` top-level block on `mmk review` JSON.**
//!   Carries `tangles[].clusters[]` — the full per-cluster path
//!   decomposition (no 8-path cap, unlike the prose form). Lets a
//!   harness render the split as a commit-split proposal without
//!   parsing `findings[].message`. Absent when no tangle qualified;
//!   `serde_skip_if = "Option::is_none"` keeps the JSON shape
//!   additive.
//! - **`findings[].layer = "health"` with `pattern = "broad_exception"`
//!   under `health.matches[]`.** New EVASION sensor (under
//!   `[health.ts] patterns = ["broad_exception", ...]`) detects
//!   newly-added non-top-level broad TS/JS catch handlers (empty
//!   body, no parameter, or `any` / `unknown` / `Error` type) — the
//!   *"evasive repairs with try-except blocks"* failure mode named in
//!   arXiv:2509.13941 *(An Empirical Study on Failures in Automated
//!   Issue Solving)* and corroborated by FSE 2025
//!   *Suppressed Static Analysis Warnings*. Severity Warn in review;
//!   skipped in pre-edit (no working-vs-HEAD diff).
//! - **TestPair (`pattern = "test_pair"`) extends to `.js` / `.jsx`.**
//!   Implementation-test pairing now fires on JS sources, not just
//!   TS. Pre-v0.7 cross-language `.js → .test.ts` mismatches stayed
//!   silent; v0.7 rejects cross-language pairing (the convention
//!   pairs same-extension).
//! - **TSX grammar dispatch fix.** Pre-v0.7 every `.tsx` file was
//!   parsed by the non-TSX TypeScript grammar — JSX-bearing files
//!   silently degraded. v0.7 selects `LANGUAGE_TSX` for `.tsx` /
//!   `.jsx`. Affects all TS Health detectors (Service, Pattern A
//!   substring scan is unchanged because it doesn't parse).
//!
//! ## v0.6.0 — what's new
//!
//! Calibration plus one new sensor and a documented hook contract.
//! All schema changes are additive (no breaking shape changes), so
//! the version bump from `0.5.0` is signal-that-something-changed
//! rather than a contract break for consumers that ignore unknown
//! fields.
//!
//! - **`findings[].layer = "cohesion"` populated.** New COHESION
//!   sensor (`[sensor.cohesion]`) detects tangled diffs — diffs
//!   that decompose into ≥2 disjoint connected components on the
//!   historical co-change graph. Severity Info; Herzig & Zeller
//!   2013-grounded. Edge metric is the max-symmetrized
//!   Wilson 95 % lower bound on the directional conditional
//!   co-change probability (Goutte & Gaussier 2005-style symmetric
//!   construction over COUPLING's gate).
//! - **`config.sensor.cohesion`** block echoed (`enabled`,
//!   `confidence_threshold`, `min_sample_size`,
//!   `min_files_per_cluster`).
//! - **Hook output envelope shape.** `mmk pre-edit` and
//!   `mmk review` auto-detect a Claude Code stdin envelope and
//!   switch to hook-shape JSON: `hookSpecificOutput.{hookEventName,
//!   additionalContext}`, optional top-level `decision` /
//!   `reason` (under `--gate warn` on PostToolUse / Stop), and
//!   `systemMessage` (dedup-suppress and no-findings cases). The
//!   argv path is preserved for manual invocation.
//! - **`mmk session-summary` JSON:** `files` (the WINDOW ranking)
//!   is now optional — omitted when the session is empty
//!   (`session_commits.len() == 0`) so the empty-session ANCHOR
//!   nudge isn't buried under window-wide hotspot noise. Same
//!   `serde_skip_if = "Option::is_none"` shape as `health` /
//!   `new_file_fraction`. `session_files` remains a required
//!   array (possibly empty).
//! - **`mmk review` JSON:** optional `review.diff.budget` sub-block
//!   with `files_gross`, `files_net`, `lines_gross`, `lines_net`,
//!   and `ignored_for_budget`. Present only when
//!   `bulk.ignore_for_budget` matched at least one file. Lets the
//!   agent see net-vs-gross BUDGET accounting transparently
//!   instead of mmk silently dropping generated-file diffs.
//! - **`config.bulk` exposes `max_files` / `max_lines` / `ignore_for_budget`**
//!   for `mokumokuren.toml` overrides. The historical-baseline
//!   bulk filter and the working-tree bulk-self-filter share the
//!   knobs so they stay symmetrically tuned. Wide-grain repos
//!   (workspace projects, infra) need this to stop their natural
//!   feature commits from being dropped from the analyzer's
//!   commit set.
//! - **`coupling.confidence_threshold`** default 0.20 → 0.30 and
//!   **`coupling.min_sample_size`** default 1 → 3. The legacy
//!   `[coupling] threshold` alias and explicit settings are
//!   unaffected; defaults-only consumers will see noise-floor
//!   COUPLING cases drop out under the new gate.
//! - **Per-key monotonic-worsening dedup generalized** to COUPLING
//!   (key `coupling::<subject>::<partner>`, axes `[k, n]`) and
//!   STRUCTURE (key `structure::<path>`, axes
//!   `[missing_imports_count, missing_templates_count]`)
//!   alongside COMPLEXITY. LRU cap at 10 000 entries on save so
//!   the cache file stays bounded for any usage pattern.
//! - **`learn_sensor_stats`** (under `mmk eval --learn`) gains
//!   `cohesion_tangled_diffs_seen` and `cohesion_components_p95`;
//!   the text writer emits a suggested `[sensor.cohesion]` block
//!   when > 10 % of sampled commits would fire COHESION.
//! - **BUDGET wording on the bulk-self-filter path:** the
//!   suppressed-tail wording moved from "analysis suppressed" to
//!   "HOTSPOT/COUPLING skipped (partners co-touched by
//!   construction)" so an agent reading the hook output knows
//!   *what* was skipped and *why* — silence on HOTSPOT/COUPLING
//!   reads as "uncomputed at this scale" rather than "all clear."
//! - **`quiet_file` fall-through wording** distinguishes truly-new
//!   files from files in HEAD whose history was filtered:
//!   "new file (not yet in HEAD)" vs "present in HEAD but no
//!   analyzable history (file may be stale or prior touches were
//!   filtered as bulk commits)". The old conflation read as "no
//!   historical risk" on files with rich edit history that
//!   happened to be invisible to the bulk filter.
//!
//! ## v0.5.0 — recap
//!
//! - `findings[].layer = "structure"` and
//!   `findings[].layer = "complexity"` are populated. Two new
//!   directory-aggregated sensors (mmk-core::sensors::structure /
//!   complexity) ship: STRUCTURE surfaces convention divergence
//!   when ≥3 siblings share imports / export shape; COMPLEXITY
//!   fires per-function on nesting and LOC. Both are on by default
//!   under `[sensor]` config.
//! - `findings[].layer = "anchor"` — previously reserved — is now
//!   populated. `mmk session-summary` emits an Anchor Info finding
//!   when the session is empty (typically `--base HEAD`), nudging
//!   the user toward `mmk review` for working-tree state.
//! - New `[sensor.budget_ramp]` block (default `enabled = true`):
//!   `mmk review` and `mmk pre-edit` emit progressive Info @ ≥50%
//!   of cap and Warn @ ≥75% of cap so the agent sees the budget
//!   meter climb before it snaps over. Set `enabled = false` to
//!   silence.
//! - `mmk eval --replay` populates an optional `replay_histogram`
//!   block with per-layer fire rate, distinct paths surfaced, and
//!   severity mix across the sampled commits — designed for
//!   cross-repo aggregation.
//! - Bulk-self-filter no longer suppresses the per-file sensors:
//!   STRUCTURE and COMPLEXITY findings now surface alongside
//!   BUDGET on over-cap diffs (HOTSPOT/COUPLING still gated on the
//!   bulk path because they need the expensive analyze pass).
//! - `mmk pre-edit` normalizes absolute path inputs against the
//!   discovered repo root before lookup. Hook integrations passing
//!   `tool_input.file_path` (Claude Code) no longer silently
//!   degrade to the OK fall-through.
//! - STRUCTURE majority defaults raised from 0.66 → 0.85 after
//!   first-run agent calibration showed 0.66 fired too readily on
//!   directories with heterogeneous file roles.
//!
//! ## v0.4.0 — recap
//!
//! - Wilson 95 % lower-bound COUPLING gate; `top_couples[]` gains
//!   `conditional_probability` and `wilson_lower_95`.
//! - `config.coupling` gains `confidence_threshold` /
//!   `min_sample_size`; legacy `threshold` mapped with deprecation.
//! - Health adapter (TypeScript) ships;
//!   `findings[].layer = "health"` populated; optional
//!   `health` block on review / pre-edit envelopes.
//! - `mmk eval` renames `jaccard_buckets` → `wilson_lower_buckets`;
//!   `--learn` emits `learn_suggestions[]`.
//! - `Severity::Ok` populated by pre-edit's quiet-file fall-through.
//!
//! Consumers (LLM harnesses, CI scripts) pin against this rather
//! than the crate version, which they should treat as diagnostic
//! only.

pub const SCHEMA_VERSION: &str = "0.9.0";
