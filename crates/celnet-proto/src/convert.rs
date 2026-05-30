//! Conversions between the wire messages and the [`celnet_types`] DTOs.
//!
//! The wire schema mirrors the in-process vocabulary one-to-one, so these
//! conversions are total and lossless except where the wire form is strictly
//! more permissive than the domain type (a free-text currency code, or a proto3
//! enum integer that has no matching variant). Those fallible cases convert via
//! [`TryFrom`] with the dedicated [`WireError`]; everything else is an infallible
//! [`From`]. Centralizing the mapping here keeps the wire and domain layers from
//! ever drifting apart.

use celnet_types::{
    Ccy, CcyPair, DeltaConvention, Greeks, OptionType, PremiumStyle, VanillaInputs,
};

use crate::{
    CcyPair as WireCcyPair, DeltaConvention as WireDeltaConvention, Greeks as WireGreeks,
    OptionType as WireOptionType, PremiumStyle as WirePremiumStyle,
    VanillaInputs as WireVanillaInputs,
};

/// A decode-side mapping failure: the wire carried a value the domain type
/// cannot represent (a malformed currency code or an out-of-range enum tag).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// A currency leg was not a 3-letter ASCII-alphabetic code.
    InvalidCcy {
        /// The leg name (`"base"` or `"quote"`).
        field: &'static str,
        /// The offending value as received on the wire.
        value: String,
    },
    /// A proto3 enum tag did not correspond to any known variant.
    UnknownEnum {
        /// The enum type name (e.g. `"OptionType"`).
        kind: &'static str,
        /// The unrecognized integer tag.
        tag: i32,
    },
    /// A required nested message was absent (`None`) on the wire.
    MissingField {
        /// The absent field's name.
        field: &'static str,
    },
}

impl core::fmt::Display for WireError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WireError::InvalidCcy { field, value } => {
                write!(f, "invalid currency code for `{field}`: {value:?}")
            }
            WireError::UnknownEnum { kind, tag } => {
                write!(f, "unknown {kind} enum tag: {tag}")
            }
            WireError::MissingField { field } => {
                write!(f, "missing required field `{field}`")
            }
        }
    }
}

impl std::error::Error for WireError {}

// ---- OptionType -----------------------------------------------------------

impl From<OptionType> for WireOptionType {
    fn from(value: OptionType) -> Self {
        match value {
            OptionType::Call => WireOptionType::Call,
            OptionType::Put => WireOptionType::Put,
        }
    }
}

impl From<WireOptionType> for OptionType {
    fn from(value: WireOptionType) -> Self {
        match value {
            WireOptionType::Call => OptionType::Call,
            WireOptionType::Put => OptionType::Put,
        }
    }
}

// ---- DeltaConvention -------------------------------------------------------

impl From<DeltaConvention> for WireDeltaConvention {
    fn from(value: DeltaConvention) -> Self {
        match value {
            DeltaConvention::SpotUnadjusted => WireDeltaConvention::SpotUnadjusted,
            DeltaConvention::ForwardUnadjusted => WireDeltaConvention::ForwardUnadjusted,
            DeltaConvention::SpotPremiumAdjusted => WireDeltaConvention::SpotPremiumAdjusted,
            DeltaConvention::ForwardPremiumAdjusted => WireDeltaConvention::ForwardPremiumAdjusted,
        }
    }
}

impl From<WireDeltaConvention> for DeltaConvention {
    fn from(value: WireDeltaConvention) -> Self {
        match value {
            WireDeltaConvention::SpotUnadjusted => DeltaConvention::SpotUnadjusted,
            WireDeltaConvention::ForwardUnadjusted => DeltaConvention::ForwardUnadjusted,
            WireDeltaConvention::SpotPremiumAdjusted => DeltaConvention::SpotPremiumAdjusted,
            WireDeltaConvention::ForwardPremiumAdjusted => DeltaConvention::ForwardPremiumAdjusted,
        }
    }
}

// ---- PremiumStyle ----------------------------------------------------------

impl From<PremiumStyle> for WirePremiumStyle {
    fn from(value: PremiumStyle) -> Self {
        match value {
            PremiumStyle::DomesticPips => WirePremiumStyle::DomesticPips,
            PremiumStyle::PercentForeign => WirePremiumStyle::PercentForeign,
            PremiumStyle::PercentDomestic => WirePremiumStyle::PercentDomestic,
            PremiumStyle::ForeignPips => WirePremiumStyle::ForeignPips,
        }
    }
}

impl From<WirePremiumStyle> for PremiumStyle {
    fn from(value: WirePremiumStyle) -> Self {
        match value {
            WirePremiumStyle::DomesticPips => PremiumStyle::DomesticPips,
            WirePremiumStyle::PercentForeign => PremiumStyle::PercentForeign,
            WirePremiumStyle::PercentDomestic => PremiumStyle::PercentDomestic,
            WirePremiumStyle::ForeignPips => PremiumStyle::ForeignPips,
        }
    }
}

// ---- CcyPair ---------------------------------------------------------------

impl From<CcyPair> for WireCcyPair {
    fn from(value: CcyPair) -> Self {
        WireCcyPair {
            base: value.base.as_str().to_owned(),
            quote: value.quote.as_str().to_owned(),
        }
    }
}

impl TryFrom<WireCcyPair> for CcyPair {
    type Error = WireError;

