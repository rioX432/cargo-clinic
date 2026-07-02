# cargo-clinic

> Diagnose slow Rust builds and get ranked, actionable prescriptions.

`cargo clinic report` unifies scattered build diagnostics — duplicate dependency versions, proc-macro and build.rs inventory, feature bloat, optional timing data — into one prescriptive report. `cargo clinic --check` guards CI against build-time regressions with a stored baseline.

**Status**: pre-v0.1, building in public. Not launched yet — interfaces and scope will change until v0.1.

- Impact claims are qualitative (`likely` / `possible`) — no fabricated seconds
- Stable-first: timing via RUSTC_WRAPPER; nightly `--timings=json` behind an explicit experimental flag
