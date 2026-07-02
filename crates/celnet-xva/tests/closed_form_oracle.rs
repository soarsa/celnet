//! Independent closed-form oracle tests for the XVA engine (W6 rigor wave).
//!
//! Every assertion here pins `celnet-xva` behaviour against a quantity derived
//! **outside** the implementation under test (`docs/plan/W6-ANALYTICS-RIGOR-PLAN.md`
//! §3.2):
//!
//! * the documented CVA/DVA/FVA quadrature re-derived in a plain in-test loop with
//!   raw `std` `exp` (code-disjoint from the crate's `libm`-routed survival /
//!   discount arithmetic), on a single-interval, a non-uniform and a 101-node grid;
//! * exact survival-curve identities — flat `Λ(t) = λ·t` to bits, an independent
//!   knot-overlap partial-sum recomputation of the piecewise hazard integral,
//!   `S(t) = exp(−Λ(t))` and `S(a) − S(b)` to bits, `S(0) = 1` exactly, and
//!   monotonicity on a deterministic pseudo-random grid;
//! * the total-adjustment decomposition `CVA − DVA + FVA` to bits plus its signs;
//! * a from-scratch domestic/foreign two-rate vanilla re-derivation (raw `Φ` via
//!   `erfc`, `std` logs/exps) for every netting-set mark, including the matured
//!   `τ ≤ 0 ⇒ 0` branch, signed notionals, and the plain-sum netting identity;
//! * bit-reproducibility of the low-discrepancy exposure simulation and frozen-bits
//!   rows (the house `fx_byte_identity` pattern) for two fixed netting sets — the
//!   integer Sobol/scramble core and the `libm`-routed transcendentals are
//!   bit-identical across platforms, so exact `u64` pins are stable. They are
//!   regression/mutation pins, not correctness claims — correctness is carried by
//!   the analytic oracles above;
//! * the full panic contract of every constructor/aggregator assert, each driven
//!   on both sides of its boundary.

use celnet_core::assert_close;
use celnet_types::OptionType;
use celnet_xva::{
    ExposureConfig, ExposureProfile, NettedTrade, NettingSet, SurvivalCurve, XvaInputs, compute_xva,
};

/// Independent reference `Φ(x) = ½·erfc(−x/√2)` — numerically stable in both
/// tails; the d1/d2/discount arithmetic around it uses raw `std` math, disjoint
/// from the `celnet_core::math` + `libm` path the production mark takes.
fn phi_ref(x: f64) -> f64 {
    0.5 * libm::erfc(-x * core::f64::consts::FRAC_1_SQRT_2)
}

/// From-scratch domestic/foreign two-rate vanilla present value (premium per 1
/// unit of base). Never calls `celnet_vanilla` — the whole formula is re-derived
/// here so any mutation of the netting-set mark plumbing is observable.
fn vanilla_ref(
    opt: OptionType,
    spot: f64,
    strike: f64,
    vol: f64,
    tau: f64,
    r_dom: f64,
    r_for: f64,
) -> f64 {
    let vsqt = vol * tau.sqrt();
    let d1 = ((spot / strike).ln() + (r_dom - r_for + 0.5 * vol * vol) * tau) / vsqt;
    let d2 = d1 - vsqt;
    let s_disc = spot * (-r_for * tau).exp();
    let k_disc = strike * (-r_dom * tau).exp();
    match opt {
        OptionType::Call => s_disc * phi_ref(d1) - k_disc * phi_ref(d2),
        OptionType::Put => k_disc * phi_ref(-d2) - s_disc * phi_ref(-d1),
    }
}

// ---------------------------------------------------------------------------
// 1. CVA / DVA / FVA vs an independent quadrature (plain loop, raw std exp).
// ---------------------------------------------------------------------------

/// Flat-hazard market for the quadrature cross-check.
struct QuadMarket {
    r_dom: f64,
    lam_cpty: f64,
    lam_own: f64,
    lgd_cpty: f64,
    lgd_own: f64,
    spread: f64,
}

