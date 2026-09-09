//! Wire ⇄ domain mapping for the **instrument reference-data** registry: between
//! the proto `InstrumentDefDesc` family (and its `definition` oneof) and the
//! persisted [`InstrumentDef`](crate::config::reference_data::InstrumentDef).
//!
//! Kept out of [`super::auth`] so the (six-family) mapping does not bloat the
//! `AuthService` impl. The `*_from_wire` direction is total and validating: an
//! unset `definition` oneof or a missing required `BrokenDate` is rejected with
//! `invalid_argument` (the deeper convention-label/uniqueness validation happens
//! in [`validate_instruments`](crate::config::reference_data::validate_instruments)
//! once the candidate store is assembled).
//!
//! `clippy::result_large_err` is allowed module-wide: `tonic::Status` is the
//! framework-mandated error type for these wire mappers, so boxing it would
//! only push the indirection into every gRPC handler without real benefit.
#![allow(clippy::result_large_err)]

use celnet_proto::{
    BondDef as WireBond, BondFutureDef as WireBondFuture, BrokenDate, DepositDef as WireDeposit,
    ExternalId as WireExternalId, FraDef as WireFra, InstrumentDefDesc, OisDef as WireOis,
    StirFutureDef as WireStir, VanillaIrsDef as WireIrs,
    instrument_def_desc::Definition as WireDefinition,
};
use tonic::Status;

use crate::config::reference_data::{
    BondDef, BondFutureDef, CivilDate, DepositDef, ExternalId, FraDef, InstrumentDef,
    InstrumentFamily, OisDef, StirFutureDef, VanillaIrsDef,
};

// --- domain → wire ----------------------------------------------------------

/// Map a stored [`InstrumentDef`] onto its wire [`InstrumentDefDesc`].
#[must_use]
pub fn instrument_to_wire(d: &InstrumentDef) -> InstrumentDefDesc {
    InstrumentDefDesc {
        instrument_id: d.instrument_id.clone(),
        name: d.name.clone(),
        description: d.description.clone(),
        currency: d.currency.clone(),
        external_ids: d.external_ids.iter().map(external_to_wire).collect(),
        sub_asset_type: d.sub_asset_type.clone(),
        region: d.region.clone(),
        definition: Some(family_to_wire(&d.definition)),
    }
}

fn external_to_wire(x: &ExternalId) -> WireExternalId {
    WireExternalId {
        scheme: x.scheme.clone(),
        value: x.value.clone(),
    }
}

fn date_to_wire(d: CivilDate) -> BrokenDate {
    BrokenDate {
        year: d.year,
        month: d.month,
        day: d.day,
    }
}

fn family_to_wire(f: &InstrumentFamily) -> WireDefinition {
    match f {
        InstrumentFamily::Deposit(d) => WireDefinition::Deposit(WireDeposit {
            index: d.index.clone(),
            tenor: d.tenor.clone(),
            day_count: d.day_count.clone(),
            business_day_convention: d.business_day_convention.clone(),
            calendars: d.calendars.clone(),
            spot_lag_days: d.spot_lag_days,
        }),
        InstrumentFamily::Fra(f) => WireDefinition::Fra(WireFra {
            float_index: f.float_index.clone(),
            start_tenor: f.start_tenor.clone(),
            end_tenor: f.end_tenor.clone(),
            accrual_day_count: f.accrual_day_count.clone(),
            business_day_convention: f.business_day_convention.clone(),
            calendars: f.calendars.clone(),
            spot_lag_days: f.spot_lag_days,
        }),
        InstrumentFamily::StirFuture(s) => WireDefinition::StirFuture(WireStir {
            contract_code: s.contract_code.clone(),
            reference_start: s.reference_start.clone(),
            reference_end: s.reference_end.clone(),
            day_count: s.day_count.clone(),
            calendars: s.calendars.clone(),
            convexity_vol: s.convexity_vol,
            contract_size: s.contract_size,
        }),
        InstrumentFamily::VanillaIrs(v) => WireDefinition::VanillaIrs(WireIrs {
            tenor: v.tenor.clone(),
            fixed_frequency: v.fixed_frequency.clone(),
            fixed_day_count: v.fixed_day_count.clone(),
            float_index: v.float_index.clone(),
            float_frequency: v.float_frequency.clone(),
            float_day_count: v.float_day_count.clone(),
            business_day_convention: v.business_day_convention.clone(),
            calendars: v.calendars.clone(),
            roll_convention: v.roll_convention.clone(),
            spot_lag_days: v.spot_lag_days,
        }),
        InstrumentFamily::Ois(o) => WireDefinition::Ois(WireOis {
            tenor: o.tenor.clone(),
            index: o.index.clone(),
            fixed_frequency: o.fixed_frequency.clone(),
            fixed_day_count: o.fixed_day_count.clone(),
            float_day_count: o.float_day_count.clone(),
            business_day_convention: o.business_day_convention.clone(),
            calendars: o.calendars.clone(),
            spot_lag_days: o.spot_lag_days,
        }),
        InstrumentFamily::Bond(b) => WireDefinition::Bond(WireBond {
            issuer: b.issuer.clone(),
            coupon_rate: b.coupon_rate,
            coupon_type: b.coupon_type.clone(),
            coupon_frequency: b.coupon_frequency.clone(),
            day_count: b.day_count.clone(),
            issue_date: b.issue_date.map(date_to_wire),
            dated_date: b.dated_date.map(date_to_wire),
            first_coupon_date: b.first_coupon_date.map(date_to_wire),
            maturity_date: Some(date_to_wire(b.maturity_date)),
            redemption: b.redemption,
            calendars: b.calendars.clone(),
        }),
        InstrumentFamily::BondFuture(f) => WireDefinition::BondFuture(WireBondFuture {
            contract_code: f.contract_code.clone(),
            contract_symbol: f.contract_symbol.clone(),
            underlying_issuer: f.underlying_issuer.clone(),
            contract_face_value: f.contract_face_value,
            tick_size_points: f.tick_size_points,
            tick_value: f.tick_value,
            notional_coupon_rate: f.notional_coupon_rate,
            deliverable_min_months: f.deliverable_min_months,
            deliverable_max_months: f.deliverable_max_months,
            delivery_month_start: Some(date_to_wire(f.delivery_month_start)),
            first_delivery_date: Some(date_to_wire(f.first_delivery_date)),
            last_trading_date: Some(date_to_wire(f.last_trading_date)),
            last_delivery_date: Some(date_to_wire(f.last_delivery_date)),
            dv01_per_contract_at_notional_yield: f.dv01_per_contract_at_notional_yield,
            calendars: f.calendars.clone(),
        }),
    }
}

