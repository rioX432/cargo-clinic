# ADR-0001: RUSTC_WRAPPER as the stable timing path

- Status: Accepted / Date: 2026-07-02

## Context
`cargo build --timings=json` is unstable (verified on cargo 1.92); the stable `--timings` HTML output is documented by the Cargo Book as "for human consumption only" with no machine-readable guarantee. Building the main data path on either would be fragile or nightly-only.

## Decision
The stable timing path is a RUSTC_WRAPPER shim (`cargo clinic measure`) that records each rustc invocation (crate, start/end, duration). Nightly users may `import --timings-json` behind an explicit experimental flag. HTML parsing is out of scope (Won't Do).

## Consequences
- Upside: works on stable; data format is ours; no dependency on Cargo internals.
- Downside: build.rs execution and Cargo scheduling are invisible to the wrapper; every report states this limitation. Interaction with sccache/other wrappers must be detected and warned about (issue #3).
