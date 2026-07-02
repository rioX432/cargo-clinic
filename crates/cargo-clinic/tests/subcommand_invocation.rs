//! Integration tests for the cargo subcommand convention (issue #6).
//!
//! When installed as `cargo-clinic`, running `cargo clinic <args>` makes Cargo
//! exec the binary with `clinic` as `argv[1]`. The CLI must behave identically
//! whether invoked directly (`cargo-clinic report`) or via the subcommand shim
//! (`cargo-clinic clinic report`, which is what `cargo clinic report` becomes).
//!
//! We exercise the binary directly with an injected `clinic` marker rather than
//! shelling out through a real `cargo`, so the test is hermetic and does not
//! depend on the binary being on `PATH` under the `cargo-clinic` name.

use std::path::PathBuf;
use std::process::Command;

/// Path to the built `cargo-clinic` binary under test.
const BIN: &str = env!("CARGO_BIN_EXE_cargo-clinic");

/// Manifest of the duplicate-version fixture workspace (a stable, planted input).
fn dup_fixture_manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/dup-workspace/Cargo.toml")
}

/// Run the report subcommand and capture stdout, asserting success.
fn run_report(args: &[&str]) -> String {
    let output = Command::new(BIN)
        .args(args)
        .arg("--manifest-path")
        .arg(dup_fixture_manifest())
        // A stray wrapper env var must not flip us into shim mode for a real
        // CLI run; clear it so the test is independent of the ambient env.
        .env_remove("CARGO_CLINIC_TIMING_LOG")
        .output()
        .expect("run cargo-clinic report");
    assert!(
        output.status.success(),
        "report failed for args {args:?}: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn direct_and_subcommand_invocations_produce_identical_output() {
    // Direct: `cargo-clinic report ...`
    let direct = run_report(&["report", "--format", "json"]);
    // Subcommand: `cargo clinic report ...` reaches the binary as
    // `cargo-clinic clinic report ...` (Cargo inserts the subcommand name).
    let via_cargo = run_report(&["clinic", "report", "--format", "json"]);

    assert_eq!(
        direct, via_cargo,
        "output must be identical whether invoked directly or via `cargo clinic`"
    );
    assert!(
        !direct.trim().is_empty(),
        "report should produce output for the fixture"
    );
}

#[test]
fn subcommand_marker_is_stripped_before_clap_parsing() {
    // If the `clinic` marker were NOT stripped, clap would see `clinic` as the
    // subcommand and error out (there is no `clinic` subcommand). Success here
    // proves the marker is consumed and `report` is parsed as the command.
    let out = run_report(&["clinic", "report"]);
    assert!(
        !out.trim().is_empty(),
        "`cargo clinic report` must parse and run the report command"
    );
}

#[test]
fn subcommand_help_reports_cargo_clinic_bin_name() {
    // `cargo clinic --help` should present usage as `cargo clinic`, matching the
    // subcommand convention rather than the raw binary name.
    let output = Command::new(BIN)
        .args(["clinic", "--help"])
        .env_remove("CARGO_CLINIC_TIMING_LOG")
        .output()
        .expect("run cargo clinic --help");
    assert!(output.status.success(), "--help should exit successfully");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("cargo clinic"),
        "help usage should read `cargo clinic`, got:\n{stdout}"
    );
}
