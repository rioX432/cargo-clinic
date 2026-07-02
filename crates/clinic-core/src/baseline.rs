//! Baseline snapshots and the CI regression check (CLAUDE.md Core Value 3:
//! CI-first). A [`Baseline`] captures the parts of a workspace's build health
//! that a regression gate cares about — duplicate versions, heavy dependencies
//! (proc-macros / build scripts), and optional timing — plus an environment
//! fingerprint. `cargo clinic check` compares a fresh snapshot against a stored
//! baseline and fails CI on a regression.
//!
//! Determinism (CLAUDE.md): the comparison [`Baseline::check`] is a pure
//! function of its two snapshots and the threshold. It reads no wall clock, no
//! RNG, and never shells out. The environment fingerprint and timing numbers
//! are captured by the binary (which by nature touches the toolchain and the
//! clock) and injected here as plain data, so the gate logic stays testable and
//! reproducible.
//!
//! Honesty (CLAUDE.md Core Value 2 / ADR-0002): timing comparison is
//! noise-aware. It only fires when both snapshots carry timing measured on the
//! *same* environment, and only when a crate (or the total) grows by MORE than
//! a conservative threshold. A changed toolchain or host auto-disables the
//! timing comparison with an explanatory skip message rather than raising a
//! false alarm.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::DependencyGraph;
use crate::timing::TimingReport;

/// Baseline file schema version. Bumped when the on-disk format changes in a
/// way that older/newer builds cannot read; [`Baseline::load`] rejects a
/// mismatch instead of silently misinterpreting fields.
pub const BASELINE_SCHEMA_VERSION: u32 = 1;

/// Default conservative timing-regression threshold, as a percentage over the
/// baseline. Build-time measurement is noisy (ADR-0001/0002: machine load,
/// caches, incremental state), so a crate (or the total) must grow by MORE than
/// this percentage to count as a regression. Configurable via the CLI.
pub const DEFAULT_TIMING_THRESHOLD_PERCENT: u32 = 20;

/// Scope label used for the whole-build timing comparison entry.
pub const TOTAL_SCOPE: &str = "<total>";

/// Percentage denominator for integer threshold math.
const PERCENT_BASE: u128 = 100;

/// Skip reason used when either snapshot lacks timing data.
const SKIP_NO_TIMING: &str = "no timing data in the baseline and/or this run; \
     run `cargo clinic measure` and pass its log via `--timings` on both \
     `report --save-baseline` and `check` to enable timing comparison";

/// Skip reason used when an environment fingerprint is missing on either side.
const SKIP_NO_ENVIRONMENT: &str = "no environment fingerprint recorded; timing \
     comparison needs a matching rustc/host on both sides and is skipped";

/// Whether a heavy dependency is a proc-macro or a build-script crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HeavyDepKind {
    /// A proc-macro crate (compiled and executed at build time).
    ProcMacro,
    /// A crate that owns a `build.rs` build script.
    BuildScript,
}

impl HeavyDepKind {
    /// Stable machine-readable identifier.
    pub fn as_str(&self) -> &'static str {
        match self {
            HeavyDepKind::ProcMacro => "proc-macro",
            HeavyDepKind::BuildScript => "build-script",
        }
    }

    /// Deterministic ordering key.
    fn rank(&self) -> u8 {
        match self {
            HeavyDepKind::ProcMacro => 0,
            HeavyDepKind::BuildScript => 1,
        }
    }
}

/// A crate name resolved to multiple versions, captured for regression checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateSnapshot {
    /// The duplicated crate name.
    pub name: String,
    /// Its resolved versions, sorted and distinct.
    pub versions: Vec<String>,
}

/// A heavy dependency (proc-macro or build-script crate) captured in a baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeavyDepSnapshot {
    /// Which kind of heavy dependency this is.
    pub kind: HeavyDepKind,
    /// Crate name.
    pub name: String,
    /// Resolved version.
    pub version: String,
}

