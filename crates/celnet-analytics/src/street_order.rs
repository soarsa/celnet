//! The **street-side execution record** — one outbound order we sent to the street,
//! and what came back.
//!
//! [`LpFlowRecord`](crate::LpFlowRecord) grades an LP's *behaviour* (how fast it ticks,
//! how often it wins). It deliberately carries no order economics, so it cannot answer
//! the desk's actual street-side question: **what went out, to whom, on what product,
//! and what happened to it**. That is this record.
//!
//! A [`StreetOrder`] is written once per outbound street execution attempt — every
//! attempt, not only the ones that filled — so the absence of street flow is itself
//! visible rather than indistinguishable from "nothing was recorded". It carries the
//! parent linkage (the hedge decision / position that produced it) so a trader can walk
//! from a risk breach to the street orders it caused, without this module duplicating
//! the decision audit trail that owns the *why*.
//!
//! # Honesty rules (guardrail 2 — no fabricated metrics)
//!
//! Every quantity here is an **input**: nothing is re-priced and nothing is inferred.
//! A datum we did not observe is [`None`], never a plausible zero:
//!
//! - `filled_price` / `slippage_bp` are absent on an order that filled nothing.
//! - `response_latency_nanos` is absent unless the routing seam actually measured a
//!   round trip to the venue. An in-process panel lift has no round trip, and timing a
//!   memory read would be a fabricated latency.
//! - `order_type` / `time_in_force` are absent unless the routing seam genuinely issued
//!   a typed order carrying them. When present they are the FIX `OrdType(40)` /
//!   `TimeInForce(59)` values actually sent — the estate's only order→fill contract is
//!   FIX `NewOrderSingle(D)` → `ExecutionReport(8)`, and this record keys off it rather
//!   than defining a parallel vocabulary.
//! - `lp_id` is whatever the routing seam reports. No LP roster is hardcoded anywhere in
//!   this module: the roster is the venue configuration's business, not the analytics'.
//! - `cover` (the runner-up we dealt away from) is absent when fewer than two LPs
//!   showed a firm price — there was no cover.
//! - A [`StreetVenue::CompositeBackstop`] order is **never** attributed to a named LP.
//!   `lp_id` is absent, and the fold refuses to credit any LP for it.
//!
//! # Purity
//!
//! [`fold_breakdown`] and [`lp_flow_records`] are pure, deterministic functions of their
//! input slice — no clock, no I/O — hence oracle-testable. The server owns capture and
//! retention; this crate owns the arithmetic.

use std::collections::BTreeMap;

use crate::LpFlowRecord;

/// The side **we** dealt on the street (the desk's own side, not the LP's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreetSide {
    /// We bought from the street (lifted an LP's offer).
    Buy,
    /// We sold to the street (hit an LP's bid).
    Sell,
}

impl StreetSide {
    /// The stable wire/GUI token.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }

    /// The side implied by shedding a signed net risk: reducing a **long**
    /// (`net_risk > 0`) means selling; reducing a short means buying. A flat/zero net
    /// has no directional risk to shed and is treated as a sell (the degenerate shed),
    /// matching the execution seam's own tie-break so the record never disagrees with
    /// the order that was actually sent.
    #[must_use]
    pub fn shedding(net_risk: f64) -> Self {
        if net_risk > 0.0 {
            Self::Sell
        } else {
            Self::Buy
        }
    }
}

/// What became of one outbound street order.
///
/// These are **observed** terminal states. There is deliberately no "unknown" variant:
/// a recorded order always reached one of these, and an attempt whose result we never
/// learned would be a bug in the routing seam, not a metric to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreetOutcome {
    /// The full requested quantity filled.
    Filled,
    /// Some — but not all — of the requested quantity filled (an IOC-style partial).
    PartiallyFilled,
    /// The venue **refused** the order (FIX `OrdStatus=8`, Rejected) — it declined to
    /// trade, and its reject reason says why. A refusal is a statement about the
    /// counterparty.
    Rejected,
    /// The venue **accepted** the order but could not satisfy it, so it was cancelled
    /// (FIX `OrdStatus=4`, Canceled) — e.g. an all-or-nothing clip larger than the
    /// available size. Deliberately NOT folded into [`Self::Rejected`]: "would not"
    /// and "could not" are different facts about an LP, and flattening them would make
    /// a deep, willing counterparty look like a refusing one.
    Cancelled,
    /// The order lapsed before it could be filled (its validity window closed).
    Expired,
    /// The LP's quote was pulled at last look after we tried to deal on it.
    LastLookPulled,
    /// No LP had a firm executable price on the required side, so nothing was routed to
    /// a named LP. Distinct from [`Self::Rejected`]: nobody said no, nobody was asked.
    NoLiquidity,
}

