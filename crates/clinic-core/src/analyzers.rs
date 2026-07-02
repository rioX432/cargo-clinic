//! Inventory analyzers: proc-macro crates, build-script crates, and
//! conservative default-features opportunities.
//!
//! These are *inventory* analyzers: they surface facts already present in the
//! resolved graph (which crates are proc-macros, which own build scripts) plus
//! one conservative judgment (default features that could be trimmed). None of
//! them predict durations; impact stays qualitative (see CLAUDE.md Core Values).

use crate::model::{DepKind, DependencyGraph, Impact, Package, PackageRef};

/// A proc-macro crate in the resolved graph, with its direct dependents.
///
/// Proc-macros extend compile time (they build and run at compile time); the
/// inventory helps a user see how many they pull in and through whom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcMacroCrate {
    /// The proc-macro package.
    pub package: PackageRef,
    /// Crates that directly depend on it, ordered.
    pub dependents: Vec<PackageRef>,
}

/// A crate that owns a build script (`build.rs` / `custom-build` target),
/// with its direct dependents.
///
/// Build scripts run at compile time and are invisible to a `RUSTC_WRAPPER`
/// timing shim, so an explicit inventory is the honest way to surface them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildScriptCrate {
    /// The package owning a build script.
    pub package: PackageRef,
    /// Crates that directly depend on it, ordered.
    pub dependents: Vec<PackageRef>,
}

/// A workspace dependency declared with default features enabled, whose
/// dependency defines a non-empty `default` feature set.
///
/// This is a *conservative* opportunity, not a confirmed win: we cannot know
/// from metadata alone whether the enabled defaults are actually used. The
/// [`impact`](Self::impact) is therefore [`Impact::Possible`], and any report
/// wording must stay hedged ("may", "possible") — disabling defaults only helps
/// if those features are genuinely unused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultFeaturesOpportunity {
    /// The workspace member that declares the dependency (where the fix goes).
    pub dependent: PackageRef,
    /// The resolved dependency whose default features are enabled.
    pub dependency: PackageRef,
    /// The features that the dependency's `default` feature turns on.
    pub default_expands_to: Vec<String>,
    /// Qualitative impact — always conservative for this analyzer.
    pub impact: Impact,
}

impl DependencyGraph {
    /// Inventory of proc-macro crates in the resolved graph, ordered by
    /// package (name, version, id).
    pub fn proc_macro_crates(&self) -> Vec<ProcMacroCrate> {
        let mut out: Vec<ProcMacroCrate> = self
            .packages()
            .filter(|p| p.is_proc_macro)
            .map(|p| ProcMacroCrate {
                package: PackageRef::from(p),
                dependents: self.direct_dependent_refs(&p.id),
            })
            .collect();
        out.sort_by(|a, b| a.package.cmp(&b.package));
        out
    }

    /// Inventory of build-script-owning crates in the resolved graph, ordered
    /// by package (name, version, id).
    pub fn build_script_crates(&self) -> Vec<BuildScriptCrate> {
        let mut out: Vec<BuildScriptCrate> = self
            .packages()
            .filter(|p| p.has_build_script)
            .map(|p| BuildScriptCrate {
                package: PackageRef::from(p),
                dependents: self.direct_dependent_refs(&p.id),
            })
            .collect();
        out.sort_by(|a, b| a.package.cmp(&b.package));
        out
    }

    /// Conservative default-features opportunities: workspace-member
    /// dependency declarations that keep default features on where the
    /// dependency actually defines a non-empty default set.
    ///
    /// Scope is limited to workspace members because that is where the user
    /// can actually apply the fix (`default-features = false`). Dev-only
    /// dependencies are ignored: they never affect the primary build.
    ///
    /// Output is ordered by (dependent, dependency) for determinism.
    pub fn default_features_opportunities(&self) -> Vec<DefaultFeaturesOpportunity> {
        let mut out = Vec::new();
        for member in self.workspace_members() {
            // Resolved direct dependencies of this member, to map a declared
            // dependency name onto its resolved package.
            let direct: Vec<&Package> = self
                .direct_dependencies(&member.id)
                .filter_map(|id| self.package(id))
                .collect();

            for decl in &member.dependencies {
                if matches!(decl.kind, DepKind::Development) {
                    continue; // dev-deps do not affect the primary build
                }
                if !decl.uses_default_features {
                    continue; // already opted out
                }
                let Some(dep_pkg) = direct.iter().find(|p| p.name == decl.name) else {
                    continue; // not resolved (e.g. platform-gated out)
                };
                if dep_pkg.default_features.is_empty() {
                    continue; // nothing to trim; no false-positive noise
                }
                out.push(DefaultFeaturesOpportunity {
                    dependent: PackageRef::from(member),
                    dependency: PackageRef::from(*dep_pkg),
                    default_expands_to: dep_pkg.default_features.clone(),
                    impact: Impact::Possible,
                });
            }
        }
        out.sort_by(|a, b| {
            a.dependent
                .cmp(&b.dependent)
                .then_with(|| a.dependency.cmp(&b.dependency))
        });
        out
    }

    /// Direct dependents of `id` as sorted [`PackageRef`]s.
    fn direct_dependent_refs(&self, id: &str) -> Vec<PackageRef> {
        let mut refs: Vec<PackageRef> = self
            .direct_dependents(id)
            .filter_map(|dependent_id| self.package(dependent_id))
            .map(PackageRef::from)
            .collect();
        refs.sort();
        refs
    }
}
