//! Prescriber + report: turn analyzer facts into ranked, prescriptive findings.
//!
//! This is the heart of the tool (CLAUDE.md Core Value 1): every finding ships
//! with a concrete fix path. The invariants enforced here are structural, not
//! stylistic:
//!
//! - **A finding cannot exist without a prescription.** [`Finding`] owns a
//!   non-optional [`Prescription`] whose `steps` are non-empty by construction
//!   (see [`FindingKind::ALL`] and the tests). A finding without a fix is a
//!   release blocker, so the type system refuses to build one.
//! - **Impact is qualitative only** (`likely` / `possible`; see ADR-0002). No
//!   rendering path emits a predicted duration. [`find_quantitative_claim`] is
//!   the machine check that guards this in tests over the rendered output.
//! - **Determinism.** Diagnosis is a pure function of the graph: findings are
//!   ranked by (impact, kind, title) with no wall clock or RNG.
//!
//! Adjacent tools (`cargo tree`, `cargo-llvm-lines`, `cargo-hakari`, …) are
//! surfaced inside prescriptions as the *next tool to reach for*, never
//! reimplemented and never framed as competitors (CLAUDE.md "Key Gotchas").

use std::fmt::{self, Write as _};

use serde::Serialize;

use crate::model::{DependencyGraph, Impact, PackageRef};

/// A workspace needs at least this many members before a workspace-hack
/// (feature unification) is worth suggesting; a single-crate project cannot
/// use `cargo-hakari` at all.
const WORKSPACE_HACK_MIN_MEMBERS: usize = 2;

/// Below this many resolved packages, linker time is unlikely to dominate an
/// edit-compile cycle, so the linker prescription would be noise.
const LINKER_ADVICE_MIN_PACKAGES: usize = 50;

/// The category of a finding. Each variant maps to exactly one prescription
/// rule, so covering every variant guarantees "every finding has a fix".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    /// One crate resolved to multiple versions (mirrors `cargo tree -d`).
    DuplicateVersions,
    /// A dependency declared with default features left on unnecessarily.
    DefaultFeatures,
    /// Proc-macro crates compiled and run at build time.
    ProcMacro,
    /// Crates that own a `build.rs` build script.
    BuildScript,
    /// Multi-member workspace that could unify features via a workspace-hack.
    WorkspaceHack,
    /// A large graph where a faster linker may help link-heavy rebuilds.
    Linker,
}

impl FindingKind {
    /// Every kind, in canonical rank order. Used both for deterministic
    /// ordering and by the "every kind has a prescription" test.
    pub const ALL: [FindingKind; 6] = [
        FindingKind::DuplicateVersions,
        FindingKind::DefaultFeatures,
        FindingKind::ProcMacro,
        FindingKind::BuildScript,
        FindingKind::WorkspaceHack,
        FindingKind::Linker,
    ];

    /// Stable machine-readable identifier (used by `--json`).
    pub fn as_str(&self) -> &'static str {
        match self {
            FindingKind::DuplicateVersions => "duplicate-versions",
            FindingKind::DefaultFeatures => "default-features",
            FindingKind::ProcMacro => "proc-macro",
            FindingKind::BuildScript => "build-script",
            FindingKind::WorkspaceHack => "workspace-hack",
            FindingKind::Linker => "linker",
        }
    }

    /// Deterministic secondary ranking key (kinds within an impact tier).
    fn rank(&self) -> u8 {
        match self {
            FindingKind::DuplicateVersions => 0,
            FindingKind::DefaultFeatures => 1,
            FindingKind::ProcMacro => 2,
            FindingKind::BuildScript => 3,
            FindingKind::WorkspaceHack => 4,
            FindingKind::Linker => 5,
        }
    }
}

/// A documentation reference attached to a prescription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reference {
    /// Human-readable label.
    pub label: String,
    /// URL to the reference.
    pub url: String,
}

impl Reference {
    fn new(label: &str, url: &str) -> Self {
        Reference {
            label: label.to_owned(),
            url: url.to_owned(),
        }
    }
}

