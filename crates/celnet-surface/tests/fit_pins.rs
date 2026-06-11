//! W6-ANALYTICS-RIGOR §3.4 — frozen reference-fit pins for the model
//! calibration fitters (`calibrate.rs`).
//!
//! The four fitters (stochastic-vol, parametric slice, parametric surface,
//! extended surface) are deterministic damped least-squares solves: fixed
//! iteration budget, fixed finite-difference step, no RNG — a calibration is
//! bit-reproducible (the crate's documented determinism guarantee, asserted by
//! `calibrate::tests::calibration_is_bit_reproducible`). That property is the
//! lever this file uses against the "converging-optimizer-internal" mutation
//! cluster: the **converged parameters and the achieved least-squares cost are
//! frozen to bits on two reference fixtures** (a benign smile and a stressed
//! five-point skew). Any mutant that perturbs the optimizer trajectory —
//! seeds, Jacobian finite differences, damping, step acceptance, the normal-
//! equation solve, projections — either lands on the exact same optimum bits
//! (then it is a *genuine* equivalent, excluded with that evidence in
//! `.config/mutants-surface.toml`) or it does not (killed here).
//!
//! Three layers per fitter × fixture:
//!  (a) `converged_cost_is_pinned`  — achieved cost ≤ frozen cost + 1e-12
//!      (catches any degradation of the optimum: damping, acceptance,
//!      Jacobian-sign errors — the largest sub-class);
//!  (b) `converged_params_are_pinned` — the calibrated parameters equal the
//!      frozen reference **to the bit** (catches different-basin convergence
//!      and any trajectory change that moves the optimum at all);
//!  (c) `fitted_smile_reprices_anchors` — the calibrated slice reproduces the
//!      input anchor vols (benign: to 1e-7, an exact-fit fixture; stressed: to
//!      the frozen least-squares residual + 1e-12) — the *economic* assertion.
//!
//! The anchors are re-derived in-test from the published recipe (Vanna-Volga
//! pillar calibration → `(k = ln(K/F), w = σ²t)` anchors), through the public
//! `calibrate_pillar` API — the same independent direction the fitters consume,
//! never read back from the fitted object under test.
//!
//! Frozen constants captured on the reference toolchain (1.96.0,
//! aarch64-apple-darwin; all arithmetic routes through `celnet_core::math`
//! libm, bit-identical across platforms).

use celnet_conventions::resolve;
use celnet_core::Smile;
use celnet_core::math::{exp, ln, sqrt};
use celnet_surface::{
    CalibratedSmile, MarketContext, MarketQuotes, ParametricSlice, SmileModel, build_model_smile,
    calibrate_pillar,
};
use celnet_types::{Carry, CcyPair, Tenor};

fn ctx(spot: f64, t: f64) -> MarketContext {
    let conv = resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
    let carry = Carry::FxRates {
        r_dom: 0.02,
        r_for: 0.01,
    };
    MarketContext::new(spot, carry, t, conv)
}

/// The benign reference fixture: EURUSD-like 1Y, mild put skew, 25Δ-only.
fn benign() -> (MarketContext, MarketQuotes) {
    (
        ctx(1.10, 1.0),
        MarketQuotes::three_point(0.10, -0.005, 0.0025),
    )
}

/// The stressed reference fixture: 6M, steep five-point put skew (10Δ wings).
fn stressed() -> (MarketContext, MarketQuotes) {
    (
        ctx(1.30, 0.5),
        MarketQuotes::five_point(0.12, -0.025, 0.0045, -0.045, 0.014),
    )
}

/// Re-derive the `(k, w)` fit anchors exactly as the calibrator's published
/// recipe defines them: Vanna-Volga-calibrate the 25Δ (and, when quoted, 10Δ)
/// pillars, then map each calibrated wing to log-moneyness / total variance.
fn anchors(c: &MarketContext, q: &MarketQuotes) -> Vec<(f64, f64)> {
    let f = c.forward();
    let t = c.t;
    let inner = calibrate_pillar(c, q.atm_vol, q.inner).unwrap();
    let mut out = vec![
        (ln(inner.put_strike / f), inner.put_vol * inner.put_vol * t),
        (0.0, q.atm_vol * q.atm_vol * t),
        (
            ln(inner.call_strike / f),
            inner.call_vol * inner.call_vol * t,
        ),
    ];
    if let Some(outer_q) = q.outer {
        let outer = calibrate_pillar(c, q.atm_vol, outer_q).unwrap();
        out.push((ln(outer.put_strike / f), outer.put_vol * outer.put_vol * t));
        out.push((
            ln(outer.call_strike / f),
            outer.call_vol * outer.call_vol * t,
        ));
    }
    out
}

