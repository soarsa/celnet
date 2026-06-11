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
| `celnet-risk-cube` | `.config/mutants-celnet-risk-cube.toml` | `just mutants-gate-risk-cube` | `mutation-gate-numerics` (matrix leg) | **MEASURED green locally** (W6 — see below) |
| `celnet-xva`       | `.config/mutants-xva.toml`       | `just mutants-gate-xva`       | `mutation-gate-numerics` (matrix leg) | **MEASURED green locally** (W6 — see below) |
| `celnet-qmc`       | `.config/mutants-qmc.toml`       | `just mutants-gate-qmc`       | `mutation-gate-numerics` (matrix leg) | **MEASURED green locally** (W6 — see below) |

Each config exits non-zero on ANY non-equivalent survivor, exactly like the
vanilla gate, so a future edit that weakens any of these suites below the kill
bar fails the build. `just mutants-gate-numerics` runs all six in sequence.

**Honesty note on scope.** cargo-mutants is slow (the vanilla crate alone is ~8
minutes for 469 mutants; `celnet-surface` is ~2 300 mutants and the exotics
PDE/MC engines are larger still), so running a full local baseline on all four
crates in one session is impractical. The enforceable gates, recipes, configs,
and the CI matrix job are committed for all four; the canonical crate-wide
baseline for exotics is **established by the CI `mutation-gate-numerics` job**
(recorded as CI-run here — no kill-rate number is fabricated for a run that was
not performed); risk-cube, xva and qmc have since been driven to **real,
MEASURED green** locally (W6 — see below). The first crate proven end-to-end:

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
  `gauss_newton_{2,3,4}`): the formerly-documented ~300-member
  "converging-optimizer" debt. **Audited and closed by the W6 analytics-rigor
  wave** — see the W6 subsection below.

#### `celnet-surface` — W6 analytics-rigor wave (file-scoped waves to zero)

Per `docs/plan/W6-ANALYTICS-RIGOR-PLAN.md` §3.4 the remaining crate was driven
to zero non-equivalent survivors in file-scoped waves, **pre-kill-first**. The
lever against the calibrate.rs converging-optimizer cluster is the crate's own
documented determinism (fixed-iteration libm-only fits, bit-reproducible):

- **`tests/fit_pins.rs`** — on two frozen fixtures per fitter (a benign
  three-point smile and a stressed five-point skew): (a) the achieved
  least-squares cost is pinned (`≤ frozen + 1e-12`, the degradation catch);
  (b) the **converged parameters are frozen to bits** (`to_bits` equality —
  any trajectory perturbation that moves the optimum at all is killed);
  (c) the fit reprices its anchors (benign: 1e-7 exact-fit; stressed: the
  frozen least-squares residual + 1e-12 — the economic assertion). Anchors are
  re-derived in-test from the published recipe through `calibrate_pillar`
  (the independent direction), never read back from the fitted object.
- **In-module solver pins** (calibrate.rs): the 2/3/4-parameter damped solvers
  driven directly on synthetic zero-residual least-squares problems with
  closed-form optima (`±1e-9`), an active-projection drive, and
  `gaussian_eliminate`/`solve3`/`solve4` vs an independently re-implemented
  textbook pivoted elimination (`±1e-10`) + singular-`None` paths; `clamp` /
  `sumsq` / `project_svi` boundary drives.
- **Module oracles** (pre-kill for the other waves): the published
  stochastic-vol expansion re-derived raw in-test on a `β < 1` slice (every
  term active) + the CEV small-ν density closed form + **frozen-bits pins on
  the wing crossover band and density-implied wing vols** (kills the
  wing-density quadrature internals that band/monotonicity properties
  tolerate); raw-recomputation pins for the parametric slice forms
  (analytic `w/w'/w''`/density-factor, scan-grid fold) and the surface-form
  curvature/variance/raw-map closed forms; butterfly/calendar
  clause-independence drives (slice, surface and report levels) with decisive
  margins; term-structure hand-interpolation pins (interior linear-in-τ, both
  extrapolation regimes incl. the negative-rate flat clamp, log-linear
  forward, fixed-strike calendar scan with differing pillar forwards);
  delta→strike round-trip through the convention-delta evaluator and the
  ATM-convention closed forms; the vanna-volga second-approximation
  multi-strike exact oracle.
- A representative damping mutant (`λ·0.5 → λ+0.5` in `gauss_newton_3`) was
  hand-applied before the waves: caught (`cargo test -p celnet-surface` real
  exit 101) — the mechanism kills trajectory-class mutants, not just
  value-class ones.

Measured wave baselines (aarch64-apple-darwin, cargo-mutants 27.0.0, toolchain
1.96.0, `--test-tool=cargo --jobs 2 --minimum-test-timeout=120`,
`PROPTEST_MAX_SHRINK_ITERS=0`, `RUSTC_WRAPPER=""`) are recorded per wave below
as each runs to green.

#### `celnet-qmc` — measured baseline (W6 analytics-rigor wave, crate-wide green)

The quasi-Monte-Carlo crate (gray-code Joe-Kuo Sobol + Owen-style nested
scramble + Brownian bridge + inverse-normal CDF + RQMC estimator) feeds
exotics, xva, the GPU path and parity, but had only 10 thin in-module tests.
Per the W6 plan (`docs/plan/W6-ANALYTICS-RIGOR-PLAN.md` §3.1) the gap was
closed **pre-kill-first**: `tests/sequence_oracle.rs` pins every module
against an independent oracle *before* the expensive run — exact dyadic
radical-inverse rationals (`to_bits`), an in-test `m_k`-domain re-derivation
of the direction-number recurrence fed with rows typed from the published
Joe-Kuo data file, published normal quantiles + a code-disjoint
`½·erfc(−x/√2)` round-trip (libm dev-dep), the exact bridge covariance
`L·Lᵀ = min(t_i,t_j)` + the hand-derived m=3 weight matrix, closed-form
monomial integrals through the full pipeline, an independent plain-loop
std-error re-derivation that simultaneously pins the per-replication seed
derivation, and frozen-bits rows (the fx_byte_identity house pattern) for the
hash/scramble internals that any distributional property tolerates. In-module
additions pin the nested-permutation prefix law and top-byte bijectivity of
the Owen scramble.

