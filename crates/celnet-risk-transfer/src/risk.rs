//! The moved-risk vector.
//!
//! Greeks and DV01 are **pass-through** sums from the input position slices,
//! scaled by the moved fraction — they are *not* re-priced here (this crate
//! carries no pricing engine). A slice that moves a fraction `f` of its notional
//! moves the same fraction `f` of the risk it carries, because greeks are linear
//! in notional.

/// A pass-through risk vector carried by a position slice.
///
/// Asset-agnostic: FX-vanilla desks populate the option greeks (`delta`,
/// `gamma`, `vega`, `theta`); linear-rates / bond desks populate `dv01`. Fields
/// left irrelevant for a given asset class are simply `0.0`. Values are already
/// notional-weighted (the risk the *whole* slice carries), so scaling by the
/// moved fraction yields the moved risk.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiskVector {
    /// Sensitivity to a 1bp parallel rate move (linear-rates / bond risk).
    pub dv01: f64,
    /// Sensitivity to spot (FX-vanilla delta).
    pub delta: f64,
    /// Sensitivity of delta to spot (FX-vanilla gamma).
    pub gamma: f64,
    /// Sensitivity to volatility (FX-vanilla vega).
    pub vega: f64,
    /// Sensitivity to the passage of time (FX-vanilla theta).
    pub theta: f64,
}

impl RiskVector {
    /// The additive identity — all sensitivities zero.
    pub const ZERO: RiskVector = RiskVector {
        dv01: 0.0,
        delta: 0.0,
        gamma: 0.0,
        vega: 0.0,
        theta: 0.0,
    };

    /// This vector scaled componentwise by `f` (the moved fraction of a slice).
    #[must_use]
    pub fn scaled(&self, f: f64) -> RiskVector {
        RiskVector {
            dv01: self.dv01 * f,
            delta: self.delta * f,
            gamma: self.gamma * f,
            vega: self.vega * f,
            theta: self.theta * f,
        }
    }

    /// Componentwise sum of two vectors.
    #[must_use]
    pub fn add(&self, other: &RiskVector) -> RiskVector {
        RiskVector {
            dv01: self.dv01 + other.dv01,
            delta: self.delta + other.delta,
            gamma: self.gamma + other.gamma,
            vega: self.vega + other.vega,
            theta: self.theta + other.theta,
        }
    }
}

/// The aggregate risk moved by a transfer — the signed base notional plus the
/// summed, fraction-scaled [`RiskVector`] of the moved slices.
///
/// Stamped on the audit record ([`crate::RiskTransferProvenance`]) so the
/// dashboard can show exactly what risk left the source and arrived in the
/// target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovedRisk {
    /// Signed base-currency notional moved (positive = long risk moved to the
    /// target; negative = short). Equals the target opening leg's signed
    /// notional and the negation of the source offsetting leg's.
    pub notional_base: f64,
    /// The summed, fraction-scaled pass-through risk vector of the moved slices.
    pub risk: RiskVector,
}
