//! Vendor wire model — the decoded shape of an external FX-options market-data
//! message, before it is mapped onto Celnet's canonical vocabulary.
//!
//! External FX-options feeds (the FMD FXO 2.0-style shapes referenced in
//! `docs/CELNET-INTEGRATION.md` §1) publish, per `(pair, tenor)`, a *delta-space*
//! smile: an at-the-money volatility plus risk-reversal and butterfly wings at
//! the `25Δ` and (for liquid pairs) `10Δ` pillars, together with the spot mid,
//! forward points (or an outright forward), and — for non-deliverable pairs —
//! an NDF cash-settlement fixing. The wire numbers are quoted in **percentage
//! points** (a `10.5` ATM means a `0.105` absolute vol; a `0.25` RR means
//! `0.0025`) and the feed carries its own **convention descriptor** (delta style,
//! ATM style, premium currency, cut) which Celnet must honour on ingest rather
//! than assume — convention error dwarfs model error
//! (`docs/ANALYTICS-SPEC.md` §1.1, open question 5 in `docs/CELNET-INTEGRATION.md`).
//!
//! This module defines *only the decoded wire shapes* and their `serde`
//! (de)serialization. It does no numerics and applies no conventions; the
//! mapping to [`celnet_surface::MarketQuotes`] / [`celnet_surface::MarketContext`]
//! lives in [`crate::normalize`]. Keeping the wire model separate means a new
//! feed format is a new decoder, never a change to the canonical pipeline.
//!
//! The public Celnet types stay vendor-neutral: this is a *data-shape adapter*
//! named for its purpose (a vendor option-smile message), carrying no vendor,
//! competitor or product name in any identifier.

use serde::{Deserialize, Serialize};

/// How an external feed expresses its quoted delta.
///
/// Mirrors the four [`celnet_types::DeltaConvention`] variants but is a *wire*
/// enum (string-tagged for human-readable JSON feeds) so a feed's self-declared
/// convention round-trips without coupling the wire format to the internal enum
/// layout. [`Self::canonical`] maps to the canonical type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WireDeltaConvention {
    /// Spot delta, premium-unadjusted (`e^{-r_f T} N(d1)`).
    SpotUnadjusted,
    /// Forward (driftless) delta, premium-unadjusted (`N(d1)`).
    ForwardUnadjusted,
    /// Spot delta, premium-adjusted (`e^{-r_f T} (K/F) N(d2)`).
    SpotPremiumAdjusted,
    /// Forward delta, premium-adjusted (`(K/F) N(d2)`).
    ForwardPremiumAdjusted,
}

impl WireDeltaConvention {
    /// The canonical [`celnet_types::DeltaConvention`] this wire variant denotes.
    #[must_use]
    pub const fn canonical(self) -> celnet_types::DeltaConvention {
        use celnet_types::DeltaConvention as D;
        match self {
            WireDeltaConvention::SpotUnadjusted => D::SpotUnadjusted,
            WireDeltaConvention::ForwardUnadjusted => D::ForwardUnadjusted,
            WireDeltaConvention::SpotPremiumAdjusted => D::SpotPremiumAdjusted,
            WireDeltaConvention::ForwardPremiumAdjusted => D::ForwardPremiumAdjusted,
        }
    }

    /// The wire variant denoting a canonical [`celnet_types::DeltaConvention`].
    #[must_use]
    pub const fn from_canonical(d: celnet_types::DeltaConvention) -> Self {
        use celnet_types::DeltaConvention as D;
        match d {
            D::SpotUnadjusted => WireDeltaConvention::SpotUnadjusted,
            D::ForwardUnadjusted => WireDeltaConvention::ForwardUnadjusted,
            D::SpotPremiumAdjusted => WireDeltaConvention::SpotPremiumAdjusted,
            D::ForwardPremiumAdjusted => WireDeltaConvention::ForwardPremiumAdjusted,
        }
    }
}

/// How an external feed expresses its at-the-money strike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WireAtmConvention {
    /// ATM-forward (`K = F`).
    AtmForward,
    /// Delta-neutral straddle (`Δ_call + Δ_put = 0`).
    DeltaNeutralStraddle,
}

