//! The risk pricer: turn a consolidated composite mid into the trader's tradable
//! two-way "risk price" — composite mid + **inventory/axe directional skew** +
//! a **size-tiered spread** with a warehousing risk charge.
//!
//! # Shape
//!
//! Given a reference `mid`, a requested `size`, and [`RiskPriceParams`]:
//!
//! ```text
//!   skewed_mid = mid · (1 + effective_skew_bp · 1e-4)      // directional lean
//!   half_bp    = floor_bp.max(spread_bp + risk_charge_bp + tier_extra_bp(size))
//!   half       = |mid| · half_bp · 1e-4                     // absolute half-spread
//!   bid        = max(0, skewed_mid − half)                  // never negative
//!   offer      = skewed_mid + half
//! ```
//!
//! The half-spread's `floor.max(charge)` form mirrors
//! `celnet_server::spread::SpreadModel::half_spread` (a risk-proportional charge
//! with a floor — the market-standard maker shape), re-expressed here in **basis
//! points of the cash mid** rather than option Greeks, and re-implemented rather
//! than imported because importing the server's model would invert the dependency
//! graph (the server depends on this aggregation core, never the reverse).
//!
//! Crucially the half-spread is scaled off the **unskewed** `|mid|`, so the skew
//! purely *translates* the two-way (both sides move by exactly the skew shift)
//! while the spread purely *widens* it — the two controls are orthogonal, which
//! is what makes "positive skew moves both sides up" strictly monotone regardless
//! of the spread.
//!
//! # Auto-skew seam
//!
//! Skew is trader-controlled today via [`RiskPriceParams::skew_bp`]. The
//! [`AutoSkewSource`] trait is the real (non-stub) extension point: the *effective*
//! skew is `params.skew_bp + auto.auto_skew_bp(instrument)`. The default wiring
//! ([`NoAutoSkew`]) contributes `0 bp` — the honest "no live position feed
//! attached" state, a complete and valid configuration, not a placeholder. A later
//! lane implements [`AutoSkewSource`] over the rates position store to lean the
//! axe automatically off live net inventory.

use crate::instrument::Instrument;

/// A source of automatic inventory/axe skew — the seam a later rates-position
/// store implements to lean the quote off live net inventory.
///
/// The returned value (basis points of mid) is **added** to the trader's manual
/// [`RiskPriceParams::skew_bp`]. Positive leans the two-way up (an axe to buy:
/// pay a touch more, sell a touch higher); negative leans it down.
pub trait AutoSkewSource {
    /// Additional inventory/axe skew for `instrument`, in bp of mid.
    fn auto_skew_bp(&self, instrument: &Instrument) -> f64;
}

/// The default auto-skew wiring: no live position feed attached, so it
/// contributes zero additional skew. A complete, valid state — the risk price is
/// then driven purely by the trader's manual [`RiskPriceParams::skew_bp`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoAutoSkew;

impl AutoSkewSource for NoAutoSkew {
    fn auto_skew_bp(&self, _instrument: &Instrument) -> f64 {
        0.0
    }
}

/// One rung of the size-tier ladder: a maximum size the rung covers and the extra
/// half-spread (bp) charged for a request that lands in it. Ascending
/// `max_size` with non-decreasing `extra_spread_bp` makes the ladder monotone —
/// a larger clip is never cheaper than a smaller one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizeTier {
    /// The largest request size (inclusive) this rung covers.
    pub max_size: f64,
    /// Extra half-spread charged for a request in this rung, in bp of mid.
    pub extra_spread_bp: f64,
}

/// The trader-controlled parameters of a risk price.
#[derive(Debug, Clone, PartialEq)]
pub struct RiskPriceParams {
    /// Manual inventory/axe directional skew, in bp of mid. Positive leans the
    /// whole two-way up; negative leans it down.
    pub skew_bp: f64,
    /// Base half-spread charged around the mid, in bp of mid.
    pub spread_bp: f64,
    /// Absolute half-spread floor in bp of mid — the minimum charge on any tier
    /// (mirrors `SpreadModel::floor`).
    pub floor_bp: f64,
    /// Warehousing / inventory risk charge added to the half-spread, in bp of mid
    /// (the cash-book analogue of the vega/gamma risk charge in
    /// `SpreadModel::half_spread`).
    pub risk_charge_bp: f64,
    /// The size-tier ladder, ascending in `max_size` (and non-decreasing in
    /// `extra_spread_bp`). Empty ⇒ no size-tiering (zero tier extra everywhere).
    pub size_tiers: Vec<SizeTier>,
}

impl RiskPriceParams {
    /// A flat risk price: only a base spread, no skew, no floor, no risk charge,
    /// no tiers. Callers set the fields they want.
    #[must_use]
    pub fn flat(spread_bp: f64) -> Self {
        Self {
            skew_bp: 0.0,
            spread_bp,
            floor_bp: 0.0,
            risk_charge_bp: 0.0,
            size_tiers: Vec::new(),
        }
    }