- **Raw crate-wide run** (no exclusions, aarch64-apple-darwin, cargo-mutants
  27.0.0, toolchain 1.96.0): **`359 mutants tested in 16m: 7 missed, 295
  caught, 2 unviable, 55 timeouts`**. Dispositions of the 7 raw survivors
  (full inline detail in `.config/mutants-qmc.toml`): 1 KILLED
  (`bisect` floor-midpoint pivot — covariance-invariant but
  plan-observable; pinned by the hand-derived m=3 weight matrix), 3 KILLED
  (`inv_norm_cdf` Halley-polish internals — sub-ULP centrally, ≥1 ULP at
  extreme tails; pinned by a frozen-bits ladder incl. the subnormal floor),
  1 ELIMINATED structurally (`point_u32` early-exit guard observable only as
  an OOB panic past the 2^32 period; loop made structurally total and the
  period documented), 2 EXCLUDED as provably equivalent (domain-guard
  short-circuit whose divergent path still NaNs through `ln(negative)`; OR vs
  XOR on disjoint bits) — each hand-reproduced: **full suite green with the
  mutant applied**.
- **Canonical gate run** (clean slate, `.config/mutants-qmc.toml`,
  `--test-tool=cargo --jobs 2 --minimum-test-timeout=240`,
  `RUSTC_WRAPPER=""`): **`350 mutants tested in 9m: 348 caught, 2 unviable`**
  — **zero missed, zero timeouts, real exit 0**. The enforceable bar
  (`just mutants-gate-qmc`, CI matrix leg) is **zero non-equivalent
  survivors** with both exclusions line-anchored and justified inline.
- **Environment lessons** (recorded for the other W6 crate gates): (a) the
  raw run's 55 timeout-class results were artifacts — macOS stalls the first
  launch of freshly linked test binaries under build churn, tripping the
  auto-set 20s floor (the suite runs in ~1s and the crate has no
  value-dependent loop that could hang); the 240s floor reclassifies all of
  them as caught. (b) One run wedged indefinitely on a stalled
  `sccache`-wrapped rustc (cargo-mutants sets no build timeout) —
  `RUSTC_WRAPPER=""` de-wedges and is also ~2× faster per mutant. (c) A
  line-anchored exclusion silently stopped matching when an unrelated
  refactor in the same file shifted its line (242→241); the survivor
  reappeared as MISSED on the next run — the gate caught its own stale
  anchor, but after ANY edit to a gated file, re-verify the `exclude_re`
  line anchors.

#### `celnet-xva` — measured baseline (W6 analytics-rigor wave, crate-wide green)

The XVA crate (CVA/DVA/FVA aggregation + piecewise-hazard survival curve +
low-discrepancy exposure simulation + synthetic netting sets) computes
regulatory/accounting numbers but had only 13 thin in-module tests. Per the W6
plan (`docs/plan/W6-ANALYTICS-RIGOR-PLAN.md` §3.2) the gap was closed
**pre-kill-first**: `tests/closed_form_oracle.rs` (32 tests) pins every module
against an independent oracle *before* the run — the documented CVA/DVA/FVA
quadrature re-derived longhand with raw std `exp` (single-interval,
non-uniform, and 101-node grids; both FVA signs), exact survival identities
(flat `Λ(t) = λ·t` to bits, an independent knot-overlap partial-sum
recomputation of the piecewise hazard integral, `S = exp(−Λ)` and
`S(a) − S(b)` to bits), the total-adjustment decomposition `CVA − DVA + FVA`
to bits, a from-scratch two-rate vanilla re-derivation (`Φ` via `erfc`, std
logs/exps) for every netting-set mark including the matured `τ ≤ 0 ⇒ 0`
boundary and signed notionals, frozen-bits exposure rows (the fx_byte_identity
house pattern) for two fixed netting sets chosen so BOTH `max(·, 0)` exposure
floors bite at the pinned nodes, and the full panic contract of every
constructor/aggregator assert driven on both sides of its boundary
(out-of-range LGD, negative hazards, non-increasing pillars/grids, zero
steps/paths/horizon).

- **Raw crate-wide run** (no exclusions, aarch64-apple-darwin, cargo-mutants
  27.0.0, toolchain 1.96.0, `RUSTC_WRAPPER=""`): **`170 mutants tested in 2m:
  3 missed, 161 caught, 6 unviable`** — zero timeouts. All 3 raw survivors
  belong to ONE equivalence cluster: the `cumulative_hazard` segment-scan
  early-return optimization (`survival.rs:83` `hi > lo` → `>=`, which differs
  only at `t = 0` where it adds exactly `+0.0`; `survival.rs:93` `&&` → `||`
  and `>` → `>=` on the post-loop extrapolation guard, whose both conjuncts
  are provably true whenever the line is reachable). Each was EXCLUDED with
  the reachability proof inline in `.config/mutants-xva.toml` and
  hand-reproduced — **full suite green with the mutant applied** — never to
  hide a value gap. The 6 unviable are `Default::default()` replacements on
  types that implement no `Default`.
- **Canonical gate run** (clean slate, `.config/mutants-xva.toml`,
  `--test-tool=cargo --jobs 3 --minimum-test-timeout=240`,
  `RUSTC_WRAPPER=""`): **`167 mutants tested in 3m: 161 caught, 6 unviable`**
  — **zero missed, zero timeouts, real exit 0**. The enforceable bar
  (`just mutants-gate-xva`, CI matrix leg) is **zero non-equivalent
  survivors** with all three exclusions line-anchored and justified inline.

#### `celnet-risk-cube` — measured baseline (W6 analytics-rigor wave, crate-wide green)

The risk-cube crate (hierarchical additive roll-up of per-ccy NetGreeks +
VegaLadder, the non-additive bump-and-revalue / sensitivity-Taylor VaR/ES lens,
FRTB-SA SbM/curvature/RRAO/DRC capital aggregation, the GPU spot×vol scenario
grid, and exotic risk-cube aggregation) is safety-critical capital arithmetic.
Per the W6 plan (`docs/plan/W6-ANALYTICS-RIGOR-PLAN.md` §3.3) the gap was
closed **pre-kill-first**: `tests/frtb_param_provenance.rs` pins every FRTB
parameter `to_bits` against a literal typed from the published BCBS MAR21 text
(paragraph cited per constant — the anti-circular 0.75ρ-lesson guard) plus
hand-built bucket-assignment pins; `tests/longhand_oracle.rs` re-derives the
firm aggregate with a naive double loop over all 11 NetGreeks lines + the
ladder, proves partition conservation across all six dimensions, drives the
VaR/ES quantile/tie/floor boundaries bit-exactly through a linear in-test
pricer, pins a hand Taylor expansion, recomputes the curvature legs from two
longhand revaluations (both down- and up-dominant combined regimes, with
in-test regime guards against fixture rot), and folds the combined
vanilla+exotic tail; `tests/fx_invariance.rs` keeps the FX `to_bits`
non-regression; in-module pins cover the 48-bit pair packing typed from hand
ASCII bytes + frozen-bits FNV group keys, scenario algebra byte-for-byte vs
the raw FX two-rate arithmetic, the MAR21.6 alternative-S_b branch hand-derived
end-to-end, the far-barrier vanilla-limit analytic oracle for the full exotic
FD Greek set (including an ultra-short-tenor leg proving the FD time bump
stays strictly inside the tenor), and row-major grid indexing with the
inclusive reconciliation band + worst-tie diagnostics.

