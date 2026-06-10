//! Parity row — **Local-Stochastic-Volatility** (LSV) booking model
//! (`celnet_exotics::lsv`).
//!
//! The full LSV engine — the [`celnet_exotics::LsvModel`] orchestration over the
//! square-root variance backbone (`stochvol`), the Dupire local-vol / leverage
//! surface (`leverage`), the interacting-particle leverage calibration
//! (`particle`), the 2-D Hundsdorfer–Verwer ADI PDE (`adi`) and the
//! counter-based Monte-Carlo engine — has unit tests *inside* its own crate, but
//! had **no** `celnet-parity` row validating it against a *genuinely independent*
//! oracle (guardrail #5). This is that row.
//!
//! Method provenance (doc comments only, per the naming guardrail): LSV
//! construction & the leverage identity — Ren-Madan-Qian (2007), Guyon &
//! Henry-Labordère (2012); variance backbone — Heston (1993); QE discretisation —
//! Andersen (2008); 2-D ADI — Craig-Sneyd (1988), Hundsdorfer-Verwer (2003),
//! in 't Hout-Foulon (2010); Dupire local vol — Dupire (1994), Gatheral (2006).
//!
//! # The four checks, each on a DIFFERENT, independent oracle
//!
//! 1. **Vanilla-surface reprice.** Calibrate the LSV (with *genuine* vol-of-
//!    variance ξ>0) to a flat-σ implied surface, then reprice the calibration
//!    strikes on the ADI PDE **and** the MC engine and recover the surface-implied
//!    Black/Garman-Kohlhagen vanilla. The oracle is the **surface vols themselves**
//!    (re-priced under the closed-form GK formula `celnet_vanilla::price`), which is
//!    independent of the LSV solver — the leverage calibration's *entire job* is to
//!    make the model's marginals reproduce that surface. Band stated honestly below.
//!
//! 2. **ξ=0 → Dupire / local-vol limit, HAND-PINNED.** With vol-of-variance ξ→0
//!    and v0=θ=σ² the variance is deterministic and the model collapses to *pure
//!    local vol*; on a **flat** implied surface the Dupire local vol equals the flat
//!    σ, so the ξ=0 LSV European PDE price equals the **exact** Black/GK closed
//!    form. We pin that not just to `celnet_vanilla::price` (which could in
//!    principle share a mis-stated constant with the solver) but to **constants
//!    hand-computed by an independent Python `math.erf` Black/GK evaluation**
//!    (derivations in [`Gk`] below). We also verify the **O(ξ²)** approach rate:
//!    halving ξ shrinks the deviation from the local-vol price by ≈4×.
//!
//! 3. **PDE ≈ MC** on a 2nd-generation **window knock-out barrier**. The ADI PDE
//!    and the Monte-Carlo engine are two *genuinely independent* numerical methods
//!    (finite-difference backward induction vs forward path simulation with a
//!    Brownian-bridge survival weight). Agreement within the MC reported standard
//!    error (plus a small documented discretisation slack) validates both.
//!
//! 4. **Published-fixture honesty.** There is **no** canonical, exactly-citable
//!    published LSV *window-barrier* price for this specific (FX carry, window,
//!    QE-ADI scheme) configuration — the Guyon & Henry-Labordère (2012) and
//!    Ren-Madan-Qian (2007) papers report calibration quality and equity-index
//!    barrier figures under different conventions and discretisations, not a
//!    number reproducible to method tolerance here. Rather than fabricate a
//!    reference (which the guardrails forbid), checks (1)–(3) carry the validation,
//!    anchored by the **hand-pinned GK constants** of check (2) — a number the LSV
//!    solver and any internal oracle cannot *both* silently mis-state.
//!
//! Plus **structural invariants**: an LSV knock-out barrier is ≤ the unbarriered
//! vanilla; a *window* (partially-active) barrier is ≥ the corresponding *full-
//! life* barrier; both are non-negative; the MC price is bit-reproducible.

use celnet_exotics::{
    AdiGrid, LsvModel, McConfig, ParticleConfig, VarianceParams, WindowBarrier,
    leverage::ImpliedVolSurface,
};
use celnet_types::{OptionType, VanillaInputs};