impl WireAtmConvention {
    /// The canonical [`celnet_types::AtmConvention`] this wire variant denotes.
    #[must_use]
    pub const fn canonical(self) -> celnet_types::AtmConvention {
        use celnet_types::AtmConvention as A;
        match self {
            WireAtmConvention::AtmForward => A::AtmForward,
            WireAtmConvention::DeltaNeutralStraddle => A::DeltaNeutralStraddle,
        }
    }

    /// The wire variant denoting a canonical [`celnet_types::AtmConvention`].
    #[must_use]
    pub const fn from_canonical(a: celnet_types::AtmConvention) -> Self {
        use celnet_types::AtmConvention as A;
        match a {
            A::AtmForward => WireAtmConvention::AtmForward,
            A::DeltaNeutralStraddle => WireAtmConvention::DeltaNeutralStraddle,
        }
    }
}

/// The self-declared convention descriptor a feed attaches to a smile message.
///
/// FMD-style feeds carry no published convention spec, so a robust adapter
/// requires the feed (or a per-feed configuration shim) to *declare* its delta
/// style, ATM style, and whether the premium is paid in the foreign (base)
/// currency. The normalizer ([`crate::normalize`]) uses this descriptor to
/// resolve the delta↔strike map correctly — and to *cross-check* the feed's
/// declared convention against the canonical convention Celnet resolves for the
/// `(pair, tenor)`, flagging a mismatch rather than silently corrupting strikes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WireConventions {
    /// The feed's declared delta convention.
    pub delta: WireDeltaConvention,
    /// The feed's declared ATM convention.
    pub atm: WireAtmConvention,
    /// Whether the premium is paid in the foreign (base) currency (and so the
    /// delta is premium-adjusted). Redundant with [`Self::delta`] but carried
    /// explicitly because some feeds declare the premium currency separately;
    /// the normalizer verifies the two agree.
    pub premium_in_foreign: bool,
}

/// A risk-reversal / butterfly wing pair as published on the wire, in
/// **percentage points** (so `0.25` denotes `0.0025` absolute vol).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireWing {
    /// Unsigned delta of the pillar in percent (e.g. `25.0` for the `25Δ` wing).
    pub delta_pct: f64,
    /// Risk-reversal `σ_call − σ_put` in percentage points.
    pub risk_reversal_pct: f64,
    /// Market (broker) butterfly `½(σ_call+σ_put) − σ_ATM` in percentage points.
    pub butterfly_pct: f64,
}

/// The forward published for the slice: either explicit forward *points* (added
/// to spot, scaled by the feed's pip factor) or an outright forward rate. FX
/// feeds publish points for short/liquid tenors and outrights for long tenors;
/// the adapter accepts either.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum WireForward {
    /// Forward *points*: `F = spot + points / pip_factor` (e.g. points `=85.0`,
    /// `pip_factor = 10_000` ⇒ `F = spot + 0.0085`).
    Points {
        /// Forward points (in pips).
        points: f64,
        /// Pips per unit of the quote currency (e.g. `10_000` for EURUSD,
        /// `100` for USDJPY).
        pip_factor: f64,
    },
    /// An outright forward rate (quote per 1 unit of base).
    Outright {
        /// The outright forward.
        rate: f64,
    },
}

impl WireForward {
    /// Resolve the outright forward from `spot`.
    #[must_use]
    pub fn outright(self, spot: f64) -> f64 {
        match self {
            WireForward::Points { points, pip_factor } => spot + points / pip_factor,
            WireForward::Outright { rate } => rate,
        }
    }
}

