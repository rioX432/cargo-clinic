//! End-to-end test for `cargo clinic measure`: run a real build of the small
//! `timing-workspace` fixture under the RUSTC_WRAPPER shim and assert we get
//! per-crate timings plus the structural disclaimer.
//!
//! It also asserts that an existing `RUSTC_WRAPPER` (e.g. sccache) is detected
//! and refused rather than silently clobbered.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Path to the built `cargo-clinic` binary under test.
const BIN: &str = env!("CARGO_BIN_EXE_cargo-clinic");

/// Manifest of the timing fixture workspace.
fn fixture_manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/timing-workspace/Cargo.toml")
}

/// A unique, cold working directory under this test binary's temp dir. A fresh
/// target dir per run guarantees Cargo actually invokes rustc (a warm cache
/// would produce zero rustc calls and an empty log).
fn cold_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{tag}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("create cold temp dir");
    dir
}

#[test]
fn measure_records_per_crate_timings_with_disclaimer() {
    // Arrange: a cold target dir and log path so the build is not a cache hit.
    let work = cold_dir("measure");
    let target_dir = work.join("target");
    let log_path = work.join("timings.jsonl");

    // Act: run `cargo clinic measure --json` against the fixture. Clear any
    // ambient wrapper (sccache) so the conflict guard does not trip in CI.
    let output = Command::new(BIN)
        .arg("measure")
        .arg("--manifest-path")
        .arg(fixture_manifest())
        .arg("--output")
        .arg(&log_path)
        .arg("--json")
        .env("CARGO_TARGET_DIR", &target_dir)
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .output()
        .expect("run cargo clinic measure");

    // Assert: the command succeeded.
    assert!(
        output.status.success(),
        "measure failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // Assert: the JSONL log holds at least the two fixture crates.
    let log = std::fs::read_to_string(&log_path).expect("read timing log");
    assert!(
        log.lines().count() >= 2,
        "expected >=2 rustc records, got:\n{log}"
    );

    // Assert: the JSON report mentions both crates and carries the disclaimer.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value =
        serde_json::from_str(&stdout).expect("parse measure --json output");

    let crate_names: Vec<&str> = report["crates"]
        .as_array()
        .expect("crates array")
        .iter()
        .filter_map(|c| c["crate_name"].as_str())
        .collect();
    assert!(
        crate_names.contains(&"leaf") && crate_names.contains(&"app"),
        "report should include both fixture crates; got {crate_names:?}"
    );

    let disclaimer = report["disclaimer"].as_str().expect("disclaimer string");
    assert!(
        disclaimer.contains("build.rs") && disclaimer.contains("RUSTC_WRAPPER"),
        "disclaimer must disclose the build.rs / RUSTC_WRAPPER limitation; got: {disclaimer}"
    );

    // Best-effort cleanup of the cold build artifacts.
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn measure_refuses_to_clobber_existing_rustc_wrapper() {
    // Arrange & Act: run measure with RUSTC_WRAPPER already set (as sccache
    // users have). The command must refuse rather than override it.
    let work = cold_dir("conflict");
    let output = Command::new(BIN)
        .arg("measure")
        .arg("--manifest-path")
        .arg(fixture_manifest())
        .env("CARGO_TARGET_DIR", work.join("target"))
        .env("RUSTC_WRAPPER", "sccache")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .output()
        .expect("run cargo clinic measure with a conflicting wrapper");

    // Assert: it failed and explained the conflict.
    assert!(
        !output.status.success(),
        "measure must fail when RUSTC_WRAPPER is already set"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("RUSTC_WRAPPER") && stderr.contains("sccache"),
        "error should name the conflicting wrapper; got: {stderr}"
    );

    let _ = std::fs::remove_dir_all(&work);
}
