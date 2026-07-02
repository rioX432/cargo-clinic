//! Timing collector: model, JSONL persistence, aggregation, and the
//! (experimental) nightly `--timings=json` import.
//!
//! The stable measurement path is a `RUSTC_WRAPPER` shim (implemented in the
//! `cargo-clinic` binary) that records one [`RustcInvocation`] per rustc call
//! into a JSONL log. This module owns the data model, the append/read helpers,
//! and the pure aggregation into a [`TimingReport`].
//!
//! Honesty constraint (see CLAUDE.md): a `RUSTC_WRAPPER` shim only observes
//! rustc invocations. Build-script (`build.rs`) execution and Cargo's own
//! scheduling/queue time are invisible to it. That limitation is surfaced
//! structurally: every [`TimingReport`] carries a non-empty
//! [`disclaimer`](TimingReport::disclaimer), and both the [`Display`] and the
//! JSON [`view`](TimingReport::view) always include it.
//!
//! Determinism: aggregation is a pure function of its inputs — it reads no
//! wall clock and no RNG. Only the wrapper process (which by nature measures
//! real durations) touches the clock, and it lives in the binary crate.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Structural disclaimer for `RUSTC_WRAPPER`-sourced timing.
///
/// This MUST accompany every wrapper-sourced report: the shim cannot see
/// build-script execution or Cargo scheduling, so the numbers are a partial
/// view of total build cost.
pub const WRAPPER_DISCLAIMER: &str = "Timing measures rustc invocations only. build.rs (build-script) execution time and Cargo's scheduling/queue time are invisible to RUSTC_WRAPPER and are NOT included in these numbers.";

/// Structural disclaimer for the experimental nightly import path.
pub const NIGHTLY_IMPORT_DISCLAIMER: &str = "EXPERIMENTAL: imported from cargo's UNSTABLE `--timings=json` output. That format is officially unstable and can change or disappear across toolchains; treat these numbers as experimental.";

/// The rustc flag that carries the crate name being compiled.
const CRATE_NAME_FLAG: &str = "--crate-name";

/// The `--crate-name=` joined-form prefix.
const CRATE_NAME_EQ_PREFIX: &str = "--crate-name=";

/// Sentinel crate name Cargo passes to its compiler-support probe invocations
/// (e.g. `rustc --crate-name ___ --print=...`). These are not real crates, so
/// they are excluded from timing to avoid a meaningless `___` report entry.
const PROBE_CRATE_NAME: &str = "___";

/// Cargo emits `--timings=json` durations in seconds; scale to milliseconds.
const MILLIS_PER_SEC: f64 = 1000.0;

/// Cargo's `reason` tag for a per-unit timing line in `--timings=json`.
const TIMING_INFO_REASON: &str = "timing-info";

