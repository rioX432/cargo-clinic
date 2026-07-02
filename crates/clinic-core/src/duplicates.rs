//! Duplicate-version detection and inverse dependency-path reverse lookup.
//!
//! Mirrors `cargo tree -d -i`: for each crate resolved to more than one
//! version, report every version together with the crates and inverse paths
//! that pull it in.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{DependencyGraph, Package, PackageId};

/// Minimum distinct versions of one crate name for it to count as duplicated.
const DUPLICATE_MIN_VERSIONS: usize = 2;

/// A crate name resolved to more than one version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicatePackage {
    /// The shared crate name.
    pub name: String,
    /// The resolved instances, ordered by version then id.
    pub instances: Vec<DuplicateInstance>,
}

/// One resolved instance (version) of a duplicated crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateInstance {
    /// Opaque unique identifier of this instance.
    pub id: PackageId,
    /// Resolved version of this instance.
    pub version: String,
    /// Crates that directly depend on this instance, ordered.
    pub direct_dependents: Vec<PackageRef>,
    /// Inverse dependency paths from this instance up to graph roots (crates
    /// with no further dependents, i.e. top-level / workspace members). Each
    /// path starts at this instance and ends at a root.
    pub inverse_paths: Vec<Vec<PackageRef>>,
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

impl DependencyGraph {
    /// Detect crates resolved to multiple versions and, for each version, the
    /// crates and inverse paths that require it.
    ///
    /// Output is deterministic: duplicates are ordered by name, instances by
    /// version, and dependents/paths are sorted.
    pub fn duplicate_versions(&self) -> Vec<DuplicatePackage> {
        // Group packages by crate name.
        let mut by_name: BTreeMap<&str, Vec<&Package>> = BTreeMap::new();
        for pkg in self.packages() {
            by_name.entry(pkg.name.as_str()).or_default().push(pkg);
        }

        let mut duplicates = Vec::new();
        for (name, pkgs) in by_name {
            let distinct_versions: BTreeSet<&str> =
                pkgs.iter().map(|p| p.version.as_str()).collect();
            if distinct_versions.len() < DUPLICATE_MIN_VERSIONS {
                continue;
            }

            let mut instances: Vec<DuplicateInstance> =
                pkgs.iter().map(|p| self.instance(p)).collect();
            instances.sort_by(|a, b| a.version.cmp(&b.version).then_with(|| a.id.cmp(&b.id)));

            duplicates.push(DuplicatePackage {
                name: name.to_owned(),
                instances,
            });
        }
        duplicates
    }

    /// Build a [`DuplicateInstance`] for a single package.
    fn instance(&self, pkg: &Package) -> DuplicateInstance {
        let mut direct_dependents: Vec<PackageRef> = self
            .direct_dependents(&pkg.id)
            .filter_map(|id| self.package(id))
            .map(PackageRef::from)
            .collect();
        direct_dependents.sort();

        let mut inverse_paths = self.inverse_paths(&pkg.id);
        inverse_paths.sort();

        DuplicateInstance {
            id: pkg.id.clone(),
            version: pkg.version.clone(),
            direct_dependents,
            inverse_paths,
        }
    }

    /// Enumerate inverse dependency paths from `start` up to graph roots.
    fn inverse_paths(&self, start: &str) -> Vec<Vec<PackageRef>> {
        let mut paths = Vec::new();
        let mut stack: Vec<PackageId> = vec![start.to_owned()];
        self.walk_up(start, &mut stack, &mut paths);
        paths
    }

    /// Depth-first walk up the reverse edges, emitting one path per root
    /// reached. `stack` holds the current path from the start instance to
    /// `node`; a cycle guard prevents revisiting nodes on the current path.
    fn walk_up(&self, node: &str, stack: &mut Vec<PackageId>, paths: &mut Vec<Vec<PackageRef>>) {
        let parents: Vec<&PackageId> = self.direct_dependents(node).collect();
        let mut advanced = false;
        for parent in parents {
            if stack.iter().any(|on_path| on_path == parent) {
                continue; // cycle guard
            }
            advanced = true;
            stack.push(parent.clone());
            self.walk_up(parent, stack, paths);
            stack.pop();
        }
        if !advanced {
            // Root reached (no dependents, or all remaining parents are cycles).
            let path: Vec<PackageRef> = stack
                .iter()
                .filter_map(|id| self.package(id))
                .map(PackageRef::from)
                .collect();
            paths.push(path);
        }
    }
}
