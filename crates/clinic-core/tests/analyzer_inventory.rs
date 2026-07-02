//! Golden tests for the inventory analyzers against the planted
//! `tests/fixtures/analyzer-workspace` fixture.
//!
//! Coverage:
//! - proc-macro inventory detects exactly the planted proc-macro, with its
//!   direct dependent
//! - build-script inventory detects exactly the planted build.rs crate, with
//!   its direct dependent
//! - default-features detection flags the crate whose defaults are left on and
//!   are non-empty, and does NOT flag the crate opted out with
//!   `default-features = false` (conservative: no false positive)
//! - the "why is it on" path shells out to `cargo tree -e features -i`

use std::path::PathBuf;

use cargo_clinic_core::{feature_why, Impact, MetadataCollector};

/// Path to the analyzer fixture workspace manifest.
fn workspace_manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/analyzer-workspace/Cargo.toml")
}

#[test]
fn proc_macro_inventory_matches_exactly() {
    // Arrange
    let graph = MetadataCollector::from_manifest_path(workspace_manifest())
        .expect("collect metadata for analyzer fixture");

    // Act
    let proc_macros = graph.proc_macro_crates();

    // Assert: exactly `mymacro`, pulled in directly by `app`.
    let names: Vec<&str> = proc_macros
        .iter()
        .map(|c| c.package.name.as_str())
        .collect();
    assert_eq!(names, vec!["mymacro"], "only the planted proc-macro");

    let dependents: Vec<&str> = proc_macros[0]
        .dependents
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(dependents, vec!["app"], "proc-macro dependent must be app");
}

#[test]
fn build_script_inventory_matches_exactly() {
    // Arrange
    let graph = MetadataCollector::from_manifest_path(workspace_manifest())
        .expect("collect metadata for analyzer fixture");

    // Act
    let build_scripts = graph.build_script_crates();

    // Assert: exactly `builder-dep`, pulled in directly by `app`.
    let names: Vec<&str> = build_scripts
        .iter()
        .map(|c| c.package.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["builder-dep"],
        "only the planted build.rs crate"
    );

    let dependents: Vec<&str> = build_scripts[0]
        .dependents
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(dependents, vec!["app"], "build.rs dependent must be app");
}

#[test]
fn default_features_flags_only_the_non_optout_non_empty_case() {
    // Arrange
    let graph = MetadataCollector::from_manifest_path(workspace_manifest())
        .expect("collect metadata for analyzer fixture");

    // Act
    let opportunities = graph.default_features_opportunities();

    // Assert: exactly `featured` is flagged. `lean` (opted out) and the
    // no-default-feature crates (mymacro, builder-dep) must not appear.
    let names: Vec<&str> = opportunities
        .iter()
        .map(|o| o.dependency.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["featured"],
        "only default-on + non-empty-default dependency is reported"
    );

    let opp = &opportunities[0];
    assert_eq!(opp.dependent.name, "app", "fix belongs in the app manifest");
    assert_eq!(
        opp.default_expands_to,
        vec!["extra".to_string()],
        "must surface which default feature would be dropped"
    );
    // Impact stays conservative: this is a possibility, not a confirmed win.
    assert_eq!(opp.impact, Impact::Possible);
}

#[test]
fn feature_why_shells_out_to_cargo_tree() {
    // Arrange & Act: ask cargo's own resolver why `featured` is present.
    let output = feature_why(workspace_manifest(), "featured")
        .expect("cargo tree -e features -i featured should succeed");

    // Assert: the inverted feature tree mentions the queried crate.
    assert!(
        output.contains("featured"),
        "cargo tree output should mention the queried crate; got:\n{output}"
    );
}
