//! Fixed-income (linear-rates) vocabulary for the SDK — fluent builders that lay
//! a USD-SOFR curve and an OIS onto the wire contract, and a domain result type.
//!
//! Mirrors the option-pricing vocabulary ([`crate::vocab`]): the trader writes
//! intent (`UsdSofrCurve::new(date).pillar(..)`, `Ois::receive_fixed(5, 0.0405)`)
//! and the builders produce the proto [`celnet_proto::CurveSet`] /
//! [`celnet_proto::RatesInstrument`] that [`crate::Client::price_rates`] sends.
//! The returned [`RatesPriced`] is already side-signed in the curve currency.

use celnet_proto::convert::WireError;
use celnet_proto::{
    AggregateRatesRiskRequest, AggregateRatesRiskResponse, BookRatesPositionRequest, BrokenDate,
    CurveSet, KeyRateDv01 as WireKeyRateDv01, ListRatesPositionsRequest, OisInstrument, OisPillar,
    PillarTenor, RatesInstrument, RatesPosition as WireRatesPosition, RatesPricingResult,
    RatesRiskNode as WireRatesRiskNode, RatesRiskScope as WireRatesRiskScope, Side, pillar_tenor,
    rates_instrument,
};

use crate::error::{ClientError, ClientResult};
use crate::risk::{Entitlements, principal_or_grant_all};

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

/// Where a curve pillar matures: a whole-year tenor, a month tenor, or an explicit
/// broken-date maturity. Mirrors the wire [`celnet_proto::PillarTenor`] oneof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillarPoint {
    /// Whole-year tenor from spot (e.g. 1, 2, 5, 10).
    Years(u32),
    /// Month tenor from spot (e.g. 3, 18, 30) — sub-/broken-year pillars.
    Months(u32),
    /// Explicit odd-dated ("broken date") maturity.
    Maturity(CivilDate),
}

impl PillarPoint {
    fn to_wire(self) -> PillarTenor {
        let point = match self {
            PillarPoint::Years(years) => pillar_tenor::Point::Years(years),
            PillarPoint::Months(months) => pillar_tenor::Point::Months(months),
            PillarPoint::Maturity(date) => pillar_tenor::Point::MaturityDate(date.to_wire()),
        };
        PillarTenor { point: Some(point) }
    }
}

/// Fluent builder for a self-discounting USD-SOFR curve, from its dated par-OIS
/// pillars. Add pillars in increasing tenor order; the server bootstraps the
/// discount/forward term structure from them.
#[derive(Debug, Clone, PartialEq)]
pub struct UsdSofrCurve {
    reference: CivilDate,
    pillars: Vec<(PillarPoint, f64)>,
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
    pub fn pillar(self, tenor_years: u32, par_rate: f64) -> Self {
        self.pillar_at(PillarPoint::Years(tenor_years), par_rate)
    }

    /// Add a pillar at a month tenor (e.g. 3, 18, 30) — a sub-year or broken-year
    /// point the whole-year grid cannot name.
    #[must_use]
    pub fn pillar_months(self, tenor_months: u32, par_rate: f64) -> Self {
        self.pillar_at(PillarPoint::Months(tenor_months), par_rate)
    }

    /// Add a pillar at an explicit broken-date `maturity` (e.g. an IMM or turn date).
    #[must_use]
    pub fn pillar_on(self, maturity: CivilDate, par_rate: f64) -> Self {
        self.pillar_at(PillarPoint::Maturity(maturity), par_rate)
    }

    /// Add one calibrating pillar at an arbitrary [`PillarPoint`].
    #[must_use]
    pub fn pillar_at(mut self, point: PillarPoint, par_rate: f64) -> Self {
        self.pillars.push((point, par_rate));
        self
    }

