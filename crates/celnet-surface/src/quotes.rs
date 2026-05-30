//! Broker market quotes and the per-tenor market context for one smile slice.
//!
//! An FX smile is quoted in **delta space**, not strike space
//! (`docs/ANALYTICS-SPEC.md` §3, §1.4): per `(pair, tenor)` the market gives an
//! at-the-money volatility plus **risk-reversals** and **butterflies** at one or
//! two delta pillars (the liquid `25Δ`, and `10Δ` for the most liquid pairs):
//!
//! ```text
//!   RR_d  = σ(dΔ call) − σ(dΔ put)                       (skew)
//!   BF_d  = ½·[σ(dΔ call) + σ(dΔ put)] − σ_ATM           (smile-butterfly / convexity)
//! ```
//!
//! These five numbers (`σ_ATM`, `RR_25/BF_25`, optionally `RR_10/BF_10`) define a
//! 5-point smile (ATM, 25dC/P, 10dC/P). Crucially the **butterfly the broker
//! actually trades is the market (broker) strangle**, *not* the arithmetic
//! smile-butterfly above — recovering the smile from the broker strangle needs an
//! explicit calibration ([`crate::strangle`]). This module holds the *inputs* and
//! the convention-aware machinery that turns a quoted delta pillar into a strike
//! via [`celnet_vanilla`]; the calibration and the smile model consume it.

use celnet_conventions::ConventionRecord;
use celnet_core::math::sqrt;
use celnet_types::{AtmConvention, DeltaConvention, OptionType, VanillaInputs};
use celnet_vanilla::{atm_strike, strike_from_delta};

/// A delta pillar at which a risk-reversal / butterfly pair is quoted.
///
/// FX desks quote the `25Δ` pillar universally and add the `10Δ` wing for the
/// most liquid pairs. The stored value is the *unsigned* delta magnitude in
/// `(0, 0.5)` (a call uses `+mag`, the matching put `−mag`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeltaPillar {
    /// Unsigned delta magnitude, e.g. `0.25` for the 25-delta pillar.
    pub magnitude: f64,
}

impl DeltaPillar {
    /// The canonical `25Δ` pillar.
    pub const TWENTY_FIVE: DeltaPillar = DeltaPillar { magnitude: 0.25 };
    /// The canonical `10Δ` wing pillar.
    pub const TEN: DeltaPillar = DeltaPillar { magnitude: 0.10 };

    /// Construct a pillar from an unsigned delta magnitude.
    ///
    /// # Panics
    ///
    /// Panics if `magnitude` is not strictly inside `(0, 0.5)` — a delta pillar
    /// outside that band is not a meaningful out-of-the-money wing.
    #[must_use]
    pub fn new(magnitude: f64) -> Self {
        assert!(
            magnitude > 0.0 && magnitude < 0.5,
            "delta pillar magnitude must lie in (0, 0.5); got {magnitude}"
        );
        Self { magnitude }
    }

    /// The signed delta target for `opt` at this pillar (`+mag` call, `−mag` put).
    #[must_use]
    pub fn signed(self, opt: OptionType) -> f64 {
        match opt {
            OptionType::Call => self.magnitude,
            OptionType::Put => -self.magnitude,
        }
    }
}

/// A risk-reversal / butterfly quote pair at a single delta pillar.
///
/// `risk_reversal = σ_call − σ_put` (skew, signed) and
/// `butterfly = ½(σ_call + σ_put) − σ_ATM` (convexity). The butterfly carried
/// here is the **market (broker) butterfly** quoted by the broker — the one used
/// to build the broker strangle that [`crate::strangle`] calibrates against — not
/// the arithmetic smile-butterfly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiskReversalButterfly {
    /// The delta pillar this pair is quoted at.
    pub pillar: DeltaPillar,
    /// Risk-reversal `σ_call − σ_put` (absolute vol units; `0.01` = 1 vol).
    pub risk_reversal: f64,
    /// Market (broker) butterfly `½(σ_call+σ_put) − σ_ATM` (absolute vol units).
    pub butterfly: f64,
}

