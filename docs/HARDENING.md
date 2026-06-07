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

### Widened mutation gates — the other numerics crates (PC-MUT-WIDEN)

The vanilla kill-rate gate above is the gold standard. The same *enforceable*
contract — `cargo mutants` exits non-zero on any non-equivalent survivor — is
now wired for the four other safety-critical numerics crates, each with its own
audited config and a CI matrix leg:

| Crate | Config | just recipe | CI job | Status |
|-------|--------|-------------|--------|--------|
| `celnet-surface`   | `.config/mutants-surface.toml`   | `just mutants-gate-surface` (`-arbitrage` for the proven slice) | `mutation-gate-numerics` (matrix leg) | arbitrage.rs **MEASURED green locally**; crate-wide CI-run |
| `celnet-exotics`   | `.config/mutants-exotics.toml`   | `just mutants-gate-exotics`   | `mutation-gate-numerics` (matrix leg) | wired; baseline CI-run |
| `celnet-risk-cube` | `.config/mutants-risk-cube.toml` | `just mutants-gate-risk-cube` | `mutation-gate-numerics` (matrix leg) | wired; baseline CI-run |
| `celnet-xva`       | `.config/mutants-xva.toml`       | `just mutants-gate-xva`       | `mutation-gate-numerics` (matrix leg) | wired; baseline CI-run |

Each config exits non-zero on ANY non-equivalent survivor, exactly like the
vanilla gate, so a future edit that weakens any of these suites below the kill
bar fails the build. `just mutants-gate-numerics` runs all five in sequence.

**Honesty note on scope.** cargo-mutants is slow (the vanilla crate alone is ~8
minutes for 469 mutants; `celnet-surface` is ~2 300 mutants and the exotics
PDE/MC engines are larger still), so running a full local baseline on all four
crates in one session is impractical. The enforceable gates, recipes, configs,
and the CI matrix job are committed for all four; the canonical crate-wide
baselines for exotics / risk-cube / xva are **established by the CI
`mutation-gate-numerics` job** (recorded as CI-run here — no kill-rate number is
fabricated for a run that was not performed). One crate was driven to a **real,
MEASURED green** locally to prove the mechanism end-to-end:

#### `celnet-surface` — measured baseline (arbitrage module)

A crate-wide raw run (no exclusions, `aarch64-apple-darwin`, cargo-mutants
27.0.0, toolchain 1.96.0) of `celnet-surface` surfaced two distinct survivor
clusters before it was scoped:

- **`arbitrage.rs` — 33 raw survivors, all GENUINE test gaps.** The no-arbitrage
  report's `forward_call`, `implied_density` (Breeden-Litzenberger second
  difference), `forward_call_strike_slope`, and the `is_arbitrage_free`
  three-clause conjunction were only checked for *sign / bound / monotonicity*,
  never pinned to a closed form — so mutating the price/density/slope arithmetic
  (`*`↔`/`, `+`↔`-`) or the `&&`↔`||` in the predicate left the suite green.
  These were **KILLED with new value-pinning tests** (NOT excluded):
  - `forward_call_matches_black_closed_form` — pins the undiscounted forward call
    to an independent in-test `F·Φ(d₁) − K·Φ(d₂)` recomputation across 23 strikes.
  - `implied_density_matches_lognormal_closed_form` — pins the BL density to the
    exact lognormal forward-measure density `φ(d₂)/(K·σ·√t)`, fixing the
    `down − 2·mid + up` numerator and the `h·h` denominator.
  - `call_strike_slope_matches_minus_phi_d2_and_fd` — pins the strike slope to
    `−Φ(d₂)` AND to a central finite difference of `forward_call`.
  - `is_arbitrage_free_each_clause_is_load_bearing` — drives each of the three
    conjunction clauses independently (and the `−tol` sign on the lower bounds),
    killing the `&&`→`||` and `≥−tol`→`<−tol` mutations.
  - `check_slice_scaling_and_first_strike_guard` — pins the `min_butterfly ==
    min_density·h²` scaling and the `k != grid[0]` first-strike guard.

  After these tests, the scoped gate
  `cargo mutants -p celnet-surface --file '**/arbitrage.rs' --config
  .config/mutants-surface.toml` (`just mutants-gate-surface-arbitrage`) runs to
  **`79 mutants tested in 74s: 78 caught, 1 unviable, 0 missed`**, exit 0 — zero survivors. This proves the
  enforceable mechanism end-to-end on real, killed survivors.