// --- wire → domain ----------------------------------------------------------

/// Resolve a wire [`InstrumentDefDesc`] into a stored [`InstrumentDef`], rejecting
/// an unset `definition` oneof or a missing required `maturity_date`.
///
/// # Errors
/// `invalid_argument` when the family oneof is unset or a required date is absent.
pub fn instrument_from_wire(d: &InstrumentDefDesc) -> Result<InstrumentDef, Status> {
    let definition = d
        .definition
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("instrument definition (family) is required"))?;
    Ok(InstrumentDef {
        instrument_id: d.instrument_id.trim().to_string(),
        name: d.name.trim().to_string(),
        description: d.description.trim().to_string(),
        currency: d.currency.trim().to_string(),
        external_ids: d.external_ids.iter().map(external_from_wire).collect(),
        sub_asset_type: d.sub_asset_type.trim().to_string(),
        region: d.region.trim().to_string(),
        definition: family_from_wire(definition)?,
    })
}

fn external_from_wire(x: &WireExternalId) -> ExternalId {
    ExternalId {
        scheme: x.scheme.trim().to_string(),
        value: x.value.trim().to_string(),
    }
}

fn date_from_wire(d: BrokenDate) -> CivilDate {
    CivilDate {
        year: d.year,
        month: d.month,
        day: d.day,
    }
}

fn require_date(d: Option<BrokenDate>, field: &str) -> Result<CivilDate, Status> {
    d.map(date_from_wire)
        .ok_or_else(|| Status::invalid_argument(format!("{field} is required")))
}