/// A toolchain/host fingerprint used to decide whether a timing comparison is
/// meaningful. If it changed, recompilation cost is not comparable, so timing
/// comparison is skipped rather than alarmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentFingerprint {
    /// The rustc release string (e.g. `1.92.0`).
    pub rustc_version: String,
    /// The host target triple (e.g. `x86_64-unknown-linux-gnu`).
    pub host: String,
}

/// Total rustc time attributed to one crate, for timing comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrateTimingSnapshot {
    /// Crate name.
    pub crate_name: String,
    /// Total attributed rustc time, in milliseconds.
    pub total_ms: u64,
}

/// Optional timing portion of a baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingSnapshot {
    /// Total attributed rustc time across all crates, in milliseconds.
    pub total_ms: u64,
    /// Per-crate breakdown.
    pub crates: Vec<CrateTimingSnapshot>,
}

impl TimingSnapshot {
    /// Capture the timing portion from an aggregated [`TimingReport`].
    pub fn from_report(report: &TimingReport) -> Self {
        let crates = report
            .crates()
            .iter()
            .map(|c| CrateTimingSnapshot {
                crate_name: c.crate_name.clone(),
                total_ms: c.total_ms,
            })
            .collect();
        TimingSnapshot {
            total_ms: report.total_ms(),
            crates,
        }
    }
}

/// A stored snapshot of a workspace's build health, used as the CI baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    /// On-disk schema version (see [`BASELINE_SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// Duplicated crate names and their versions, ordered by name.
    pub duplicates: Vec<DuplicateSnapshot>,
    /// Heavy dependencies, ordered by (kind, name, version).
    pub heavy_deps: Vec<HeavyDepSnapshot>,
    /// Environment fingerprint (present when timing is recorded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<EnvironmentFingerprint>,
    /// Optional timing snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<TimingSnapshot>,
}

