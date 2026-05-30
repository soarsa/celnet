//! Robust strike↔delta root-finder for the configured delta convention.
//!
//! FX smiles are quoted in delta space, so pricing a quoted point requires
//! solving `Δ(K) = Δ*` for the strike `K` in the configured [`DeltaConvention`]
//! (`docs/ANALYTICS-SPEC.md` §3.5). The map is nonlinear and vol-dependent, so
//! we use a **bracketing Brent-style bisection to a sign change, then Newton
//! refinement** with the closed-form `∂Δ/∂K` ([`crate::delta_d_strike`]) — falling
//! back to the bracket whenever a Newton step would leave it (guaranteed
//! convergence with near-quadratic speed where the function is well-behaved).
//!
//! The **premium-adjusted call delta is non-monotone in strike** (it rises from
//! `0`, peaks at [`premium_adjusted_call_delta_max`], then falls). We handle
//! that branch explicitly: bracket strictly *below* the maximum-delta strike (the
//! standard OTM-call branch on which delta is increasing) and reject targets that
//! exceed the achievable maximum. All other conventions are monotone and bracket
//! over the full strike range.
//!
//! The vol passed in is the smile vol *at the trial strike* in the calling
//! layer; here we take it as a fixed argument (the surface layer iterates the
//! outer smile/strike fixed point). Inputs are mutated only through a cloned
//! [`VanillaInputs`]; the function is pure and zero-allocation.

use celnet_core::math::sqrt;
use celnet_types::{DeltaConvention, OptionType, VanillaInputs};

use crate::delta::{delta, delta_d_strike, premium_adjusted_call_delta_max};

/// Why a strike↔delta inversion could not be performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaSolveError {
    /// The requested delta has the wrong sign for the option type (a call
    /// delta must be `≥ 0`, a put delta `≤ 0`).
    WrongSign,
    /// The requested delta is unreachable: its magnitude exceeds `1` for an
    /// unadjusted convention, or the premium-adjusted call maximum for the
    /// premium-adjusted branch.
    Unreachable,
    /// The bracketing/iteration failed to converge within the iteration budget
    /// (should not occur for well-posed inputs; surfaced rather than panicking).
    NoConvergence,
}

/// Absolute convergence tolerance on the delta residual.
const DELTA_TOL: f64 = 1e-12;
/// Maximum bracketing/iteration steps (bisection alone needs ≲60 for f64).
const MAX_ITERS: usize = 200;

/// Solve for the strike whose `opt` delta equals `target_delta` in `conv`.
///
/// `template` supplies spot, vol, time and the two rates; its `strike` field is
/// ignored (it is the unknown). Returns the solved strike or a
/// [`DeltaSolveError`]. `target_delta` is signed in market convention
/// (`+` for calls, `−` for puts).
///
/// # Errors
///
/// Returns [`DeltaSolveError`] when the target has the wrong sign, is
/// unreachable in the convention, or iteration fails to converge.
pub fn strike_from_delta(
    conv: DeltaConvention,
    opt: OptionType,
    target_delta: f64,
    template: &VanillaInputs,
) -> Result<f64, DeltaSolveError> {
    // Sign discipline: a call delta is non-negative, a put delta non-positive.
    match opt {
        OptionType::Call if target_delta < 0.0 => return Err(DeltaSolveError::WrongSign),
        OptionType::Put if target_delta > 0.0 => return Err(DeltaSolveError::WrongSign),
        _ => {}
    }

    let f = template.forward();
    let at = |k: f64| {
        let mut inp = *template;
        inp.strike = k;
        delta(conv, opt, &inp)
    };

    // Establish a bracket [lo, hi] with a sign change of g(K) = Δ(K) − target.
    // Strike domain is (0, ∞); we search multiplicatively around the forward.
    let (mut lo, mut hi) = bracket(conv, opt, target_delta, template, f, &at)?;

    let g = |k: f64| at(k) - target_delta;
    let (mut glo, mut ghi) = (g(lo), g(hi));
    debug_assert!(glo * ghi <= 0.0, "bracket must straddle the root");

    // Brent-lite: bisection guaranteeing the bracket, Newton when it stays in.
    let mut k = 0.5 * (lo + hi);
    for _ in 0..MAX_ITERS {
        let gk = g(k);
        if gk.abs() <= DELTA_TOL {
            return Ok(k);
        }
        // Shrink the bracket using the current point.
        if glo * gk <= 0.0 {
            hi = k;
            ghi = gk;
        } else {
            lo = k;
            glo = gk;
        }
        let _ = ghi; // kept for bracket bookkeeping symmetry

        // Newton step using ∂Δ/∂K (= ∂g/∂K).
        let mut inp = *template;
        inp.strike = k;
        let slope = delta_d_strike(conv, opt, &inp);
        let next = if slope.abs() > f64::MIN_POSITIVE {
            k - gk / slope
        } else {
            f64::NAN
        };

        k = if next.is_finite() && next > lo && next < hi {
            next
        } else {
            0.5 * (lo + hi)
        };

        if (hi - lo) <= DELTA_TOL * (1.0 + k) {
            return Ok(k);
        }
    }
    // Final bisection answer if Newton stalled exactly on tolerance edge.
    let k = 0.5 * (lo + hi);
    if g(k).abs() <= 1e-9 {
        Ok(k)
    } else {
        Err(DeltaSolveError::NoConvergence)
    }
}

