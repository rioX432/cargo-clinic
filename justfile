# cargo-clinic task runner

# All checks: format, lint, build
check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo build --workspace

fmt:
    cargo fmt --all

build:
    cargo build --workspace

# Tests (nextest)
test:
    cargo nextest run --workspace

# Analyzer golden tests against planted fixture workspaces (lands with issues #1/#2)
test-analyzers:
    cargo nextest run -p cargo-clinic-core

# Self-diagnosis smoke: run the report against this workspace (lands with issue #4)
smoke-self:
    cargo run --quiet -p cargo-clinic -- report || echo "smoke-self: not implemented yet"

# Local verification gate (primary gate; CI mirrors it for the public repo).
verify: check test
    @echo "verify: all checks passed"
