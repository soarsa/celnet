//! Parity row — **smile-overlay magnitude** (RC blocker R6
//! `vanna-volga-overlay-magnitude-unvalidated`).
//!
//! The production smile overlay (`celnet_exotics::market_hedge_overlay`,
//! landed `4dd500d`) had only flat-smile / sign / scaling tests — no
//! *quantitative magnitude* oracle. This row closes that gap by validating the
//! engine's collapsed projection route (market price of one unit of
//! vanna / volga off linearized risk-reversal / butterfly surpluses) against a
//! **first-principles replicating-portfolio oracle** re-derived in
//! `celnet_golden::oracle`: the full 3×3 system equating the hedge portfolio's
//! **vega, vanna and volga** to the exotic's at the three smile pillars,
//! solved by explicit Cramer determinants, with the cost as the **exact**
//! (not linearized) market-over-flat reprice of the weighted portfolio.
//! Provenance (doc-only): Castagna & Mercurio (2007); Bossens, Rayée,
//! Skantzos & Deelstra (2010) IJTAF 13(8).
//!
//! # Why the oracle can genuinely disagree (anti-circularity)
//!
//! The two routes share no construction: projection-of-two-ratios vs explicit
//! 3×3 Cramer solve; first-order `vega·Δσ` surpluses vs exact closed-form wing
//! reprices; `erfc`-route CDF (`celnet_core::math`) vs `erf`-route (`libm`);
//! the engine mixes forward-measure pillar vanna with the exotic's
//! spot-measure vanna while the oracle is consistently spot-measure. Each
//! difference is a real, quantified contribution to the documented band.
//!
//! # The documented magnitude band (how 20 % was derived)
//!
//! The structural gap between the routes was quantified by an independent
//! Python float64 study (both routes re-implemented outside Rust) over
//! RR ∈ [−0.025, +0.015], BF ∈ [0.002, 0.008] on the four-product set below:
//! observed relative gaps 3.8 % – 12.6 % on every materially-sized cost,
//! decomposing into (a) the engine's linearized wing surpluses
//! (`O(Δσ·d₁d₂/σ)`, a few %), (b) the dropped vega row / ATM-pillar
//! cross-Greeks, and (c) the vanna measure mixing (`e^{b·t} − 1 ≈ 2 %`).
//! Gate: **≤ 20 % relative** (≈ 1.6× the worst observed gap) **plus sign
//! agreement plus a 2e-3 materiality floor** so the row can never pass
//! vacuously on a near-zero cost. This is an *approximation-consistency*
//! band between two stated constructions — explicitly not a closed-form
//! precision claim.
//!
//! # Rows
//!
//! 1. **Magnitude parity** on a barrier/touch set under a real EURUSD-style
//!    smile built from frozen broker RR/BF quotes (the same fixture family the
//!    broker-smile parity rows freeze): engine overlay cost ≈ oracle
//!    replicating-portfolio cost within the derived band, same sign, material.
//! 2. **Flat-smile ⇒ exactly zero adjustment** in both routes (the
//!    zero-adjustment law, asserted with zero tolerance).
//! 3. **Monotonicity in the risk-reversal sign** on the vanna-dominated
//!    no-touch, and in the butterfly on the volga-dominated double-no-touch —
//!    both routes. (Volga-dominated products need *not* be RR-monotone — the
//!    pillars themselves move with the quotes — so the law is pinned on the
//!    dominated products where the direction is structural.)
//! 4. **Survival damping** scales both routes identically (engine exact by
//!    construction; oracle compared in-band).
//! 5. **Externally pinned literals**: the oracle's weights / cost / hedge
//!    determinant on an explicit pillar fixture, recomputed OUTSIDE the Rust
//!    codebase (route stated at the constants).
//! 6. **Cross-model validation** (the spec's row): the VV-adjusted window
//!    barrier vs the LSV-priced window barrier on the same smile. These are
//!    DIFFERENT models — the row pins that the two smile adjustments **agree
//!    in sign and order of magnitude** (ratio within [0.1, 10]), never
//!    bit-equality; stated honestly as a qualitative band.

