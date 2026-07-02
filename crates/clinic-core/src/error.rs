//! Error types for collectors.

use thiserror::Error;

/// Errors raised while collecting a dependency graph via `cargo metadata`.
#[derive(Debug, Error)]
pub enum CollectError {
    /// `cargo metadata` failed to run or its output could not be parsed.
    #[error("failed to obtain cargo metadata: {0}")]
    Metadata(#[from] cargo_metadata::Error),

    /// `cargo metadata` was run without a dependency resolution graph.
    #[error("cargo metadata output has no dependency resolution graph; re-run without --no-deps")]
    MissingResolve,

    /// A package referenced by the resolve graph is missing from the package list.
    #[error("package id `{0}` appears in the resolve graph but not in the package list")]
    UnknownPackage(String),
}