/// Errors from timing persistence, parsing, and import.
#[derive(Debug, Error)]
pub enum TimingError {
    /// The timing log or import file could not be opened, read, or written.
    #[error("failed to access timing file `{path}`: {source}")]
    Io {
        /// The path that was being accessed.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// A timing record could not be encoded to JSON.
    #[error("failed to encode timing record: {0}")]
    Encode(#[source] serde_json::Error),

    /// A line in a timing file could not be parsed as JSON.
    #[error("malformed timing record on line {line} of `{path}`: {source}")]
    Decode {
        /// The file being parsed.
        path: String,
        /// 1-based line number of the offending record.
        line: usize,
        /// The underlying JSON error.
        #[source]
        source: serde_json::Error,
    },
}

/// Build a [`TimingError::Io`] from a path and the underlying error.
fn io_err(path: &Path, source: std::io::Error) -> TimingError {
    TimingError::Io {
        path: path.display().to_string(),
        source,
    }
}

/// One rustc invocation, as recorded by the wrapper shim (one JSONL line).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustcInvocation {
    /// The `--crate-name` value passed to rustc.
    pub crate_name: String,
    /// Start time, in milliseconds since the Unix epoch.
    pub start_unix_ms: u64,
    /// End time, in milliseconds since the Unix epoch.
    pub end_unix_ms: u64,
    /// Wall-clock duration of the rustc invocation, in milliseconds.
    pub duration_ms: u64,
}

/// Extract the `--crate-name` value from a rustc argument list.
///
/// Handles both the separate (`--crate-name foo`) and joined
/// (`--crate-name=foo`) forms. Returns `None` for rustc probe invocations —
/// both those carrying no crate name (e.g. `--print` calls) and Cargo's
/// compiler-support probe (`--crate-name ___`) — so the log holds only real
/// compilations.
pub fn crate_name_from_args<I, S>(args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut iter = args.into_iter();
    let name = loop {
        let arg = iter.next()?;
        let arg = arg.as_ref();
        if arg == CRATE_NAME_FLAG {
            break iter.next().map(|v| v.as_ref().to_owned())?;
        }
        if let Some(value) = arg.strip_prefix(CRATE_NAME_EQ_PREFIX) {
            break value.to_owned();
        }
    };
    if name == PROBE_CRATE_NAME {
        return None;
    }
    Some(name)
}

/// Append one invocation record as a single JSON line to `path`.
///
/// The file is created if absent and opened in append mode. Writing the whole
/// line in one `write_all` keeps concurrent rustc processes (Cargo builds many
/// crates in parallel) from interleaving partial lines on POSIX append writes.
pub fn append_invocation(path: &Path, record: &RustcInvocation) -> Result<(), TimingError> {
    let mut line = serde_json::to_string(record).map_err(TimingError::Encode)?;
    line.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| io_err(path, e))?;
    file.write_all(line.as_bytes()).map_err(|e| io_err(path, e))
}

/// Read all invocation records from a JSONL timing log.
///
/// Blank lines are skipped. A malformed line aborts with the offending line
/// number rather than being silently dropped.
pub fn read_invocations(path: &Path) -> Result<Vec<RustcInvocation>, TimingError> {
    let file = File::open(path).map_err(|e| io_err(path, e))?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| io_err(path, e))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let record = serde_json::from_str(trimmed).map_err(|source| TimingError::Decode {
            path: path.display().to_string(),
            line: idx + 1,
            source,
        })?;
        out.push(record);
    }
    Ok(out)
}

/// A single unit parsed from cargo's unstable `--timings=json` stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NightlyTimingUnit {
    /// The compiled crate's name (from the unit `target.name`).
    pub crate_name: String,
    /// Compile mode string as emitted by cargo (e.g. `build`, `run-custom-build`).
    pub mode: String,
    /// Unit duration in milliseconds (converted from cargo's seconds).
    pub duration_ms: u64,
}

/// The `target` object nested in a `timing-info` line; only `name` is used.
#[derive(Debug, Deserialize, Default)]
struct RawTarget {
    #[serde(default)]
    name: String,
}

/// A defensive view of one `--timings=json` line. Unknown fields are ignored,
/// and every field is optional so non-`timing-info` lines parse without error.
#[derive(Debug, Deserialize, Default)]
struct RawTimingLine {
    #[serde(default)]
    reason: String,
    #[serde(default)]
    target: RawTarget,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    duration: f64,
}

/// Convert a non-negative, finite duration in seconds to whole milliseconds.
///
/// NaN/negative inputs clamp to zero so a corrupt line cannot poison the total.
fn seconds_to_millis(seconds: f64) -> u64 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (seconds * MILLIS_PER_SEC).round() as u64
}

