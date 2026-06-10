//! Target-Redemption Forward (TARF) — a strip of periodic fixings with a
//! cumulative target that **knocks the structure out** once reached, and
//! leverage/gearing on the adverse side.
//!
//! # Structure
//!
//! On each of `n` equally-spaced fixing dates the realised spot `S_k` is compared
//! to the strike `K`. From the client's point of view (a typical exporter selling
//! the base currency forward at an enhanced rate):
//!
//! * **Favourable fixing** (`S_k` on the in-the-money side of `K`) accrues a
//!   per-fixing *gain* `g_k = |S_k − K|` (times the per-fixing notional). The
//!   running sum of gains accumulates toward a **target**.
//! * **Adverse fixing** (`S_k` on the wrong side of `K`) costs the client
//!   `leverage · |S_k − K|` — the **gearing** that funds the enhanced strike.
//! * Once the accumulated gain **reaches the target**, the structure **redeems**
//!   (knocks out): no further fixings settle. The fixing that breaches the target
//!   is the *gap-risk* event — depending on the settlement style it pays either the
//!   full last gain (potentially overshooting the target) or only the gain capped
//!   at the remaining target.
//!
//! The product's value to the *option seller* (the bank) is what
//! [`tarf_price`] returns: the discounted expected net of the geared adverse legs
//! minus the favourable legs paid away, i.e. the **bank's** present value. A
//! positive number is value to the bank.
//!
//! # Gap risk
//!
//! The defining risk of a TARF is the **gap** at redemption: the last favourable
//! fixing can carry the client far past the target in one jump, so the realised
//! final gain overshoots. Two market conventions are modelled by
//! [`RedemptionStyle`]:
//! * [`RedemptionStyle::FullGain`] — the breaching fixing pays its full intrinsic
//!   (the client keeps the overshoot); this is the genuine gap exposure.
//! * [`RedemptionStyle::CappedGain`] — the breaching fixing pays only up to the
//!   remaining target (exact redemption); no overshoot.
//!
//! The spread between the two is the explicit gap-risk premium, asserted in the
//! tests.
//!
//! # Numerics
//!
//! Priced on the Monte-Carlo engine ([`crate::rng::CounterRng`] +
//! [`crate::normal::inverse_cdf`]) with antithetic variates. The terminal law of
//! the GBM at each fixing is exact (no time-step bias between fixings), so the only
//! Monte-Carlo error is statistical. Reproducible from the seed.
//!
//! Provenance (doc-only): the TARF payoff and its gap-risk decomposition are
//! described in Wystup (2017), *FX Options and Structured Products*, and in
//! Caspers (2014). Identifiers are purpose-named and vendor/research-neutral.

use celnet_core::math::{exp, ln, sqrt};
use celnet_types::OptionType;

use crate::inputs::ExoticInputs;
use crate::normal::inverse_cdf;
use crate::rng::CounterRng;

/// How the redeeming (target-breaching) fixing settles — the gap-risk convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RedemptionStyle {
    /// The breaching fixing pays its **full** intrinsic gain (the accumulated gain
    /// may overshoot the target). This carries the genuine gap exposure.
    FullGain,
    /// The breaching fixing pays only the **remaining** target (exact redemption,
    /// no overshoot).
    CappedGain,
}

/// A Target-Redemption Forward specification.
///
/// `favourable_side` is the side of the strike on which the client *gains*: a
/// [`OptionType::Put`] means the client gains when `S_k < K` (an exporter selling
/// the base at the enhanced strike — the classic TARF), a [`OptionType::Call`]
/// means the client gains when `S_k > K`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tarf {
    /// Strike `K` of every fixing.
    pub strike: f64,
    /// Number of equally-spaced fixing dates over `[0, T]` (the last at `T`).
    pub fixings: usize,
    /// Cumulative gain target. Accumulated client gain at or above this redeems.
    pub target: f64,
    /// Gearing/leverage multiplier on the adverse (loss) leg (`≥ 1` typically).
    pub leverage: f64,
    /// The side on which the client accrues gains.
    pub favourable_side: OptionType,
    /// Per-fixing notional (units of base currency per fixing).
    pub notional: f64,
    /// Gap-risk settlement convention of the redeeming fixing.
    pub redemption: RedemptionStyle,
}

