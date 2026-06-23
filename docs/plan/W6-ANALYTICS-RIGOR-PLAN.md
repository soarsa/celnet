# W6-ANALYTICS-RIGOR — Mutation-to-Zero + Fuzz Floor for the Numerics Crates

**Lane:** W6-ANALYTICS-RIGOR (session-A, claimed).
**Scope:** drive mutation testing to **zero non-equivalent survivors** and add fuzz
targets for the five numerics crates now stable post-ADR-0008-tail:
`celnet-exotics`, `celnet-surface`, `celnet-risk-cube`, `celnet-xva`, `celnet-qmc`.
**Read base:** a clean worktree of origin/main (HEAD
`7f3209a`). All counts below were measured there with `grep`/`wc` only (no cargo —
compute courtesy §4.1: session-B holds the proto window).

---

## 0. Standing constraints (apply to every step)

1. **No mocks, no placeholders** — every new test asserts against a real engine and an
   independent oracle; every fuzz target drives production code by path dependency.
2. **Vendor-neutral names** — Joe–Kuo direction numbers, Wichura AS241,
   Broadie–Detemple, Andersen QE, Basel MAR21, QuantLib appear in **doc comments /
   prose only**, never in identifiers. Test names state the *purpose*
   (`low_discrepancy_reference_points`, not `joe_kuo_table`).
3. **ADR-0008 carry seam** — no test, oracle, or fuzz draw may introduce hot-path
   asset-class matching. Scenario/exposure oracles work in the carry-neutral `(r, b)`
   basis on `Carry`, exactly like `celnet_risk_cube::Scenario` does.
4. **One unversioned contract** — no new public API; this lane adds tests, configs,
   fuzz targets, and justfile/CI plumbing only.
5. **Independent, non-circular oracles** — every value-pin is re-derived in-test from
   the published formula/source (the FRTB 0.75ρ lesson: re-derive constants vs the
   *published* text, never read them back from the code under test).
6. **FX byte-identity preserved** — this lane changes **zero product code**. If a
   mutant audit or a fuzz finding exposes a genuine product bug, the fix is a
   separate, ledgered change and must keep
   `crates/celnet-exotics/tests/fx_byte_identity.rs` green **bit-for-bit**
   (`f64::to_bits` equality). A fix that legitimately changes frozen bits is a
   STOP-and-escalate: re-freezing requires its own justified ledger entry.
7. **Compute courtesy** — nothing in §5 runs while session-B is building. Every
   mutation run uses `--test-tool=cargo --jobs 3` (plain `cargo test`, never nextest —
   a concurrent session may `pkill -f nextest`), a `timeout` wrapper, and one
   cargo-mutants process at a time on the M4.

---

## 1. Measured baseline (what exists today)

### 1.1 Mutation configs (`.config/`)

| Config | Crate | exclude_re | Status |
|---|---|---|---|
| `mutants.toml` | celnet-vanilla | 35 audited equivalents | **green** (469 mutants, 100 % non-equiv kill) |
| `mutants-surface.toml` | celnet-surface | **empty** | `arbitrage.rs` green locally (79 mutants, 0 missed); ~300-member `calibrate.rs` converging-optimizer cluster **documented but unaudited**; crate-wide CI-run |
| `mutants-exotics.toml` | celnet-exotics | **empty** | wired, baseline CI-run, never locally measured |
| `mutants-celnet-risk-cube.toml` (then `mutants-risk-cube.toml`) | celnet-risk-cube | **empty** | wired, baseline CI-run, never locally measured |
| `mutants-xva.toml` | celnet-xva | **empty** | wired, baseline CI-run, never locally measured |
| `mutants-fanout/-journal/-celnet-router/-celnet-replog.toml` | infra | audited | green (W6 infra wave — the house playbook this plan follows) |
| *(none)* | **celnet-qmc** | — | **no config, no recipe, no CI leg — must be created** |

House style (from the four W6 infra configs): header documents the gate command, the
killing suite, the **independent oracle**, and a MEASURED-baseline section; `exclude_re`
starts empty and gains only **line-anchored, hand-reproduced, individually-justified**
entries; timeout-class catches (hangs the original never exhibits) are recorded
honestly, never hidden.

### 1.2 Crate sizes, test counts, mutant estimates

Calibration: `arbitrage.rs` = 159 non-test LOC → 79 mutants (0.50/LOC);
`celnet-surface` ≈ 4,190 non-test LOC → ~2,300 documented (0.55/LOC). Estimates below
use **0.5 × non-test LOC** (lines before `#[cfg(test)]`; comment-heavy files run lower).

| Crate | non-test LOC | `#[test]` | proptest | est. mutants | Strongest existing oracle |
|---|---|---|---|---|---|
| celnet-exotics | ~9,400 | 169 + 22 integration | 2 | **~4,500–5,000** | `tests/fx_byte_identity.rs`: 22 tests / **107 frozen `to_bits` gates** over MC/PDE/ADI/American/composite/pivot; `tests/pivot_oracle.rs` (455 LOC); QuantLib golden grids (barrier/digital/touch) in celnet-golden; parity rows |
| celnet-surface | ~4,190 | 95 | 2 | **~2,300** (documented) | arbitrage value-pins (green); strangle/stochvol/market_hedge independent-oracle tests; coverage gate 90/90 |
| celnet-risk-cube | ~3,220 | 40 | 0 | **~1,600** | `tests/longhand_oracle.rs` (378 LOC), `tests/fx_invariance.rs` (337 LOC) |
| celnet-xva | ~600 | 13 | 0 | **~300** | thin — in-module unit tests only |
| celnet-qmc | ~720 functional (+1,529 const-table LOC in `direction_numbers.rs` that yields few mutants) | 10 | 0 | **~350** | thin — in-module unit tests only |

### 1.3 Fuzz harness (`fuzz/`)

