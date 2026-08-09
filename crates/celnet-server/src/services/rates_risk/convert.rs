//! Wire ↔ domain mapping for the rates portfolio-risk rollup — the linear-rates
//! analogue of [`super::super::risk::convert`].
//!
//! Two directions live here:
//!
//! * **inbound** — a proto [`RatesPosition`] priced against the request's
//!   [`CurveSet`] becomes one additive [`RatesRiskFact`]
//!   ([`fact_from_position`]). The position is priced through the **same**
//!   [`crate::rates_pricing::price_rates`] entry the `PricingService.PriceRates`
//!   edge uses (the OIS math is never re-implemented here); the per-pillar
//!   `key_rate_ladder` doubles are zipped back onto the `CurveSet` pillar tenors so
//!   each ladder bucket maps to its tradeable hedge tenor.
//! * **outbound** — a [`RatesFirmRollup`] becomes an [`AggregateRatesRiskResponse`]
//!   ([`rollup_to_response`]), one [`RatesRiskNode`] per settlement currency in the
//!   rollup's ascending-currency order.

// `tonic::Status` is a large error type; the whole `RiskService` surface carries it
// by value, mirroring `services::risk` (`#![allow(clippy::result_large_err)]`).
#![allow(clippy::result_large_err)]

use celnet_proto::{
    AggregateRatesRiskResponse, CurveSet, KeyRateDv01, RatesInstrument, RatesPosition,
    RatesPriceRequest, RatesRiskNode,
};
use celnet_risk_cube::{BookId, EntityId};
use celnet_risk_fleet::{KeyRateBucket, RatesFactKey, RatesFirmRollup, RatesRiskFact};
use celnet_types::Ccy;
use tonic::Status;

use crate::rates_pricing::price_rates;
use crate::services::error_status::rates_price_error_to_status;

/// The whole-year tenor a pillar labels, or `None` if it uses the month /
/// broken-date arm. The per-tenor key-rate ladder is whole-year-labelled today;
/// generalising it to month/dated buckets is a deferred follow-on, so the
/// federation rejects those pillars rather than mislabel a bucket.
fn whole_year_pillar(pillar: &celnet_proto::OisPillar) -> Option<u32> {
    use celnet_proto::pillar_tenor::Point;
    match pillar.tenor.as_ref()?.point.as_ref()? {
        Point::Years(years) => Some(*years),
        Point::Months(_) | Point::MaturityDate(_) => None,
    }
}

/// Price one [`RatesPosition`] against `curve_set` and build its additive
/// [`RatesRiskFact`]: the `(entity, ccy, book)` cell plus the side-signed
/// PV / PV01 / DV01 and the per-tenor key-rate DV01 ladder.
///
/// The settlement currency is the curve currency (never carried per position); the
/// ladder's tenor labels come from the `CurveSet` pillars the priced
/// `key_rate_ladder` doubles align with (one entry per pillar, in pillar order).
///
/// # Errors
/// * `invalid_argument` if the curve currency is not a valid ISO 4217 code, or the
///   position carries no instrument.
/// * Whatever [`price_rates`] returns (malformed pillars/instrument ⇒
///   `invalid_argument`; a numeric bootstrap failure ⇒ `internal`).
pub fn fact_from_position(
    position: &RatesPosition,
    curve_set: &CurveSet,
) -> Result<RatesRiskFact, Status> {
    let ccy = Ccy::parse(&curve_set.currency).ok_or_else(|| {
        Status::invalid_argument(format!(
            "AggregateRatesRisk: curve currency `{}` is not a valid ISO 4217 code",
            curve_set.currency
        ))
    })?;
    let instrument: RatesInstrument = position.instrument.clone().ok_or_else(|| {
        Status::invalid_argument(format!(
            "AggregateRatesRisk: position {} carries no instrument",
            position.position_id
        ))
    })?;

    // Price through the shared edge entry — never a re-implementation of OIS math.
    // The market is the request-supplied `CurveSet`, so this is a pure calculation.
    let price_req = RatesPriceRequest {
        request_id: position.position_id,
        curve_set: Some(curve_set.clone()),
        instrument: Some(instrument),
        correlation_id: None,
    };
    let priced = price_rates(&price_req).map_err(|e| rates_price_error_to_status(&e))?;

    // Zip the per-pillar DV01 doubles back onto their `CurveSet` pillar tenors so
    // each ladder bucket carries the tradeable hedge tenor it bumps. `price_rates`
    // emits exactly one ladder entry per pillar in pillar order, so the lengths
    // agree by construction; a mismatch would be a pricer contract break.
    let key_rate_ladder: Vec<KeyRateBucket> = curve_set
        .ois_pillars
        .iter()
        .zip(priced.key_rate_ladder.iter())
        .map(|(pillar, &dv01)| {
            let tenor_years = whole_year_pillar(pillar).ok_or_else(|| {
                Status::invalid_argument(
                    "AggregateRatesRisk: per-tenor key-rate risk currently requires whole-year \
                     curve pillars; month and broken-date pillars price correctly but are not \
                     yet labelled in the key-rate ladder",
                )
            })?;
            Ok(KeyRateBucket { tenor_years, dv01 })
        })
        .collect::<Result<_, Status>>()?;

    Ok(RatesRiskFact {
        key: RatesFactKey {
            entity: EntityId(position.entity),
            ccy,
            book: BookId(position.book),
        },
        pv: priced.pv,
        pv01: priced.pv01,
        dv01: priced.dv01,
        key_rate_ladder,
    })
}

/// Map a [`RatesFirmRollup`] to the wire [`AggregateRatesRiskResponse`]: one
/// [`RatesRiskNode`] per settlement currency, in the rollup's ascending-currency
/// order, each carrying the netted scalars and the tenor-bucketed ladder.
#[must_use]
pub fn rollup_to_response(
    rollup: &RatesFirmRollup,
    correlation_id: Option<u64>,
) -> AggregateRatesRiskResponse {
    let nodes = rollup
        .books()
        .iter()
        .map(|book| RatesRiskNode {
            ccy: book.ccy.as_str().to_owned(),
            net_pv: book.net_pv,
            net_pv01: book.net_pv01,
            net_dv01: book.net_dv01,
            key_rate_ladder: book
                .key_rate_ladder
                .iter()
                .map(|bucket| KeyRateDv01 {
                    tenor_years: bucket.tenor_years,
                    dv01: bucket.dv01,
                })
                .collect(),
        })
        .collect();
    AggregateRatesRiskResponse {
        nodes,
        correlation_id,
    }
}