impl StreetOutcome {
    /// The stable wire/GUI token.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Filled => "filled",
            Self::PartiallyFilled => "partially_filled",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
            Self::LastLookPulled => "last_look_pulled",
            Self::NoLiquidity => "no_liquidity",
        }
    }

    /// Whether any quantity at all changed hands.
    #[must_use]
    pub const fn is_fill(self) -> bool {
        matches!(self, Self::Filled | Self::PartiallyFilled)
    }
}

/// Where the order actually executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreetVenue {
    /// A named street liquidity provider — the only venue that earns LP attribution.
    NamedLp,
    /// The internal composite book, used as a backstop when the street showed no firm
    /// price. A synthetic mid, **not** a counterparty: it is never credited to an LP.
    CompositeBackstop,
    /// Nothing executed anywhere (an honest miss, or an advisory/shadow run).
    None,
}

impl StreetVenue {
    /// The stable wire/GUI token.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NamedLp => "named_lp",
            Self::CompositeBackstop => "composite_backstop",
            Self::None => "none",
        }
    }
}

/// One LP that showed a firm executable price on the required side when the order was
/// worked — the competition the order was ranked against.
#[derive(Debug, Clone, PartialEq)]
pub struct StreetCompetitor {
    /// The competing LP's id.
    pub lp_id: String,
    /// Its firm price on the side we needed (its bid when we sold, its offer when we
    /// bought), in the instrument's quote convention.
    pub price: f64,
}

/// One outbound street order and its result.
///
/// Construct via [`StreetOrder::new`] and override only the fields the attempt actually
/// established, so an unobserved datum stays absent by default rather than by omission.
#[derive(Debug, Clone, PartialEq)]
pub struct StreetOrder {
    /// Stable id for this street order (unique within the recording process).
    pub order_id: String,
    /// When the order was worked (epoch nanos, UTC).
    pub ts_nanos: i64,
    /// The LP the order executed against. **Absent** for a composite backstop or an
    /// order that reached no venue — never a placeholder name.
    pub lp_id: Option<String>,
    /// Where it executed.
    pub venue: StreetVenue,
    /// The instrument actually dealt (the tradeable security id, or the product family
    /// label when the cell resolved no security master id).
    pub instrument: String,
    /// The product family token (`ois` / `irs` / `fra` / `bond` / `bond_future` /
    /// `stir_future`, lower-cased). Empty when the family is genuinely unclassified.
    pub family: String,
    /// The risk tenor of the hedged exposure, in years. Absent when the cell carries no
    /// parseable tenor or maturity — never defaulted to zero (a zero-year tenor is a
    /// real and very different thing).
    pub tenor_years: Option<f64>,
    /// The desk's own side.
    pub side: StreetSide,
    /// The quantity we asked for, in the order's native metric units.
    pub requested_qty: f64,
    /// The quantity that actually filled (`0.0` when nothing did).
    pub filled_qty: f64,
    /// The reference price the order was worked against (the composite mid at fire).
    pub requested_price: f64,
    /// The realised fill price. Absent when nothing filled — a miss carries no price.
    pub filled_price: Option<f64>,
    /// Signed slippage of the fill against the reference, in basis points of the
    /// instrument's quote convention. Absent when nothing filled.
    pub slippage_bp: Option<f64>,
    /// The terminal outcome.
    pub outcome: StreetOutcome,
    /// A short machine-stable **qualifier** on the outcome.
    ///
    /// On a non-fill it says why (`no_firm_lp_price`, `venue_rejected`, …). On a fill it
    /// may qualify *how* the fill was obtained — in particular `quote_derived_lift`,
    /// which states that the price was lifted from the LP's standing quote in-process
    /// rather than by routing an order to it. That distinction is not cosmetic: a
    /// blotter row reading "Filled on <LP>" would otherwise be read as "we sent that LP
    /// an order and it traded with us", which is a stronger claim than the record can
    /// support until the routing seam issues real orders. Absent when the outcome needs
    /// no qualification.
    pub reason: Option<String>,
    /// Every LP that showed a firm price on the required side, best-first. Empty when
    /// the street showed nothing.
    pub competitors: Vec<StreetCompetitor>,
    /// The hedge decision this order belongs to (the id carried by the hedge provenance
    /// record). Absent for an order with no hedge parent.
    pub parent_hedge_id: Option<String>,
    /// The client fill / position this order was shed from. Absent when unlinked.
    pub parent_position_id: Option<u64>,
    /// The order type actually sent, as the FIX `OrdType(40)` value (`"1"` market,
    /// `"2"` limit, …) — the estate's only order contract is FIX
    /// `NewOrderSingle(D)` → `ExecutionReport(8)`, so this carries that tag verbatim
    /// rather than inventing a parallel vocabulary. **Absent** on a row that sent no
    /// order (a composite backstop, an advisory run) — see the module honesty rules.
    pub order_type: Option<String>,
    /// The time-in-force actually sent, as the FIX `TimeInForce(59)` value (`"3"` IOC,
    /// `"4"` FOK, …). **Absent** on a row that sent no order.
    pub time_in_force: Option<String>,
    /// Measured venue round-trip latency, nanoseconds — send to `ExecutionReport(8)`.
    /// **Absent** only where there was no round trip to measure: a member with no
    /// configured order endpoint, or a composite backstop.
    pub response_latency_nanos: Option<u64>,
}

