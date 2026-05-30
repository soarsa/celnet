# Celnet task runner. Every recipe sources the cargo env first because the
# shell does not persist it between invocations.
set shell := ["bash", "-uc"]
export PATH := env_var('HOME') + "/.cargo/bin:" + env_var('PATH')

_cargo := 'source "$HOME/.cargo/env" && cargo'

# List recipes
default:
    @just --list

# Build the whole workspace (debug)
build:
    {{_cargo}} build --workspace --all-targets

# Build optimized release artifacts
release:
    {{_cargo}} build --workspace --release

# Run all tests via nextest (fast, deterministic)
test:
    {{_cargo}} nextest run --workspace --all-features

# Run doctests (nextest does not run them)
doctest:
    {{_cargo}} test --workspace --doc

# Lint: clippy with warnings denied
lint:
    {{_cargo}} clippy --workspace --all-targets --all-features -- -D warnings

# Format all code
fmt:
    {{_cargo}} fmt --all

# Check formatting without writing
fmt-check:
    {{_cargo}} fmt --all -- --check

# Supply-chain: advisories, bans, licenses, sources
deny:
    {{_cargo}} deny check

# Security advisories audit
audit:
    {{_cargo}} audit

# Find unused dependencies
machete:
    {{_cargo}} machete

# Coverage report (HTML + lcov)
coverage:
    {{_cargo}} llvm-cov --workspace --all-features --lcov --output-path lcov.info

# Mutation testing (test-suite quality)
mutants:
    {{_cargo}} mutants --workspace

# Crate-scoped gate: fmt + clippy(-D warnings) + nextest for ONE crate.
# Incremental — only this crate (and its already-built deps) are touched.
check-crate CRATE:
    {{_cargo}} fmt -p {{CRATE}} -- --check
    {{_cargo}} clippy -p {{CRATE}} --all-targets --all-features -- -D warnings
    {{_cargo}} nextest run -p {{CRATE}}

# Build / test a single crate (incremental).
build-crate CRATE:
    {{_cargo}} build -p {{CRATE}} --all-targets
test-crate CRATE:
    {{_cargo}} nextest run -p {{CRATE}}

# Gate ONLY the crates changed in this cycle (working tree vs HEAD + untracked).
# This is the per-iteration gate: unchanged crates are neither recompiled
# (sccache + cargo incremental) nor re-tested. Use `check` for the full
# cross-crate integration gate before committing a milestone.
check-changed:
    #!/usr/bin/env bash
    set -euo pipefail
    source "$HOME/.cargo/env"
    mapfile -t changed < <( { git diff --name-only HEAD; git ls-files --others --exclude-standard; } \
        | grep -oE '^crates/[^/]+/' | sed 's#crates/##; s#/##' | sort -u )
    if [ ${#changed[@]} -eq 0 ]; then
        echo "No changed crates — running full check."; just check; exit 0
    fi
    echo "Changed crates: ${changed[*]}"
    pflags=(); for c in "${changed[@]}"; do pflags+=( -p "$c" ); done
    cargo fmt "${pflags[@]}" -- --check
    cargo clippy "${pflags[@]}" --all-targets --all-features -- -D warnings
    cargo nextest run "${pflags[@]}"
    cargo deny check
    echo "Changed-crate gate passed: ${changed[*]}"

# Full cross-crate integration gate: fmt, lint, test, supply-chain (whole workspace).
# Run before committing a milestone; per-iteration use `check-changed` / `check-crate`.
check: fmt-check lint test deny
    @echo "All gates passed."