use celnet_conventions::resolve;
use celnet_core::is_close;
use celnet_exotics::{
    AdiGrid, BarrierKind, BarrierStyle, DoubleNoTouch, ExoticInputs, ExoticSensitivities, LsvModel,
    ParticleConfig, RebateTiming, SingleBarrier, SurvivalWeight, VarianceParams, WindowBarrier,
    double_no_touch_price, exotic_sensitivities_fd, hedge_smile_overlay,
    leverage::ImpliedVolSurface, market_price_of_hedge_smile, no_touch_price, one_touch_price,
    single_barrier_price,
};
use celnet_golden::oracle::{
    Cp, HedgeOverlayBreakdown, SmileExposures, hedge_smile_overlay_cost, smile_exposures_fd,
    window_barrier_smooth_mc,
};
use celnet_surface::{MarketContext, MarketHedgeSmile, MarketQuotes, build_smile};
use celnet_types::{Carry, CcyPair, OptionType, Tenor, VanillaInputs};

// ---------------------------------------------------------------------------
// Frozen market fixture: EURUSD-like 1Y — the same (spot, rates, ATM) slice
// the LSV parity row freezes, so row 6 prices both models on one market.
// ---------------------------------------------------------------------------

const SPOT: f64 = 1.30;
const SIGMA: f64 = 0.10;
const R_DOM: f64 = 0.03;
const R_FOR: f64 = 0.01;
const HORIZON: f64 = 1.0;

/// Per-slice market context (resolved EURUSD 1Y conventions).
fn ctx() -> MarketContext {
    let conv = resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
    MarketContext::new(
        SPOT,
        Carry::FxRates {
            r_dom: R_DOM,
            r_for: R_FOR,
        },
        HORIZON,
        conv,
    )
}

/// Calibrated three-pillar smile from frozen broker RR/BF quotes.
fn smile_from(rr_25: f64, bf_25: f64) -> MarketHedgeSmile {
    build_smile(&ctx(), &MarketQuotes::three_point(SIGMA, rr_25, bf_25))
        .expect("smile builds from broker quotes")
}

/// Agnostic exotic inputs at a bumped `(spot, vol)` on the frozen market.
fn exotic_inputs(spot: f64, vol: f64) -> ExoticInputs {
    (&VanillaInputs::new(spot, SPOT, vol, HORIZON, R_DOM, R_FOR)).into()
}

/// A named flat-vol `(spot, vol)` pricer for one product of the set.
type FlatPricer = Box<dyn Fn(f64, f64) -> f64>;

/// The barrier/touch product set: flat-vol closed forms as `(spot, vol)`
/// pricers. The flat closed forms themselves are independently gated by the
/// golden corpus (touch/barrier vector tables); this row's subject is the
/// OVERLAY priced on top of them.
fn product_set() -> Vec<(&'static str, FlatPricer)> {
    vec![
        (
            "one-touch up H=1.45 (at expiry)",
            Box::new(|s, v| {
                one_touch_price(&exotic_inputs(s, v), 1.45, 1.0, RebateTiming::AtExpiry)
            }) as FlatPricer,
        ),
        (
            "no-touch down H=1.15",
            Box::new(|s, v| no_touch_price(&exotic_inputs(s, v), 1.15, 1.0)),
        ),
        (
            "double-no-touch [1.18, 1.43]",
            Box::new(|s, v| {
                double_no_touch_price(&exotic_inputs(s, v), DoubleNoTouch::new(1.18, 1.43, 1.0))
            }),
        ),
        (
            "up-and-out call K=1.30 H=1.45",
            Box::new(|s, v| {
                single_barrier_price(
                    &exotic_inputs(s, v),
                    SingleBarrier {
                        kind: BarrierKind {
                            up: true,
                            style: BarrierStyle::KnockOut,
                            option: OptionType::Call,
                        },
                        strike: 1.30,
                        barrier: 1.45,
                        rebate: 0.0,
                    },
                )
            }),
        ),
    ]
}

/// ENGINE route: unweighted (European-survival) overlay cost via the
/// production projection, exactly as the server pricer assembles it.
fn engine_cost(
    smile: &MarketHedgeSmile,
    price: &dyn Fn(f64, f64) -> f64,
) -> (f64, ExoticSensitivities) {
    let i = exotic_inputs(SPOT, SIGMA);
    let strikes = smile.benchmark_strikes();
    let market = market_price_of_hedge_smile(smile, &i, strikes[0], strikes[2]);
    let x = exotic_sensitivities_fd(price, SPOT, SIGMA);
    let flat = price(SPOT, SIGMA);
    let overlay = hedge_smile_overlay(flat, x, market, SurvivalWeight::EUROPEAN);
    (overlay.hedge_smile_cost, x)
}