/// An adjacent tool to reach for next. We integrate/credit these, never
/// compete with them (CLAUDE.md "Key Gotchas").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NextTool {
    /// Command or crate name (e.g. `cargo tree -d`, `cargo-hakari`).
    pub name: String,
    /// What the tool does for this finding.
    pub purpose: String,
    /// Where to find it.
    pub url: String,
}

impl NextTool {
    fn new(name: &str, purpose: &str, url: &str) -> Self {
        NextTool {
            name: name.to_owned(),
            purpose: purpose.to_owned(),
            url: url.to_owned(),
        }
    }
}

/// The concrete fix for a finding. `steps` is always non-empty; a prescription
/// with no steps would violate Core Value 1 and is never constructed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Prescription {
    /// Ordered, actionable steps.
    pub steps: Vec<String>,
    /// The next tool to reach for, if one applies.
    pub next_tool: Option<NextTool>,
    /// Documentation references.
    pub references: Vec<Reference>,
}

/// One ranked, prescriptive finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// The finding category.
    #[serde(rename = "kind", serialize_with = "serialize_kind")]
    pub kind: FindingKind,
    /// Qualitative impact.
    #[serde(rename = "impact", serialize_with = "serialize_impact")]
    pub impact: Impact,
    /// Short title.
    pub title: String,
    /// What is wrong and why it costs build time.
    pub diagnosis: String,
    /// The supporting data (never a predicted duration).
    pub evidence: Vec<String>,
    /// The concrete fix.
    pub prescription: Prescription,
}

fn serialize_kind<S: serde::Serializer>(kind: &FindingKind, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(kind.as_str())
}

fn serialize_impact<S: serde::Serializer>(impact: &Impact, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(impact.as_str())
}

/// Structural note carried by every report: impact is qualitative and no
/// durations are predicted (ADR-0002). Emitted by all three renderers.
pub const REPORT_DISCLAIMER: &str = "Impact is qualitative (likely / possible). This report makes no build-time predictions; measure before/after in your own environment to confirm any change.";

/// A ranked set of prescriptive findings for one workspace/crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    findings: Vec<Finding>,
}

/// A `Serialize`-able snapshot of a [`Report`] that always carries the
/// disclaimer, so `--json` cannot omit the qualitative-impact notice.
#[derive(Debug, Serialize)]
pub struct ReportView<'a> {
    /// Number of findings.
    pub finding_count: usize,
    /// The structural qualitative-impact disclaimer.
    pub disclaimer: &'a str,
    /// Findings, already ranked.
    pub findings: &'a [Finding],
}

impl Report {
    /// Diagnose a resolved dependency graph into ranked, prescriptive findings.
    ///
    /// Pure and deterministic: same graph in, same report out.
    pub fn diagnose(graph: &DependencyGraph) -> Report {
        let mut findings = Vec::new();

        findings.extend(duplicate_findings(graph));
        findings.extend(default_feature_findings(graph));
        findings.extend(proc_macro_finding(graph));
        findings.extend(build_script_finding(graph));
        findings.extend(workspace_hack_finding(graph));
        findings.extend(linker_finding(graph));

        // Rank: most certain first, then by kind, then by title (stable).
        findings.sort_by(|a, b| {
            a.impact
                .rank()
                .cmp(&b.impact.rank())
                .then_with(|| a.kind.rank().cmp(&b.kind.rank()))
                .then_with(|| a.title.cmp(&b.title))
        });

        Report { findings }
    }