/// Import cargo's UNSTABLE `--timings=json` output (one JSON object per line).
///
/// Only `reason == "timing-info"` lines are kept; any other message reasons
/// (present when the stream is mixed with `--message-format=json`) and blank
/// lines are ignored. The format is unofficial and version-dependent — this is
/// a deliberately best-effort, experimental importer (see CLAUDE.md "Won't
/// Do": the stable path is the wrapper, never this).
pub fn import_nightly_timings(path: &Path) -> Result<Vec<NightlyTimingUnit>, TimingError> {
    let file = File::open(path).map_err(|e| io_err(path, e))?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| io_err(path, e))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let raw: RawTimingLine =
            serde_json::from_str(trimmed).map_err(|source| TimingError::Decode {
                path: path.display().to_string(),
                line: idx + 1,
                source,
            })?;
        if raw.reason != TIMING_INFO_REASON || raw.target.name.is_empty() {
            continue;
        }
        out.push(NightlyTimingUnit {
            crate_name: raw.target.name,
            mode: raw.mode,
            duration_ms: seconds_to_millis(raw.duration),
        });
    }
    Ok(out)
}

/// Provenance of a [`TimingReport`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimingSource {
    /// The stable `RUSTC_WRAPPER` shim.
    RustcWrapper,
    /// The experimental nightly `--timings=json` import.
    NightlyTimingsJson,
}

impl TimingSource {
    /// Stable machine-readable identifier.
    pub fn as_str(&self) -> &'static str {
        match self {
            TimingSource::RustcWrapper => "rustc-wrapper",
            TimingSource::NightlyTimingsJson => "nightly-timings-json",
        }
    }

    /// The disclaimer that structurally accompanies this source.
    fn disclaimer(&self) -> &'static str {
        match self {
            TimingSource::RustcWrapper => WRAPPER_DISCLAIMER,
            TimingSource::NightlyTimingsJson => NIGHTLY_IMPORT_DISCLAIMER,
        }
    }
}

/// Aggregated timing for a single crate, summed across its invocations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrateTiming {
    /// Crate name.
    pub crate_name: String,
    /// Total rustc time attributed to this crate, in milliseconds.
    pub total_ms: u64,
    /// Number of rustc invocations aggregated.
    pub invocations: u32,
}

/// A report-ready timing summary.
///
/// The [`disclaimer`](Self::disclaimer) is a structural, non-optional part of
/// the report: it is emitted by both [`Display`] and [`view`](Self::view), so
/// no rendering path can drop it.
#[derive(Debug, Clone, PartialEq)]
pub struct TimingReport {
    crates: Vec<CrateTiming>,
    total_ms: u64,
    source: TimingSource,
}

/// A `Serialize`-able snapshot of a [`TimingReport`] that always includes the
/// disclaimer, so `--json` output cannot omit the limitation notice.
#[derive(Debug, Serialize)]
pub struct TimingReportView<'a> {
    /// Machine-readable source identifier.
    pub source: &'a str,
    /// The structural limitation disclaimer.
    pub disclaimer: &'a str,
    /// Total attributed rustc time in milliseconds.
    pub total_ms: u64,
    /// Per-crate breakdown, ordered by descending time.
    pub crates: &'a [CrateTiming],
}

impl TimingReport {
    /// Aggregate a wrapper JSONL log into a report.
    pub fn from_wrapper_log(invocations: &[RustcInvocation]) -> Self {
        let mut totals: BTreeMap<&str, (u64, u32)> = BTreeMap::new();
        for inv in invocations {
            let entry = totals.entry(inv.crate_name.as_str()).or_default();
            entry.0 = entry.0.saturating_add(inv.duration_ms);
            entry.1 = entry.1.saturating_add(1);
        }
        Self::from_totals(totals, TimingSource::RustcWrapper)
    }

    /// Aggregate imported nightly units into a report.
    pub fn from_nightly_import(units: &[NightlyTimingUnit]) -> Self {
        let mut totals: BTreeMap<&str, (u64, u32)> = BTreeMap::new();
        for unit in units {
            let entry = totals.entry(unit.crate_name.as_str()).or_default();
            entry.0 = entry.0.saturating_add(unit.duration_ms);
            entry.1 = entry.1.saturating_add(1);
        }
        Self::from_totals(totals, TimingSource::NightlyTimingsJson)
    }