/// ORACLE route: the replicating-portfolio cost on the SAME pillars, target
/// exposures by the oracle's own (different-stencil) finite differences.
fn oracle_cost(
    smile: &MarketHedgeSmile,
    price: &dyn Fn(f64, f64) -> f64,
) -> (HedgeOverlayBreakdown, SmileExposures) {
    let target = smile_exposures_fd(price, SPOT, SIGMA);
    let breakdown = hedge_smile_overlay_cost(
        SPOT,
        HORIZON,
        R_DOM,
        R_FOR,
        smile.benchmark_strikes(),
        smile.benchmark_vols(),
        target,
    )
    .expect("calibrated pillars are well-separated, never singular");
    (breakdown, target)
}

// ===========================================================================
// ROW 1 — magnitude parity: engine projection ≈ replicating-portfolio oracle
// on the real EURUSD-style smile, per product: same sign, ≤ 20 % relative
// (band derivation in the module docs), and material (≥ 2e-3) so the row
// cannot pass on a vanishing cost.
// ===========================================================================

#[test]
fn engine_overlay_matches_replicating_portfolio_oracle_in_magnitude() {
    let smile = smile_from(-0.015, 0.0040);
    for (name, price) in product_set() {
        let (cost_engine, x) = engine_cost(&smile, price.as_ref());
        let (oracle, target) = oracle_cost(&smile, price.as_ref());
        let cost_oracle = oracle.cost;

        // The two routes' cross-Greek targets must describe the same exotic
        // (different stencils, same smooth closed form): ≤ 1 % relative.
        assert!(
            (x.vanna - target.vanna).abs() <= 1e-2 * target.vanna.abs().max(1.0),
            "[{name}] engine vanna {} vs oracle vanna {}",
            x.vanna,
            target.vanna
        );
        assert!(
            (x.volga - target.volga).abs() <= 1e-2 * target.volga.abs().max(1.0),
            "[{name}] engine volga {} vs oracle volga {}",
            x.volga,
            target.volga
        );

        // Materiality: the fixture must produce a real smile cost.
        assert!(
            cost_oracle.abs() > 2.0e-3,
            "[{name}] oracle cost {cost_oracle} too small to validate a magnitude"
        );
        // Sign agreement.
        assert!(
            cost_engine.signum() == cost_oracle.signum(),
            "[{name}] sign disagreement: engine {cost_engine} vs oracle {cost_oracle}"
        );
        // The derived 20 % magnitude band.
        let gap = (cost_engine - cost_oracle).abs();
        let band = 0.20 * cost_engine.abs().max(cost_oracle.abs());
        assert!(
            gap <= band,
            "[{name}] overlay magnitude off: engine {cost_engine} vs oracle {cost_oracle} \
             (gap {gap}, band {band})"
        );
    }
}

// ===========================================================================
// ROW 2 — the flat-smile ⇒ zero-adjustment law, both routes, zero tolerance.
// Engine: zero wing surpluses make both unit prices exactly 0; oracle: every
// market-over-flat reprice is exactly 0. `is_close(_, 0.0, 0.0, 0.0)` is the
// platform's exact-equality idiom (±0.0-safe).
// ===========================================================================

#[test]
fn flat_smile_produces_exactly_zero_adjustment_in_both_routes() {
    let f = ctx().forward();
    let smile = MarketHedgeSmile::try_new([1.24, f, 1.42], [SIGMA; 3], f, HORIZON)
        .expect("flat three-pillar smile builds");
    for (name, price) in product_set() {
        let (cost_engine, _) = engine_cost(&smile, price.as_ref());
        let (oracle, _) = oracle_cost(&smile, price.as_ref());
        assert!(
            is_close(cost_engine, 0.0, 0.0, 0.0),
            "[{name}] engine flat-smile cost must be exactly zero, got {cost_engine}"
        );
        assert!(
            is_close(oracle.cost, 0.0, 0.0, 0.0),
            "[{name}] oracle flat-smile cost must be exactly zero, got {}",
            oracle.cost
        );
    }
}

// ===========================================================================
// ROW 3 — monotonicity in the skew/convexity quotes, both routes.
//
// (a) RR sweep on the VANNA-dominated no-touch (its vanna ≈ +23.5 dwarfs the
//     volga leg): with the call-over-put RR convention the market price of
//     vanna rises with RR, so the overlay cost must be strictly increasing.
// (b) BF sweep on the VOLGA-dominated double-no-touch (volga ≈ +136): a fatter
//     butterfly raises the market price of volga, so the cost must rise.
//
// The independent Python study shows consecutive-step margins ≈ 2e-2 (a),
// ≈ 5e-2 (b) — far above any pillar-recalibration jitter, so strict `>` is a
// robust gate, not a knife-edge.
// ===========================================================================