    pub(crate) fn to_wire(&self) -> CurveSet {
        CurveSet {
            currency: "USD".to_string(),
            reference_date: Some(self.reference.to_wire()),
            ois_pillars: self
                .pillars
                .iter()
                .map(|&(point, par_rate)| OisPillar {
                    tenor: Some(point.to_wire()),
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

    /// Decode the wire `Side` tag an OIS instrument carries: `SIDE_BUY` pays fixed,
    /// `SIDE_SELL` receives fixed. `SIDE_TWO_WAY` is never a firm OIS direction (the
    /// server rejects it at price time), so it decodes as a typed wire error rather
    /// than a silent default.
    pub(crate) fn from_wire(tag: i32) -> ClientResult<Self> {
        match Side::try_from(tag) {
            Ok(Side::Buy) => Ok(Self::PayFixed),
            Ok(Side::Sell) => Ok(Self::ReceiveFixed),
            _ => Err(ClientError::Wire(WireError::UnknownEnum {
                kind: "OisSide",
                tag,
            })),
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

    /// The swap tenor in whole years from spot.
    #[must_use]
    pub fn tenor_years(&self) -> u32 {
        self.tenor_years
    }

    /// The fixed-leg rate as a decimal (`0.041` = 4.10%).
    #[must_use]
    pub fn fixed_rate(&self) -> f64 {
        self.fixed_rate
    }

    /// The notional in the curve currency (always positive; direction is [`Ois::side`]).
    #[must_use]
    pub fn notional_amount(&self) -> f64 {
        self.notional
    }

    /// The client's directional side (pay-fixed / receive-fixed).
    #[must_use]
    pub fn side(&self) -> OisSide {
        self.side
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

    /// Decode the OIS arm of a wire [`RatesInstrument`] into the typed builder value.
    /// The linear-rates oneof carries exactly one arm today (OIS); a message with no
    /// arm — or an arm the SDK cannot type — is a contract violation surfaced as a
    /// typed error, never a silent default.
    pub(crate) fn from_wire(w: &RatesInstrument) -> ClientResult<Self> {
        match w.instrument.as_ref() {
            Some(rates_instrument::Instrument::Ois(ois)) => Ok(Self {
                tenor_years: ois.tenor_years,
                fixed_rate: ois.fixed_rate,
                notional: ois.notional,
                side: OisSide::from_wire(ois.side)?,
            }),
            None => Err(ClientError::MissingField("RatesInstrument.instrument")),
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

// ===========================================================================
// firm-scale linear-rates (fixed-income) portfolio risk — the typed SDK face of
// the `RiskService` FI-risk contract (`AggregateRatesRisk` / `BookRatesPosition` /
// `ListRatesPositions`). The rates analogue of the options hierarchical-risk
// surface in [`crate::risk`]: a client submits / books a rates position and reads
// the server's netted firm rollup — it never prices or sums itself (api-first
// parity). Rates risk is *purely additive* per settlement currency (PV / PV01 /
// DV01 / key-rate ladder sum over a netting set; currencies never cross-net).
// ===========================================================================

/// One rates position in the firm rates book: its identity, its `(entity, book)`
/// cell, and the linear-rates instrument it holds. The linear-rates analogue of
/// [`crate::RiskPosition`]. The settlement currency is taken from the priced
/// [`UsdSofrCurve`], never carried per position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RatesPosition {
    /// The position identity (one current fact per id). `0` on a fresh booking ⇒
    /// the server assigns an id; a non-zero id upserts that position.
    pub position_id: u64,
    /// The legal entity (regulatory-capital unit) the position books into — the
    /// primary netting/partition sub-key.
    pub entity: u32,
    /// The netting book the position sits in (netting/attribution; not a partition
    /// key, so a currency's whole cross-book risk stays co-resident).
    pub book: u32,
    /// The linear-rates instrument the position holds (the USD-SOFR OIS arm).
    pub instrument: Ois,
}

impl RatesPosition {
    /// A fresh rates position in the `(entity, book)` cell holding `instrument`,
    /// with `position_id == 0` so the server assigns a stable id on booking.
    #[must_use]
    pub fn new(entity: u32, book: u32, instrument: Ois) -> Self {
        Self {
            position_id: 0,
            entity,
            book,
            instrument,
        }
    }

    /// Pin an explicit `position_id` (an upsert of that position on booking, or the
    /// identity a position already carries when submitted inline to an aggregate).
    #[must_use]
    pub fn with_id(mut self, position_id: u64) -> Self {
        self.position_id = position_id;
        self
    }

    pub(crate) fn to_wire(self) -> WireRatesPosition {
        WireRatesPosition {
            position_id: self.position_id,
            entity: self.entity,
            book: self.book,
            instrument: Some(self.instrument.to_wire()),
        }
    }

    pub(crate) fn from_wire(w: &WireRatesPosition) -> ClientResult<Self> {
        let instrument = w
            .instrument
            .as_ref()
            .ok_or(ClientError::MissingField("RatesPosition.instrument"))
            .and_then(Ois::from_wire)?;
        Ok(Self {
            position_id: w.position_id,
            entity: w.entity,
            book: w.book,
            instrument,
        })
    }
}

/// A scope filter narrowing a rates rollup / listing to one `(entity, book, ccy)`
/// subtree before the roll-up — the rates-native analogue of [`crate::Scope`], in
/// the `(entity, ccy, book)` key space. Each filter is presence-tracked: an absent
/// field does not constrain, and a position is included iff every present filter
/// matches it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RatesRiskScope {
    entity: Option<u32>,
    book: Option<u32>,
    ccy: Option<String>,
}

impl RatesRiskScope {
    /// The unconstrained scope (the whole submitted / booked book).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pin the rollup to one legal entity.
    #[must_use]
    pub fn entity(mut self, entity: u32) -> Self {
        self.entity = Some(entity);
        self
    }

    /// Pin the rollup to one netting book.
    #[must_use]
    pub fn book(mut self, book: u32) -> Self {
        self.book = Some(book);
        self
    }

    /// Pin the rollup to one settlement currency, ISO 4217 (matched against the
    /// `CurveSet` currency the positions price under).
    #[must_use]
    pub fn ccy(mut self, ccy: impl Into<String>) -> Self {
        self.ccy = Some(ccy.into());
        self
    }

    fn to_wire(&self) -> WireRatesRiskScope {
        WireRatesRiskScope {
            entity: self.entity,
            book: self.book,
            ccy: self.ccy.clone(),
        }
    }
}

/// One bucket of a netted key-rate (instrument-Jacobian) DV01 ladder: the DV01
/// attributable to a single calibrating-instrument tenor, in settlement-currency PV.
/// The typed form of the wire `KeyRateDv01`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyRateDv01 {
    /// The calibrating-instrument tenor in whole years (the ladder's bucket key).
    pub tenor_years: u32,
    /// The netted DV01 of a +1bp bump of that tenor's quote, in settlement-ccy PV.
    pub dv01: f64,
}

impl KeyRateDv01 {
    fn from_wire(w: &WireKeyRateDv01) -> Self {
        Self {
            tenor_years: w.tenor_years,
            dv01: w.dv01,
        }
    }
}

/// One per-currency node of the firm rates rollup: the netted PV / PV01 / DV01 and
/// the tenor-bucketed key-rate ladder for one settlement currency. All measures are
/// in `ccy`; currencies never cross-net. The typed form of the wire `RatesRiskNode`.
#[derive(Debug, Clone, PartialEq)]
pub struct RatesRiskNode {
    /// The settlement currency this node nets, ISO 4217.
    pub ccy: String,
    /// Net present value over the netting set, in `ccy`.
    pub net_pv: f64,
    /// Net analytic PV01, in `ccy` PV.
    pub net_pv01: f64,
    /// Net parallel DV01, in `ccy` PV.
    pub net_dv01: f64,
    /// The merged key-rate DV01 ladder, bucketed by tenor and ascending by tenor.
    pub key_rate_ladder: Vec<KeyRateDv01>,
}

impl RatesRiskNode {
    fn from_wire(w: &WireRatesRiskNode) -> Self {
        Self {
            ccy: w.ccy.clone(),
            net_pv: w.net_pv,
            net_pv01: w.net_pv01,
            net_dv01: w.net_dv01,
            key_rate_ladder: w
                .key_rate_ladder
                .iter()
                .map(KeyRateDv01::from_wire)
                .collect(),
        }
    }
}

/// The firm rates rollup from [`crate::Client::aggregate_rates_risk`]: one
/// [`RatesRiskNode`] per settlement currency (ascending by currency code).
#[derive(Debug, Clone, PartialEq)]
pub struct RatesRiskAggregate {
    /// The per-currency rollup nodes, ascending by currency code (deterministic).
    pub nodes: Vec<RatesRiskNode>,
    /// Echo of the request's correlation id, if one was supplied.
    pub correlation_id: Option<u64>,
}

impl RatesRiskAggregate {
    pub(crate) fn from_wire(w: AggregateRatesRiskResponse) -> Self {
        Self {
            nodes: w.nodes.iter().map(RatesRiskNode::from_wire).collect(),
            correlation_id: w.correlation_id,
        }
    }
}

/// A fluent builder for a linear-rates portfolio-risk aggregate: price `positions`
/// against `curve` and net the firm rollup SERVER-SIDE. Defaults are the whole
/// submitted book under the grant-all (show-all-now) principal. Pass to
/// [`crate::Client::aggregate_rates_risk`].
///
/// The positions travel inline on the request (the caller submits its own book);
/// the same additive fan-in the store-backed rollup runs equals the single-node
/// aggregate bit-for-bit, so the client never prices or sums.
#[derive(Debug, Clone)]
pub struct RatesAggregateQuery {
    curve: UsdSofrCurve,
    positions: Vec<RatesPosition>,
    scope: Option<RatesRiskScope>,
    principal: Option<Entitlements>,
    correlation_id: Option<u64>,
}

impl RatesAggregateQuery {
    /// A new aggregate pricing `positions` against `curve` (the calibrated rates
    /// market every position is valued on).
    #[must_use]
    pub fn new(curve: UsdSofrCurve, positions: impl IntoIterator<Item = RatesPosition>) -> Self {
        Self {
            curve,
            positions: positions.into_iter().collect(),
            scope: None,
            principal: None,
            correlation_id: None,
        }
    }

    /// Narrow the rollup to one `(entity, book, ccy)` subtree before the fan-in.
    #[must_use]
    pub fn scoped(mut self, scope: RatesRiskScope) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Apply an entitlement principal. Omit ⇒ the grant-all (show-all-now) default.
    #[must_use]
    pub fn entitled(mut self, principal: Entitlements) -> Self {
        self.principal = Some(principal);
        self
    }

    /// Attach a caller correlation id, echoed on the response.
    #[must_use]
    pub fn correlation_id(mut self, id: u64) -> Self {
        self.correlation_id = Some(id);
        self
    }

    pub(crate) fn to_wire(&self, session_token: Option<String>) -> AggregateRatesRiskRequest {
        AggregateRatesRiskRequest {
            curve_set: Some(self.curve.to_wire()),
            positions: self.positions.iter().map(|p| p.to_wire()).collect(),
            scope: self.scope.as_ref().map(RatesRiskScope::to_wire),
            principal: Some(principal_or_grant_all(self.principal.as_ref())),
            correlation_id: self.correlation_id,
            session_token,
        }
    }
}

/// A fluent builder for a rates position listing. Defaults list the whole booked
/// rates book under the grant-all (show-all-now) principal. Pass to
/// [`crate::Client::list_rates_positions`].
#[derive(Debug, Clone, Default)]
pub struct RatesPositionQuery {
    scope: Option<RatesRiskScope>,
    principal: Option<Entitlements>,
    correlation_id: Option<String>,
}

impl RatesPositionQuery {
    /// A new listing query (the whole booked rates book by default).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Narrow the listing to one `(entity, book, ccy)` subtree.
    #[must_use]
    pub fn scoped(mut self, scope: RatesRiskScope) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Apply an entitlement principal. Omit ⇒ the grant-all (show-all-now) default.
    #[must_use]
    pub fn entitled(mut self, principal: Entitlements) -> Self {
        self.principal = Some(principal);
        self
    }

    /// Attach a caller correlation id, echoed by the server.
    #[must_use]
    pub fn correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    pub(crate) fn to_wire(&self, session_token: Option<String>) -> ListRatesPositionsRequest {
        ListRatesPositionsRequest {
            session_token,
            scope: self.scope.as_ref().map(RatesRiskScope::to_wire),
            principal: Some(principal_or_grant_all(self.principal.as_ref())),
            correlation_id: self.correlation_id.clone(),
        }
    }
}

/// Build the `BookRatesPosition` request for `position`, threading the client's
/// bearer `session_token` (booking carries the `Book·FixedIncome` capability, which
/// resolves ONLY from an authenticated session — a body principal cannot self-grant
/// it) and the grant-all-or-asserted `principal`.
pub(crate) fn book_rates_request(
    position: &RatesPosition,
    principal: Option<&Entitlements>,
    session_token: Option<String>,
) -> BookRatesPositionRequest {
    BookRatesPositionRequest {
        session_token,
        position: Some(position.to_wire()),
        principal: Some(principal_or_grant_all(principal)),
        correlation_id: None,
    }
}

/// Decode the booked/listed wire positions into the typed [`RatesPosition`] set.
pub(crate) fn positions_from_wire(
    positions: &[WireRatesPosition],
) -> ClientResult<Vec<RatesPosition>> {
    positions.iter().map(RatesPosition::from_wire).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve() -> UsdSofrCurve {
        UsdSofrCurve::new(CivilDate::new(2026, 6, 25))
            .pillar(1, 0.0432)
            .pillar_months(18, 0.0418)
            .pillar_on(CivilDate::new(2031, 6, 30), 0.0405)
    }

    #[test]
    fn curve_builds_wire_curve_set() {
        use pillar_tenor::Point;
        let wire = curve().to_wire();
        assert_eq!(wire.currency, "USD");
        let rd = wire.reference_date.unwrap();
        assert_eq!((rd.year, rd.month, rd.day), (2026, 6, 25));
        assert_eq!(wire.ois_pillars.len(), 3);
        // Each builder method routes to the matching wire arm.
        assert_eq!(
            wire.ois_pillars[0].tenor.as_ref().unwrap().point,
            Some(Point::Years(1))
        );
        assert_eq!(
            wire.ois_pillars[1].tenor.as_ref().unwrap().point,
            Some(Point::Months(18))
        );
        assert!(matches!(
            wire.ois_pillars[2].tenor.as_ref().unwrap().point,
            Some(Point::MaturityDate(_))
        ));
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

    #[test]
    fn rates_position_round_trips_through_the_wire() {
        let pos =
            RatesPosition::new(3, 10, Ois::pay_fixed(5, 0.0405).notional(25_000_000.0)).with_id(7);
        let wire = pos.to_wire();
        assert_eq!(wire.position_id, 7);
        assert_eq!(wire.entity, 3);
        assert_eq!(wire.book, 10);
        let back = RatesPosition::from_wire(&wire).expect("decodes");
        assert_eq!(back, pos);
        assert_eq!(back.instrument.tenor_years(), 5);
        assert_eq!(back.instrument.notional_amount(), 25_000_000.0);
        assert_eq!(back.instrument.side(), OisSide::PayFixed);
    }

    #[test]
    fn ois_from_wire_rejects_a_missing_arm() {
        let empty = RatesInstrument { instrument: None };
        assert!(matches!(
            Ois::from_wire(&empty),
            Err(ClientError::MissingField("RatesInstrument.instrument"))
        ));
    }

    #[test]
    fn aggregate_query_carries_positions_and_scope() {
        let q = RatesAggregateQuery::new(
            curve(),
            [RatesPosition::new(1, 2, Ois::receive_fixed(2, 0.0418))],
        )
        .scoped(RatesRiskScope::new().entity(1).ccy("USD"))
        .correlation_id(9);
        let wire = q.to_wire(Some("tok".to_owned()));
        assert_eq!(wire.positions.len(), 1);
        assert_eq!(wire.correlation_id, Some(9));
        assert_eq!(wire.session_token.as_deref(), Some("tok"));
        let scope = wire.scope.expect("scope carried");
        assert_eq!(scope.entity, Some(1));
        assert_eq!(scope.ccy.as_deref(), Some("USD"));
        assert!(
            wire.principal.expect("grant-all default").grant_all,
            "no entitlement ⇒ explicit grant-all"
        );
    }

    #[test]
    fn rates_aggregate_decodes_the_per_ccy_nodes() {
        let resp = AggregateRatesRiskResponse {
            nodes: vec![WireRatesRiskNode {
                ccy: "USD".to_owned(),
                net_pv: -1000.0,
                net_pv01: 50.0,
                net_dv01: 49.0,
                key_rate_ladder: vec![WireKeyRateDv01 {
                    tenor_years: 5,
                    dv01: 49.0,
                }],
            }],
            correlation_id: Some(11),
        };
        let agg = RatesRiskAggregate::from_wire(resp);
        assert_eq!(agg.correlation_id, Some(11));
        assert_eq!(agg.nodes.len(), 1);
        assert_eq!(agg.nodes[0].ccy, "USD");
        assert_eq!(agg.nodes[0].key_rate_ladder[0].tenor_years, 5);
    }
}