// ---------------------------------------------------------------------------
// Independent oracle 1: a self-contained Black / Garman-Kohlhagen closed form,
// written here from scratch (NOT calling celnet_vanilla), used to cross-check the
// production analytic pricer and to host the hand-pinned constants.
// ---------------------------------------------------------------------------

/// A standalone Garman-Kohlhagen evaluator — an independent re-implementation of
/// the FX-vanilla closed form, kept inside this test so it shares *no* code with
/// the LSV solver or with `celnet_vanilla`. Used (a) to derive the band targets
/// and (b) to confirm the production pricer agrees with the hand-pinned numbers.
struct Gk;

impl Gk {
    /// Standard normal CDF via the error function (`libm::erf`), the same special
    /// function the Python oracle used to pin the constants.
    fn cdf(x: f64) -> f64 {
        0.5 * (1.0 + libm::erf(x / core::f64::consts::SQRT_2))
    }

    /// Closed-form Garman-Kohlhagen price.
    fn price(opt: OptionType, s: f64, k: f64, sigma: f64, t: f64, r_d: f64, r_f: f64) -> f64 {
        let f = s * libm::exp((r_d - r_f) * t);
        let df = libm::exp(-r_d * t);
        let sqt = sigma * t.sqrt();
        let d1 = ((f / k).ln() + 0.5 * sigma * sigma * t) / sqt;
        let d2 = d1 - sqt;
        match opt {
            OptionType::Call => df * (f * Self::cdf(d1) - k * Self::cdf(d2)),
            OptionType::Put => df * (k * Self::cdf(-d2) - f * Self::cdf(-d1)),
        }
    }
}

// ---------------------------------------------------------------------------
// HAND-PINNED CONSTANTS (Lesson c).
//
// Computed by an INDEPENDENT Python `math.erf` Black/Garman-Kohlhagen evaluation,
// OUTSIDE the Rust codebase, for the flat-σ market:
//   S = 1.30, σ = 0.10, T = 1.0, r_d = 0.03, r_f = 0.01  (so carry b = 0.02,
//   forward F = 1.30·e^{0.02} = 1.32626174… (e^{0.02} = 1.02020134…).
//
//   ATM call  (K=1.30): 0.06457179059698764
//   ATM put   (K=1.30): 0.039086200336129764
//   OTM call  (K=1.40): 0.02450280708906806
//   OTM put   (K=1.20): 0.010183667558081791
//
// Because these are pinned to an external evaluator, neither the LSV solver nor the
// production `celnet_vanilla::price` can silently mis-state them — a shared bug in
// the Rust GK path would break check (2a).
// ---------------------------------------------------------------------------

/// Hand-computed (Python `math.erf`) GK reference prices for the flat-σ market.
mod pinned {
    pub(super) const ATM_CALL_K130: f64 = 0.064_571_790_596_987_64;
    pub(super) const ATM_PUT_K130: f64 = 0.039_086_200_336_129_764;
    pub(super) const OTM_CALL_K140: f64 = 0.024_502_807_089_068_06;
    pub(super) const OTM_PUT_K120: f64 = 0.010_183_667_558_081_791;
}

// ---------------------------------------------------------------------------
// Market & calibration helpers.
// ---------------------------------------------------------------------------

const SPOT: f64 = 1.30;
const SIGMA: f64 = 0.10;
const R_DOM: f64 = 0.03;
const R_FOR: f64 = 0.01;
const HORIZON: f64 = 1.0;

/// EURUSD-like market: spot 1.30, 1Y, r_d 3 %, r_f 1 %. (`strike`/`vol` fields are
/// placeholders for the LSV calibration entry; the smile lives in the surface.)
fn market() -> VanillaInputs {
    VanillaInputs::new(SPOT, SPOT, SIGMA, HORIZON, R_DOM, R_FOR)
}

/// A flat (constant-σ) implied-vol surface adapter. Its Dupire local vol is the
/// constant σ everywhere, the textbook limit that hosts the hand-pinned check.
struct FlatIv {
    sigma: f64,
    spot: f64,
    carry: f64,
}
impl ImpliedVolSurface for FlatIv {
    fn implied_vol(&self, _k: f64, _t: f64) -> f64 {
        self.sigma
    }
    fn forward(&self, t: f64) -> f64 {
        self.spot * libm::exp(self.carry * t)
    }
}

