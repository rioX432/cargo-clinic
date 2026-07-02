//! Collectors, analyzers, prescriber. See CLAUDE.md / design docs.
//!
//! Issue #1 lands the metadata collector and duplicate-version analyzer.
//! Issue #2 adds the inventory analyzers (proc-macro / build.rs /
//! default-features) and the `cargo tree` feature-why helper.
//! Issue #3 adds the timing collector model: JSONL persistence, aggregation,
//! and the experimental nightly `--timings=json` import.
//! Issue #4 adds the prescriber + report: ranked, prescriptive findings with
//! qualitative impact, rendered as table / JSON / markdown.
//! Issue #5 adds the baseline snapshot + `cargo clinic check` regression gate:
//! a saved baseline compared against a fresh snapshot, failing CI on new
//! duplicates / heavy deps or a noise-aware timing regression.

mod analyzers;
mod baseline;
mod collector;
mod duplicates;
mod error;
mod feature_why;
mod model;
mod prescriber;
mod timing;

pub use analyzers::{BuildScriptCrate, DefaultFeaturesOpportunity, ProcMacroCrate};
pub use baseline::{
    Baseline, BaselineError, CheckReport, CheckReportView, CrateTimingSnapshot,
    DuplicateRegression, DuplicateSnapshot, EnvironmentFingerprint, HeavyDepKind, HeavyDepSnapshot,
    TimingComparison, TimingRegression, TimingSnapshot, BASELINE_SCHEMA_VERSION,
    DEFAULT_TIMING_THRESHOLD_PERCENT, TOTAL_SCOPE,
};
pub use collector::MetadataCollector;
pub use duplicates::{DuplicateInstance, DuplicatePackage};
pub use error::CollectError;
pub use feature_why::{feature_why, FeatureWhyError};
pub use model::{DepKind, DependencyDecl, DependencyGraph, Impact, Package, PackageId, PackageRef};
pub use prescriber::{
    find_quantitative_claim, Finding, FindingKind, NextTool, Prescription, Reference, Report,
    ReportView, REPORT_DISCLAIMER,
};
pub use timing::{
    append_invocation, crate_name_from_args, import_nightly_timings, read_invocations, CrateTiming,
    NightlyTimingUnit, RustcInvocation, TimingError, TimingReport, TimingReportView, TimingSource,
    NIGHTLY_IMPORT_DISCLAIMER, WRAPPER_DISCLAIMER,
};