/// A single vendor smile message: one `(pair, tenor)` slice as decoded from the
/// feed, in the feed's own units (percent vols, forward points) and convention.
///
/// `pair` is the 6-letter market form (`"EURUSD"`); `tenor_label` is the feed's
/// tenor token (`"1M"`, `"3M"`, `"1Y"`, `"2Y"`, `"ON"`, `"2W"`) parsed by
/// [`crate::normalize`]. The outer (`10Δ`) wing is optional. `ndf_fixing`, when
/// present, is the published cash-settlement fixing for a non-deliverable pair
/// (used by the NDF/NDO settlement path; informational for the surface input).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VendorSmileMessage {
    /// 6-letter market pair form, e.g. `"EURUSD"`.
    pub pair: String,
    /// Feed tenor token, e.g. `"3M"`, `"1Y"`.
    pub tenor_label: String,
    /// Spot mid (quote per 1 unit of base).
    pub spot: f64,
    /// The forward for the slice (points or outright).
    pub forward: WireForward,
    /// At-the-money volatility in percentage points (e.g. `10.5` ⇒ `0.105`).
    pub atm_vol_pct: f64,
    /// The inner (`25Δ`) wing.
    pub inner: WireWing,
    /// The optional outer (`10Δ`) wing.
    pub outer: Option<WireWing>,
    /// Optional published NDF cash-settlement fixing (non-deliverable pairs).
    pub ndf_fixing: Option<f64>,
    /// The feed's self-declared conventions for this slice.
    pub conventions: WireConventions,
    /// Source feed identifier (free-form, e.g. `"feed-a"`), used by the
    /// aggregation layer to attribute divergence and staleness to a source.
    pub source: String,
    /// Feed timestamp in **epoch nanoseconds** — the instant the slice was
    /// observed/published, used by the staleness-decay weighting.
    pub observed_at_nanos: i64,
}

impl VendorSmileMessage {
    /// Decode a JSON wire body into a [`VendorSmileMessage`].
    ///
    /// # Errors
    ///
    /// Returns the underlying [`serde_json::Error`] when the body is not a
    /// valid encoding of this shape.
    pub fn from_json(body: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(body)
    }

    /// Encode this message to a JSON wire body.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`serde_json::Error`] on a serialization failure
    /// (e.g. a non-finite float, which JSON cannot represent).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_convention_round_trips_through_canonical() {
        use celnet_types::DeltaConvention as D;
        for d in [
            D::SpotUnadjusted,
            D::ForwardUnadjusted,
            D::SpotPremiumAdjusted,
            D::ForwardPremiumAdjusted,
        ] {
            assert_eq!(WireDeltaConvention::from_canonical(d).canonical(), d);
        }
    }

    #[test]
    fn atm_convention_round_trips_through_canonical() {
        use celnet_types::AtmConvention as A;
        for a in [A::AtmForward, A::DeltaNeutralStraddle] {
            assert_eq!(WireAtmConvention::from_canonical(a).canonical(), a);
        }
    }

    #[test]
    fn forward_points_and_outright_resolve() {
        let pts = WireForward::Points {
            points: 85.0,
            pip_factor: 10_000.0,
        };
        celnet_core::assert_close!(pts.outright(1.1000), 1.1085);
        let out = WireForward::Outright { rate: 1.1085 };
        celnet_core::assert_close!(out.outright(1.1000), 1.1085);
    }

    #[test]
    fn message_json_round_trip() {
        let msg = VendorSmileMessage {
            pair: "EURUSD".into(),
            tenor_label: "1Y".into(),
            spot: 1.10,
            forward: WireForward::Points {
                points: 110.0,
                pip_factor: 10_000.0,
            },
            atm_vol_pct: 10.5,
            inner: WireWing {
                delta_pct: 25.0,
                risk_reversal_pct: 0.45,
                butterfly_pct: 0.20,
            },
            outer: Some(WireWing {
                delta_pct: 10.0,
                risk_reversal_pct: 0.80,
                butterfly_pct: 0.55,
            }),
            ndf_fixing: None,
            conventions: WireConventions {
                delta: WireDeltaConvention::ForwardPremiumAdjusted,
                atm: WireAtmConvention::DeltaNeutralStraddle,
                premium_in_foreign: true,
            },
            source: "feed-a".into(),
            observed_at_nanos: 1_700_000_000_000_000_000,
        };
        let json = msg.to_json().unwrap();
        let back = VendorSmileMessage::from_json(&json).unwrap();
        assert_eq!(msg, back);
    }
}