impl StreetOrder {
    /// A street order with everything unobserved left absent: no LP, no venue, nothing
    /// filled, no cover, no parent, no typed-order fields, no measured latency. Callers
    /// override exactly the fields their attempt established.
    #[must_use]
    pub fn new(
        order_id: impl Into<String>,
        ts_nanos: i64,
        instrument: impl Into<String>,
        side: StreetSide,
        requested_qty: f64,
        requested_price: f64,
    ) -> Self {
        Self {
            order_id: order_id.into(),
            ts_nanos,
            lp_id: None,
            venue: StreetVenue::None,
            instrument: instrument.into(),
            family: String::new(),
            tenor_years: None,
            side,
            requested_qty,
            filled_qty: 0.0,
            requested_price,
            filled_price: None,
            slippage_bp: None,
            outcome: StreetOutcome::NoLiquidity,
            reason: None,
            competitors: Vec::new(),
            parent_hedge_id: None,
            parent_position_id: None,
            order_type: None,
            time_in_force: None,
            response_latency_nanos: None,
        }
    }

    /// The runner-up we dealt away from — the second-best firm price on the required
    /// side. `None` when fewer than two LPs showed one (there was no cover).
    #[must_use]
    pub fn cover(&self) -> Option<&StreetCompetitor> {
        self.competitors.get(1)
    }

    /// The absolute distance between the best and the cover price, in the instrument's
    /// quote convention. `None` when there was no cover, or either price is non-finite.
    #[must_use]
    pub fn cover_distance(&self) -> Option<f64> {
        let best = self.competitors.first()?;
        let cover = self.cover()?;
        let d = (cover.price - best.price).abs();
        d.is_finite().then_some(d)
    }

    /// Fill ratio = filled ÷ requested. `None` when nothing was requested (the
    /// divide-by-zero guard — an order for nothing has no ratio, not a ratio of zero).
    #[must_use]
    pub fn fill_ratio(&self) -> Option<f64> {
        (self.requested_qty > 0.0).then(|| self.filled_qty / self.requested_qty)
    }
}

// ---------------------------------------------------------------------------
// StreetOrder → LpFlowRecord (the league-table seam)
// ---------------------------------------------------------------------------