- **File-scoped waves** (R1 dimension/additive/cube, R2 frtb_params/frtb,
  R3 nonadditive, R4 exotic/scenario_grid), each run to zero non-equivalent
  survivors. The raw R1 wave (121 mutants) surfaced exactly two `|`→`^`
  survivors in `underlying_group_value`: the bit-63 force was a GENUINE gap
  (both original pin inputs had raw-hash bit 63 clear) — killed by pinning
  OTHR/USD whose raw hash carries bit 63, asserted in-test; the byte fold is
  provably equivalent (disjoint bit positions) and excluded.
- **Full-crate discovery run** (no exclusions beyond the wave audits,
  aarch64-apple-darwin, cargo-mutants 27.0.0, toolchain 1.96.0,
  `RUSTC_WRAPPER=""`): **`961 mutants tested in 31m: 14 missed, 899 caught,
  48 unviable`**. Of the 14 survivors the file-scoped fixtures had not seen,
  9 GENUINE gaps were KILLED with oracle-pinned tests (the exotic FD time-bump
  `t/2` clamp via the ultra-short-tenor far-barrier vanilla-limit pin;
  `sbm_total`'s curvature term via a nonzero-curvature max-of-sums hand pin;
  the combined curvature UP-sum via an up-dominant long-digital regime; the
  scenario-grid `std_err` row-major stride, inclusive band boundary, and
  worst-tie diagnostics via hand-built grid literals against the
  identically-zero empty-book reference; the `std_err` notional-scaling
  quadrature via a bit-exact recomputation of the propagation chain) and 5
  audited equivalents were EXCLUDED with inline proofs in
  `.config/mutants-celnet-risk-cube.toml` (the `ccy_id` byte fold on disjoint
  bits; the ψ both-negative `<`→`<=` pair whose flipped branch multiplies an
  identically-zero cross term; the documented-zero MAR22 FX DRC body→`0.0`
  differing only in an uncontractual zero sign; and the `fx_vega_rw` cluster
  where the MAR21.93/.94 cap provably binds — anchored by an `uncapped > 1.0`
  assertion in the provenance suite so a future MAR revision forces re-audit).
  Each exclusion was hand-reproduced: **full suite green with the mutant
  applied**.
- **Canonical gate run** (clean slate, `.config/mutants-celnet-risk-cube.toml`,
  `--test-tool=cargo --jobs 2 --minimum-test-timeout=120`, `RUSTC_WRAPPER=""`):
  **`956 mutants tested in 27m: 908 caught, 48 unviable`** — **zero missed,
  zero timeouts, real exit 0**, disposition counts verified from the
  `mutants.out` ground truth at completion (908/0/0/48); an independent
  same-session clean-slate run reproduced the identical summary (39m under
  heavier concurrent-lane contention). The enforceable bar
  (`just mutants-gate-risk-cube`, CI matrix leg) is **zero non-equivalent
  survivors** with all 8 exclusions line-anchored and justified inline.

### Infra-crate mutation gate — `celnet-fanout` (W6, MEASURED green locally)

The SPMC broadcast ring (`celnet-fanout`) is the per-shard price fan-out
substrate: one pricing core → 100s–1000s of counterparty readers. Its hand-rolled
seqlock publish + torn-read protocol + conflation accounting is exactly the kind
of lock-free code where a silently-weakened suite is dangerous, so it gets its own
enforceable mutation gate in the house style — with two deliberate deltas from the
numerics gates (recorded in `.config/mutants-fanout.toml`):

- **plain `cargo test` runner** (`--test-tool=cargo`, NOT nextest): the gate must
  be reproducible independent of the workspace nextest profile, and a concurrent
  build session may `pkill -f nextest`.
- **`--jobs 3`**: bounds wall-time deterministically on the M4 and stays courteous
  to a parallel session.

| Crate | Config | just recipe | Loom oracle | Status |
|-------|--------|-------------|-------------|--------|
| `celnet-fanout` | `.config/mutants-fanout.toml` | `just mutants-gate-fanout` | `just loom-fanout` (`tests/loom_seqlock.rs`) | **MEASURED green locally** — zero non-equivalent survivors |

**Measured baseline** (`aarch64-apple-darwin`, cargo-mutants 27.0.0, toolchain
1.96.0). A crate-wide raw run surfaced **11 survivors** (62 viable mutants, 51
caught). Each was reproduced by hand and classified — none hidden:

- **KILLED with new tests (genuine gaps).** The conflation-frontier comparison
  `cursor < oldest_live`, the head empty-check, and the `Producer::published()` /
  `capacity()` accessors were untested. They were killed by
  `tests/conflation_boundaries.rs` — deterministic, single-threaded tests whose
  INDEPENDENT ORACLE is the closed-form half-open live window
  `[head − capacity, head)`: a single deterministic producer makes
  `oldest_live = head − capacity` an exact integer, so every delivered sequence and
  every skip count is predicted in closed form and asserted bit-for-bit (and
  `published()`/`capacity()` are asserted directly).
- **EQUIVALENT (excluded with inline justification + verified evidence).**
  - `mem.rs:69 spin_loop → ()`: `std::hint::spin_loop()` is a pure CPU-pause hint
    with no architectural effect.
  - `ring.rs:171 | with ^`: `seq << 1` is always even and `even ^ 1 == even | 1`
    bit-for-bit — an identical-value mutant (loom green with it applied).
  - `ring.rs:171 << with >>`: `(seq >> 1) | 1` stays odd, so it is still a valid
    in-progress marker — the only property the protocol uses is the odd-ness
    (loom green with it applied).
  - `ring.rs:277 < with ==` and `< with <=`: the conflation skip is performed a
    SECOND time in the seqlock retry loop with the same `head − capacity`, which
    compensates exactly (identical delivered sequence + skip count).
  - `ring.rs:304 < with <=` and `ring.rs:309 >= with <`: retry-path boundary /
    dead-branch mutants whose effect is a zero-length skip or a spin-vs-`Empty`
    that converges — the conservation contract (`received + skipped == produced`,
    strict order) is unchanged. Each verified: full std suite green with the
    mutant applied.