impl Tarf {
    /// Validate the basic structural invariants.
    fn validate(&self) {
        assert!(self.fixings >= 1, "TARF needs ≥1 fixing");
        assert!(self.target > 0.0, "TARF target must be positive");
        assert!(self.leverage >= 0.0, "TARF leverage must be non-negative");
        assert!(self.notional > 0.0, "TARF notional must be positive");
    }
}

/// Welford accumulator (mean + std-error of the mean).
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

/// Monte-Carlo configuration for the TARF engine.
#[derive(Debug, Clone, Copy)]
pub struct TarfMcConfig {
    /// Number of antithetic path **pairs**.
    pub pairs: usize,
    /// Seed for the counter-based RNG.
    pub seed: u64,
}

/// The result of a TARF Monte-Carlo run: the bank's present value plus the
/// standard error, the expected redemption fixing index, and the expected
/// realised overshoot past the target (the gap exposure).
#[derive(Debug, Clone, Copy)]
pub struct TarfResult {
    /// Discounted present value to the **bank** (seller). Positive = value to bank.
    pub price: f64,
    /// Standard error of the mean of the present value.
    pub std_error: f64,
    /// Expected (fractional) fixing index at which the structure redeems, or
    /// `fixings` if it never redeems on average — a measure of expected life.
    pub expected_redemption_fixing: f64,
    /// Expected realised gain **overshoot** beyond the target at redemption (the
    /// gap exposure; identically zero under [`RedemptionStyle::CappedGain`]).
    pub expected_overshoot: f64,
}

/// Price a TARF by Monte-Carlo with antithetic variates, returning the bank's
/// present value and the gap-risk diagnostics.
///
/// Each path simulates the `fixings` spots from the exact GBM terminal law per
/// fixing date, walks the strip applying the favourable-gain / geared-loss legs,
/// and stops when the cumulative gain reaches the target (redemption). All cash
/// flows are discounted at the domestic rate to their own fixing date.
#[must_use]
pub fn tarf_price(i: &ExoticInputs, spec: Tarf, cfg: TarfMcConfig) -> TarfResult {
    spec.validate();
    let n = spec.fixings;
    let dt = i.t / n as f64;
    let ln_s0 = ln(i.spot);
    // Carry accessors read ONCE, outside the path loop (no `Carry` dispatch in the
    // hot path, ADR-0008); byte-identical to the FX two-rate form for
    // `Carry::FxRates`.
    let drift_step = (i.carry_rate() - 0.5 * i.vol * i.vol) * dt;
    let vol_sqrt_dt = i.vol * sqrt(dt);

    // Per-fixing numeraire discount factors e^{-r · t_k}, t_k = (k+1)·dt.
    let mut dfs = vec![0.0f64; n];
    for (k, df) in dfs.iter_mut().enumerate() {
        let t_k = (k + 1) as f64 * dt;
        *df = i.discount_df_at(t_k);
    }

    let mut pv = Welford::default();
    let mut redemption_fixing = Welford::default();
    let mut overshoot = Welford::default();

    let mut z = vec![0.0f64; n];
    for pair in 0..cfg.pairs as u64 {
        let mut rng = CounterRng::new(cfg.seed, 0, pair, 0);
        for zk in z.iter_mut() {
            *zk = inverse_cdf(rng.next_u01());
        }
        let pa = walk_path(ln_s0, drift_step, vol_sqrt_dt, &z, &dfs, spec, 1.0);
        let pb = walk_path(ln_s0, drift_step, vol_sqrt_dt, &z, &dfs, spec, -1.0);
        pv.push(0.5 * (pa.bank_pv + pb.bank_pv));
        redemption_fixing.push(0.5 * (pa.redeem_index + pb.redeem_index));
        overshoot.push(0.5 * (pa.overshoot + pb.overshoot));
    }

    TarfResult {
        price: pv.mean,
        std_error: pv.std_error(),
        expected_redemption_fixing: redemption_fixing.mean,
        expected_overshoot: overshoot.mean,
    }
}