/// Map one street order onto the per-LP league-table records it honestly implies.
///
/// - A fill on a **named LP** ⇒ that LP is `was_quoted` + `was_won`, carrying the filled
///   notional; every other LP that showed a firm price is `was_quoted` + `was_missed`
///   (it was on our panel and we dealt away from it), and the cover additionally carries
///   the real `cover_distance`.
/// - A **last-look pull** ⇒ that LP is `was_quoted` + `was_last_look_reject`. The others
///   are not marked missed: no deal was done, so nobody lost one.
/// - A **rejection / expiry** ⇒ that LP is `was_quoted` only (it showed a price; the
///   order did not trade). A reject is not a last-look pull and is never counted as one.
/// - A **composite backstop** or a **no-liquidity** miss ⇒ **no** records at all. There
///   is no LP to credit or debit, and inventing a row would fabricate street activity.
#[must_use]
pub fn lp_flow_records(order: &StreetOrder) -> Vec<LpFlowRecord> {
    if order.venue == StreetVenue::CompositeBackstop || order.competitors.is_empty() {
        return Vec::new();
    }
    let Some(dealt_lp) = order.lp_id.as_deref() else {
        return Vec::new();
    };
    let cover_lp = order.cover().map(|c| c.lp_id.as_str());
    let cover_distance = order.cover_distance();
    let dealt = order.outcome.is_fill();

    order
        .competitors
        .iter()
        .map(|c| {
            let is_dealt_lp = c.lp_id == dealt_lp;
            LpFlowRecord {
                lp_id: c.lp_id.clone(),
                instrument: order.instrument.clone(),
                // Every competitor showed a firm price on the side we needed — that is
                // exactly the response-presence signal `was_quoted` denotes.
                was_quoted: true,
                was_won: is_dealt_lp && dealt,
                // A miss requires a deal to have been done elsewhere.
                was_missed: !is_dealt_lp && dealt,
                was_last_look_reject: is_dealt_lp && order.outcome == StreetOutcome::LastLookPulled,
                notional: if is_dealt_lp && dealt {
                    order.filled_qty.max(0.0)
                } else {
                    0.0
                },
                tick: false,
                // Only the cover carries a cover distance; everyone else's is absent.
                cover_distance: (Some(c.lp_id.as_str()) == cover_lp)
                    .then_some(cover_distance)
                    .flatten(),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Breakdown fold
// ---------------------------------------------------------------------------

/// The axis a street-side breakdown groups on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakdownDimension {
    /// One row per liquidity provider (composite-backstop orders group under the
    /// explicit [`COMPOSITE_KEY`], never under an LP).
    Lp,
    /// One row per product family (`ois`, `bond_future`, …).
    Family,
    /// One row per dealt instrument.
    Instrument,
    /// One row per tenor bucket (see [`tenor_bucket`]).
    TenorBucket,
    /// One row per UTC hour of activity (`YYYY-MM-DDTHH`).
    Hour,
}

impl BreakdownDimension {
    /// The stable wire/GUI token.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Lp => "lp",
            Self::Family => "family",
            Self::Instrument => "instrument",
            Self::TenorBucket => "tenor_bucket",
            Self::Hour => "hour",
        }
    }

    /// Parse a wire token back to a dimension. `None` for an unknown token — the caller
    /// rejects rather than silently grouping on some default axis.
    #[must_use]
    pub fn from_label(s: &str) -> Option<Self> {
        match s {
            "lp" => Some(Self::Lp),
            "family" => Some(Self::Family),
            "instrument" => Some(Self::Instrument),
            "tenor_bucket" => Some(Self::TenorBucket),
            "hour" => Some(Self::Hour),
            _ => None,
        }
    }
}

/// The group key used for orders that backstopped to the composite instead of reaching a
/// named LP. Explicit rather than blank so the GUI can label it "composite backstop" —
/// it is a real, meaningful bucket, not missing data.
pub const COMPOSITE_KEY: &str = "COMPOSITE";

/// The group key used when the grouping dimension is genuinely **absent** on an order
/// (an unclassified family, an unparseable tenor). Rendered as an explicit
/// "unattributed" bucket, never merged into a real one.
pub const UNATTRIBUTED_KEY: &str = "";

/// Bucket a tenor in years onto the desk's standard curve segments. `None` in ⇒
/// [`UNATTRIBUTED_KEY`] out: an order whose tenor we never learned is not silently
/// filed under the front bucket.
#[must_use]
pub fn tenor_bucket(tenor_years: Option<f64>) -> &'static str {
    let Some(t) = tenor_years.filter(|t| t.is_finite() && *t >= 0.0) else {
        return UNATTRIBUTED_KEY;
    };
    if t < 1.0 {
        "0-1Y"
    } else if t < 3.0 {
        "1-3Y"
    } else if t < 7.0 {
        "3-7Y"
    } else if t < 15.0 {
        "7-15Y"
    } else {
        "15Y+"
    }
}

/// The UTC hour bucket label (`YYYY-MM-DDTHH`) for an epoch-nanos timestamp.
///
/// The civil-date conversion is done here rather than via a calendar dependency so this
/// crate stays free of runtime deps (it is the pure numerical layer). The days →
/// year/month/day step is the standard proleptic-Gregorian `civil_from_days` algorithm
/// (H. Hinnant, *chrono-Compatible Low-Level Date Algorithms*), exact for every day in
/// the representable range and shifted to a 0000-03-01 era origin so it handles
/// pre-epoch dates without a special case.
#[must_use]
pub fn hour_bucket(ts_nanos: i64) -> String {
    const NANOS_PER_SEC: i64 = 1_000_000_000;
    const SECS_PER_DAY: i64 = 86_400;
    // Floor-divide throughout so pre-epoch timestamps bucket into the hour that
    // contains them rather than truncating toward zero.
    let secs = ts_nanos.div_euclid(NANOS_PER_SEC);
    let days = secs.div_euclid(SECS_PER_DAY);
    let secs_of_day = secs.rem_euclid(SECS_PER_DAY);
    let hour = secs_of_day / 3_600;

    // Shift the epoch from 1970-01-01 to 0000-03-01 (era-based, March-first year).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11], March-based
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if month <= 2 { y + 1 } else { y };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}")
}

