//! Linear-rates pricing for the `PricingService::PriceRates` edge RPC.
//!
//! This module is the faithful, allocation-light bridge from the wire contract
//! ([`celnet_proto::RatesPriceRequest`]) to the [`celnet_rates`] term-structure
//! engine and back ([`celnet_proto::RatesPricingResult`]). It is deliberately a
//! **pure function** — no IO, no live-market read — because the caller supplies
//! the market explicitly as a [`celnet_proto::CurveSet`] (exactly as `Price`
//! takes an explicit `MarketContext`). That keeps the gRPC handler in
//! [`crate::services::pricing`] a thin `Status`-wrapping shell and lets the whole
//! conversion + numeric path be unit-tested without a server.
//!
//! ## What it does
//!
//! 1. Validates the curve set (USD only for the P0 arm; non-empty, strictly
//!    increasing pillar tenors) and rebuilds each pillar's spot-starting
//!    USD-SOFR schedule from the curve reference date, so the wire only carries
//!    the economic `(tenor, par_rate)` quote.
//! 2. Bootstraps the self-discounting curve (sequential, acyclic) and prices the
//!    requested [`celnet_proto::OisInstrument`] with [`celnet_rates::ois_risk`],
//!    which returns the **receive-fixed** PV / PV01 / DV01 / key-rate ladder.
//! 3. Applies the client `side` sign (`SIDE_SELL` = receive fixed = `+1`;
//!    `SIDE_BUY` = pay fixed = `-1`) to every signed measure, so a payer and a
//!    receiver of the same swap report equal-and-opposite risk. The par rate is
//!    side-independent and never sign-flipped.
//!
//! Every failure mode is a typed [`RatesPriceError`]; the handler maps it to a
//! `tonic::Status` (invalid-argument for malformed input, internal for a
//! bootstrap failure on otherwise-valid input).

use celnet_proto::{
    BrokenDate, CurveSet, OisInstrument, RatesPriceRequest, RatesPricingResult, Side,
    rates_instrument,
};
use celnet_rates::{
    BootstrapError, OisQuote, ScheduleError, bootstrap_ois, ois_par_rate, ois_risk,
    usd_sofr_ois_schedule,
};
use celnet_types::Rate;
use time::{Date, Month};

/// The ISO 4217 code of the only currency the P0 rates arm supports.
const SUPPORTED_CURRENCY: &str = "USD";

/// A typed failure of [`price_rates`]. Malformed-input variants map to
/// `Status::invalid_argument`; [`RatesPriceError::Bootstrap`] (a numeric failure
/// on otherwise-valid input) maps to `Status::internal`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RatesPriceError {
    /// The request carried no `curve_set`.
    MissingCurveSet,
    /// The curve set carried no `reference_date`.
    MissingReferenceDate,
    /// The `reference_date` is not a real calendar date.
    InvalidReferenceDate,
    /// The curve currency is not the supported P0 currency (USD).
    UnsupportedCurrency(String),
    /// The curve set carried no OIS pillars.
    EmptyPillars,
    /// A pillar (curve or instrument) had a zero tenor; tenors must be `>= 1`.
    ZeroTenor,
    /// The pillar tenors were not strictly increasing (duplicate or out of order).
    NonIncreasingPillars,
    /// The request carried no `instrument`, or an unset oneof arm.
    MissingInstrument,
    /// The instrument notional was not strictly positive.
    NonPositiveNotional,
    /// `SIDE_TWO_WAY` (or an unknown side code) was supplied for an outright price.
    InvalidSide,
    /// Building a spot-starting USD-SOFR schedule failed.
    Schedule(ScheduleError),
    /// Bootstrapping the discount curve failed (a numeric failure).
    Bootstrap(BootstrapError),
}