/// Build a sign-change bracket `[lo, hi]` for `g(K) = Δ(K) − target`.
///
/// For monotone conventions the delta sweeps its full range over `(0, ∞)`, so a
/// geometric expansion around the forward finds a sign change. For the
/// non-monotone premium-adjusted **call**, delta rises from `0` to a maximum at
/// `K_max`, then falls back to `0`. The market-quoted delta strike is the
/// out-of-the-money one — the *decreasing* branch `K ∈ [K_max, ∞)` (which
/// contains the ATM and OTM-call region) — so we bracket there and reject
/// `target > Δ(K_max)` (Reiswich & Wystup, 2010, §5.1; Clark, 2011, §3.4: the
/// premium-adjusted call delta is double-valued and the larger-strike solution
/// is taken).
#[allow(clippy::type_complexity)]
fn bracket<G: Fn(f64) -> f64>(
    conv: DeltaConvention,
    opt: OptionType,
    target_delta: f64,
    template: &VanillaInputs,
    f: f64,
    at: &G,
) -> Result<(f64, f64), DeltaSolveError> {
    let prem_adj = matches!(
        conv,
        DeltaConvention::SpotPremiumAdjusted | DeltaConvention::ForwardPremiumAdjusted
    );

    // Reachability + branch handling for the premium-adjusted call.
    if prem_adj && opt == OptionType::Call {
        let k_max = premium_adjusted_call_delta_max(template);
        let delta_max = at(k_max);
        if target_delta > delta_max + 1e-12 {
            return Err(DeltaSolveError::Unreachable);
        }
        // Decreasing (OTM) branch: K from K_max upward. At K_max, Δ = delta_max
        // ≥ target ⇒ g(lo) ≥ 0; as K→∞, Δ→0 ⇒ g(hi) ≤ 0. Expand hi until the
        // sign flips.
        let lo = k_max;
        let mut hi = f.max(k_max) * 2.0;
        let g = |k: f64| at(k) - target_delta;
        let mut iters = 0;
        while g(hi) > 0.0 {
            iters += 1;
            if iters > 64 {
                return Err(DeltaSolveError::Unreachable);
            }
            hi *= 2.0;
        }
        return Ok((lo, hi));
    }

    // Reachability for unadjusted: |target| ≤ 1 (×factor for spot, but factor ≤ 1
    // only shrinks the range, so the achievable max is at extreme strikes).
    // We bracket by geometric expansion and trust the sign change; if none is
    // found the target is unreachable.
    let mut lo = tiny_strike(template, f);
    let mut hi = f;
    let g = |k: f64| at(k) - target_delta;

    // Calls: delta decreases from ~max (small K) to 0 (large K). Puts: delta
    // increases from ~min (small K, near −max) to 0 (large K). In both cases a
    // sign change is found by expanding the bracket outward.
    let mut glo = g(lo);
    let mut ghi = g(hi);
    let mut iters = 0;
    while glo * ghi > 0.0 {
        iters += 1;
        if iters > 64 {
            return Err(DeltaSolveError::Unreachable);
        }
        // Expand toward where the residual is shrinking in magnitude.
        if glo.abs() < ghi.abs() {
            lo *= 0.5;
            glo = g(lo);
        } else {
            hi *= 2.0;
            ghi = g(hi);
        }
    }
    Ok((lo, hi))
}