    /// The ranked findings.
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    /// A JSON-serializable snapshot that always carries the disclaimer.
    pub fn view(&self) -> ReportView<'_> {
        ReportView {
            finding_count: self.findings.len(),
            disclaimer: REPORT_DISCLAIMER,
            findings: &self.findings,
        }
    }

    /// Render the terminal (plain-text) report.
    pub fn render_table(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "cargo clinic report — {} finding(s)",
            self.findings.len()
        );
        if self.findings.is_empty() {
            let _ = writeln!(out, "  No findings. Nothing to prescribe.");
        }
        for (idx, f) in self.findings.iter().enumerate() {
            let _ = writeln!(out);
            let _ = writeln!(out, "{}. [{}] {}", idx + 1, f.impact.label(), f.title);
            let _ = writeln!(out, "   Diagnosis: {}", f.diagnosis);
            if !f.evidence.is_empty() {
                let _ = writeln!(out, "   Evidence:");
                for line in &f.evidence {
                    let _ = writeln!(out, "     - {line}");
                }
            }
            let _ = writeln!(out, "   Prescription:");
            for (i, step) in f.prescription.steps.iter().enumerate() {
                let _ = writeln!(out, "     {}. {step}", i + 1);
            }
            if let Some(tool) = &f.prescription.next_tool {
                let _ = writeln!(
                    out,
                    "   Next tool: {} — {} ({})",
                    tool.name, tool.purpose, tool.url
                );
            }
            if !f.prescription.references.is_empty() {
                let _ = writeln!(out, "   References:");
                for r in &f.prescription.references {
                    let _ = writeln!(out, "     - {}: {}", r.label, r.url);
                }
            }
        }
        let _ = writeln!(out);
        let _ = write!(out, "NOTE: {REPORT_DISCLAIMER}");
        out
    }

    /// Render the Markdown report.
    pub fn render_markdown(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# cargo clinic report");
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "_{} finding(s). {REPORT_DISCLAIMER}_",
            self.findings.len()
        );
        if self.findings.is_empty() {
            let _ = writeln!(out);
            let _ = writeln!(out, "No findings. Nothing to prescribe.");
        }
        for (idx, f) in self.findings.iter().enumerate() {
            let _ = writeln!(out);
            let _ = writeln!(out, "## {}. [{}] {}", idx + 1, f.impact.label(), f.title);
            let _ = writeln!(out);
            let _ = writeln!(out, "**Diagnosis:** {}", f.diagnosis);
            if !f.evidence.is_empty() {
                let _ = writeln!(out);
                let _ = writeln!(out, "**Evidence:**");
                let _ = writeln!(out);
                for line in &f.evidence {
                    let _ = writeln!(out, "- {line}");
                }
            }
            let _ = writeln!(out);
            let _ = writeln!(out, "**Prescription:**");
            let _ = writeln!(out);
            for (i, step) in f.prescription.steps.iter().enumerate() {
                let _ = writeln!(out, "{}. {step}", i + 1);
            }
            if let Some(tool) = &f.prescription.next_tool {
                let _ = writeln!(out);
                let _ = writeln!(
                    out,
                    "**Next tool:** [{}]({}) — {}",
                    tool.name, tool.url, tool.purpose
                );
            }
            if !f.prescription.references.is_empty() {
                let _ = writeln!(out);
                let _ = writeln!(out, "**References:**");
                let _ = writeln!(out);
                for r in &f.prescription.references {
                    let _ = writeln!(out, "- [{}]({})", r.label, r.url);
                }
            }
        }
        out
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.render_table())
    }
}

// ---------------------------------------------------------------------------
// Finding generation (the prescription rule table)
// ---------------------------------------------------------------------------

/// Render a list of package refs as `name` (deduped by name, comma-joined).
fn dependent_names(refs: &[PackageRef]) -> String {
    if refs.is_empty() {
        return "(no direct dependents)".to_owned();
    }
    let mut names: Vec<&str> = refs.iter().map(|r| r.name.as_str()).collect();
    names.dedup();
    names.join(", ")
}

/// Render an inverse path (instance-first) as `root -> ... -> instance`.
fn render_inverse_path(path: &[PackageRef]) -> String {
    path.iter()
        .rev()
        .map(|r| r.name.as_str())
        .collect::<Vec<_>>()
        .join(" -> ")
}

