//! Collectors, analyzers, prescriber. See CLAUDE.md / design docs.
//!
//! Issue #1 lands the metadata collector and duplicate-version analyzer.

mod collector;
mod duplicates;
mod error;
mod model;

pub use collector::MetadataCollector;
pub use duplicates::{DuplicateInstance, DuplicatePackage, PackageRef};
pub use error::CollectError;
pub use model::{DependencyGraph, Package, PackageId};