/// Log-spaced leverage spot grid spanning ≈ ±5 standard deviations around spot.
fn spot_grid() -> Vec<f64> {
    (0..41)
        .map(|k| SPOT * libm::exp(-0.6 + 0.03 * k as f64))
        .collect()
}

/// A reasonably resolved ADI grid for the European/limit checks.
fn euro_grid() -> AdiGrid {
    AdiGrid {
        x_steps: 160,
        v_steps: 44,
        time_steps: 100,
        ..AdiGrid::default()
    }
}

// ===========================================================================
// ROW 1 — vanilla-surface reprice (ξ>0): PDE AND MC recover the surface vanilla.
//
// Independent oracle: the surface's own Black/GK vanilla (closed form, `Gk`),
// which knows nothing of the LSV solver. With genuine stochastic variance the
// particle-calibrated leverage must absorb the vol-of-variance so the model's
// marginal still reproduces the flat-σ surface.
//
// Documented band: the calibrated-LSV repricing band is wider than the ξ=0 limit
// because it carries three independent approximation sources stacked together —
// the finite particle ensemble (calibration MC error), the ADI/MC discretisation,
// and the Dupire finite-difference spacings. We state it honestly at:
//   * PDE: ≤ 2.0e-2 absolute (ATM and the calibration wings);
//   * MC : ≤ max(4·stderr, 6.0e-3) absolute.
// These are *approximation bands*, explicitly NOT a closed-form-precision claim.
// ===========================================================================

#[test]
fn calibrated_lsv_reprices_surface_vanilla_pde_and_mc() {
    let i = market();
    let carry = R_DOM - R_FOR;
    let iv = FlatIv {
        sigma: SIGMA,
        spot: SPOT,
        carry,
    };
    // Genuine, Feller-respecting stochastic variance (2κθ = 0.04 ≥ ξ² = 0.01).
    let var = VarianceParams::new(SIGMA * SIGMA, 2.0, SIGMA * SIGMA, 0.10, -0.3);
    let grid = spot_grid();
    let model = LsvModel::calibrate(
        (&i).into(),
        var,
        &iv,
        &grid,
        ParticleConfig {
            particles: 30_000,
            steps: 40,
            seed: 9,
            ..ParticleConfig::default()
        },
    );
    let agrid = euro_grid();

    // Reprice the calibration strikes; oracle = surface-flat-σ closed-form GK.
    let cases = [
        (OptionType::Call, SPOT),        // ATM
        (OptionType::Put, SPOT),         // ATM put
        (OptionType::Call, SPOT * 1.05), // OTM call wing
        (OptionType::Put, SPOT * 0.95),  // OTM put wing
    ];

    // PDE leg.
    for (opt, k) in cases {
        let gk = Gk::price(opt, SPOT, k, SIGMA, HORIZON, R_DOM, R_FOR);
        let pde = model.price_european_pde(opt, k, agrid);
        assert!(
            (pde - gk).abs() < 2.0e-2,
            "ROW1 PDE {opt:?} K={k}: LSV {pde} vs surface-GK {gk} (band 2e-2)"
        );
    }

    // MC leg — a second, independent engine reprices the same surface vanilla.
    let mc_cfg = McConfig {
        pairs: 40_000,
        steps: 80,
        seed: 0xA11CE,
    };
    for (opt, k) in cases {
        let gk = Gk::price(opt, SPOT, k, SIGMA, HORIZON, R_DOM, R_FOR);
        let mc = model.price_european_mc(opt, k, mc_cfg);
        let band = (4.0 * mc.std_error).max(6.0e-3);
        assert!(
            (mc.price - gk).abs() < band,
            "ROW1 MC {opt:?} K={k}: LSV {} vs surface-GK {gk} (se {}, band {band})",
            mc.price,
            mc.std_error
        );
    }
}