fn duplicate_findings(graph: &DependencyGraph) -> Vec<Finding> {
    graph
        .duplicate_versions()
        .into_iter()
        .map(|dup| {
            let versions: Vec<&str> = dup.instances.iter().map(|i| i.version.as_str()).collect();
            let diagnosis = format!(
                "`{}` is resolved to {} different versions ({}). Each version is compiled \
                 separately, so the crate and its unique transitive dependencies are built more \
                 than once.",
                dup.name,
                dup.instances.len(),
                versions.join(", ")
            );

            let mut evidence = Vec::new();
            for inst in &dup.instances {
                evidence.push(format!(
                    "v{} required by {}",
                    inst.version,
                    dependent_names(&inst.direct_dependents)
                ));
                if let Some(path) = inst.inverse_paths.first() {
                    evidence.push(format!("  path: {}", render_inverse_path(path)));
                }
            }

            Finding {
                kind: FindingKind::DuplicateVersions,
                impact: Impact::Likely,
                title: format!("Duplicate versions of `{}`", dup.name),
                diagnosis,
                evidence,
                prescription: Prescription {
                    steps: vec![
                        format!(
                            "Run `cargo tree -d -i {}` to see every path that pulls in each \
                             version.",
                            dup.name
                        ),
                        "Align the requirements: bump the crate(s) pinning the older version, or \
                         relax an over-tight `=`/`~` requirement, so all dependents can share one \
                         version."
                            .to_owned(),
                        format!(
                            "If a direct dependency pins the old `{}`, update that dependency (or \
                             its parent) to a release that accepts the newer version.",
                            dup.name
                        ),
                        "Re-run `cargo clinic report` (or `cargo tree -d`) to confirm the \
                         duplicate is gone."
                            .to_owned(),
                    ],
                    next_tool: Some(NextTool::new(
                        "cargo tree -d -i",
                        "list duplicate versions and the exact paths that require each one",
                        "https://doc.rust-lang.org/cargo/commands/cargo-tree.html",
                    )),
                    references: vec![Reference::new(
                        "The Cargo Book — Dependency resolution",
                        "https://doc.rust-lang.org/cargo/reference/resolver.html",
                    )],
                },
            }
        })
        .collect()
}

fn default_feature_findings(graph: &DependencyGraph) -> Vec<Finding> {
    graph
        .default_features_opportunities()
        .into_iter()
        .map(|opp| {
            let features = opp.default_expands_to.join(", ");
            let diagnosis = format!(
                "`{}` depends on `{}` with default features left on; its `default` feature turns \
                 on: {}. If those features are unused, they add compile work for no benefit.",
                opp.dependent.name, opp.dependency.name, features
            );
            let evidence = vec![
                format!("declared in `{}`'s Cargo.toml", opp.dependent.name),
                format!("`{}` default expands to: {}", opp.dependency.name, features),
            ];
            Finding {
                kind: FindingKind::DefaultFeatures,
                impact: opp.impact,
                title: format!("Default features enabled on `{}`", opp.dependency.name),
                diagnosis,
                evidence,
                prescription: Prescription {
                    steps: vec![
                        format!(
                            "Confirm the default features are actually unused: `cargo tree -e \
                             features -i {}` shows what enables each feature.",
                            opp.dependency.name
                        ),
                        format!(
                            "In `{}`'s Cargo.toml set `default-features = false` for `{}`, then \
                             re-add only the features you use: `{} = {{ version = \"…\", \
                             default-features = false, features = [\"…\"] }}`.",
                            opp.dependent.name, opp.dependency.name, opp.dependency.name
                        ),
                        "Build and test to confirm nothing you rely on was behind a default \
                         feature."
                            .to_owned(),
                    ],
                    next_tool: Some(NextTool::new(
                        "cargo tree -e features -i",
                        "trace which crate or feature enables each default feature",
                        "https://doc.rust-lang.org/cargo/commands/cargo-tree.html",
                    )),
                    references: vec![Reference::new(
                        "The Cargo Book — Features",
                        "https://doc.rust-lang.org/cargo/reference/features.html",
                    )],
                },
            }
        })
        .collect()
}

