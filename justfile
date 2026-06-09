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
check: workspace-deps verification-coverage fmt-check lint test deny
    @echo "All gates passed."

# Verification-contract coverage lint (docs/VERIFICATION-CONTRACT.md gates (b)/(a)+(c)):
# parse the `oneof product` arms in crates/celnet-proto/proto/celnet.proto and assert
# EVERY product family has BOTH a frozen cross-client golden vector
# (crates/celnet-golden/vectors/<family>.json) AND an independent-oracle celnet-parity
# row (crates/celnet-parity/tests/). Exits non-zero listing any arm missing either, so
# a new proto product arm cannot ship without its vector + parity row. Never weakened.
verification-coverage:
    #!/usr/bin/env bash
    set -euo pipefail
    node tools/check-verification-coverage.mjs

# Lint: NO internal crate may depend on another by a relative `path = "../celnet-*"`.
# All internal deps go through the central [workspace.dependencies] registry in the
# root Cargo.toml (`celnet-x.workspace = true`). This keeps the root manifest the sole
# shared file so parallel "lane" agents never collide on a member manifest, and makes
# adding a new crate a single-line registry change. Exits non-zero listing offenders.
workspace-deps:
    #!/usr/bin/env bash
    set -euo pipefail
    offenders=$(grep -rn 'path = "\.\./celnet-' crates/*/Cargo.toml || true)
    if [ -n "$offenders" ]; then
        echo "ERROR: internal crates must use the [workspace.dependencies] registry"
        echo "       (celnet-x.workspace = true), not a relative path-dep:"
        echo "$offenders"
        exit 1
    fi
    echo "workspace-deps: OK — no internal path-deps outside the registry."

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

# Mutation GATEs on the other safety-critical numerics crates (PC-MUT-WIDEN).
# Each uses its own `.config/mutants-<crate>.toml` (an audited, justified
# equivalence/exclude set — see that file + docs/HARDENING.md s2) and exits
# non-zero on ANY non-equivalent survivor, exactly like the vanilla gate. These
# crates are large (surface ~2.3k mutants, exotics PDE/MC even larger), so the
# canonical baselines are CI-run by the `mutation-gate-numerics` job; the local
# recipes are for re-baselining / debugging a single crate. The timeout is a
# belt-and-braces wedge guard (cargo-mutants also self-times-out per mutant).

# Surface calibration crate (VV/SABR/SVI/SSVI/eSSVI + arbitrage + strangle).
mutants-gate-surface:
    timeout 3600 {{_cargo}} mutants -p celnet-surface --config .config/mutants-surface.toml

# Surface arbitrage module ONLY — the locally-proven-green slice (PC-MUT-WIDEN).
# Drives the gate over src/arbitrage.rs (no-arbitrage report numerics) to zero
# survivors, exercising the enforceable mechanism end-to-end quickly. The full
# crate-wide gate (mutants-gate-surface) is CI-run.
mutants-gate-surface-arbitrage:
    timeout 600 {{_cargo}} mutants -p celnet-surface --file '**/arbitrage.rs' --config .config/mutants-surface.toml

# Exotics pricing crate (digitals/barriers/Asian/TARF/... + PDE/MC/particle/LSV).
mutants-gate-exotics:
    timeout 5400 {{_cargo}} mutants -p celnet-exotics --config .config/mutants-exotics.toml

# Risk-cube crate (additive roll-up + non-additive VaR/ES + FRTB-SA capital).
mutants-gate-risk-cube:
    timeout 3600 {{_cargo}} mutants -p celnet-risk-cube --config .config/mutants-risk-cube.toml

# XVA crate (CVA/DVA/FVA over exposure + survival curve + netting).
mutants-gate-xva:
    timeout 1800 {{_cargo}} mutants -p celnet-xva --config .config/mutants-xva.toml

# All numerics mutation gates in sequence (vanilla + the four widened crates).
mutants-gate-numerics: mutants-gate-vanilla mutants-gate-surface mutants-gate-exotics mutants-gate-risk-cube mutants-gate-xva
    @echo "All numerics mutation gates passed."

# ---------------------------------------------------------------------------
# Infra rigor gates (W6): the loom model-check of the fan-out seqlock ring + the
# infra-crate mutation gates. See docs/HARDENING.md (W6 section).
# ---------------------------------------------------------------------------

# Loom model-check of the SPMC seqlock ring (relaxed-memory interleaving search).
# Built ONLY under `--cfg loom` so the std hot path is byte-for-byte unaffected
# (loom never enters a release build; it is a `[target.'cfg(loom)'.dependencies]`
# dev/cfg-only dep). Bounded-preemption exploration keeps the search inside CI
# time while exhaustively covering the producer/consumer interleavings the
# two-stamp + Acquire-fence torn-read protocol must reject (see ring.rs
# §"Seqlock reader barrier" and crates/celnet-fanout/src/mem.rs).
loom-fanout:
    timeout 1800 env RUSTFLAGS="--cfg loom" LOOM_MAX_PREEMPTIONS=3 \
        {{_cargo}} test -p celnet-fanout --test loom_seqlock --release

# Mutation GATE on the SPMC fan-out ring (seqlock publish + torn-read protocol +
# conflation/skip accounting). Plain `cargo test` runner (NOT nextest: a
# concurrent session may `pkill nextest`, and the gate must be reproducible
# independent of the workspace nextest profile); `--jobs 3` bounds wall-time on
# the M4 and stays courteous to a parallel session. Zero non-equivalent
# survivors. See docs/HARDENING.md.
mutants-gate-fanout:
    timeout 1200 {{_cargo}} mutants -p celnet-fanout --test-tool=cargo --jobs 3 \
        --config .config/mutants-fanout.toml

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

# Re-render ALL capability-doc figures (committed, reproducible) from their authored
# HTML in docs/assets/celnet-capabilities/_src/ via headless Chromium (Playwright).
# Reads diagram-meta.json for each figure's exact w×h and screenshots the .canvas
# element to docs/assets/celnet-capabilities/<figname>.png. Uses the Playwright
# vendored under gui/node_modules; ensures the Chromium binary is installed first.
# Pass an optional figure name to render just one (e.g. `just render-figures fig-04-surface-pipeline`).
render-figures FIG="":
    #!/usr/bin/env bash
    set -euo pipefail
    npx --prefix gui playwright install chromium
    node tools/render-capability-figures.mjs {{FIG}}

# Verify every relative link, figure/img source, and in-document #anchor across the
# capability showcase corpus (docs/CELNET-CAPABILITIES.md, docs/celnet-capabilities/*.md,
# docs/celnet-capabilities.html) actually resolves on disk. Exits non-zero listing any
# broken reference. No network calls — external http(s) links are out of scope.
check-docs:
    #!/usr/bin/env bash
    set -euo pipefail
    source "$HOME/.cargo/env"
    node tools/check-doc-links.mjs

# Verify docs/celnet-capabilities.html has NO horizontal overflow, no element wider
# than the viewport, and a vertically-reachable footer across a viewport sweep
# (1920×1080 → 375×667) in headless Chromium. Reuses the Playwright vendored under
# gui/node_modules; ensures the Chromium binary is installed first. Exits non-zero
# listing any failing viewport/element.
check-html-responsive:
    #!/usr/bin/env bash
    set -euo pipefail
    source "$HOME/.cargo/env"
    npx --prefix gui playwright install chromium
    node tools/check-html-responsive.mjs

# Build the fully self-contained capabilities document: inline every figure/screenshot as a
# base64 data URI into docs/celnet-capabilities.standalone.html (gitignored, ~34 MiB). The
# committed docs/celnet-capabilities.html keeps relative refs; this is the portable single
# file + the PDF source. See docs/celnet-capabilities.html for the rendered relative-ref view.
embed-capabilities:
    #!/usr/bin/env bash
    set -euo pipefail
    node tools/embed-capabilities-assets.mjs

# Render the professionally-styled capabilities PDF (embeds assets → standalone → print-to-PDF
# off the @media print stylesheet). Output: docs/celnet-capabilities.pdf.
capabilities-pdf:
    #!/usr/bin/env bash
    set -euo pipefail
    npx --prefix gui playwright install chromium
    node tools/embed-capabilities-assets.mjs
    node tools/render-capabilities-pdf.mjs