/// The group key an order contributes on `dimension`.
#[must_use]
pub fn group_key(order: &StreetOrder, dimension: BreakdownDimension) -> String {
    match dimension {
        BreakdownDimension::Lp => match (order.venue, order.lp_id.as_deref()) {
            (StreetVenue::CompositeBackstop, _) => COMPOSITE_KEY.to_owned(),
            (_, Some(lp)) => lp.to_owned(),
            (_, None) => UNATTRIBUTED_KEY.to_owned(),
        },
        BreakdownDimension::Family => order.family.clone(),
        BreakdownDimension::Instrument => order.instrument.clone(),
        BreakdownDimension::TenorBucket => tenor_bucket(order.tenor_years).to_owned(),
        BreakdownDimension::Hour => hour_bucket(order.ts_nanos),
    }
}

/// One aggregated street-side row. Every ratio is [`None`] when its denominator is zero
/// — the divide-by-zero guard the GUI renders as an explicit absence marker.
#[derive(Debug, Clone, PartialEq)]
pub struct StreetBreakdownRow {
    /// The axis this row was grouped on.
    pub dimension: BreakdownDimension,
    /// The group key (an LP id, a family token, a tenor bucket, an hour label,
    /// [`COMPOSITE_KEY`], or [`UNATTRIBUTED_KEY`]).
    pub key: String,
    /// Orders in this group.
    pub orders: u64,
    /// Orders that fully filled.
    pub filled: u64,
    /// Orders that partially filled.
    pub partially_filled: u64,
    /// Orders the venue refused (`OrdStatus=8`).
    pub rejected: u64,
    /// Orders the venue accepted but could not satisfy (`OrdStatus=4`).
    pub cancelled: u64,
    /// Orders that expired unfilled.
    pub expired: u64,
    /// Orders pulled at last look.
    pub last_look_pulled: u64,
    /// Orders that found no firm street price at all.
    pub no_liquidity: u64,
    /// Orders that backstopped to the composite instead of reaching a named LP.
    pub composite_backstop: u64,
    /// Total quantity requested across the group.
    pub requested_qty: f64,
    /// Total quantity filled across the group.
    pub filled_qty: f64,
    /// Filled ÷ requested quantity. `None` when nothing was requested.
    pub fill_ratio: Option<f64>,
    /// Orders that filled (fully or partially) ÷ orders. `None` when the group is empty.
    pub win_rate: Option<f64>,
    /// Mean signed slippage (bp) over the orders that actually filled. `None` when none
    /// did — never a slippage of zero for a group that never traded.
    pub mean_slippage_bp: Option<f64>,
    /// Mean measured venue round-trip latency (ns) over the orders that carried one.
    /// `None` when no order in the group had a measured latency.
    pub mean_response_latency_nanos: Option<u64>,
    /// Last-look pulls ÷ orders. `None` when the group is empty.
    pub last_look_rate: Option<f64>,
    /// Mean cover distance over the orders that had a cover. `None` when none did.
    pub mean_cover: Option<f64>,
    /// Mean number of LPs showing a firm price when these orders were worked. `None`
    /// when the group is empty.
    pub mean_competitors: Option<f64>,
}

/// Single-pass accumulator for one group.
#[derive(Debug, Clone, Copy, Default)]
struct Acc {
    orders: u64,
    filled: u64,
    partially_filled: u64,
    rejected: u64,
    cancelled: u64,
    expired: u64,
    last_look_pulled: u64,
    no_liquidity: u64,
    composite_backstop: u64,
    requested_qty: f64,
    filled_qty: f64,
    slippage_sum: f64,
    slippage_n: u64,
    latency_sum: u128,
    latency_n: u64,
    cover_sum: f64,
    cover_n: u64,
    competitors_sum: u64,
}

