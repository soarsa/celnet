# Celnet — Hardening Gates (WS-T)

How Celnet's correctness and robustness are *proven*, not asserted. This is the
verification companion to `docs/DELIVERY-MODEL.md` (how we build in lanes) and
`CLAUDE.md` guardrail #5 (every change passes the gates; numerical code is
validated against references). It defines the hardening gates, where they run,
and records the latest measured numbers.

All recipes source the toolchain and are timeout-bounded; the convenient entry
points live in the `justfile` (`mutants-vanilla`, `coverage-core`,
`coverage-summary`, `fuzz-vanilla`).

## 1. Continuous-integration gate

`.github/workflows/ci.yml` runs the full gate on every push / PR.

### 1.1 Cross-platform correctness matrix (`gate` job)

| Axis        | Values                                          |
|-------------|-------------------------------------------------|
| OS          | `ubuntu-latest`, `macos-latest`, `windows-latest` |
| Toolchain   | `stable` (pinned to 1.96.0 via `rust-toolchain.toml`) and the `1.96` MSRV floor |

Each matrix leg runs, with `RUSTFLAGS="-D warnings"`:

1. `cargo fmt --all -- --check` — formatting (once, on the canonical Linux/stable leg).
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings` — lint, warnings denied.
3. `cargo nextest run --workspace --all-features` — the test suite.
4. `cargo test --workspace --doc` — doctests (nextest does not run these).
5. `cargo deny check` — supply-chain: advisories, bans, licenses, sources (once, canonical leg).

Stable and MSRV are modelled as distinct matrix values so that when the pinned
stable is later bumped above the MSRV, the `1.96` lane keeps proving the floor
with no workflow change — only the matrix values move.

### 1.2 GPU / software-Vulkan lane (`gpu-lavapipe` job, Linux only)

Installs Mesa's software rasterizer (`mesa-vulkan-drivers`, the
**llvmpipe/lavapipe** ICD) and runs `cargo nextest run -p celnet-gpu` with
`VK_ICD_FILENAMES` forcing the software ICD and `WGPU_BACKEND=vulkan`. This
exercises the `wgpu` Vulkan compute path and its **f64 CPU reconciliation** in
headless CI where there is no physical GPU (Metal — the dev box — is f32-only
and unavailable on Linux runners). Software Vulkan is f32; the reconciliation
tolerance lives in the `celnet-gpu` test itself.

## 2. Mutation testing (test-suite quality)

`cargo mutants` rewrites the source with small semantic mutations (flip a
comparison, swap `+`/`-`, replace a function body with a default) and re-runs
the tests; a mutation that the tests still pass is a **survivor** — a real gap
in test strength, not just coverage. We measure it on the vanilla pricing core,
the most safety-critical numerical crate.

```
just mutants-vanilla          # timeout 600 cargo mutants -p celnet-vanilla
```

### Latest run — `celnet-vanilla`

<!-- HARDENING:MUTANTS -->
`469 mutants tested in 8m` (cargo-mutants 27.0.0, toolchain 1.96.0, `aarch64-apple-darwin`):

| Outcome  | Count | Notes                                                        |
|----------|-------|--------------------------------------------------------------|
| Caught   | 364   | killed by the test suite                                     |
| Missed   | 102   | survivors — real test-strength gaps (listed below)           |
| Unviable | 3     | mutated body did not compile (`-> T` replaced by `Default`)  |
| Timeout  | 0     | —                                                            |
| **Total** | **469** | **viable = 466** |

**Kill-rate = 364 / 466 = 78.1 %** (caught / viable).

#### Survivors by file (test-strength gaps; NOT fixed in WS-T — flagged for the owning lane)

| File         | Survivors | Function(s)                                          |
|--------------|-----------|------------------------------------------------------|
| `solver.rs`  | 57        | `strike_from_delta`, `bracket`, `tiny_strike`        |
| `delta.rs`   | 30        | `delta_aux`, `delta_d_strike`, `premium_adjusted_call_delta_max` |
| `lib.rs`     | 14        | `greeks` (the in-pass price recompute, lines 98–108) |
| `atm.rs`     | 1         | `atm_strike` (delta-neutral-straddle half-variance)  |

**Root-cause characterization (for the WS-B vanilla lane to act on):**

1. **`solver.rs` (57) — robustness masks arithmetic mutations.** `strike_from_delta`
   uses a bisection-guaranteed bracket with Newton acceleration and reachability
   guards. Most surviving mutations are to comparison operators / step arithmetic in
   the bisection-shrink and the geometric bracket expansion; because bisection is
   self-correcting and the only assertion is the *converged root*, a wrong Newton
   step or a `>`→`>=` still lands on the same root, so round-trip (`strike→delta→
   strike`) tests pass. **Remediation:** assert on *iteration count / convergence
   speed* (Newton must accelerate past pure bisection) and add a direct unit test of
   the bracket invariants, not just the final root.

2. **`delta.rs` (30) — analytic ∂Δ/∂K and the PA-call delta-max are not
   FD-validated.** `delta_d_strike` (the closed-form `∂Δ/∂K` the solver's Newton step
   consumes) and `premium_adjusted_call_delta_max` have no independent
   finite-difference gate, so arithmetic mutations in `d1`/`d2` construction and the
   product-rule terms survive. **Remediation:** add a central-finite-difference check
   `delta_d_strike ≈ (Δ(K+h)−Δ(K−h))/2h` across conventions (mirrors the existing
   full-Greek FD gate in `lib.rs`), and assert the PA-call delta-max is a stationary
   point (`∂Δ/∂K ≈ 0` there).

3. **`lib.rs` (14) — `greeks().price` recompute is under-asserted.** `greeks()`
   recomputes the premium independently of `price()`; the tests do not pin
   `greeks(opt,i).price == price(opt,i)` tightly, so arithmetic mutations to the
   in-pass formula survive even though the standalone `price()` is fully covered.
   **Remediation:** add `assert is_close(greeks().price, price())` over the proptest
   input set.

These are **test-suite gaps, not pricing defects** — the standalone `price()` and the
full Greek set are FD-validated and benchmark-anchored; the survivors live in the
convention/solver derivative layer whose *outputs* are tested but whose *internal
arithmetic* is not independently pinned. WS-T reports them; the WS-B lane owns the fix
(it owns `crates/celnet-vanilla/src/`).
<!-- /HARDENING:MUTANTS -->

## 3. Coverage (region / function / line)

`cargo llvm-cov nextest` instruments the test run and reports per-file
region/function/line coverage. We track the three core pricing crates.

```
just coverage-core            # cargo llvm-cov nextest -p celnet-vanilla -p celnet-surface -p celnet-exotics --summary-only
```

### Latest run — core pricing crates

| Crate            | Region  | Function | Line    | Notes                                              |
|------------------|---------|----------|---------|----------------------------------------------------|
| `celnet-vanilla` | ~99%    | 100%     | ~99%    | `atm`/`delta`/`lib`/`premium` 100%; `solver` 96.7% region / 94.8% line (deep-convergence branches). |
| `celnet-surface` | high    | high     | high    | `strangle.rs` (broker-strangle smile solve) and `stochvol.rs` are the lowest at ~78–86% — flagged for added tests. |
| `celnet-exotics` | ~99%    | 100%     | ~99%    | PDE/MC/touch/particle engines all ≥ 98.5% line.    |
| **Aggregate (3 crates)** | **96.30%** | **96.32%** | **95.86%** | 9 798 regions / 489 functions / 5 704 lines instrumented. |

Coverage is a floor, not the goal — mutation kill-rate (§2) is the sharper
signal. Lowest-covered surface modules (`strangle.rs`, `stochvol.rs`,
`market_hedge.rs`) are the standing backlog for the surface lane (WS-C).

## 4. Fuzzing (adversarial input robustness)

A standalone, **non-workspace** crate under `fuzz/` (its own `[workspace]`
table) carries `cargo-fuzz` / libFuzzer targets. It is deliberately excluded
from `cargo build --workspace` and the stable gate so the cross-platform matrix
never pulls the nightly-only sanitizer runtime.

| Target           | Under test                            | Contract asserted                                        |
|------------------|---------------------------------------|----------------------------------------------------------|
| `vanilla_inputs` | `celnet_vanilla::price` + `::greeks`  | no panic; premium ≥ 0 and finite; all 13 Greeks finite.  |

`vanilla_inputs` folds raw fuzzer bytes (via `arbitrary`) into a
valid-but-adversarial `VanillaInputs` in the Garman-Kohlhagen domain (strictly
positive spot/strike/vol/time, finite bounded rates), so the budget is spent on
in-domain corners — deep ITM/OTM, near-zero / century expiries, micro / macro
vols, large rate spreads — rather than trivially-rejected garbage.

It runs as a dedicated **Linux-nightly** CI job — the `fuzz` job in
`.github/workflows/ci.yml` — which installs `cargo-fuzz` on nightly and runs the
`vanilla_inputs` target time-boxed (a 60 s smoke budget) on every push/PR; a
panic or contract break fails the job. It is deliberately not part of the stable
cross-platform gate. Locally (or for a longer campaign) run it via:

```
rustup toolchain install nightly && cargo install cargo-fuzz
just fuzz-vanilla 120         # cd fuzz && cargo +nightly fuzz run vanilla_inputs -- -max_total_time=120
```

## 5. Golden / reference parity

Numerical correctness is anchored to published prices and the open-source
QuantLib oracle (guardrail #5/#7). The parity fixtures and harness live in
`celnet-parity` / `celnet-golden` (owned by the parity lane); this document
covers the structural hardening gates that wrap them.