fn proc_macro_finding(graph: &DependencyGraph) -> Option<Finding> {
    let crates = graph.proc_macro_crates();
    if crates.is_empty() {
        return None;
    }
    let evidence: Vec<String> = crates
        .iter()
        .map(|c| {
            format!(
                "{} v{} <- {}",
                c.package.name,
                c.package.version,
                dependent_names(&c.dependents)
            )
        })
        .collect();
    Some(Finding {
        kind: FindingKind::ProcMacro,
        impact: Impact::Possible,
        title: "Proc-macro crates in the build graph".to_owned(),
        diagnosis: format!(
            "{} proc-macro crate(s) are compiled and executed at build time. Proc-macros run \
             during every compile of their dependents and can dominate front-end time.",
            crates.len()
        ),
        evidence,
        prescription: Prescription {
            steps: vec![
                "Check whether each proc-macro is essential; some derives have lighter \
                 alternatives, or can be replaced by a hand-written impl in hot crates."
                    .to_owned(),
                "Measure front-end cost with `cargo llvm-lines` before removing anything — inspect, \
                 do not guess."
                    .to_owned(),
                "Keep proc-macro-heavy dependencies in a stable lower layer so they are not \
                 recompiled when your own code changes."
                    .to_owned(),
            ],
            next_tool: Some(NextTool::new(
                "cargo-llvm-lines",
                "reveal which generic and macro-generated code expands the most",
                "https://github.com/dtolnay/cargo-llvm-lines",
            )),
            references: vec![Reference::new(
                "The Rust Performance Book — Compile times",
                "https://nnethercote.github.io/perf-book/compile-times.html",
            )],
        },
    })
}

fn build_script_finding(graph: &DependencyGraph) -> Option<Finding> {
    let crates = graph.build_script_crates();
    if crates.is_empty() {
        return None;
    }
    let evidence: Vec<String> = crates
        .iter()
        .map(|c| {
            format!(
                "{} v{} <- {}",
                c.package.name,
                c.package.version,
                dependent_names(&c.dependents)
            )
        })
        .collect();
    Some(Finding {
        kind: FindingKind::BuildScript,
        impact: Impact::Possible,
        title: "Build scripts (build.rs) in the build graph".to_owned(),
        diagnosis: format!(
            "{} crate(s) run a build script. build.rs runs at build time and is INVISIBLE to \
             RUSTC_WRAPPER timing, so its cost never appears in measured rustc numbers.",
            crates.len()
        ),
        evidence,
        prescription: Prescription {
            steps: vec![
                "Confirm each build script is required; some exist only for optional features you \
                 may not use."
                    .to_owned(),
                "Prefer crates that gate native or codegen build scripts behind features, and \
                 disable those features when unused."
                    .to_owned(),
                "For your own build scripts, cache expensive work and emit precise \
                 `cargo:rerun-if-changed` lines so they do not re-run needlessly."
                    .to_owned(),
            ],
            next_tool: None,
            references: vec![Reference::new(
                "The Cargo Book — Build scripts",
                "https://doc.rust-lang.org/cargo/reference/build-scripts.html",
            )],
        },
    })
}

