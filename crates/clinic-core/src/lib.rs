//! Collectors, analyzers, prescriber. See CLAUDE.md / design docs.
//!
//! Issue #1 lands the metadata collector and duplicate-version analyzer.
//! Issue #2 adds the inventory analyzers (proc-macro / build.rs /
//! default-features) and the `cargo tree` feature-why helper.
//! Issue #3 adds the timing collector model: JSONL persistence, aggregation,
//! and the experimental nightly `--timings=json` import.
//! Issue #4 adds the prescriber + report: ranked, prescriptive findings with
//! qualitative impact, rendered as table / JSON / markdown.

mod analyzers;
mod collector;
mod duplicates;
mod error;
mod feature_why;
mod model;
mod prescriber;
mod timing;

pub use analyzers::{BuildScriptCrate, DefaultFeaturesOpportunity, ProcMacroCrate};
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
