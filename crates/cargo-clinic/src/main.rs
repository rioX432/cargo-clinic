//! `cargo clinic` CLI entry point.
//!
//! This binary has two roles, chosen at startup:
//!
//! 1. **CLI** (`cargo clinic measure` / `cargo clinic import`): the normal
//!    subcommand surface.
//! 2. **`RUSTC_WRAPPER` shim**: when `cargo clinic measure` runs a build, it
//!    points `RUSTC_WRAPPER` back at this same executable. Cargo then invokes
//!    us as `cargo-clinic <rustc> <args...>` for every rustc call; in that
//!    role we time the real rustc and append a record to the JSONL log.
//!
//! The two roles are distinguished by [`WRAPPER_LOG_ENV`]: `measure` sets it
//! only on the child build process, so its presence (plus a first argument that
//! is not the `clinic` subcommand marker) means "we are the shim".

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use cargo_clinic_core::{
    append_invocation, crate_name_from_args, import_nightly_timings, read_invocations,
    RustcInvocation, TimingReport,
};

/// Environment variable that both marks shim mode and carries the JSONL log
/// path. Set by `measure` on the child build only.
const WRAPPER_LOG_ENV: &str = "CARGO_CLINIC_TIMING_LOG";

/// Cargo's own wrapper env var — used for conflict detection.
const RUSTC_WRAPPER_ENV: &str = "RUSTC_WRAPPER";

/// Cargo's workspace-scoped wrapper env var — also checked for conflicts.
const RUSTC_WORKSPACE_WRAPPER_ENV: &str = "RUSTC_WORKSPACE_WRAPPER";

/// The cargo subcommand marker: `cargo clinic ...` invokes us with this as
/// `argv[1]`. Used to keep shim detection from misfiring on a real CLI run.
const SUBCOMMAND_MARKER: &str = "clinic";

/// Default timing log filename, written in the current directory.
const DEFAULT_LOG_FILENAME: &str = "cargo-clinic-timings.jsonl";

/// Exit code used when the child process was terminated without a code
/// (e.g. by a signal), so we still surface a failure.
const EXIT_TERMINATED: u8 = 1;

fn main() -> ExitCode {
    if is_shim_invocation() {
        return run_shim();
    }
    match run_cli() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Detect whether Cargo invoked us as the `RUSTC_WRAPPER` shim.
///
/// True only when the shim env var is set AND the first argument is not the
/// `clinic` subcommand marker. That guards against a user who exported the env
/// var manually and then ran `cargo clinic ...` directly.
fn is_shim_invocation() -> bool {
    if std::env::var_os(WRAPPER_LOG_ENV).is_none() {
        return false;
    }
    match std::env::args_os().nth(1) {
        Some(first) => first != SUBCOMMAND_MARKER,
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Shim mode
// ---------------------------------------------------------------------------

/// Run as the `RUSTC_WRAPPER` shim: exec the real rustc, timing it, and record
/// the invocation. The child's exit status is forwarded verbatim so the build
/// behaves exactly as it would without the shim.
fn run_shim() -> ExitCode {
    match shim_inner() {
        Ok(code) => code,
        Err(err) => {
            // A shim failure must not silently corrupt the build; report it.
            eprintln!("cargo-clinic (wrapper): {err:#}");
            ExitCode::from(EXIT_TERMINATED)
        }
    }
}

fn shim_inner() -> Result<ExitCode> {
    let log_path: PathBuf = std::env::var_os(WRAPPER_LOG_ENV)
        .map(PathBuf::from)
        .context("shim invoked without a timing log path")?;

    // argv layout: [self, <rustc>, <rustc-args...>].
    let mut args = std::env::args_os();
    let _self = args.next();
    let rustc = args
        .next()
        .context("RUSTC_WRAPPER shim invoked without a rustc path")?;
    let rustc_args: Vec<OsString> = args.collect();

    // Determine the crate name (probe invocations without one are not recorded).
    let arg_strings: Vec<String> = rustc_args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let crate_name = crate_name_from_args(&arg_strings);

    let start_wall = SystemTime::now();
    let start = Instant::now();
    let status = Command::new(&rustc)
        .args(&rustc_args)
        .status()
        .with_context(|| format!("failed to run rustc at `{}`", rustc.to_string_lossy()))?;
    let duration = start.elapsed();
    let end_wall = SystemTime::now();

    if let Some(name) = crate_name {
        let record = RustcInvocation {
            crate_name: name,
            start_unix_ms: unix_millis(start_wall),
            end_unix_ms: unix_millis(end_wall),
            duration_ms: duration_millis(duration),
        };
        // Best-effort: a logging failure must not break the build, but it is
        // reported rather than silently swallowed.
        if let Err(err) = append_invocation(&log_path, &record) {
            eprintln!("cargo-clinic (wrapper): failed to record timing: {err:#}");
        }
    }

    Ok(exit_code_from_status(status))
}

/// Milliseconds since the Unix epoch, or 0 if the clock predates the epoch.
fn unix_millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// A [`Duration`] in whole milliseconds, saturating on overflow.
fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Map a child [`std::process::ExitStatus`] to an [`ExitCode`].
fn exit_code_from_status(status: std::process::ExitStatus) -> ExitCode {
    match status.code() {
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(EXIT_TERMINATED)),
        None => ExitCode::from(EXIT_TERMINATED),
    }
}

// ---------------------------------------------------------------------------
// CLI mode
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "cargo-clinic",
    bin_name = "cargo clinic",
    about = "Diagnose slow Rust builds and get ranked, actionable prescriptions"
)]
struct Cli {
    #[command(subcommand)]
    command: ClinicCommand,
}