fn workspace_hack_finding(graph: &DependencyGraph) -> Option<Finding> {
    let member_count = graph.workspace_members().count();
    if member_count < WORKSPACE_HACK_MIN_MEMBERS {
        return None;
    }
    let has_duplicates = !graph.duplicate_versions().is_empty();
    let mut evidence = vec![format!("{member_count} workspace members")];
    if has_duplicates {
        evidence
            .push("duplicate versions present, which compounds feature-driven rebuilds".to_owned());
    }
    Some(Finding {
        kind: FindingKind::WorkspaceHack,
        impact: Impact::Possible,
        title: "Workspace feature unification (consider a workspace-hack)".to_owned(),
        diagnosis: format!(
            "This workspace has {member_count} members. Building individual members (e.g. `cargo \
             build -p …`) can compile shared dependencies with different feature sets, causing \
             redundant rebuilds."
        ),
        evidence,
        prescription: Prescription {
            steps: vec![
                "Adopt `cargo-hakari` to generate a `workspace-hack` crate that unifies dependency \
                 features across the whole workspace."
                    .to_owned(),
                "Install and set up: `cargo install cargo-hakari`, then `cargo hakari init`, \
                 `cargo hakari generate`, and `cargo hakari manage-deps`."
                    .to_owned(),
                "Add `cargo hakari generate --diff` to CI so the hack crate stays current."
                    .to_owned(),
            ],
            next_tool: Some(NextTool::new(
                "cargo-hakari",
                "generate and maintain a workspace-hack crate for feature unification",
                "https://docs.rs/cargo-hakari",
            )),
            references: vec![Reference::new(
                "cargo-hakari — About workspace-hack crates",
                "https://docs.rs/cargo-hakari/latest/cargo_hakari/",
            )],
        },
    })
}

fn linker_finding(graph: &DependencyGraph) -> Option<Finding> {
    let package_count = graph.packages().count();
    if package_count < LINKER_ADVICE_MIN_PACKAGES {
        return None;
    }
    Some(Finding {
        kind: FindingKind::Linker,
        impact: Impact::Possible,
        title: "Consider a faster linker for link-heavy builds".to_owned(),
        diagnosis: format!(
            "This build resolves {package_count} packages. Large graphs spend a meaningful share \
             of each incremental rebuild in the linker; the default linker is often the slowest \
             step of an edit-compile cycle."
        ),
        evidence: vec![format!("{package_count} resolved packages")],
        prescription: Prescription {
            steps: vec![
                "Linux: install `lld` or `mold` and select it in `.cargo/config.toml`: \
                 `[target.x86_64-unknown-linux-gnu] rustflags = [\"-C\", \
                 \"link-arg=-fuse-ld=lld\"]` (swap `lld` for `mold` if installed)."
                    .to_owned(),
                "macOS: recent toolchains already use a fast default linker; prefer upgrading \
                 Xcode and Rust before adding a custom linker."
                    .to_owned(),
                "Windows: `lld-link` ships with rustup's LLVM tools; select it the same way in \
                 `.cargo/config.toml`."
                    .to_owned(),
                "Change one environment at a time and compare your own before/after; linker gains \
                 depend heavily on hardware and graph shape."
                    .to_owned(),
            ],
            next_tool: Some(NextTool::new(
                "mold",
                "a fast drop-in linker for Linux (and its macOS sibling, sold)",
                "https://github.com/rui314/mold",
            )),
            references: vec![Reference::new(
                "The Rust Performance Book — Linking",
                "https://nnethercote.github.io/perf-book/compile-times.html#linking",
            )],
        },
    })
}

// ---------------------------------------------------------------------------
// Output lint: no fabricated duration claims (ADR-0002)
// ---------------------------------------------------------------------------

/// Time-unit suffixes that, attached to a number, denote a duration.
const TIME_UNITS: &[&str] = &[
    "ms", "s", "sec", "secs", "second", "seconds", "min", "mins", "minute", "minutes", "hr", "hrs",
    "hour", "hours",
];

/// Phrases that would imply a fabricated speedup prediction.
const PREDICTION_PHRASES: &[&str] = &[
    "will save",
    "will reduce build",
    "will cut",
    "faster by",
    "% faster",
    "x faster",
    "speedup of",
    "save you",
    "shaves off",
];