    fn try_from(value: WireCcyPair) -> Result<Self, Self::Error> {
        let base = Ccy::parse(&value.base).ok_or(WireError::InvalidCcy {
            field: "base",
            value: value.base,
        })?;
        let quote = Ccy::parse(&value.quote).ok_or(WireError::InvalidCcy {
            field: "quote",
            value: value.quote,
        })?;
        Ok(CcyPair::new(base, quote))
    }
}

// ---- VanillaInputs ---------------------------------------------------------

impl From<VanillaInputs> for WireVanillaInputs {
    fn from(value: VanillaInputs) -> Self {
        WireVanillaInputs {
            spot: value.spot,
            strike: value.strike,
            vol: value.vol,
            t: value.t,
            r_dom: value.r_dom,
            r_for: value.r_for,
        }
    }
}

impl From<WireVanillaInputs> for VanillaInputs {
    fn from(value: WireVanillaInputs) -> Self {
        VanillaInputs::new(
            value.spot,
            value.strike,
            value.vol,
            value.t,
            value.r_dom,
            value.r_for,
        )
    }
}

// ---- Greeks ----------------------------------------------------------------

impl From<Greeks> for WireGreeks {
    fn from(value: Greeks) -> Self {
        WireGreeks {
            price: value.price,
            delta_spot: value.delta_spot,
            delta_forward: value.delta_forward,
            gamma: value.gamma,
            vega: value.vega,
            theta: value.theta,
            rho_dom: value.rho_dom,
            rho_for: value.rho_for,
            vanna: value.vanna,
            volga: value.volga,
            charm: value.charm,
            speed: value.speed,
            zomma: value.zomma,
            color: value.color,
        }
    }
}

impl From<WireGreeks> for Greeks {
    fn from(value: WireGreeks) -> Self {
        Greeks {
            price: value.price,
            delta_spot: value.delta_spot,
            delta_forward: value.delta_forward,
            gamma: value.gamma,
            vega: value.vega,
            theta: value.theta,
            rho_dom: value.rho_dom,
            rho_for: value.rho_for,
            vanna: value.vanna,
            volga: value.volga,
            charm: value.charm,
            speed: value.speed,
            zomma: value.zomma,
            color: value.color,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_core::assert_close;

    #[test]
    fn option_type_round_trips() {
        for ot in [OptionType::Call, OptionType::Put] {
            assert_eq!(OptionType::from(WireOptionType::from(ot)), ot);
        }
    }

    #[test]
    fn delta_convention_round_trips() {
        for dc in [
            DeltaConvention::SpotUnadjusted,
            DeltaConvention::ForwardUnadjusted,
            DeltaConvention::SpotPremiumAdjusted,
            DeltaConvention::ForwardPremiumAdjusted,
        ] {
            assert_eq!(DeltaConvention::from(WireDeltaConvention::from(dc)), dc);
        }
    }

    #[test]
    fn premium_style_round_trips() {
        for ps in [
            PremiumStyle::DomesticPips,
            PremiumStyle::PercentForeign,
            PremiumStyle::PercentDomestic,
            PremiumStyle::ForeignPips,
        ] {
            assert_eq!(PremiumStyle::from(WirePremiumStyle::from(ps)), ps);
        }
    }

    #[test]
    fn ccy_pair_round_trips() {
        let pair = CcyPair::new(Ccy::EUR, Ccy::USD);
        let wire = WireCcyPair::from(pair);
        assert_eq!(wire.base, "EUR");
        assert_eq!(wire.quote, "USD");
        assert_eq!(CcyPair::try_from(wire).unwrap(), pair);
    }

    #[test]
    fn ccy_pair_rejects_bad_code() {
        let bad = WireCcyPair {
            base: "EU1".to_owned(),
            quote: "USD".to_owned(),
        };
        assert_eq!(
            CcyPair::try_from(bad),
            Err(WireError::InvalidCcy {
                field: "base",
                value: "EU1".to_owned(),
            })
        );
    }

    #[test]
    fn vanilla_inputs_round_trips() {
        let inputs = VanillaInputs::new(1.1, 1.105, 0.0825, 0.5, 0.045, 0.03);
        let back = VanillaInputs::from(WireVanillaInputs::from(inputs));
        assert_close!(back.spot, inputs.spot);
        assert_close!(back.strike, inputs.strike);
        assert_close!(back.vol, inputs.vol);
        assert_close!(back.t, inputs.t);
        assert_close!(back.r_dom, inputs.r_dom);
        assert_close!(back.r_for, inputs.r_for);
    }

    #[test]
    fn greeks_round_trips() {
        let g = Greeks {
            price: 0.0123,
            delta_spot: 0.48,
            delta_forward: 0.49,
            gamma: 2.1,
            vega: 0.3,
            theta: -0.018,
            rho_dom: 0.06,
            rho_for: -0.058,
            vanna: -0.072,
            volga: 0.144,
            charm: 0.0009,
            speed: -1.21,
            zomma: 0.33,
            color: 0.0004,
        };
        let back = Greeks::from(WireGreeks::from(g));
        assert_close!(back.price, g.price);
        assert_close!(back.delta_spot, g.delta_spot);
        assert_close!(back.delta_forward, g.delta_forward);
        assert_close!(back.gamma, g.gamma);
        assert_close!(back.vega, g.vega);
        assert_close!(back.theta, g.theta);
        assert_close!(back.rho_dom, g.rho_dom);
        assert_close!(back.rho_for, g.rho_for);
        assert_close!(back.vanna, g.vanna);
        assert_close!(back.volga, g.volga);
        assert_close!(back.charm, g.charm);
        assert_close!(back.speed, g.speed);
        assert_close!(back.zomma, g.zomma);
        assert_close!(back.color, g.color);
    }
}