/// The documented aggregation (`cva.rs` doc comment), re-derived factor by
/// factor with raw `std` `exp` — no shared code with the implementation:
///
/// ```text
/// CVA = LGD_c · Σ_k D(t_k)·EPE(t_k)·[S_c(t_{k−1}) − S_c(t_k)]
/// DVA = LGD_o · Σ_k D(t_k)·ENE(t_k)·[S_o(t_{k−1}) − S_o(t_k)]
/// FVA = s_f   · Σ_k D(t_k)·(EPE−ENE)(t_k)·Δt_k·S_c(t_k)·S_o(t_k)
/// ```
fn xva_longhand(grid: &[f64], epe: &[f64], ene: &[f64], m: &QuadMarket) -> (f64, f64, f64) {
    let df = |t: f64| (-m.r_dom * t).exp();
    let s_c = |t: f64| (-m.lam_cpty * t).exp();
    let s_o = |t: f64| (-m.lam_own * t).exp();
    let (mut cva, mut dva, mut fva) = (0.0, 0.0, 0.0);
    for k in 1..grid.len() {
        let (a, b) = (grid[k - 1], grid[k]);
        cva += m.lgd_cpty * df(b) * epe[k] * (s_c(a) - s_c(b));
        dva += m.lgd_own * df(b) * ene[k] * (s_o(a) - s_o(b));
        fva += m.spread * df(b) * (epe[k] - ene[k]) * (b - a) * s_c(b) * s_o(b);
    }
    (cva, dva, fva)
}

/// Run `compute_xva` on a deterministic profile and assert all three components
/// against the longhand quadrature to ≤ 1e-12 rel.
fn check_quadrature(
    grid: Vec<f64>,
    epe: Vec<f64>,
    ene: Vec<f64>,
    m: &QuadMarket,
) -> celnet_xva::XvaResult {
    let (cva_ref, dva_ref, fva_ref) = xva_longhand(&grid, &epe, &ene, m);
    let profile = ExposureProfile::deterministic(grid, epe, ene, m.r_dom);
    let r = compute_xva(&XvaInputs {
        profile: &profile,
        counterparty: &SurvivalCurve::flat(m.lam_cpty),
        own: &SurvivalCurve::flat(m.lam_own),
        lgd_counterparty: m.lgd_cpty,
        lgd_own: m.lgd_own,
        funding_spread: m.spread,
    });
    assert_close!(r.cva, cva_ref, 1e-12, 1e-15);
    assert_close!(r.dva, dva_ref, 1e-12, 1e-15);
    assert_close!(r.fva, fva_ref, 1e-12, 1e-15);
    r
}

#[test]
fn cva_dva_fva_match_independent_quadrature() {
    let m = QuadMarket {
        r_dom: 0.027,
        lam_cpty: 0.023,
        lam_own: 0.011,
        lgd_cpty: 0.6,
        lgd_own: 0.4,
        spread: 0.0075,
    };

    // Single interval, EPE-dominant ⇒ funding COST (FVA > 0).
    let r1 = check_quadrature(vec![0.0, 1.0], vec![0.0, 7.5], vec![0.0, 2.25], &m);
    assert!(r1.cva > 0.0 && r1.dva > 0.0 && r1.fva > 0.0);

    // Non-uniform grid, ENE-dominant ⇒ funding BENEFIT (FVA < 0). The uneven
    // Δt_k makes the FVA time-weighting (`grid[k] − grid[k−1]`) load-bearing.
    let r2 = check_quadrature(
        vec![0.0, 0.25, 1.0, 1.75, 4.0],
        vec![0.0, 1.5, 2.0, 1.25, 0.5],
        vec![0.0, 4.0, 6.5, 5.0, 2.5],
        &m,
    );
    assert!(r2.fva < 0.0);

    // 101-node dense grid: smooth profiles, accumulation across 100 intervals.
    let grid: Vec<f64> = (0..=100u32).map(|k| f64::from(k) * 0.05).collect();
    let epe: Vec<f64> = grid.iter().map(|&t| 12.0 * t * (-0.6 * t).exp()).collect();
    let ene: Vec<f64> = grid.iter().map(|&t| 4.0 * (1.0 - (-t).exp())).collect();
    check_quadrature(grid, epe, ene, &m);
}