    /// Build a report from name -> (total_ms, count) aggregates. Ordered by
    /// descending total time, then crate name, for deterministic output.
    fn from_totals(totals: BTreeMap<&str, (u64, u32)>, source: TimingSource) -> Self {
        let mut crates: Vec<CrateTiming> = totals
            .into_iter()
            .map(|(name, (total_ms, invocations))| CrateTiming {
                crate_name: name.to_owned(),
                total_ms,
                invocations,
            })
            .collect();
        crates.sort_by(|a, b| {
            b.total_ms
                .cmp(&a.total_ms)
                .then_with(|| a.crate_name.cmp(&b.crate_name))
        });
        let total_ms = crates
            .iter()
            .fold(0u64, |acc, c| acc.saturating_add(c.total_ms));
        Self {
            crates,
            total_ms,
            source,
        }
    }

    /// Per-crate breakdown, ordered by descending time.
    pub fn crates(&self) -> &[CrateTiming] {
        &self.crates
    }

    /// Total attributed rustc time, in milliseconds.
    pub fn total_ms(&self) -> u64 {
        self.total_ms
    }

    /// Provenance of this report.
    pub fn source(&self) -> TimingSource {
        self.source
    }

    /// The structural limitation disclaimer for this report's source.
    ///
    /// Always non-empty — the caller cannot construct a report without one.
    pub fn disclaimer(&self) -> &'static str {
        self.source.disclaimer()
    }

    /// A JSON-serializable snapshot that always carries the disclaimer.
    pub fn view(&self) -> TimingReportView<'_> {
        TimingReportView {
            source: self.source.as_str(),
            disclaimer: self.disclaimer(),
            total_ms: self.total_ms,
            crates: &self.crates,
        }
    }
}

