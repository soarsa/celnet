//! Fixed-income (linear-rates) vocabulary for the SDK — fluent builders that lay
//! a USD-SOFR curve and an OIS onto the wire contract, and a domain result type.
//!
//! Mirrors the option-pricing vocabulary ([`crate::vocab`]): the trader writes
//! intent (`UsdSofrCurve::new(date).pillar(..)`, `Ois::receive_fixed(5, 0.0405)`)
//! and the builders produce the proto [`celnet_proto::CurveSet`] /
//! [`celnet_proto::RatesInstrument`] that [`crate::Client::price_rates`] sends.
//! The returned [`RatesPriced`] is already side-signed in the curve currency.

use celnet_proto::{
    BrokenDate, CurveSet, OisInstrument, OisPillar, RatesInstrument, RatesPricingResult, Side,
    rates_instrument,
};

/// A civil (calendar) date: `year`, `month` 1..=12, `day` 1..=31.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CivilDate {
    /// Gregorian year (e.g. 2026).
    pub year: i32,
    /// Month of year, 1 (January) ..= 12 (December).
    pub month: u32,
    /// Day of month, 1 ..= 31 (validated server-side on resolution).
    pub day: u32,
}

impl CivilDate {
    /// A civil date from `(year, month, day)`.
    #[must_use]
    pub fn new(year: i32, month: u32, day: u32) -> Self {
        Self { year, month, day }
    }

    fn to_wire(self) -> BrokenDate {
        BrokenDate {
            year: self.year,
            month: self.month,
            day: self.day,
        }
    }
}

/// Fluent builder for a self-discounting USD-SOFR curve, from its dated par-OIS
/// pillars. Add pillars in increasing tenor order; the server bootstraps the
/// discount/forward term structure from them.
#[derive(Debug, Clone, PartialEq)]
pub struct UsdSofrCurve {
    reference: CivilDate,
    pillars: Vec<(u32, f64)>,
}

impl UsdSofrCurve {
    /// Start an empty curve anchored at `reference` (the spot date the pillar
    /// schedules roll from).
    #[must_use]
    pub fn new(reference: CivilDate) -> Self {
        Self {
            reference,
            pillars: Vec::new(),
        }
    }

    /// Add one calibrating pillar: the observed par rate (decimal, `0.0405` =
    /// 4.05%) of the spot-starting OIS of `tenor_years` whole years.
    #[must_use]
    pub fn pillar(mut self, tenor_years: u32, par_rate: f64) -> Self {
        self.pillars.push((tenor_years, par_rate));
        self
    }

    pub(crate) fn to_wire(&self) -> CurveSet {
        CurveSet {
            currency: "USD".to_string(),
            reference_date: Some(self.reference.to_wire()),
            ois_pillars: self
                .pillars
                .iter()
                .map(|&(tenor_years, par_rate)| OisPillar {
                    tenor_years,
                    par_rate,
                })
                .collect(),
        }
    }
}

/// The client's directional side of an OIS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OisSide {
    /// Pay the fixed leg (payer swap; long the floating rate).
    PayFixed,
    /// Receive the fixed leg (receiver swap).
    ReceiveFixed,
}

impl OisSide {
    fn to_wire(self) -> i32 {
        match self {
            Self::PayFixed => Side::Buy as i32,
            Self::ReceiveFixed => Side::Sell as i32,
        }
    }
}

/// Fluent builder for an overnight-indexed swap. Defaults to unit notional; set a
/// notional with [`Ois::notional`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ois {
    tenor_years: u32,
    fixed_rate: f64,
    notional: f64,
    side: OisSide,
}

impl Ois {
    /// A pay-fixed OIS of `tenor_years` at `fixed_rate` (decimal).
    #[must_use]
    pub fn pay_fixed(tenor_years: u32, fixed_rate: f64) -> Self {
        Self {
            tenor_years,
            fixed_rate,
            notional: 1.0,
            side: OisSide::PayFixed,
        }
    }

