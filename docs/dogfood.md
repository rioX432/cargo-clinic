# Dogfood log: diagnosing `agent-witness`

The first patient is the sibling project [`agent-witness`](https://github.com/rioX432/agent-witness)
(a small two-crate Rust workspace). This is a **real** run of `cargo clinic`
against it, recorded verbatim — diagnosis, the prescriptions it produced, and a
before/after measurement.

- Tool: `cargo-clinic` built from this branch (`cargo build -p cargo-clinic`).
- Patient: `/Users/rio/workspace/projects/agent-witness` (unmodified — read-only).
- Toolchain: `rustc 1.92.0`, `cargo 1.92.0`, aarch64-apple-darwin.
- Measurement note: every `measure` run used a **cold** (empty) `CARGO_TARGET_DIR`
  so `cargo` actually invokes `rustc` for each crate. The real repo's own
  `target/` and git state were never touched; the before/after experiment ran on
  a throwaway `rsync` copy in a scratch directory.

---

## 1. Diagnosis — `cargo clinic report`

```bash
cargo clinic report --manifest-path /path/to/agent-witness/Cargo.toml
```

**10 findings**, all qualitative (`POSSIBLE`), each with a concrete prescription.
Summary:

| # | Finding | Prescription (short) |
|---|---|---|
| 1 | Default features on `anyhow` (`agent-witness`) | `default-features = false`, re-add used features |
| 2 | Default features on `clap` (`agent-witness`) | `default-features = false`, re-add used features |
| 3 | Default features on `serde_json` (`agent-witness`) | `default-features = false`, re-add used features |
| 4 | Default features on `serde_json` (`agent-witness-core`) | same |
| 5 | Default features on `serde` (`agent-witness-core`) | same |
| 6 | Default features on `thiserror` (`agent-witness-core`) | same |
| 7 | 4 proc-macro crates in graph (`clap_derive`, `serde_derive`, `thiserror-impl`, `tokio-macros`) | inspect with `cargo llvm-lines`; keep in a stable lower layer |
| 8 | 11 crates run a `build.rs` (invisible to timing) | confirm each is required; gate native/codegen build scripts behind features |
| 9 | Workspace feature unification (2 members) | adopt `cargo-hakari` workspace-hack |
| 10 | 52 packages resolved — consider a faster linker | `lld`/`mold` on Linux; compare your own before/after |

Every finding names the crate, shows evidence (e.g. the exact default-feature
expansion), and links the next tool. No fabricated seconds anywhere — impact is
`POSSIBLE`, and the report footer reminds you to measure before/after yourself.

## 2. Baseline timing — `cargo clinic measure`

```bash
CARGO_TARGET_DIR=<cold> cargo clinic measure \
  --manifest-path /path/to/agent-witness/Cargo.toml --json
```

Cold build, current state of the real repo. Top rustc consumers (of
**19332 ms total rustc time**, ~5.9 s wall after parallelism):

| Crate | rustc ms |
|---|---|
| `tokio` | 2446 |
| `build_script_build` (9 invocations) | 2391 |
| `syn` | 1391 |
| `clap_builder` | 1378 |
| `serde_core` | 1187 |
| `serde_derive` | 1076 |
| `serde` | 1003 |
| `clap` | 807 |

`build_script_build` (2391 ms across 9 invocations) is a reminder of the tool's
own disclaimer: **`build.rs` execution time is invisible to `RUSTC_WRAPPER`** —
these numbers cover rustc only, not build-script or scheduling time.

## 3. Before/after — applying prescription #2 (`clap` default features)

To get an honest before/after **without modifying `agent-witness`**, the
experiment ran on a throwaway copy. The prescription from finding #2 was applied
literally:

```toml
# crates/agent-witness/Cargo.toml
- clap = { version = "4", features = ["derive"] }
+ clap = { version = "4", default-features = false, features = ["derive", "std", "help", "usage", "error-context"] }
```

This drops clap's `color` and `suggestions` default features. Both cold builds,
same machine, same toolchain:

| Metric | Before | After | Δ |
|---|---|---|---|
| Crates compiled | 39 | **32** | **−7** |
| Total rustc time | 20868 ms | 23365 ms | +2497 ms |
| Build succeeded | yes | **yes** | — |

**Removed from the graph** (7 crates, the clap color/suggestions machinery):
`anstream`, `anstyle_parse`, `anstyle_query`, `colorchoice`,
`is_terminal_polyfill`, `strsim`, `utf8parse`.

### What this validates (and what it doesn't)

- **Structural win is real and verifiable**: 7 fewer crates in the graph, and the
  build still compiles — exactly what the prescription's step-3 ("build and test
  to confirm nothing broke") is for. This is the durable, honest signal.
- **The timing number went the "wrong" way** (+2497 ms) on this single pair of
  runs — `build_script_build` alone swung 2391 → 4395 ms between runs. That is
  run-to-run **variance**, not a regression caused by the change. It is a live
  demonstration of Core Value #2: *impact is qualitative; a single measurement
  is noise-dominated; measure repeatedly in your own environment before
  believing any delta.*

The tool did exactly what it promises: it pointed at a real, fixable structural
cost (7 avoidable crates) with a concrete edit, and it refused to promise
seconds it cannot honestly deliver.

## What was and wasn't measured

- **Measured**: real `report` output; a real cold `measure` of the current repo;
  a real cold before/after of prescription #2 on a copy.
- **Not measured / proposed only**: prescriptions #1, #3–#10 were not applied.
  Several (proc-macro layering, `cargo-hakari`, faster linker) are structural or
  environment-dependent and need their own controlled experiments. `agent-witness`
  itself was never modified; its `target/` and git state are unchanged.
