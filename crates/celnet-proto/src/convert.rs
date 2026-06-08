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
    AtmConvention, BrokenDate, Carry, Ccy, CcyPair, Cut, DayCount, DeltaConvention, Greeks,
    OptionType, PremiumStyle, RateSensitivities, Settlement, SmileModel, Tenor, Underlying,
    VanillaInputs,
};

use crate::{
    AtmConvention as WireAtmConvention, BrokenDate as WireBrokenDate, CarryModel as WireCarryModel,
    CcyPair as WireCcyPair, Cut as WireCut, DayCount as WireDayCount,
    DeltaConvention as WireDeltaConvention, Greeks as WireGreeks, OptionType as WireOptionType,
    PremiumStyle as WirePremiumStyle, RateSensitivities as WireRateSensitivities,
    Settlement as WireSettlement, SmileModel as WireSmileModel, Tenor as WireTenor,
    Underlying as WireUnderlying, VanillaInputs as WireVanillaInputs, carry_model,
    rate_sensitivities, tenor, underlying,
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
    /// A wire scalar carried a value outside the domain type's representable
    /// range (e.g. a tenor count that does not fit a `u16`).
    OutOfRange {
        /// The offending field's name.
        field: &'static str,
        /// The out-of-range value as received on the wire.
        value: i64,
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
            WireError::OutOfRange { field, value } => {
                write!(f, "value {value} out of range for `{field}`")
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

// ---- AtmConvention ---------------------------------------------------------

impl From<AtmConvention> for WireAtmConvention {
    fn from(value: AtmConvention) -> Self {
        match value {
            AtmConvention::AtmForward => WireAtmConvention::AtmForward,
            AtmConvention::DeltaNeutralStraddle => WireAtmConvention::DeltaNeutralStraddle,
        }
    }
}

impl From<WireAtmConvention> for AtmConvention {
    fn from(value: WireAtmConvention) -> Self {
        match value {
            WireAtmConvention::AtmForward => AtmConvention::AtmForward,
            WireAtmConvention::DeltaNeutralStraddle => AtmConvention::DeltaNeutralStraddle,
        }
    }
}

// ---- Cut -------------------------------------------------------------------

impl From<Cut> for WireCut {
    fn from(value: Cut) -> Self {
        match value {
            Cut::NewYork1000 => WireCut::NewYork1000,
            Cut::Tokyo1500 => WireCut::Tokyo1500,
        }
    }
}

impl From<WireCut> for Cut {
    fn from(value: WireCut) -> Self {
        match value {
            WireCut::NewYork1000 => Cut::NewYork1000,
            WireCut::Tokyo1500 => Cut::Tokyo1500,
        }
    }
}

// ---- DayCount --------------------------------------------------------------

impl From<DayCount> for WireDayCount {
    fn from(value: DayCount) -> Self {
        match value {
            DayCount::Act365Fixed => WireDayCount::Act365Fixed,
            DayCount::Act360 => WireDayCount::Act360,
        }
    }
}

impl From<WireDayCount> for DayCount {
    fn from(value: WireDayCount) -> Self {
        match value {
            WireDayCount::Act365Fixed => DayCount::Act365Fixed,
            WireDayCount::Act360 => DayCount::Act360,
        }
    }
}

// ---- Settlement ------------------------------------------------------------

impl From<Settlement> for WireSettlement {
    fn from(value: Settlement) -> Self {
        match value {
            Settlement::Deliverable => WireSettlement::Deliverable,
            Settlement::NonDeliverable => WireSettlement::NonDeliverable,
        }
    }
}

impl From<WireSettlement> for Settlement {
    fn from(value: WireSettlement) -> Self {
        match value {
            WireSettlement::Deliverable => Settlement::Deliverable,
            WireSettlement::NonDeliverable => Settlement::NonDeliverable,
        }
    }
}

// ---- Tenor -----------------------------------------------------------------

impl From<BrokenDate> for WireBrokenDate {
    fn from(value: BrokenDate) -> Self {
        WireBrokenDate {
            year: value.year,
            month: u32::from(value.month),
            day: u32::from(value.day),
        }
    }
}

impl TryFrom<WireBrokenDate> for BrokenDate {
    type Error = WireError;

    fn try_from(value: WireBrokenDate) -> Result<Self, Self::Error> {
        let month = u8::try_from(value.month).map_err(|_| WireError::OutOfRange {
            field: "BrokenDate.month",
            value: i64::from(value.month),
        })?;
        let day = u8::try_from(value.day).map_err(|_| WireError::OutOfRange {
            field: "BrokenDate.day",
            value: i64::from(value.day),
        })?;
        Ok(BrokenDate {
            year: value.year,
            month,
            day,
        })
    }
}

impl From<Tenor> for WireTenor {
    fn from(value: Tenor) -> Self {
        let (unit, count, broken_date) = match value {
            Tenor::Overnight => (tenor::Unit::Overnight, 0_u32, None),
            Tenor::TomNext => (tenor::Unit::TomNext, 0_u32, None),
            Tenor::SpotNext => (tenor::Unit::SpotNext, 0_u32, None),
            Tenor::Weeks(n) => (tenor::Unit::Weeks, u32::from(n), None),
            Tenor::Months(n) => (tenor::Unit::Months, u32::from(n), None),
            Tenor::Years(n) => (tenor::Unit::Years, u32::from(n), None),
            Tenor::Imm(n) => (tenor::Unit::Imm, u32::from(n), None),
            Tenor::BrokenDate(b) => (
                tenor::Unit::BrokenDate,
                0_u32,
                Some(WireBrokenDate::from(b)),
            ),
        };
        WireTenor {
            unit: unit as i32,
            count,
            broken_date,
        }
    }
}

impl TryFrom<WireTenor> for Tenor {
    type Error = WireError;

    fn try_from(value: WireTenor) -> Result<Self, Self::Error> {
        let unit = tenor::Unit::try_from(value.unit).map_err(|_| WireError::UnknownEnum {
            kind: "Tenor.Unit",
            tag: value.unit,
        })?;
        // A tenor count must fit a `u16`; reject an out-of-range wire value.
        let count_u16 = || {
            u16::try_from(value.count).map_err(|_| WireError::OutOfRange {
                field: "Tenor.count",
                value: i64::from(value.count),
            })
        };
        // The IMM ordinal is a small 1-based count fitting a `u8`.
        let count_u8 = || {
            u8::try_from(value.count).map_err(|_| WireError::OutOfRange {
                field: "Tenor.count",
                value: i64::from(value.count),
            })
        };
        Ok(match unit {
            tenor::Unit::Overnight => Tenor::Overnight,
            tenor::Unit::TomNext => Tenor::TomNext,
            tenor::Unit::SpotNext => Tenor::SpotNext,
            tenor::Unit::Weeks => Tenor::Weeks(count_u16()?),
            tenor::Unit::Months => Tenor::Months(count_u16()?),
            tenor::Unit::Years => Tenor::Years(count_u16()?),
            tenor::Unit::Imm => Tenor::Imm(count_u8()?),
            tenor::Unit::BrokenDate => {
                let wire = value.broken_date.ok_or(WireError::MissingField {
                    field: "Tenor.broken_date",
                })?;
                Tenor::BrokenDate(BrokenDate::try_from(wire)?)
            }
        })
    }
}

impl From<SmileModel> for WireSmileModel {
    fn from(value: SmileModel) -> Self {
        match value {
            SmileModel::MarketHedge => WireSmileModel::MarketHedge,
            SmileModel::StochasticVol => WireSmileModel::StochasticVol,
            SmileModel::Parametric => WireSmileModel::Parametric,
            SmileModel::ParametricSurface => WireSmileModel::ParametricSurface,
            SmileModel::ExtendedSurface => WireSmileModel::ExtendedSurface,
        }
    }
}

impl TryFrom<WireSmileModel> for SmileModel {
    type Error = WireError;

    fn try_from(value: WireSmileModel) -> Result<Self, Self::Error> {
        Ok(match value {
            WireSmileModel::MarketHedge => SmileModel::MarketHedge,
            WireSmileModel::StochasticVol => SmileModel::StochasticVol,
            WireSmileModel::Parametric => SmileModel::Parametric,
            WireSmileModel::ParametricSurface => SmileModel::ParametricSurface,
            WireSmileModel::ExtendedSurface => SmileModel::ExtendedSurface,
        })
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

// ---- product × underlying validity -----------------------------------------

/// The asset-class validity guard: confirm an instrument's `underlying` is one
/// the product family can actually price, decoding it to a domain [`Underlying`].
///
/// W1 ships exactly one asset-class arm — FX — and every one of the 18 product
/// `oneof` arms is an FX product. So the rule is simply: the underlying must be
/// present and FX; a missing or non-FX underlying for an FX product is rejected
/// as `INVALID_ARGUMENT` rather than silently coerced (mirrors the
/// `PricingModel` guard's no-silent-fallback contract). Later asset-class waves
/// extend this to a genuine product×underlying matrix; today the matrix has one
/// valid cell.
///
/// # Errors
///
/// [`WireError::MissingField`] if `underlying.ref` is absent;
/// [`WireError::InvalidCcy`] if the FX pair's legs are malformed. (A future
/// non-FX arm reaching an FX-only product would surface as an
/// [`WireError::UnknownEnum`]-style rejection here.)
pub fn validate_fx_underlying(underlying: &WireUnderlying) -> Result<Underlying, WireError> {
    match &underlying.r#ref {
        Some(underlying::Ref::Fx(pair)) => Ok(Underlying::Fx(CcyPair::try_from(pair.clone())?)),
        None => Err(WireError::MissingField {
            field: "Instrument.underlying",
        }),
    }
}

// ---- Underlying ------------------------------------------------------------

impl From<Underlying> for WireUnderlying {
    fn from(value: Underlying) -> Self {
        match value {
            Underlying::Fx(pair) => WireUnderlying::fx(WireCcyPair::from(pair)),
        }
    }
}

impl TryFrom<WireUnderlying> for Underlying {
    type Error = WireError;

    fn try_from(value: WireUnderlying) -> Result<Self, Self::Error> {
        match value.r#ref {
            Some(underlying::Ref::Fx(pair)) => Ok(Underlying::Fx(CcyPair::try_from(pair)?)),
            None => Err(WireError::MissingField {
                field: "Underlying.ref",
            }),
        }
    }
}

// ---- Carry -----------------------------------------------------------------

impl From<Carry> for WireCarryModel {
    fn from(value: Carry) -> Self {
        match value {
            // FX: the foreign rate travels in the FX carry arm; the discount rate
            // (`r_dom`) lives out of band on `MarketContext`/`VanillaInputs`.
            Carry::FxRates { r_for, .. } => WireCarryModel::fx(r_for),
            Carry::CostOfCarry { b, .. } => WireCarryModel {
                model: Some(carry_model::Model::Generalized(crate::CostOfCarry { b })),
            },
        }
    }
}

// ---- RateSensitivities -----------------------------------------------------

impl From<RateSensitivities> for WireRateSensitivities {
    fn from(value: RateSensitivities) -> Self {
        match value {
            RateSensitivities::Fx { rho_dom, rho_for } => {
                WireRateSensitivities::fx(rho_dom, rho_for)
            }
            RateSensitivities::Carry {
                discount_rho,
                carry_rho,
            } => WireRateSensitivities {
                sensitivities: Some(rate_sensitivities::Sensitivities::Carry(
                    rate_sensitivities::CarryRho {
                        discount_rho,
                        carry_rho,
                    },
                )),
            },
        }
    }
}

impl TryFrom<WireRateSensitivities> for RateSensitivities {
    type Error = WireError;

    fn try_from(value: WireRateSensitivities) -> Result<Self, Self::Error> {
        match value.sensitivities {
            Some(rate_sensitivities::Sensitivities::Fx(fx)) => Ok(RateSensitivities::Fx {
                rho_dom: fx.rho_dom,
                rho_for: fx.rho_for,
            }),
            Some(rate_sensitivities::Sensitivities::Carry(c)) => Ok(RateSensitivities::Carry {
                discount_rho: c.discount_rho,
                carry_rho: c.carry_rho,
            }),
            None => Err(WireError::MissingField {
                field: "RateSensitivities.sensitivities",
            }),
        }
    }
}

// ---- VanillaInputs ---------------------------------------------------------

impl From<VanillaInputs> for WireVanillaInputs {
    fn from(value: VanillaInputs) -> Self {
        // The FX projection: `discount_rate = r_dom`, `carry.fx.r_for = r_for`.
        // `fx` round-trips both rates' exact bits.
        WireVanillaInputs::fx(
            value.spot,
            value.strike,
            value.vol,
            value.t,
            value.r_dom,
            value.r_for,
        )
    }
}

impl From<WireVanillaInputs> for VanillaInputs {
    fn from(value: WireVanillaInputs) -> Self {
        // The two FX rates are the `discount_rate` and the FX carry arm's `r_for`
        // (bit-identical to the former flat scalars).
        VanillaInputs::new(
            value.spot,
            value.strike,
            value.vol,
            value.t,
            value.r_dom(),
            value.r_for(),
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
            // The FX rate-sensitivity arm carries the two flat rhos bit-for-bit.
            rate_sensitivities: Some(WireRateSensitivities::fx(value.rho_dom, value.rho_for)),
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
            // The FX rhos are read back from the FX rate-sensitivity arm
            // (bit-identical to the former flat scalars; 0.0 for a non-FX strip).
            rho_dom: value.rho_dom(),
            rho_for: value.rho_for(),
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
    fn atm_convention_round_trips() {
        for ac in [
            AtmConvention::AtmForward,
            AtmConvention::DeltaNeutralStraddle,
        ] {
            assert_eq!(AtmConvention::from(WireAtmConvention::from(ac)), ac);
        }
    }

    #[test]
    fn cut_round_trips() {
        for c in [Cut::NewYork1000, Cut::Tokyo1500] {
            assert_eq!(Cut::from(WireCut::from(c)), c);
        }
    }

    #[test]
    fn day_count_round_trips() {
        for dc in [DayCount::Act365Fixed, DayCount::Act360] {
            assert_eq!(DayCount::from(WireDayCount::from(dc)), dc);
        }
    }

    #[test]
    fn settlement_round_trips() {
        for s in [Settlement::Deliverable, Settlement::NonDeliverable] {
            assert_eq!(Settlement::from(WireSettlement::from(s)), s);
        }
    }

    #[test]
    fn tenor_round_trips() {
        for t in [
            Tenor::Overnight,
            Tenor::TomNext,
            Tenor::SpotNext,
            Tenor::Weeks(2),
            Tenor::Months(3),
            Tenor::Years(1),
            Tenor::Imm(1),
            Tenor::Imm(4),
            Tenor::BrokenDate(BrokenDate::new(2026, 7, 17)),
        ] {
            let back = Tenor::try_from(WireTenor::from(t)).expect("tenor must round-trip");
            assert_eq!(back, t);
        }
    }

    #[test]
    fn smile_model_round_trips() {
        for m in [
            SmileModel::MarketHedge,
            SmileModel::StochasticVol,
            SmileModel::Parametric,
            SmileModel::ParametricSurface,
            SmileModel::ExtendedSurface,
        ] {
            let back =
                SmileModel::try_from(WireSmileModel::from(m)).expect("smile model round-trips");
            assert_eq!(back, m);
        }
    }

    #[test]
    fn broken_date_tenor_requires_the_date() {
        // A BROKEN_DATE wire tenor with no `broken_date` is a decode error, not a
        // silent fallback.
        let wire = WireTenor {
            unit: tenor::Unit::BrokenDate as i32,
            count: 0,
            broken_date: None,
        };
        assert_eq!(
            Tenor::try_from(wire),
            Err(WireError::MissingField {
                field: "Tenor.broken_date",
            })
        );
    }

    #[test]
    fn tenor_rejects_out_of_range_count() {
        let wire = WireTenor {
            unit: tenor::Unit::Months as i32,
            count: u32::from(u16::MAX) + 1,
            broken_date: None,
        };
        assert_eq!(
            Tenor::try_from(wire),
            Err(WireError::OutOfRange {
                field: "Tenor.count",
                value: i64::from(u16::MAX) + 1,
            })
        );
    }

    #[test]
    fn tenor_rejects_unknown_unit() {
        let wire = WireTenor {
            unit: 99,
            count: 1,
            broken_date: None,
        };
        assert_eq!(
            Tenor::try_from(wire),
            Err(WireError::UnknownEnum {
                kind: "Tenor.Unit",
                tag: 99,
            })
        );
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
    fn underlying_round_trips_fx() {
        let u = Underlying::Fx(CcyPair::new(Ccy::EUR, Ccy::USD));
        let wire = WireUnderlying::from(u);
        // The settlement currency is stamped as the FX quote (domestic) leg.
        assert_eq!(wire.settlement_ccy, "USD");
        assert_eq!(Underlying::try_from(wire).unwrap(), u);
    }

    #[test]
    fn underlying_rejects_missing_ref() {
        let wire = WireUnderlying {
            r#ref: None,
            settlement_ccy: "USD".to_owned(),
        };
        assert_eq!(
            Underlying::try_from(wire),
            Err(WireError::MissingField {
                field: "Underlying.ref",
            })
        );
    }

    #[test]
    fn validate_fx_underlying_accepts_fx_rejects_absent() {
        let ok = WireUnderlying::fx(WireCcyPair::from(CcyPair::new(Ccy::GBP, Ccy::USD)));
        assert_eq!(
            super::validate_fx_underlying(&ok).unwrap(),
            Underlying::Fx(CcyPair::new(Ccy::GBP, Ccy::USD))
        );
        let absent = WireUnderlying {
            r#ref: None,
            settlement_ccy: String::new(),
        };
        assert_eq!(
            super::validate_fx_underlying(&absent),
            Err(WireError::MissingField {
                field: "Instrument.underlying",
            })
        );
    }

    #[test]
    fn carry_fx_maps_r_for_only() {
        // The FX carry arm carries `r_for`; the discount (`r_dom`) is out of band.
        let wire = WireCarryModel::from(Carry::FxRates {
            r_dom: 0.045,
            r_for: 0.03,
        });
        assert_eq!(wire.fx_r_for(), Some(0.03));
    }

    #[test]
    fn rate_sensitivities_round_trips_both_arms() {
        let fx = RateSensitivities::Fx {
            rho_dom: 0.06,
            rho_for: -0.058,
        };
        assert_eq!(
            RateSensitivities::try_from(WireRateSensitivities::from(fx)).unwrap(),
            fx
        );
        let carry = RateSensitivities::Carry {
            discount_rho: 0.25,
            carry_rho: 0.125,
        };
        assert_eq!(
            RateSensitivities::try_from(WireRateSensitivities::from(carry)).unwrap(),
            carry
        );
    }

    #[test]
    fn fx_vanilla_inputs_byte_identical_round_trip() {
        use prost::Message;
        // The FX projection of VanillaInputs must round-trip bit-for-bit through
        // the generalized {discount_rate, carry} wire form. Use rates whose bits
        // must be preserved exactly (0.1 + 0.2 is not representable as 0.3).
        let inputs = VanillaInputs::new(1.1, 1.105, 0.0825, 0.5, 0.1 + 0.2, 0.3);
        let wire = WireVanillaInputs::from(inputs);
        // discount_rate IS r_dom, bit-for-bit; carry.fx.r_for IS r_for.
        assert_eq!(wire.discount_rate.to_bits(), inputs.r_dom.to_bits());
        assert_eq!(wire.r_for().to_bits(), inputs.r_for.to_bits());
        // Encode → decode the proto bytes and confirm the domain rates survive
        // to_bits-identically through the wire codec.
        let bytes = wire.encode_to_vec();
        let decoded = WireVanillaInputs::decode(bytes.as_slice()).unwrap();
        let back = VanillaInputs::from(decoded);
        assert_eq!(back.spot.to_bits(), inputs.spot.to_bits());
        assert_eq!(back.strike.to_bits(), inputs.strike.to_bits());
        assert_eq!(back.vol.to_bits(), inputs.vol.to_bits());
        assert_eq!(back.t.to_bits(), inputs.t.to_bits());
        assert_eq!(back.r_dom.to_bits(), inputs.r_dom.to_bits());
        assert_eq!(back.r_for.to_bits(), inputs.r_for.to_bits());
    }

    #[test]
    fn fx_greeks_rho_byte_identical_round_trip() {
        use prost::Message;
        let g = Greeks {
            price: 0.0123,
            delta_spot: 0.48,
            delta_forward: 0.49,
            gamma: 2.1,
            vega: 0.3,
            theta: -0.018,
            rho_dom: 0.1 + 0.2,
            rho_for: -0.058,
            vanna: -0.072,
            volga: 0.144,
            charm: 0.0009,
            speed: -1.21,
            zomma: 0.33,
            color: 0.0004,
        };
        let wire = WireGreeks::from(g);
        let bytes = wire.encode_to_vec();
        let back = Greeks::from(WireGreeks::decode(bytes.as_slice()).unwrap());
        assert_eq!(back.rho_dom.to_bits(), g.rho_dom.to_bits());
        assert_eq!(back.rho_for.to_bits(), g.rho_for.to_bits());
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
