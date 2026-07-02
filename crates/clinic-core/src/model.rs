//! Internal dependency-graph model.
//!
//! This model is deliberately decoupled from `cargo_metadata` types so that
//! analyzers depend only on this stable surface, not on the metadata crate.

use std::collections::{BTreeMap, BTreeSet};

/// Opaque package identifier, mirroring cargo's `PackageId` string form.
pub type PackageId = String;

/// Qualitative impact of a finding.
///
/// Impact is never a fabricated duration (see CLAUDE.md Core Values): we only
/// distinguish "likely" from "possible" so prescriptions stay honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Impact {
    /// The finding is very likely to matter for build cost.
    Likely,
    /// The finding may matter, but requires user judgment to confirm.
    Possible,
}

/// The kind of a declared dependency, mirroring cargo's dependency kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    /// A normal (runtime) dependency.
    Normal,
    /// A build-time dependency (used by build scripts).
    Build,
    /// A dev-dependency (tests, examples, benches).
    Development,
}

/// A dependency as *declared* in a package's `Cargo.toml`.
///
/// This captures the manifest-level declaration (not the resolved node) so
/// analyzers can reason about author intent, e.g. whether default features
/// were left enabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyDecl {
    /// Crate name as declared.
    pub name: String,
    /// Declared dependency kind.
    pub kind: DepKind,
    /// Whether the declaration keeps the dependency's default features on.
    pub uses_default_features: bool,
    /// Whether the dependency is optional.
    pub optional: bool,
}

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
    /// Whether this package is a proc-macro crate (`lib.proc-macro = true`).
    pub is_proc_macro: bool,
    /// Whether this package owns a build script (a `custom-build` target).
    pub has_build_script: bool,
    /// Expansion of this crate's own `default` feature (empty if it defines
    /// no default feature or the default set is empty).
    pub default_features: Vec<String>,
    /// Dependencies as declared in this package's manifest.
    pub dependencies: Vec<DependencyDecl>,
}

/// Lightweight reference to a package for reporting.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackageRef {
    /// Crate name.
    pub name: String,
    /// Resolved version.
    pub version: String,
    /// Opaque unique identifier.
    pub id: PackageId,
}

impl From<&Package> for PackageRef {
    fn from(pkg: &Package) -> Self {
        PackageRef {
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            id: pkg.id.clone(),
        }
    }
}

/// A resolved dependency graph built from `cargo metadata`.
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    /// All resolved packages, keyed by id and kept ordered for determinism.
    packages: BTreeMap<PackageId, Package>,
    /// Forward edges: package id -> ids of its direct dependencies.
    forward: BTreeMap<PackageId, BTreeSet<PackageId>>,
    /// Reverse edges: package id -> ids of packages that directly depend on it.
    reverse: BTreeMap<PackageId, BTreeSet<PackageId>>,
}

impl DependencyGraph {
    /// Assemble a graph from its parts. Used by the collector.
    pub(crate) fn from_parts(
        packages: BTreeMap<PackageId, Package>,
        forward: BTreeMap<PackageId, BTreeSet<PackageId>>,
        reverse: BTreeMap<PackageId, BTreeSet<PackageId>>,
    ) -> Self {
        Self {
            packages,
            forward,
            reverse,
        }
    }

    /// Iterate over all resolved packages, ordered by id.
    pub fn packages(&self) -> impl Iterator<Item = &Package> {
        self.packages.values()
    }

    /// Iterate over resolved packages that are workspace members, ordered by id.
    pub(crate) fn workspace_members(&self) -> impl Iterator<Item = &Package> {
        self.packages.values().filter(|p| p.is_workspace_member)
    }

    /// Look up a package by id.
    pub fn package(&self, id: &str) -> Option<&Package> {
        self.packages.get(id)
    }

    /// Ids of `id`'s direct dependencies, ordered.
    pub(crate) fn direct_dependencies(&self, id: &str) -> impl Iterator<Item = &PackageId> {
        self.forward.get(id).into_iter().flatten()
    }

    /// Ids of packages that directly depend on `id`, ordered.
    pub(crate) fn direct_dependents(&self, id: &str) -> impl Iterator<Item = &PackageId> {
        self.reverse.get(id).into_iter().flatten()
    }
}