// ===========================================================================
// ROW 2a — ξ=0 pure-local-vol limit, HAND-PINNED to external GK constants.
//
// With ξ=0, v0=θ=σ² the variance is deterministic; on a FLAT surface Dupire LV ==
// flat σ, so the ξ=0 LSV PDE price == the exact Black/GK closed form. We pin to the
// Python-`math.erf` hand-computed constants (the strongest, non-circular oracle).
//
// Documented band: the ξ=0 ADI PDE on a 140×30×80 grid recovers the closed form to
// finite-difference accuracy. We gate at ≤ 5e-3 absolute — the same order the
// engine's own in-crate limit test uses — explicitly a *discretisation* band, not
// an exact-arithmetic claim. We ALSO confirm the production `celnet_vanilla::price`
// matches the hand-pinned constants to ≤ 1e-12 (so the Rust analytic path is not
// silently off), making the band attributable purely to ADI discretisation.
// ===========================================================================

#[test]
fn xi_zero_limit_matches_hand_pinned_gk() {
    // First: the independent in-test `Gk` and the hand-pinned Python constants
    // agree to closed-form precision — this validates the oracle itself.
    assert!(
        (Gk::price(OptionType::Call, SPOT, SPOT, SIGMA, HORIZON, R_DOM, R_FOR)
            - pinned::ATM_CALL_K130)
            .abs()
            < 1e-14,
        "in-test GK must reproduce the hand-pinned ATM call constant"
    );
    assert!(
        (Gk::price(OptionType::Put, SPOT, SPOT, SIGMA, HORIZON, R_DOM, R_FOR)
            - pinned::ATM_PUT_K130)
            .abs()
            < 1e-14
    );
    assert!(
        (Gk::price(OptionType::Call, SPOT, 1.40, SIGMA, HORIZON, R_DOM, R_FOR)
            - pinned::OTM_CALL_K140)
            .abs()
            < 1e-14
    );
    assert!(
        (Gk::price(OptionType::Put, SPOT, 1.20, SIGMA, HORIZON, R_DOM, R_FOR)
            - pinned::OTM_PUT_K120)
            .abs()
            < 1e-14
    );

    // Second: the production analytic pricer matches the hand-pinned constants to
    // ≤1e-12 — so any residual in the PDE comparison below is ADI discretisation,
    // not a mis-stated Rust GK constant shared with the solver.
    assert!(
        (celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(SPOT, SPOT, SIGMA, HORIZON, R_DOM, R_FOR)
        ) - pinned::ATM_CALL_K130)
            .abs()
            < 1e-12,
        "production celnet_vanilla must match the hand-pinned GK constant"
    );

    // Third: the ξ=0 LSV PDE reprices each hand-pinned constant within the
    // documented discretisation band.
    let i = market();
    let carry = R_DOM - R_FOR;
    let iv = FlatIv {
        sigma: SIGMA,
        spot: SPOT,
        carry,
    };
    // ξ = 0 (pure local vol), v0 = θ = σ². κ is positive but irrelevant since the
    // variance is pinned at θ when v0 = θ and ξ = 0.
    let var = VarianceParams::new(SIGMA * SIGMA, 1.0, SIGMA * SIGMA, 0.0, 0.0);
    let grid = spot_grid();
    let model = LsvModel::calibrate(
        (&i).into(),
        var,
        &iv,
        &grid,
        ParticleConfig {
            particles: 8_000,
            steps: 20,
            seed: 5,
            ..ParticleConfig::default()
        },
    );
    let agrid = AdiGrid {
        x_steps: 140,
        v_steps: 30,
        time_steps: 80,
        ..AdiGrid::default()
    };

    let pinned_cases = [
        (OptionType::Call, SPOT, pinned::ATM_CALL_K130),
        (OptionType::Put, SPOT, pinned::ATM_PUT_K130),
        (OptionType::Call, 1.40, pinned::OTM_CALL_K140),
        (OptionType::Put, 1.20, pinned::OTM_PUT_K120),
    ];
    for (opt, k, pinned_px) in pinned_cases {
        let pde = model.price_european_pde(opt, k, agrid);
        assert!(
            (pde - pinned_px).abs() < 5e-3,
            "ROW2a {opt:?} K={k}: ξ=0 LSV PDE {pde} vs hand-pinned GK {pinned_px} (band 5e-3)"
        );
    }
}