Standalone non-workspace crate, nightly libFuzzer, 8 targets: `vanilla_inputs`
(in-domain structured draw via `arbitrary` + `clamp_into` tanh squash), 5
untrusted-bytes decode targets, 2 property targets (`fanout_ring`, `router_route`).
House pattern: every fuzz property is **mirrored as a proptest suite inside the stable
`just check` gate** (e.g. `celnet-replog/tests/decode_fuzz.rs`), so the property blocks
merges even without the nightly lane. CI: `fuzz` job runs each target time-boxed
(30–60 s smoke) on every push. **None of the five analytics crates is fuzzed today.**

---

## 2. (a) Priority order + rationale

Run order is **gap-likelihood ÷ cost**, smallest-first so green gates land early and
durably (commit after each crate/wave):

| # | Crate | Why here |
|---|---|---|
| 1 | **celnet-qmc** | Weakest suite (10 tests) over real numerics that feed exotics, xva, gpu, parity. Smallest run (~350 mutants). Needs new config/recipe/CI leg anyway. Real gaps near-certain (e.g. `direction_numbers` accessors, scramble path, bridge weights have zero direct value-pins). |
| 2 | **celnet-xva** | 13 tests over regulatory numbers (CVA/DVA/FVA). ~300 mutants, closed-form oracles are cheap to write. Quick second green. |
| 3 | **celnet-risk-cube** | Capital arithmetic (VaR/ES, FRTB-SA) with a good longhand oracle but only 40 tests over ~3.2 k LOC; `cube.rs`/`dimension.rs`/`additive.rs` have **zero** in-module tests (covered only via lib.rs/integration). ~1,600 mutants, file-scopable. |
| 4 | **celnet-surface** | The documented debt: the ~300-member `calibrate.rs` cluster is the single largest *unaudited* exclusion candidate in the repo. arbitrage.rs already proves the mechanism. ~2,300 mutants, run as 5 file-scoped waves. |
| 5 | **celnet-exotics** | Largest (~4,700 mutants, wall-time dominated). The 107 frozen `to_bits` gates make engine-trajectory mutants *killable* (any arithmetic perturbation on a priced path changes bits), so the **already-byte-identity-gated paths (mc/pde/american/composite/pivot) are LOW gap-risk**; the real gaps concentrate where in-module tests are thin **and** the module is not on the frozen-bits path: `particle.rs` (251 LOC / 2 tests, not in the fx gate), `leverage.rs` (285/2), `adi.rs` internals off the priced path (844/2), `stochvol.rs` (263/4), `inputs.rs` seam accessors (267/2). Order waves accordingly (weak-first), closed-form golden-gated modules last. |

Fuzz targets (§4) are **authoring-parallel**: they can be written during session-B's
window (no cargo needed to write code); their build/run waits for the compute plan.

---

## 3. (b) Per-crate execution plans

Common shape for every crate:
**(i)** pre-kill tests (close the predictable gaps *before* the expensive run, so the
run measures the residue, not the known holes) → **(ii)** file-scoped mutation waves →
**(iii)** classify every survivor: KILL with a new test (default) / line-anchored
EXCLUDE with hand-verified justification / honest timeout-or-environment-only record →
**(iv)** re-run wave to green → **(v)** update config header + `docs/HARDENING.md` §2
measured-baseline section → commit.

### 3.1 celnet-qmc (new gate, est. ~350 mutants)

**New files**

- `.config/mutants-qmc.toml` — house-style header; gate command
  `cargo mutants -p celnet-qmc --test-tool=cargo --jobs 3 --config .config/mutants-qmc.toml`;
  `exclude_re = []` (strict).
- `justfile`:
  ```
  # Mutation GATE on the low-discrepancy sequence crate (zero non-equivalent survivors).
  mutants-gate-qmc:
      timeout 1800 {{_cargo}} mutants -p celnet-qmc --test-tool=cargo --jobs 3 \
          --config .config/mutants-qmc.toml
  ```
  and append `mutants-gate-qmc` to the `mutants-gate-numerics` aggregate.
- CI: add `celnet-qmc` to the `mutation-gate-numerics` matrix (the existing
  `cfg=".config/mutants-${crate#celnet-}.toml"` derivation already resolves it).

**Pre-kill tests** (new `crates/celnet-qmc/tests/sequence_oracle.rs` +
in-module additions):

1. `low_discrepancy_reference_points` — dimension 0 of `SobolSequence::point` is the
   base-2 radical-inverse sequence; points 1..=8 are the exact dyadic rationals
   `0.5, 0.25, 0.75, 0.125, 0.625, 0.375, 0.875, 0.0625`. Assert
   **`to_bits` equality** (they are exactly representable). Derivation in the test
   doc: `x_{i+1} = x_i ⊕ v_{ctz(i+1)}`, `v_k = m_k·2^{32−k}`, leading direction
   number `m_1 = 1` for every dimension ⇒ **first nonzero point is 0.5 in every
   dimension** — assert across dims `{1, 2, 7, 31, 63, MAX_DIM−1}`.
2. `direction_numbers_match_recurrence` — independent in-test re-implementation of
   the primitive-polynomial recurrence (provenance: the published Joe–Kuo D(6) table,
   prose only): for a sampled dimension set, recompute all 32 `v_k` from the
   polynomial degree/coefficients and initial `m`-values and compare to
   `direction_numbers(j)` **bit-for-bit**. This kills table-accessor and recurrence
   arithmetic mutants without touching the giant const table itself.
3. `gray_code_stream_equals_direct_points` — `SobolStream::next_point` over the first
   1,024 indices equals `point(i)` permuted per the Gray-code ordering contract the
   crate documents (or index-for-index if that is the contract — implementer pins
   whichever `sobol.rs` documents). Kills the XOR/ctz update internals.