// ---------------------------------------------------------------------------
// 2. Survival-curve identities (bits where exact, independent partial sums).
// ---------------------------------------------------------------------------

/// Independent piecewise hazard integral: per-segment overlap of `[0, t]` with
/// the knot interval `[knot_{i−1}, knot_i]`, plus flat extrapolation — no early
/// exit, structurally different from the implementation's scan.
fn cum_hazard_ref(pillars: &[f64], hazards: &[f64], t: f64) -> f64 {
    let mut acc = 0.0;
    let mut knot = 0.0;
    for (&p, &h) in pillars.iter().zip(hazards) {
        acc += h * (t.min(p) - knot).max(0.0);
        knot = p;
    }
    let last = *pillars.last().expect("non-empty");
    if t > last {
        acc += *hazards.last().expect("non-empty") * (t - last);
    }
    acc
}

#[test]
fn survival_curve_identities() {
    // Flat curve: Λ(t) = λ·t is a single multiply on both sides ⇒ bits-exact.
    let lam = 0.0375;
    let flat = SurvivalCurve::flat(lam);
    for &t in &[0.0, 0.1, 0.5, 1.0, 2.5, 7.0, 30.0] {
        assert_eq!(
            flat.cumulative_hazard(t).to_bits(),
            (lam * t).to_bits(),
            "flat Λ(t) ≠ λ·t at t={t}"
        );
        assert_eq!(
            flat.survival(t).to_bits(),
            libm::exp(-(lam * t)).to_bits(),
            "flat S(t) ≠ exp(−λt) at t={t}"
        );
    }
    assert_eq!(flat.survival(0.0).to_bits(), 1.0_f64.to_bits());

    // Piecewise curve (with a zero-hazard segment — a valid boundary input).
    let pillars = [0.5, 1.0, 2.0, 5.0];
    let hazards = [0.01, 0.0, 0.035, 0.05];
    let pw = SurvivalCurve::piecewise(pillars.to_vec(), hazards.to_vec());
    // At, between, and beyond every pillar boundary.
    for &t in &[0.0, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.7, 5.0, 6.0, 12.0] {
        assert_close!(
            pw.cumulative_hazard(t),
            cum_hazard_ref(&pillars, &hazards, t),
            1e-14,
            1e-18
        );
        assert_eq!(
            pw.survival(t).to_bits(),
            libm::exp(-pw.cumulative_hazard(t)).to_bits(),
            "S(t) ≠ exp(−Λ(t)) at t={t}"
        );
    }
    assert_eq!(pw.survival(0.0).to_bits(), 1.0_f64.to_bits());

    // Marginal default probability is exactly the survival increment.
    for &(a, b) in &[(0.0, 0.5), (0.25, 0.75), (1.0, 1.0), (1.5, 6.0)] {
        assert_eq!(
            pw.marginal_default(a, b).to_bits(),
            (pw.survival(a) - pw.survival(b)).to_bits(),
            "marginal_default({a},{b}) ≠ S(a) − S(b)"
        );
    }

    // Monotone non-increasing on a deterministic pseudo-random grid (LCG).
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut ts: Vec<f64> = (0..64)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 11) as f64 / (1_u64 << 53) as f64 * 40.0
        })
        .collect();
    ts.sort_by(f64::total_cmp);
    let mut prev = 1.0;
    for &t in &ts {
        let s = pw.survival(t);
        assert!(s <= prev, "survival must be non-increasing (t={t})");
        prev = s;
    }
}