/// Errors from baseline persistence and loading.
#[derive(Debug, Error)]
pub enum BaselineError {
    /// The baseline file could not be read or written.
    #[error("failed to access baseline file `{path}`: {source}")]
    Io {
        /// The path being accessed.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The baseline could not be encoded to JSON.
    #[error("failed to encode baseline: {0}")]
    Encode(#[source] serde_json::Error),

    /// The baseline file could not be parsed as JSON.
    #[error("failed to parse baseline `{path}`: {source}")]
    Decode {
        /// The path being parsed.
        path: String,
        /// The underlying JSON error.
        #[source]
        source: serde_json::Error,
    },

    /// The baseline file uses an unsupported schema version.
    #[error("unsupported baseline schema version {found} (this build supports {supported}); re-run `cargo clinic report --save-baseline` to regenerate it")]
    UnsupportedSchema {
        /// The version found in the file.
        found: u32,
        /// The version this build supports.
        supported: u32,
    },
}

fn io_err(path: &Path, source: std::io::Error) -> BaselineError {
    BaselineError::Io {
        path: path.display().to_string(),
        source,
    }
}

impl Baseline {
    /// Capture the structural portion (duplicates + heavy deps) from a resolved
    /// graph. Timing and environment are attached separately via
    /// [`Baseline::with_timing`] / [`Baseline::with_environment`].
    ///
    /// Pure and deterministic: same graph in, same baseline out.
    pub fn from_graph(graph: &DependencyGraph) -> Self {
        let duplicates = graph
            .duplicate_versions()
            .into_iter()
            .map(|dup| {
                let mut versions: Vec<String> =
                    dup.instances.iter().map(|i| i.version.clone()).collect();
                versions.sort();
                versions.dedup();
                DuplicateSnapshot {
                    name: dup.name,
                    versions,
                }
            })
            .collect();

        let mut heavy_deps: Vec<HeavyDepSnapshot> = Vec::new();
        for c in graph.proc_macro_crates() {
            heavy_deps.push(HeavyDepSnapshot {
                kind: HeavyDepKind::ProcMacro,
                name: c.package.name,
                version: c.package.version,
            });
        }
        for c in graph.build_script_crates() {
            heavy_deps.push(HeavyDepSnapshot {
                kind: HeavyDepKind::BuildScript,
                name: c.package.name,
                version: c.package.version,
            });
        }
        heavy_deps.sort_by(|a, b| {
            a.kind
                .rank()
                .cmp(&b.kind.rank())
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.version.cmp(&b.version))
        });

        Baseline {
            schema_version: BASELINE_SCHEMA_VERSION,
            duplicates,
            heavy_deps,
            environment: None,
            timing: None,
        }
    }

    /// Attach an environment fingerprint (needed for timing comparison).
    pub fn with_environment(mut self, environment: EnvironmentFingerprint) -> Self {
        self.environment = Some(environment);
        self
    }

    /// Attach a timing snapshot.
    pub fn with_timing(mut self, timing: TimingSnapshot) -> Self {
        self.timing = Some(timing);
        self
    }

    /// Serialize the baseline to `path` as pretty JSON, creating parent
    /// directories as needed.
    pub fn save(&self, path: &Path) -> Result<(), BaselineError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| io_err(path, e))?;
            }
        }
        let json = serde_json::to_string_pretty(self).map_err(BaselineError::Encode)?;
        std::fs::write(path, json).map_err(|e| io_err(path, e))
    }

    /// Load a baseline from `path`, rejecting an unsupported schema version.
    pub fn load(path: &Path) -> Result<Self, BaselineError> {
        let text = std::fs::read_to_string(path).map_err(|e| io_err(path, e))?;
        let baseline: Baseline =
            serde_json::from_str(&text).map_err(|source| BaselineError::Decode {
                path: path.display().to_string(),
                source,
            })?;
        if baseline.schema_version != BASELINE_SCHEMA_VERSION {
            return Err(BaselineError::UnsupportedSchema {
                found: baseline.schema_version,
                supported: BASELINE_SCHEMA_VERSION,
            });
        }
        Ok(baseline)
    }

    /// Compare a fresh snapshot (`current`) against this baseline and produce a
    /// [`CheckReport`]. Pure: no clock, no RNG, no I/O.
    ///
    /// A regression is any of: a new duplicate version, a new heavy dependency,
    /// or a crate/total whose timing grew by more than `threshold_percent`.
    /// Timing is compared only when both sides carry timing recorded on the same
    /// environment; otherwise it is skipped with an explanation.
    pub fn check(&self, current: &Baseline, threshold_percent: u32) -> CheckReport {
        CheckReport {
            new_duplicates: compare_duplicates(&self.duplicates, &current.duplicates),
            new_heavy_deps: compare_heavy_deps(&self.heavy_deps, &current.heavy_deps),
            timing: compare_timing(self, current, threshold_percent),
        }
    }
}

/// A duplicate-version regression: a crate that gained version(s) since the
/// baseline (or became duplicated at all).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateRegression {
    /// The crate name.
    pub name: String,
    /// Versions present now that were not in the baseline.
    pub new_versions: Vec<String>,
    /// All versions currently resolved for this crate.
    pub current_versions: Vec<String>,
}

/// A timing regression for one scope (a crate name or [`TOTAL_SCOPE`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimingRegression {
    /// Crate name, or [`TOTAL_SCOPE`] for the whole build.
    pub scope: String,
    /// Baseline time in milliseconds.
    pub baseline_ms: u64,
    /// Current time in milliseconds.
    pub current_ms: u64,
    /// Integer percentage increase over the baseline (floored).
    pub increase_percent: u64,
}

/// The result of a timing comparison: either compared (possibly with
/// regressions) or intentionally skipped with a reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum TimingComparison {
    /// Timing was compared. `regressions` may be empty (no regression).
    Compared {
        /// The threshold used, as a percentage.
        threshold_percent: u32,
        /// Crates/total that exceeded the threshold.
        regressions: Vec<TimingRegression>,
    },
    /// Timing comparison was skipped, with a human-readable reason (missing data
    /// or a changed environment).
    Skipped {
        /// Why the comparison was skipped.
        reason: String,
    },
}

