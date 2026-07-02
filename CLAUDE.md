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

## Build & Run

```bash
rustup default stable
cargo install cargo-nextest just

just verify    # primary local gate: check (fmt+clippy+build) + nextest. Run before merge.
just check     # fmt + clippy + build
just test      # nextest
just build
```

## Verification

The primary merge gate is **`just verify` locally**; GitHub Actions CI mirrors the exact same gate (public-OSS trust signal — see ADR-0003 for why this diverges from avatar-core's PoC no-CI stance). If `just verify` is green, the change is mergeable.

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

## Rust Conventions

- `just check` must pass (fmt, clippy `-D warnings`, build); tests via nextest.
- No `unsafe`. Determinism in core logic: no wall-clock or RNG inside pure paths — inject them.
- No magic numbers; errors via `thiserror` in lib crates, `anyhow` in bins; no `unwrap`/`expect` outside tests.

## Development Harness

Issue-driven development with `/dev` (single issue) and `/dev-all` (sequential). Other skills: `/audit`, `/update-docs`, `/decompose`, `/investigate`, `/review`, `/tech-debt`, `/pr`.

Review accumulation (ADR-0003): valid review findings are promoted into rules (`.claude/rules/`), lints, and skills. Promotion: a finding that recurs twice becomes a rule. Retirement: a rule unused for 3 months is removed.

Human judgment points: event/report schema changes, CLI surface, launch copy. Every PR includes a short design-decision note (launch article material). No squash merges.

## Phasing

Design is decided ahead (docs/adr/, zero-base design docs), but implementation may diverge. v0.1 issues are fully detailed; v0.2+ exist only as ADR notes and are detailed at the v0.1 gate. The v0.1 gate is a human decision after the first vertical spike (calibrate actual pace; shrink scope if estimates double).

## Language

- Code comments, variable names: English
- Commits: concise single line, English