4. `scrambled_point_is_deterministic_and_in_unit_cube` — same `(i, seed)` twice →
   `to_bits`-equal; different seeds differ somewhere in the first 64 points; all
   coordinates in `[0,1)`.
5. `inverse_normal_matches_reference` — pin `inv_norm_cdf` at published
   high-precision quantiles (`Φ⁻¹(0.5)=0`, `Φ⁻¹(0.975)=1.959963984540054`,
   symmetric negatives, `p = 1e-12` deep tail) to ≤ 1e-13 rel, plus the symmetry
   property `inv(p) == −inv(1−p)` on a grid, plus a round-trip against an
   **independent** in-test `Φ(x) = ½·erfc(−x/√2)` (via `libm::erfc` as dev-dep —
   code-disjoint from the rational-approximation implementation under test).
6. `bridge_factorization_reproduces_brownian_covariance` — the exact mathematical
   characterization: `L = weight_matrix()` must satisfy
   `(L·Lᵀ)[i][j] == min(t_i, t_j)` to ≤ 1e-12 for `m ∈ {1, 2, 3, 8, 64}` steps.
   Kills every arithmetic mutant in `BrownianBridge::new`/`build` in one shot
   (any wrong weight breaks the covariance). Additionally `build` on the standard
   basis vectors reproduces the columns of `L` (links `build` to `weight_matrix`).
7. `rqmc_estimate_integrates_monomials` — `∫₀¹ x dx = 1/2`, `∫₀¹ x² dx = 1/3` in
   dim 1–4 to ≤ 1e-4 with the standard config; `std_error` re-derived in-test from
   the replicate means (independent plain-loop variance).

**Expected equivalence clusters:** near none. Candidates: bit-loop guard flips that
are unreachable for validated `dim ≤ MAX_DIM` (audit individually); `debug_assert`
boundary mutants. **Policy: strict — expect 0–3 exclusions, each line-anchored.**

**Run:** single un-scoped run, `timeout 1800`, est. 30–60 min wall.

### 3.2 celnet-xva (est. ~300 mutants)

**Pre-kill tests** (new `crates/celnet-xva/tests/closed_form_oracle.rs`):

1. `cva_dva_fva_match_independent_quadrature` — with
   `ExposureProfile::deterministic(grid, epe, ene, r_dom)` and flat-hazard curves,
   re-derive in-test (plain loop, raw `exp`, no shared code):
   `CVA = LGD_c·Σ_k DF(t_k)·EPE(t_k)·(S_c(t_{k−1})−S_c(t_k))` (and the DVA/FVA
   analogues exactly as `cva.rs` documents its aggregation — the implementer copies
   the *documented formula from the doc comment*, re-deriving each factor with `exp`
   directly), assert ≤ 1e-12 rel. Drive at least 3 grids including a single-interval
   and a 100-point grid. Kills the discount/marginal-default/accumulation arithmetic.
2. `survival_curve_identities` — `flat(λ)`: `cumulative_hazard(t) == λ·t` (to bits
   where exact, else 1e-15 rel); `piecewise`: hazard integral at and between pillar
   boundaries vs independent partial-sum recomputation; `survival(t) ==
   exp(−cumulative_hazard(t))`; `marginal_default(a,b) == survival(a) − survival(b)`;
   monotonicity on a random grid; `survival(0) == 1.0` exactly.
3. `total_adjustment_sign_decomposition` — `total_adjustment() == cva − dva + fva`
   re-derived; CVA/DVA ≥ 0 on positive-EPE/negative-ENE profiles.
4. `exposure_simulation_is_bit_reproducible` — `ExposureProfile::simulate` with a
   frozen `(spot0, sigma, paths, seed)` twice → `epe`/`ene`/`discount` vectors
   `to_bits`-equal element-wise; plus **frozen-bits rows**: capture (once, during
   implementation, in a quiet window) the first/median/last `epe`/`ene` values for
   two fixed netting sets and pin them as `u64` bit constants in the test (the
   fx_byte_identity house pattern). This makes the whole low-discrepancy exposure
   path mutant-killable (seed/scramble/marking arithmetic all shift bits).
5. `netting_set_marks_match_vanilla_closed_form` — `NettedTrade::mark` vs an
   independent in-test undiscounted/discounted vanilla re-derivation at `τ = expiry −
   t_obs` (raw `Φ` via `erfc`), incl. the matured-trade `τ ≤ 0 ⇒ 0` branch and signed
   notional; `NettingSet::net_value` == plain in-test sum.

**Expected equivalence clusters:** the grid loop `<`/`<=` boundary at an exactly-full
buffer style mutants (kill by constructing exact-boundary grids first); possibly
`max(0.0)` floors on already-nonnegative EPE (kill via ENE-dominant sets that make the
floor bite). **Policy: kill-first; expect 0–5 exclusions.** Note `compute_xva`
*panics* on out-of-range LGD by contract — mutants weakening that assert are killed by
a `#[should_panic]` test on `lgd = 1.0 + ε` and `−ε`.

**Run:** single un-scoped run, `timeout 1800` (existing recipe gains
`--test-tool=cargo --jobs 3`, see §6), est. 30–60 min.

### 3.3 celnet-risk-cube (est. ~1,600 mutants, 4 file-scoped waves)

**Pre-kill tests**

1. `tests/frtb_param_provenance.rs` — every risk weight / correlation / curvature
   parameter in `frtb_params.rs` pinned to a literal typed **from the published
   Basel FRTB-SA text** (MAR21 paragraph cited in a comment per constant). Never
   computed from the crate. Kills all return-value mutants on the params surface and
   re-applies the 0.75ρ circular-oracle lesson.