- **`calibrate.rs` — the iterative-fit internals** (the damped Gauss-Newton /
  Levenberg-Marquardt fitters `fit_sabr`/`fit_svi`/`fit_ssvi`/`fit_essvi` +
  `gauss_newton_{2,3,4}`). The bulk of these survivors are SEED / Jacobian
  finite-difference / step-direction internals of a *converging* optimizer that
  only accepts a step when it strictly lowers the cost, so the converged
  parameters (and every smile assertion downstream) are unchanged by the
  perturbation — the same equivalent class as the vanilla solver's "initial
  guess / expansion factor" exclusions. Auditing each of these ~300 members
  individually (genuinely-equivalent vs a real tighten-the-fit-assertion gap) is
  the CI-scoped follow-on; the crate-wide gate runs in the
  `mutation-gate-numerics` CI job, which surfaces any non-converging-internal
  survivor. Until each member is individually audited and listed (with
  justification) in `.config/mutants-surface.toml`, the locally-proven slice is
  `arbitrage.rs` and the config's `exclude_re` is deliberately **empty** (no
  survivor is hidden behind an unjustified exclusion).

## 3. Coverage (region / function / line)

`cargo llvm-cov nextest` instruments the test run and reports per-file
region/function/line coverage. We track the three core pricing crates.

```
just coverage-core            # cargo llvm-cov nextest -p celnet-vanilla -p celnet-surface -p celnet-exotics --summary-only
just coverage-gate-vanilla    # vanilla, with --fail-under-lines 95 --fail-under-regions 95 (the GATE)
just coverage-gate-surface    # surface, with --fail-under-lines 90 --fail-under-regions 90 (the GATE)
```

The `celnet-vanilla` coverage GATE (`just coverage-gate-vanilla`, the CI
`mutation-coverage-gate` job) fails below **95 % line / 95 % region**; the
`celnet-surface` coverage GATE (`just coverage-gate-surface`, same CI job) fails
below **90 % line / 90 % region**. The measured baselines (below) clear both with
headroom, so a regression in either floor fails the build before the mutation gate
even runs.

### Latest run — core pricing crates

| Crate            | Region  | Function | Line    | Notes                                              |
|------------------|---------|----------|---------|----------------------------------------------------|
| `celnet-vanilla` | 98.62%  | 95.18%   | 97.83%  | `atm`/`delta`/`lib`/`premium` 100%; `solver` 97.2% region / 95.7% line (the residual is the deep-convergence safety branches + in-test bisection oracle helpers). Gate floor 95/95. |
| `celnet-surface` | 96.43%  | 96.35%   | 96.10%  | Surface-lane backlog **closed**: `market_hedge.rs` 98.9% line, `stochvol.rs` 98.0% line, `strangle.rs` 92.0% line (was 87.2/85.0/80.2). Residual uncovered lines are the `?`-error sub-regions + deep-convergence safety branches (the calibration bracket watchdog caps, the never-taken `mean ≤ 0` / downward far-wing fallbacks, measure-zero bisection exact-converge exits) — the same equivalent-class as the vanilla solver residual. Gate floor 90/90. |
| `celnet-exotics` | ~99%    | 100%     | ~99%    | PDE/MC/touch/particle engines all ≥ 98.5% line.    |

Coverage is a floor, not the goal — mutation kill-rate (§2) is the sharper
signal. The previously-lowest surface modules (`strangle.rs`, `stochvol.rs`,
`market_hedge.rs`) — the standing backlog for the surface lane (WS-C) — were
closed by targeted branch/edge tests, each asserting against an **independent
oracle** rather than padding coverage:

- `market_hedge.rs`: the `Smile`-trait re-evaluation against a non-reference
  forward/time is pinned bit-for-bit against an in-test Castagna-Mercurio
  second-approximation re-derivation; `try_new`/`new` rejection surface and the
  benchmark accessors are exercised directly.
- `stochvol.rs`: `implied_black_vol` is round-tripped against an independent
  forward-Black-call oracle (including its three no-arbitrage `None` rejections);
  the small-ν risk-neutral density is matched exactly to a hand-derived
  transformed-Gaussian closed form; the wing crossover path
  (`density_implied_vol`) is forced by a strongly-curved slice with a finite
  two-sided core band and shown to genuinely switch off the asymptotic vol.
- `strangle.rs`: the geometric **bracket-expansion** `else` block is driven on
  both step directions by two confirmed EM-like quotes (a high-vol short-tenor
  strongly-skewed quote stepping down, and a tiny-butterfly quote stepping up
  with a 17× convexity correction), each validated by the two defining
  invariants (broker-strike reprice + risk-reversal); the degenerate-via-floor,
  immediate-seed-floor, unbracketable-NoConvergence, and strike-inversion error
  paths are each pinned to their specific `CalibrationError` variant.

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