impl Acc {
    fn ingest(&mut self, o: &StreetOrder) {
        self.orders += 1;
        match o.outcome {
            StreetOutcome::Filled => self.filled += 1,
            StreetOutcome::PartiallyFilled => self.partially_filled += 1,
            StreetOutcome::Rejected => self.rejected += 1,
            StreetOutcome::Cancelled => self.cancelled += 1,
            StreetOutcome::Expired => self.expired += 1,
            StreetOutcome::LastLookPulled => self.last_look_pulled += 1,
            StreetOutcome::NoLiquidity => self.no_liquidity += 1,
        }
        if o.venue == StreetVenue::CompositeBackstop {
            self.composite_backstop += 1;
        }
        if o.requested_qty.is_finite() {
            self.requested_qty += o.requested_qty.max(0.0);
        }
        if o.filled_qty.is_finite() {
            self.filled_qty += o.filled_qty.max(0.0);
        }
        // Slippage is only meaningful where something filled — an unfilled order has no
        // realised price to slip against.
        if let Some(s) = o.slippage_bp.filter(|s| s.is_finite())
            && o.outcome.is_fill()
        {
            self.slippage_sum += s;
            self.slippage_n += 1;
        }
        if let Some(l) = o.response_latency_nanos {
            self.latency_sum += u128::from(l);
            self.latency_n += 1;
        }
        if let Some(c) = o.cover_distance() {
            self.cover_sum += c;
            self.cover_n += 1;
        }
        self.competitors_sum += o.competitors.len() as u64;
    }

    fn finish(self, dimension: BreakdownDimension, key: String) -> StreetBreakdownRow {
        let orders_f = self.orders as f64;
        StreetBreakdownRow {
            dimension,
            key,
            orders: self.orders,
            filled: self.filled,
            partially_filled: self.partially_filled,
            rejected: self.rejected,
            cancelled: self.cancelled,
            expired: self.expired,
            last_look_pulled: self.last_look_pulled,
            no_liquidity: self.no_liquidity,
            composite_backstop: self.composite_backstop,
            requested_qty: self.requested_qty,
            filled_qty: self.filled_qty,
            fill_ratio: (self.requested_qty > 0.0).then(|| self.filled_qty / self.requested_qty),
            win_rate: (self.orders > 0)
                .then(|| (self.filled + self.partially_filled) as f64 / orders_f),
            mean_slippage_bp: (self.slippage_n > 0)
                .then(|| self.slippage_sum / self.slippage_n as f64),
            mean_response_latency_nanos: (self.latency_n > 0).then(|| {
                u64::try_from(self.latency_sum / u128::from(self.latency_n)).unwrap_or(u64::MAX)
            }),
            last_look_rate: (self.orders > 0).then(|| self.last_look_pulled as f64 / orders_f),
            mean_cover: (self.cover_n > 0).then(|| self.cover_sum / self.cover_n as f64),
            mean_competitors: (self.orders > 0).then(|| self.competitors_sum as f64 / orders_f),
        }
    }
}

