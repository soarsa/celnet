//! Parity row (W11-B): **robust eSSVI calibration stays no-arbitrage under
//! stress** — wide skews and sparse quote sets.
//!
//! `celnet-surface`'s [`ExtendedSurface::calibrate`] fits a multi-maturity eSSVI
//! surface and *projects* it into the static no-arbitrage domain (butterfly per
//! slice + calendar between slices) rather than trusting the raw least-squares
//! fit. This row proves the guarantee holds across a **stress grid** of wide /
//! sparse synthetic quote sets, and it does so with an **independent numerical
//! oracle**, never the closed-form predicate the calibrator itself used:
//!
//!  (i)   **positive risk-neutral density everywhere** — the Breeden-Litzenberger
//!        density `g(K) = ∂²C/∂K²`, computed by a central second difference of the
//!        *re-priced* undiscounted forward call ([`celnet_surface::implied_density`]
//!        over the materialised raw slice), is `≥ 0` at every strike on a dense
//!        grid, for **every** calibrated slice. Pointwise option re-pricing is a
//!        genuinely independent method from the `(θ,ρ,ψ)` butterfly inequality;
//!  (ii)  **calendar-monotone total variance** — `w(k,θ₂) ≥ w(k,θ₁)` pointwise for
//!        every consecutive pillar pair on a dense `k` grid (the *definitional*
//!        calendar-no-arbitrage truth, independent of the eSSVI pair inequality);
//!  (iii) **zero violations** — across the whole stress grid the running count of
//!        density and calendar violations is exactly `0`;
//!  (iv)  **hand-pinned published no-arb constants** — to defend against a
//!        mis-stated bound shared by code and oracle (Lesson c), the eSSVI
//!        butterfly `ψ`-cap at `ρ = 0` and the Hendriks-Martini (2019) calendar
//!        pair condition are pinned to values computed **by hand** from the
//!        published inequalities, and a slice exactly at the cap is butterfly-free
//!        while one a hair over is not.
//!
//! Provenance (doc-only): the eSSVI parameterisation and its closed-form static
//! no-arbitrage (butterfly + calendar) conditions are Hendriks & Martini (2019),
//! "The extended SSVI volatility surface"; the per-slice butterfly bound is the
//! SSVI sufficient condition of Gatheral & Jacquier (2014, Thm 4.2) re-expressed
//! in `(θ,ρ,ψ)`. Identifiers carry no person names (guardrail #8).

use celnet_surface::{ExtendedSlice, ExtendedSurface, implied_density};

/// A dense log-spaced strike grid straddling the forward for the static checks
/// (wide: ±~50% in strike, i.e. roughly k ∈ [−0.7, +0.5]).
fn strike_grid(forward: f64) -> Vec<f64> {
    let mut g = Vec::new();
    let mut k = 0.5 * forward;
    while k <= 1.6 * forward {
        g.push(k);
        k += 0.02 * forward;
    }
    g
}

/// One stress scenario: a list of (θ, quote-set) pillars in ascending θ, with a
/// human-readable label for failure messages.
struct Scenario {
    label: &'static str,
    pillars: Vec<(f64, Vec<(f64, f64)>)>,
}

