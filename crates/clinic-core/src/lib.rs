//! Collectors, analyzers, prescriber. See CLAUDE.md / design docs.
//!
//! Issue #1 lands the metadata collector and duplicate-version analyzer.
//! Issue #2 adds the inventory analyzers (proc-macro / build.rs /
//! default-features) and the `cargo tree` feature-why helper.

mod analyzers;
mod collector;
mod duplicates;
mod error;
mod feature_why;
mod model;

pub use analyzers::{BuildScriptCrate, DefaultFeaturesOpportunity, ProcMacroCrate};
pub use collector::MetadataCollector;
pub use duplicates::{DuplicateInstance, DuplicatePackage};
pub use error::CollectError;
pub use feature_why::{feature_why, FeatureWhyError};
pub use model::{DepKind, DependencyDecl, DependencyGraph, Impact, Package, PackageId, PackageRef};
