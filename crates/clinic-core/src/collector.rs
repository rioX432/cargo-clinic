//! Ingest `cargo metadata` output into the internal [`DependencyGraph`] model.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use cargo_metadata::{Metadata, MetadataCommand};

use crate::error::CollectError;
use crate::model::{DependencyGraph, Package, PackageId};

/// Collects a resolved dependency graph via `cargo metadata`.
///
/// Works for both workspace and single-crate targets: `cargo metadata`
/// resolves whichever manifest it is pointed at.
pub struct MetadataCollector;

impl MetadataCollector {
    /// Run `cargo metadata` for the workspace or crate at `manifest_path`.
    pub fn from_manifest_path(
        manifest_path: impl AsRef<Path>,
    ) -> Result<DependencyGraph, CollectError> {
        let metadata = MetadataCommand::new()
            .manifest_path(manifest_path.as_ref())
            .exec()?;
        build_graph(&metadata)
    }

    /// Run `cargo metadata` for the current working directory.
    pub fn from_current_dir() -> Result<DependencyGraph, CollectError> {
        let metadata = MetadataCommand::new().exec()?;
        build_graph(&metadata)
    }
}

/// Build the internal model from a `cargo metadata` result.
fn build_graph(metadata: &Metadata) -> Result<DependencyGraph, CollectError> {
    let resolve = metadata
        .resolve
        .as_ref()
        .ok_or(CollectError::MissingResolve)?;

    let workspace_members: BTreeSet<PackageId> = metadata
        .workspace_members
        .iter()
        .map(|id| id.repr.clone())
        .collect();

    let mut packages: BTreeMap<PackageId, Package> = BTreeMap::new();
    for pkg in &metadata.packages {
        let id = pkg.id.repr.clone();
        let is_workspace_member = workspace_members.contains(&id);
        packages.insert(
            id.clone(),
            Package {
                id,
                name: pkg.name.as_ref().to_owned(),
                version: pkg.version.to_string(),
                is_workspace_member,
            },
        );
    }

    // Build reverse edges from the resolved graph. `deps` covers all
    // dependency kinds (normal, build, dev) and handles renames.
    let mut reverse: BTreeMap<PackageId, BTreeSet<PackageId>> = BTreeMap::new();
    for node in &resolve.nodes {
        let from = node.id.repr.clone();
        if !packages.contains_key(&from) {
            return Err(CollectError::UnknownPackage(from));
        }
        for dep in &node.deps {
            let to = dep.pkg.repr.clone();
            if !packages.contains_key(&to) {
                return Err(CollectError::UnknownPackage(to));
            }
            reverse.entry(to).or_default().insert(from.clone());
        }
    }

    Ok(DependencyGraph::from_parts(packages, reverse))
}
