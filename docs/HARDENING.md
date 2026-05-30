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
just mutants-vanilla          # timeout 600 cargo mutants -p celnet-vanilla (raw, no exclusions)
just mutants-gate-vanilla     # cargo mutants … --config .config/mutants.toml (the GATE)
```

### Latest run — `celnet-vanilla`

<!-- HARDENING:MUTANTS -->
`469 mutants tested in 8m` (cargo-mutants 27.0.0, toolchain 1.96.0, `aarch64-apple-darwin`),
raw (no exclusions):

| Outcome  | Count | Notes                                                        |
|----------|-------|--------------------------------------------------------------|
| Caught   | 429   | killed by the test suite (test fails / panics)               |
| Timeout  | 2     | killed by non-termination (a corrupted bracket-expansion counter spins; the mutation harness times out — see below) |
| Missed   | 35    | survivors — all **semantically-equivalent** solver internals (audited below) |
| Unviable | 3     | mutated body did not compile (`-> T` replaced by `Default`)  |
| **Total** | **469** | **viable = 466** |

**Kill-rate (counting the 35 equivalents as un-killable) = (429 + 2) / (466 − 35) =
431 / 431 = 100 %** of the *non-equivalent* viable mutants. As a raw fraction of all
viable mutants, `(429 + 2) / 466 = 92.5 %`; the residual 7.5 % is the equivalent set.

This is up from the prior `78.1 %` (364 caught / 102 missed) recorded for the same
crate: the 67 newly-killed mutants were closed by the targeted tests added to
`solver.rs` (`solver_residual_is_at_solver_tolerance_on_wide_grid`,
`every_reachable_target_solves_without_failure`,
`premium_adjusted_call_reachability_boundary_is_exact`, `zero_delta_is_not_wrong_sign`,
`out_of_range_unadjusted_target_is_unreachable`) plus the existing `delta.rs` /
`lib.rs` FD and price-recompute gates from the earlier remediation wave.

#### The mutation GATE

`just mutants-gate-vanilla` (and the CI `mutation-coverage-gate` job) run cargo-mutants
under `.config/mutants.toml`, which **excludes the 35 audited equivalent mutants** and
nothing else. cargo-mutants exits non-zero on *any* survivor, so the gate enforces
**zero non-equivalent survivors** — a future edit that weakens the suite below the kill
bar fails the build. Verified clean: `434 mutants tested … 429 caught, 3 unviable, 2
timeouts, 0 missed` (exit 0).

#### Why the 35 survivors are genuine equivalent mutants

All 35 live in the strike↔delta root-finder (`strike_from_delta`, `bracket`,
`tiny_strike`) and the PA-call peak solver. The solver is a **bracketing bisection with
Newton acceleration**, guarded so the bracket is never left, with a self-correcting
geometric bracket-expansion and several independent convergence exits. That design makes
a class of internal perturbations provably unobservable in the *result*:

1. **Initial guess / expansion-factor / lower-bound value** (`strike_from_delta:88`,
   `bracket:169/202/203`, `tiny_strike:219/220`). The converged strike is independent of
   where iteration starts or which finite step the expansion takes — bisection converges
   from any straddling bracket, and the geometric expansion re-derives a straddle from
   any positive lower bound. Pinned indirectly: the bound's *contract* (positive, finite,
   below the forward and below the deepest wing) is still asserted by
   `tiny_strike_is_a_valid_lower_bound`.
2. **Sign-identical comparisons** (`strike_from_delta:95` `glo*gk`↔`glo/gk`;
   `bracket:196` straddle test). The branch returns at `gk==0`, so the sign of the
   product equals the sign of the quotient — the same branch is taken.
3. **Measure-zero boundary flips** (`:108/:114/:162/:172/:174/:196/:198/:202`
   `>`↔`>=`, `<`↔`<=`): differ only at an exact float-equality boundary never produced by
   the iteration.
4. **Never-hit safety code** (`strike_from_delta:120/125/126` width early-return +
   post-loop fallback; `bracket:173/174/198` watchdog *caps*): these execute only when
   the primary residual exit fails to converge in 200 iterations, which never happens for
   a well-posed bracketed problem. **Counter-example that is NOT equivalent:** the
   watchdog *advance* `iters += 1` at `bracket:197` — mutating it to `-=`/`*=` stops the
   counter from ever reaching the cap, so an *unreachable* unadjusted target spins
   forever. Those two mutants are CAUGHT (timeout), pinned by
   `out_of_range_unadjusted_target_is_unreachable`.
5. **PA-peak bisection tie-break** (`delta.rs:140` `g(mid) > 0`↔`>=`): differs only when
   a bisection midpoint lands exactly on the root, which f64 bisection never does.

Each was confirmed against a 200 000-case differential oracle (an independent
re-implementation of the solver, randomized over a market domain wider than the test
grid): every excluded mutant reproduces the unmutated result bit-for-bit on both the
solved strike and the reachable/unreachable verdict. The full list with per-line
justification is the `exclude_re` table in `.config/mutants.toml`.
<!-- /HARDENING:MUTANTS -->

## 3. Coverage (region / function / line)

`cargo llvm-cov nextest` instruments the test run and reports per-file
region/function/line coverage. We track the three core pricing crates.

```
just coverage-core            # cargo llvm-cov nextest -p celnet-vanilla -p celnet-surface -p celnet-exotics --summary-only
just coverage-gate-vanilla    # same, with --fail-under-lines 95 --fail-under-regions 95 (the GATE)
```

The `celnet-vanilla` coverage GATE (`just coverage-gate-vanilla`, the CI
`mutation-coverage-gate` job) fails below **95 % line / 95 % region**; the measured
baseline (below) clears it with headroom, so a regression in either floor fails the
build before the mutation gate even runs.

### Latest run — core pricing crates

| Crate            | Region  | Function | Line    | Notes                                              |
|------------------|---------|----------|---------|----------------------------------------------------|
| `celnet-vanilla` | 98.62%  | 95.18%   | 97.83%  | `atm`/`delta`/`lib`/`premium` 100%; `solver` 97.2% region / 95.7% line (the residual is the deep-convergence safety branches + in-test bisection oracle helpers). Gate floor 95/95. |
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