// ---------------------------------------------------------------------------
// 3. Total-adjustment decomposition and signs.
// ---------------------------------------------------------------------------

#[test]
fn total_adjustment_sign_decomposition() {
    let profile = ExposureProfile::deterministic(
        vec![0.0, 0.5, 1.25, 3.0],
        vec![0.0, 3.0, 4.5, 1.0],
        vec![0.0, 1.0, 2.5, 6.0],
        0.03,
    );
    let cpty = SurvivalCurve::flat(0.04);
    let own = SurvivalCurve::flat(0.02);
    let r = compute_xva(&XvaInputs {
        profile: &profile,
        counterparty: &cpty,
        own: &own,
        lgd_counterparty: 0.55,
        lgd_own: 0.45,
        funding_spread: 0.008,
    });
    // The decomposition is the literal definition ⇒ bits-exact.
    assert_eq!(
        r.total_adjustment().to_bits(),
        (r.cva - r.dva + r.fva).to_bits()
    );
    assert!(r.cva > 0.0, "CVA must be ≥ 0 on a positive-EPE profile");
    assert!(r.dva > 0.0, "DVA must be ≥ 0 on a positive-ENE profile");

    // Boundary LGDs are valid by contract; CVA is linear in LGD and DVA
    // vanishes at LGD_own = 0.
    let rb = compute_xva(&XvaInputs {
        profile: &profile,
        counterparty: &cpty,
        own: &own,
        lgd_counterparty: 1.0,
        lgd_own: 0.0,
        funding_spread: 0.008,
    });
    assert_eq!(rb.dva.to_bits(), 0.0_f64.to_bits());
    assert_close!(rb.cva, r.cva / 0.55, 1e-12, 1e-15);
    assert_eq!(rb.fva.to_bits(), r.fva.to_bits(), "FVA is LGD-independent");
}

// ---------------------------------------------------------------------------
// 4. Netting-set marks vs the from-scratch vanilla re-derivation.
// ---------------------------------------------------------------------------

#[test]
fn netting_set_marks_match_vanilla_closed_form() {
    let (r_dom, r_for) = (0.025, 0.012);
    let call = NettedTrade::new(OptionType::Call, 1.10, 1.0, 0.12, 25.0);
    let put = NettedTrade::new(OptionType::Put, 1.05, 2.0, 0.115, -40.0);

    // Signed marks at several observation times and spots, τ = expiry − t_obs.
    for &(t_obs, spot) in &[(0.0, 1.0820), (0.3, 1.0500), (0.85, 1.1800)] {
        let want_call = 25.0
            * vanilla_ref(
                OptionType::Call,
                spot,
                1.10,
                0.12,
                1.0 - t_obs,
                r_dom,
                r_for,
            );
        assert_close!(
            call.mark(t_obs, spot, r_dom, r_for),
            want_call,
            1e-12,
            1e-15
        );
        let want_put = -40.0
            * vanilla_ref(
                OptionType::Put,
                spot,
                1.05,
                0.115,
                2.0 - t_obs,
                r_dom,
                r_for,
            );
        assert_close!(put.mark(t_obs, spot, r_dom, r_for), want_put, 1e-12, 1e-15);
    }

    // Matured: exactly at expiry (τ = 0, the boundary) and beyond, the trade has
    // settled and leaves the live MtM ⇒ exact 0.0 (spot ≠ strike so a mutant
    // that prices τ = 0 yields the non-zero intrinsic value instead).
    assert_eq!(
        call.mark(1.0, 1.18, r_dom, r_for).to_bits(),
        0.0_f64.to_bits()
    );
    assert_eq!(
        call.mark(1.7, 1.18, r_dom, r_for).to_bits(),
        0.0_f64.to_bits()
    );

    // The set: accessors and the plain-sum netting identity.
    let set = NettingSet::new(vec![call, put], r_dom, r_for);
    assert_eq!(set.trades(), &[call, put][..]);
    assert_eq!(set.r_dom().to_bits(), r_dom.to_bits());
    assert_eq!(set.r_for().to_bits(), r_for.to_bits());
    assert_eq!(set.horizon().to_bits(), 2.0_f64.to_bits());
    for &(t, s) in &[(0.0, 1.0820), (0.6, 1.1300), (1.4, 0.9900)] {
        let call_ref = if t < 1.0 {
            25.0 * vanilla_ref(OptionType::Call, s, 1.10, 0.12, 1.0 - t, r_dom, r_for)
        } else {
            0.0 // matured inside the netting horizon
        };
        let put_ref = -40.0 * vanilla_ref(OptionType::Put, s, 1.05, 0.115, 2.0 - t, r_dom, r_for);
        assert_close!(set.net_value(t, s), call_ref + put_ref, 1e-12, 1e-15);
    }
}