/// Outcome of walking one TARF path.
struct PathOutcome {
    /// Discounted net cash flow **to the bank** (geared losses received minus
    /// favourable gains paid away).
    bank_pv: f64,
    /// Fixing index (1-based, fractional via averaging) at which the path redeemed,
    /// or `fixings` if it survived to the last fixing.
    redeem_index: f64,
    /// Realised gain overshoot beyond the target at the redeeming fixing.
    overshoot: f64,
}

/// Walk one antithetic-signed TARF path.
fn walk_path(
    ln_s0: f64,
    drift_step: f64,
    vol_sqrt_dt: f64,
    z: &[f64],
    dfs: &[f64],
    spec: Tarf,
    sign: f64,
) -> PathOutcome {
    let mut ln_s = ln_s0;
    let mut accumulated_gain = 0.0f64;
    // Bank PV = +geared adverse legs (bank receives) − favourable legs (bank pays).
    let mut bank_pv = 0.0f64;
    let n = z.len();
    let gain_sign = spec.favourable_side.sign(); // +1 gain when S>K (call), −1 when S<K (put)

    for k in 0..n {
        ln_s += drift_step + vol_sqrt_dt * sign * z[k];
        let s_k = exp(ln_s);
        let signed = gain_sign * (s_k - spec.strike);
        if signed > 0.0 {
            // Favourable fixing: client gains `signed` per unit; bank pays it.
            let remaining = spec.target - accumulated_gain;
            let raw_gain = signed; // per-unit gain before notional
            if raw_gain >= remaining {
                // This fixing breaches the target ⇒ redemption.
                let settled_per_unit = match spec.redemption {
                    RedemptionStyle::FullGain => raw_gain,
                    RedemptionStyle::CappedGain => remaining,
                };
                bank_pv -= settled_per_unit * spec.notional * dfs[k];
                let overshoot = match spec.redemption {
                    RedemptionStyle::FullGain => (raw_gain - remaining).max(0.0),
                    RedemptionStyle::CappedGain => 0.0,
                };
                return PathOutcome {
                    bank_pv,
                    redeem_index: (k + 1) as f64,
                    overshoot,
                };
            }
            // Below target: full favourable leg settles, accumulate.
            bank_pv -= raw_gain * spec.notional * dfs[k];
            accumulated_gain += raw_gain;
        } else if signed < 0.0 {
            // Adverse fixing: client loses `leverage·|signed|`; bank receives it.
            bank_pv += spec.leverage * (-signed) * spec.notional * dfs[k];
        }
        // signed == 0: at-the-money fixing, no cash flow, no accrual.
    }

    PathOutcome {
        bank_pv,
        redeem_index: n as f64,
        overshoot: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A typical exporter TARF: sell EUR at an enhanced strike above forward,
    /// client gains when spot falls below strike (put side), geared 2x on the
    /// downside for the bank.
    fn base() -> ExoticInputs {
        celnet_types::VanillaInputs::new(1.30, 1.30, 0.10, 1.0, 0.03, 0.01).into()
    }

    fn spec(redemption: RedemptionStyle) -> Tarf {
        Tarf {
            strike: 1.32,
            fixings: 12,
            target: 0.06,
            leverage: 2.0,
            favourable_side: OptionType::Put,
            notional: 1.0,
            redemption,
        }
    }

    /// Reproducibility: identical seed ⇒ bit-identical price.
    #[test]
    fn mc_is_reproducible() {
        let i = base();
        let cfg = TarfMcConfig {
            pairs: 30_000,
            seed: 0x7A4F,
        };
        let a = tarf_price(&i, spec(RedemptionStyle::FullGain), cfg);
        let b = tarf_price(&i, spec(RedemptionStyle::FullGain), cfg);
        assert_eq!(a.price.to_bits(), b.price.to_bits());
        assert_eq!(a.std_error.to_bits(), b.std_error.to_bits());
    }

    /// Gap-risk stress: the FullGain settlement (client keeps the overshoot) must
    /// be **more expensive to the bank** than the CappedGain settlement (exact
    /// redemption). The spread is the explicit gap-risk premium, and FullGain must
    /// carry a strictly positive expected overshoot while CappedGain has none.
    #[test]
    fn gap_risk_full_gain_costs_more_than_capped() {
        let i = base();
        let cfg = TarfMcConfig {
            pairs: 200_000,
            seed: 0x6A9,
        };
        let full = tarf_price(&i, spec(RedemptionStyle::FullGain), cfg);
        let capped = tarf_price(&i, spec(RedemptionStyle::CappedGain), cfg);

        // FullGain pays the client more on the breaching fixing ⇒ lower bank PV.
        let tol = 4.0 * (full.std_error + capped.std_error) + 1e-9;
        assert!(
            full.price < capped.price - 1e-4 + tol,
            "FullGain bank PV {} should be ≤ CappedGain {} (gap premium)",
            full.price,
            capped.price
        );
        // The realised overshoot is the gap exposure: present under FullGain only.
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

    /// Target redemption shortens expected life: a *tight* target redeems earlier
    /// (smaller expected redemption fixing index) than a *loose* target. This pins
    /// that the knockout-on-target mechanic is actually wired.
    #[test]
    fn tighter_target_redeems_earlier() {
        let i = base();
        let cfg = TarfMcConfig {
            pairs: 120_000,
            seed: 0x7A11,
        };
        let tight = Tarf {
            target: 0.02,
            ..spec(RedemptionStyle::FullGain)
        };
        let loose = Tarf {
            target: 0.20,
            ..spec(RedemptionStyle::FullGain)
        };
        let t = tarf_price(&i, tight, cfg);
        let l = tarf_price(&i, loose, cfg);
        assert!(
            t.expected_redemption_fixing < l.expected_redemption_fixing,
            "tight target should redeem earlier: {} < {}",
            t.expected_redemption_fixing,
            l.expected_redemption_fixing
        );
    }

    /// Higher gearing raises the bank's value: the leverage multiplier scales the
    /// adverse leg the bank receives, so the structure is worth more to the bank.
    #[test]
    fn higher_leverage_raises_bank_value() {
        let i = base();
        let cfg = TarfMcConfig {
            pairs: 150_000,
            seed: 0x6EA2,
        };
        let g1 = Tarf {
            leverage: 1.0,
            ..spec(RedemptionStyle::FullGain)
        };
        let g3 = Tarf {
            leverage: 3.0,
            ..spec(RedemptionStyle::FullGain)
        };
        let v1 = tarf_price(&i, g1, cfg);
        let v3 = tarf_price(&i, g3, cfg);
        assert!(
            v3.price > v1.price,
            "more gearing should raise bank PV: {} > {}",
            v3.price,
            v1.price
        );
    }

    /// A target so large it is never reached degenerates to a plain geared forward
    /// strip with no redemption: the expected redemption fixing equals the fixing
    /// count and the overshoot is zero. Sanity-checks the no-knockout limit.
    #[test]
    fn unreachable_target_never_redeems() {
        let i = base();
        let cfg = TarfMcConfig {
            pairs: 80_000,
            seed: 0xBEEF,
        };
        let s = Tarf {
            target: 1.0e6,
            ..spec(RedemptionStyle::FullGain)
        };
        let r = tarf_price(&i, s, cfg);
        assert!(
            (r.expected_redemption_fixing - s.fixings as f64).abs() < 1e-9,
            "unreachable target should never redeem, got {}",
            r.expected_redemption_fixing
        );
        assert!(r.expected_overshoot.abs() < 1e-12);
    }
}
