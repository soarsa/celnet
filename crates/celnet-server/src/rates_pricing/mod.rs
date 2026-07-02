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
    BrokenDate, CurveSet, OisInstrument, OisPillar, PillarTenor, RatesPriceRequest,
    RatesPricingResult, Side, pillar_tenor, rates_instrument,
};
use celnet_rates::{
    AccrualBasis, BootstrapError, OisQuote, OisSchedule, ScheduleError, bootstrap_ois,
    ois_par_rate, ois_risk, usd_ois_schedule_for_months, usd_ois_schedule_to_maturity,
    usd_sofr_ois_schedule,
};
use celnet_types::Rate;
use time::{Date, Month};

mod contract;
pub use contract::{BondPriced, price_bond_via_contract, price_rates_via_contract};

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
    /// A pillar carried no `tenor` (the `PillarTenor` oneof was unset).
    MissingPillarTenor,
    /// A pillar's `maturity_date` arm was not a real calendar date.
    InvalidPillarDate,
    /// The pillar maturities were not strictly increasing (duplicate or out of order).
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
            Self::ZeroTenor => f.write_str("pillar tenor must be >= 1"),
            Self::MissingPillarTenor => f.write_str("a pillar carried no `tenor`"),
            Self::InvalidPillarDate => {
                f.write_str("a pillar `maturity_date` is not a real calendar date")
            }
            Self::NonIncreasingPillars => {
                f.write_str("`ois_pillars` maturities must be strictly increasing")
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

/// Parse a wire [`BrokenDate`] to a real [`time::Date`], or `None` if it is not a
/// real civil date. Callers map `None` to the context-appropriate error
/// (reference date vs. pillar maturity).
fn parse_broken_date(d: &BrokenDate) -> Option<Date> {
    let month = u8::try_from(d.month)
        .ok()
        .and_then(|m| Month::try_from(m).ok())?;
    let day = u8::try_from(d.day).ok()?;
    Date::from_calendar_date(d.year, month, day).ok()
}

/// Resolve the `CurveSet` reference (spot-anchor) date, or fail.
fn resolve_date(d: &BrokenDate) -> Result<Date, RatesPriceError> {
    parse_broken_date(d).ok_or(RatesPriceError::InvalidReferenceDate)
}

/// A whole-year [`PillarTenor`] — the canonical liquid-grid arm. Convenience for
/// the static ladder and any caller building a regular-tenor curve.
#[must_use]
pub fn years_pillar(years: u32) -> PillarTenor {
    PillarTenor {
        point: Some(pillar_tenor::Point::Years(years)),
    }
}

/// Build the spot-starting USD-SOFR schedule that prices one curve pillar from its
/// [`PillarTenor`] arm: a whole-year tenor, a month tenor, or an explicit
/// broken-date maturity. All arms use the ACT/360 fixed-leg accrual that the
/// par-OIS quote convention assumes.
fn pillar_schedule(tenor: &PillarTenor, reference: Date) -> Result<OisSchedule, RatesPriceError> {
    match tenor
        .point
        .as_ref()
        .ok_or(RatesPriceError::MissingPillarTenor)?
    {
        pillar_tenor::Point::Years(0) | pillar_tenor::Point::Months(0) => {
            Err(RatesPriceError::ZeroTenor)
        }
        pillar_tenor::Point::Years(years) => Ok(usd_sofr_ois_schedule(reference, *years)?),
        pillar_tenor::Point::Months(months) => Ok(usd_ois_schedule_for_months(
            reference,
            *months,
            AccrualBasis::Act360,
        )?),
        pillar_tenor::Point::MaturityDate(d) => {
            let maturity = parse_broken_date(d).ok_or(RatesPriceError::InvalidPillarDate)?;
            Ok(usd_ois_schedule_to_maturity(
                reference,
                maturity,
                AccrualBasis::Act360,
            )?)
        }
    }
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
    // Order pillars by their final ACT/365F payment time from spot — strictly
    // increasing iff the resolved maturities are, regardless of which arm
    // (years / months / broken date) located each pillar.
    let mut prev_pay = 0.0_f64;
    for pillar in &curve.ois_pillars {
        let tenor = pillar
            .tenor
            .as_ref()
            .ok_or(RatesPriceError::MissingPillarTenor)?;
        let schedule = pillar_schedule(tenor, reference)?;
        let last_pay = schedule
            .periods()
            .last()
            .map(|p| p.pay.0)
            .ok_or(RatesPriceError::ZeroTenor)?;
        if last_pay <= prev_pay {
            return Err(RatesPriceError::NonIncreasingPillars);
        }
        prev_pay = last_pay;
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

/// The P0 static USD-SOFR par-OIS pillar ladder `(tenor_years, par_rate)`.
///
/// This is the market the FIX rates edge quotes against until a live SOFR feed is
/// wired (operator question Q6 — test-environment data-provider access — is
/// deferred). It is a *real* calibrating market, not a stub: it bootstraps a real
/// self-discounting curve and produces real par rates and risk. When the feed
/// lands it simply replaces this table; nothing else changes.
const P0_USD_SOFR_PILLARS: &[(u32, f64)] = &[
    (1, 0.0432),
    (2, 0.0418),
    (3, 0.0409),
    (5, 0.0405),
    (7, 0.0408),
    (10, 0.0415),
    (15, 0.0421),
    (20, 0.0424),
    (30, 0.0423),
];

/// The reference (spot-anchor) date of the P0 static curve.
const P0_REFERENCE: BrokenDate = BrokenDate {
    year: 2026,
    month: 6,
    day: 25,
};

/// The P0 static USD-SOFR [`CurveSet`] the FIX rates edge prices against (see
/// [`P0_USD_SOFR_PILLARS`]). A real calibrating market pending a live SOFR feed.
#[must_use]
pub fn default_usd_sofr_curve_set() -> CurveSet {
    CurveSet {
        currency: SUPPORTED_CURRENCY.to_string(),
        reference_date: Some(P0_REFERENCE),
        ois_pillars: P0_USD_SOFR_PILLARS
            .iter()
            .map(|&(years, par_rate)| OisPillar {
                tenor: Some(years_pillar(years)),
                par_rate,
            })
            .collect(),
    }
}

/// The fair (par) fixed rate of a spot-starting USD-SOFR OIS of `tenor_years`,
/// bootstrapped from `curve` — the number the FIX rates edge centres a two-way
/// RFQ market on. Side- and notional-independent.
///
/// # Errors
///
/// Returns [`RatesPriceError`] for an invalid curve, an unsupported currency, or a
/// numeric schedule/bootstrap failure.
pub fn par_rate_for(curve: &CurveSet, tenor_years: u32) -> Result<f64, RatesPriceError> {
    if tenor_years == 0 {
        return Err(RatesPriceError::ZeroTenor);
    }
    let reference_date = curve
        .reference_date
        .as_ref()
        .ok_or(RatesPriceError::MissingReferenceDate)?;
    let reference = resolve_date(reference_date)?;
    let quotes = build_quotes(curve, reference)?;
    let schedule = usd_sofr_ois_schedule(reference, tenor_years)?;
    let base = bootstrap_ois(&quotes)?;
    Ok(ois_par_rate(&base, &schedule).0)
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

    fn pillar(years: u32, par_rate: f64) -> OisPillar {
        OisPillar {
            tenor: Some(years_pillar(years)),
            par_rate,
        }
    }

    fn pillars() -> Vec<OisPillar> {
        vec![
            pillar(1, 0.0420),
            pillar(2, 0.0410),
            pillar(5, 0.0405),
            pillar(10, 0.0415),
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
        req.curve_set.as_mut().unwrap().ois_pillars = vec![pillar(2, 0.041), pillar(2, 0.041)];
        assert_eq!(
            price_rates(&req),
            Err(RatesPriceError::NonIncreasingPillars)
        );
    }

    #[test]
    fn month_tenor_of_whole_year_matches_year_pillar() {
        // A 24-month pillar resolves to the same schedule (hence the same bootstrap
        // and par rate) as the 2-year pillar — the dated/tenor parity guarantee.
        let mut req = request(2, 0.04, 100.0, Side::Sell);
        req.curve_set.as_mut().unwrap().ois_pillars = vec![pillar(1, 0.0420), pillar(2, 0.0410)];
        let by_years = price_rates(&req).expect("years price");

        req.curve_set.as_mut().unwrap().ois_pillars = vec![
            OisPillar {
                tenor: Some(PillarTenor {
                    point: Some(pillar_tenor::Point::Months(12)),
                }),
                par_rate: 0.0420,
            },
            OisPillar {
                tenor: Some(PillarTenor {
                    point: Some(pillar_tenor::Point::Months(24)),
                }),
                par_rate: 0.0410,
            },
        ];
        let by_months = price_rates(&req).expect("months price");
        assert_eq!(by_years.par_rate, by_months.par_rate);
        assert_eq!(by_years.pv, by_months.pv);
    }

    #[test]
    fn broken_date_pillar_prices() {
        // An explicit broken-date maturity (≈18M) calibrates and prices without
        // error, exercising the dated arm end to end.
        let mut req = request(1, 0.04, 100.0, Side::Sell);
        req.curve_set.as_mut().unwrap().ois_pillars = vec![
            pillar(1, 0.0420),
            OisPillar {
                tenor: Some(PillarTenor {
                    point: Some(pillar_tenor::Point::MaturityDate(BrokenDate {
                        year: 2027,
                        month: 12,
                        day: 27,
                    })),
                }),
                par_rate: 0.0412,
            },
        ];
        let priced = price_rates(&req).expect("broken-date price");
        assert!(priced.par_rate.is_finite() && priced.par_rate > 0.0);
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

    #[test]
    fn default_curve_reprices_its_own_pillar() {
        // The bootstrap reprices each calibrating pillar at par, so the par rate
        // of a fresh schedule at a pillar tenor returns the quoted pillar rate.
        let curve = default_usd_sofr_curve_set();
        let par5 = par_rate_for(&curve, 5).unwrap();
        assert!((par5 - 0.0405).abs() < 1e-6, "5y par {par5}");
    }

    #[test]
    fn par_rate_for_agrees_with_price_rates() {
        let curve = default_usd_sofr_curve_set();
        let par = par_rate_for(&curve, 7).unwrap();
        let req = RatesPriceRequest {
            request_id: 1,
            curve_set: Some(curve),
            instrument: Some(RatesInstrument {
                instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                    tenor_years: 7,
                    fixed_rate: 0.0,
                    notional: 1.0,
                    side: Side::Sell as i32,
                })),
            }),
            correlation_id: None,
        };
        assert_eq!(price_rates(&req).unwrap().par_rate, par);
    }
}