/// Fold street orders into one aggregated row per group on `dimension`.
///
/// Pure and deterministic: rows come back in `BTreeMap` key order, so the same input
/// always yields the same output ordering regardless of input order.
#[must_use]
pub fn fold_breakdown(
    orders: &[StreetOrder],
    dimension: BreakdownDimension,
) -> Vec<StreetBreakdownRow> {
    let mut groups: BTreeMap<String, Acc> = BTreeMap::new();
    for o in orders {
        groups.entry(group_key(o, dimension)).or_default().ingest(o);
    }
    groups
        .into_iter()
        .map(|(key, acc)| acc.finish(dimension, key))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn competitor(lp: &str, price: f64) -> StreetCompetitor {
        StreetCompetitor {
            lp_id: lp.to_owned(),
            price,
        }
    }

    /// A filled order on `lp` with a three-LP panel, best-first.
    fn filled_order(id: &str, lp: &str, qty: f64, ts: i64) -> StreetOrder {
        StreetOrder {
            lp_id: Some(lp.to_owned()),
            venue: StreetVenue::NamedLp,
            family: "ois".to_owned(),
            tenor_years: Some(10.0),
            filled_qty: qty,
            filled_price: Some(0.03005),
            slippage_bp: Some(0.5),
            outcome: StreetOutcome::Filled,
            competitors: vec![
                competitor(lp, 0.03005),
                competitor("LP-B", 0.03007),
                competitor("LP-C", 0.03011),
            ],
            ..StreetOrder::new(id, ts, "USSW10", StreetSide::Buy, qty, 0.03)
        }
    }

    #[test]
    fn a_named_lp_fill_wins_and_every_other_panel_lp_misses() {
        let o = filled_order("S-1", "LP-A", 5_000.0, 1_000);
        let recs = lp_flow_records(&o);
        assert_eq!(recs.len(), 3, "one record per LP that showed a firm price");

        let a = recs.iter().find(|r| r.lp_id == "LP-A").expect("LP-A");
        assert!(a.was_quoted && a.was_won && !a.was_missed);
        assert_eq!(a.notional, 5_000.0);

        let b = recs.iter().find(|r| r.lp_id == "LP-B").expect("LP-B");
        assert!(b.was_quoted && b.was_missed && !b.was_won);
        assert_eq!(b.notional, 0.0, "a loser books no notional");
        // LP-B was the cover (second best) ⇒ it carries the REAL distance to the winner.
        assert_eq!(b.cover_distance, Some(0.03007 - 0.03005));

        let c = recs.iter().find(|r| r.lp_id == "LP-C").expect("LP-C");
        assert_eq!(c.cover_distance, None, "only the cover carries a distance");
    }

    #[test]
    fn a_composite_backstop_is_never_attributed_to_any_lp() {
        let o = StreetOrder {
            venue: StreetVenue::CompositeBackstop,
            // A backstop carries no lp_id by construction; even if a caller wrongly set
            // one AND a panel, the venue alone must veto attribution.
            lp_id: Some("LP-A".to_owned()),
            competitors: vec![competitor("LP-A", 1.0)],
            filled_qty: 5_000.0,
            outcome: StreetOutcome::Filled,
            ..StreetOrder::new("S-2", 1, "USSW10", StreetSide::Sell, 5_000.0, 0.03)
        };
        assert!(
            lp_flow_records(&o).is_empty(),
            "COMPOSITE is a synthetic mid, not a street counterparty"
        );
    }

    #[test]
    fn a_no_liquidity_miss_produces_no_records() {
        let o = StreetOrder::new("S-3", 1, "USSW10", StreetSide::Sell, 5_000.0, 0.03);
        assert_eq!(o.outcome, StreetOutcome::NoLiquidity);
        assert!(lp_flow_records(&o).is_empty());
    }

    #[test]
    fn a_last_look_pull_is_a_reject_not_a_miss_for_the_others() {
        let o = StreetOrder {
            outcome: StreetOutcome::LastLookPulled,
            filled_qty: 0.0,
            filled_price: None,
            slippage_bp: None,
            ..filled_order("S-4", "LP-A", 0.0, 1)
        };
        let recs = lp_flow_records(&o);
        let a = recs.iter().find(|r| r.lp_id == "LP-A").expect("LP-A");
        assert!(a.was_last_look_reject && !a.was_won);
        let b = recs.iter().find(|r| r.lp_id == "LP-B").expect("LP-B");
        assert!(
            !b.was_missed,
            "no deal was done, so nobody lost one — a pull is not a miss for the field"
        );
        assert!(b.was_quoted, "it still showed a firm price");
    }

    #[test]
    fn a_rejection_is_never_counted_as_a_last_look_pull() {
        let o = StreetOrder {
            outcome: StreetOutcome::Rejected,
            filled_qty: 0.0,
            reason: Some("venue_rejected".to_owned()),
            ..filled_order("S-5", "LP-A", 0.0, 1)
        };
        let recs = lp_flow_records(&o);
        let a = recs.iter().find(|r| r.lp_id == "LP-A").expect("LP-A");
        assert!(!a.was_last_look_reject, "a reject is not a last-look pull");
        assert!(!a.was_won && a.was_quoted);
    }

    #[test]
    fn partial_fill_ratio_and_absent_ratio_on_a_zero_request() {
        let mut o = filled_order("S-6", "LP-A", 2_500.0, 1);
        o.requested_qty = 10_000.0;
        o.outcome = StreetOutcome::PartiallyFilled;
        assert_eq!(o.fill_ratio(), Some(0.25));

        let empty = StreetOrder::new("S-7", 1, "X", StreetSide::Buy, 0.0, 1.0);
        assert_eq!(
            empty.fill_ratio(),
            None,
            "an order for nothing has no ratio, not a ratio of zero"
        );
    }

    #[test]
    fn cover_is_absent_with_fewer_than_two_firm_prices() {
        let mut o = filled_order("S-8", "LP-A", 1.0, 1);
        o.competitors.truncate(1);
        assert_eq!(o.cover(), None);
        assert_eq!(o.cover_distance(), None);
    }

    #[test]
    fn breakdown_by_lp_separates_the_composite_backstop_bucket() {
        let orders = vec![
            filled_order("S-1", "LP-A", 5_000.0, 1_000),
            filled_order("S-2", "LP-A", 5_000.0, 2_000),
            StreetOrder {
                venue: StreetVenue::CompositeBackstop,
                outcome: StreetOutcome::Filled,
                filled_qty: 4_000.0,
                slippage_bp: Some(-0.5),
                family: "ois".to_owned(),
                ..StreetOrder::new("S-3", 3_000, "USSW10", StreetSide::Sell, 4_000.0, 0.03)
            },
        ];
        let rows = fold_breakdown(&orders, BreakdownDimension::Lp);
        assert_eq!(rows.len(), 2, "LP-A and COMPOSITE, never merged");
        // BTreeMap order: "COMPOSITE" < "LP-A".
        assert_eq!(rows[0].key, COMPOSITE_KEY);
        assert_eq!(rows[0].composite_backstop, 1);
        assert_eq!(rows[1].key, "LP-A");
        assert_eq!(rows[1].orders, 2);
        assert_eq!(rows[1].filled, 2);
        assert_eq!(rows[1].filled_qty, 10_000.0);
        assert_eq!(rows[1].fill_ratio, Some(1.0));
        assert_eq!(rows[1].win_rate, Some(1.0));
        assert_eq!(rows[1].mean_slippage_bp, Some(0.5));
        assert_eq!(rows[1].mean_competitors, Some(3.0));
        assert_eq!(
            rows[1].mean_response_latency_nanos, None,
            "no order carried a measured latency ⇒ ABSENT, never 0"
        );
    }

    #[test]
    fn a_group_that_never_filled_has_absent_slippage_not_zero() {
        let orders = vec![StreetOrder {
            outcome: StreetOutcome::Rejected,
            venue: StreetVenue::None,
            lp_id: Some("LP-A".to_owned()),
            competitors: vec![competitor("LP-A", 1.0)],
            ..StreetOrder::new("S-1", 1, "USSW10", StreetSide::Buy, 1_000.0, 0.03)
        }];
        let rows = fold_breakdown(&orders, BreakdownDimension::Lp);
        assert_eq!(rows[0].mean_slippage_bp, None);
        assert_eq!(rows[0].fill_ratio, Some(0.0), "0 of 1000 filled IS zero");
        assert_eq!(rows[0].win_rate, Some(0.0));
        assert_eq!(rows[0].rejected, 1);
    }

    #[test]
    fn tenor_buckets_match_their_boundaries_and_absent_stays_absent() {
        assert_eq!(tenor_bucket(Some(0.5)), "0-1Y");
        assert_eq!(tenor_bucket(Some(1.0)), "1-3Y");
        assert_eq!(tenor_bucket(Some(3.0)), "3-7Y");
        assert_eq!(tenor_bucket(Some(7.0)), "7-15Y");
        assert_eq!(tenor_bucket(Some(15.0)), "15Y+");
        assert_eq!(tenor_bucket(Some(30.0)), "15Y+");
        assert_eq!(tenor_bucket(None), UNATTRIBUTED_KEY);
        assert_eq!(tenor_bucket(Some(f64::NAN)), UNATTRIBUTED_KEY);
    }

    #[test]
    fn hour_bucket_is_a_stable_utc_label() {
        assert_eq!(hour_bucket(0), "1970-01-01T00");
        assert_eq!(hour_bucket(3_600 * 1_000_000_000 + 5), "1970-01-01T01");
    }

    #[test]
    fn breakdown_by_family_and_tenor_are_independent_partitions() {
        let orders = vec![
            filled_order("S-1", "LP-A", 1.0, 1),
            StreetOrder {
                family: "bond_future".to_owned(),
                tenor_years: Some(5.0),
                ..filled_order("S-2", "LP-B", 1.0, 2)
            },
        ];
        let fam = fold_breakdown(&orders, BreakdownDimension::Family);
        assert_eq!(fam.len(), 2);
        assert_eq!(fam[0].key, "bond_future");
        assert_eq!(fam[1].key, "ois");

        let ten = fold_breakdown(&orders, BreakdownDimension::TenorBucket);
        assert_eq!(ten.len(), 2);
        assert_eq!(ten[0].key, "3-7Y");
        assert_eq!(ten[1].key, "7-15Y");
    }

    #[test]
    fn an_unclassified_family_groups_under_the_explicit_unattributed_key() {
        let orders = vec![StreetOrder {
            family: String::new(),
            ..filled_order("S-1", "LP-A", 1.0, 1)
        }];
        let rows = fold_breakdown(&orders, BreakdownDimension::Family);
        assert_eq!(rows[0].key, UNATTRIBUTED_KEY);
    }

    #[test]
    fn dimension_tokens_round_trip() {
        for d in [
            BreakdownDimension::Lp,
            BreakdownDimension::Family,
            BreakdownDimension::Instrument,
            BreakdownDimension::TenorBucket,
            BreakdownDimension::Hour,
        ] {
            assert_eq!(BreakdownDimension::from_label(d.label()), Some(d));
        }
        assert_eq!(BreakdownDimension::from_label("nonsense"), None);
    }
}
