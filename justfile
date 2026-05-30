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

# Full pre-commit gate: fmt, lint, test, supply-chain
check: fmt-check lint test deny
    @echo "All gates passed."
