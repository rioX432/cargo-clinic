# cargo clinic report

_3 finding(s). Impact is qualitative (likely / possible). This report makes no build-time predictions; measure before/after in your own environment to confirm any change._

## 1. [POSSIBLE] Default features enabled on `featured`

**Diagnosis:** `app` depends on `featured` with default features left on; its `default` feature turns on: extra. If those features are unused, they add compile work for no benefit.

**Evidence:**

- declared in `app`'s Cargo.toml
- `featured` default expands to: extra

**Prescription:**

1. Confirm the default features are actually unused: `cargo tree -e features -i featured` shows what enables each feature.
2. In `app`'s Cargo.toml set `default-features = false` for `featured`, then re-add only the features you use: `featured = { version = "…", default-features = false, features = ["…"] }`.
3. Build and test to confirm nothing you rely on was behind a default feature.

**Next tool:** [cargo tree -e features -i](https://doc.rust-lang.org/cargo/commands/cargo-tree.html) — trace which crate or feature enables each default feature

**References:**

- [The Cargo Book — Features](https://doc.rust-lang.org/cargo/reference/features.html)

## 2. [POSSIBLE] Proc-macro crates in the build graph

**Diagnosis:** 1 proc-macro crate(s) are compiled and executed at build time. Proc-macros run during every compile of their dependents and can dominate front-end time.

**Evidence:**

- mymacro v0.1.0 <- app

**Prescription:**

1. Check whether each proc-macro is essential; some derives have lighter alternatives, or can be replaced by a hand-written impl in hot crates.
2. Measure front-end cost with `cargo llvm-lines` before removing anything — inspect, do not guess.
3. Keep proc-macro-heavy dependencies in a stable lower layer so they are not recompiled when your own code changes.

**Next tool:** [cargo-llvm-lines](https://github.com/dtolnay/cargo-llvm-lines) — reveal which generic and macro-generated code expands the most

**References:**

- [The Rust Performance Book — Compile times](https://nnethercote.github.io/perf-book/compile-times.html)

## 3. [POSSIBLE] Build scripts (build.rs) in the build graph

**Diagnosis:** 1 crate(s) run a build script. build.rs runs at build time and is INVISIBLE to RUSTC_WRAPPER timing, so its cost never appears in measured rustc numbers.

**Evidence:**

- builder-dep v0.1.0 <- app

**Prescription:**

1. Confirm each build script is required; some exist only for optional features you may not use.
2. Prefer crates that gate native or codegen build scripts behind features, and disable those features when unused.
3. For your own build scripts, cache expensive work and emit precise `cargo:rerun-if-changed` lines so they do not re-run needlessly.

**References:**

- [The Cargo Book — Build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html)

