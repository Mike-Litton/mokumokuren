//! Language-agnostic structural facts extracted from a source file.
//!
//! Sensors (STRUCTURE, COMPLEXITY, …) operate only on
//! [`StructuredFacts`]; whatever per-language adapter produced them
//! is opaque past this boundary. Adding a new language means writing
//! one more `mmk-health/src/<lang>/` adapter that emits these types
//! — every sensor automatically picks it up.

use serde::Serialize;

/// What a single import or `use`/`from` statement contributes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportFact {
    /// The module specifier — quoted string in JS/TS, dotted module
    /// path in Python, `use` path in Rust. Compared verbatim across
    /// siblings to compute STRUCTURE's "common imports" set.
    pub source: String,
    /// Names imported from that source. May be empty (`import 'foo'`,
    /// `use foo;`). Order is the import statement's source order.
    pub symbols: Vec<String>,
}

/// What a single top-level declaration contributes when it's
/// exported (or always, in languages without an explicit export).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportFact {
    /// The declared name as it would appear at the call site
    /// (`CreateAwardDialog`, `parseApplication`).
    pub name: String,
    pub kind: ExportKind,
    /// Name with the file's basename stem (PascalCased) replaced by
    /// `*`. STRUCTURE uses this to spot conventions like
    /// `Create<X>Dialog` across siblings: if the sibling's filename
    /// stem appears inside the export name, the convention is the
    /// *shape*, not the literal name.
    ///
    /// Example: file `award.tsx`, export `CreateAwardDialog` →
    /// `template_stem = "Create*Dialog"`. Two siblings with stems
    /// `award` and `goal` exporting `CreateAwardDialog` and
    /// `CreateGoalDialog` collapse to the same template, making the
    /// shared convention visible.
    pub template_stem: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportKind {
    Function,
    Class,
    Const,
    Type,
}

/// Per-function structural facts COMPLEXITY consumes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionFact {
    pub name: String,
    /// Lines from the function header (or first-non-blank body line)
    /// through the closing brace, inclusive — matches what a reader
    /// would point at when saying "this function is N lines."
    pub loc: u32,
    /// Deepest control-flow nesting inside the body. The function's
    /// own body counts as depth 1; an `if` inside it is depth 2;
    /// nested `if/for/while/try/switch/match` push deeper.
    pub max_nesting_depth: u32,
}

/// Type-density summary for a single source file.
///
/// Currently only surfaced via the TS adapter; the v0.7
/// VERIFIABILITY sensor reads these fields. Other adapters set the
/// counters to 0 and the sensor refuses to fire on them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TypeDensity {
    pub total_value_exprs: u32,
    pub typed_value_exprs: u32,
    pub any_count: u32,
    pub unknown_count: u32,
}

/// Everything a sensor needs from one source file. Produced by a
/// per-language adapter; consumed by language-agnostic code.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StructuredFacts {
    pub imports: Vec<ImportFact>,
    pub exports: Vec<ExportFact>,
    pub functions: Vec<FunctionFact>,
    pub type_density: TypeDensity,
}

/// Replace the sibling-file basename stem with `*` inside `name` to
/// produce an [`ExportFact::template_stem`].
///
/// Matches the *PascalCased* form of the stem: filename `award.tsx`
/// pulls stem `award`, normalises to `Award`, and replaces only the
/// `Award` substring inside `CreateAwardDialog`. Plain lowercase
/// matching would be too permissive — it'd collapse `useAward` and
/// `awardSomething` into one shape that isn't really shared.
///
/// If the stem doesn't appear inside `name`, returns `name`
/// unchanged — the export simply isn't a template-shaped one.
#[must_use]
pub fn template_for(file_stem: &str, name: &str) -> String {
    if file_stem.is_empty() {
        return name.to_owned();
    }
    let pascal = pascal_case(file_stem);
    if pascal.is_empty() {
        return name.to_owned();
    }
    if let Some(idx) = name.find(&pascal) {
        let mut out = String::with_capacity(name.len());
        out.push_str(&name[..idx]);
        out.push('*');
        out.push_str(&name[idx + pascal.len()..]);
        return out;
    }
    name.to_owned()
}

/// `award` → `Award`, `job-tracker` → `JobTracker`,
/// `useResumeStore` → `UseResumeStore`. Best-effort: splits on
/// non-alphanumerics and uppercases the first letter of each part.
fn pascal_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut next_upper = true;
    for ch in s.chars() {
        if ch == '-' || ch == '_' || ch == '.' {
            next_upper = true;
            continue;
        }
        if next_upper {
            out.extend(ch.to_uppercase());
            next_upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{pascal_case, template_for};

    #[test]
    fn template_for_replaces_pascalcased_stem() {
        assert_eq!(template_for("award", "CreateAwardDialog"), "Create*Dialog");
    }

    #[test]
    fn template_for_handles_kebab_stem() {
        assert_eq!(
            template_for("job-tracker", "CreateJobTrackerDialog"),
            "Create*Dialog"
        );
    }

    #[test]
    fn template_for_returns_name_unchanged_when_stem_absent() {
        assert_eq!(template_for("award", "Helper"), "Helper");
    }

    #[test]
    fn template_for_empty_stem_returns_name() {
        assert_eq!(template_for("", "CreateAwardDialog"), "CreateAwardDialog");
    }

    #[test]
    fn pascal_case_kebab_to_pascal() {
        assert_eq!(pascal_case("job-tracker"), "JobTracker");
        assert_eq!(pascal_case("award"), "Award");
        assert_eq!(pascal_case("create_thing"), "CreateThing");
    }
}