// ---------------------------------------------------------------------------
// 5. Exposure simulation: bit-reproducibility + frozen-bits regression rows.
// ---------------------------------------------------------------------------

/// Mixed-sign netting set: long calls vs short puts, both alive at the pinned
/// median node, so the net mark changes sign across paths and BOTH `max(·, 0)`
/// floors bite there (and `epe[0] > 0` pins the positive t = 0 branch).
fn mixed_set() -> (NettingSet, ExposureConfig, usize) {
    let set = NettingSet::new(
        vec![
            NettedTrade::new(OptionType::Call, 1.1000, 2.0, 0.12, 25.0),
            NettedTrade::new(OptionType::Put, 1.0500, 1.6, 0.115, -40.0),
        ],
        0.025,
        0.012,
    );
    let cfg = ExposureConfig {
        spot0: 1.0820,
        sigma: 0.14,
        paths: 2048,
        seed: 0x00C0_FFEE_1234_5678,
    };
    (set, cfg, 8)
}

/// Net-short set: the t = 0 mark is strictly negative, so `ene[0] > 0` and
/// `epe[0] = 0` — the t = 0 branch of both floors is pinned from the other side.
fn net_short_set() -> (NettingSet, ExposureConfig, usize) {
    let set = NettingSet::new(
        vec![
            NettedTrade::new(OptionType::Call, 1.2500, 1.5, 0.10, -60.0),
            NettedTrade::new(OptionType::Put, 1.2000, 0.75, 0.11, 15.0),
        ],
        0.015,
        0.030,
    );
    let cfg = ExposureConfig {
        spot0: 1.2350,
        sigma: 0.105,
        paths: 1024,
        seed: 0xDEAD_BEEF,
    };
    (set, cfg, 6)
}

#[test]
fn exposure_simulation_is_bit_reproducible() {
    for (set, cfg, steps) in [mixed_set(), net_short_set()] {
        let a = ExposureProfile::simulate(&set, &cfg, steps);
        let b = ExposureProfile::simulate(&set, &cfg, steps);
        for k in 0..=steps {
            assert_eq!(a.grid()[k].to_bits(), b.grid()[k].to_bits(), "grid k={k}");
            assert_eq!(a.epe()[k].to_bits(), b.epe()[k].to_bits(), "epe k={k}");
            assert_eq!(a.ene()[k].to_bits(), b.ene()[k].to_bits(), "ene k={k}");
            assert_eq!(
                a.discount()[k].to_bits(),
                b.discount()[k].to_bits(),
                "discount k={k}"
            );
        }
    }
}