#[test]
fn no_touch_overlay_is_monotone_in_risk_reversal_in_both_routes() {
    let price = |s: f64, v: f64| no_touch_price(&exotic_inputs(s, v), 1.15, 1.0);
    // Direction anchor: the product must be vanna-dominated and positive.
    let x = exotic_sensitivities_fd(price, SPOT, SIGMA);
    assert!(
        x.vanna > 0.0 && x.vanna.abs() > 5.0 * x.volga.abs(),
        "fixture must be vanna-dominated: vanna {} volga {}",
        x.vanna,
        x.volga
    );
    let mut prev: Option<(f64, f64)> = None;
    for rr in [-0.02, -0.01, 0.0, 0.01, 0.02] {
        let smile = smile_from(rr, 0.0040);
        let (cost_engine, _) = engine_cost(&smile, &price);
        let (oracle, _) = oracle_cost(&smile, &price);
        if let Some((prev_engine, prev_oracle)) = prev {
            assert!(
                cost_engine > prev_engine,
                "engine cost must rise with RR: {prev_engine} -> {cost_engine} at rr={rr}"
            );
            assert!(
                oracle.cost > prev_oracle,
                "oracle cost must rise with RR: {prev_oracle} -> {} at rr={rr}",
                oracle.cost
            );
        }
        prev = Some((cost_engine, oracle.cost));
    }
}

#[test]
fn double_no_touch_overlay_is_monotone_in_butterfly_in_both_routes() {
    let corridor = DoubleNoTouch::new(1.18, 1.43, 1.0);
    let price = |s: f64, v: f64| double_no_touch_price(&exotic_inputs(s, v), corridor);
    // Direction anchor: the corridor must be volga-dominated and positive.
    let x = exotic_sensitivities_fd(price, SPOT, SIGMA);
    assert!(
        x.volga > 0.0 && x.volga.abs() > 5.0 * x.vanna.abs(),
        "fixture must be volga-dominated: vanna {} volga {}",
        x.vanna,
        x.volga
    );
    let mut prev: Option<(f64, f64)> = None;
    for bf in [0.002, 0.004, 0.006, 0.008] {
        let smile = smile_from(-0.015, bf);
        let (cost_engine, _) = engine_cost(&smile, &price);
        let (oracle, _) = oracle_cost(&smile, &price);
        if let Some((prev_engine, prev_oracle)) = prev {
            assert!(
                cost_engine > prev_engine,
                "engine cost must rise with BF: {prev_engine} -> {cost_engine} at bf={bf}"
            );
            assert!(
                oracle.cost > prev_oracle,
                "oracle cost must rise with BF: {prev_oracle} -> {} at bf={bf}",
                oracle.cost
            );
        }
        prev = Some((cost_engine, oracle.cost));
    }
}

// ===========================================================================
// ROW 4 — survival (first-exit) damping: the engine scales its cost by one
// multiply (exact by construction, asserted to the bit), and the damped
// engine cost still matches the equally-damped oracle within the ROW-1 band.
// ===========================================================================

#[test]
fn survival_damping_scales_both_routes_identically() {
    let smile = smile_from(-0.015, 0.0040);
    let corridor = DoubleNoTouch::new(1.18, 1.43, 1.0);
    let price = |s: f64, v: f64| double_no_touch_price(&exotic_inputs(s, v), corridor);

    let i = exotic_inputs(SPOT, SIGMA);
    let flat = price(SPOT, SIGMA);
    // The DNT's own no-touch (survival) probability: flat = e^{−r_d T}·R·p.
    let p = flat / (i.discount_df() * corridor.rebate);
    assert!(p > 0.0 && p < 1.0, "survival probability sane: {p}");

    let strikes = smile.benchmark_strikes();
    let market = market_price_of_hedge_smile(&smile, &i, strikes[0], strikes[2]);
    let x = exotic_sensitivities_fd(price, SPOT, SIGMA);
    let full = hedge_smile_overlay(flat, x, market, SurvivalWeight::EUROPEAN);
    let damped = hedge_smile_overlay(flat, x, market, SurvivalWeight::new(p));

    // Engine law: damping is the single multiply `p · cost` — exact.
    assert_eq!(
        damped.hedge_smile_cost.to_bits(),
        (p * full.hedge_smile_cost).to_bits(),
        "engine survival damping must be the exact one-multiply scaling"
    );

    // The damped engine cost matches the equally-damped oracle in the band.
    let (oracle, _) = oracle_cost(&smile, &price);
    let oracle_damped = p * oracle.cost;
    let gap = (damped.hedge_smile_cost - oracle_damped).abs();
    let band = 0.20 * damped.hedge_smile_cost.abs().max(oracle_damped.abs());
    assert!(
        gap <= band,
        "damped overlay off: engine {} vs oracle {oracle_damped} (gap {gap}, band {band})",
        damped.hedge_smile_cost
    );
}

