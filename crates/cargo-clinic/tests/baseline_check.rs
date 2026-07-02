//! End-to-end tests for `cargo clinic report --save-baseline` and
//! `cargo clinic check`: the CI regression gate (issue #5).
//!
//! These exercise the real binary against the planted `dup-workspace` fixture,
//! which resolves one crate to two versions. They cover the acceptance
//! criteria: a baseline round-trips through the CLI without a false regression,
//! and an injected regression makes `check` exit nonzero. The noise-aware
//! timing / environment-change behavior is covered by the pure unit tests in
//! `clinic-core::baseline`.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Path to the built `cargo-clinic` binary under test.
const BIN: &str = env!("CARGO_BIN_EXE_cargo-clinic");

/// Manifest of the duplicate-version fixture workspace.
fn dup_fixture_manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/dup-workspace/Cargo.toml")
}

/// A unique temp file path under this test binary's temp dir.
fn temp_file(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{tag}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join("baseline.json")
}

#[test]
fn save_baseline_then_check_reports_no_regression() {
    // Arrange: save a baseline from the fixture.
    let baseline = temp_file("roundtrip");
    let save = Command::new(BIN)
        .args(["report", "--manifest-path"])
        .arg(dup_fixture_manifest())
        .arg("--save-baseline")
        .arg(&baseline)
        .output()
        .expect("run report --save-baseline");
    assert!(
        save.status.success(),
        "save failed: stderr={}",
        String::from_utf8_lossy(&save.stderr)
    );
    assert!(baseline.exists(), "baseline file should have been written");

    // Act: check the same fixture against that baseline.
    let check = Command::new(BIN)
        .args(["check", "--manifest-path"])
        .arg(dup_fixture_manifest())
        .arg("--baseline")
        .arg(&baseline)
        .output()
        .expect("run check");

    // Assert: no regression, exit 0.
    assert!(
        check.status.success(),
        "check should pass against its own baseline; stdout={} stderr={}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    let stdout = String::from_utf8_lossy(&check.stdout);
    assert!(
        stdout.contains("OK"),
        "expected an OK summary; got: {stdout}"
    );
}

#[test]
fn check_fails_on_injected_regression() {
    // Arrange: a baseline that claims a clean tree (no duplicates, no heavy
    // deps). The fixture actually has a duplicate, so every current duplicate
    // is "new since baseline" — a regression.
    let baseline = temp_file("regression");
    std::fs::write(
        &baseline,
        r#"{"schema_version":1,"duplicates":[],"heavy_deps":[]}"#,
    )
    .expect("write injected baseline");

    // Act
    let check = Command::new(BIN)
        .args(["check", "--manifest-path"])
        .arg(dup_fixture_manifest())
        .arg("--baseline")
        .arg(&baseline)
        .output()
        .expect("run check");

    // Assert: nonzero exit and a REGRESSION summary.
    assert!(
        !check.status.success(),
        "check must fail when the fixture regressed against the baseline"
    );
    let stdout = String::from_utf8_lossy(&check.stdout);
    assert!(
        stdout.contains("REGRESSION"),
        "expected a REGRESSION summary; got: {stdout}"
    );
}

#[test]
fn check_json_sets_regression_flag() {
    let baseline = temp_file("json-regression");
    std::fs::write(
        &baseline,
        r#"{"schema_version":1,"duplicates":[],"heavy_deps":[]}"#,
    )
    .expect("write injected baseline");

    let check = Command::new(BIN)
        .args(["check", "--json", "--manifest-path"])
        .arg(dup_fixture_manifest())
        .arg("--baseline")
        .arg(&baseline)
        .output()
        .expect("run check --json");

    let stdout = String::from_utf8_lossy(&check.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("check --json must emit valid JSON");
    assert_eq!(
        value["regression"],
        serde_json::Value::Bool(true),
        "regression flag should be true; got: {stdout}"
    );
    // Timing is skipped when no timing data is present (never a false alarm).
    assert_eq!(value["timing"]["status"], "skipped");
}

#[test]
fn example_github_action_is_valid_yaml() {
    // The example workflow must be valid YAML that references the real command.
    // Validation is best-effort via python3+pyyaml (present on GitHub runners);
    // if that toolchain is unavailable locally, we still assert the command
    // reference rather than failing spuriously.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/github-action.yml");
    let contents = std::fs::read_to_string(&path).expect("read example workflow");
    assert!(
        contents.contains("cargo clinic check"),
        "example workflow must run `cargo clinic check`"
    );

    let script = format!(
        "import sys, yaml; yaml.safe_load(open({:?}))",
        path.to_string_lossy()
    );
    match Command::new("python3").arg("-c").arg(&script).output() {
        Ok(out) if out.status.success() => { /* valid YAML */ }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // Only fail on a genuine parse error; a missing pyyaml module skips.
            if stderr.contains("yaml.") && !stderr.contains("ModuleNotFoundError") {
                panic!("example workflow is not valid YAML:\n{stderr}");
            }
            eprintln!("skipping YAML parse (python/pyyaml unavailable): {stderr}");
        }
        Err(e) => eprintln!("skipping YAML parse (python3 not runnable): {e}"),
    }
}
