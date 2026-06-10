//! Pivot Target-Redemption Accumulator (pivot TRA) — a strip of periodic fixings
//! with a TWO-level piecewise-linear per-fixing payoff (a `pivot` P at which the
//! geared adverse leg engages, distinct from the `strike` K against which intrinsic
//! is measured) and a cumulative-gain `target` redemption (knock-out on target)
//! with explicit gap-risk handling at the breaching fixing.
//!
//! Collapses to the plain [`crate::tarf::Tarf`] exactly when `pivot == strike`
//! (gated to 1e-12 in tests). Priced on the shared counter-based MC engine with
//! antithetic variates + a forward-strip control variate; reproducible from seed.
//!
//! # Structure
//!
//! On each of `n` equally-spaced fixings the realised spot `S_k` produces, for the
//! client, a per-unit cash flow `c_k` that is a piecewise-linear function of `S_k`
//! with a kink at the pivot `P`. With the favourable-side sign `g` (`+1` for a
//! [`celnet_types::OptionType::Call`]-favourable structure, `−1` for a
//! [`celnet_types::OptionType::Put`]-favourable one):
//!
//! ```text
//! c_k(S) = 𝟙[g·(S−P) ≥ 0] · g·(S−K)  −  𝟙[g·(S−P) < 0] · L·g·(S−K)
//! ```
//!
//! — slope `g` on the favourable side of the pivot (un-geared) and slope `L·g` on
//! the adverse side (geared by the leverage `L`). The two distinct levels are:
//!
//! * `K` (target strike) — the level intrinsic `S − K` is measured against; the
//!   running sum of the positive part of `c_k` accrues toward the cumulative
//!   target `T`.
//! * `P` (pivot) — the level at which the leg switches from the un-geared
//!   favourable leg to the `L`-geared adverse leg. `P` sitting away from `K` opens
//!   a dead band (`P > K`, call-favourable) or an overlap (`P < K`). Setting
//!   `P == K` recovers the plain TARF exactly.
//!
//! # Redemption (target knock-out)
//!
//! Identical mechanic to [`crate::tarf`]. The cumulative favourable gain
//! `G = Σ max(c_k, 0)` accrues; once `G ≥ T` the structure redeems (no further
//! fixings). The breaching fixing settles per [`RedemptionStyle`] — `FullGain`
//! keeps the overshoot (gap risk), `CappedGain` redeems exactly.
//!
//! # Degeneracy map
//!
//! 1. `P = K`  ⇒  exact [`crate::tarf::Tarf`] (gated to_bits / 1e-12).
//! 2. `T = +∞` with `K = P`  ⇒  geared forward strip (secondary bound).
//!
//! # Numerics
//!
//! Priced on the Monte-Carlo engine ([`crate::rng::CounterRng`] +
//! [`crate::normal::inverse_cdf`]) with antithetic variates and a forward-strip
//! control variate. The terminal law of the GBM at each fixing is exact (no
//! time-step bias between fixings), so the only Monte-Carlo error is statistical.
//! Reproducible from the seed. To reproduce [`crate::tarf`] bit-for-bit in the
//! `P = K` slice, the default pricer uses the same RNG coordinates and arithmetic
//! order; the control-variate estimate lives in a separate entry point.
//!
//! Provenance (doc-only): TARF payoff & gap-risk decomposition — Wystup (2017),
//! *FX Options and Structured Products*; Caspers (2014). Pivot/boosted accumulator
//! structuring — Wystup (2017). Identifiers are purpose-named & vendor-neutral.

use celnet_core::math::{exp, ln, sqrt};
use celnet_types::{OptionType, VanillaInputs};

use crate::normal::inverse_cdf;
use crate::rng::CounterRng;
use crate::tarf::RedemptionStyle; // reuse the frozen gap-risk enum — no duplication