/// The full set of broker market quotes for one `(pair, tenor)` smile slice.
///
/// Either a single (`25Δ`-only) pillar or two pillars (`25Δ` + `10Δ`). The ATM
/// volatility is quoted on the slice's [`AtmConvention`] (carried in the
/// resolved [`ConventionRecord`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketQuotes {
    /// At-the-money volatility (absolute), on the slice's ATM convention.
    pub atm_vol: f64,
    /// The inner (`25Δ`) risk-reversal / butterfly pair.
    pub inner: RiskReversalButterfly,
    /// The optional outer (`10Δ`) wing pair, present for liquid pairs.
    pub outer: Option<RiskReversalButterfly>,
}

impl MarketQuotes {
    /// A `25Δ`-only three-quote slice (ATM + one RR/BF pair).
    #[must_use]
    pub fn three_point(atm_vol: f64, rr_25: f64, bf_25: f64) -> Self {
        Self {
            atm_vol,
            inner: RiskReversalButterfly {
                pillar: DeltaPillar::TWENTY_FIVE,
                risk_reversal: rr_25,
                butterfly: bf_25,
            },
            outer: None,
        }
    }

    /// A full five-quote slice (ATM + `25Δ` + `10Δ` RR/BF pairs).
    #[must_use]
    pub fn five_point(atm_vol: f64, rr_25: f64, bf_25: f64, rr_10: f64, bf_10: f64) -> Self {
        Self {
            atm_vol,
            inner: RiskReversalButterfly {
                pillar: DeltaPillar::TWENTY_FIVE,
                risk_reversal: rr_25,
                butterfly: bf_25,
            },
            outer: Some(RiskReversalButterfly {
                pillar: DeltaPillar::TEN,
                risk_reversal: rr_10,
                butterfly: bf_10,
            }),
        }
    }

    /// Iterator over the quoted pillars, innermost first.
    pub fn pairs(&self) -> impl Iterator<Item = RiskReversalButterfly> + '_ {
        core::iter::once(self.inner).chain(self.outer)
    }
}

/// The convention-aware market context for one smile slice: the diffusion state
/// (spot, the two rates, vol-time) and the resolved FX conventions.
///
/// This is the single object the calibration ([`crate::strangle`]) and the smile
/// model ([`crate::vannavolga`]) consult to turn a delta pillar into a strike and
/// to price a benchmark option. It owns no quotes — those live in
/// [`MarketQuotes`] — only the *market state* and *conventions* that determine
/// the delta↔strike map.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketContext {
    /// Spot FX rate (quote per 1 unit of base).
    pub spot: f64,
    /// Continuously-compounded domestic (quote) rate.
    pub r_dom: f64,
    /// Continuously-compounded foreign (base) rate.
    pub r_for: f64,
    /// Vol-time to expiry in years.
    pub t: f64,
    /// Resolved FX conventions for this `(pair, tenor)` (delta + ATM styles).
    pub conventions: ConventionRecord,
}

impl MarketContext {
    /// Construct a context from market state and resolved conventions.
    #[must_use]
    pub fn new(spot: f64, r_dom: f64, r_for: f64, t: f64, conventions: ConventionRecord) -> Self {
        Self {
            spot,
            r_dom,
            r_for,
            t,
            conventions,
        }
    }

    /// The configured delta convention for this slice.
    #[must_use]
    pub fn delta_convention(&self) -> DeltaConvention {
        self.conventions.delta
    }

    /// The configured ATM convention for this slice.
    #[must_use]
    pub fn atm_convention(&self) -> AtmConvention {
        self.conventions.atm
    }

    /// Outright forward `F = S·e^{(r_dom−r_for)·t}`.
    #[must_use]
    pub fn forward(&self) -> f64 {
        self.template(self.spot, 0.0).forward()
    }

    /// Total ATM variance `σ_ATM²·t` — the natural calendar coordinate.
    #[must_use]
    pub fn atm_total_variance(&self, atm_vol: f64) -> f64 {
        atm_vol * atm_vol * self.t
    }

    /// A [`VanillaInputs`] template carrying this context's market state at a
    /// given `strike` and `vol`. The pricing/delta layers consume it directly.
    #[must_use]
    pub fn template(&self, strike: f64, vol: f64) -> VanillaInputs {
        VanillaInputs::new(self.spot, strike, vol, self.t, self.r_dom, self.r_for)
    }