/// Scan rendered output for a quantitative build-time prediction, returning a
/// description of the first offender (or `None` if clean).
///
/// This backs the ADR-0002 output lint. It flags two things: (1) a denylist of
/// predictive phrases, and (2) any `<number><time-unit>` token (e.g. `40s`,
/// `250ms`, `3 minutes`). Version strings like `0.1.0` are not durations and
/// are not flagged, because their trailing part is not a time unit.
pub fn find_quantitative_claim(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    for phrase in PREDICTION_PHRASES {
        if lower.contains(phrase) {
            return Some(format!("prediction phrase: `{phrase}`"));
        }
    }

    let tokens: Vec<&str> = text.split_whitespace().collect();
    for (i, raw) in tokens.iter().enumerate() {
        let tok = trim_edges(raw);
        if tok.is_empty() {
            continue;
        }
        // Case A: number and unit fused in one token, e.g. `40s`, `250ms`.
        if let Some((num, unit)) = split_number_unit(tok) {
            if is_number(num) && is_time_unit(unit) {
                return Some(format!("numeric duration token: `{tok}`"));
            }
        }
        // Case B: a bare number followed by a unit word, e.g. `3 minutes`.
        if is_number(tok) {
            if let Some(next) = tokens.get(i + 1) {
                let unit = trim_edges(next);
                if is_time_unit(unit) {
                    return Some(format!("numeric duration: `{tok} {unit}`"));
                }
            }
        }
    }
    None
}

/// Trim leading/trailing punctuation, keeping inner alphanumerics and dots.
fn trim_edges(tok: &str) -> &str {
    tok.trim_matches(|c: char| !c.is_ascii_alphanumeric())
}

/// Split a token into a leading `[0-9.]` run and the remaining suffix.
fn split_number_unit(tok: &str) -> Option<(&str, &str)> {
    let idx = tok.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    Some((&tok[..idx], &tok[idx..]))
}

/// A parseable decimal number that starts with a digit (so word tokens like
/// `inf` / `nan`, which `f64::parse` also accepts, are not treated as numbers).
fn is_number(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_digit()) && s.parse::<f64>().is_ok()
}

