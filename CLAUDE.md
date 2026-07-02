# cargo-clinic

@REVIEW.md

## Project Overview

Diagnose slow Rust builds and get ranked, actionable prescriptions. `cargo clinic report` unifies scattered diagnostics (duplicate versions, proc-macro/build.rs inventory, feature bloat, optional timing data) into one prescriptive report, with a baseline `--check` mode for CI regression.

## Core Values

1. **Prescriptions, not raw data** — every finding ships with a concrete fix path (what to change, where, how to verify). A finding without a prescription doesn't ship.
2. **Honest impact claims** — impact is qualitative (`likely` / `possible`), never fabricated seconds. Measurement variance is real; we don't pretend otherwise.
3. **CI-first** — `--check` against a stored baseline is a first-class citizen, not an afterthought. The tool should live in CI, not just in one-off runs.

## Won't Do

- **Own timing engine on stable via HTML parsing**: Cargo's `--timings` HTML is officially "human consumption only" — parsing it is a fallback experiment (`import --timings-html`), never the main path
- **Quantitative speedup predictions** ("this will save 40s"): unverifiable, slop-adjacent
- **Auto-applying fixes**: we prescribe, the user operates
- **TUI in v0.1**: CLI table + `--json` + markdown only; value is analysis correctness

## Commands

```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

## Architecture

```
crates/
  clinic-core/     # lib: collectors (cargo metadata, timing), analyzers, prescriber, baseline
  cargo-clinic/    # bin: cargo subcommand CLI (report / --check / import)
tests/fixtures/    # mini cargo workspaces with planted problems (dup versions, heavy proc-macros, default-features) — golden test inputs
```

## Tech Stack

| Layer | Technology |
|---|---|
| metadata | cargo_metadata crate |
| timing (stable) | RUSTC_WRAPPER shim measuring rustc invocations (opt-in) |
| timing (nightly) | `--timings=json` import behind explicit flag (unstable upstream) |
| CLI | clap (derive), cargo subcommand convention (`cargo clinic`) |
| output | terminal table + `--json` + markdown |
| distribution | cargo-dist + Homebrew tap + cargo-binstall |

## Key Gotchas

- `cargo --timings=json` is UNSTABLE (verified on cargo 1.92); stable HTML output is documented as human-only. Never build the main path on either — stable path = RUSTC_WRAPPER
- Feature unification "why is this feature on" — don't reimplement the resolver; shell out to / mirror `cargo tree -e features -i <crate>`
- RUSTC_WRAPPER measures rustc invocations only — build.rs execution and Cargo scheduling are invisible to it; report must say so
- Adjacent tools to integrate/credit, not compete with: `cargo tree -d`, `cargo-llvm-lines`, `cargo-hakari`, `cargo-bloat`, sccache
- Dogfooding target: the agent-witness workspace (sibling project) — its build is our first patient

## Development Process

- AI-driven: issues are Dev Ready; use `/dev` per issue, `/dev-all` for batches. Human judgment points: finding taxonomy, prescription wording, launch copy
- Every PR includes a short design-decision note (launch article material)
- No squash merges; incremental history is part of the public ownership story

## Language

- Code comments, variable names: English
- Commits: concise single line, English