/// The outcome of `cargo clinic check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckReport {
    /// New duplicate versions since the baseline.
    pub new_duplicates: Vec<DuplicateRegression>,
    /// New heavy dependencies since the baseline.
    pub new_heavy_deps: Vec<HeavyDepSnapshot>,
    /// Timing comparison result.
    pub timing: TimingComparison,
}

/// A `Serialize`-able view of a [`CheckReport`] that leads with the overall
/// `regression` boolean, so a CI consumer can branch on one field.
#[derive(Debug, Serialize)]
pub struct CheckReportView<'a> {
    /// Whether any regression was detected (drives the exit code).
    pub regression: bool,
    /// New duplicate versions since the baseline.
    pub new_duplicates: &'a [DuplicateRegression],
    /// New heavy dependencies since the baseline.
    pub new_heavy_deps: &'a [HeavyDepSnapshot],
    /// Timing comparison result.
    pub timing: &'a TimingComparison,
}

impl CheckReport {
    /// Whether the check found any regression. Drives the process exit code.
    pub fn is_regression(&self) -> bool {
        !self.new_duplicates.is_empty()
            || !self.new_heavy_deps.is_empty()
            || matches!(
                &self.timing,
                TimingComparison::Compared { regressions, .. } if !regressions.is_empty()
            )
    }