impl core::fmt::Display for RatesPriceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingCurveSet => f.write_str("missing `curve_set`"),
            Self::MissingReferenceDate => f.write_str("missing `curve_set.reference_date`"),
            Self::InvalidReferenceDate => {
                f.write_str("`reference_date` is not a real calendar date")
            }
            Self::UnsupportedCurrency(c) => {
                write!(
                    f,
                    "unsupported currency `{c}` (only USD is priced in the P0 arm)"
                )
            }
            Self::EmptyPillars => f.write_str("`curve_set.ois_pillars` is empty"),
            Self::ZeroTenor => f.write_str("tenor_years must be >= 1"),
            Self::NonIncreasingPillars => {
                f.write_str("`ois_pillars` tenors must be strictly increasing")
            }
            Self::MissingInstrument => f.write_str("missing `instrument`"),
            Self::NonPositiveNotional => f.write_str("notional must be > 0"),
            Self::InvalidSide => {
                f.write_str("side must be SIDE_BUY (pay fixed) or SIDE_SELL (receive fixed)")
            }
            Self::Schedule(e) => write!(f, "schedule: {e}"),
            Self::Bootstrap(e) => write!(f, "bootstrap: {e}"),
        }
    }
}

impl std::error::Error for RatesPriceError {}

impl From<ScheduleError> for RatesPriceError {
    fn from(e: ScheduleError) -> Self {
        Self::Schedule(e)
    }
}

impl From<BootstrapError> for RatesPriceError {
    fn from(e: BootstrapError) -> Self {
        Self::Bootstrap(e)
    }
}

/// Resolve a wire [`BrokenDate`] to a real [`time::Date`], or fail.
fn resolve_date(d: &BrokenDate) -> Result<Date, RatesPriceError> {
    let month = u8::try_from(d.month)
        .ok()
        .and_then(|m| Month::try_from(m).ok())
        .ok_or(RatesPriceError::InvalidReferenceDate)?;
    let day = u8::try_from(d.day).map_err(|_| RatesPriceError::InvalidReferenceDate)?;
    Date::from_calendar_date(d.year, month, day).map_err(|_| RatesPriceError::InvalidReferenceDate)
}

/// The receive-fixed sign for a client `side`: `SIDE_SELL` receives fixed (the
/// engine's native convention, `+1`); `SIDE_BUY` pays fixed (`-1`).
fn side_sign(side: Side) -> Result<f64, RatesPriceError> {
    match side {
        Side::Sell => Ok(1.0),
        Side::Buy => Ok(-1.0),
        Side::TwoWay => Err(RatesPriceError::InvalidSide),
    }
}

/// Rebuild the calibrating OIS quotes from a wire [`CurveSet`]: one spot-starting
/// USD-SOFR schedule per pillar, paired with its observed par rate. Validates the
/// currency, non-emptiness, and strictly-increasing tenors.
fn build_quotes(curve: &CurveSet, reference: Date) -> Result<Vec<OisQuote>, RatesPriceError> {
    if !curve.currency.eq_ignore_ascii_case(SUPPORTED_CURRENCY) {
        return Err(RatesPriceError::UnsupportedCurrency(curve.currency.clone()));
    }
    if curve.ois_pillars.is_empty() {
        return Err(RatesPriceError::EmptyPillars);
    }

    let mut quotes = Vec::with_capacity(curve.ois_pillars.len());
    let mut prev_tenor = 0u32;
    for pillar in &curve.ois_pillars {
        if pillar.tenor_years == 0 {
            return Err(RatesPriceError::ZeroTenor);
        }
        if pillar.tenor_years <= prev_tenor {
            return Err(RatesPriceError::NonIncreasingPillars);
        }
        prev_tenor = pillar.tenor_years;
        let schedule = usd_sofr_ois_schedule(reference, pillar.tenor_years)?;
        quotes.push(OisQuote {
            schedule,
            par_rate: Rate(pillar.par_rate),
        });
    }
    Ok(quotes)
}