impl fmt::Display for TimingReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const NAME_WIDTH: usize = 40;
        const MS_WIDTH: usize = 8;
        writeln!(f, "Timing (source: {})", self.source.as_str())?;
        if self.crates.is_empty() {
            writeln!(f, "  (no rustc invocations recorded)")?;
        } else {
            for crate_timing in &self.crates {
                writeln!(
                    f,
                    "  {:<NAME_WIDTH$} {:>MS_WIDTH$} ms  ({} invocation(s))",
                    crate_timing.crate_name, crate_timing.total_ms, crate_timing.invocations
                )?;
            }
            writeln!(
                f,
                "  {:<NAME_WIDTH$} {:>MS_WIDTH$} ms  (total)",
                "", self.total_ms
            )?;
        }
        // Disclaimer is structural: always emitted, never behind a flag.
        write!(f, "NOTE: {}", self.disclaimer())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Unique temp path for a filesystem test (avoids cross-test collisions).
    fn temp_path(tag: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "cargo-clinic-{tag}-{}-{n}.jsonl",
            std::process::id()
        ))
    }

    #[test]
    fn crate_name_parsed_from_separate_form() {
        let args = ["--edition=2021", "--crate-name", "serde", "--crate-type"];
        assert_eq!(crate_name_from_args(args), Some("serde".to_owned()));
    }

    #[test]
    fn crate_name_parsed_from_joined_form() {
        let args = ["--crate-name=my_crate", "src/lib.rs"];
        assert_eq!(crate_name_from_args(args), Some("my_crate".to_owned()));
    }

    #[test]
    fn crate_name_absent_for_probe_invocation() {
        let args = ["--print", "cfg", "-"];
        assert_eq!(crate_name_from_args(args), None);
    }

    #[test]
    fn crate_name_absent_for_cargo_compiler_probe_sentinel() {
        // Cargo runs `rustc --crate-name ___ --print=...` to detect compiler
        // support; that sentinel must not appear as a crate in the report.
        let args = ["--crate-name", "___", "--print=file-names"];
        assert_eq!(crate_name_from_args(args), None);
    }

    #[test]
    fn append_then_read_roundtrips_records() {
        // Arrange
        let path = temp_path("roundtrip");
        let a = RustcInvocation {
            crate_name: "leaf".to_owned(),
            start_unix_ms: 10,
            end_unix_ms: 40,
            duration_ms: 30,
        };
        let b = RustcInvocation {
            crate_name: "app".to_owned(),
            start_unix_ms: 40,
            end_unix_ms: 100,
            duration_ms: 60,
        };

        // Act
        append_invocation(&path, &a).expect("append a");
        append_invocation(&path, &b).expect("append b");
        let read = read_invocations(&path).expect("read back");

        // Assert
        assert_eq!(read, vec![a, b]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_skips_blank_lines_and_flags_malformed() {
        let path = temp_path("malformed");
        std::fs::write(&path, "\n{ not json }\n").expect("write");
        let err = read_invocations(&path).expect_err("must reject malformed line");
        match err {
            TimingError::Decode { line, .. } => assert_eq!(line, 2, "line number reported"),
            other => panic!("expected Decode error, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn wrapper_report_aggregates_and_orders_by_descending_time() {
        // Arrange: leaf appears twice (codegen + metadata), app once.
        let invocations = vec![
            RustcInvocation {
                crate_name: "leaf".to_owned(),
                start_unix_ms: 0,
                end_unix_ms: 20,
                duration_ms: 20,
            },
            RustcInvocation {
                crate_name: "leaf".to_owned(),
                start_unix_ms: 20,
                end_unix_ms: 30,
                duration_ms: 10,
            },
            RustcInvocation {
                crate_name: "app".to_owned(),
                start_unix_ms: 30,
                end_unix_ms: 80,
                duration_ms: 50,
            },
        ];

        // Act
        let report = TimingReport::from_wrapper_log(&invocations);

        // Assert: app (50) before leaf (30); leaf's two invocations summed.
        let crates = report.crates();
        assert_eq!(crates[0].crate_name, "app");
        assert_eq!(crates[0].total_ms, 50);
        assert_eq!(crates[1].crate_name, "leaf");
        assert_eq!(crates[1].total_ms, 30);
        assert_eq!(crates[1].invocations, 2);
        assert_eq!(report.total_ms(), 80);
        assert_eq!(report.source(), TimingSource::RustcWrapper);
    }

    #[test]
    fn wrapper_report_always_carries_the_build_rs_disclaimer() {
        let report = TimingReport::from_wrapper_log(&[]);
        // Structural: present in the disclaimer, the Display, and the JSON view.
        assert!(report.disclaimer().contains("build.rs"));
        assert!(report.to_string().contains("build.rs"));
        assert_eq!(report.view().disclaimer, WRAPPER_DISCLAIMER);
    }

    #[test]
    fn nightly_import_keeps_only_timing_info_lines() {
        // Arrange: a mixed stream with one timing-info line and noise.
        let path = temp_path("nightly");
        let contents = concat!(
            "{\"reason\":\"compiler-artifact\",\"package_id\":\"app\"}\n",
            "{\"reason\":\"timing-info\",\"package_id\":\"path+file:///x#leaf@0.1.0\",",
            "\"target\":{\"name\":\"leaf\"},\"mode\":\"build\",\"duration\":0.25}\n",
            "\n",
        );
        std::fs::write(&path, contents).expect("write nightly stream");

        // Act
        let units = import_nightly_timings(&path).expect("import");

        // Assert: only the timing-info unit, seconds converted to ms.
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].crate_name, "leaf");
        assert_eq!(units[0].mode, "build");
        assert_eq!(units[0].duration_ms, 250);

        // The report from this source carries the experimental disclaimer.
        let report = TimingReport::from_nightly_import(&units);
        assert_eq!(report.source(), TimingSource::NightlyTimingsJson);
        assert_eq!(report.disclaimer(), NIGHTLY_IMPORT_DISCLAIMER);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn seconds_to_millis_clamps_invalid_input() {
        assert_eq!(seconds_to_millis(1.5), 1500);
        assert_eq!(seconds_to_millis(0.0), 0);
        assert_eq!(seconds_to_millis(-3.0), 0);
        assert_eq!(seconds_to_millis(f64::NAN), 0);
        assert_eq!(seconds_to_millis(f64::INFINITY), 0);
    }
}