// ===========================================================================
// ROW 2b — O(ξ²) approach rate to the local-vol limit.
//
// The leverage absorbs the leading stochastic-variance effect, so on a flat
// surface the residual European mispricing vs the local-vol (ξ=0) price is
// second order in the vol-of-variance: E(ξ) ≈ c·ξ². Halving ξ should shrink the
// residual by ≈4×. We assert the ratio E(ξ)/E(ξ/2) ≳ 3.0 (allowing slack for the
// finite particle ensemble + ADI/Dupire discretisation noise floor), which is
// only achievable if the dependence is genuinely super-linear (O(ξ²)), not O(ξ).
//
// Oracle: the same flat-σ surface GK price (independent of the solver). The ξ=0
// price is itself computed by the engine, so this is a *self-consistency rate*
// check on the engine's convergence — complementary to the absolute hand-pinned
// check above, and reported honestly as such.
// ===========================================================================

#[test]
fn local_vol_limit_approach_is_second_order_in_vol_of_var() {
    let i = market();
    let carry = R_DOM - R_FOR;
    let iv = FlatIv {
        sigma: SIGMA,
        spot: SPOT,
        carry,
    };
    let grid = spot_grid();
    let agrid = euro_grid();
    let gk_atm = Gk::price(OptionType::Call, SPOT, SPOT, SIGMA, HORIZON, R_DOM, R_FOR);

    // Deviation from the GK (local-vol) price at a given ξ, calibrated freshly.
    let deviation = |xi: f64| -> f64 {
        let var = VarianceParams::new(SIGMA * SIGMA, 2.0, SIGMA * SIGMA, xi, -0.3);
        let model = LsvModel::calibrate(
            (&i).into(),
            var,
            &iv,
            &grid,
            ParticleConfig {
                particles: 30_000,
                steps: 40,
                seed: 21,
                ..ParticleConfig::default()
            },
        );
        (model.price_european_pde(OptionType::Call, SPOT, agrid) - gk_atm).abs()
    };

    let e_big = deviation(0.20);
    let e_small = deviation(0.10);
    // O(ξ²) ⇒ ratio ≈ 4; require ≳ 3 to robustly exclude O(ξ) (ratio ≈ 2) while
    // tolerating the calibration/discretisation noise floor. Guard against a
    // degenerate (both ≈ 0) measurement.
    assert!(
        e_small > 1e-6,
        "ROW2b small-ξ deviation {e_small} unexpectedly at the noise floor"
    );
    let ratio = e_big / e_small;
    assert!(
        ratio > 3.0,
        "ROW2b approach rate not second-order: E(0.20)={e_big}, E(0.10)={e_small}, ratio {ratio} (need >3)"
    );
}

// ===========================================================================
// ROW 3 — PDE ≈ MC on a 2nd-generation window knock-out barrier.
//
// Two genuinely independent numerical methods: ADI backward induction with a
// calendar-toggled Dirichlet wall vs forward path simulation with a Brownian-
// bridge per-step survival weight. Agreement within the MC reported stderr (plus a
// documented discretisation slack) validates both engines on a path-dependent,
// second-generation payoff.
//
// Documented band: 3·stderr + 1.2e-2 absolute — the same slack the engine's own
// in-crate cross-engine test uses, justified there as MC noise + ADI/MC
// discretisation only (the unbiased fractional-survival estimator removes the
// median-rule bias). Reported, not asserted, MC stderr appears in the message.
// ===========================================================================