#[test]
fn simulate_accepts_minimal_config_boundary() {
    // steps = 1, paths = 1 are the documented minimum — must run, not panic.
    let set = NettingSet::new(
        vec![NettedTrade::new(OptionType::Call, 1.10, 1.0, 0.12, 1.0)],
        0.02,
        0.01,
    );
    let cfg = ExposureConfig {
        spot0: 1.10,
        sigma: 0.12,
        paths: 1,
        seed: 1,
    };
    let p = ExposureProfile::simulate(&set, &cfg, 1);
    assert_eq!(p.grid().len(), 2);
    // Node 0 is the deterministic mark of the set (single long call ⇒ ENE ≡ 0).
    let v0 = vanilla_ref(OptionType::Call, 1.10, 1.10, 0.12, 1.0, 0.02, 0.01);
    assert_close!(p.epe()[0], v0.max(0.0), 1e-12, 1e-15);
    assert_eq!(p.ene()[0].to_bits(), 0.0_f64.to_bits());
    assert_eq!(p.ene()[1].to_bits(), 0.0_f64.to_bits());
    assert!(p.epe()[1] >= 0.0);
}

/// Frozen-bits regression rows (originally captured 2026-06-10 on
/// aarch64-apple-darwin in a quiet window, toolchain 1.96.0 — the
/// fx_byte_identity house pattern; the integer Sobol/scramble core and the
/// `libm`-routed transcendentals are bit-identical across platforms).
/// Regenerated 2026-06-30 for the gBSM forward-space kernel unification
/// (ADR-0012): `NettedTrade::mark` calls `celnet_vanilla::price`, which was
/// re-associated spot-space → forward-space (`F = S·e^{bt}`), so the marks —
/// and every exposure quantity derived from them — round differently at ~1e-15.
/// The regenerated marks were re-validated ≤1e-12 vs the independent two-rate
/// closed form (`vanilla_ref` above); this is sub-1e-12 reformulation drift
/// (deterministic node 0 rel ≤ 3e-15, MC nodes ≤ 1e-15), NOT a correctness
/// change. Any mutation of the seed/scramble plumbing, the GBM drift/diffusion
/// arithmetic, the exposure floors, or the path averaging still shifts these bits.
///
/// Nodes pinned per set: first (t = 0, the deterministic mark), median, last
/// (= the longest expiry, where every trade has matured ⇒ exactly 0.0 — this
/// pins the `τ ≤ 0 ⇒ 0` boundary *through the simulation path*: a mutant that
/// prices τ = 0 picks up the in-the-money paths' intrinsic value instead).
///
/// MIXED   node 0: epe = 0.41142764321375447  ene = 0
/// MIXED   node 4: epe = 1.7888267787714307   ene = 1.5366117309930545
/// MIXED   node 8: epe = 0                     ene = 0
/// NET_SHORT node 0: ene = 1.8765675546592466 (epe = 0: strictly net-short)
/// NET_SHORT node 3: ene = 2.5328546593151922
/// NET_SHORT node 6: epe = ene = 0
const MIXED_EPE_BITS: [u64; 3] = [
    0x3FDA_54D4_9C11_7FE8,
    0x3FFC_9F08_D410_8464,
    0x0000_0000_0000_0000,
];
const MIXED_ENE_BITS: [u64; 3] = [
    0x0000_0000_0000_0000,
    0x3FF8_95F6_2EB4_3DED,
    0x0000_0000_0000_0000,
];
const NET_SHORT_EPE_BITS: [u64; 3] = [
    0x0000_0000_0000_0000,
    0x0000_0000_0000_0000,
    0x0000_0000_0000_0000,
];
const NET_SHORT_ENE_BITS: [u64; 3] = [
    0x3FFE_066B_B33F_F042,
    0x4004_4349_4DBA_3D62,
    0x0000_0000_0000_0000,
];