/// Price a single [`OisInstrument`] against the calibrating `quotes`, returning
/// the side-signed [`RatesPricingResult`].
fn price_ois(
    ois: &OisInstrument,
    quotes: &[OisQuote],
    reference: Date,
) -> Result<RatesPricingResult, RatesPriceError> {
    if ois.tenor_years == 0 {
        return Err(RatesPriceError::ZeroTenor);
    }
    if ois.notional <= 0.0 {
        return Err(RatesPriceError::NonPositiveNotional);
    }
    let side = Side::try_from(ois.side).map_err(|_| RatesPriceError::InvalidSide)?;
    let sign = side_sign(side)?;

    let schedule = usd_sofr_ois_schedule(reference, ois.tenor_years)?;

    // Receive-fixed risk from the engine, then sign to the client's perspective.
    let risk = ois_risk(quotes, &schedule, Rate(ois.fixed_rate), ois.notional)?;
    // Par (fair fixed) rate is side-independent — bootstrap once for it.
    let base = bootstrap_ois(quotes)?;
    let par = ois_par_rate(&base, &schedule);

    Ok(RatesPricingResult {
        pv: sign * risk.pv,
        par_rate: par.0,
        pv01: sign * risk.pv01,
        dv01: sign * risk.dv01,
        key_rate_ladder: risk.key_rate.iter().map(|k| sign * k).collect(),
    })
}

