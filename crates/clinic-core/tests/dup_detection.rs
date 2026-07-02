//! Golden tests for duplicate-version detection against the planted
//! `tests/fixtures/dup-workspace` fixture.
//!
//! Coverage:
//! - all planted duplicates detected exactly (no false positives/negatives)
//! - inverse path reverse-lookup matches `cargo tree -d` (the inverted
//!   duplicates view; a bare `-i` requires a package spec)
//! - both workspace and single-crate targets work

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use cargo_clinic_core::{DuplicatePackage, MetadataCollector};

/// Path to the workspace fixture manifest.
fn workspace_manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/dup-workspace/Cargo.toml")
}

/// Path to a single (non-workspace) crate manifest inside the fixture tree.
fn single_crate_manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/dup-workspace/vendor/numparse-1.0/Cargo.toml")
}

/// Collect (name, version) pairs from the duplicate report.
fn name_version_pairs(dups: &[DuplicatePackage]) -> BTreeSet<(String, String)> {
    dups.iter()
        .flat_map(|d| {
            d.instances
                .iter()
                .map(move |i| (d.name.clone(), i.version.clone()))
        })
        .collect()
}

/// Find a duplicate package by name.
fn find<'a>(dups: &'a [DuplicatePackage], name: &str) -> &'a DuplicatePackage {
    dups.iter()
        .find(|d| d.name == name)
        .unwrap_or_else(|| panic!("expected `{name}` to be reported as duplicated"))
}

/// Direct-dependent (name, version) pairs for a given instance version.
fn dependents_of(dup: &DuplicatePackage, version: &str) -> BTreeSet<(String, String)> {
    let instance = dup
        .instances
        .iter()
        .find(|i| i.version == version)
        .unwrap_or_else(|| panic!("expected `{}` v{version}", dup.name));
    instance
        .direct_dependents
        .iter()
        .map(|d| (d.name.clone(), d.version.clone()))
        .collect()
}

#[test]
fn detects_all_planted_duplicates_exactly() {
    // Arrange
    let graph = MetadataCollector::from_manifest_path(workspace_manifest())
        .expect("collect metadata for fixture workspace");

    // Act
    let dups = graph.duplicate_versions();

    // Assert: exactly the two planted duplicate crate names, nothing else.
    let names: BTreeSet<&str> = dups.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        names,
        BTreeSet::from(["leftpad", "numparse"]),
        "only the planted crates must be reported as duplicated"
    );

    // Assert: exact (name, version) instance set.
    let expected: BTreeSet<(String, String)> = [
        ("leftpad", "0.1.0"),
        ("leftpad", "0.2.0"),
        ("numparse", "1.0.0"),
        ("numparse", "2.0.0"),
    ]
    .iter()
    .map(|(n, v)| (n.to_string(), v.to_string()))
    .collect();
    assert_eq!(name_version_pairs(&dups), expected);
}

#[test]
fn reverse_lookup_matches_expected_dependents() {
    // Arrange
    let graph = MetadataCollector::from_manifest_path(workspace_manifest())
        .expect("collect metadata for fixture workspace");
    let dups = graph.duplicate_versions();

    // Assert direct dependents per instance (mirrors `cargo tree -d -i` roots).
    let leftpad = find(&dups, "leftpad");
    assert_eq!(
        dependents_of(leftpad, "0.1.0"),
        BTreeSet::from([("app-a".to_string(), "0.0.0".to_string())])
    );
    assert_eq!(
        dependents_of(leftpad, "0.2.0"),
        BTreeSet::from([("mid".to_string(), "0.0.0".to_string())])
    );

    let numparse = find(&dups, "numparse");
    assert_eq!(
        dependents_of(numparse, "1.0.0"),
        BTreeSet::from([("app-a".to_string(), "0.0.0".to_string())])
    );
    assert_eq!(
        dependents_of(numparse, "2.0.0"),
        BTreeSet::from([("app-b".to_string(), "0.0.0".to_string())])
    );
}

#[test]
fn inverse_paths_reach_workspace_roots() {
    // Arrange
    let graph = MetadataCollector::from_manifest_path(workspace_manifest())
        .expect("collect metadata for fixture workspace");
    let dups = graph.duplicate_versions();

    // leftpad 0.2.0 is pulled in through mid, whose only dependent is app-b:
    // the inverse path must be leftpad -> mid -> app-b.
    let leftpad = find(&dups, "leftpad");
    let v02 = leftpad
        .instances
        .iter()
        .find(|i| i.version == "0.2.0")
        .expect("leftpad 0.2.0");
    let path_names: Vec<Vec<&str>> = v02
        .inverse_paths
        .iter()
        .map(|p| p.iter().map(|r| r.name.as_str()).collect())
        .collect();
    assert_eq!(path_names, vec![vec!["leftpad", "mid", "app-b"]]);
}

#[test]
fn cross_check_against_cargo_tree_duplicates() {
    // Arrange: our detection.
    let graph = MetadataCollector::from_manifest_path(workspace_manifest())
        .expect("collect metadata for fixture workspace");
    let ours = name_version_pairs(&graph.duplicate_versions());

    // Act: parse `cargo tree -d` top-level entries. `-d` (duplicates) already
    // inverts the tree and roots each duplicated instance at column 0 as
    // "name vX.Y.Z (path)"; a bare `-i` additionally requires a package spec.
    let output = Command::new("cargo")
        .args(["tree", "-d", "--manifest-path"])
        .arg(workspace_manifest())
        .output()
        .expect("run cargo tree");
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 cargo tree output");

    let mut from_tree: BTreeSet<(String, String)> = BTreeSet::new();
    for line in stdout.lines() {
        // Top-level entries are not indented and not tree-branch continuations.
        if line.is_empty() || line.starts_with(char::is_whitespace) {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(name), Some(version)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Some(version) = version.strip_prefix('v') else {
            continue;
        };
        from_tree.insert((name.to_string(), version.to_string()));
    }

    // Assert: our duplicate instances equal cargo tree's duplicate instances.
    assert_eq!(ours, from_tree, "detection must match `cargo tree -d`");
}

#[test]
fn single_crate_target_has_no_duplicates() {
    // Arrange & Act: a standalone crate resolves with no duplicates.
    let graph = MetadataCollector::from_manifest_path(single_crate_manifest())
        .expect("collect metadata for single crate");

    // Assert: the collector works for a non-workspace target and reports none.
    assert!(graph.duplicate_versions().is_empty());
    assert!(
        graph.packages().any(|p| p.name == "numparse"),
        "single-crate graph must contain the crate itself"
    );
}
