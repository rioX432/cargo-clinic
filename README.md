# cargo-clinic

> Diagnose slow Rust builds and get ranked, actionable prescriptions.

`cargo clinic report` unifies scattered build diagnostics — duplicate dependency versions, proc-macro and build.rs inventory, feature bloat, optional timing data — into one prescriptive report. `cargo clinic --check` guards CI against build-time regressions with a stored baseline.

**Status**: pre-v0.1, building in public. Not launched yet — interfaces and scope will change until v0.1.

- Impact claims are qualitative (`likely` / `possible`) — no fabricated seconds
- Stable-first: timing via RUSTC_WRAPPER; nightly `--timings=json` behind an explicit experimental flag

## Install

`cargo-clinic` is a cargo subcommand: once installed, invoke it as `cargo clinic …`.

```bash
# From source
cargo install --path crates/cargo-clinic

# Prebuilt binaries (published on each version tag via cargo-dist):
#   Homebrew
brew install rioX432/tap/cargo-clinic
#   cargo-binstall (auto-detects the release artifacts)
cargo binstall cargo-clinic
#   Shell one-liner (macOS/Linux) — see the GitHub Releases page for the URL
```

Release binaries for macOS/Linux/Windows are built and published automatically
when a `v*` version tag is pushed (see `.github/workflows/release.yml`, generated
by [`dist`](https://opensource.axo.dev/cargo-dist/)). A first real diagnosis run
against a sibling project is logged in [`docs/dogfood.md`](docs/dogfood.md).