/// A Pivot Target-Redemption Accumulator specification.
///
/// `favourable_side` is the side of `strike` on which the client accrues gains:
/// [`OptionType::Call`] ⇒ gain when `S_k > strike`; [`OptionType::Put`] ⇒ gain when
/// `S_k < strike` (the classic exporter orientation). The `pivot` is the level at
/// which the geared adverse leg engages; setting `pivot == strike` recovers the
/// plain TARF.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PivotTra {
    /// Target strike `K`: the level intrinsic `S − K` is measured against, and the
    /// reference for the cumulative gain that accrues toward `target`.
    pub strike: f64,
    /// Pivot `P`: the kink at which the per-fixing leg switches from the un-geared
    /// favourable leg to the geared adverse leg. `pivot == strike` ⇒ plain TARF.
    pub pivot: f64,
    /// Number of equally-spaced fixing dates over `[0, T]` (the last at `T`).
    pub fixings: usize,
    /// Cumulative gain target. Accrued client gain at or above this redeems.
    pub target: f64,
    /// Gearing/leverage on the adverse leg (the far side of the pivot). `≥ 0`.
    pub leverage: f64,
    /// The side of `strike` on which the client accrues gains.
    pub favourable_side: OptionType,
    /// Per-fixing notional (units of base per fixing).
    pub notional: f64,
    /// Gap-risk settlement convention of the redeeming fixing.
    pub redemption: RedemptionStyle,
}

impl PivotTra {
    /// Validate the basic structural invariants.
    fn validate(&self) {
        assert!(self.fixings >= 1, "pivot TRA needs ≥1 fixing");
        assert!(self.target > 0.0, "pivot TRA target must be positive");
        assert!(
            self.leverage >= 0.0,
            "pivot TRA leverage must be non-negative"
        );
        assert!(self.notional > 0.0, "pivot TRA notional must be positive");
        assert!(
            self.strike > 0.0 && self.pivot > 0.0,
            "strike/pivot must be positive"
        );
    }

    /// The degenerate slice equal to a plain TARF — the canonical way to take the
    /// `P = K` limit in tests.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn as_tarf_slice(
        strike: f64,
        fixings: usize,
        target: f64,
        leverage: f64,
        favourable_side: OptionType,
        notional: f64,
        redemption: RedemptionStyle,
    ) -> Self {
        Self {
            strike,
            pivot: strike,
            fixings,
            target,
            leverage,
            favourable_side,
            notional,
            redemption,
        }
    }
}

/// Monte-Carlo configuration for the pivot-TRA engine.
#[derive(Debug, Clone, Copy)]
pub struct PivotTraMcConfig {
    /// Number of antithetic path **pairs**.
    pub pairs: usize,
    /// Seed for the counter-based RNG.
    pub seed: u64,
}

/// The result of a pivot-TRA Monte-Carlo run.
#[derive(Debug, Clone, Copy)]
pub struct PivotTraResult {
    /// Discounted present value to the **bank** (seller). Positive = value to bank.
    pub price: f64,
    /// Standard error of the mean of the present value.
    pub std_error: f64,
    /// Expected (fractional) fixing index at which the structure redeems, or
    /// `fixings` if it never redeems on average.
    pub expected_redemption_fixing: f64,
    /// Expected realised gain overshoot beyond the target (gap exposure; identically
    /// zero under [`RedemptionStyle::CappedGain`]).
    pub expected_overshoot: f64,
}

/// Welford accumulator (mean + std-error of the mean). A self-contained copy per
/// leaf module, matching the `tarf.rs` / `accumulator.rs` pattern.
#[derive(Default)]
struct Welford {
    n: u64,
    mean: f64,
    m2: f64,
}

impl Welford {
    #[inline]
    fn push(&mut self, x: f64) {
        self.n += 1;
        let d = x - self.mean;
        self.mean += d / self.n as f64;
        self.m2 += d * (x - self.mean);
    }
    fn std_error(&self) -> f64 {
        if self.n < 2 {
            0.0
        } else {
            sqrt(self.m2 / ((self.n - 1) as f64) / self.n as f64)
        }
    }
}

/// Pre-computed per-run dynamics shared by every path (computed once, outside the
/// path loop — no `match` on carry in the hot path, ADR-0008).
struct PivotDynamics {
    ln_s0: f64,
    drift_step: f64,
    vol_sqrt_dt: f64,
    dfs: Vec<f64>,
}

impl PivotDynamics {
    fn new(i: &VanillaInputs, n: usize) -> Self {
        let dt = i.t / n as f64;
        let ln_s0 = ln(i.spot);
        let drift_step = (i.r_dom - i.r_for - 0.5 * i.vol * i.vol) * dt;
        let vol_sqrt_dt = i.vol * sqrt(dt);
        let mut dfs = vec![0.0f64; n];
        for (k, df) in dfs.iter_mut().enumerate() {
            let t_k = (k + 1) as f64 * dt;
            *df = exp(-i.r_dom * t_k);
        }
        Self {
            ln_s0,
            drift_step,
            vol_sqrt_dt,
            dfs,
        }
    }
}