- **CONCURRENCY-ONLY, caught by the loom oracle (excluded from the `cargo test`
  gate, kept under the loom gate).** `ring.rs:171 | with &` makes the in-progress
  stamp `(seq << 1) & 1 == 0` (even), destroying the writer-side marker so a reader
  can accept a mid-write slot — a genuine TORN-READ bug. It manifests only as a
  relaxed-memory race a deterministic `cargo test` cannot reliably trigger (the
  "surfaced once under 16× CPU oversubscription" class the ring doc records). The
  **exhaustive loom model** (`just loom-fanout`) catches it DETERMINISTICALLY:
  verified this session — applying the mutant makes
  `spmc_seqlock_no_torn_read_under_all_interleavings` FAIL with a torn pair
  `(0, 2)`. It is excluded from the std gate (which cannot kill it) and stands
  under the loom gate; recorded here, not hidden.

After these tests + the justified exclusions, `just mutants-gate-fanout` runs to
**zero non-equivalent survivors**, exit 0.

#### Loom model-check of the seqlock ring (`just loom-fanout`)

`tests/loom_seqlock.rs` is an EXHAUSTIVE relaxed-memory model-check of the SPMC
seqlock under `loom` (MIT; a `[target.'cfg(loom)'.dependencies]` dev/`cfg`-only
dep that never enters a release build). The std hot path is byte-for-byte
unchanged: `src/mem.rs` is a `cfg(loom)` shim that re-exports the std atomics/cell
under `not(loom)` and loom-instrumented primitives under `--cfg loom`. The model
runs a 1-producer / 1-concurrent-consumer interleaving over a 2-slot ring (3
publishes ⇒ guaranteed in-place overwrite concurrent with a read), bounded by
`LOOM_MAX_PREEMPTIONS=3`, and proves NO interleaving returns a torn pair, with
strict in-order, conserving delivery.

- **Honesty boundary.** Strict-C11 loom (correctly) refuses to bless the
  production *non-atomic* `UnsafeCell` payload copy — the well-known benign
  seqlock data race. The model therefore renders the payload as `Acquire`/`Release`
  atomic lanes (the model-faithful image of the hardware coherence the production
  `Acquire` fence relies on) and verifies the **stamp/fence rejection protocol**
  exhaustively. The production non-atomic copy's soundness on the weakly-ordered
  target rests on the documented `Acquire` fence + cache coherence (`ring.rs`
  §"Seqlock reader barrier") and is exercised by the std conflation-stress suite.
- **The model is a LIVE oracle, not a vacuous pass.** Verified this session by a
  manual probe: disabling the consumer's post-copy `stamp_after != want` torn-read
  re-check makes the model FAIL deterministically (torn pair returned). So the
  model genuinely exercises the rejection logic. Run `just loom-fanout` after any
  edit to the publish/consume stamp protocol or the reader fence.

### Infra-crate mutation gate — `celnet-router` (W6, MEASURED green locally)

The fleet router (`celnet-router`) is the horizontal scale-out substrate: it
shards pricing work across stateless replicas by **highest-random-weight
(rendezvous) hashing**, fails over to hot standbys with no key loss, and sheds at
a per-replica inflight cap. The whole crate is a pure, deterministic routing core
(no IO, no pricing state); a silently-weakened suite could let a non-deterministic
assignment (fleet divergence), a fallback mis-selection, a down-standby route, or
a cap overshoot ship. It gets its own enforceable mutation gate in the house
style, with the same two deltas as the fanout gate (`.config/mutants-celnet-router.toml`):

- **plain `cargo test` runner** (`--test-tool=cargo`, NOT nextest) — reproducible
  independent of the workspace nextest profile; a concurrent session may
  `pkill -f nextest`.
- **`--jobs 3`** — bounds wall-time on the M4, courteous to a parallel session.

| Crate | Config | just recipe | Independent oracle | Status |
|-------|--------|-------------|--------------------|--------|
| `celnet-router` | `.config/mutants-celnet-router.toml` | `just mutants-gate-celnet-router` | code-disjoint splitmix64/digest + brute-force argmax (`tests/router_mutation.rs`) | **MEASURED green locally** — 99 mutants, 83 caught, 16 unviable, zero non-equivalent survivors |

