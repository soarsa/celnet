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

# ---------------------------------------------------------------------------
# Tiered gates (docs/PARALLEL-SESSIONS.md §4.2): T0 per edit, ONE T1 per
# accumulated lane batch, T2 once per push milestone — never bespoke full gates
# per fix-iteration. T1/T2 run through the resumable runner
# (tools/gate-runner.sh → .gate-ledger.jsonl): each step's PASS/FAIL + literal
# output line + REAL exit code is journaled, keyed on HEAD + a dirty-tree hash,
# so a killed/spend-walled gate resumes from the last green step.
# ---------------------------------------------------------------------------

# T0 — per-edit (seconds): does the crate still compile? Iterate on T0 only;
# no test/clippy in the edit loop.
t0 CRATE:
    {{_cargo}} check -p {{CRATE}}

# T1 — per-lane-batch: ONE invocation settles the whole accumulated batch.
# Pass the union of every crate changed since the last green T1; with no args
# it derives the set itself (diff vs the last green T1's HEAD from the gate
# ledger, else vs HEAD, plus untracked files). A SINGLE multi-`-p` cargo test
# avoids feature-unification rebuilds between per-crate invocations. Plain
# `cargo test` (NOT nextest: its orchestration wedges under load on this M4
# and a parallel session may `pkill -f nextest`).
t1 *CRATES:
    #!/usr/bin/env bash
    set -euo pipefail
    source "$HOME/.cargo/env"
    crates=( {{CRATES}} )
    if [ ${#crates[@]} -eq 0 ]; then
        base=$(bash tools/gate-runner.sh last-green t1 || true)
        diffbase=HEAD
        if [ -n "$base" ] && git rev-parse --verify --quiet "$base^{commit}" >/dev/null; then
            diffbase=$base
        fi
        mapfile -t crates < <( { git diff --name-only "$diffbase" --; git ls-files --others --exclude-standard; } \
            | grep -oE '^crates/[^/]+/' | sed 's#crates/##; s#/##' | sort -u )
    fi
    if [ ${#crates[@]} -eq 0 ]; then
        echo "t1: no crate changed since the last green T1 (base $diffbase) — nothing to gate."
        echo "    (non-Rust changes gate via their own suites; landings gate via t2;"
        echo "     pass crates explicitly to force: just t1 <crate> …)"
        exit 0
    fi
    echo "t1 batch: ${crates[*]}"
    pflags=""
    for c in "${crates[@]}"; do pflags="$pflags -p $c"; done
    # Test execution is journaled PER CRATE (after one union --no-run build) so a
    # single failing crate re-runs ONLY itself on resume — the monolithic test
    # step re-ran all 9 crates per fix-iteration before this (4x T1 churn,
    # journaled 2026-06-11). The union build keeps feature unification
    # consistent; per-crate runs are warm (observed: seconds) because every
    # crate uses the workspace dep registry with uniform features.
    steps=( \
        "fmt::source \"\$HOME/.cargo/env\" && cargo fmt$pflags -- --check" \
        "clippy::source \"\$HOME/.cargo/env\" && cargo clippy$pflags --all-targets --all-features -- -D warnings" \
        "build-tests::source \"\$HOME/.cargo/env\" && cargo test$pflags --no-run" )
    for c in "${crates[@]}"; do
        steps+=( "test-$c::source \"\$HOME/.cargo/env\" && cargo test -p $c" )
    done
    exec bash tools/gate-runner.sh t1 "${steps[@]}"

# T2 — landing-only: the full `check` gate set (same steps, decomposed so the
# resumable ledger can skip the already-green ones after a kill) + the GUI/Excel
# typecheck + unit vitest + the live e2e suites. Run ONCE per push milestone,
# never per fix-iteration. demo_edge is pre-built first (env lesson: the e2e
# ready-timeout silently covers a cold `cargo run --example` build). gui-touching
# work ⇒ the Playwright+axe suite is NOT skippable (deferred-e2e lesson: a deferred
# suite is a defect reservoir). The web typecheck + unit suites are gated here too:
# they are NOT covered by the e2e (which exercises a live build, not `tsc --noEmit`
# nor the unit vitest), and a unit/type regression that the e2e can't see — e.g. a
# corpus family added without its legacy-parity case — must fail the landing gate,
# not lurk. Plain `cargo test` — see t1.
t2:
    #!/usr/bin/env bash
    set -euo pipefail
    exec bash tools/gate-runner.sh t2 \
        "workspace-deps::just workspace-deps" \
        "verification-coverage::node tools/check-verification-coverage.mjs" \
        "fmt::source \"\$HOME/.cargo/env\" && cargo fmt --all -- --check" \
        "clippy::source \"\$HOME/.cargo/env\" && cargo clippy --workspace --all-targets --all-features -- -D warnings" \
        "build-tests::source \"\$HOME/.cargo/env\" && cargo test --workspace --all-features --no-run" \
        "test-libs::source \"\$HOME/.cargo/env\" && cargo test --workspace --all-features --lib --bins" \
        "test-integration::source \"\$HOME/.cargo/env\" && cargo test --workspace --all-features --test '*'" \
        "test-docs::source \"\$HOME/.cargo/env\" && cargo test --workspace --all-features --doc" \
        "deny::source \"\$HOME/.cargo/env\" && cargo deny check" \
        "gui-typecheck::npm --prefix gui run typecheck" \
        "gui-unit::npm --prefix gui run test" \
        "excel-typecheck::npm --prefix excel run typecheck" \
        "excel-unit::npm --prefix excel run test" \
        "build-edge::source \"\$HOME/.cargo/env\" && cargo build -p celnet-server --example demo_edge" \
        "gui-e2e::npm --prefix gui run e2e:install && npm --prefix gui run e2e" \
        "excel-e2e::npm --prefix excel run test:e2e"

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
# Plain-cargo runner (nextest-kill-proof, the W6 §5 protocol); the proptest
# suite in strangle.rs must fail fast under a killing mutant, not shrink.
mutants-gate-surface:
    PROPTEST_MAX_SHRINK_ITERS=0 timeout 7200 {{_cargo}} mutants -p celnet-surface --test-tool=cargo --jobs 3 \
        --minimum-test-timeout=120 --config .config/mutants-surface.toml

# Surface arbitrage module ONLY — the locally-proven-green slice (PC-MUT-WIDEN).
# Drives the gate over src/arbitrage.rs (no-arbitrage report numerics) to zero
# survivors, exercising the enforceable mechanism end-to-end quickly. The full
# crate-wide gate (mutants-gate-surface) is CI-run.
mutants-gate-surface-arbitrage:
    PROPTEST_MAX_SHRINK_ITERS=0 timeout 600 {{_cargo}} mutants -p celnet-surface --test-tool=cargo --jobs 3 \
        --minimum-test-timeout=120 --file '**/arbitrage.rs' --config .config/mutants-surface.toml

# Exotics pricing crate (digitals/barriers/Asian/TARF/... + PDE/MC/particle/LSV).
mutants-gate-exotics:
    timeout 5400 {{_cargo}} mutants -p celnet-exotics --config .config/mutants-exotics.toml

# Risk-cube crate (additive roll-up + non-additive VaR/ES + FRTB-SA capital).
# Plain-cargo runner + jobs 2 (the joint-window bounded-compute cap) + a generous
# per-mutant test-timeout floor (the macOS first-launch stall guard) — W6 rigor
# protocol, measured green locally (see HARDENING.md s2).
mutants-gate-risk-cube:
    timeout 7200 {{_cargo}} mutants -p celnet-risk-cube --test-tool=cargo --jobs 2 \
        --minimum-test-timeout=120 --config .config/mutants-celnet-risk-cube.toml

# XVA crate (CVA/DVA/FVA over exposure + survival curve + netting).
# Plain-cargo runner + jobs 3 + the 240s per-mutant floor (macOS first-launch
# stall guard) — W6 rigor protocol, measured green locally (see HARDENING.md s2).
mutants-gate-xva:
    timeout 1800 {{_cargo}} mutants -p celnet-xva --test-tool=cargo --jobs 3 \
        --minimum-test-timeout=240 --config .config/mutants-xva.toml

# Mutation GATE on the low-discrepancy sequence crate (zero non-equivalent survivors).
# `--minimum-test-timeout=240`: the auto-set 20s floor misclassifies mutants as
# timeouts when macOS stalls the first launch of freshly linked test binaries
# under build churn (the suite itself runs in ~1s; the crate has no
# value-dependent loops that could genuinely hang).
mutants-gate-qmc:
    timeout 3600 {{_cargo}} mutants -p celnet-qmc --test-tool=cargo --jobs 3 \
        --minimum-test-timeout=240 --config .config/mutants-qmc.toml

# All numerics mutation gates in sequence (vanilla + the five widened crates).
mutants-gate-numerics: mutants-gate-vanilla mutants-gate-surface mutants-gate-exotics mutants-gate-risk-cube mutants-gate-xva mutants-gate-qmc
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

# Mutation GATE on the durable journal (per-record sync-word framing + CRC-32 +
# recovery state machine — incl. the intact-sync-word + failing-CRC ->
# CorruptInterior discrimination — + atomic compaction). Plain `cargo test` runner
# (NOT nextest: a concurrent session may `pkill nextest`, and the gate must be
# reproducible independent of the workspace nextest profile) so the proptest
# adversarial-bytes recovery test + the unit/recovery/compaction tests drive every
# mutant; `--jobs 3` bounds wall-time on the M4 and stays courteous to a parallel
# session. The INDEPENDENT oracle is the running-sum replay machine + the
# kill-restart byte-identity test (they grade recovered STATE, not the framing
# code). Zero non-equivalent survivors. See docs/HARDENING.md.
mutants-gate-journal:
    timeout 1800 {{_cargo}} mutants -p celnet-journal --test-tool=cargo --jobs 3 \
        --config .config/mutants-journal.toml

# All infra-crate mutation gates (the SPMC fan-out ring, the durable journal, the
# fleet router, and the replicated event log) in sequence — the crash-recovery /
# fan-out / scale-out / consensus substrate, separate from the numerics-only
# `mutants-gate-numerics` aggregate.
mutants-gate-infra: mutants-gate-fanout mutants-gate-journal mutants-gate-celnet-router mutants-gate-celnet-replog
    @echo "All infra mutation gates passed."

# Mutation GATE on the fleet router (HRW rendezvous assignment + argmax tie-break,
# splitmix64 mixer / digest, hot-standby failover + HRW fallback, membership
# validation, per-replica inflight cap). Plain `cargo test` runner (NOT nextest);
# `--jobs 3` bounds wall-time on the M4 and stays courteous to a parallel session.
# The INDEPENDENT oracle is the code-disjoint splitmix64/digest + brute-force
# argmax in tests/router_mutation.rs. Zero non-equivalent survivors. See
# docs/HARDENING.md.
#
# `PROPTEST_MAX_SHRINK_ITERS=0`: a hash-collapse mutant (e.g. `mix64 -> 0`) makes
# the balance/reshuffle proptests fail — correctly CAUGHT — but proptest's default
# shrinking then re-runs the 6000–8000-key case hundreds of times, which under
# `--jobs 3` contention can exceed any per-mutant timeout and be MIS-scored as a
# spurious Timeout. Disabling shrink iters makes a failing case report
# immediately, so every such mutant is deterministically scored CAUGHT.
# `PROPTEST_DISABLE_FAILURE_PERSISTENCE=1` stops the gate writing transient
# `.proptest-regressions` artifacts from the mutated runs. `--minimum-test-timeout`
# is a generous floor for the remaining (non-shrinking) cases.
mutants-gate-celnet-router:
    timeout 1500 env PROPTEST_MAX_SHRINK_ITERS=0 PROPTEST_DISABLE_FAILURE_PERSISTENCE=1 \
        {{_cargo}} mutants -p celnet-router --test-tool=cargo --jobs 3 \
        --minimum-test-timeout=60 --config .config/mutants-celnet-router.toml

# Mutation GATE on the leader-replicated, deterministic-replay event log
# (`celnet-replog`): the index-addressed durable `Log` over the journal (append /
# reconcile / conflicting-tail truncate / prefix discard / snapshot install +
# absolute<->physical index translation), the Raft election state machine
# (vote / commit-quorum / RPC receivers / step-down / failover), the wire codec
# over real loopback sockets, the persisted hard-state, the compaction snapshot
# codec, and the deterministic `u64 -> f64` priced-book state machine. Plain
# `cargo test` runner (NOT nextest: a concurrent session may `pkill -f nextest`,
# and the gate must be reproducible independent of the workspace nextest profile);
# `--jobs 3` bounds wall-time on the M4 and stays courteous to a parallel session.
# The INDEPENDENT oracle is the running priced-book replay (`gate_a`/`gate_d` in
# tests/replication.rs re-apply the committed deltas to a fresh BookState and
# compare by `f64::to_bits` — code-disjoint from the log) + the byte-identical
# committed-log assertion. Zero non-equivalent survivors. See docs/HARDENING.md.
mutants-gate-celnet-replog:
    timeout 2400 {{_cargo}} mutants -p celnet-replog --test-tool=cargo --jobs 3 \
        --config .config/mutants-celnet-replog.toml

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