#[derive(Subcommand)]
enum ClinicCommand {
    /// Measure per-crate rustc time by building under a RUSTC_WRAPPER shim.
    Measure(MeasureArgs),
    /// Import timing data from an external source (experimental).
    Import(ImportArgs),
}

#[derive(Parser)]
struct MeasureArgs {
    /// Path to the Cargo.toml to build (defaults to the current directory).
    #[arg(long, value_name = "PATH")]
    manifest_path: Option<PathBuf>,

    /// Where to write the JSONL timing log (defaults to the current directory).
    #[arg(long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Emit the aggregated report as JSON instead of a text table.
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct ImportArgs {
    /// EXPERIMENTAL: import cargo's UNSTABLE nightly `--timings=json` output.
    ///
    /// The `--timings=json` format is officially unstable and may change or
    /// break across toolchains; this import is best-effort and experimental.
    #[arg(long = "timings-json", value_name = "FILE")]
    timings_json: PathBuf,

    /// Emit the aggregated report as JSON instead of a text table.
    #[arg(long)]
    json: bool,
}

fn run_cli() -> Result<()> {
    let cli = Cli::parse_from(cli_args());
    match cli.command {
        ClinicCommand::Measure(args) => run_measure(args),
        ClinicCommand::Import(args) => run_import(args),
    }
}

/// Normalize argv so clap sees a clean argument list whether we were called as
/// `cargo clinic ...` (argv[1] == "clinic") or directly as `cargo-clinic ...`.
fn cli_args() -> Vec<OsString> {
    let mut args: Vec<OsString> = std::env::args_os().collect();
    if args.get(1).map(|a| a == SUBCOMMAND_MARKER).unwrap_or(false) {
        args.remove(1);
    }
    args
}

fn run_measure(args: MeasureArgs) -> Result<()> {
    detect_wrapper_conflict()?;

    let self_exe =
        std::env::current_exe().context("failed to locate the cargo-clinic executable")?;
    let log_path = match args.output {
        Some(path) => path,
        None => std::env::current_dir()
            .context("failed to determine the current directory")?
            .join(DEFAULT_LOG_FILENAME),
    };

    // Start each run from a clean log so stale records cannot leak in.
    if log_path.exists() {
        std::fs::remove_file(&log_path)
            .with_context(|| format!("failed to reset timing log `{}`", log_path.display()))?;
    }

    let mut command = Command::new("cargo");
    command.arg("build");
    if let Some(manifest) = &args.manifest_path {
        command.arg("--manifest-path").arg(manifest);
    }
    command
        .env(RUSTC_WRAPPER_ENV, &self_exe)
        .env(WRAPPER_LOG_ENV, &log_path);

    let status = command
        .status()
        .context("failed to run `cargo build` for measurement")?;
    if !status.success() {
        bail!("`cargo build` failed; no timing report produced");
    }

    let invocations = read_invocations(&log_path)?;
    let report = TimingReport::from_wrapper_log(&invocations);
    print_report(&report, args.json)
}

fn run_import(args: ImportArgs) -> Result<()> {
    eprintln!(
        "warning: `import --timings-json` is EXPERIMENTAL; cargo's `--timings=json` \
         format is unstable and may change across toolchains."
    );
    let units = import_nightly_timings(&args.timings_json)?;
    if units.is_empty() {
        eprintln!(
            "warning: no `timing-info` records found in `{}`",
            args.timings_json.display()
        );
    }
    let report = TimingReport::from_nightly_import(&units);
    print_report(&report, args.json)
}

/// Print a report as JSON or as the text table. The structural disclaimer is
/// carried by the report itself, so both formats include it.
fn print_report(report: &TimingReport, as_json: bool) -> Result<()> {
    if as_json {
        let json = serde_json::to_string_pretty(&report.view())
            .context("failed to serialize timing report as JSON")?;
        println!("{json}");
    } else {
        println!("{report}");
    }
    Ok(())
}

/// Detect an existing `RUSTC_WRAPPER` / `RUSTC_WORKSPACE_WRAPPER` (e.g. sccache)
/// and refuse to silently clobber it.
fn detect_wrapper_conflict() -> Result<()> {
    for key in [RUSTC_WRAPPER_ENV, RUSTC_WORKSPACE_WRAPPER_ENV] {
        if let Some(value) = std::env::var_os(key) {
            if !value.is_empty() {
                bail!(
                    "{key} is already set to `{}` (e.g. sccache). `cargo clinic measure` would \
                     override it. Unset {key} and retry.",
                    Path::new(&value).display()
                );
            }
        }
    }
    Ok(())
}