    /// The extra half-spread (bp) charged for `size`: the first ascending tier
    /// whose `max_size` covers `size`, or the last (widest) tier's extra when
    /// `size` exceeds every rung (a request beyond the top rung is charged the
    /// widest rung, never a cheaper one). `0` when there are no tiers.
    #[must_use]
    pub fn tier_extra_bp(&self, size: f64) -> f64 {
        if self.size_tiers.is_empty() {
            return 0.0;
        }
        for tier in &self.size_tiers {
            if size <= tier.max_size {
                return tier.extra_spread_bp;
            }
        }
        // Beyond every rung → the widest rung's extra.
        self.size_tiers.last().map_or(0.0, |t| t.extra_spread_bp)
    }
}

/// The two-way risk price produced by [`RiskPricer::two_way`], plus the
/// decomposition a trader/desk needs to justify it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiskTwoWay {
    /// The bid the desk shows (never negative).
    pub bid: f64,
    /// The offer the desk shows.
    pub offer: f64,
    /// The skewed mid the two-way is centred on.
    pub skewed_mid: f64,
    /// The effective skew applied (manual + auto), in bp of mid.
    pub effective_skew_bp: f64,
    /// The half-spread charged, in bp of mid.
    pub half_spread_bp: f64,
}

/// The directional-skew risk pricer. Holds the trader-controlled
/// [`RiskPriceParams`] and an [`AutoSkewSource`] (default [`NoAutoSkew`]); prices
/// a two-way off a consolidated composite mid.
#[derive(Debug, Clone)]
pub struct RiskPricer<S = NoAutoSkew> {
    params: RiskPriceParams,
    auto_skew: S,
}

impl RiskPricer<NoAutoSkew> {
    /// A pricer driven purely by the trader's manual params (no live position
    /// feed). This is the complete, sanctioned path today.
    #[must_use]
    pub fn new(params: RiskPriceParams) -> Self {
        Self {
            params,
            auto_skew: NoAutoSkew,
        }
    }
}

impl<S: AutoSkewSource> RiskPricer<S> {
    /// A pricer whose skew is the trader's manual `skew_bp` plus a live auto-skew
    /// read from `auto_skew` (the rates-position seam).
    #[must_use]
    pub fn with_auto_skew(params: RiskPriceParams, auto_skew: S) -> Self {
        Self { params, auto_skew }
    }

    /// The trader-controlled parameters backing this pricer.
    #[must_use]
    pub fn params(&self) -> &RiskPriceParams {
        &self.params
    }

    /// The effective skew (bp of mid) for `instrument`: manual + auto.
    #[must_use]
    pub fn effective_skew_bp(&self, instrument: &Instrument) -> f64 {
        self.params.skew_bp + self.auto_skew.auto_skew_bp(instrument)
    }

    /// The half-spread (bp of mid) charged for `size`:
    /// `floor_bp.max(spread_bp + risk_charge_bp + tier_extra_bp(size))`.
    #[must_use]
    pub fn half_spread_bp(&self, size: f64) -> f64 {
        let charge =
            self.params.spread_bp + self.params.risk_charge_bp + self.params.tier_extra_bp(size);
        self.params.floor_bp.max(charge)
    }

