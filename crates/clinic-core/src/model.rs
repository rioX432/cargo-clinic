//! Internal dependency-graph model.
//!
//! This model is deliberately decoupled from `cargo_metadata` types so that
//! analyzers depend only on this stable surface, not on the metadata crate.

use std::collections::{BTreeMap, BTreeSet};

/// Opaque package identifier, mirroring cargo's `PackageId` string form.
pub type PackageId = String;

/// A resolved package in the dependency graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// Opaque unique identifier.
    pub id: PackageId,
    /// Crate name (may be shared across multiple versions).
    pub name: String,
    /// Resolved semantic version, rendered as a string.
    pub version: String,
    /// Whether this package is a member of the target workspace.
    pub is_workspace_member: bool,
}

/// A resolved dependency graph built from `cargo metadata`.
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    /// All resolved packages, keyed by id and kept ordered for determinism.
    packages: BTreeMap<PackageId, Package>,
    /// Reverse edges: package id -> ids of packages that directly depend on it.
    reverse: BTreeMap<PackageId, BTreeSet<PackageId>>,
}

impl DependencyGraph {
    /// Assemble a graph from its parts. Used by the collector.
    pub(crate) fn from_parts(
        packages: BTreeMap<PackageId, Package>,
        reverse: BTreeMap<PackageId, BTreeSet<PackageId>>,
    ) -> Self {
        Self { packages, reverse }
    }

    /// Iterate over all resolved packages, ordered by id.
    pub fn packages(&self) -> impl Iterator<Item = &Package> {
        self.packages.values()
    }

    /// Look up a package by id.
    pub fn package(&self, id: &str) -> Option<&Package> {
        self.packages.get(id)
    }

    /// Ids of packages that directly depend on `id`, ordered.
    pub(crate) fn direct_dependents(&self, id: &str) -> impl Iterator<Item = &PackageId> {
        self.reverse.get(id).into_iter().flatten()
    }
}