    /// A receive-fixed OIS of `tenor_years` at `fixed_rate` (decimal).
    #[must_use]
    pub fn receive_fixed(tenor_years: u32, fixed_rate: f64) -> Self {
        Self {
            tenor_years,
            fixed_rate,
            notional: 1.0,
            side: OisSide::ReceiveFixed,
        }
    }

    /// Set the notional in the curve currency (must be `> 0`).
    #[must_use]
    pub fn notional(mut self, notional: f64) -> Self {
        self.notional = notional;
        self
    }

    pub(crate) fn to_wire(self) -> RatesInstrument {
        RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                tenor_years: self.tenor_years,
                fixed_rate: self.fixed_rate,
                notional: self.notional,
                side: self.side.to_wire(),
            })),
        }
    }
}

/// A priced linear-rates instrument: present value plus first-order risk, in the
/// curve currency, already side-signed (a payer and a receiver of the same swap
/// report equal-and-opposite values; the par rate is side-independent).
#[derive(Debug, Clone, PartialEq)]
pub struct RatesPriced {
    /// Present value (sign per the instrument side).
    pub pv: f64,
    /// The breakeven (par) fixed rate of the instrument schedule, decimal.
    pub par_rate: f64,
    /// Analytic PV01 (per 1bp of the instrument's own fixed rate), signed.
    pub pv01: f64,
    /// DV01 (per 1bp parallel bump of every curve pillar), signed.
    pub dv01: f64,
    /// The key-rate (bucketed) DV01 ladder, one entry per curve pillar in order.
    pub key_rate_ladder: Vec<f64>,
}

impl RatesPriced {
    pub(crate) fn from_wire(r: RatesPricingResult) -> Self {
        Self {
            pv: r.pv,
            par_rate: r.par_rate,
            pv01: r.pv01,
            dv01: r.dv01,
            key_rate_ladder: r.key_rate_ladder,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve() -> UsdSofrCurve {
        UsdSofrCurve::new(CivilDate::new(2026, 6, 25))
            .pillar(1, 0.0432)
            .pillar(2, 0.0418)
            .pillar(5, 0.0405)
    }

    #[test]
    fn curve_builds_wire_curve_set() {
        let wire = curve().to_wire();
        assert_eq!(wire.currency, "USD");
        let rd = wire.reference_date.unwrap();
        assert_eq!((rd.year, rd.month, rd.day), (2026, 6, 25));
        assert_eq!(wire.ois_pillars.len(), 3);
        assert_eq!(wire.ois_pillars[2].tenor_years, 5);
        assert_eq!(wire.ois_pillars[2].par_rate, 0.0405);
    }

    #[test]
    fn ois_receive_fixed_maps_to_sell() {
        let wire = Ois::receive_fixed(5, 0.0405)
            .notional(100_000_000.0)
            .to_wire();
        let rates_instrument::Instrument::Ois(ois) = wire.instrument.unwrap();
        assert_eq!(ois.tenor_years, 5);
        assert_eq!(ois.notional, 100_000_000.0);
        assert_eq!(ois.side, Side::Sell as i32);
    }

    #[test]
    fn ois_pay_fixed_maps_to_buy() {
        let wire = Ois::pay_fixed(10, 0.0415).to_wire();
        let rates_instrument::Instrument::Ois(ois) = wire.instrument.unwrap();
        assert_eq!(ois.side, Side::Buy as i32);
        assert_eq!(ois.notional, 1.0); // default unit notional
    }

    #[test]
    fn priced_round_trips_from_wire() {
        let wire = RatesPricingResult {
            pv: -1234.5,
            par_rate: 0.0405,
            pv01: 50.0,
            dv01: 49.5,
            key_rate_ladder: vec![10.0, 15.0, 24.5],
        };
        let priced = RatesPriced::from_wire(wire);
        assert_eq!(priced.pv, -1234.5);
        assert_eq!(priced.key_rate_ladder, vec![10.0, 15.0, 24.5]);
    }
}