    /// The ATM strike for the configured ATM/delta conventions at `atm_vol`.
    ///
    /// `K = F` for ATM-forward; the delta-neutral straddle strike (convention-
    /// signed) otherwise — delegated to [`celnet_vanilla::atm_strike`].
    #[must_use]
    pub fn atm_strike(&self, atm_vol: f64) -> f64 {
        atm_strike(
            self.atm_convention(),
            self.delta_convention(),
            self.forward(),
            atm_vol,
            self.t,
        )
    }

    /// The strike whose `opt` delta equals `pillar.signed(opt)` **at the smile
    /// vol `vol`** (the vol the surface assigns to that strike). The strike↔delta
    /// map is nonlinear and vol-dependent, so the caller supplies the trial vol
    /// (`docs/ANALYTICS-SPEC.md` §3.5); inversion is delegated to the
    /// convention-aware solver in [`celnet_vanilla`].
    ///
    /// # Errors
    ///
    /// Propagates [`celnet_vanilla::DeltaSolveError`] when the delta is
    /// unreachable in the convention or the solver fails to converge.
    pub fn strike_at_delta(
        &self,
        opt: OptionType,
        pillar: DeltaPillar,
        vol: f64,
    ) -> Result<f64, celnet_vanilla::DeltaSolveError> {
        let template = self.template(self.spot, vol);
        strike_from_delta(self.delta_convention(), opt, pillar.signed(opt), &template)
    }

    /// `σ√t` at the ATM vol — a handy scale for tolerances and bracketing.
    #[must_use]
    pub fn atm_std_dev(&self, atm_vol: f64) -> f64 {
        atm_vol * sqrt(self.t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_conventions::resolve;
    use celnet_core::assert_close;
    use celnet_types::{CcyPair, Tenor};

    fn eurusd_1y_ctx(spot: f64, atm: f64) -> MarketContext {
        let conv = resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record;
        let _ = atm;
        MarketContext::new(spot, 0.02, 0.01, 1.0, conv)
    }

    #[test]
    fn pillar_signs() {
        let p = DeltaPillar::TWENTY_FIVE;
        assert_close!(p.signed(OptionType::Call), 0.25);
        assert_close!(p.signed(OptionType::Put), -0.25);
    }

    #[test]
    #[should_panic(expected = "delta pillar magnitude")]
    fn pillar_rejects_out_of_band() {
        let _ = DeltaPillar::new(0.6);
    }

    #[test]
    fn quotes_pairs_iter() {
        let q = MarketQuotes::five_point(0.10, 0.01, 0.002, 0.018, 0.006);
        let pairs: Vec<_> = q.pairs().collect();
        assert_eq!(pairs.len(), 2);
        assert_close!(pairs[0].pillar.magnitude, 0.25);
        assert_close!(pairs[1].pillar.magnitude, 0.10);
        let three = MarketQuotes::three_point(0.10, 0.01, 0.002);
        assert_eq!(three.pairs().count(), 1);
    }

    /// The 25Δ call/put strikes (at the ATM vol) straddle the forward, and the
    /// 10Δ wings are further out — the basic FX-smile geometry.
    #[test]
    fn pillar_strikes_straddle_forward() {
        let ctx = eurusd_1y_ctx(1.10, 0.10);
        let f = ctx.forward();
        let kc25 = ctx
            .strike_at_delta(OptionType::Call, DeltaPillar::TWENTY_FIVE, 0.10)
            .unwrap();
        let kp25 = ctx
            .strike_at_delta(OptionType::Put, DeltaPillar::TWENTY_FIVE, 0.10)
            .unwrap();
        let kc10 = ctx
            .strike_at_delta(OptionType::Call, DeltaPillar::TEN, 0.10)
            .unwrap();
        let kp10 = ctx
            .strike_at_delta(OptionType::Put, DeltaPillar::TEN, 0.10)
            .unwrap();
        assert!(kp25 < f && f < kc25, "25Δ strikes straddle forward");
        assert!(kc10 > kc25, "10Δ call beyond 25Δ call");
        assert!(kp10 < kp25, "10Δ put beyond 25Δ put");
    }

    #[test]
    fn atm_strike_is_consistent_with_convention() {
        let ctx = eurusd_1y_ctx(1.10, 0.10);
        let k_atm = ctx.atm_strike(0.10);
        // Must be positive and near the forward for a 10-vol 1Y slice.
        assert!(k_atm > 0.0);
        let f = ctx.forward();
        assert!((k_atm / f - 1.0).abs() < 0.05);
    }
}
