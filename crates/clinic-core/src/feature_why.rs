//! Explain why a crate/feature is enabled, by invoking cargo's own resolver.
//!
//! Project rule (CLAUDE.md "Key Gotchas" / ADR): we do NOT reimplement feature
//! unification. To answer "why is this feature on", we shell out to
//! `cargo tree -e features -i <spec>` and hand back its output for the report
//! layer to present.

use std::path::Path;
use std::process::Command;

use thiserror::Error;

/// Errors raised while running `cargo tree` to explain a feature.
#[derive(Debug, Error)]
pub enum FeatureWhyError {
    /// The `cargo tree` process could not be spawned.
    #[error("failed to run `cargo tree`: {0}")]
    Spawn(#[from] std::io::Error),

    /// `cargo tree` ran but exited unsuccessfully.
    #[error("`cargo tree` failed ({status}): {stderr}")]
    CargoTree {
        /// Rendered exit status.
        status: String,
        /// Captured standard error.
        stderr: String,
    },

    /// `cargo tree` produced output that was not valid UTF-8.
    #[error("`cargo tree` produced non-UTF-8 output")]
    NonUtf8,
}

/// Run `cargo tree -e features -i <spec> --manifest-path <manifest_path>` and
/// return its stdout.
///
/// `spec` is a cargo package spec (e.g. `serde` or `serde@1.0.0`). The inverted
/// (`-i`) feature-edge tree shows *why* each feature on that crate is enabled.
pub fn feature_why(manifest_path: impl AsRef<Path>, spec: &str) -> Result<String, FeatureWhyError> {
    let output = Command::new("cargo")
        .args(["tree", "-e", "features", "-i", spec, "--manifest-path"])
        .arg(manifest_path.as_ref())
        .output()?;

    if !output.status.success() {
        return Err(FeatureWhyError::CargoTree {
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    String::from_utf8(output.stdout).map_err(|_| FeatureWhyError::NonUtf8)
}
