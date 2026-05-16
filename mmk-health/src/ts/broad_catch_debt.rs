//! BROAD_CATCH_DEBT — static count of broad non-top-level catch
//! handlers in the working tree.
//!
//! The audit-mode counterpart to EVASION (`broad_exception`). Where
//! EVASION fires only on the *addition* of a broad catch (working
//! count > HEAD count), `BroadCatchDebt` reports the static count
//! as it stands — the right shape for codebases that already
//! accumulated evasion debt before mmk was enabled.
//!
//! Reuses the predicate set from `broad_exception::is_broad`, so the
//! same shapes count in both modes (empty body, no parameter,
//! `any | unknown | Error` typed param, log-and-swallow on a
//! configured log identifier).

use crate::ts::broad_exception::collect_broad_non_top_level_with_locations;
use crate::{HealthFinding, HealthFindingDetail, HealthPattern};
use std::path::Path;

/// Emit one `BroadCatchDebt` finding for `subject` if the working
/// body contains at least one broad non-top-level catch handler.
/// Returns an empty vec for clean files.
///
/// `log_identifiers` is propagated to the same `is_broad` predicate
/// EVASION uses, so the log-and-swallow shape counts here too.
#[must_use]
pub fn detect(subject: &Path, body: &str, log_identifiers: &[String]) -> Vec<HealthFinding> {
    let locations = collect_broad_non_top_level_with_locations(subject, body, log_identifiers);
    if locations.is_empty() {
        return Vec::new();
    }
    let count = u32::try_from(locations.len()).unwrap_or(u32::MAX);
    let lines: Vec<usize> = locations.iter().map(|(line, _col)| *line).collect();
    vec![HealthFinding {
        pattern: HealthPattern::BroadCatchDebt,
        subject: subject.to_path_buf(),
        related: Vec::new(),
        detail: Some(HealthFindingDetail::BroadCatchDebt { count, lines }),
    }]
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

    fn detail(f: &HealthFinding) -> (u32, Vec<usize>) {
        match &f.detail {
            Some(HealthFindingDetail::BroadCatchDebt { count, lines }) => (*count, lines.clone()),
            _ => panic!("expected BroadCatchDebt detail; got {:?}", f.detail),
        }
    }

    #[test]
    fn single_broad_handler_emits_count_one() {
        let body = "function f() { try { g(); } catch {} }";
        let f = detect(&ts_subject(), body, &default_log_ids());
        assert_eq!(f.len(), 1);
        let (count, lines) = detail(&f[0]);
        assert_eq!(count, 1);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn three_broad_handlers_emits_count_three_with_lines() {
        let body = "function f() { try { g(); } catch {} }\n\
                    function h() { try { g(); } catch (e: any) { throw e; } }\n\
                    function i() { try { g(); } catch (e) {} }\n";
        let f = detect(&ts_subject(), body, &default_log_ids());
        assert_eq!(f.len(), 1);
        let (count, lines) = detail(&f[0]);
        assert_eq!(count, 3);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn top_level_handlers_excluded() {
        let body = "try { boot(); } catch {}\n\
                    function f() { try { g(); } catch {} }\n";
        let f = detect(&ts_subject(), body, &default_log_ids());
        assert_eq!(f.len(), 1);
        let (count, _) = detail(&f[0]);
        assert_eq!(count, 1, "top-level catch must be excluded");
    }

    #[test]
    fn log_and_swallow_counted() {
        let body = "function f() { try { g(); } catch (e) { logger.warn(e); } }";
        let f = detect(&ts_subject(), body, &default_log_ids());
        assert_eq!(f.len(), 1);
        let (count, _) = detail(&f[0]);
        assert_eq!(count, 1);
    }

    #[test]
    fn empty_file_emits_no_finding() {
        let body = "";
        let f = detect(&ts_subject(), body, &default_log_ids());
        assert!(f.is_empty());
    }

    #[test]
    fn tsx_grammar_dispatched() {
        let body = "function App() { try { f(); } catch {} return <div />; }";
        let subject = PathBuf::from("src/App.tsx");
        let f = detect(&subject, body, &default_log_ids());
        assert_eq!(f.len(), 1, "tsx grammar must parse JSX correctly");
    }
}