2. Extend `tests/longhand_oracle.rs`:
   - `firm_aggregate_equals_naive_double_loop` — independent naive sum over every
     `RiskFact` for **each `NetGreeks` field** and the `VegaLadder`
     (`vega_in`/`total`), ≤ 1e-12 rel (float order may differ from the cube's merge
     order, so not bit-exact — state this in the test doc).
   - `group_by_partitions_conserve_the_firm_total` — Σ over `group_by(dim)` nodes ==
     `firm_aggregate` per dimension, all five `DimensionId`s.
   - `var_es_quantile_boundary_is_exact` — hand-built PnL vectors where the quantile
     index lands exactly on a tie and exactly between ranks; expected VaR/ES
     re-derived in-test by sort + tail-mean. Kills the `>=`/`>` index arithmetic
     that would otherwise be measure-zero-equivalent.
   - `taylor_pnl_matches_hand_expansion` — `PositionSensitivity::taylor_pnl` vs the
     in-test `Δ·dS + ½Γ·dS² + ν·dσ + (rate terms)` literal re-derivation.
   - `curvature_legs_match_two_revaluations` — `vanilla_curvature_legs` /
     `sbm_curvature_spot` vs direct up/down re-pricings through the same
     `CarryPricer` (the production `celnet_vanilla::FxPricer` — never a stub).
3. Scenario algebra (in-module, `nonadditive.rs` tests):
   - `base_scenario_is_bitwise_identity` — `Scenario::base().apply(i)` reproduces
     every field `to_bits`-equal (`x·1.0` and `x+0.0` are exact for finite positive
     inputs — the doc states the domain).
   - `fx_rate_mapping_reproduces_two_rate_shock` — `Scenario::fx_rates(s, v, dr_d,
     dr_f)` then `apply` equals the pre-generalization FX arithmetic re-derived
     in-test on `Carry::FxRates` **byte-for-byte** (the ADR-0008/W5-A contract; no
     asset-class match in the oracle either — it works on the `(r, b)` coordinates).
   - `shift_carry_matches_coordinate_arithmetic` per `Carry` arm.

**Waves** (each `cargo mutants -p celnet-risk-cube --file src/<glob> …`):

| Wave | Files | est. mutants | timebox |
|---|---|---|---|
| R1 | `dimension.rs`, `additive.rs`, `cube.rs` (zero in-module tests today) | ~420 | 90 min |
| R2 | `frtb_params.rs`, `frtb.rs` | ~480 | 90 min |
| R3 | `nonadditive.rs` | ~350 | 90 min |
| R4 | `exotic.rs`, `scenario_grid.rs`, `lib.rs` | ~350 | 90 min |

**Expected equivalence clusters + policy:**

- *Quantile/index boundary flips* — killable after the exact-tie tests above; not
  excluded.
- *`max(0.0)` curvature floors at exactly-zero* — measure-zero; exclude line-anchored
  after hand-verification (mirror the vanilla `>`↔`>=` cluster wording).
- *Symmetric correlation double-loop direction* — audit individually: a `j < i → j <= i`
  flip that double-counts the diagonal **changes the value** (caught); only a flip
  provably visiting the same unordered pair set is equivalent.
- *`gpu_pv_grid` (scenario_grid.rs)* — requires a live GPU device; the deterministic
  CPU reference `analytic_pv_grid` and `reconciles_to_analytic` are the testable
  surface. Mutants inside the device-orchestration path that no local `cargo test`
  executes are recorded as **environment-gated** (the loom/dir-fsync precedent):
  excluded with the honest justification that the `gpu-lavapipe` CI lane exercises the
  path, NOT claimed as suite-killable. Keep the exclusion set minimal — everything
  value-producing on the CPU side stays in the kill bar.

### 3.4 celnet-surface (est. ~2,300 mutants, 5 waves; arbitrage.rs already green)

**Pre-kill tests**

1. `parametric.rs`/`parametric_surface.rs`/`surface.rs`/`termstructure.rs` — pin each
   slice family's total-variance formula against a raw in-test recomputation (the
   published SVI/SSVI forms, provenance prose only), term-structure interpolation vs
   hand partial sums, `TenorPillar` boundary behavior at exact pillar times.
2. `quotes.rs` — `strike_at_delta` round-trip: re-evaluate the *delta* of the returned
   strike through `celnet-vanilla`'s delta (the independent direction) and assert it
   reproduces the target to solver tolerance; `atm_strike` per `AtmConvention` vs
   closed forms.