/// Build the stress grid of wide/sparse synthetic eSSVI quote sets. These are
/// deliberately adversarial for a naive least-squares fit: very steep skews,
/// fat convexity that would push past the butterfly bound, decreasing-looking
/// wings that would invite a calendar crossing, and pillars with as little as a
/// single off-ATM quote (or none).
fn stress_scenarios() -> Vec<Scenario> {
    vec![
        // 1) Very wide, steep negative skew (EM-like), three maturities.
        Scenario {
            label: "wide steep negative skew",
            pillars: vec![
                (
                    0.010,
                    vec![(-0.7, 0.030), (-0.2, 0.014), (0.0, 0.010), (0.3, 0.013)],
                ),
                (
                    0.030,
                    vec![(-0.6, 0.060), (-0.2, 0.038), (0.0, 0.030), (0.3, 0.036)],
                ),
                (0.080, vec![(-0.5, 0.120), (0.0, 0.080), (0.4, 0.095)]),
            ],
        },
        // 2) Fat butterfly that would exceed the curvature bound if unprojected.
        Scenario {
            label: "fat butterfly (curvature-bound stress)",
            pillars: vec![
                (
                    0.006,
                    vec![
                        (-0.4, 0.040),
                        (-0.1, 0.011),
                        (0.0, 0.006),
                        (0.1, 0.011),
                        (0.4, 0.040),
                    ],
                ),
                (0.020, vec![(-0.4, 0.080), (0.0, 0.020), (0.4, 0.080)]),
            ],
        },
        // 3) Sparse: a single wide off-ATM quote per pillar.
        Scenario {
            label: "sparse single wide wing",
            pillars: vec![
                (0.008, vec![(0.0, 0.008), (-0.6, 0.018)]),
                (0.025, vec![(0.0, 0.025), (-0.5, 0.045)]),
                (0.070, vec![(0.0, 0.070), (0.45, 0.085)]),
            ],
        },
        // 4) Calendar-crossing stress: a later pillar whose raw fit would have a
        //    smaller skew-scale ψ or an incompatible ATM skew vs the earlier one.
        Scenario {
            label: "calendar-crossing stress",
            pillars: vec![
                (0.015, vec![(-0.5, 0.045), (0.0, 0.015), (0.3, 0.024)]),
                (0.018, vec![(0.4, 0.030), (0.0, 0.018), (-0.2, 0.020)]), // skew flips sign
                (0.090, vec![(-0.5, 0.150), (0.0, 0.090), (0.5, 0.130)]),
            ],
        },
        // 5) Strong positive skew (commodity-like), wide.
        Scenario {
            label: "wide positive skew",
            pillars: vec![
                (0.012, vec![(-0.3, 0.014), (0.0, 0.012), (0.6, 0.034)]),
                (0.040, vec![(-0.3, 0.043), (0.0, 0.040), (0.6, 0.085)]),
            ],
        },
        // 6) Empty + minimal pillars (sparsest possible): no shape at all on the
        //    first, one point on the rest.
        Scenario {
            label: "empty + minimal pillars",
            pillars: vec![
                (0.005, vec![]),
                (0.020, vec![(0.0, 0.020), (-0.4, 0.030)]),
                (0.060, vec![(0.0, 0.060)]),
            ],
        },
        // 7) Many maturities, a long term structure with alternating skews.
        Scenario {
            label: "long term structure, alternating skews",
            pillars: vec![
                (0.004, vec![(-0.4, 0.0085), (0.0, 0.004), (0.4, 0.0080)]),
                (0.011, vec![(-0.4, 0.020), (0.0, 0.011), (0.4, 0.018)]),
                (0.025, vec![(0.3, 0.040), (0.0, 0.025), (-0.3, 0.038)]),
                (0.050, vec![(-0.4, 0.085), (0.0, 0.050), (0.4, 0.075)]),
                (0.090, vec![(-0.4, 0.150), (0.0, 0.090), (0.4, 0.130)]),
            ],
        },
    ]
}