**Measured baseline** (`aarch64-apple-darwin`, cargo-mutants 27.0.0, toolchain
1.96.0). A crate-wide raw run surfaced **32 MISSED** mutants (108 total; 45 caught,
16 unviable, the rest caught). The `mix64`/`digest`/`seed`/`fold64`/
`rendezvous_weight` whole-fn `-> 0/1`/operator collapse mutants ARE caught (they
make the hash degenerate, which the exact-value oracle + balance/avalanche suites
fail), but they originally **hung** the gate: three test helpers searched for a
key with a given owner via an unbounded `(0..).find(...)`, and a collapsed hash
pins the owner to one id so the search never terminates. Fixed by bounding those
searches to `[0, 10000)` with an `.expect()` (in `src/map.rs`'s two failover
tests and `tests/router_mutation.rs`'s standby test) so a degenerate-hash mutant
FAILS FAST (caught) instead of looping; the gate additionally sets
`PROPTEST_MAX_SHRINK_ITERS=0` (skip slow shrinking of a guaranteed-failing
proptest case) and a 60s timeout floor for deterministic scoring (exit 0). Each of
the 32 MISSED was reproduced and classified — none hidden:

- **KILLED with new tests (23 genuine gaps).** The pre-existing
  `tests/routing_properties.rs` proves the *shape* of routing (balance, minimal
  reshuffle, no key loss, bounded shed) but only asserts *relations* — descending
  rank, distinct digests, `route.replica != downed` — so a mutant that changes the
  hash mixing, the digest byte-packing, or *which* healthy replica a fallback picks
  still satisfied them. `tests/router_mutation.rs` closes the gap by pinning every
  load-bearing value against an **INDEPENDENT ORACLE**: a code-disjoint
  re-implementation of the frozen `splitmix64` finalizer / rotate-fold / rendezvous
  weight / replica seed / partition-key digest (re-derived from the documented
  algorithm, never calling the crate's `pub(crate)` `mix64`/`fold64`), plus a
  brute-force max-weight + lowest-id argmax. It kills:
  - the exact `digest` bits — every per-slot `<<` shift, the `|` pack, the `tagged`
    xor-mask, and the second-step `mix64` `^`/`>>` (digest calls `mix64`);
  - the exact `RankedReplica.weight` vs the independent rendezvous spec;
  - the natural-owner / primary-route argmax (vs brute-force);
  - the **HRW-fallback selection** — `route.replica` must equal the argmax over the
    HEALTHY subset, which distinguishes `172 > with <` (min-pick), `> with ==`,
    `|| with &&`, `&& with ||`, `== with !=`, and the guard `true`/`false` (each
    selects a different healthy replica for some down/seed configuration);
  - the `healthy_standby` health gate (`matches!(.., Up) -> true`) — a DOWN standby
    must be rejected and the key fall through to HRW;
  - the `cap` accessor and both `Display` impls (`fmt -> Ok(default)`).
- **EQUIVALENT (9 excluded with inline justification + verified evidence).** Two
  provably-equivalent clusters:
  - *Rendezvous-weight tie-break (4):* `map.rs:134` and `map.rs:172`,
    `bw > w -> bw >= w` and the tie clause `id.0 <= r.id.0 -> id.0 > r.id.0`. The
    `bw == w` tie clause is reached ONLY when two DISTINCT replicas have the EXACT
    same 64-bit rendezvous weight for one key; since the weight is
    `mix64(fold64(seed, digest))` with `mix64` a splitmix64 bijection, a tie
    requires a full 64-bit collision `fold64(seed_a, d) == fold64(seed_b, d)` for
    distinct seeds — computationally infeasible to exhibit, so no
    test/proptest/production input reaches it.
  - *Disjoint-bit-slot OR == XOR in the digest pair-pack (5):* `key.rs:95–99`,
    `| with ^`. `digest` packs the six validated ASCII currency bytes into one u64
    at NON-OVERLAPPING 8-bit positions (`b0<<40 | … | q2`). Each byte is `<= 0xFF`
    (8 bits) and the slots are 8 bits apart, so the operands have pairwise-disjoint
    bits — for which `a | b == a ^ b` bit-for-bit. The mutated lane (hence digest)
    is IDENTICAL for every valid pair. (`| with &` is NOT excluded: `a & b == 0`
    for disjoint bits changes the digest and IS caught by the exact-value oracle.)
  Each was VERIFIED equivalent this session: applied to the source, the FULL suite
  (`router_mutation.rs`'s exact-digest + argmax pinning over 6 replicas × 400–600
  keys, and `routing_properties.rs`'s 8000-key balance/reshuffle) stayed GREEN.

After these tests + the 9 justified exclusions, `just mutants-gate-celnet-router`
runs to **zero non-equivalent survivors**, exit 0 (MEASURED: 99 mutants tested in
42s after the 9 exclusions — 83 caught, 16 unviable, **0 missed, 0 timeout**).

### Infra-crate mutation gate — `celnet-journal` (W6, MEASURED green locally)

The durable event journal (`celnet-journal`) is the crash-recovery substrate: an
fsync'd, append-only, sequence-ordered log that, on restart, heals a torn tail and
replays to rebuild state bit-identically. The W6 frame change makes every record
begin with an 8-byte **sync word** (`SYNC_WORD`, ASCII `"CLNJRNL\0"` LE, inside the
CRC coverage), which lets recovery tell an **interior CRC failure** (intact sync
word + a complete frame body whose CRC fails ⇒ `CorruptInterior`) apart from a
**torn tail** (a short read, or a start not led by the sync word ⇒ heal) — closing
the one limitation the old marker-less format documented. A silently-weakened suite
could let a torn-tail-vs-interior-corruption misclassification or a non-deterministic
replay ship, so the crate gets its own enforceable mutation gate in the house style,
with the same two deltas as the fanout/router gates (`.config/mutants-journal.toml`):

- **plain `cargo test` runner** (`--test-tool=cargo`, NOT nextest) — reproducible
  independent of the workspace nextest profile; a concurrent session may
  `pkill -f nextest`.
- **`--jobs 3`** — bounds wall-time on the M4, courteous to a parallel session.

| Crate | Config | just recipe | Independent oracle | Status |
|-------|--------|-------------|--------------------|--------|
| `celnet-journal` | `.config/mutants-journal.toml` | `just mutants-gate-journal` | running-sum replay (`replay_from_compacted_equals_replay_from_full_bit_identical`, reference state = plain integer sum, code-disjoint from framing) + kill-restart byte-identity (`src/tests.rs`) + the 512-case adversarial-bytes proptest (`tests/decode_fuzz.rs`) | **MEASURED green locally** — 116 mutants, zero non-equivalent survivors |

**Measured baseline** (`aarch64-apple-darwin`, cargo-mutants 27.0.0, toolchain
1.96.0). A strict (empty-exclude) run surfaced **16 MISSED** of 116 mutants (86
caught, 14 unviable). Each was reproduced and classified — none hidden:

- **KILLED with new tests (11 genuine gaps).** The behavioral tests exercise these
  branches but did not *assert exactly*, so a syntactic mutant survived. New
  value-pinning tests in `src/tests.rs` close every one:
  - the recovery safety bound `MAX_PAYLOAD_LEN = 64 * 1024 * 1024` — both `*`→`+`
    mutants (which change the constant to 1_048_640 / 66_560) — pinned bit-for-bit
    by `max_payload_len_is_exactly_64_mib` (the oversize-rejection tests reference
    the constant itself, so only an exact pin distinguishes the original value);
  - the error surface — `<Display>::fmt -> Ok(default)`, `<Error>::source -> None`,
    and `delete match arm Io(e)` — pinned by `journal_error_display_and_source_are_exact`
    (asserts each variant's `Display` text and that only `Io` exposes a `source`);
  - the recovery length-bound guards `len_field > MAX_PAYLOAD_LEN` (`read_one`) and
    `snap_len > MAX_PAYLOAD_LEN` (`read_snapshot`), BOTH `> with ==` and `> with >=`
    — a FULLY-VALID record/snapshot whose body is EXACTLY `MAX_PAYLOAD_LEN` bytes
    must round-trip (append/compact accept `len <= MAX`); under either mutant the
    exact-MAX body is rejected as torn and dropped, failing the round-trip:
    `exact_max_payload_record_roundtrips` (data) and
    `exact_max_snapshot_record_roundtrips` (snapshot). A complete exact-MAX body is
    the only input that separates the boundary — a short hostile body heals
    identically under all three operators. A companion robustness test
    (`oversized_length_field_heals_without_wild_allocation`) pins that a hostile
    above-bound length field heals as a torn tail without a multi-GiB allocation;
  - `read_full_or_short`'s `Interrupted`-retry guard (`guard -> true`/`false`,
    `== with !=`) and the short/empty/full classification — driven by a scripted
    `Read` (the in-module `ScriptedReader`) in `read_full_or_short_retries_on_interrupted_then_fills`,
    `..._propagates_non_interrupted_errors`, and `..._reports_short_then_full_and_empty`.
- **EQUIVALENT / fault-injection-only (5 excluded with inline justification +
  verified evidence).** Two clusters, each VERIFIED by applying the mutant and
  running the full suite GREEN:
  - *No-op-at-boundary (2):* `Journal::open`'s truncation guard
    `scan.good_end_offset < file_len -> <=` (at equality, `set_len(file_len)` is a
    no-op + a harmless extra fsync), and `read_full_or_short`'s fill loop
    `while filled < buf.len() -> <=` (the extra iteration reads into an empty
    sub-slice, returns `Ok(0)`, and breaks — identical classification/bytes).
  - *Directory-entry durability, power-loss-only (3):* `sync_parent_dir -> Ok(())`
    and the `delete !` in its `filter(|p| !p.as_os_str().is_empty())`. This is the
    POSIX directory fsync that makes a freshly-created/renamed file's *directory
    entry* durable; its effect is observable ONLY across a real crash + power loss,
    which a process-level test cannot exhibit (every test's parent dir opens+fsyncs
    fine either way; `delete !` fsyncs the cwd `"."` instead of the parent — still a
    valid `Ok`). Recorded honestly as fault-injection-only, not a suite gap.

After these tests + the 5 justified exclusions, `just mutants-gate-journal` runs to
**zero non-equivalent survivors**, exit 0.

#### Sync-word frame discrimination — the dedicated regression tests

Beyond the mutation gate, the frame change is pinned by behavioral tests in
`src/tests.rs`: `intact_syncword_failing_crc_interior_is_corrupt` (interior CRC
failure surfaced), `torn_tail_still_heals_with_syncword_format` (healing intact),
`interior_corruption_distinct_from_torn_tail` (the two formerly-indistinguishable
inputs now yield different verdicts), `snapshot_interior_crc_failure_is_surfaced`
(the snapshot analogue), and the repurposed
`interior_crc_corruption_is_surfaced_not_healed` /
`flipped_byte_in_final_record_with_intact_sync_is_interior` /
`truncated_final_record_heals_as_torn_tail`. The compaction round-trip and
kill-restart byte-identity tests pass UNCHANGED after the frame change — the
non-negotiable regression check that recovered *state* is unaffected.

### Infra-crate mutation gate — `celnet-replog` (W6, MEASURED green locally)

The leader-replicated, deterministic-replay event log (`celnet-replog`) is the
distributed-correctness backbone: a real multi-node Raft consensus module (election,
log-matching, conflicting-tail truncation, quorum commit, §7 snapshot/compaction +
InstallSnapshot) layered on the durable `celnet-journal`. It is the largest infra
leaf (≈ 4 500 src lines across `log` / `election` / `wire` / `state` / `persist` /
`compaction` / `entry`), so it gets its own enforceable mutation gate in the house
style, with the same two deltas as the fanout/router/journal gates
(`.config/mutants-celnet-replog.toml`):

- **plain `cargo test` runner** (`--test-tool=cargo`, NOT nextest) — reproducible
  independent of the workspace nextest profile; a concurrent session may
  `pkill -f nextest`.
- **`--jobs 3`** — bounds wall-time on the M4, courteous to a parallel session.

| Crate | Config | just recipe | Independent oracle | Status |
|-------|--------|-------------|--------------------|--------|
| `celnet-replog` | `.config/mutants-celnet-replog.toml` | `just mutants-gate-celnet-replog` | running priced-book replay (`gate_a`/`gate_d` in `tests/replication.rs` re-apply the committed deltas to a fresh `BookState`, comparing by `f64::to_bits` — code-disjoint from the log/framing) + the byte-identical committed-log assertion + the 512-case adversarial-bytes decoder proptest (`tests/decode_fuzz.rs`) | **MEASURED locally** — 511 mutants, **zero MISSED (deterministic) survivors**; residual = genuine infinite-loop TIMEOUT catches + 2 std-`TcpStream`-untestable `read_frame_or_idle` boundaries (documented below, none hidden) |

**Frame-change prerequisite (fixed first).** The journal sync-word frame change
(above) silently broke `celnet-replog`: its log-rewrite helper `write_fresh_journal`
hand-rolled the *old* journal frame layout (`len ‖ seq ‖ payload ‖ crc`, no sync
word), so a rewritten log re-opened as a torn tail and recovered to **zero** records
(5 `log` tests failed: `discard_prefix_*`, `reconcile_truncates_*`,
`install_snapshot_matching_tail_*`, `append_after_discard_*`). The fix removes the
duplicated format entirely — `write_fresh_journal` now opens a fresh `Journal` and
`append`s each payload, so the frame layout has a **single owner** (`celnet-journal`)
and cannot drift again (guardrail 10, zero legacy / single source of truth).

**Measured baseline** (`aarch64-apple-darwin`, cargo-mutants 27.0.0, toolchain
1.96.0). A strict (empty-exclude) run surfaced **76 MISSED + 12 TIMEOUT** of 534
mutants (255 caught, 191 unviable); fixing the `write_fresh_journal` bug above
*unmasked* a further set of `compact_to`/`install_snapshot` branches (they had been
"caught" only because recovery failed outright). Every survivor was reproduced and
classified — none hidden:

- **KILLED with new direct unit tests (the genuine gaps).** The 1 581-line
  `election` module had **zero** direct unit tests — its pure decision logic was only
  exercised *indirectly* by the end-to-end loopback gates, so a syntactic mutation of
  a comparison / boolean connective / returned value survived (the cluster outcome was
  unchanged or merely slower). A new in-module `election::core_tests` suite constructs
  a bare `NodeCore` (no threads/sockets) and a single-node `RaftNode`, and asserts the
  branch behavior *directly*:
  - the §5.4.1 up-to-date vote rule `candidate_log_ok` (higher-term-wins,
    equal-term-length compare, the `+ 1` length arithmetic, the EMPTY_LOG sentinel);
  - `reset_election_timer`'s jitter-span arithmetic (deadline within `[now+min,
    now+max]` — kills the `+`/`%` mutants and their timeout-hangs);
  - the three RPC receivers `handle_append_entries` / `handle_install_snapshot` /
    `handle_request_vote` (stale-term reject, term-adopt direction, the commit guard's
    `&&`, the snapshot header-equality `||`/`!=`, the pre-vote/real-vote connectives,
    the already-voted `==`, step-down-on-higher-term);
  - the §5.4.2 commit-quorum `leader_advance_commit` (a lone leader cannot commit; a
    current-term majority does, applying in order — graded by `to_bits`);
  - the accessor/lifecycle surface (`last_log_index`, `applied_state`, `propose`,
    `wait_for_commit`, `compact_applied`, `safe_compact_index`, and `signal_wake` /
    `shutdown` / `Drop` / `wait_or_wake` teardown — a no-op teardown hangs the join).
  Targeted unit tests in the other modules close the rest: `log` (`is_empty`, the
  `discard_prefix` idempotent-guard boundary + drop-count subtraction over a shifted
  base, the `reconcile` skip-loop over a present prefix); `wire` (`WireError` Display,
  `MAX_FRAME_LEN` exact value, the `read_frame` / `read_frame_or_idle` frame-length
  boundary and the idle-vs-frame guard, and a fast no-hang `write_frame` no-op kill via
  a short receiver read-timeout); `state` (`UpdateError` Display, `len`/`is_empty`/
  `to_bits` exact contents, the Remove-tag length guard); `persist` &  `compaction`
  (the `NotFound` match-guard — absent file is default/None, a real IO error
  propagates — the `state()` accessor, and the `SnapshotError`/`UpdateError` Display).
- **EQUIVALENT / fault-injection-only (excluded with inline justification + verified
  evidence).** Two clusters, each VERIFIED by hand-applying the mutant and running the
  full suite GREEN:
  - *Directory-entry durability, power-loss-only (3 copies × 2 = 6):* `sync_parent_dir
    -> Ok(())` and the `delete !` in its `filter(|p| !p.as_os_str().is_empty())`, in
    the atomic log rewrite, the persist store, and the snapshot store. Identical to the
    journal gate's documented exclusion — the POSIX directory fsync's effect is
    observable ONLY across a real crash + power loss, which a process test cannot
    exhibit.
  - *No-op-at-boundary length/path guards:* `Snapshot::decode`'s header floor
    `bytes.len() < HEADER + 4 -> <=` (NO 28-byte input ever decodes Ok — the embedded
    empty state still needs its 8-byte count, so the smallest decodable snapshot is 36
    bytes; both operators return `Malformed` for every input), and `Log::reconcile`'s
    cut-placement `cut_phys < terms.len() -> <=` at 312 (at equality, the pure-append
    case routes through `truncate_and_append(keep = len)`, byte-identical result).
  - *Raft liveness / self-healing optimizations (safety unaffected):* `candidate_log_ok`
    `> -> >=` at 154 (unreachable equality — the enclosing `!=` guard excludes it);
    `compact_to`'s prior-snapshot-seed guard `-> true` (the durable-snapshot boundary
    always equals `log.snapshot_index()` by the save+discard pairing invariant);
    `leader_advance_commit`'s scan lower-bound `c + 1 -> c * 1` at 376 (a re-scan of
    already-committed indices, idempotent); `boot_on`'s recovery replay-start and peer
    `next_index` init `+ 1 -> *`/`-` (the former clamped by `entries_from`, the latter
    a self-healed optimistic guess); `propose`'s single-node fast-commit `== 1 -> != 1`
    and `signal_wake -> ()` (the heartbeat tick re-commits within a beat); `shutdown ->
    ()` and the `Drop` `|| -> &&` guard (Drop is the backstop / handles move in
    lockstep); `wait_for_commit`'s `< -> <=` loop bound and `>= -> <` checks (timing /
    masked by the final return). Each is **line-anchored** in the config so a sibling
    mutation at a DIFFERENT line that IS a real bug (e.g. `leader_advance_commit`'s
    `majority = size/2 + 1 -> * 1` at 374; `reconcile`'s loop bound `< -> <=` at 288
    that would panic; `first_new == len -> !=` at 296; the in-loop term-adopt branches)
    is **not** masked — those are KILLED by the new tests.
  - *Caught by the per-mutant TIMEOUT (NOT excluded — a real detection):* the
    infinite-loop mutants `tick_loop`'s deadline check `>= -> <` (1179) and the
    `reconcile` skip-counter `+= -> *=` (291:53) make a loop run forever that the
    original always exits — the test suite hangs and cargo-mutants scores them as
    Timeout. A legitimate catch (a hang the original never exhibits).
  - *Std-`TcpStream` unit-testability limit (2 mutants, recorded honestly, NOT
    excluded):* `read_frame_or_idle`'s length-boundary `> -> >=`/`==` (464) and the
    first-byte error guard `matches!(…) -> true` (450). `read_frame_or_idle` CLEARS
    the read-timeout after the first byte (by design — so a slow-but-live sender never
    desyncs the stream), so an over-bound / exactly-`MAX_FRAME_LEN` prefix the mutant
    fails to reject blocks on a body that never arrives → caught only as a TIMEOUT;
    and exercising the 450 guard's negative arm needs a forced TCP RST first-byte
    error (`TcpStream::set_linger`, nightly-only on 1.96). These are honestly logged as
    caught-by-timeout / std-untestable rather than excluded — the production serve loop
    only ever hits these paths with a timeout or a clean EOF, both handled correctly.
    (The sibling, deterministically-testable wire boundaries — `read_frame`'s `> ->
    ==`/`<`/`>=` at 412, `write_frame -> Ok(())` at 390, `MAX_FRAME_LEN`'s `* -> +`,
    the `WireError` Display — are all KILLED fast via a short-read-timeout harness so
    a missing/oversize frame fails as a deterministic `Io`/`FrameTooLarge` rather than
    a hang.)

After these tests + the line-anchored equivalence exclusions, `just
mutants-gate-celnet-replog` reaches **zero MISSED (deterministic) survivors**. The
residual non-MISSED outcomes are the genuine infinite-loop TIMEOUT catches and the
two std-`TcpStream`-untestable `read_frame_or_idle` boundaries above — so the run
reports a non-zero (timeout-class) exit, with every deterministic survivor at zero
and nothing hidden behind an unjustified exclusion.

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

Two classes of input are fuzzed: the **numerical pricing domain** (one target)
and the **untrusted-bytes decode/recovery boundaries** (five targets — the
durable-log and wire-contract surfaces that ingest attacker-controllable bytes,
where a panic/OOM/torn-state would be a denial-of-service or a corruption bug,
not just a wrong price):

| Target               | Under test                                          | Contract asserted                                                        |
|----------------------|-----------------------------------------------------|--------------------------------------------------------------------------|
| `vanilla_inputs`     | `celnet_vanilla::price` + `::greeks`                | no panic; premium ≥ 0 and finite; all 13 Greeks finite.                  |
| `replog_log_entry`   | `celnet-replog` log-entry codec (`entry`/`log`)     | decode of arbitrary bytes never panics; round-trips when it decodes.     |
| `replog_snapshot`    | `celnet-replog` §7 snapshot codec (CRC'd snapshot)  | corrupt/truncated snapshot bytes are rejected, never silently accepted.  |
| `replog_wire_message`| `celnet-replog` Raft RPC wire framing (`wire`)      | length-prefixed RPC decode of arbitrary bytes never panics.             |
| `journal_recover`    | `celnet-journal` torn-tail recovery over a real temp file | crash-mid-append heals to last good record; interior corruption surfaced, bounded alloc. |
| `proto_convert`      | `celnet-proto` prost decode + `convert.rs` `TryFrom` | decode of arbitrary wire bytes + the typed-conversion layer never panics. |

`vanilla_inputs` folds raw fuzzer bytes (via `arbitrary`) into a
valid-but-adversarial `VanillaInputs` in the Garman-Kohlhagen domain (strictly
positive spot/strike/vol/time, finite bounded rates), so the budget is spent on
in-domain corners — deep ITM/OTM, near-zero / century expiries, micro / macro
vols, large rate spreads — rather than trivially-rejected garbage. The decode
targets instead feed the **raw** fuzzer bytes straight at the decoder, since the
whole point is that the byte boundary must be panic-free on arbitrary input. The
decode targets are mirrored as deterministic in-tree corpus-replay tests
(`celnet-replog/tests/decode_fuzz.rs`, `celnet-journal/tests/decode_fuzz.rs`) so
the boundary stays covered inside the stable `just check` gate even without the
nightly sanitizer runtime.

They run as a dedicated **Linux-nightly** CI job — the `fuzz` job in
`.github/workflows/ci.yml` — which installs `cargo-fuzz` on nightly and runs
**all six** targets time-boxed on every push/PR (`vanilla_inputs` a 60 s smoke
budget, each decode target a 30 s smoke budget); a panic or contract break fails
the job. It is deliberately not part of the stable cross-platform gate. Locally
(or for a longer campaign) run a target via:

```
rustup toolchain install nightly && cargo install cargo-fuzz
just fuzz-vanilla 120         # cd fuzz && cargo +nightly fuzz run vanilla_inputs -- -max_total_time=120
cd fuzz && cargo +nightly fuzz run replog_snapshot -- -max_total_time=120   # any decode target
```

## 5. Golden / reference parity

Numerical correctness is anchored to **published prices** and the open-source
**QuantLib** oracle (guardrail #5/#7): numerical code is validated against an
*independent* reference, never merely asserted plausible. The fixtures and
harness live in two crates owned by the parity lane — `celnet-golden` (frozen
reference tables + the loaders/gating that read them) and `celnet-parity`
(executable parity rows that drive a Celnet engine against an independent
in-test re-derivation or a `celnet-golden` table). This document covers the
**structural** hardening gates (mutation/coverage/fuzz above) that wrap them and
records the golden surface so no consumer over-reads it.

### 5.1 Frozen reference tables (`celnet-golden`)

Each table is a committed CSV with full provenance, loaded by a typed record and
gated by a `tests/*_grid.rs` row to a stated tolerance. The closed-form/QuantLib
families are gated near the last bit (~1e-10); the Heston family is gated to the
*published* precision of its third-party oracle (not a last-bit claim — see
below).

| Table (`data/`)          | Family                                   | Gate (`tests/`)          | Oracle / provenance                                                      |
|--------------------------|------------------------------------------|--------------------------|-------------------------------------------------------------------------|
| `vanilla_gk.csv`         | Garman-Kohlhagen vanilla + Greeks        | `vanilla_grid.rs`        | QuantLib 1.42.1 frozen table, ~1e-10                                     |
| `barrier_gk.csv`         | single barriers (all 8 in/out × up/down) | `barrier_grid.rs`        | QuantLib 1.42.1, ~1e-10                                                  |
| `double_barrier_gk.csv`  | double-knockout barriers                 | `double_barrier_grid.rs` | QuantLib 1.42.1, ~1e-10                                                  |
| `digital_gk.csv`         | cash-or-nothing / asset-or-nothing       | `digital_grid.rs`        | QuantLib 1.42.1, ~1e-10                                                  |
| `touch_gk.csv`           | one-touch / no-touch                      | `touch_grid.rs`          | QuantLib 1.42.1, ~1e-10                                                  |
| `heston_fo.csv`          | Heston stochastic-vol European           | `heston_grid.rs`         | **Fang & Oosterlee (2008)** published "Reference val." (Carr-Madan `N=2¹⁷`) |

**Heston golden — the independent-third-implementation gate.** The
`celnet-heston` crate's own gate proves its two transforms (Carr-Madan damped-
integral quadrature and the Fang-Oosterlee COS expansion) **agree**, but two
transforms of one possibly mis-derived characteristic function could agree on a
*wrong* value — a shared CF / quadrature / carry / discounting error is invisible
to a cross-check between them. `heston_grid.rs` closes that gap with a *third*
implementation: the published Fang & Oosterlee (2008, *SIAM J. Sci. Comput.*
31(2):826–848, §5.3) "Reference val." figures, computed by the authors with the
Carr-Madan method at `N = 2¹⁷` points — `5.785155450…` (`T=1`, Table 4) and
`22.318945791…` (`T=10`, Table 5) over their eq. (53) parameter set. QuantLib's
`AnalyticHestonEngine` is unavailable in this build environment, so rather than
fabricate an oracle the table is **narrowed honestly** to those authoritative
published constants. Carr-Madan is gated on **every** row (it reproduces `T=10`
to ~1.5e-10); **COS is gated only on rows whose `cos_valid` flag is set** — the
`≤3y` FX-vanilla regime the crate documents — because past the Fourier-COS
precision wall (`T=10`) the cosine method legitimately diverges, and the table
deliberately does **not** assert COS there so a regression that quietly "fixed"
COS to match would surface as a new, separately-reviewed claim. Tolerance
`abs/rel 1e-6` is set to the published nine-decimal granularity (honestly *not* a
last-bit claim); celnet meets it with ~2 orders of margin (worst deviation
≈ 1.6e-8).

### 5.2 Executable parity rows (`celnet-parity`)

Beyond the frozen tables, `celnet-parity` carries ~30 executable rows that drive
a Celnet engine against an **independent** oracle per row — either a from-scratch
in-test re-derivation (a second, deliberately-disjoint implementation) or a
`celnet-golden` table — covering the full shipped catalogue: vanilla Greeks,
exotics + exotic risk-cube, Asians, baskets/best-of/worst-of (`basket.rs`,
`structured.rs`), LSV, eSSVI (+ `essvi_hardening.rs`), variance/vol swaps,
forward-start/cliquet, Heston (`heston.rs`), FRTB-SA (`frtb.rs`), XVA, conventions
+ pair-universe, broker→smile + surface, QMC (+ high-dim RQMC), the GPU path/Greeks
kernels (`gpu_path.rs`/`gpu_greeks.rs`), and the Raft consensus correctness rows
(`raft_election.rs`/`raft_compaction.rs`/`raft_snapshot.rs`). Each row is a real
gate, not a smoke test: it fails the build if the engine diverges from the
independent reference beyond the stated tolerance (closed-form/golden families
near the last bit; Monte-Carlo families within a reported **price std-error** band,
never a "machine precision" claim).