/// Price a pivot TRA by Monte-Carlo with antithetic variates, returning the bank's
/// present value and the gap-risk diagnostics.
///
/// The reported `price` is the plain antithetic-pair mean (the control variate is a
/// separate entry point, [`pivot_tra_price_cv`]) so the `P = K` degenerate slice is
/// `to_bits`-identical to [`crate::tarf::tarf_price`]: the same RNG coordinates and
/// the same arithmetic order as `tarf.rs`.
#[must_use]
pub fn pivot_tra_price(i: &VanillaInputs, spec: PivotTra, cfg: PivotTraMcConfig) -> PivotTraResult {
    spec.validate();
    let n = spec.fixings;
    let dyn_ = PivotDynamics::new(i, n);

    let mut pv = Welford::default();
    let mut redemption_fixing = Welford::default();
    let mut overshoot = Welford::default();

    let mut z = vec![0.0f64; n];
    for pair in 0..cfg.pairs as u64 {
        let mut rng = CounterRng::new(cfg.seed, 0, pair, 0);
        for zk in z.iter_mut() {
            *zk = inverse_cdf(rng.next_u01());
        }
        let pa = walk_path(&dyn_, &z, spec, 1.0);
        let pb = walk_path(&dyn_, &z, spec, -1.0);
        pv.push(0.5 * (pa.bank_pv + pb.bank_pv));
        redemption_fixing.push(0.5 * (pa.redeem_index + pb.redeem_index));
        overshoot.push(0.5 * (pa.overshoot + pb.overshoot));
    }

    PivotTraResult {
        price: pv.mean,
        std_error: pv.std_error(),
        expected_redemption_fixing: redemption_fixing.mean,
        expected_overshoot: overshoot.mean,
    }
}

/// Price a pivot TRA with the forward-strip **control variate** in addition to the
/// antithetic variates — the variance-reduced estimate used for convergence and
/// Greeks tests.
///
/// The control is the same strip with the target disabled and no kink — a plain
/// forward strip `X = Σ_k g·(S_k − K)·e^{−r_d·t_k}` whose expectation is closed
/// form: `E[X] = Σ_k g·(S_0·e^{−r_for·t_k} − K·e^{−r_d·t_k})` (each leg a forward,
/// `F_k·e^{−r_d·t_k} = S_0·e^{−r_for·t_k}`). The optimal coefficient is the
/// regression `β = Cov(pivot_pv, X)/Var(X)` estimated from the same sample; the
/// guard `β = 0` when `Var(X) = 0` degrades gracefully to antithetic-only.
#[must_use]
pub fn pivot_tra_price_cv(
    i: &VanillaInputs,
    spec: PivotTra,
    cfg: PivotTraMcConfig,
) -> PivotTraResult {
    spec.validate();
    let n = spec.fixings;
    let dyn_ = PivotDynamics::new(i, n);
    let g = spec.favourable_side.sign();

    // Closed-form control mean E[X] = Σ_k g·(S_0·e^{−r_for·t_k} − K·e^{−r_d·t_k}).
    let dt = i.t / n as f64;
    let mut control_mean = 0.0f64;
    for k in 0..n {
        let t_k = (k + 1) as f64 * dt;
        let fwd_leg = i.spot * exp(-i.r_for * t_k);
        let strike_leg = spec.strike * dyn_.dfs[k];
        control_mean += g * (fwd_leg - strike_leg);
    }

    // First pass: collect paired (pivot_pv, X) antithetic-mean samples + online cov.
    let mut samples: Vec<(f64, f64)> = Vec::with_capacity(cfg.pairs);
    let mut control = Welford::default();
    let mut redemption_fixing = Welford::default();
    let mut overshoot = Welford::default();
    let mut cov = 0.0f64;
    let (mut mp, mut mx) = (0.0f64, 0.0f64);

    let mut z = vec![0.0f64; n];
    for pair in 0..cfg.pairs as u64 {
        let mut rng = CounterRng::new(cfg.seed, 0, pair, 0);
        for zk in z.iter_mut() {
            *zk = inverse_cdf(rng.next_u01());
        }
        let pa = walk_path(&dyn_, &z, spec, 1.0);
        let pb = walk_path(&dyn_, &z, spec, -1.0);
        let s_pv = 0.5 * (pa.bank_pv + pb.bank_pv);
        let s_x = 0.5 * (pa.control + pb.control);
        samples.push((s_pv, s_x));
        control.push(s_x);
        redemption_fixing.push(0.5 * (pa.redeem_index + pb.redeem_index));
        overshoot.push(0.5 * (pa.overshoot + pb.overshoot));

        let m = (pair + 1) as f64;
        let dp = s_pv - mp;
        let dx = s_x - mx;
        mp += dp / m;
        mx += dx / m;
        cov += dp * (s_x - mx);
    }

    let var_x = control.m2;
    let beta = if var_x > 0.0 { cov / var_x } else { 0.0 };

    // Second pass: Y = pivot_pv − β·(X − E[X]).
    let mut corrected = Welford::default();
    for (s_pv, s_x) in &samples {
        corrected.push(s_pv - beta * (s_x - control_mean));
    }

    PivotTraResult {
        price: corrected.mean,
        std_error: corrected.std_error(),
        expected_redemption_fixing: redemption_fixing.mean,
        expected_overshoot: overshoot.mean,
    }
}