// ===========================================================================
// ROW 5 — externally pinned literals (Lesson c: constants the engine and the
// oracle cannot BOTH silently mis-state).
// ===========================================================================

/// Recomputed OUTSIDE the Rust codebase: an independent Python 3 IEEE-754
/// float64 evaluation (normal CDF via `math.erf`, Gaussian density via the
/// independently recomputed `1/√(2π)`, explicit first-row-cofactor Cramer
/// determinants, closed-form Garman-Kohlhagen wing reprices) of the
/// replicating-portfolio construction on the explicit fixture below —
/// 2026-06-11. Agreement is gated at 1e-10 relative, absorbing only
/// `libm`-vs-CPython ulp differences in the shared transcendental calls.
mod pinned {
    /// Hedge weights `(w₁, w₂, w₃)` for the fixture in
    /// [`super::oracle_reproduces_externally_pinned_weights_and_cost`].
    pub(super) const WEIGHTS: [f64; 3] = [
        -10.990_433_234_387_27,
        26.406_719_969_663_133,
        -12.474_146_379_049_955,
    ];
    /// Unweighted overlay cost `Σ wᵢ·[V(Kᵢ,σᵢ) − V(Kᵢ,σ₀)]`.
    pub(super) const COST: f64 = -0.033_138_141_706_169_05;
    /// Determinant of the benchmark-exposure matrix.
    pub(super) const DET: f64 = 4.171_691_382_975_692;
}

#[test]
fn oracle_reproduces_externally_pinned_weights_and_cost() {
    // Explicit pillar fixture (literals, NOT build_smile output, so the pin is
    // stable): EURUSD-like spot 1.30, 1Y, r_d 3 %, r_f 1 %, skewed pillars.
    let target = SmileExposures {
        vega: 4.0,
        vanna: -3.8,
        volga: -45.0,
    };
    let b = hedge_smile_overlay_cost(
        1.30,
        1.0,
        0.03,
        0.01,
        [1.2400, 1.3263, 1.4220],
        [0.1115, 0.1000, 0.0965],
        target,
    )
    .expect("explicit pillars are non-singular");

    for (w, pin) in b.weights.iter().zip(pinned::WEIGHTS) {
        assert!(
            (w - pin).abs() <= 1e-10 * pin.abs(),
            "pinned weight: got {w}, external {pin}"
        );
    }
    assert!(
        (b.cost - pinned::COST).abs() <= 1e-10 * pinned::COST.abs(),
        "pinned cost: got {}, external {}",
        b.cost,
        pinned::COST
    );
    assert!(
        (b.hedge_matrix_det - pinned::DET).abs() <= 1e-10 * pinned::DET.abs(),
        "pinned determinant: got {}, external {}",
        b.hedge_matrix_det,
        pinned::DET
    );
}

