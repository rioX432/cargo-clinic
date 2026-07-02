# Test Strategy

- **Planted fixture workspaces** (issues #1/#2): mini cargo workspaces with known problems (duplicate versions, heavy proc-macros, build.rs, unpruned default-features). Analyzer output is snapshot-tested against them.
- **Cross-check**: duplicate detection must agree with `cargo tree -d`; feature reasoning mirrors `cargo tree -e features -i`.
- **Timing**: RUSTC_WRAPPER integration test on a small fixture build; wrapper-conflict (sccache) detection test.
- **Output lint**: no quantitative prediction strings (ADR-0002); every finding has a prescription.
- **Baseline**: round-trip + planted-regression failure test; environment-change false-alarm suppression.
- Run everything via `just verify`.