/// Row (i)+(ii)+(iii) — across the whole stress grid, every calibrated surface has
/// positive BL density at every strike on every slice AND is calendar-monotone
/// pointwise, with a total violation count of exactly zero. The oracle re-prices
/// undiscounted forward calls and second-differences them (the BL density), which
/// is independent of the `(θ,ρ,ψ)` butterfly inequality the calibrator projected
/// against; the calendar check compares total variances pointwise (the definition).
#[test]
fn calibrated_surface_is_no_arb_across_stress_grid() {
    // A small negative tolerance absorbs only the second-difference round-off of
    // the density oracle (a finite-difference of repriced calls), NOT a genuine
    // arbitrage — far below any real density violation, which would be O(1) negative.
    const DENS_TOL: f64 = -1e-6;
    const CAL_TOL: f64 = -1e-9;

    let forward = 1.10;
    let t = 1.0;
    let grid = strike_grid(forward);
    let h = 0.004 * forward;

    let mut density_violations = 0usize;
    let mut calendar_violations = 0usize;
    let mut slices_checked = 0usize;
    let mut pairs_checked = 0usize;

    for sc in stress_scenarios() {
        let surf = ExtendedSurface::calibrate(&sc.pillars);

        // The calibrator's OWN closed-form predicates must already pass — but the
        // real proof is the independent numerics below.
        assert!(
            surf.is_butterfly_free(),
            "[{}] calibrated surface fails its own butterfly predicate",
            sc.label
        );
        assert!(
            surf.is_calendar_free(),
            "[{}] calibrated surface fails its own calendar predicate",
            sc.label
        );

        // (i) Independent BL density ≥ 0 on every calibrated slice.
        for pillar in surf.pillars() {
            let raw = pillar.to_slice(forward, t);
            for &k_strike in &grid {
                let dens = implied_density(&raw, k_strike, forward, t, h);
                if dens < DENS_TOL {
                    density_violations += 1;
                    eprintln!(
                        "[{}] NEGATIVE density {dens} at K={k_strike} (θ={}, ρ={}, ψ={})",
                        sc.label, pillar.theta, pillar.rho, pillar.psi
                    );
                }
            }
            slices_checked += 1;
        }

        // (ii) Independent calendar monotonicity: w(k,θ₂) ≥ w(k,θ₁) pointwise.
        let pillars = surf.pillars();
        for w in pillars.windows(2) {
            let (lo, hi) = (&w[0], &w[1]);
            for &strike in &grid {
                let k = (strike / forward).ln();
                let w1 = lo.total_variance(k);
                let w2 = hi.total_variance(k);
                if w2 - w1 < CAL_TOL {
                    calendar_violations += 1;
                    eprintln!(
                        "[{}] CALENDAR arb at k={k}: w(θ={})={w1} > w(θ={})={w2}",
                        sc.label, lo.theta, hi.theta
                    );
                }
            }
            pairs_checked += 1;
        }
    }

    println!(
        "[essvi-hardening] checked {slices_checked} slices, {pairs_checked} calendar pairs across the stress grid: \
         {density_violations} density violations, {calendar_violations} calendar violations"
    );
    // (iii) ZERO violations across the entire stress grid.
    assert_eq!(density_violations, 0, "density violations under stress");
    assert_eq!(calendar_violations, 0, "calendar violations under stress");
    // Non-vacuity: the grid actually exercised many slices/pairs.
    assert!(
        slices_checked >= 20,
        "stress grid under-gated: {slices_checked} slices"
    );
    assert!(
        pairs_checked >= 12,
        "stress grid under-gated: {pairs_checked} pairs"
    );
}

/// Row (iv) — **hand-pinned published no-arb constants** (Lesson c).
///
/// The eSSVI per-slice butterfly domain (Hendriks-Martini 2019 §2; Gatheral-
/// Jacquier 2014 Thm 4.2 in `(θ,ρ,ψ)`) is `ψ(1+|ρ|) < 4` AND `(ψ²/θ)(1+|ρ|) ≤ 4`.
/// At `ρ = 0` the cap is `ψ_max = min( 4, √(4θ) )`. Worked **by hand**:
///
/// * θ = 0.04 → `√(4·0.04) = √0.16 = 0.4`, and `4 > 0.4`, so `ψ_max = 0.4`.
/// * θ = 0.25 → `√(4·0.25) = √1.0 = 1.0`, and `4 > 1.0`, so `ψ_max = 1.0`.
/// * θ = 5.0  → `√(4·5) = √20 ≈ 4.4721`, but `4 < 4.4721`, so the *large-strike*
///   cap binds: `ψ_max = 4`.
///
/// These are independent of any code path — pure arithmetic on the published
/// inequalities — so they catch a mis-stated bound that a calibrator-and-oracle
/// pair could otherwise share.
#[test]
fn butterfly_cap_matches_hand_computed_constants() {
    // The implementation shrinks the cap by (1 − 1e-9) for strict-boundary safety;
    // the hand value is the exact mathematical bound, so compare with that margin.
    const SHRINK: f64 = 1.0 - 1e-9;

    let cases = [
        (0.04_f64, 0.4_f64), // curvature bound binds
        (0.25, 1.0),         // curvature bound binds
        (5.0, 4.0),          // large-strike bound binds
    ];
    for &(theta, hand_cap) in &cases {
        let got = ExtendedSlice::butterfly_psi_cap(theta, 0.0);
        let expected = SHRINK * hand_cap;
        assert!(
            (got - expected).abs() <= 1e-12 * hand_cap.max(1.0),
            "butterfly_psi_cap({theta},0) = {got}, hand-computed {expected} (= (1−1e-9)·{hand_cap})"
        );
        // A slice exactly at the cap is butterfly-free; one a hair over is not.
        let at_cap = ExtendedSlice::new(theta, 0.0, got);
        assert!(
            at_cap.is_butterfly_free(),
            "θ={theta}: ψ at cap must be butterfly-free"
        );
        let over = ExtendedSlice::new(theta, 0.0, hand_cap * 1.001);
        assert!(
            !over.is_butterfly_free(),
            "θ={theta}: ψ = {} (0.1% over the hand cap {hand_cap}) must violate butterfly",
            hand_cap * 1.001
        );
    }
}