/// Outcome of walking one pivot-TRA path.
struct PathOutcome {
    /// Discounted net cash flow **to the bank** (geared adverse legs received minus
    /// favourable legs paid away).
    bank_pv: f64,
    /// Fixing index (1-based, fractional via averaging) at which the path redeemed,
    /// or `fixings` if it survived to the last fixing.
    redeem_index: f64,
    /// Realised gain overshoot beyond the target at the redeeming fixing.
    overshoot: f64,
    /// The forward-strip control value `X = Σ_k g·(S_k − K)·e^{−r_d·t_k}` along the
    /// **full** strip (no target, no kink) — used only by [`pivot_tra_price_cv`].
    control: f64,
}

/// Walk one antithetic-signed pivot-TRA path.
fn walk_path(dyn_: &PivotDynamics, z: &[f64], spec: PivotTra, sign: f64) -> PathOutcome {
    let mut ln_s = dyn_.ln_s0;
    let mut accumulated_gain = 0.0f64;
    // Bank PV = +geared adverse legs (bank receives) − favourable legs (bank pays).
    let mut bank_pv = 0.0f64;
    let mut control = 0.0f64;
    let n = z.len();
    let g = spec.favourable_side.sign(); // +1 call-favourable, −1 put-favourable

    for k in 0..n {
        ln_s += dyn_.drift_step + dyn_.vol_sqrt_dt * sign * z[k];
        let s = exp(ln_s);

        // Full forward strip control leg, computed on every fixing regardless of
        // redemption (it is the un-knocked-out reference, closed-form expectation).
        let d_strike = g * (s - spec.strike); // favourable intrinsic > 0 when in gain
        control += d_strike * dyn_.dfs[k];

        let d_pivot = g * (s - spec.pivot); // > 0 on the favourable side of the pivot

        // Leg SELECTED by the pivot (kink), VALUED by the strike (intrinsic).
        // On the adverse side the gearing is folded into `c` with the SAME operand
        // order as `tarf.rs` (`leverage * (-signed)`), so the `P == K` slice is
        // bit-identical: there `d_pivot ≥ 0 ⇔ d_strike ≥ 0`, the favourable branch
        // gives `c = d_strike` and the adverse branch settles the bank
        // `(-c) = leverage·(-d_strike)`, matching `tarf.rs` to the bit.
        let c = if d_pivot >= 0.0 {
            d_strike
        } else {
            -spec.leverage * (-d_strike)
        };

        let raw_gain = c; // signed per-unit client cash flow
        if raw_gain > 0.0 {
            let remaining = spec.target - accumulated_gain;
            if raw_gain >= remaining {
                // Breach ⇒ redemption.
                let settled = match spec.redemption {
                    RedemptionStyle::FullGain => raw_gain,
                    RedemptionStyle::CappedGain => remaining,
                };
                bank_pv -= settled * spec.notional * dyn_.dfs[k];
                let overshoot = match spec.redemption {
                    RedemptionStyle::FullGain => (raw_gain - remaining).max(0.0),
                    RedemptionStyle::CappedGain => 0.0,
                };
                // Finish the control strip over the remaining fixings (the control
                // is the un-knocked-out forward strip — independent of redemption).
                for (j, &_zk) in z.iter().enumerate().skip(k + 1) {
                    ln_s += dyn_.drift_step + dyn_.vol_sqrt_dt * sign * z[j];
                    let sj = exp(ln_s);
                    control += g * (sj - spec.strike) * dyn_.dfs[j];
                }
                return PathOutcome {
                    bank_pv,
                    redeem_index: (k + 1) as f64,
                    overshoot,
                    control,
                };
            }
            bank_pv -= raw_gain * spec.notional * dyn_.dfs[k];
            accumulated_gain += raw_gain;
        } else if raw_gain < 0.0 {
            // Adverse leg: gearing already folded into `c`; bank receives `(-c)`.
            bank_pv += (-raw_gain) * spec.notional * dyn_.dfs[k];
        }
        // raw_gain == 0: no cash flow, no accrual.
    }

    PathOutcome {
        bank_pv,
        redeem_index: n as f64,
        overshoot: 0.0,
        control,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> VanillaInputs {
        VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01)
    }

    /// A dead-band call-favourable pivot TRA (`P > K`).
    fn dead_band(redemption: RedemptionStyle) -> PivotTra {
        PivotTra {
            strike: 1.28,
            pivot: 1.33,
            fixings: 12,
            target: 0.08,
            leverage: 2.0,
            favourable_side: OptionType::Call,
            notional: 1.0,
            redemption,
        }
    }

    /// Reproducibility: identical seed ⇒ bit-identical price and std-error.
    #[test]
    fn mc_is_reproducible() {
        let i = base();
        let cfg = PivotTraMcConfig {
            pairs: 30_000,
            seed: 0x7A4F,
        };
        let a = pivot_tra_price(&i, dead_band(RedemptionStyle::FullGain), cfg);
        let b = pivot_tra_price(&i, dead_band(RedemptionStyle::FullGain), cfg);
        assert_eq!(a.price.to_bits(), b.price.to_bits());
        assert_eq!(a.std_error.to_bits(), b.std_error.to_bits());
        // The control-variate path is reproducible too.
        let c = pivot_tra_price_cv(&i, dead_band(RedemptionStyle::FullGain), cfg);
        let d = pivot_tra_price_cv(&i, dead_band(RedemptionStyle::FullGain), cfg);
        assert_eq!(c.price.to_bits(), d.price.to_bits());
        assert_eq!(c.std_error.to_bits(), d.std_error.to_bits());
    }

    /// Target redemption shortens expected life: a tighter target redeems earlier.
    #[test]
    fn tighter_target_redeems_earlier() {
        let i = base();
        let cfg = PivotTraMcConfig {
            pairs: 120_000,
            seed: 0x7A11,
        };
        let tight = PivotTra {
            target: 0.03,
            ..dead_band(RedemptionStyle::FullGain)
        };
        let loose = PivotTra {
            target: 0.30,
            ..dead_band(RedemptionStyle::FullGain)
        };
        let t = pivot_tra_price(&i, tight, cfg);
        let l = pivot_tra_price(&i, loose, cfg);
        assert!(
            t.expected_redemption_fixing < l.expected_redemption_fixing,
            "tight target should redeem earlier: {} < {}",
            t.expected_redemption_fixing,
            l.expected_redemption_fixing
        );
    }

    /// Higher gearing on the adverse leg raises the bank's value.
    #[test]
    fn higher_leverage_raises_bank_value() {
        let i = base();
        let cfg = PivotTraMcConfig {
            pairs: 150_000,
            seed: 0x6EA2,
        };
        let g1 = PivotTra {
            leverage: 1.0,
            ..dead_band(RedemptionStyle::FullGain)
        };
        let g3 = PivotTra {
            leverage: 3.0,
            ..dead_band(RedemptionStyle::FullGain)
        };
        let v1 = pivot_tra_price(&i, g1, cfg);
        let v3 = pivot_tra_price(&i, g3, cfg);
        assert!(
            v3.price > v1.price,
            "more gearing should raise bank PV: {} > {}",
            v3.price,
            v1.price
        );
    }

    /// Gap risk: FullGain is more expensive to the bank than CappedGain, and only
    /// FullGain carries a positive expected overshoot.
    #[test]
    fn gap_risk_full_gain_costs_more_than_capped() {
        let i = base();
        let cfg = PivotTraMcConfig {
            pairs: 200_000,
            seed: 0x6A9,
        };
        let full = pivot_tra_price(&i, dead_band(RedemptionStyle::FullGain), cfg);
        let capped = pivot_tra_price(&i, dead_band(RedemptionStyle::CappedGain), cfg);
        let tol = 4.0 * (full.std_error + capped.std_error) + 1e-9;
        assert!(
            full.price < capped.price - 1e-4 + tol,
            "FullGain bank PV {} should be ≤ CappedGain {} (gap premium)",
            full.price,
            capped.price
        );
        assert!(
            full.expected_overshoot > 1e-4,
            "FullGain must carry positive expected overshoot, got {}",
            full.expected_overshoot
        );
        assert!(
            capped.expected_overshoot.abs() < 1e-12,
            "CappedGain must have zero overshoot, got {}",
            capped.expected_overshoot
        );
    }

    /// Unreachable target: never redeems (expected fixing == fixings), zero
    /// overshoot, and at `P = K` matches the `Tarf` geared-strip limit to MC noise.
    #[test]
    fn unreachable_target_never_redeems() {
        use crate::tarf::{Tarf, TarfMcConfig, tarf_price};
        let i = base();
        let cfg = PivotTraMcConfig {
            pairs: 80_000,
            seed: 0xBEEF,
        };
        let s = PivotTra {
            target: 1.0e6,
            ..dead_band(RedemptionStyle::FullGain)
        };
        let r = pivot_tra_price(&i, s, cfg);
        assert!(
            (r.expected_redemption_fixing - s.fixings as f64).abs() < 1e-9,
            "unreachable target should never redeem, got {}",
            r.expected_redemption_fixing
        );
        assert!(r.expected_overshoot.abs() < 1e-12);

        // At P = K with an unreachable target, the pivot TRA is a plain geared
        // forward strip — equals the Tarf same-limit price (bit-for-bit via the
        // identical RNG/arithmetic order, in fact, but assert to MC tolerance).
        let piv = PivotTra::as_tarf_slice(
            1.32,
            12,
            1.0e6,
            2.0,
            OptionType::Put,
            1.0,
            RedemptionStyle::FullGain,
        );
        let tarf = Tarf {
            strike: 1.32,
            fixings: 12,
            target: 1.0e6,
            leverage: 2.0,
            favourable_side: OptionType::Put,
            notional: 1.0,
            redemption: RedemptionStyle::FullGain,
        };
        let pcfg = PivotTraMcConfig {
            pairs: 80_000,
            seed: 0xBEEF,
        };
        let tcfg = TarfMcConfig {
            pairs: 80_000,
            seed: 0xBEEF,
        };
        let p = pivot_tra_price(&i, piv, pcfg);
        let t = tarf_price(&(&i).into(), tarf, tcfg);
        assert_eq!(
            p.price.to_bits(),
            t.price.to_bits(),
            "P=K geared strip must equal Tarf bit-for-bit: {} vs {}",
            p.price,
            t.price
        );
    }

    /// Convergence + variance reduction: the control-variate std-error shrinks ~1/√N
    /// and is strictly below the plain antithetic std-error at matched `pairs`. Uses
    /// a loose target where the forward-strip control is genuinely effective.
    #[test]
    fn control_variate_reduces_variance_and_converges() {
        let i = base();
        // Loose target so the structure rarely redeems and the linear forward-strip
        // exposure dominates the variance — the control is maximally effective.
        let spec = PivotTra {
            target: 0.50,
            ..dead_band(RedemptionStyle::FullGain)
        };
        let mut prev_se = f64::INFINITY;
        for &pairs in &[25_000usize, 100_000, 400_000] {
            let cfg = PivotTraMcConfig {
                pairs,
                seed: 0x5EED,
            };
            let plain = pivot_tra_price(&i, spec, cfg);
            let cv = pivot_tra_price_cv(&i, spec, cfg);
            // Control variate strictly reduces variance.
            assert!(
                cv.std_error < plain.std_error,
                "CV SE {} should beat plain SE {} at pairs={}",
                cv.std_error,
                plain.std_error,
                pairs
            );
            // Std-error shrinks with more paths (monotone decreasing).
            assert!(
                cv.std_error < prev_se,
                "CV SE should shrink with N: {} !< {} at pairs={}",
                cv.std_error,
                prev_se,
                pairs
            );
            prev_se = cv.std_error;
        }
        // ~1/√N: quadrupling pairs cuts the SE by roughly 2 (loose band).
        let se_lo = pivot_tra_price_cv(
            &i,
            spec,
            PivotTraMcConfig {
                pairs: 100_000,
                seed: 0x5EED,
            },
        )
        .std_error;
        let se_hi = pivot_tra_price_cv(
            &i,
            spec,
            PivotTraMcConfig {
                pairs: 400_000,
                seed: 0x5EED,
            },
        )
        .std_error;
        let ratio = se_lo / se_hi;
        assert!(
            (1.6..2.6).contains(&ratio),
            "4x paths should cut SE ~2x, got ratio {ratio}"
        );
    }
}
