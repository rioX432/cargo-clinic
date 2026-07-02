# ADR-0004: Baseline snapshot + `check` regression gate

- Status: Accepted / Date: 2026-07-02

## Context
Core Value 3 (CI-first) needs a stored baseline and a regression gate. Two open
questions: (1) the CLI surface — the issue wrote `cargo clinic --check`, but the
existing CLI is subcommand-based (`report` / `measure` / `import`), and clap
does not mix a top-level flag with required subcommands cleanly; (2) how to
compare noisy timing without false alarms (ADR-0002 already forbids fabricated
durations).

## Decision
- **CLI**: add a `check` subcommand (`cargo clinic check --baseline <path>`),
  matching the cargo subcommand convention, rather than a top-level `--check`
  flag. Baselines are written by `report --save-baseline <path>` (default
  compare path `.cargo-clinic/baseline.json`). A regression exits `1`, distinct
  from a hard error (which prints `error:`), so CI separates "found a
  regression" from "tool failed to run".
- **Schema**: a versioned JSON baseline (`schema_version`) capturing duplicate
  versions, heavy deps (proc-macro / build.rs) keyed by (kind, name), and an
  optional timing snapshot plus an environment fingerprint (rustc release +
  host). `load` rejects an unknown schema version.
- **Regression = subset check**: new duplicate (name, version) pair, or new
  heavy dep (kind, name) not in the baseline. A patch bump of a known heavy dep
  is not flagged.
- **Noise-aware timing**: comparison runs only when both sides carry timing on
  the *same* environment; a changed rustc/host auto-skips with an explanation
  instead of alarming. A crate/total must exceed a conservative default
  threshold (`+20%`, configurable) to count. The comparison is a pure function
  (no clock/RNG); environment + timing are captured in the binary and injected.

## Consequences
- Upside: gate lives in CI as a first-class subcommand; timing false-alarms are
  structurally prevented, consistent with ADR-0002.
- Downside: timing regression needs a committed baseline measured on a matching
  toolchain; on hosted runners timing usually auto-skips, so the example Action
  gates on structural regressions (duplicates / heavy deps) only.