// ===========================================================================
// ROW 6 — cross-model validation: VV-adjusted vs LSV-priced window barrier on
// the SAME smile.
//
// HONESTY STATEMENT. Vanna-volga is a static-hedge correction; LSV is a full
// dynamics. They do NOT agree to a tolerance — the literature comparison
// (Bossens-Rayée-Skantzos-Deelstra 2010, doc-only) shows the VV overlay
// tracks smile-model exotic adjustments in sign and size but not precisely.
// This row therefore pins exactly what is defensible: the two models' SMILE
// ADJUSTMENTS for the same window barrier (model-on-smile minus
// model-on-flat) agree **in sign and order of magnitude** (ratio ∈ [0.1, 10]),
// with materiality floors stated against the numerical noise of each leg.
// Never bit-equality, never a tight band — any tighter claim would be
// fabricated.
//
// VV leg: flat window-barrier price and cross-Greeks from the golden
// bridge-survival MC under common random numbers, overlay by BOTH the engine
// projection and the oracle solve. The exposure vector is measured ONCE (the
// oracle's larger-stencil FD at 60k antithetic pairs) and fed to BOTH
// constructions: the bridge-survival estimator is continuous but not C¹ at
// endpoint-touches (the per-step survival factor has a slope break where an
// endpoint meets the barrier), so the engine's 1e-4 closed-form-tuned stencil
// is kink-noise dominated on an MC estimator (quantified in the Python study:
// tiny-step vanna even flips sign). Sharing one stated exposure measurement
// isolates exactly what this row tests — the two OVERLAY constructions —
// observed disagreement 6–7 %, gated at 20 % + 2e-4. The residual
// vanna-stencil uncertainty (±0.15 absolute) moves the cost by ≲ 12 % —
// inside the same band and irrelevant at the order-of-magnitude LSV
// comparison. The estimator itself is validated in-row against the engine's
// continuous-barrier closed form on the full-life window.
//
// LSV leg: two calibrations of the SAME variance backbone / particle seed /
// ADI grid — one to the pillar smile, one to the flat ATM surface — priced on
// the identical window-barrier PDE; the difference cancels the shared
// discretisation bias to first order.
// ===========================================================================

/// Sticky-strike, calendar-flat adapter exposing the calibrated pillar smile
/// to the LSV leverage calibration (the Dupire slice sees the same smile at
/// every maturity; total variance still grows in `t`, so no calendar issue).
struct PillarSmileIv<'a> {
    smile: &'a MarketHedgeSmile,
    spot: f64,
    carry: f64,
}

impl ImpliedVolSurface for PillarSmileIv<'_> {
    fn implied_vol(&self, k: f64, _t: f64) -> f64 {
        self.smile.vol_at(k)
    }
    fn forward(&self, t: f64) -> f64 {
        self.spot * libm::exp(self.carry * t)
    }
}

/// Flat (constant-σ) surface adapter for the flat LSV leg.
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