/// Row (iv, cont.) — **hand-pinned calendar pair condition** (Hendriks-Martini
/// 2019, Prop. 3.1). For consecutive slices `θ₁ < θ₂` the surface is calendar-
/// arbitrage-free iff `ψ₁ ≤ ψ₂` AND `|ρ₂ψ₂ − ρ₁ψ₁| ≤ ψ₂ − ψ₁`.
///
/// A hand-worked pair (chosen so both slices are individually butterfly-free —
/// each ψ verified below its curvature cap `√(4θ/(1+|ρ|))` by hand):
/// * slice 1: θ₁ = 0.04, ρ₁ = −0.5, ψ₁ = 0.20  → ATM skew ρ₁ψ₁ = **−0.10**.
///   curvature cap `√(4·0.04/1.5) = √0.106… ≈ 0.3266 > 0.20` ✓.
/// * slice 2: θ₂ = 0.08, ρ₂ = −0.25, ψ₂ = 0.30 → ATM skew ρ₂ψ₂ = **−0.075**.
///   curvature cap `√(4·0.08/1.25) = √0.256 = 0.506 > 0.30` ✓.
///
/// Hand checks: ψ-gap = 0.30 − 0.20 = **0.10**; skew-gap = |−0.075 − (−0.10)| =
/// |0.025| = **0.025 ≤ 0.10** ⇒ calendar-free. Now perturb slice 2's skew to
/// ρ₂ψ₂ = +0.05 (ρ₂ = +0.1667): skew-gap = |0.05 − (−0.10)| = **0.15 > 0.10** ⇒
/// a calendar crossing, must be flagged. These thresholds are pure arithmetic on
/// the published inequality, pinned independently of the code.
#[test]
fn calendar_pair_condition_matches_hand_computed_constants() {
    let s1 = ExtendedSlice::new(0.04, -0.5, 0.20);
    assert!(s1.is_butterfly_free());
    // ATM skew is exactly ρ₁ψ₁ = −0.10.
    assert!((s1.rho * s1.psi - (-0.10)).abs() <= 1e-15);

    let s2_ok = ExtendedSlice::new(0.08, -0.25, 0.30);
    assert!(s2_ok.is_butterfly_free());
    assert!((s2_ok.rho * s2_ok.psi - (-0.075)).abs() <= 1e-15);
    // ψ-gap 0.10, skew-gap 0.025 ⇒ calendar-free.
    assert!(
        s1.is_calendar_free_with(&s2_ok),
        "hand-computed calendar-free pair must pass: skew-gap 0.025 ≤ ψ-gap 0.10"
    );

    // Perturb the second slice's ATM skew across the boundary: target ρ₂ψ₂ = +0.05
    // with ψ₂ = 0.30 ⇒ ρ₂ = 0.05/0.30 = 0.16666… (well inside the butterfly cap).
    let rho2_bad = 0.05 / 0.30;
    let s2_bad = ExtendedSlice::new(0.08, rho2_bad, 0.30);
    assert!(s2_bad.is_butterfly_free());
    let skew_gap = (s2_bad.rho * s2_bad.psi - s1.rho * s1.psi).abs();
    let psi_gap = s2_bad.psi - s1.psi;
    // Hand: skew-gap = |0.05 − (−0.10)| = 0.15, ψ-gap = 0.10.
    assert!(
        (skew_gap - 0.15).abs() <= 1e-12,
        "skew-gap should be 0.15, got {skew_gap}"
    );
    assert!(
        (psi_gap - 0.10).abs() <= 1e-12,
        "ψ-gap should be 0.10, got {psi_gap}"
    );
    assert!(
        !s1.is_calendar_free_with(&s2_bad),
        "hand-computed crossing pair must be flagged: skew-gap 0.15 > ψ-gap 0.10"
    );

    // And the calibrator's projection RESCUES the crossing candidate: projecting
    // the bad second slice after s1 yields a calendar-free pair (no-arb by
    // construction), proving the projection — not luck — is what guarantees it.
    let rescued = s1.project_after(0.08, rho2_bad, 0.30);
    assert!(rescued.is_butterfly_free());
    assert!(
        s1.is_calendar_free_with(&rescued),
        "project_after must rescue the hand-computed crossing into the no-arb cone"
    );
}