#[test]
fn exposure_simulation_matches_frozen_bits() {
    for (name, (set, cfg, steps), epe_bits, ene_bits) in [
        ("MIXED", mixed_set(), MIXED_EPE_BITS, MIXED_ENE_BITS),
        (
            "NET_SHORT",
            net_short_set(),
            NET_SHORT_EPE_BITS,
            NET_SHORT_ENE_BITS,
        ),
    ] {
        let p = ExposureProfile::simulate(&set, &cfg, steps);
        let r_dom = set.r_dom();
        let dt = set.horizon() / steps as f64;
        for (i, &k) in [0, steps / 2, steps].iter().enumerate() {
            assert_eq!(p.epe()[k].to_bits(), epe_bits[i], "{name} epe node {k}");
            assert_eq!(p.ene()[k].to_bits(), ene_bits[i], "{name} ene node {k}");
        }
        // Grid and discount re-derived independently: t_k = k·Δt exactly (same
        // float op order), discount = e^{−r_dom·t_k} via raw std exp.
        for (k, &t) in p.grid().iter().enumerate() {
            assert_eq!(t.to_bits(), (k as f64 * dt).to_bits(), "{name} grid {k}");
            assert_close!(p.discount()[k], (-r_dom * t).exp(), 1e-14, 0.0);
        }
    }
}

// ---------------------------------------------------------------------------
// 6. Panic contracts — every constructor/aggregator assert, both boundary sides.
// ---------------------------------------------------------------------------

fn valid_profile() -> ExposureProfile {
    ExposureProfile::deterministic(vec![0.0, 1.0], vec![0.0, 5.0], vec![0.0, 2.0], 0.02)
}

fn xva_with_lgds(lgd_cpty: f64, lgd_own: f64) -> celnet_xva::XvaResult {
    let profile = valid_profile();
    let alive = SurvivalCurve::flat(0.01);
    compute_xva(&XvaInputs {
        profile: &profile,
        counterparty: &alive,
        own: &alive,
        lgd_counterparty: lgd_cpty,
        lgd_own,
        funding_spread: 0.005,
    })
}

#[test]
#[should_panic(expected = "LGD must be in [0, 1]")]
fn xva_panics_on_lgd_counterparty_above_one() {
    let _ = xva_with_lgds(1.0 + 1e-9, 0.5);
}

#[test]
#[should_panic(expected = "LGD must be in [0, 1]")]
fn xva_panics_on_lgd_counterparty_negative() {
    let _ = xva_with_lgds(-1e-9, 0.5);
}

#[test]
#[should_panic(expected = "LGD must be in [0, 1]")]
fn xva_panics_on_lgd_own_above_one() {
    let _ = xva_with_lgds(0.5, 1.0 + 1e-9);
}

#[test]
#[should_panic(expected = "LGD must be in [0, 1]")]
fn xva_panics_on_lgd_own_negative() {
    let _ = xva_with_lgds(0.5, -1e-9);
}

#[test]
#[should_panic(expected = "hazard rate must be finite and non-negative")]
fn flat_panics_on_negative_hazard() {
    let _ = SurvivalCurve::flat(-0.01);
}

#[test]
#[should_panic(expected = "hazard rate must be finite and non-negative")]
fn flat_panics_on_non_finite_hazard() {
    let _ = SurvivalCurve::flat(f64::NAN);
}

#[test]
#[should_panic(expected = "pillar times must be strictly increasing")]
fn piecewise_panics_on_equal_pillars() {
    let _ = SurvivalCurve::piecewise(vec![1.0, 1.0], vec![0.01, 0.02]);
}

#[test]
#[should_panic(expected = "pillar times must be strictly increasing")]
fn piecewise_panics_on_decreasing_pillars() {
    let _ = SurvivalCurve::piecewise(vec![1.0, 0.5], vec![0.01, 0.02]);
}

#[test]
#[should_panic(expected = "pillar times must be strictly increasing")]
fn piecewise_panics_on_zero_first_pillar() {
    // The first pillar must be strictly positive (prev starts at 0).
    let _ = SurvivalCurve::piecewise(vec![0.0, 1.0], vec![0.01, 0.02]);
}

#[test]
#[should_panic(expected = "hazard rate must be finite and non-negative")]
fn piecewise_panics_on_negative_hazard() {
    let _ = SurvivalCurve::piecewise(vec![1.0, 2.0], vec![0.01, -0.02]);
}

#[test]
#[should_panic(expected = "pillars and hazards must be non-empty and equal length")]
fn piecewise_panics_on_length_mismatch() {
    let _ = SurvivalCurve::piecewise(vec![1.0, 2.0], vec![0.01]);
}