#[test]
fn window_barrier_smile_adjustment_agrees_with_lsv_in_sign_and_order() {
    // A fat (EM-stress) convex smile so the adjustment is decisively material:
    // RR −2.5 vol, BF 80 bp on ATM 10 vol.
    let smile = smile_from(-0.025, 0.0080);

    // ---- VV leg: golden bridge-survival MC flat pricer (CRN by fixed seed).
    // 60 steps align the window edges (0.5, 1.0) to step boundaries; 60k
    // antithetic pairs hold the larger-stencil FD exposures stable (the
    // bridge estimator is unbiased at any step count).
    let (steps, prs, seed) = (60, 60_000, 0x00C0_FFEE);
    let price = |s: f64, v: f64| {
        window_barrier_smooth_mc(
            Cp::Call,
            true,
            s,
            1.30,
            1.40,
            0.5,
            1.0,
            v,
            HORIZON,
            R_DOM,
            R_FOR,
            steps,
            prs,
            seed,
        )
        .price
    };

    // Estimator validation: on the FULL-life window the bridge MC must
    // reprice the engine's continuous-barrier closed form within MC noise.
    let full_life = window_barrier_smooth_mc(
        Cp::Call,
        true,
        SPOT,
        1.30,
        1.40,
        0.0,
        1.0,
        SIGMA,
        HORIZON,
        R_DOM,
        R_FOR,
        steps,
        prs,
        seed,
    );
    let closed = single_barrier_price(
        &exotic_inputs(SPOT, SIGMA),
        SingleBarrier {
            kind: BarrierKind {
                up: true,
                style: BarrierStyle::KnockOut,
                option: OptionType::Call,
            },
            strike: 1.30,
            barrier: 1.40,
            rebate: 0.0,
        },
    );
    assert!(
        (full_life.price - closed).abs() <= 4.0 * full_life.std_error + 1e-4,
        "bridge MC must reprice the continuous closed form: MC {} (se {}) vs closed {closed}",
        full_life.price,
        full_life.std_error
    );

    // ONE exposure measurement (larger-stencil oracle FD on the smooth MC,
    // CRN), fed to BOTH overlay constructions — see the row header for why
    // the engine's tiny closed-form stencil cannot be used on an MC pricer.
    // Take the flat reference price first (the last use of the `price` closure),
    // then hand the closure by value to the FD exposure stencil.
    let flat = price(SPOT, SIGMA);
    let exposures = smile_exposures_fd(price, SPOT, SIGMA);
    let x = ExoticSensitivities {
        vanna: exposures.vanna,
        volga: exposures.volga,
    };
    // The window KO call is long volga at this corridor — the documented
    // driver of a positive adjustment under a convex (positive-BF) smile.
    assert!(
        exposures.volga > 0.0,
        "window barrier must be long volga: {}",
        exposures.volga
    );

    let i = exotic_inputs(SPOT, SIGMA);
    let strikes = smile.benchmark_strikes();
    let market = market_price_of_hedge_smile(&smile, &i, strikes[0], strikes[2]);
    let cost_engine =
        hedge_smile_overlay(flat, x, market, SurvivalWeight::EUROPEAN).hedge_smile_cost;
    let cost_oracle = hedge_smile_overlay_cost(
        SPOT,
        HORIZON,
        R_DOM,
        R_FOR,
        strikes,
        smile.benchmark_vols(),
        exposures,
    )
    .expect("calibrated pillars are well-separated, never singular")
    .cost;

    // With the exposures shared, the residual gap is purely the two overlay
    // constructions (projection vs 3×3 solve): observed 6–7 %, gated 20 %.
    assert!(
        cost_engine.signum() == cost_oracle.signum(),
        "VV routes disagree in sign: engine {cost_engine} vs oracle {cost_oracle}"
    );
    assert!(
        (cost_engine - cost_oracle).abs()
            <= 0.20 * cost_engine.abs().max(cost_oracle.abs()) + 2.0e-4,
        "VV routes diverge: engine {cost_engine} vs oracle {cost_oracle}"
    );
    assert!(
        cost_oracle.abs() > 1.0e-3,
        "fixture must produce a material VV adjustment, got {cost_oracle}"
    );

    // ---- LSV leg: same backbone/seed/grid, smile vs flat calibration.
    let i = VanillaInputs::new(SPOT, SPOT, SIGMA, HORIZON, R_DOM, R_FOR);
    let carry = R_DOM - R_FOR;
    let grid: Vec<f64> = (0..41)
        .map(|k| SPOT * libm::exp(-0.6 + 0.03 * k as f64))
        .collect();
    let spec = WindowBarrier {
        option: OptionType::Call,
        strike: 1.30,
        barrier: 1.40,
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
    let smile_leg = LsvModel::calibrate(
        (&i).into(),
        VarianceParams::new(SIGMA * SIGMA, 2.0, SIGMA * SIGMA, 0.08, -0.2),
        &PillarSmileIv {
            smile: &smile,
            spot: SPOT,
            carry,
        },
        &grid,
        ParticleConfig {
            particles: 16_000,
            steps: 32,
            seed: 11,
            ..ParticleConfig::default()
        },
    );
    let flat_leg = LsvModel::calibrate(
        (&i).into(),
        VarianceParams::new(SIGMA * SIGMA, 2.0, SIGMA * SIGMA, 0.08, -0.2),
        &FlatIv {
            sigma: SIGMA,
            spot: SPOT,
            carry,
        },
        &grid,
        ParticleConfig {
            particles: 16_000,
            steps: 32,
            seed: 11,
            ..ParticleConfig::default()
        },
    );
    let lsv_adjustment = smile_leg.price_window_barrier_pde(spec, agrid)
        - flat_leg.price_window_barrier_pde(spec, agrid);

    // The qualitative cross-model band (see the row header for why no tighter
    // claim is defensible): sign agreement + order of magnitude + a noise
    // floor (the PDE-difference residual after shared-bias cancellation is
    // far below 2e-4; an adjustment under the floor would mean the fixture
    // cannot distinguish the models and the row must fail, not pass).
    assert!(
        lsv_adjustment.abs() > 2.0e-4,
        "LSV smile adjustment {lsv_adjustment} below the materiality floor"
    );
    assert!(
        cost_oracle.signum() == lsv_adjustment.signum(),
        "models disagree in sign: VV {cost_oracle} vs LSV {lsv_adjustment}"
    );
    let ratio = lsv_adjustment / cost_oracle;
    assert!(
        (0.1..=10.0).contains(&ratio),
        "models disagree in order of magnitude: VV {cost_oracle} vs LSV {lsv_adjustment} \
         (ratio {ratio})"
    );
}