    /// A JSON-serializable view leading with the `regression` flag.
    pub fn view(&self) -> CheckReportView<'_> {
        CheckReportView {
            regression: self.is_regression(),
            new_duplicates: &self.new_duplicates,
            new_heavy_deps: &self.new_heavy_deps,
            timing: &self.timing,
        }
    }

    /// Render a human-readable summary for the terminal.
    pub fn render(&self) -> String {
        let mut out = String::new();
        if self.is_regression() {
            let _ = writeln!(
                out,
                "cargo clinic check: REGRESSION detected against baseline."
            );
        } else {
            let _ = writeln!(
                out,
                "cargo clinic check: OK — no regressions against baseline."
            );
        }

        if !self.new_duplicates.is_empty() {
            let _ = writeln!(out, "\nNew duplicate versions:");
            for d in &self.new_duplicates {
                let _ = writeln!(
                    out,
                    "  - {} now resolves to [{}] (new since baseline: {})",
                    d.name,
                    d.current_versions.join(", "),
                    d.new_versions.join(", ")
                );
            }
        }

        if !self.new_heavy_deps.is_empty() {
            let _ = writeln!(out, "\nNew heavy dependencies (proc-macro / build.rs):");
            for h in &self.new_heavy_deps {
                let _ = writeln!(out, "  - [{}] {} v{}", h.kind.as_str(), h.name, h.version);
            }
        }

        match &self.timing {
            TimingComparison::Skipped { reason } => {
                let _ = writeln!(out, "\nTiming comparison skipped: {reason}");
            }
            TimingComparison::Compared {
                threshold_percent,
                regressions,
            } => {
                if regressions.is_empty() {
                    let _ = writeln!(
                        out,
                        "\nTiming: all crates within the +{threshold_percent}% threshold."
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "\nTiming regressions (grew more than +{threshold_percent}% over baseline):"
                    );
                    for r in regressions {
                        let _ = writeln!(
                            out,
                            "  - {}: {} ms -> {} ms (+{}%)",
                            r.scope, r.baseline_ms, r.current_ms, r.increase_percent
                        );
                    }
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Pure comparison helpers
// ---------------------------------------------------------------------------

/// Duplicate regressions: for each currently-duplicated crate, the versions not
/// present in the baseline. A crate absent from the baseline duplicates counts
/// all of its current versions as new.
fn compare_duplicates(
    baseline: &[DuplicateSnapshot],
    current: &[DuplicateSnapshot],
) -> Vec<DuplicateRegression> {
    let baseline_by_name: BTreeMap<&str, &Vec<String>> = baseline
        .iter()
        .map(|d| (d.name.as_str(), &d.versions))
        .collect();

    let empty: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for dup in current {
        let known = baseline_by_name
            .get(dup.name.as_str())
            .copied()
            .unwrap_or(&empty);
        let new_versions: Vec<String> = dup
            .versions
            .iter()
            .filter(|v| !known.contains(v))
            .cloned()
            .collect();
        if !new_versions.is_empty() {
            out.push(DuplicateRegression {
                name: dup.name.clone(),
                new_versions,
                current_versions: dup.versions.clone(),
            });
        }
    }
    out
}

/// Heavy-dep regressions: dependencies present now, identified by (kind, name),
/// that were not in the baseline. Keyed by name (not version) so a patch bump of
/// an already-known heavy dep is not flagged as a brand-new one.
fn compare_heavy_deps(
    baseline: &[HeavyDepSnapshot],
    current: &[HeavyDepSnapshot],
) -> Vec<HeavyDepSnapshot> {
    let known: std::collections::BTreeSet<(u8, &str)> = baseline
        .iter()
        .map(|h| (h.kind.rank(), h.name.as_str()))
        .collect();
    current
        .iter()
        .filter(|h| !known.contains(&(h.kind.rank(), h.name.as_str())))
        .cloned()
        .collect()
}

/// Compare timing, skipping (never alarming) when data is missing or the
/// environment changed.
fn compare_timing(
    baseline: &Baseline,
    current: &Baseline,
    threshold_percent: u32,
) -> TimingComparison {
    let (Some(base_timing), Some(cur_timing)) = (&baseline.timing, &current.timing) else {
        return TimingComparison::Skipped {
            reason: SKIP_NO_TIMING.to_owned(),
        };
    };
    if let Some(reason) =
        environment_skip_reason(baseline.environment.as_ref(), current.environment.as_ref())
    {
        return TimingComparison::Skipped { reason };
    }
    TimingComparison::Compared {
        threshold_percent,
        regressions: timing_regressions(base_timing, cur_timing, threshold_percent),
    }
}

/// Return a skip reason if the timing comparison should be disabled because the
/// environment cannot be confirmed identical.
fn environment_skip_reason(
    baseline: Option<&EnvironmentFingerprint>,
    current: Option<&EnvironmentFingerprint>,
) -> Option<String> {
    match (baseline, current) {
        (Some(base), Some(cur)) => {
            if base.rustc_version != cur.rustc_version {
                return Some(format!(
                    "environment changed: rustc `{}` -> `{}`; timing comparison skipped to avoid a \
                     false alarm (recompilation cost differs across toolchains). Re-save the \
                     baseline on this toolchain.",
                    base.rustc_version, cur.rustc_version
                ));
            }
            if base.host != cur.host {
                return Some(format!(
                    "environment changed: host `{}` -> `{}`; timing comparison skipped (CPU/target \
                     affects build time). Re-save the baseline on this machine.",
                    base.host, cur.host
                ));
            }
            None
        }
        _ => Some(SKIP_NO_ENVIRONMENT.to_owned()),
    }
}

/// Compute per-crate and total timing regressions above the threshold.
fn timing_regressions(
    baseline: &TimingSnapshot,
    current: &TimingSnapshot,
    threshold_percent: u32,
) -> Vec<TimingRegression> {
    let mut regressions = Vec::new();
    if let Some(reg) = regression_for(
        TOTAL_SCOPE,
        baseline.total_ms,
        current.total_ms,
        threshold_percent,
    ) {
        regressions.push(reg);
    }

    let baseline_by_name: BTreeMap<&str, u64> = baseline
        .crates
        .iter()
        .map(|c| (c.crate_name.as_str(), c.total_ms))
        .collect();
    for c in &current.crates {
        // Only compare crates present in the baseline. A brand-new crate is a
        // dependency change (surfaced as a heavy dep, if relevant), not a
        // timing regression of existing work, and comparing against a zero
        // baseline would be pure noise.
        if let Some(&base_ms) = baseline_by_name.get(c.crate_name.as_str()) {
            if let Some(reg) = regression_for(&c.crate_name, base_ms, c.total_ms, threshold_percent)
            {
                regressions.push(reg);
            }
        }
    }
    regressions
}

/// A regression for one scope, or `None` if within threshold. Integer math only
/// (u128 to avoid overflow), so the result is exact and deterministic.
fn regression_for(
    scope: &str,
    baseline_ms: u64,
    current_ms: u64,
    threshold_percent: u32,
) -> Option<TimingRegression> {
    if baseline_ms == 0 {
        // A ratio against a zero baseline is undefined; do not alarm.
        return None;
    }
    let allowed =
        (u128::from(baseline_ms) * (PERCENT_BASE + u128::from(threshold_percent))) / PERCENT_BASE;
    if u128::from(current_ms) <= allowed {
        return None;
    }
    let increase = ((u128::from(current_ms) - u128::from(baseline_ms)) * PERCENT_BASE)
        / u128::from(baseline_ms);
    Some(TimingRegression {
        scope: scope.to_owned(),
        baseline_ms,
        current_ms,
        increase_percent: u64::try_from(increase).unwrap_or(u64::MAX),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Package, PackageId};
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::{AtomicU32, Ordering};

    fn graph_from(pkgs: Vec<Package>, edges: &[(&str, &str)]) -> DependencyGraph {
        let packages: BTreeMap<PackageId, Package> =
            pkgs.into_iter().map(|p| (p.id.clone(), p)).collect();
        let mut forward: BTreeMap<PackageId, BTreeSet<PackageId>> = BTreeMap::new();
        let mut reverse: BTreeMap<PackageId, BTreeSet<PackageId>> = BTreeMap::new();
        for (from, to) in edges {
            forward
                .entry((*from).to_owned())
                .or_default()
                .insert((*to).to_owned());
            reverse
                .entry((*to).to_owned())
                .or_default()
                .insert((*from).to_owned());
        }
        DependencyGraph::from_parts(packages, forward, reverse)
    }

    fn pkg(name: &str, version: &str, member: bool) -> Package {
        Package {
            id: format!("{name}@{version}"),
            name: name.to_owned(),
            version: version.to_owned(),
            is_workspace_member: member,
            is_proc_macro: false,
            has_build_script: false,
            default_features: Vec::new(),
            dependencies: Vec::new(),
        }
    }

    fn temp_path(tag: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "cargo-clinic-baseline-{tag}-{}-{n}.json",
            std::process::id()
        ))
    }

    fn env_a() -> EnvironmentFingerprint {
        EnvironmentFingerprint {
            rustc_version: "1.92.0".to_owned(),
            host: "x86_64-unknown-linux-gnu".to_owned(),
        }
    }

    #[test]
    fn from_graph_captures_duplicates_and_heavy_deps() {
        let app = pkg("app", "0.1.0", true);
        let dup1 = pkg("dup", "1.0.0", false);
        let dup2 = pkg("dup", "2.0.0", false);
        let mut pm = pkg("pm", "1.0.0", false);
        pm.is_proc_macro = true;
        let mut bs = pkg("bs", "1.0.0", false);
        bs.has_build_script = true;
        let graph = graph_from(
            vec![app, dup1, dup2, pm, bs],
            &[
                ("app@0.1.0", "dup@1.0.0"),
                ("app@0.1.0", "pm@1.0.0"),
                ("app@0.1.0", "bs@1.0.0"),
            ],
        );
        let baseline = Baseline::from_graph(&graph);

        assert_eq!(baseline.duplicates.len(), 1);
        assert_eq!(baseline.duplicates[0].name, "dup");
        assert_eq!(baseline.duplicates[0].versions, vec!["1.0.0", "2.0.0"]);

        let kinds: Vec<&str> = baseline
            .heavy_deps
            .iter()
            .map(|h| h.kind.as_str())
            .collect();
        assert!(kinds.contains(&"proc-macro"));
        assert!(kinds.contains(&"build-script"));
    }

    #[test]
    fn baseline_roundtrips_through_disk() {
        let baseline = Baseline {
            schema_version: BASELINE_SCHEMA_VERSION,
            duplicates: vec![DuplicateSnapshot {
                name: "dup".to_owned(),
                versions: vec!["1.0.0".to_owned(), "2.0.0".to_owned()],
            }],
            heavy_deps: vec![HeavyDepSnapshot {
                kind: HeavyDepKind::ProcMacro,
                name: "serde_derive".to_owned(),
                version: "1.0.0".to_owned(),
            }],
            environment: Some(env_a()),
            timing: Some(TimingSnapshot {
                total_ms: 100,
                crates: vec![CrateTimingSnapshot {
                    crate_name: "app".to_owned(),
                    total_ms: 100,
                }],
            }),
        };
        let path = temp_path("roundtrip");
        baseline.save(&path).expect("save");
        let loaded = Baseline::load(&path).expect("load");
        assert_eq!(loaded, baseline);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_rejects_unknown_schema_version() {
        let path = temp_path("schema");
        std::fs::write(
            &path,
            r#"{"schema_version":999,"duplicates":[],"heavy_deps":[]}"#,
        )
        .expect("write");
        let err = Baseline::load(&path).expect_err("must reject unknown schema");
        assert!(matches!(
            err,
            BaselineError::UnsupportedSchema { found: 999, .. }
        ));
        let _ = std::fs::remove_file(&path);
    }

    fn structural(duplicates: Vec<DuplicateSnapshot>, heavy: Vec<HeavyDepSnapshot>) -> Baseline {
        Baseline {
            schema_version: BASELINE_SCHEMA_VERSION,
            duplicates,
            heavy_deps: heavy,
            environment: None,
            timing: None,
        }
    }

    #[test]
    fn new_duplicate_version_is_a_regression() {
        let baseline = structural(
            vec![DuplicateSnapshot {
                name: "dup".to_owned(),
                versions: vec!["1.0.0".to_owned()],
            }],
            Vec::new(),
        );
        // A brand-new second version appears.
        let current = structural(
            vec![DuplicateSnapshot {
                name: "dup".to_owned(),
                versions: vec!["1.0.0".to_owned(), "2.0.0".to_owned()],
            }],
            Vec::new(),
        );
        let report = baseline.check(&current, DEFAULT_TIMING_THRESHOLD_PERCENT);
        assert!(report.is_regression());
        assert_eq!(report.new_duplicates.len(), 1);
        assert_eq!(report.new_duplicates[0].new_versions, vec!["2.0.0"]);
    }

    #[test]
    fn new_heavy_dep_is_a_regression() {
        let baseline = structural(Vec::new(), Vec::new());
        let current = structural(
            Vec::new(),
            vec![HeavyDepSnapshot {
                kind: HeavyDepKind::ProcMacro,
                name: "async-trait".to_owned(),
                version: "0.1.0".to_owned(),
            }],
        );
        let report = baseline.check(&current, DEFAULT_TIMING_THRESHOLD_PERCENT);
        assert!(report.is_regression());
        assert_eq!(report.new_heavy_deps.len(), 1);
        assert_eq!(report.new_heavy_deps[0].name, "async-trait");
    }

    #[test]
    fn identical_snapshot_is_not_a_regression() {
        let baseline = structural(
            vec![DuplicateSnapshot {
                name: "dup".to_owned(),
                versions: vec!["1.0.0".to_owned(), "2.0.0".to_owned()],
            }],
            vec![HeavyDepSnapshot {
                kind: HeavyDepKind::BuildScript,
                name: "ring".to_owned(),
                version: "0.17.0".to_owned(),
            }],
        );
        let current = baseline.clone();
        let report = baseline.check(&current, DEFAULT_TIMING_THRESHOLD_PERCENT);
        assert!(!report.is_regression());
    }

    #[test]
    fn heavy_dep_version_bump_is_not_a_new_dep() {
        let baseline = structural(
            Vec::new(),
            vec![HeavyDepSnapshot {
                kind: HeavyDepKind::ProcMacro,
                name: "serde_derive".to_owned(),
                version: "1.0.0".to_owned(),
            }],
        );
        let current = structural(
            Vec::new(),
            vec![HeavyDepSnapshot {
                kind: HeavyDepKind::ProcMacro,
                name: "serde_derive".to_owned(),
                version: "1.0.1".to_owned(),
            }],
        );
        let report = baseline.check(&current, DEFAULT_TIMING_THRESHOLD_PERCENT);
        assert!(!report.is_regression());
    }

    fn with_timing(total_ms: u64, crates: &[(&str, u64)], env: EnvironmentFingerprint) -> Baseline {
        structural(Vec::new(), Vec::new())
            .with_timing(TimingSnapshot {
                total_ms,
                crates: crates
                    .iter()
                    .map(|(n, ms)| CrateTimingSnapshot {
                        crate_name: (*n).to_owned(),
                        total_ms: *ms,
                    })
                    .collect(),
            })
            .with_environment(env)
    }

    #[test]
    fn timing_over_threshold_is_a_regression() {
        let baseline = with_timing(100, &[("app", 100)], env_a());
        // +25% > +20% default threshold.
        let current = with_timing(125, &[("app", 125)], env_a());
        let report = baseline.check(&current, DEFAULT_TIMING_THRESHOLD_PERCENT);
        assert!(report.is_regression());
        match report.timing {
            TimingComparison::Compared { regressions, .. } => {
                assert!(regressions.iter().any(|r| r.scope == TOTAL_SCOPE));
                assert!(regressions.iter().any(|r| r.scope == "app"));
            }
            other => panic!("expected Compared, got {other:?}"),
        }
    }

    #[test]
    fn timing_within_threshold_is_not_a_regression() {
        let baseline = with_timing(100, &[("app", 100)], env_a());
        // +15% <= +20% threshold: noise, not a regression.
        let current = with_timing(115, &[("app", 115)], env_a());
        let report = baseline.check(&current, DEFAULT_TIMING_THRESHOLD_PERCENT);
        assert!(!report.is_regression());
    }

    #[test]
    fn changed_environment_skips_timing_without_false_alarm() {
        let baseline = with_timing(100, &[("app", 100)], env_a());
        // Same code, but a much slower build on a NEW toolchain: must NOT alarm.
        let newer_env = EnvironmentFingerprint {
            rustc_version: "1.93.0".to_owned(),
            host: "x86_64-unknown-linux-gnu".to_owned(),
        };
        let current = with_timing(500, &[("app", 500)], newer_env);
        let report = baseline.check(&current, DEFAULT_TIMING_THRESHOLD_PERCENT);
        assert!(
            !report.is_regression(),
            "env change must not be a false alarm"
        );
        match report.timing {
            TimingComparison::Skipped { reason } => {
                assert!(
                    reason.contains("rustc"),
                    "reason should explain the toolchain change"
                );
            }
            other => panic!("expected Skipped, got {other:?}"),
        }
    }

    #[test]
    fn missing_timing_data_skips_comparison() {
        let baseline = structural(Vec::new(), Vec::new());
        let current = structural(Vec::new(), Vec::new());
        let report = baseline.check(&current, DEFAULT_TIMING_THRESHOLD_PERCENT);
        assert!(matches!(report.timing, TimingComparison::Skipped { .. }));
        assert!(!report.is_regression());
    }
}