#[test]
#[should_panic(expected = "pillars and hazards must be non-empty and equal length")]
fn piecewise_panics_on_empty() {
    let _ = SurvivalCurve::piecewise(vec![], vec![]);
}

#[test]
#[should_panic(expected = "time must be finite")]
fn cumulative_hazard_panics_on_negative_time() {
    let _ = SurvivalCurve::flat(0.02).cumulative_hazard(-1e-9);
}

#[test]
#[should_panic(expected = "time must be finite")]
fn cumulative_hazard_panics_on_non_finite_time() {
    let _ = SurvivalCurve::flat(0.02).cumulative_hazard(f64::INFINITY);
}

#[test]
#[should_panic(expected = "interval must satisfy a ≤ b")]
fn marginal_default_panics_on_reversed_interval() {
    let _ = SurvivalCurve::flat(0.02).marginal_default(2.0, 1.0);
}

#[test]
#[should_panic(expected = "grid must start at t = 0")]
fn deterministic_panics_on_nonzero_grid_start() {
    let _ = ExposureProfile::deterministic(vec![0.5, 1.0], vec![1.0, 1.0], vec![0.0, 0.0], 0.02);
}

#[test]
#[should_panic(expected = "grid must be strictly increasing")]
fn deterministic_panics_on_non_increasing_grid() {
    let _ = ExposureProfile::deterministic(
        vec![0.0, 1.0, 1.0],
        vec![0.0, 1.0, 1.0],
        vec![0.0, 0.0, 0.0],
        0.02,
    );
}

#[test]
#[should_panic(expected = "EPE/ENE must be non-negative")]
fn deterministic_panics_on_negative_epe() {
    let _ = ExposureProfile::deterministic(vec![0.0, 1.0], vec![0.0, -1.0], vec![0.0, 1.0], 0.02);
}

#[test]
#[should_panic(expected = "EPE/ENE must be non-negative")]
fn deterministic_panics_on_negative_ene() {
    let _ = ExposureProfile::deterministic(vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, -1.0], 0.02);
}

#[test]
#[should_panic(expected = "grid/epe/ene must be non-empty and equal length")]
fn deterministic_panics_on_epe_length_mismatch() {
    let _ = ExposureProfile::deterministic(
        vec![0.0, 1.0, 2.0],
        vec![0.0, 1.0],
        vec![0.0, 1.0, 1.0],
        0.02,
    );
}

#[test]
#[should_panic(expected = "grid/epe/ene must be non-empty and equal length")]
fn deterministic_panics_on_ene_length_mismatch() {
    let _ = ExposureProfile::deterministic(
        vec![0.0, 1.0, 2.0],
        vec![0.0, 1.0, 1.0],
        vec![0.0, 1.0],
        0.02,
    );
}

#[test]
#[should_panic]
fn deterministic_panics_on_empty() {
    let _ = ExposureProfile::deterministic(vec![], vec![], vec![], 0.02);
}

#[test]
#[should_panic(expected = "need at least one exposure step")]
fn simulate_panics_on_zero_steps() {
    let (set, cfg, _) = mixed_set();
    let _ = ExposureProfile::simulate(&set, &cfg, 0);
}

#[test]
#[should_panic(expected = "need at least one path")]
fn simulate_panics_on_zero_paths() {
    let (set, mut cfg, steps) = mixed_set();
    cfg.paths = 0;
    let _ = ExposureProfile::simulate(&set, &cfg, steps);
}

#[test]
#[should_panic(expected = "netting set has zero horizon")]
fn simulate_panics_on_zero_horizon() {
    let set = NettingSet::new(vec![], 0.02, 0.01);
    let cfg = ExposureConfig {
        spot0: 1.10,
        sigma: 0.12,
        paths: 16,
        seed: 1,
    };
    let _ = ExposureProfile::simulate(&set, &cfg, 4);
}
