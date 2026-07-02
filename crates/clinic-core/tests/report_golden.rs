//! Golden snapshot tests for the prescriber + report against the planted
//! fixtures, plus the two structural machine checks required by issue #4:
//!
//! - every finding carries a non-empty prescription (Core Value 1: a finding
//!   without a fix is a release blocker);
//! - no rendered output contains a quantitative build-time prediction
//!   (ADR-0002 output lint).
//!
//! Golden files live in `tests/golden/`. Rendered output is compared with
//! trailing whitespace normalized (the CLI adds a trailing newline that the
//! renderers themselves do not), so the snapshots stay stable.

use std::path::PathBuf;

use cargo_clinic_core::{find_quantitative_claim, MetadataCollector, Report};

fn manifest(fixture: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../tests/fixtures/{fixture}/Cargo.toml"))
}

fn report_for(fixture: &str) -> Report {
    let graph = MetadataCollector::from_manifest_path(manifest(fixture))
        .unwrap_or_else(|e| panic!("collect metadata for `{fixture}`: {e}"));
    Report::diagnose(&graph)
}

/// Compare with trailing-whitespace normalization.
fn assert_matches_golden(rendered: &str, golden: &str, label: &str) {
    assert_eq!(
        rendered.trim_end(),
        golden.trim_end(),
        "rendered `{label}` output diverged from its golden snapshot"
    );
}

#[test]
fn dup_workspace_report_matches_golden_in_all_formats() {
    let report = report_for("dup-workspace");
    assert_matches_golden(
        &report.render_table(),
        include_str!("golden/dup-workspace.txt"),
        "dup-workspace/table",
    );
    assert_matches_golden(
        &report.render_markdown(),
        include_str!("golden/dup-workspace.md"),
        "dup-workspace/markdown",
    );
    let json = serde_json::to_string_pretty(&report.view()).expect("serialize json");
    assert_matches_golden(
        &json,
        include_str!("golden/dup-workspace.json"),
        "dup-workspace/json",
    );
}

#[test]
fn analyzer_workspace_report_matches_golden_in_all_formats() {
    let report = report_for("analyzer-workspace");
    assert_matches_golden(
        &report.render_table(),
        include_str!("golden/analyzer-workspace.txt"),
        "analyzer-workspace/table",
    );
    assert_matches_golden(
        &report.render_markdown(),
        include_str!("golden/analyzer-workspace.md"),
        "analyzer-workspace/markdown",
    );
    let json = serde_json::to_string_pretty(&report.view()).expect("serialize json");
    assert_matches_golden(
        &json,
        include_str!("golden/analyzer-workspace.json"),
        "analyzer-workspace/json",
    );
}

#[test]
fn every_finding_carries_a_prescription() {
    // Machine check across both fixtures: no finding ships without steps.
    for fixture in ["dup-workspace", "analyzer-workspace"] {
        let report = report_for(fixture);
        assert!(
            !report.findings().is_empty(),
            "fixture `{fixture}` should produce findings"
        );
        for f in report.findings() {
            assert!(
                !f.prescription.steps.is_empty(),
                "finding `{}` in `{fixture}` has no prescription steps",
                f.kind.as_str()
            );
        }
    }
}

#[test]
fn no_rendered_output_contains_a_quantitative_prediction() {
    // ADR-0002: no fabricated durations may ship in any format.
    for fixture in ["dup-workspace", "analyzer-workspace"] {
        let report = report_for(fixture);
        let json = serde_json::to_string_pretty(&report.view()).expect("serialize json");
        for (label, rendered) in [
            ("table", report.render_table()),
            ("markdown", report.render_markdown()),
            ("json", json),
        ] {
            assert_eq!(
                find_quantitative_claim(&rendered),
                None,
                "`{fixture}` {label} output contains a duration prediction:\n{rendered}"
            );
        }
    }
}