    /// Price the trader's two-way for `instrument` off the consolidated `mid` for
    /// a request of `size`.
    ///
    /// The skew translates the mid; the size-tiered, risk-charged, floored
    /// half-spread (scaled off the **unskewed** `|mid|`) brackets it. The bid is
    /// floored at zero so a cheap instrument never shows a negative bid (mirrors
    /// `SpreadModel::two_way`).
    #[must_use]
    pub fn two_way(&self, instrument: &Instrument, mid: f64, size: f64) -> RiskTwoWay {
        let effective_skew_bp = self.effective_skew_bp(instrument);
        let skewed_mid = mid * (1.0 + effective_skew_bp * 1.0e-4);
        let half_spread_bp = self.half_spread_bp(size);
        let half = mid.abs() * half_spread_bp * 1.0e-4;
        RiskTwoWay {
            bid: (skewed_mid - half).max(0.0),
            offer: skewed_mid + half,
            skewed_mid,
            effective_skew_bp,
            half_spread_bp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_types::{Ccy, CcyPair, Tenor};

    fn instr() -> Instrument {
        Instrument::new(CcyPair::new(Ccy::EUR, Ccy::USD), Tenor::Months(3))
    }

    fn tiers() -> Vec<SizeTier> {
        vec![
            SizeTier {
                max_size: 1_000_000.0,
                extra_spread_bp: 0.0,
            },
            SizeTier {
                max_size: 10_000_000.0,
                extra_spread_bp: 1.0,
            },
            SizeTier {
                max_size: 100_000_000.0,
                extra_spread_bp: 3.0,
            },
        ]
    }

    fn params() -> RiskPriceParams {
        RiskPriceParams {
            skew_bp: 0.0,
            spread_bp: 2.0,
            floor_bp: 0.5,
            risk_charge_bp: 0.0,
            size_tiers: tiers(),
        }
    }

    #[test]
    fn bid_never_negative() {
        // Cheap instrument, huge spread ⇒ raw bid would go negative; must floor.
        let p = RiskPriceParams {
            skew_bp: 0.0,
            spread_bp: 20_000.0, // 200% half-spread
            floor_bp: 0.0,
            risk_charge_bp: 0.0,
            size_tiers: Vec::new(),
        };
        let tw = RiskPricer::new(p).two_way(&instr(), 0.01, 1_000_000.0);
        assert!(tw.bid >= 0.0, "bid {} must be floored at zero", tw.bid);
    }

    #[test]
    fn positive_skew_moves_both_sides_up_monotonically() {
        let base = params();
        let mid = 1.10;
        let size = 1_000_000.0;
        let mut last_bid = f64::NEG_INFINITY;
        let mut last_offer = f64::NEG_INFINITY;
        for skew_bp in [-10.0, -5.0, 0.0, 5.0, 10.0, 25.0] {
            let p = RiskPriceParams {
                skew_bp,
                ..base.clone()
            };
            let tw = RiskPricer::new(p).two_way(&instr(), mid, size);
            assert!(
                tw.bid > last_bid,
                "bid must increase with skew ({} !> {})",
                tw.bid,
                last_bid
            );
            assert!(
                tw.offer > last_offer,
                "offer must increase with skew ({} !> {})",
                tw.offer,
                last_offer
            );
            last_bid = tw.bid;
            last_offer = tw.offer;
        }
    }

    #[test]
    fn spread_widens_with_size_tier() {
        let pricer = RiskPricer::new(params());
        let mid = 1.10;
        let small = pricer.two_way(&instr(), mid, 500_000.0);
        let mid_size = pricer.two_way(&instr(), mid, 5_000_000.0);
        let large = pricer.two_way(&instr(), mid, 50_000_000.0);
        let w = |t: &RiskTwoWay| t.offer - t.bid;
        assert!(w(&mid_size) > w(&small), "tier 2 wider than tier 1");
        assert!(w(&large) > w(&mid_size), "tier 3 wider than tier 2");
    }

    #[test]
    fn spread_widens_with_risk_charge() {
        let mid = 1.10;
        let size = 1_000_000.0;
        let base = RiskPricer::new(params()).two_way(&instr(), mid, size);
        let charged = RiskPricer::new(RiskPriceParams {
            risk_charge_bp: 4.0,
            ..params()
        })
        .two_way(&instr(), mid, size);
        assert!(
            (charged.offer - charged.bid) > (base.offer - base.bid),
            "a risk charge must widen the two-way"
        );
    }

    #[test]
    fn size_tier_laddering_is_monotone() {
        let p = params();
        let mut last = f64::NEG_INFINITY;
        for size in [
            500_000.0,
            1_000_000.0,
            2_000_000.0,
            10_000_000.0,
            50_000_000.0,
            500_000_000.0, // beyond top rung ⇒ charged the widest rung
        ] {
            let extra = p.tier_extra_bp(size);
            assert!(
                extra >= last,
                "tier extra must be non-decreasing in size ({extra} < {last})"
            );
            last = extra;
        }
    }

    #[test]
    fn zero_skew_zero_spread_collapses_to_mid() {
        let p = RiskPriceParams {
            skew_bp: 0.0,
            spread_bp: 0.0,
            floor_bp: 0.0,
            risk_charge_bp: 0.0,
            size_tiers: Vec::new(),
        };
        let mid = 1.2345;
        let tw = RiskPricer::new(p).two_way(&instr(), mid, 1_000_000.0);
        assert_eq!(tw.bid.to_bits(), mid.to_bits());
        assert_eq!(tw.offer.to_bits(), mid.to_bits());
        assert_eq!(tw.skewed_mid.to_bits(), mid.to_bits());
    }

    #[test]
    fn floor_binds_when_charge_is_below_it() {
        let p = RiskPriceParams {
            skew_bp: 0.0,
            spread_bp: 0.1,
            floor_bp: 2.0,
            risk_charge_bp: 0.0,
            size_tiers: Vec::new(),
        };
        let pricer = RiskPricer::new(p);
        assert_eq!(pricer.half_spread_bp(1.0), 2.0, "floor must bind");
    }

    #[test]
    fn auto_skew_source_adds_to_manual_skew() {
        struct FixedAxe(f64);
        impl AutoSkewSource for FixedAxe {
            fn auto_skew_bp(&self, _instrument: &Instrument) -> f64 {
                self.0
            }
        }
        let p = RiskPriceParams {
            skew_bp: 3.0,
            ..params()
        };
        let pricer = RiskPricer::with_auto_skew(p, FixedAxe(4.0));
        // Effective skew is manual (3) + auto (4) = 7 bp.
        assert_eq!(pricer.effective_skew_bp(&instr()), 7.0);
        let tw = pricer.two_way(&instr(), 1.10, 1_000_000.0);
        assert_eq!(tw.effective_skew_bp, 7.0);
        // Sanity: the skewed mid leans up by 7 bp.
        assert!(tw.skewed_mid > 1.10);
    }
}