/// The fitter's own cost functional, re-derived in-test: vol-space squared
/// residuals at the anchor strikes for the stochastic-vol fit, total-variance
/// squared residuals at the anchor log-moneynesses for the parametric fits.
fn cost_of(s: &CalibratedSmile, pts: &[(f64, f64)], f: f64, t: f64) -> f64 {
    match s {
        CalibratedSmile::StochasticVol(sv) => pts
            .iter()
            .map(|&(k, w)| {
                let strike = f * exp(k);
                let vol = sqrt(w / t);
                let r = sv.params().black_vol(strike) - vol;
                r * r
            })
            .sum(),
        CalibratedSmile::Parametric(sl)
        | CalibratedSmile::ParametricSurface(sl)
        | CalibratedSmile::ExtendedSurface(sl) => pts
            .iter()
            .map(|&(k, w)| {
                let r = sl.total_variance(k) - w;
                r * r
            })
            .sum(),
        CalibratedSmile::MarketHedge(_) => unreachable!("not a fitted model"),
    }
}

/// One frozen parametric-slice reference: `(a, b, ρ, m, σ)` as exact bits.
struct FrozenSlice {
    a: u64,
    b: u64,
    rho: u64,
    m: u64,
    sigma: u64,
}

impl FrozenSlice {
    fn assert_matches(&self, sl: &ParametricSlice, label: &str) {
        for (name, got, want) in [
            ("a", sl.a, self.a),
            ("b", sl.b, self.b),
            ("rho", sl.rho, self.rho),
            ("m", sl.m, self.m),
            ("sigma", sl.sigma, self.sigma),
        ] {
            assert_eq!(
                got.to_bits(),
                want,
                "{label}: converged {name}={got:?} (0x{:016x}) drifted off the frozen \
                 reference 0x{want:016x} ({:?})",
                got.to_bits(),
                f64::from_bits(want)
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Frozen references (captured on the reference toolchain — see module docs).
// ---------------------------------------------------------------------------

/// Benign stochastic-vol fit: (α, ρ, ν) bits + converged vol-space cost bits.
const BENIGN_SV: (u64, u64, u64, u64) = (
    0x3fb8f5f9fa78d946, // alpha = 0.09750330319443376
    0xbfc015832bf556a7, // rho   = -0.1256565060272596
    0x3fe29090760fe19b, // nu    = 0.5801470094584603
    0x38f0000000000000, // cost  = 1.925929944387236e-34
);
const STRESSED_SV: (u64, u64, u64, u64) = (
    0x3fbe9795797fd613, // alpha = 0.11950048652928329
    0xbfddc50bae6fe472, // rho   = -0.46515171084281015
    0x3feb88a782b3719f, // nu    = 0.860431437754915
    0x3ed2dd05b5fb4e76, // cost  = 4.497377488827197e-6
);

const BENIGN_SVI: FrozenSlice = FrozenSlice {
    a: 0x3f7cdf7831542726,     // 0.007049054625221795
    b: 0x3f9c26482a2e6a35,     // 0.027489783834974985
    rho: 0xbfaad85234d9449a,   // -0.05243164916158456
    m: 0x3f9af2ed4e21a753,     // 0.026317317861333688
    sigma: 0x3fba4714cd1f52f1, // 0.10264711387510396
};
const BENIGN_SVI_COST: u64 = 0x0000000000000000; // exact fit: cost = 0.0

const STRESSED_SVI: FrozenSlice = FrozenSlice {
    a: 0x3f790ad1fe57418e,     // 0.0061138346549828426
    b: 0x3f935362b3db34ec,     // 0.018872778155857015
    rho: 0xbfeff7ced916872b,   // -0.999 (projection boundary active)
    m: 0x3f95ec0722683ea4,     // 0.021408187365578826
    sigma: 0x3f9dd8fd8c20341f, // 0.029148065259551822
};
const STRESSED_SVI_COST: u64 = 0x3e56bb6f15e404b7; // 2.1170977298517854e-8

const BENIGN_SSVI: FrozenSlice = FrozenSlice {
    a: 0x3f743c7c79333e12,
    b: 0x3fa1c5d0e3dcc1fb,
    rho: 0xbfbbed5594fb4eb7,
    m: 0x3f90173053d49c35,
    sigma: 0x3fc253c413c985a7,
};
const BENIGN_SSVI_COST: u64 = 0x3890000000000000; // 3.009265538105056e-36

const STRESSED_SSVI: FrozenSlice = FrozenSlice {
    a: 0x3f69260dacdc7494,
    b: 0x3fa120323e2b49de,
    rho: 0xbfd88f0a9b6d8679,
    m: 0x3fa525421ef08172,
    sigma: 0x3fb9717c4c66a8df,
};
const STRESSED_SSVI_COST: u64 = 0x3e796d4d40b71622; // 9.472280753434902e-8

const BENIGN_ESSVI: FrozenSlice = FrozenSlice {
    a: 0x3f743c7c79333e12,
    b: 0x3fa1c5d0e3dcc1fb,
    rho: 0xbfbbed5594fb4eb3,
    m: 0x3f90173053d49c33,
    sigma: 0x3fc253c413c985a7,
};
const BENIGN_ESSVI_COST: u64 = 0x3890000000000000; // 3.009265538105056e-36

const STRESSED_ESSVI: FrozenSlice = FrozenSlice {
    a: 0x3f69260db340e0ae,
    b: 0x3fa120324a1e9353,
    rho: 0xbfd88f0a895a3cb7,
    m: 0x3fa52542009ef5ea,
    sigma: 0x3fb9717c3de15ae9,
};
const STRESSED_ESSVI_COST: u64 = 0x3e796d4d40b71f29; // 9.472280753437961e-8

/// Frozen max |fitted vol − anchor vol| for the stressed five-point fixture
/// (a genuine least-squares fit: 2–4 shape parameters over five anchors).
const STRESSED_MAX_ANCHOR_ERR: [(SmileModel, f64); 4] = [
    (SmileModel::StochasticVol, 1.5287853966807485e-3),
    (SmileModel::Parametric, 7.887281675096197e-4),
    (SmileModel::ParametricSurface, 1.804674255801192e-3),
    (SmileModel::ExtendedSurface, 1.8046739823601166e-3),
];

// ---------------------------------------------------------------------------
// (b) Converged parameters are pinned to bits.
// ---------------------------------------------------------------------------

#[test]
fn converged_params_are_pinned_benign() {
    let (c, q) = benign();
    for model in [
        SmileModel::StochasticVol,
        SmileModel::Parametric,
        SmileModel::ParametricSurface,
        SmileModel::ExtendedSurface,
    ] {
        let s = build_model_smile(model, &c, &q).unwrap();
        match &s {
            CalibratedSmile::StochasticVol(sv) => {
                let p = sv.params();
                assert_eq!(p.alpha.to_bits(), BENIGN_SV.0, "benign alpha drifted");
                assert_eq!(p.rho.to_bits(), BENIGN_SV.1, "benign rho drifted");
                assert_eq!(p.nu.to_bits(), BENIGN_SV.2, "benign nu drifted");
                assert_eq!(
                    p.forward.to_bits(),
                    c.forward().to_bits(),
                    "fitted slice must be anchored at the context forward"
                );
                assert_eq!(p.t.to_bits(), c.t.to_bits());
            }
            CalibratedSmile::Parametric(sl) => BENIGN_SVI.assert_matches(sl, "benign/Parametric"),
            CalibratedSmile::ParametricSurface(sl) => {
                BENIGN_SSVI.assert_matches(sl, "benign/ParametricSurface");
            }
            CalibratedSmile::ExtendedSurface(sl) => {
                BENIGN_ESSVI.assert_matches(sl, "benign/ExtendedSurface");
            }
            CalibratedSmile::MarketHedge(_) => unreachable!(),
        }
        // The materialised parametric slices are anchored at the context state.
        if let CalibratedSmile::Parametric(sl)
        | CalibratedSmile::ParametricSurface(sl)
        | CalibratedSmile::ExtendedSurface(sl) = &s
        {
            assert_eq!(sl.forward.to_bits(), c.forward().to_bits());
            assert_eq!(sl.t.to_bits(), c.t.to_bits());
        }
    }
}

#[test]
fn converged_params_are_pinned_stressed() {
    let (c, q) = stressed();
    for model in [
        SmileModel::StochasticVol,
        SmileModel::Parametric,
        SmileModel::ParametricSurface,
        SmileModel::ExtendedSurface,
    ] {
        let s = build_model_smile(model, &c, &q).unwrap();
        match &s {
            CalibratedSmile::StochasticVol(sv) => {
                let p = sv.params();
                assert_eq!(p.alpha.to_bits(), STRESSED_SV.0, "stressed alpha drifted");
                assert_eq!(p.rho.to_bits(), STRESSED_SV.1, "stressed rho drifted");
                assert_eq!(p.nu.to_bits(), STRESSED_SV.2, "stressed nu drifted");
            }
            CalibratedSmile::Parametric(sl) => {
                STRESSED_SVI.assert_matches(sl, "stressed/Parametric");
            }
            CalibratedSmile::ParametricSurface(sl) => {
                STRESSED_SSVI.assert_matches(sl, "stressed/ParametricSurface");
            }
            CalibratedSmile::ExtendedSurface(sl) => {
                STRESSED_ESSVI.assert_matches(sl, "stressed/ExtendedSurface");
            }
            CalibratedSmile::MarketHedge(_) => unreachable!(),
        }
    }
}

// ---------------------------------------------------------------------------
// (a) The achieved least-squares cost is pinned (no degradation of the optimum).
// ---------------------------------------------------------------------------

#[test]
fn converged_cost_is_pinned() {
    for ((c, q), refs) in [
        (
            benign(),
            [
                (SmileModel::StochasticVol, BENIGN_SV.3),
                (SmileModel::Parametric, BENIGN_SVI_COST),
                (SmileModel::ParametricSurface, BENIGN_SSVI_COST),
                (SmileModel::ExtendedSurface, BENIGN_ESSVI_COST),
            ],
        ),
        (
            stressed(),
            [
                (SmileModel::StochasticVol, STRESSED_SV.3),
                (SmileModel::Parametric, STRESSED_SVI_COST),
                (SmileModel::ParametricSurface, STRESSED_SSVI_COST),
                (SmileModel::ExtendedSurface, STRESSED_ESSVI_COST),
            ],
        ),
    ] {
        let pts = anchors(&c, &q);
        let f = c.forward();
        for (model, frozen_bits) in refs {
            let s = build_model_smile(model, &c, &q).unwrap();
            let cost = cost_of(&s, &pts, f, c.t);
            let frozen = f64::from_bits(frozen_bits);
            assert!(
                cost <= frozen + 1e-12,
                "{model:?}: achieved cost {cost:e} degraded past the frozen optimum {frozen:e}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// (c) The economic assertion: the fit reprices its anchors.
// ---------------------------------------------------------------------------

#[test]
fn fitted_smile_reprices_anchors() {
    // Benign three-point fixture: every model has at least as many shape
    // parameters as off-ATM anchors, so the fit is exact — 1e-7 in vol.
    let (c, q) = benign();
    let pts = anchors(&c, &q);
    let f = c.forward();
    for model in [
        SmileModel::StochasticVol,
        SmileModel::Parametric,
        SmileModel::ParametricSurface,
        SmileModel::ExtendedSurface,
    ] {
        let s = build_model_smile(model, &c, &q).unwrap();
        for &(k, w) in &pts {
            let strike = f * exp(k);
            let target = sqrt(w / c.t);
            let got = s.implied_vol(strike, f, c.t).0;
            assert!(
                (got - target).abs() <= 1e-7,
                "{model:?}: anchor at k={k} repriced to {got}, want {target} (±1e-7)"
            );
        }
    }

    // Stressed five-point fixture: a genuine least-squares fit; the worst
    // anchor error is pinned to the frozen reference residual.
    let (c, q) = stressed();
    let pts = anchors(&c, &q);
    let f = c.forward();
    for (model, frozen_err) in STRESSED_MAX_ANCHOR_ERR {
        let s = build_model_smile(model, &c, &q).unwrap();
        let max_err = pts
            .iter()
            .map(|&(k, w)| {
                let strike = f * exp(k);
                let target = sqrt(w / c.t);
                (s.implied_vol(strike, f, c.t).0 - target).abs()
            })
            .fold(0.0_f64, f64::max);
        assert!(
            max_err <= frozen_err + 1e-12,
            "{model:?}: stressed anchor reprice error {max_err:e} degraded past the \
             frozen residual {frozen_err:e}"
        );
    }
}

// ---------------------------------------------------------------------------
// Selection plumbing: the unified enum reports the right family and forward.
// ---------------------------------------------------------------------------

#[test]
fn calibrated_smile_reports_model_and_forward() {
    let (c, q) = benign();
    for model in [
        SmileModel::MarketHedge,
        SmileModel::StochasticVol,
        SmileModel::Parametric,
        SmileModel::ParametricSurface,
        SmileModel::ExtendedSurface,
    ] {
        let s = build_model_smile(model, &c, &q).unwrap();
        assert_eq!(s.model(), model, "model() must echo the selected family");
        assert_eq!(
            s.forward().to_bits(),
            c.forward().to_bits(),
            "{model:?}: forward() must report the calibration forward"
        );
    }
}