/// Price the [`RatesPriceRequest`]'s instrument against its curve set.
///
/// Pure and IO-free: the market is the request-supplied [`CurveSet`]. Returns the
/// side-signed [`RatesPricingResult`]; every failure is a typed [`RatesPriceError`].
///
/// # Errors
///
/// Returns [`RatesPriceError`] for a missing/invalid curve set, an unsupported
/// currency, malformed pillars, a missing/invalid instrument, or a numeric
/// schedule/bootstrap failure.
pub fn price_rates(req: &RatesPriceRequest) -> Result<RatesPricingResult, RatesPriceError> {
    let curve = req
        .curve_set
        .as_ref()
        .ok_or(RatesPriceError::MissingCurveSet)?;
    let reference_date = curve
        .reference_date
        .as_ref()
        .ok_or(RatesPriceError::MissingReferenceDate)?;
    let reference = resolve_date(reference_date)?;

    let quotes = build_quotes(curve, reference)?;

    let instrument = req
        .instrument
        .as_ref()
        .and_then(|i| i.instrument.as_ref())
        .ok_or(RatesPriceError::MissingInstrument)?;

    match instrument {
        rates_instrument::Instrument::Ois(ois) => price_ois(ois, &quotes, reference),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{CurveSet, OisInstrument, OisPillar, RatesInstrument};

    /// 2026-06-25 reference, a representative USD-SOFR par-OIS ladder.
    fn reference() -> BrokenDate {
        BrokenDate {
            year: 2026,
            month: 6,
            day: 25,
        }
    }

    fn pillars() -> Vec<OisPillar> {
        vec![
            OisPillar {
                tenor_years: 1,
                par_rate: 0.0420,
            },
            OisPillar {
                tenor_years: 2,
                par_rate: 0.0410,
            },
            OisPillar {
                tenor_years: 5,
                par_rate: 0.0405,
            },
            OisPillar {
                tenor_years: 10,
                par_rate: 0.0415,
            },
        ]
    }

    fn curve_set() -> CurveSet {
        CurveSet {
            currency: "USD".to_string(),
            reference_date: Some(reference()),
            ois_pillars: pillars(),
        }
    }

    fn request(tenor: u32, fixed_rate: f64, notional: f64, side: Side) -> RatesPriceRequest {
        RatesPriceRequest {
            request_id: 1,
            curve_set: Some(curve_set()),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                    tenor_years: tenor,
                    fixed_rate,
                    notional,
                    side: side as i32,
                })),
            }),
            correlation_id: None,
        }
    }

    #[test]
    fn server_path_reproduces_engine_exactly() {
        // The wire conversion must be byte-identical to a direct engine call:
        // price a 5y receive-fixed OIS @ 4.00% on 100mm both ways and compare.
        let req = request(5, 0.04, 100_000_000.0, Side::Sell);
        let got = price_rates(&req).unwrap();

        let reference = resolve_date(&reference()).unwrap();
        let quotes = build_quotes(&curve_set(), reference).unwrap();
        let schedule = usd_sofr_ois_schedule(reference, 5).unwrap();
        let risk = ois_risk(&quotes, &schedule, Rate(0.04), 100_000_000.0).unwrap();

        assert_eq!(got.pv, risk.pv);
        assert_eq!(got.pv01, risk.pv01);
        assert_eq!(got.dv01, risk.dv01);
        assert_eq!(got.key_rate_ladder, risk.key_rate);
        assert_eq!(got.key_rate_ladder.len(), pillars().len());
    }

    #[test]
    fn par_rate_priced_at_par_gives_zero_pv() {
        // Pricing at the engine's own par rate must yield ~0 PV (the defining
        // identity of a par swap).
        let probe = price_rates(&request(5, 0.04, 100_000_000.0, Side::Sell)).unwrap();
        let at_par = price_rates(&request(5, probe.par_rate, 100_000_000.0, Side::Sell)).unwrap();
        assert!(at_par.pv.abs() < 1e-6, "par PV not zero: {}", at_par.pv);
    }

    #[test]
    fn payer_is_exactly_opposite_receiver() {
        let recv = price_rates(&request(7, 0.041, 50_000_000.0, Side::Sell)).unwrap();
        let pay = price_rates(&request(7, 0.041, 50_000_000.0, Side::Buy)).unwrap();
        assert_eq!(pay.pv, -recv.pv);
        assert_eq!(pay.pv01, -recv.pv01);
        assert_eq!(pay.dv01, -recv.dv01);
        assert_eq!(pay.par_rate, recv.par_rate); // par is side-independent
        for (p, r) in pay.key_rate_ladder.iter().zip(&recv.key_rate_ladder) {
            assert_eq!(*p, -r);
        }
    }

    #[test]
    fn key_rate_ladder_sums_to_dv01() {
        let res = price_rates(&request(7, 0.041, 100_000_000.0, Side::Sell)).unwrap();
        let summed: f64 = res.key_rate_ladder.iter().sum();
        // First-order additive; residual is curve cross-gamma (~0.04%).
        assert!(
            (summed - res.dv01).abs() / res.dv01.abs() < 5e-3,
            "ladder sum {summed} vs dv01 {}",
            res.dv01
        );
    }

    #[test]
    fn rejects_non_usd_currency() {
        let mut req = request(5, 0.04, 100.0, Side::Sell);
        req.curve_set.as_mut().unwrap().currency = "EUR".to_string();
        assert_eq!(
            price_rates(&req),
            Err(RatesPriceError::UnsupportedCurrency("EUR".to_string()))
        );
    }

    #[test]
    fn rejects_two_way_side() {
        let req = request(5, 0.04, 100.0, Side::TwoWay);
        assert_eq!(price_rates(&req), Err(RatesPriceError::InvalidSide));
    }

    #[test]
    fn rejects_non_positive_notional() {
        let req = request(5, 0.04, 0.0, Side::Sell);
        assert_eq!(price_rates(&req), Err(RatesPriceError::NonPositiveNotional));
    }

    #[test]
    fn rejects_non_increasing_pillars() {
        let mut req = request(5, 0.04, 100.0, Side::Sell);
        req.curve_set.as_mut().unwrap().ois_pillars = vec![
            OisPillar {
                tenor_years: 2,
                par_rate: 0.041,
            },
            OisPillar {
                tenor_years: 2,
                par_rate: 0.041,
            },
        ];
        assert_eq!(
            price_rates(&req),
            Err(RatesPriceError::NonIncreasingPillars)
        );
    }

    #[test]
    fn rejects_missing_curve_set() {
        let mut req = request(5, 0.04, 100.0, Side::Sell);
        req.curve_set = None;
        assert_eq!(price_rates(&req), Err(RatesPriceError::MissingCurveSet));
    }

    #[test]
    fn rejects_invalid_reference_date() {
        let mut req = request(5, 0.04, 100.0, Side::Sell);
        req.curve_set.as_mut().unwrap().reference_date = Some(BrokenDate {
            year: 2026,
            month: 13,
            day: 1,
        });
        assert_eq!(
            price_rates(&req),
            Err(RatesPriceError::InvalidReferenceDate)
        );
    }
}