/// Case-insensitive membership in [`TIME_UNITS`].
fn is_time_unit(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    TIME_UNITS.contains(&lower.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DepKind, DependencyDecl, Package, PackageId};
    use std::collections::{BTreeMap, BTreeSet};

    /// Build a graph from packages plus (from -> to) direct-dependency edges.
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

    #[test]
    fn every_kind_has_a_non_empty_prescription() {
        // Machine check: build a graph that triggers every finding kind, then
        // assert each produced finding carries a non-empty prescription.
        let mut app = pkg("app", "0.1.0", true);
        let lib = pkg("lib", "0.1.0", true);
        let mut pm = pkg("pm", "1.0.0", false);
        pm.is_proc_macro = true;
        let mut bs = pkg("bs", "1.0.0", false);
        bs.has_build_script = true;
        let mut featured = pkg("featured", "1.0.0", false);
        featured.default_features = vec!["extra".to_owned()];
        app.dependencies = vec![DependencyDecl {
            name: "featured".to_owned(),
            kind: DepKind::Normal,
            uses_default_features: true,
            optional: false,
        }];
        // Duplicate: dup@1 and dup@2.
        let dup1 = pkg("dup", "1.0.0", false);
        let dup2 = pkg("dup", "2.0.0", false);

        // Pad the graph past the linker threshold with filler packages.
        let mut pkgs = vec![app, lib, pm, bs, featured, dup1, dup2];
        for i in 0..LINKER_ADVICE_MIN_PACKAGES {
            pkgs.push(pkg(&format!("filler{i}"), "0.1.0", false));
        }

        let edges = [
            ("app@0.1.0", "pm@1.0.0"),
            ("app@0.1.0", "bs@1.0.0"),
            ("app@0.1.0", "featured@1.0.0"),
            ("app@0.1.0", "dup@1.0.0"),
            ("lib@0.1.0", "dup@2.0.0"),
        ];
        let graph = graph_from(pkgs, &edges);
        let report = Report::diagnose(&graph);

        let kinds: BTreeSet<&str> = report.findings().iter().map(|f| f.kind.as_str()).collect();
        // All six kinds fire on this planted graph.
        for kind in FindingKind::ALL {
            assert!(
                kinds.contains(kind.as_str()),
                "expected a `{}` finding to be produced",
                kind.as_str()
            );
        }
        // The core invariant: no finding ships without steps.
        for f in report.findings() {
            assert!(
                !f.prescription.steps.is_empty(),
                "finding `{}` has no prescription steps",
                f.kind.as_str()
            );
        }
    }

    #[test]
    fn findings_are_ranked_likely_before_possible() {
        let app = pkg("app", "0.1.0", true);
        let lib = pkg("lib", "0.1.0", true);
        let dup1 = pkg("dup", "1.0.0", false);
        let dup2 = pkg("dup", "2.0.0", false);
        let graph = graph_from(
            vec![app, lib, dup1, dup2],
            &[("app@0.1.0", "dup@1.0.0"), ("lib@0.1.0", "dup@2.0.0")],
        );
        let report = Report::diagnose(&graph);
        // Duplicate (Likely) must rank ahead of the workspace-hack (Possible).
        assert_eq!(report.findings()[0].kind, FindingKind::DuplicateVersions);
        assert_eq!(report.findings()[0].impact, Impact::Likely);
        assert!(report
            .findings()
            .iter()
            .any(|f| f.kind == FindingKind::WorkspaceHack));
    }

    #[test]
    fn empty_graph_yields_no_findings() {
        let graph = graph_from(vec![pkg("solo", "0.1.0", true)], &[]);
        let report = Report::diagnose(&graph);
        assert!(report.findings().is_empty());
        assert!(report.render_table().contains("No findings"));
        assert!(report.render_markdown().contains("No findings"));
    }

    #[test]
    fn rendered_output_has_no_quantitative_predictions() {
        // A graph that triggers every kind, so the lint sees the full surface.
        let mut app = pkg("app", "0.1.0", true);
        let lib = pkg("lib", "0.1.0", true);
        let mut pm = pkg("pm", "1.0.0", false);
        pm.is_proc_macro = true;
        let mut bs = pkg("bs", "1.0.0", false);
        bs.has_build_script = true;
        let mut featured2 = pkg("featured", "1.0.0", false);
        featured2.default_features = vec!["extra".to_owned()];
        app.dependencies = vec![DependencyDecl {
            name: "featured".to_owned(),
            kind: DepKind::Normal,
            uses_default_features: true,
            optional: false,
        }];
        let dup1 = pkg("dup", "1.0.0", false);
        let dup2 = pkg("dup", "2.0.0", false);
        let mut pkgs = vec![app, lib, pm, bs, featured2, dup1, dup2];
        for i in 0..LINKER_ADVICE_MIN_PACKAGES {
            pkgs.push(pkg(&format!("filler{i}"), "0.1.0", false));
        }
        let graph = graph_from(
            pkgs,
            &[
                ("app@0.1.0", "pm@1.0.0"),
                ("app@0.1.0", "bs@1.0.0"),
                ("app@0.1.0", "featured@1.0.0"),
                ("app@0.1.0", "dup@1.0.0"),
                ("lib@0.1.0", "dup@2.0.0"),
            ],
        );
        let report = Report::diagnose(&graph);

        for rendered in [
            report.render_table(),
            report.render_markdown(),
            serde_json::to_string_pretty(&report.view()).expect("json"),
        ] {
            assert_eq!(
                find_quantitative_claim(&rendered),
                None,
                "rendered output must not contain a duration prediction:\n{rendered}"
            );
        }
    }

    #[test]
    fn lint_flags_duration_tokens_but_not_versions() {
        assert!(find_quantitative_claim("this will save 40s of build").is_some());
        assert!(find_quantitative_claim("saves 250ms per crate").is_some());
        assert!(find_quantitative_claim("about 3 minutes faster").is_some());
        assert!(find_quantitative_claim("cuts 2 hours").is_some());
        // Non-durations must stay clean.
        assert_eq!(find_quantitative_claim("leftpad 0.1.0 and 0.2.0"), None);
        assert_eq!(find_quantitative_claim("resolved to 2 versions"), None);
        assert_eq!(find_quantitative_claim("x86_64-unknown-linux-gnu"), None);
        assert_eq!(find_quantitative_claim("3 workspace members"), None);
    }
}