3. **`calibrate.rs` — the ~300-member cluster audit (the heart of this crate's work).**
   New `tests/fit_pins.rs`:
   - For each fitter (`fit_sabr`, `fit_svi`, `fit_ssvi`, `fit_essvi`) on **two frozen
     quote fixtures each** (a benign smile + a stressed skew):
     a. `converged_cost_is_pinned` — the achieved least-squares cost ≤ frozen cost
        + 1e-12 (catches any mutant that degrades the optimum — the largest
        sub-class: damping, step-acceptance, Jacobian sign errors);
     b. `converged_params_are_pinned` — params within 1e-9 abs of frozen values
        (catches different-basin convergence);
     c. `fitted_smile_reprices_anchors` — the calibrated slice reproduces the input
        quote vols at the anchor strikes to 1e-7 (the *economic* assertion).
     The fitters are deterministic (fixed `FIT_ITERS = 60`, fixed `FIT_FD_H`, no RNG),
     so (a)+(b) convert most "converging-optimizer-internal" survivors into kills:
     a trajectory perturbation either lands on the same optimum to 1e-9 (then it is a
     *genuine* equivalent and is excluded with that evidence) or it doesn't (killed).
   - `gaussian_eliminate_matches_dense_reference` — the in-crate `N×N` solver vs an
     in-test textbook Gaussian elimination with partial pivoting on random
     well-conditioned systems (≤ 1e-10), plus the singular-matrix `None` path.
4. `strangle.rs` — already has the two-direction bracket-expansion tests; add
   exact-boundary drives for any comparison the wave run surfaces.

**Waves:**

| Wave | Files | est. mutants | timebox |
|---|---|---|---|
| S1 | `mathx.rs`, `parametric_surface.rs`, `surface.rs`, `termstructure.rs`, `quotes.rs`, `parametric.rs`, `lib.rs` | ~530 | 2 h |
| S2 | `market_hedge.rs`, `stochvol.rs` | ~390 | 90 min |
| S3 | `extended_surface.rs` | ~310 | 90 min |
| S4 | `strangle.rs` | ~240 | 60 min |
| S5 | `calibrate.rs` | ~415 (contains the ~300 cluster) | 2 × 2 h |

**Equivalence policy for the calibrate cluster:** after `tests/fit_pins.rs`, expect the
~300 to split roughly 50–60 % **killed** / 40–50 % **audited equivalents** in 5–8 named
sub-clusters, each excluded line-anchored with the vanilla-config wording:
- seed/initial-guess values (converges from any in-domain start — evidenced by (b));
- FD step `FIT_FD_H` arithmetic (Jacobian direction preserved ⇒ same accepted steps to
  tolerance — evidenced by (a)+(b));
- damping expand/shrink factors (step acceptance is cost-monotone — evidenced by (a));
- `cost_new < cost` strict-vs-equal boundary (measure-zero float equality);
- projection/clamp internals whose bounds the fixtures never activate — first try to
  *activate* them with a stressed fixture; exclude only if provably unreachable for
  valid quotes.
Every exclusion records "Verified: full suite green with the mutant applied" only after
it actually was (hand-apply, `cargo test -p celnet-surface`, real exit code, in the
quiet window).
**The watchdog rule:** iteration-*cap* comparisons may be excluded if never hit;
iteration-*advance* mutants must be CAUGHT (timeout) — verify one per fitter.

### 3.5 celnet-exotics (est. ~4,500–5,000 mutants, 6 waves)

**The lever:** the 107 frozen `to_bits` gates pin the *price* of every MC/PDE/ADI/
American/composite/pivot path bit-for-bit on three fixtures (`fx_a/b/c`). Any mutant
that perturbs arithmetic on a priced path changes the bits ⇒ caught. The residual gap
classes are therefore: (1) modules **not** on the frozen-bits path (`particle`,
`leverage`, parts of `stochvol`/`lsv`/`adi`), (2) **auxiliary outputs** the gates don't
assert (`std_error`, Greeks strips, grid accessors, error paths), (3) validation /
defensive branches.

**Pre-kill tests** (new `crates/celnet-exotics/tests/engine_pins.rs` — deliberately a
*new* file: `fx_byte_identity.rs` is the frozen ADR-0008 migration gate and is never
edited):

1. `mc_std_error_pins` — freeze `McEstimate::std_error` bits for the same fixture grid
   the byte-identity file uses (capture once in a quiet window). Kills the
   variance-accumulation arithmetic the price-only gates miss.
2. `tridiagonal_solver_matches_dense_reference` — the PDE/ADI Thomas-algorithm pass vs
   an in-test dense LU on random diagonally-dominant systems (n ∈ {2, 3, 17, 101},
   ≤ 1e-11). A *fast* kill for `adi.rs`/`pde.rs` interior arithmetic that otherwise
   relies on slow full-engine runs.
3. `leverage_surface_interpolation_pins` — `LocalVolSurface`/`LeverageSurface` lookup
   vs hand bilinear/nearest re-derivation at interior, edge, and exactly-on-node
   points (the exact-node case kills `<`/`<=` bin-search flips).
4. `particle_calibration_pins` — (i) bit-reproducibility of `calibrate_leverage` for a
   frozen `ParticleConfig` (deterministic `CounterRng`); (ii) frozen-bits rows for the
   calibrated leverage at a few `(t, s)` nodes; (iii) the conditional-expectation
   estimator vs a brute-force in-test kernel sum on a tiny fixed particle set
   (independent plain-loop oracle).
5. `variance_step_moment_pins` — the QE variance step (`stochvol.rs`) vs the published
   scheme's exact conditional mean/variance re-derived in-test (provenance prose:
   Andersen's quadratic-exponential scheme), both branches of `QE_SWITCH`, plus the
   switch boundary exactly.
6. `american_reference_values` — `american_fd` vs published American-put reference
   values typed in-test (provenance prose: standard binomial/FD benchmark tables) at
   stated tolerance, complementing the bits pins with an *external* anchor; PSOR
   watchdog: one test asserting a pathological-but-valid grid still converges under
   `psor_max_iters`, and (during the audit) verify the cap-*advance* mutant hangs ⇒
   timeout-caught.
7. `seam_accessor_identities` (`inputs.rs`) — `discount_df == exp(−discount_rate·t)`,
   `carry_df_at`, `forward == spot·exp(carry_rate·t)` re-derived raw; the
   `as_fx_vanilla` round-trip on the FX arm `to_bits`-exact (it is the ADR-0008
   carry-seam identity).
8. Greeks-path pins where exported (`digital_greeks`, `american_fd_greeks`,
   `exotic_sensitivities_fd`): central-FD cross-check against the engine's own price
   function (independent direction: FD of price vs analytic greek).

**Waves** (gap-likelihood first; `--file` scoped; `--shard k/4` available for E3/E4 if
a window is short):

| Wave | Files | est. mutants | timebox |
|---|---|---|---|
| E1 | `particle.rs`, `leverage.rs`, `stochvol.rs` (weakest-tested, off the bits path) | ~400 | 2 h |
| E2 | `inputs.rs`, `payoff.rs`, `normal.rs`, `rng.rs` (the seam + primitives) | ~310 | 90 min |
| E3 | `adi.rs`, `pde.rs` | ~650 | 2 × 2 h |
| E4 | `american.rs` | ~450 | 2 h |
| E5 | `mc.rs`, `asian.rs`, `lookback.rs`, `forward_start.rs`, `tarf.rs`, `accumulator.rs`, `pivot.rs` (bits-pinned engines) | ~1,200 | 3 × 2 h |
| E6 | `barrier.rs`, `touch.rs`, `digital.rs`, `quanto.rs`, `var_swap.rs`, `vol_swap.rs`, `multiasset.rs`, `lsv.rs`, `market_hedge_overlay.rs`, `lib.rs` (golden/parity-gated closed forms) | ~1,600 | 3 × 2 h |

**Equivalence clusters + policy:**

- **MC seed/stream/step internals: NOT excluded.** The frozen bits kill them — this is
  precisely why this crate waited for the ADR-0008 tail. Any "MC is stochastic so it's
  equivalent" argument is rejected: the estimators are deterministic functions of
  `(inputs, McConfig)`.
- **Watchdog caps** (`psor_max_iters`, MC/LSM iteration guards): cap-boundary flips
  excluded after hand-verification; cap-advance mutants must be timeout-caught
  (verified per module, recorded as the honest timeout class in the config header).
- **Convergence-tolerance boundary flips** (`< tol` → `<= tol`): measure-zero;
  line-anchored exclusions after hand-verify.
- **Defensive clamps / validated-input guards**: first try to kill with an in-domain
  adversarial test; exclude only with a proof of unreachability post-validation.
- **`rng.rs` rotation/multiply constants**: killable via frozen stream bits (pre-kill
  test 4i analog for raw `CounterRng` output) — not excluded.

---

## 4. (c) Fuzz-target designs (4 new targets, house style)

All four follow the `vanilla_inputs` pattern: `#![no_main]`, `libfuzzer-sys 0.4`,
structured **in-domain** draws via `arbitrary::Unstructured` + the `clamp_into` tanh
squash (copy the helper; keep `libm`), `Debug` on the draw struct, contracts asserted
with `assert!`. Each is mirrored by a **proptest suite in the owning crate's `tests/`**
so the property gates `just check` on stable (the decode_fuzz precedent). No product
logic is re-implemented; the harness depends on product crates by path.

**`fuzz/Cargo.toml` additions** — dependencies: `celnet-surface`, `celnet-conventions`,
`celnet-core`, `celnet-risk-cube`, `celnet-risk-normalize`, `celnet-exotics`,
`celnet-xva` (all by `path = "../crates/…"`); four `[[bin]]` entries
(`test/doc/bench = false`). `fuzz/README.md` table gains four rows. CI `fuzz` job gains
four 30 s-budget steps.

### 4.1 `surface_quote_grid` — calibration on adversarial broker grids

```rust
struct Draw {
    spot: f64,            // clamp_into 1e-3 .. 1e4
    t: f64,               // clamp_into 1/365 .. 30.0
    carry: Carry,         // FxRates { r_dom, r_for } each in −0.25 .. 0.25
    atm_vol: f64,         // clamp_into 1e-4 .. 3.0
    rr_25: f64, bf_25: f64,            // clamp_into −0.5 .. 0.5  (sign-adversarial)
    outer: Option<(f64, f64)>,         // 10Δ rr/bf, same range
    pair_pick: u8,        // selects a ConventionRecord from a fixed resolved set
}
```
Builds `MarketContext::new(spot, carry, t, conventions)` +
`MarketQuotes::{three_point, five_point}`, then drives **`build_smile_and_outer`**.
Contract:
- never panics — `Ok` or a typed `CalibrationError`, on *any* in-domain quote grid
  (including economically absurd RR/BF: the library promise documented in `lib.rs` is
  "a degenerate set surfaces as a calibration error, never a panic");
- `Ok(smile)` ⇒ smile vol at the three anchor strikes and on a 21-point strike scan
  around the forward is finite and strictly positive (no NaN poisoning);
- `Ok` ⇒ `check_slice` on the scanned grid returns a finite `ArbitrageReport`
  (fields finite — the report may legitimately *flag* arbitrage; it must never NaN).

Stable mirror: `crates/celnet-surface/tests/quote_grid_fuzz.rs` (proptest, 512 cases,
same draw ranges, same three assertions).

### 4.2 `risk_cube_scenarios` — scenario algebra + roll-up conservation

```rust
struct Draw {
    positions: Vec<PosDraw>,   // 1..=24; PosDraw → PositionRisk over CarryInputs
                               //   (spot/strike/vol/t clamped in-domain, Carry::FxRates)
    scenarios: Vec<ScenDraw>,  // 1..=16; spot_rel −0.5..1.0, vol_abs −0.05..0.25,
                               //   discount_abs/carry_abs −0.05..0.05
}
```
Pricer: the **production** `celnet_vanilla::FxPricer` (`CarryPricer` impl) — no stub.
Contracts (each is a documented algebraic identity, asset-class-agnostic on the carry
seam — the oracle never matches on underlying):
- `Scenario::base().apply(i)` reproduces every `CarryInputs` field `to_bits`-equal;
- `Scenario::fx_rates(s, v, dr_d, dr_f)` produces `discount_abs == dr_d` and
  `carry_abs == dr_d − dr_f` exactly (`to_bits`), and `apply` shifts the `(r, b)`
  coordinates by exactly those amounts;
- `position_pnl` and `node_pnl` are finite for every (position, scenario);
- `node_pnl(positions, s) == Σ position_pnl(p, s)` within 4 ULP-scaled tolerance
  (`≤ 1e-12·(1 + Σ|pnl|)` — summation order may differ);
- `historical_var_es` over the drawn scenario set: `es >= var`, both finite, and both
  reproduce an independent in-target sort-and-tail-mean recomputation to 1e-12 rel.

Stable mirror: `crates/celnet-risk-cube/tests/scenario_fuzz.rs`.

### 4.3 `xva_netting` — netting/exposure/adjustment contracts

```rust
struct Draw {
    trades: Vec<TradeDraw>,        // 1..=16 NettedTrade: strike/vol/expiry in-domain,
                                   //   notional clamp_into −1e7..1e7 (signed: net books)
    r_dom: f64, r_for: f64,        // −0.25 .. 0.25
    spot0: f64, sigma: f64,        // in-domain
    paths_pow: u8,                 // paths = 64 << (paths_pow % 3)  — bounded work
    seed: u64,
    lambda_cpty: f64, lambda_own: f64, // clamp_into 1e-6 .. 2.0  (flat hazards)
    lgd_c: f64, lgd_o: f64,        // clamp_into 0.0 .. 1.0  (the documented domain)
    funding_spread: f64,           // clamp_into −0.05 .. 0.05
    steps: u8,                     // 4 ..= 32 grid points
}
```
Drives `NettingSet::new` → `ExposureProfile::simulate` → `compute_xva`. Contracts:
- no panic anywhere on the in-domain draw (LGD is clamped into `[0,1]`, the documented
  precondition — the panic contract itself is unit-tested, not fuzzed);
- profile invariants: `grid[0] == 0.0`; `epe[k] >= 0`, `ene[k] <= 0`, all finite;
  `discount[k] == exp(−r_dom·t_k)` to 1e-12;
- determinism: same draw ⇒ `simulate` bit-identical (re-run inside the target);
- `XvaResult`: `cva >= 0`, `dva >= 0`, all three finite;
  `total_adjustment() == cva − dva + fva` exactly;
- comparative monotonicity: doubling `lambda_cpty` (re-running `compute_xva` on the
  same profile) does not *decrease* CVA (survival mass only moves earlier; loss weight
  `S(t_{k−1})−S(t_k)` integrates a non-negative EPE — stated as ≥ −1e-12 slack).

Stable mirror: `crates/celnet-xva/tests/netting_fuzz.rs`.

### 4.4 `exotic_payoff_bounds` — model-free bounds under arbitrary-but-finite inputs

```rust
enum FamilyDraw {                  // arbitrary-derived discriminant
    SingleBarrier { kind: BarrierKind, style: BarrierStyle, barrier_rel: f64, rebate: f64 },
    DoubleKnockOut { lo_rel: f64, hi_rel: f64 },
    Digital { kind: DigitalKind, style: DigitalStyle },
    Touch { side: TouchSide, timing: RebateTiming },
    DoubleNoTouch { lo_rel: f64, hi_rel: f64 },
    AnalyticAsian { fixings: u8 },
    LookbackClosedForm { style: LookbackStyle },
    ForwardStart { start_frac: f64, strike_frac: f64 },
    QuantoVanilla { fx_vol: f64, rho: f64 },
}
struct Draw { opt: OptionType, inputs_raw: [f64; 6], family: FamilyDraw }
```
`inputs_raw` folds into a `VanillaInputs::new(spot, strike, vol, t, r_dom, r_for)` via
`clamp_into` (the `vanilla_inputs` envelope), then `.into()` an `ExoticInputs` — the
exact carry-seam conversion the byte-identity gate uses. Barriers/touch levels are
placed *relative* to spot (including straddling-the-spot adversarial placements that
the constructors must reject as typed errors, never panic). Contracts per family
(closed forms only — no MC in the fuzz loop, budget stays on the analytics):
- **universal:** no panic; every price finite; every price ≥ 0;
- **single barrier:** `knock_in + knock_out == vanilla` (in/out parity, with zero
  rebate) to 1e-9 rel; each leg `<=` vanilla + 1e-12;
- **double KO:** `0 <= dko <= vanilla`;
- **digital cash:** `0 <= price <= discount_df()`; asset digital `<= spot·carry-adjusted
  bound` (use `inputs.forward()·discount_df()` + 1e-12);
- **touch:** `one_touch + no_touch == discounted-rebate parity` per `RebateTiming`
  to 1e-9; `0 <= DNT <= discount_df`;
- **lookback:** floating-strike lookback ≥ the corresponding vanilla − 1e-12
  (the max/min dominates the terminal fixing);
- **analytic Asian:** `0 <= price`, finite; geometric `<=` arithmetic estimator
  + 1e-9 (AM–GM on the average);
- **forward start / quanto closed forms:** finite, ≥ 0, and quanto with `rho = 0,
  fx_vol = 0` reproduces the un-quantoed price to 1e-10 (degenerate-parameter
  consistency).

Stable mirror: `crates/celnet-exotics/tests/payoff_bounds_fuzz.rs` (proptest; the
in/out and touch parities double as fast mutation killers for `barrier.rs`/`touch.rs`).

---

## 5. (d) Compute plan

**Protocol (single M4, shared with session-B):**

- **Never start while session-B builds.** Before any wave: `pgrep -fl 'cargo|rustc|node'`
  must show no session-B activity twice, 10 minutes apart (the sustained-quiet rule
  from the mesh notes). Never `pkill` anything.
- **One cargo-mutants process at a time**, always `--test-tool=cargo --jobs 3`
  (plain cargo runner: reproducible, nextest-kill-proof; jobs 3 is the audited
  courteous cap). Long waves run detached with a hard `timeout` (the justfile values)
  so an unattended window can never wedge the machine past its box.
- `PROPTEST_MAX_SHRINK_ITERS=0` on any wave whose killing suite includes proptest
  (surface, exotics after the mirrors land) — a guaranteed-failing case must fail
  fast, not shrink for minutes (the router-gate precedent). Add
  `--minimum-test-timeout=60` where MC tests run multi-second baselines.
- **Durability per wave:** each green wave commits (tests + config delta + HARDENING
  baseline numbers) before the next starts — a cleared session loses nothing.
- Fuzz building/running is nightly + cargo-fuzz and is **Linux-CI work**; locally only
  the stable proptest mirrors run (cheap, inside the per-crate `cargo test`).

**Order when the machine frees (each item = one quiet window unless noted):**

| Slot | Work | Budget |
|---|---|---|
| 0 | Author everything that needs no cargo *now*, during session-B's window: all pre-kill tests (§3), fuzz targets + mirrors (§4), `.config/mutants-qmc.toml`, justfile/CI edits | 0 compute |
| 1 | `cargo test -p celnet-qmc -p celnet-xva -p celnet-risk-cube -p celnet-surface -p celnet-exotics` (plain cargo, serial) — prove all new tests green + capture the frozen-bits constants (§3.2-4, §3.5-1/4) | ~30 min |
| 2 | qmc full run → audit → green → commit | 30–60 min |
| 3 | xva full run → audit → green → commit | 30–60 min |
| 4–7 | risk-cube R1→R4 (audit + re-run each) | 4 × 90 min |
| 8–12 | surface S1→S5 (S5 twice: raw, then post-audit re-run) | ~9 h total |
| 13–25 | exotics E1→E6 (`--shard k/4` inside a short window if needed) | ~14–18 h total |
| 26 | Full-crate confirm runs of any file-scoped crate (risk-cube, surface, exotics un-scoped with final configs) — the gate that CI will repeat | 3 windows |
| 27 | Final: `just mutants-gate-numerics` definition now includes qmc; update HARDENING/ledger; cross-crate `just check` before the milestone commit | 1 window |

Estimated mutant wall-time at ~8–15 s/mutant effective (jobs 3, incremental scratch
build): qmc+xva ≈ 1.5–3 h; risk-cube ≈ 4–6 h; surface ≈ 6–9 h; exotics ≈ 12–20 h.
These are background-window hours spread over days; nothing blocks session-B.

---

## 6. (e) Gate definition per crate (the "done" bar)

A crate is DONE when **all** of:

1. **Config** — `.config/mutants-<crate>.toml` exists, house-style header documents
   the command, killing suite, independent oracle, and the MEASURED baseline; every
   `exclude_re` member is line-anchored, individually hand-reproduced
   ("Verified: full suite green with the mutant applied" only when literally true),
   and belongs to a named, justified equivalence cluster. Timeout-class and
   environment-only (GPU) catches recorded honestly, never hidden.
2. **Zero non-equivalent survivors** — `just mutants-gate-<crate>` (recipe carries
   `--test-tool=cargo --jobs 3` + the timeout box; **migrate the four existing
   numerics recipes to these flags** as part of slot 0, mirroring the infra gates)
   exits **0**, with the literal cargo-mutants summary line captured in
   HARDENING.md (`N mutants tested … 0 missed`). Verify the real exit code, not the
   log tail (the hard gate lesson).
3. **Plain-cargo green** — `cargo test -p <crate>` (the gate's own runner) passes
   standalone, real exit 0.
4. **Fuzz floor** (where the crate owns a target, §4) — the libFuzzer target is wired
   in `fuzz/Cargo.toml` + CI with a smoke budget, and the **stable proptest mirror**
   is green inside the crate's `cargo test` (the mirror is the merge-blocking gate).
5. **Docs current** — HARDENING.md §2 gains/updates the crate's measured-baseline
   subsection (counts, kill-rate, cluster table — never fabricated for a run not
   performed); `fuzz/README.md` + the CI workflow list the new targets; the
   implementation ledger gets the wave entry; the resume anchor is replaced in place.
6. **Byte-identity unbroken** — `cargo test -p celnet-exotics --test fx_byte_identity`
   passes bit-for-bit at every commit of this lane (it is part of every exotics wave's
   killing suite by construction, and is re-asserted explicitly in the final gate).

Aggregate exit criterion for the lane: `just mutants-gate-numerics` (now 6 crates:
vanilla + surface + exotics + risk-cube + xva + **qmc**) green end-to-end in one quiet
window, plus the four new fuzz mirrors green inside `just check`.

---

## 7. Dependency order (implementation sequence)

1. **Slot-0 authoring (no cargo):**
   a. `.config/mutants-qmc.toml`; justfile: `mutants-gate-qmc`, add qmc to
      `mutants-gate-numerics`, add `--test-tool=cargo --jobs 3` to the four existing
      numerics gate recipes; CI: add `celnet-qmc` to the `mutation-gate-numerics`
      matrix and four fuzz steps to the `fuzz` job.
   b. Pre-kill test files: `celnet-qmc/tests/sequence_oracle.rs`,
      `celnet-xva/tests/closed_form_oracle.rs`,
      `celnet-risk-cube/tests/frtb_param_provenance.rs` + longhand/in-module
      extensions, `celnet-surface/tests/fit_pins.rs` + small-module pins,
      `celnet-exotics/tests/engine_pins.rs` + in-module additions.
      (Frozen-bits constants left as named `const` placeholders is NOT allowed —
      instead structure those tests so the constants are appended in slot 1 when the
      capture run executes; commit them only with real captured values.)
   c. Fuzz targets `fuzz/fuzz_targets/{surface_quote_grid,risk_cube_scenarios,xva_netting,exotic_payoff_bounds}.rs`,
      `fuzz/Cargo.toml` deps + bins, `fuzz/README.md` rows, and the four stable
      proptest mirrors in the owning crates.
2. **Slot 1:** serial plain-cargo test pass over the five crates; capture and commit
   the frozen-bits pins.
3. **Slots 2–25:** mutation waves in §5 order; per-wave audit → kill/exclude →
   re-run → HARDENING + config + commit.
4. **Slot 26–27:** un-scoped confirm runs, `just mutants-gate-numerics`, full
   `just check`, ledger + resume-anchor update, milestone commit.

Cross-lane note: everything here is additive (tests/configs/fuzz/docs) inside crates
this lane owns; no interface crate changes; no proto window needed; zero overlap with
session-B's payoff-shapes lane.
