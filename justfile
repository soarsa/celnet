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

# ---------------------------------------------------------------------------
# Hardening recipes (WS-T). See docs/HARDENING.md for the gate definitions and
# the latest recorded kill-rate / coverage numbers.
# ---------------------------------------------------------------------------

# Mutation testing on the vanilla pricing core (kill-rate; survivors = test gaps).
mutants-vanilla:
    timeout 600 {{_cargo}} mutants -p celnet-vanilla

# Mutation GATE on the vanilla pricing core. Uses `.config/mutants.toml` to
# exclude the audited set of semantically-equivalent solver-internal mutants
# (each justified there), so this exits non-zero on ANY non-equivalent survivor
# — i.e. it fails the build if a future edit weakens the suite below the kill
# bar. This is the enforceable per-crate mutation contract (see docs/HARDENING.md).
mutants-gate-vanilla:
    timeout 700 {{_cargo}} mutants -p celnet-vanilla --config .config/mutants.toml

# Coverage GATE on the vanilla pricing core: fail if region/line coverage drops
# below the committed floor (see docs/HARDENING.md). `--fail-under-lines` /
# `--fail-under-regions` make llvm-cov exit non-zero below the threshold.
coverage-gate-vanilla:
    source "$HOME/.cargo/env" && timeout 600 cargo llvm-cov nextest -p celnet-vanilla \
        --fail-under-lines 95 --fail-under-regions 95 --summary-only

# Coverage GATE on the celnet-surface calibration crate: fail if region/line
# coverage drops below the committed 90% floor (see docs/HARDENING.md §3). Closes
# the standing surface-lane backlog (strangle/stochvol/market_hedge) recorded in
# the post-completion audit (PC-SURFACE-COV); the measured baseline clears it
# with headroom (line 96.1% / region 96.4%), so a regression fails the build.
coverage-gate-surface:
    source "$HOME/.cargo/env" && timeout 600 cargo llvm-cov nextest -p celnet-surface \
        --fail-under-lines 90 --fail-under-regions 90 --summary-only

# Coverage summary for the core pricing crates (region/function/line %).
coverage-core:
    timeout 600 {{_cargo}} llvm-cov nextest -p celnet-vanilla -p celnet-surface -p celnet-exotics --summary-only

# Coverage summary for the whole workspace (region/function/line %).
coverage-summary:
    {{_cargo}} llvm-cov nextest --workspace --all-features --summary-only

# ---------------------------------------------------------------------------
# Wire-path latency-under-load proof + CI bench-regression gate (celnet-bench).
# Both binaries spin the real service edge in-process, drive sustained RFS +
# RFQ load over the loopback gRPC wire, and record the client-observed RFQ
# round-trip in an HdrHistogram. Both self-terminate (request budget + hard
# wall-clock cap); the timeouts here are belt-and-braces so a wedge fails fast.
# ---------------------------------------------------------------------------

# Published wire-path proof: large sustained load, prints the p50/p99/p99.9/p99.99
# histogram + throughput. Pass a path to also write the JSON report.
bench-wire PATH="":
    timeout 200 {{_cargo}} run --release -p celnet-bench --bin wire_load -- {{PATH}}

# Re-baseline the committed CI wire-path baseline on THIS host (gate-sized load).
bench-baseline:
    timeout 200 {{_cargo}} run --release -p celnet-bench --bin wire_load -- --ci crates/celnet-bench/baselines/wire_path.json

# CI bench-regression gate: re-measure the gate-sized wire-path load and fail if
# any RFQ percentile regresses beyond the committed baseline tolerance.
bench-gate:
    timeout 200 {{_cargo}} run --release -p celnet-bench --bin bench_gate

# Run the standalone Linux-nightly fuzz harness (NOT a workspace member).
# Requires: rustup toolchain install nightly && cargo install cargo-fuzz.
# DURATION is the per-target wall-clock budget in seconds (default 120).
fuzz-vanilla DURATION="120":
    #!/usr/bin/env bash
    set -euo pipefail
    source "$HOME/.cargo/env"
    cd fuzz
    cargo +nightly fuzz run vanilla_inputs -- -max_total_time={{DURATION}}