#[test]
fn window_barrier_pde_matches_mc() {
    let i = market();
    let carry = R_DOM - R_FOR;
    let iv = FlatIv {
        sigma: SIGMA,
        spot: SPOT,
        carry,
    };
    let var = VarianceParams::new(SIGMA * SIGMA, 2.0, SIGMA * SIGMA, 0.08, -0.2);
    let grid = spot_grid();
    let model = LsvModel::calibrate(
        (&i).into(),
        var,
        &iv,
        &grid,
        ParticleConfig {
            particles: 16_000,
            steps: 32,
            seed: 3,
            ..ParticleConfig::default()
        },
    );

    // Up-and-out call, barrier active only in the back half of the life.
    let spec = WindowBarrier {
        option: OptionType::Call,
        strike: SPOT,
        barrier: 1.50,
        up: true,
        start: 0.5,
        end: 1.0,
    };
    let agrid = AdiGrid {
        x_steps: 160,
        v_steps: 40,
        time_steps: 120,
        ..AdiGrid::default()
    };
    let pde = model.price_window_barrier_pde(spec, agrid);
    let mc = model.price_window_barrier_mc(
        spec,
        McConfig {
            pairs: 60_000,
            steps: 120,
            seed: 0xB17,
        },
    );
    let tol = 3.0 * mc.std_error + 1.2e-2;
    assert!(
        (pde - mc.price).abs() < tol,
        "ROW3 window-barrier PDE {pde} vs MC {} (se {}, tol {tol})",
        mc.price,
        mc.std_error
    );
}

// ===========================================================================
// ROW 4 — structural invariants (no separate oracle needed; these are model-
// internal monotonicities that any correct LSV must obey, and they catch a whole
// class of sign/indexing errors that an absolute oracle might tolerate).
//
//   (a) a knock-out barrier is ≤ the unbarriered vanilla (knocking out can only
//       remove payoff);
//   (b) a *window* (partially-active) up-and-out barrier is ≥ the corresponding
//       *full-life* (always-active) up-and-out barrier (less monitoring ⇒ fewer
//       knock-outs ⇒ more value);
//   (c) both barrier prices are non-negative;
//   (d) the MC price is bit-reproducible for a fixed seed.
// ===========================================================================

#[test]
fn structural_invariants() {
    let i = market();
    let carry = R_DOM - R_FOR;
    let iv = FlatIv {
        sigma: SIGMA,
        spot: SPOT,
        carry,
    };
    let var = VarianceParams::new(SIGMA * SIGMA, 2.0, SIGMA * SIGMA, 0.08, -0.2);
    let grid = spot_grid();
    let model = LsvModel::calibrate(
        (&i).into(),
        var,
        &iv,
        &grid,
        ParticleConfig {
            particles: 16_000,
            steps: 32,
            seed: 7,
            ..ParticleConfig::default()
        },
    );
    let agrid = AdiGrid {
        x_steps: 160,
        v_steps: 40,
        time_steps: 120,
        ..AdiGrid::default()
    };

    // Up-and-out call: full-life [0, T] and a back-window [0.5, T].
    let barrier = 1.50;
    let strike = SPOT;
    let full_life = model.price_barrier_pde(OptionType::Call, strike, barrier, true, agrid);
    let window = model.price_window_barrier_pde(
        WindowBarrier {
            option: OptionType::Call,
            strike,
            barrier,
            up: true,
            start: 0.5,
            end: 1.0,
        },
        agrid,
    );
    let vanilla = model.price_european_pde(OptionType::Call, strike, agrid);

    // (c) non-negativity (allow a tiny ADI under-shoot tolerance).
    assert!(
        full_life > -1e-9,
        "ROW4c full-life barrier negative: {full_life}"
    );
    assert!(window > -1e-9, "ROW4c window barrier negative: {window}");

    // (a) knock-out ≤ vanilla (with a small ADI slack on both barrier legs).
    let slack = 5e-3;
    assert!(
        full_life <= vanilla + slack,
        "ROW4a full-life KO {full_life} should be ≤ vanilla {vanilla}"
    );
    assert!(
        window <= vanilla + slack,
        "ROW4a window KO {window} should be ≤ vanilla {vanilla}"
    );

    // (b) window (less monitoring) ≥ full-life (more monitoring). Use a small slack
    // since both carry the same ADI discretisation.
    assert!(
        window + slack >= full_life,
        "ROW4b window KO {window} should be ≥ full-life KO {full_life}"
    );

    // (d) MC bit-reproducibility for a fixed seed.
    let cfg = McConfig {
        pairs: 8_000,
        steps: 40,
        seed: 0xABC,
    };
    let a = model.price_european_mc(OptionType::Call, strike, cfg);
    let b = model.price_european_mc(OptionType::Call, strike, cfg);
    assert_eq!(
        a.price.to_bits(),
        b.price.to_bits(),
        "ROW4d LSV MC must be bit-reproducible"
    );
}