/// A safe lower strike bound: far enough below the forward that delta is at its
/// extreme but strictly positive (avoids `ln(0)`).
#[inline]
fn tiny_strike(template: &VanillaInputs, f: f64) -> f64 {
    // 12 standard deviations of log-moneyness below the forward is effectively
    // the deep-ITM-call / deep-OTM-put boundary while remaining finite.
    let vsqt = template.vol * sqrt(template.t);
    f * celnet_core::math::exp(-12.0 * vsqt - 6.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;

    fn base() -> VanillaInputs {
        // EURUSD-like: spot 1.10, 10% vol, 1Y, 2% DOM, 1% FOR.
        VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.02, 0.01)
    }

    const ALL: [DeltaConvention; 4] = [
        DeltaConvention::SpotUnadjusted,
        DeltaConvention::ForwardUnadjusted,
        DeltaConvention::SpotPremiumAdjusted,
        DeltaConvention::ForwardPremiumAdjusted,
    ];

    /// Deep-wing & extreme-delta solves that genuinely exercise the
    /// **bracketing** machinery (`bracket` / `tiny_strike`), not just the
    /// converged root. Bisection is self-correcting, so a degraded bracket
    /// (wrong `tiny_strike` lower bound, wrong geometric-expansion factor, or a
    /// flipped expansion comparison) still converges *whenever it happens to
    /// straddle the root* — which masks those mutants on near-ATM targets. Here
    /// we drive the solver into regimes where the root lies far outside the
    /// initial `[tiny_strike, F]` bracket, so a broken lower bound or expansion
    /// either fails to straddle (→ `Unreachable`/`NoConvergence`) or lands on the
    /// wrong side:
    ///
    ///  * very small deltas (1Δ, 0.5Δ) ⇒ a strike far in the OTM wing, forcing
    ///    several `hi *= 2` expansions (a too-slow `+=` expansion hits the
    ///    64-step cap) and, for puts, a very small `tiny_strike` lower bound;
    ///  * large unadjusted call deltas (deep ITM) ⇒ a strike far below `F`,
    ///    forcing `lo` expansion down toward `tiny_strike` (a `tiny_strike` that
    ///    is too large, zero, or wrongly signed cannot bracket the root);
    ///  * a very low-vol, long-dated regime where `tiny_strike`'s
    ///    `exp(−12σ√T − 6)` exponent must be small enough to clear the wing.
    ///
    /// Every solve must succeed AND round-trip to tight tolerance.
    #[test]
    fn deep_wing_solves_exercise_bracketing() {
        let regimes = [
            VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.02, 0.01),
            VanillaInputs::new(1.10, 1.10, 0.04, 3.0, 0.02, 0.01), // low vol, long T
            VanillaInputs::new(100.0, 100.0, 0.45, 0.25, 0.05, 0.0), // high vol, short T
            VanillaInputs::new(0.80, 0.80, 0.20, 1.0, -0.01, 0.06),
        ];
        for i in &regimes {
            for conv in [
                DeltaConvention::SpotUnadjusted,
                DeltaConvention::ForwardUnadjusted,
            ] {
                // Tiny OTM deltas — far OTM wing strikes (forces hi-expansion for
                // calls, lo→tiny_strike for puts).
                for &mag in &[0.01_f64, 0.005, 0.02] {
                    for opt in [OptionType::Call, OptionType::Put] {
                        let target = if opt == OptionType::Call { mag } else { -mag };
                        let k = strike_from_delta(conv, opt, target, i).unwrap_or_else(|e| {
                            panic!("{conv:?} {opt:?} tiny Δ={target} on {i:?} failed: {e:?}")
                        });
                        let mut probe = *i;
                        probe.strike = k;
                        let back = delta(conv, opt, &probe);
                        assert!(
                            is_close(back, target, 1e-9, 1e-11),
                            "{conv:?} {opt:?} tiny Δ*={target} back={back} K={k} on {i:?}"
                        );
                        // OTM wing geometry: a tiny call delta sits ABOVE the
                        // forward; a tiny put delta BELOW.
                        let f = i.forward();
                        if opt == OptionType::Call {
                            assert!(k > f, "{conv:?} tiny call wing K={k} must exceed F={f}");
                        } else {
                            assert!(k < f, "{conv:?} tiny put wing K={k} must be below F={f}");
                        }
                    }
                }
                // Large unadjusted CALL delta (deep ITM) — strike far below F,
                // forcing lo to expand down toward tiny_strike. The unadjusted
                // call delta is capped by the discount factor (e^{−r_f T} for
                // spot, 1 for forward), so target a high fraction of that cap to
                // stay reachable yet deep enough to drive lo-expansion.
                let cap = match conv {
                    DeltaConvention::SpotUnadjusted => (-i.r_for * i.t).exp(),
                    _ => 1.0,
                };
                for &frac in &[0.90_f64, 0.97] {
                    let mag = frac * cap;
                    let k = strike_from_delta(conv, OptionType::Call, mag, i).unwrap_or_else(|e| {
                        panic!("{conv:?} deep-ITM call Δ={mag} (cap {cap}) on {i:?} failed: {e:?}")
                    });
                    let mut probe = *i;
                    probe.strike = k;
                    let back = delta(conv, OptionType::Call, &probe);
                    assert!(
                        is_close(back, mag, 1e-9, 1e-11),
                        "{conv:?} deep-ITM call Δ*={mag} back={back} K={k} on {i:?}"
                    );
                    assert!(
                        k < i.forward(),
                        "{conv:?} deep-ITM call K={k} must be below F={}",
                        i.forward()
                    );
                }
            }
        }
    }

    /// `tiny_strike` must be a strictly positive lower bound that sits **below**
    /// the deepest reachable wing strike for the regime, so the unadjusted
    /// bracket always straddles. We pin its contract directly (independent of the
    /// solver): it is positive, finite, far below the forward, and below the
    /// 0.5Δ-put strike (the deepest wing the solver is asked to reach in
    /// practice) — a mutant that returns `0.0`, inflates it, or flips a factor
    /// breaks at least one of these.
    #[test]
    fn tiny_strike_is_a_valid_lower_bound() {
        for i in [
            base(),
            VanillaInputs::new(1.10, 1.10, 0.04, 3.0, 0.02, 0.01),
            VanillaInputs::new(100.0, 100.0, 0.45, 0.25, 0.05, 0.0),
        ] {
            let f = i.forward();
            let ts = tiny_strike(&i, f);
            assert!(
                ts.is_finite() && ts > 0.0,
                "tiny_strike must be positive finite: {ts}"
            );
            assert!(ts < f, "tiny_strike {ts} must be below forward {f}");
            // Below the 0.5Δ-put strike (the deepest OTM-put wing exercised).
            let deep_put =
                strike_from_delta(DeltaConvention::SpotUnadjusted, OptionType::Put, -0.005, &i)
                    .unwrap();
            assert!(
                ts < deep_put,
                "tiny_strike {ts} must sit below the 0.5Δ-put strike {deep_put} so the \
                 bracket straddles"
            );
        }
    }

    /// strike → delta → strike round-trips across all four conventions, both
    /// option types, a 25Δ/10Δ set.
    #[test]
    fn round_trip_all_conventions() {
        let i = base();
        for conv in ALL {
            for opt in [OptionType::Call, OptionType::Put] {
                for mag in [0.10_f64, 0.25, 0.40] {
                    let target = match opt {
                        OptionType::Call => mag,
                        OptionType::Put => -mag,
                    };
                    let k = strike_from_delta(conv, opt, target, &i).unwrap();
                    let mut inp = i;
                    inp.strike = k;
                    let back = delta(conv, opt, &inp);
                    assert!(
                        is_close(back, target, 1e-7, 1e-9),
                        "{conv:?} {opt:?} Δ*={target} got Δ={back} K={k}"
                    );
                    // And strike round-trips from that delta.
                    let k2 = strike_from_delta(conv, opt, back, &i).unwrap();
                    assert!(is_close(k2, k, 1e-7, 1e-9), "{conv:?} {opt:?} strike RT");
                }
            }
        }
    }

    /// 25Δ call strike sits above ATM-forward; 25Δ put strike below — the basic
    /// FX-smile geometry (Clark 2011, §3.4).
    #[test]
    fn delta_set_strikes_straddle_forward() {
        let i = base();
        let f = i.forward();
        for conv in ALL {
            let kc = strike_from_delta(conv, OptionType::Call, 0.25, &i).unwrap();
            let kp = strike_from_delta(conv, OptionType::Put, -0.25, &i).unwrap();
            assert!(
                kc > f,
                "{conv:?} 25dC strike {kc} should exceed forward {f}"
            );
            assert!(
                kp < f,
                "{conv:?} 25dP strike {kp} should be below forward {f}"
            );
            // 10Δ wings are further out than 25Δ.
            let kc10 = strike_from_delta(conv, OptionType::Call, 0.10, &i).unwrap();
            let kp10 = strike_from_delta(conv, OptionType::Put, -0.10, &i).unwrap();
            assert!(kc10 > kc, "{conv:?} 10dC beyond 25dC");
            assert!(kp10 < kp, "{conv:?} 10dP beyond 25dP");
        }
    }

    /// Premium-adjusted call delta is capped: a target above the achievable
    /// maximum is rejected, and the maximum-delta strike is the turning point.
    #[test]
    fn premium_adjusted_call_is_capped_and_non_monotone() {
        let i = base();
        let conv = DeltaConvention::SpotPremiumAdjusted;
        let k_max = premium_adjusted_call_delta_max(&i);
        let mut at_max = i;
        at_max.strike = k_max;
        let dmax = delta(conv, OptionType::Call, &at_max);

        // A reachable target resolves on the OTM (decreasing) branch K > K_max —
        // the market-quoted side — not the deep-ITM increasing branch.
        let reachable = dmax * 0.5;
        let k = strike_from_delta(conv, OptionType::Call, reachable, &i).unwrap();
        assert!(
            k > k_max,
            "premium-adjusted call must resolve on the OTM K > K_max branch (k={k}, k_max={k_max})"
        );
        // It round-trips to the requested delta.
        let mut probe = i;
        probe.strike = k;
        assert!(celnet_core::is_close(
            delta(conv, OptionType::Call, &probe),
            reachable,
            1e-7,
            1e-9
        ));

        // Above the peak is unreachable.
        assert_eq!(
            strike_from_delta(conv, OptionType::Call, dmax + 0.05, &i),
            Err(DeltaSolveError::Unreachable)
        );

        // Non-monotone: a strike beyond K_max has a *lower* delta than at K_max.
        let mut beyond = i;
        beyond.strike = k_max * 1.10;
        assert!(delta(conv, OptionType::Call, &beyond) < dmax);
    }

    /// Wrong-sign targets are rejected, not silently solved.
    #[test]
    fn wrong_sign_rejected() {
        let i = base();
        assert_eq!(
            strike_from_delta(DeltaConvention::SpotUnadjusted, OptionType::Call, -0.25, &i),
            Err(DeltaSolveError::WrongSign)
        );
        assert_eq!(
            strike_from_delta(
                DeltaConvention::ForwardUnadjusted,
                OptionType::Put,
                0.25,
                &i
            ),
            Err(DeltaSolveError::WrongSign)
        );
    }

    /// **An out-of-range unadjusted target is reported `Unreachable` — and the
    /// bracket-expansion loop terminates while doing so.** For the unadjusted
    /// conventions the achievable call delta is capped by the carry factor
    /// (`e^{−r_f T}` for spot, `1` for forward); a target strictly above that cap
    /// has no finite strike. The geometric bracket never finds a sign change, so
    /// the expansion's safety counter (`bracket` line 197 `iters += 1`) MUST
    /// advance to its cap (line 198) and return `Unreachable`.
    ///
    /// This pins the counter's *monotonic advance*: a mutant that turns the
    /// `iters += 1` into `iters -= 1` or `iters *= 1` makes the counter never
    /// reach the cap, so the loop spins forever on an unreachable target — a
    /// non-terminating behaviour the (timeout-bounded) mutation harness flags,
    /// and which this test's expectation of a prompt `Unreachable` return
    /// enforces. The unmutated solver returns immediately.
    #[test]
    fn out_of_range_unadjusted_target_is_unreachable() {
        let i = base();
        // Spot-unadjusted call cap = e^{-r_f T}; pick a target above it but ≤ 1.
        let cap_spot = (-i.r_for * i.t).exp();
        let above_cap = 0.5 * (cap_spot + 1.0);
        assert!(above_cap > cap_spot && above_cap < 1.0);
        assert_eq!(
            strike_from_delta(
                DeltaConvention::SpotUnadjusted,
                OptionType::Call,
                above_cap,
                &i
            ),
            Err(DeltaSolveError::Unreachable),
            "a spot-unadjusted call delta above the carry-factor cap {cap_spot} is unreachable"
        );
        // A magnitude above 1 is unreachable for every unadjusted convention and
        // both option types (|Δ| ≤ factor ≤ 1).
        for conv in [
            DeltaConvention::SpotUnadjusted,
            DeltaConvention::ForwardUnadjusted,
        ] {
            assert_eq!(
                strike_from_delta(conv, OptionType::Call, 1.5, &i),
                Err(DeltaSolveError::Unreachable),
                "{conv:?} call Δ*=1.5 must be Unreachable"
            );
            assert_eq!(
                strike_from_delta(conv, OptionType::Put, -1.5, &i),
                Err(DeltaSolveError::Unreachable),
                "{conv:?} put Δ*=-1.5 must be Unreachable"
            );
        }
    }

    /// **Zero is a sign-valid (if unreachable-in-the-limit) delta, NOT a
    /// wrong-sign target.** The sign guard rejects a *strictly* mis-signed
    /// target: a call delta `< 0` (line 67) or a put delta `> 0` (line 68). A
    /// zero target has the correct sign for either option type — the delta of a
    /// call tends to `0` from above as `K → ∞` (a put from below), so `0` is the
    /// limiting wing value, never a sign violation. The solver must therefore
    /// treat `Δ* = 0` as a reachability question (it returns `Unreachable`, since
    /// the limit is not attained at a finite strike), never `WrongSign`.
    ///
    /// A mutant that loosens the guard to `≤ 0` (call) or `≥ 0` (put) would
    /// mis-route `Δ* = 0` to `WrongSign`; we pin that exact boundary.
    #[test]
    fn zero_delta_is_not_wrong_sign() {
        let i = base();
        for conv in ALL {
            let call = strike_from_delta(conv, OptionType::Call, 0.0, &i);
            assert_ne!(
                call,
                Err(DeltaSolveError::WrongSign),
                "{conv:?} call Δ*=0 must not be WrongSign (0 is a valid call-delta sign); got {call:?}"
            );
            let put = strike_from_delta(conv, OptionType::Put, 0.0, &i);
            assert_ne!(
                put,
                Err(DeltaSolveError::WrongSign),
                "{conv:?} put Δ*=0 must not be WrongSign (0 is a valid put-delta sign); got {put:?}"
            );
            // And the strictly mis-signed targets ARE rejected as WrongSign —
            // pinning that the guard still fires on the correct side.
            assert_eq!(
                strike_from_delta(conv, OptionType::Call, -1e-6, &i),
                Err(DeltaSolveError::WrongSign),
                "{conv:?} negative call delta must be WrongSign"
            );
            assert_eq!(
                strike_from_delta(conv, OptionType::Put, 1e-6, &i),
                Err(DeltaSolveError::WrongSign),
                "{conv:?} positive put delta must be WrongSign"
            );
        }
    }

    /// Pin solver correctness with a TIGHT residual and an independent
    /// (derivative-free) bisection oracle. The production solver uses Newton
    /// steps off `delta_d_strike`; a wrong-arithmetic mutant there can be masked
    /// because the guarded bisection still converges. So here we (a) require the
    /// returned strike to reproduce the target delta to a far tighter tolerance
    /// than the `1e-6` round-trip property, and (b) cross-check it against a pure
    /// bisection that never touches `delta_d_strike` — the two must agree to
    /// machine-ish precision, which catches a solver that "converges to the wrong
    /// place" yet still self-consistently round-trips its own (mutated) delta.
    #[test]
    fn solver_strike_matches_independent_bisection() {
        let i = base();
        for conv in ALL {
            for opt in [OptionType::Call, OptionType::Put] {
                for mag in [0.10_f64, 0.25, 0.40] {
                    let target = match opt {
                        OptionType::Call => mag,
                        OptionType::Put => -mag,
                    };
                    let k = strike_from_delta(conv, opt, target, &i).unwrap();

                    // (a) Tight residual: the solved strike reproduces target.
                    let mut probe = i;
                    probe.strike = k;
                    let residual = delta(conv, opt, &probe) - target;
                    assert!(
                        residual.abs() < 1e-11,
                        "{conv:?} {opt:?} loose residual {residual} at K={k}"
                    );

                    // (b) Independent derivative-free bisection oracle. For the
                    // premium-adjusted call we restrict to the OTM (decreasing)
                    // branch K ≥ K_max, matching the production branch choice; all
                    // other conventions are monotone over (0, ∞).
                    let prem_adj_call = matches!(
                        conv,
                        DeltaConvention::SpotPremiumAdjusted
                            | DeltaConvention::ForwardPremiumAdjusted
                    ) && opt == OptionType::Call;
                    let lo0 = if prem_adj_call {
                        premium_adjusted_call_delta_max(&i)
                    } else {
                        1e-6 * i.forward()
                    };
                    let oracle = bisect_strike(conv, opt, target, &i, lo0, 50.0 * i.forward());
                    assert!(
                        is_close(k, oracle, 1e-7, 1e-9),
                        "{conv:?} {opt:?} solver K={k} disagrees with bisection oracle {oracle}"
                    );
                }
            }
        }
    }

    /// A pure, derivative-free bisection on `Δ(K) − target` over `[lo, hi]`,
    /// independent of the production solver's Newton step / `delta_d_strike`.
    fn bisect_strike(
        conv: DeltaConvention,
        opt: OptionType,
        target: f64,
        template: &VanillaInputs,
        lo: f64,
        hi: f64,
    ) -> f64 {
        let g = |k: f64| {
            let mut inp = *template;
            inp.strike = k;
            delta(conv, opt, &inp) - target
        };
        let (mut a, mut b) = (lo, hi);
        let (mut ga, gb) = (g(a), g(b));
        assert!(
            ga * gb <= 0.0,
            "oracle bracket must straddle: g(lo)={ga} g(hi)={gb}"
        );
        for _ in 0..200 {
            let m = 0.5 * (a + b);
            let gm = g(m);
            // Converged when the residual or the bracket width is negligible.
            if gm.abs() < 1e-15 || (b - a) < 1e-13 * (1.0 + m) {
                return m;
            }
            if ga * gm <= 0.0 {
                b = m;
            } else {
                a = m;
                ga = gm;
            }
        }
        0.5 * (a + b)
    }

    /// A deterministic, wide market-state grid covering the regimes where the
    /// solver's iteration internals become load-bearing: very low vol, very short
    /// and very long expiries, large and small rate spreads, and large strikes.
    /// These are the cells where a perturbed Newton step / guard / convergence
    /// width (mutating `strike_from_delta` lines 88/95/108/109/114/120/125/126)
    /// or a perturbed bracket-expansion (lines 162–206) or `tiny_strike`
    /// (219/220) actually moves the converged answer off the root or makes the
    /// solver fail — masked at the benign EURUSD `base()` regime but exposed here.
    fn wide_regimes() -> [VanillaInputs; 12] {
        [
            VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.02, 0.01),
            VanillaInputs::new(50.0, 50.0, 0.021, 0.05, -0.01, 0.073), // low vol, short T
            VanillaInputs::new(135.0, 135.0, 0.031, 3.0, 0.064, 0.052), // long T, low vol
            VanillaInputs::new(2.70, 2.70, 0.032, 3.1, 0.031, 0.058),
            VanillaInputs::new(100.0, 100.0, 0.45, 0.25, 0.05, 0.0), // high vol, short T
            VanillaInputs::new(0.80, 0.80, 0.20, 1.0, -0.01, 0.06),
            VanillaInputs::new(1.25, 1.25, 0.60, 2.0, 0.03, 0.03),
            VanillaInputs::new(7.50, 7.50, 0.08, 0.10, 0.09, -0.02),
            VanillaInputs::new(0.65, 0.65, 0.12, 0.5, 0.04, 0.04),
            VanillaInputs::new(180.0, 180.0, 0.18, 1.5, 0.01, 0.07),
            // High vol + very long T: the premium-adjusted-call hi-expansion leg
            // (`bracket` lines 169–177) sweeps many doublings here, and a corrupted
            // residual `g(k)` on that leg (line 170 `at−td` → `at/td`) blows the
            // expansion past its 64-step cap, failing a reachable target.
            VanillaInputs::new(97.0, 97.0, 0.58, 4.5, 0.02, -0.007),
            VanillaInputs::new(21.0, 21.0, 0.55, 4.9, 0.06, 0.078),
        ]
    }

    /// The reachable, signed delta grid for `(conv, opt, i)` — for the
    /// premium-adjusted call the magnitude is capped strictly below the peak
    /// delta so every entry is a genuinely solvable target.
    fn reachable_targets(conv: DeltaConvention, opt: OptionType, i: &VanillaInputs) -> Vec<f64> {
        let mags = [0.005_f64, 0.01, 0.05, 0.10, 0.25, 0.40, 0.45];
        let pa_call = matches!(
            conv,
            DeltaConvention::SpotPremiumAdjusted | DeltaConvention::ForwardPremiumAdjusted
        ) && opt == OptionType::Call;
        let cap = if pa_call {
            let k_max = premium_adjusted_call_delta_max(i);
            let mut at = *i;
            at.strike = k_max;
            0.9 * delta(conv, OptionType::Call, &at)
        } else {
            f64::INFINITY
        };
        mags.iter()
            .map(|&m| if opt == OptionType::Call { m } else { -m })
            .map(|t| if pa_call { t.min(cap) } else { t })
            .collect()
    }

    /// **Solver tolerance is the SOLVER's, not the test's.** The production loop
    /// declares convergence at a `1e-12` delta residual (`DELTA_TOL`), so a
    /// correct solve must reproduce the target delta to that order. We assert a
    /// `5e-12` absolute residual across the full wide grid × all four conventions
    /// × both option types × the reachable delta grid.
    ///
    /// This is the gate that the looser `1e-11` round-trip / oracle tests cannot
    /// be: several iteration-internal mutants (`strike_from_delta`'s Newton-step
    /// arithmetic at line 109, its in-bracket Newton guard at 114, the slope-
    /// magnitude test at 108) leave the guarded bisection *converging*, but only
    /// to a `~3e-11` residual — above `5e-12` yet below `1e-11`. Likewise a
    /// broken premium-adjusted hi-expansion factor (line 169) or a corrupted
    /// `tiny_strike` carry (line 220) lands the answer a finite distance off the
    /// root. Pinning the residual at the solver's own `DELTA_TOL` order, in the
    /// low-vol / short-and-long-T regimes where the Newton step is most active,
    /// kills them without ever referencing the convergence trajectory.
    #[test]
    fn solver_residual_is_at_solver_tolerance_on_wide_grid() {
        for i in wide_regimes() {
            for conv in ALL {
                for opt in [OptionType::Call, OptionType::Put] {
                    for target in reachable_targets(conv, opt, &i) {
                        let k = strike_from_delta(conv, opt, target, &i).unwrap_or_else(|e| {
                            panic!(
                                "{conv:?} {opt:?} Δ*={target} on {i:?} unexpectedly failed: {e:?}"
                            )
                        });
                        let mut probe = i;
                        probe.strike = k;
                        let residual = delta(conv, opt, &probe) - target;
                        assert!(
                            residual.abs() < 5e-12,
                            "{conv:?} {opt:?} Δ*={target} on {i:?}: residual {residual:e} \
                             exceeds the solver tolerance (K={k})"
                        );
                    }
                }
            }
        }
    }

    /// **Every reachable target must SOLVE — no spurious failure or panic.** A
    /// mutant that corrupts the initial Newton midpoint (`strike_from_delta` line
    /// 88, `0.5*(lo+hi)` → `0.5+(lo+hi)` / `0.5*(lo−hi)` / `0.5*(lo*hi)`), the
    /// bracket-expansion step or its safety counter on the premium-adjusted call
    /// hi-leg (lines 174/177) or the unadjusted leg (lines 197/206), or the
    /// `tiny_strike` lower-bound carry (line 220 `f*e` → `f/e`) does NOT change a
    /// converged value — it instead drives the iteration out of its bracket or
    /// trips the iteration cap, so the solver returns `NoConvergence` /
    /// `Unreachable` (or panics on a NaN bracket) for a target it must reach.
    /// Asserting `Ok` over the full reachable grid catches exactly that failure
    /// mode, which a residual-only test cannot see.
    #[test]
    fn every_reachable_target_solves_without_failure() {
        for i in wide_regimes() {
            for conv in ALL {
                for opt in [OptionType::Call, OptionType::Put] {
                    for target in reachable_targets(conv, opt, &i) {
                        let got = strike_from_delta(conv, opt, target, &i);
                        assert!(
                            got.is_ok(),
                            "{conv:?} {opt:?} reachable Δ*={target} on {i:?} must solve, got {got:?}"
                        );
                    }
                }
            }
        }
    }

    /// **Premium-adjusted call reachability boundary is pinned at the peak
    /// delta.** The branch rejects `target > Δ(K_max) + 1e-12` (`bracket` line
    /// 162). Two adjacent probes pin the predicate sharply:
    ///
    ///  * a target a hair *below* the peak (`Δ(K_max) − 5e-10`) is reachable and
    ///    must solve — this kills the `+ 1e-12 → − 1e-12` mutant (line 162:37),
    ///    which would shrink the reachable ceiling and wrongly reject it;
    ///  * a target a hair *above* the peak (`Δ(K_max) + 1e-6`) is unreachable and
    ///    must return `Unreachable` — this pins the `>`/`>=` boundary (line
    ///    162:25) and the rejection itself.
    ///
    /// The solved near-peak strike must also reproduce its target to solver
    /// tolerance and sit on the OTM (decreasing, `K ≥ K_max`) branch.
    #[test]
    fn premium_adjusted_call_reachability_boundary_is_exact() {
        for conv in [
            DeltaConvention::SpotPremiumAdjusted,
            DeltaConvention::ForwardPremiumAdjusted,
        ] {
            for i in wide_regimes() {
                let k_max = premium_adjusted_call_delta_max(&i);
                let mut at_max = i;
                at_max.strike = k_max;
                let dmax = delta(conv, OptionType::Call, &at_max);

                // Just below the peak, INSIDE the `+1e-12` reachability margin:
                // a target in `(Δ(K_max) − 1e-12, Δ(K_max)]` is reachable under
                // the production `target > Δ(K_max) + 1e-12` test, but a mutant
                // that flips the margin sign to `Δ(K_max) − 1e-12` (line 162:37)
                // would classify it as *unreachable* and wrongly reject it.
                // Sitting the probe at `Δ(K_max) − 5e-13` straddles exactly that
                // mutated boundary, so the solve must still succeed.
                let just_below = dmax - 5e-13;
                if just_below > 0.0 {
                    let k = strike_from_delta(conv, OptionType::Call, just_below, &i)
                        .unwrap_or_else(|e| {
                            panic!(
                                "{conv:?} near-peak Δ*={just_below} (peak {dmax}) on {i:?} \
                                 must be reachable, got {e:?}"
                            )
                        });
                    let mut probe = i;
                    probe.strike = k;
                    let back = delta(conv, OptionType::Call, &probe);
                    assert!(
                        is_close(back, just_below, 1e-7, 1e-9),
                        "{conv:?} near-peak Δ*={just_below} back={back} K={k} on {i:?}"
                    );
                    assert!(
                        k >= k_max * (1.0 - 1e-9),
                        "{conv:?} near-peak solve K={k} must be on the OTM branch K≥K_max={k_max}"
                    );
                }
                // A clearly-reachable target well below the peak also solves — this
                // keeps the test honest if the near-peak point is numerically
                // pathological for some regime.
                let clearly = 0.5 * dmax;
                if clearly > 0.0 {
                    assert!(
                        strike_from_delta(conv, OptionType::Call, clearly, &i).is_ok(),
                        "{conv:?} mid-branch Δ*={clearly} (peak {dmax}) on {i:?} must solve"
                    );
                }

                // Just above the peak: unreachable, rejected.
                let just_above = dmax + 1e-6;
                assert_eq!(
                    strike_from_delta(conv, OptionType::Call, just_above, &i),
                    Err(DeltaSolveError::Unreachable),
                    "{conv:?} Δ*={just_above} above peak {dmax} on {i:?} must be Unreachable"
                );
            }
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]
        /// Across random market states and the full delta grid, every convention
        /// round-trips strike → delta → strike to high precision (the premium-
        /// adjusted call uses its reachable, capped grid).
        #[test]
        fn round_trip_property(
            spot in 0.5f64..200.0,
            vol in 0.04f64..0.5,
            t in 0.05f64..3.0,
            r_dom in -0.02f64..0.08,
            r_for in -0.02f64..0.08,
            mag in 0.05f64..0.45,
            conv_ix in 0usize..4,
            is_call in proptest::prelude::any::<bool>(),
        ) {
            let conv = ALL[conv_ix];
            let opt = if is_call { OptionType::Call } else { OptionType::Put };
            let i = VanillaInputs::new(spot, spot, vol, t, r_dom, r_for);
            let target = if is_call { mag } else { -mag };

            // Premium-adjusted call delta is capped; cap the requested magnitude.
            let target = if matches!(
                conv,
                DeltaConvention::SpotPremiumAdjusted | DeltaConvention::ForwardPremiumAdjusted
            ) && opt == OptionType::Call {
                let kmax = premium_adjusted_call_delta_max(&i);
                let mut at = i;
                at.strike = kmax;
                let dmax = delta(conv, OptionType::Call, &at);
                target.min(0.9 * dmax)
            } else {
                target
            };

            if let Ok(k) = strike_from_delta(conv, opt, target, &i) {
                let mut probe = i;
                probe.strike = k;
                let back = delta(conv, opt, &probe);
                proptest::prop_assert!(
                    is_close(back, target, 1e-6, 1e-9),
                    "{conv:?} {opt:?} Δ*={target} back={back} K={k}"
                );
            }
        }
    }
}
