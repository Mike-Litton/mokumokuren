//! Pure architecture-fitness sensors.
//!
//! Each sensor is a function from a small input bundle to a list of
//! [`SensorFinding`]s. No I/O, no git: callers gather the input
//! (file bodies, sibling paths, configuration) and pass it in. The
//! output is consumed by `mmk-cli` to render the wording for either
//! `mmk pre-edit` or `mmk review`.
//!
//! Every sensor reads [`mmk_health::StructuredFacts`] only — adapter
//! or fallback details stay in `mmk-health`.

pub mod complexity;
pub mod structure;

pub use complexity::{
    compute_complexity_findings, ComplexityFinding, ComplexityFindingKind, ComplexityInput,
};
pub use structure::{
    compute_structure_finding, DirectoryConvention, StructureFinding, StructureFindingKind,
    StructureInput, StructureMode,
};

/// File body keyed by repo-relative path. Sensors look up the body
/// for a sibling without touching the filesystem; the caller is
/// responsible for reading the bytes once and reusing the map.
pub type FilesMap = ahash::AHashMap<std::path::PathBuf, String>;
