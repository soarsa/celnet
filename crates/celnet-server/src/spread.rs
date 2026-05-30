//! The maker spread model: turn a mid price into a tradable two-way bid/offer.
//!
//! A request-for-quote returns a *two-way* market — a price the maker buys at
//! (bid) and a price it sells at (offer) — not a single mid. The half-spread the
//! maker charges around mid is the compensation for warehousing the risk until
//! the position can be hedged; it is wider for riskier (higher-vega, higher-gamma)
//! structures and never narrower than a configured floor.
//!
//! The model here is intentionally simple, deterministic, and explainable (a desk
//! must be able to justify every quoted spread): the half-spread is
//!
//! ```text
//!   half = max( floor , vega_charge·|vega| + gamma_charge·|gamma| )
//! ```
//!
//! expressed in the same premium units as `mid`, and the two-way is
//! `(mid − half, mid + half)`. The coefficients are vendor-neutral risk charges;
//! a production desk calibrates them per pair/tenor from its hedging cost, but the
//! shape — a risk-proportional charge with a floor — is the market-standard form.

use celnet_proto::TwoWayPrice;
use celnet_types::Greeks;

/// A deterministic maker spread model parameterised by per-Greek risk charges and
/// an absolute half-spread floor (all in the quote's premium units).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpreadModel {
    /// Minimum half-spread (premium units) charged on any quote.
    pub floor: f64,
    /// Premium charged per unit of absolute vega (vol risk warehoused).
    pub vega_charge: f64,
    /// Premium charged per unit of absolute gamma (spot-convexity risk).
    pub gamma_charge: f64,
}

impl Default for SpreadModel {
    /// A sensible default: a 1-bp-of-premium floor with mild risk charges. These
    /// produce tight but non-degenerate two-way markets for the EURUSD-scale
    /// fixtures the edge serves.
    fn default() -> Self {
        Self {
            floor: 1.0e-4,
            vega_charge: 1.0e-3,
            gamma_charge: 1.0e-4,
        }
    }
}

impl SpreadModel {
    /// The half-spread (premium units) this model charges for `greeks`.
    #[must_use]
    pub fn half_spread(&self, greeks: &Greeks) -> f64 {
        let risk = self.vega_charge * greeks.vega.abs() + self.gamma_charge * greeks.gamma.abs();
        self.floor.max(risk)
    }

    /// The two-way market around `mid` for a structure with the given `greeks`.
    ///
    /// `bid = mid − half`, `offer = mid + half`. The bid is floored at zero so a
    /// cheap structure never quotes a negative bid.
    #[must_use]
    pub fn two_way(&self, mid: f64, greeks: &Greeks) -> TwoWayPrice {
        let half = self.half_spread(greeks);
        TwoWayPrice {
            bid: (mid - half).max(0.0),
            offer: mid + half,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::is_close;

    fn greeks(vega: f64, gamma: f64) -> Greeks {
        Greeks {
            price: 0.0,
            delta_spot: 0.0,
            delta_forward: 0.0,
            gamma,
            vega,
            theta: 0.0,
            rho_dom: 0.0,
            rho_for: 0.0,
            vanna: 0.0,
            volga: 0.0,
            charm: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
        }
    }

    #[test]
    fn floor_applies_when_risk_is_tiny() {
        let m = SpreadModel::default();
        let g = greeks(0.0, 0.0);
        assert!(is_close(m.half_spread(&g), m.floor, 1e-15, 1e-15));
    }

    #[test]
    fn spread_widens_with_vega() {
        let m = SpreadModel::default();
        let lo = m.half_spread(&greeks(1.0, 0.0));
        let hi = m.half_spread(&greeks(100.0, 0.0));
        assert!(hi > lo, "more vega ⇒ wider spread: {lo} < {hi}");
    }

    #[test]
    fn two_way_brackets_mid() {
        let m = SpreadModel::default();
        let g = greeks(0.3, 2.0);
        let tw = m.two_way(0.012, &g);
        let half = m.half_spread(&g);
        assert!(is_close(tw.bid, 0.012 - half, 1e-15, 1e-15));
        assert!(is_close(tw.offer, 0.012 + half, 1e-15, 1e-15));
        assert!(tw.bid < tw.offer);
    }

    #[test]
    fn bid_never_negative() {
        let m = SpreadModel {
            floor: 0.05,
            ..SpreadModel::default()
        };
        let tw = m.two_way(0.01, &greeks(0.0, 0.0));
        assert!(tw.bid >= 0.0);
    }
}