fn family_from_wire(def: &WireDefinition) -> Result<InstrumentFamily, Status> {
    Ok(match def {
        WireDefinition::Deposit(d) => InstrumentFamily::Deposit(DepositDef {
            index: d.index.trim().to_string(),
            tenor: d.tenor.trim().to_string(),
            day_count: d.day_count.trim().to_string(),
            business_day_convention: d.business_day_convention.trim().to_string(),
            calendars: trimmed(&d.calendars),
            spot_lag_days: d.spot_lag_days,
        }),
        WireDefinition::Fra(f) => InstrumentFamily::Fra(FraDef {
            float_index: f.float_index.trim().to_string(),
            start_tenor: f.start_tenor.trim().to_string(),
            end_tenor: f.end_tenor.trim().to_string(),
            accrual_day_count: f.accrual_day_count.trim().to_string(),
            business_day_convention: f.business_day_convention.trim().to_string(),
            calendars: trimmed(&f.calendars),
            spot_lag_days: f.spot_lag_days,
        }),
        WireDefinition::StirFuture(s) => InstrumentFamily::StirFuture(StirFutureDef {
            contract_code: s.contract_code.trim().to_string(),
            reference_start: s.reference_start.trim().to_string(),
            reference_end: s.reference_end.trim().to_string(),
            day_count: s.day_count.trim().to_string(),
            calendars: trimmed(&s.calendars),
            convexity_vol: s.convexity_vol,
            contract_size: s.contract_size,
        }),
        WireDefinition::VanillaIrs(v) => InstrumentFamily::VanillaIrs(VanillaIrsDef {
            tenor: v.tenor.trim().to_string(),
            fixed_frequency: v.fixed_frequency.trim().to_string(),
            fixed_day_count: v.fixed_day_count.trim().to_string(),
            float_index: v.float_index.trim().to_string(),
            float_frequency: v.float_frequency.trim().to_string(),
            float_day_count: v.float_day_count.trim().to_string(),
            business_day_convention: v.business_day_convention.trim().to_string(),
            calendars: trimmed(&v.calendars),
            roll_convention: v.roll_convention.trim().to_string(),
            spot_lag_days: v.spot_lag_days,
        }),
        WireDefinition::Ois(o) => InstrumentFamily::Ois(OisDef {
            tenor: o.tenor.trim().to_string(),
            index: o.index.trim().to_string(),
            fixed_frequency: o.fixed_frequency.trim().to_string(),
            fixed_day_count: o.fixed_day_count.trim().to_string(),
            float_day_count: o.float_day_count.trim().to_string(),
            business_day_convention: o.business_day_convention.trim().to_string(),
            calendars: trimmed(&o.calendars),
            spot_lag_days: o.spot_lag_days,
        }),
        WireDefinition::Bond(b) => InstrumentFamily::Bond(BondDef {
            issuer: b.issuer.trim().to_string(),
            coupon_rate: b.coupon_rate,
            coupon_type: b.coupon_type.trim().to_string(),
            coupon_frequency: b.coupon_frequency.trim().to_string(),
            day_count: b.day_count.trim().to_string(),
            issue_date: b.issue_date.map(date_from_wire),
            dated_date: b.dated_date.map(date_from_wire),
            first_coupon_date: b.first_coupon_date.map(date_from_wire),
            maturity_date: require_date(b.maturity_date, "bond maturity_date")?,
            redemption: b.redemption,
            calendars: trimmed(&b.calendars),
            pool_factor: None,
        }),
        WireDefinition::BondFuture(f) => InstrumentFamily::BondFuture(BondFutureDef {
            contract_code: f.contract_code.trim().to_string(),
            contract_symbol: f.contract_symbol.trim().to_string(),
            underlying_issuer: f.underlying_issuer.trim().to_string(),
            contract_face_value: f.contract_face_value,
            tick_size_points: f.tick_size_points,
            tick_value: f.tick_value,
            notional_coupon_rate: f.notional_coupon_rate,
            deliverable_min_months: f.deliverable_min_months,
            deliverable_max_months: f.deliverable_max_months,
            delivery_month_start: require_date(
                f.delivery_month_start,
                "bond future delivery_month_start",
            )?,
            first_delivery_date: require_date(
                f.first_delivery_date,
                "bond future first_delivery_date",
            )?,
            last_trading_date: require_date(f.last_trading_date, "bond future last_trading_date")?,
            last_delivery_date: require_date(
                f.last_delivery_date,
                "bond future last_delivery_date",
            )?,
            dv01_per_contract_at_notional_yield: f.dv01_per_contract_at_notional_yield,
            calendars: trimmed(&f.calendars),
        }),
    })
}

fn trimmed(v: &[String]) -> Vec<String> {
    v.iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::reference_data::{
        ensure_seed_instruments, government_bond_defs, treasury_future_defs,
    };

    /// Every instrument family the server seeds on boot: the rates curve pillars from
    /// [`ensure_seed_instruments`] PLUS the curated cash bonds and listed Treasury
    /// futures, which are seeded additively on every boot.
    ///
    /// The bond / bond-future families are drawn from the curated universe rather than
    /// from the rates seed on purpose. The rates seed used to carry two sample bonds; they
    /// were retired because no liquidity provider quotes an invented security. Taking the
    /// coverage from the real universe keeps every family exercised here AND keeps the
    /// tradeable set equal to the quotable one.
    fn seeded() -> Vec<InstrumentDef> {
        let mut v = Vec::new();
        let _ = ensure_seed_instruments(&mut v);
        v.extend(government_bond_defs().into_iter().take(2));
        v.extend(treasury_future_defs().into_iter().take(2));
        v
    }

    #[test]
    fn every_seeded_family_round_trips_through_wire() {
        let seeded = seeded();
        for family in ["bond", "bond_future"] {
            assert!(
                seeded.iter().any(|i| i.definition.kind() == family),
                "the round-trip corpus must still cover the {family} family"
            );
        }
        for inst in seeded {
            let wire = instrument_to_wire(&inst);
            let back = instrument_from_wire(&wire).expect("round-trips");
            assert_eq!(inst, back, "family {}", inst.definition.kind());
        }
    }

    #[test]
    fn unset_definition_is_rejected() {
        let wire = InstrumentDefDesc {
            sub_asset_type: String::new(),
            region: String::new(),
            instrument_id: "x".into(),
            name: "X".into(),
            description: String::new(),
            currency: "USD".into(),
            external_ids: Vec::new(),
            definition: None,
        };
        let err = instrument_from_wire(&wire).expect_err("unset family rejected");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }
}
