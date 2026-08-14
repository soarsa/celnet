//! The **order-execution core** every simulated counterparty fills against.
//!
//! Until now the simulators only *published prices*. The server's hedge path
//! (`AggregationHub::best_fill`) therefore synthesised a fill from the last quote it
//! had received: it picked the best non-stale contribution, ignored the requested
//! size entirely, and reported the whole clip as done. Nothing in the estate ever
//! asked a counterparty whether it would actually trade, so no order could ever be
//! partially filled, rejected, or filled at a level worse than the touch — and the
//! LP panel could not show a trader anything they could act on.
//!
//! This module is the missing half: a **pure** matching function that takes an order
//! and one counterparty's quoted depth and returns what that counterparty really
//! does with it. It is deliberately free of transport, clocks and I/O so it is
//! exhaustively testable; [`crate::orders`] wires it to the FIX
//! `NewOrderSingle(D)` → `ExecutionReport(8)` contract the rest of the estate
//! already speaks.
//!
//! # Market model: quote-driven, not a central limit order book
//!
//! Every simulated counterparty here is **quote-driven**: it streams a firm two-way
//! with quoted depth behind it and trades against that quote. It does not maintain a
//! resting order book. This is the correct model for the counterparties being
//! simulated — a bank principal dealer and a dealer-to-client platform fill against
//! their own stream, they do not queue your order — and it has a hard consequence
//! for time-in-force which is honoured rather than papered over:
//!
//! * **[`TimeInForce::FillOrKill`]** and **[`TimeInForce::ImmediateOrCancel`]** are
//!   the two TIFs that mean something to a quote-driven venue, and both are fully
//!   implemented.
//! * Every **resting** TIF ([`TimeInForce::Day`], [`TimeInForce::GoodTillCancel`],
//!   [`TimeInForce::GoodTillDate`], …) is **explicitly rejected** with
//!   [`RejectReason::RestingTifUnsupported`], because honouring it would require a
//!   working-order book that does not exist here. That is a real venue rule (LP
//!   streaming sessions routinely accept IOC/FOK only), and a rejection carrying a
//!   reason is the honest answer — silently treating a Day order as an IOC would
//!   fabricate a cancel the taker never asked for.
//!
//! # What decides how much fills
//!
//! An order walks the counterparty's [`DepthLadder`], which is derived from that
//! counterparty's own [`SimLpProfile`](crate::roster::SimLpProfile) — appetite at
//! the touch, how many levels sit behind it, and how fast size decays across them.
//! Because the four roster members have deliberately different depth shapes, the
//! **same order genuinely fills different amounts on different counterparties**,
//! which is precisely what makes an LP panel worth looking at.

use crate::roster::SimLpProfile;

/// The taker's direction, from the taker's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The taker buys — it lifts the counterparty's **offer**.
    Buy,
    /// The taker sells — it hits the counterparty's **bid**.
    Sell,
}

impl Side {
    /// The FIX `Side(54)` byte (`1` buy / `2` sell).
    #[must_use]
    pub const fn fix_byte(self) -> u8 {
        match self {
            Side::Buy => b'1',
            Side::Sell => b'2',
        }
    }

    /// Recover a side from its FIX `Side(54)` byte. Any byte other than the two the
    /// venue trades is `None` — an unrecognised side is rejected, never guessed.
    #[must_use]
    pub const fn from_fix_byte(b: u8) -> Option<Self> {
        match b {
            b'1' => Some(Side::Buy),
            b'2' => Some(Side::Sell),
            _ => None,
        }
    }

    /// Whether `candidate` is at least as good as `limit` for this side (a buyer
    /// wants a low price, a seller a high one).
    #[must_use]
    pub fn is_marketable(self, candidate: f64, limit: f64) -> bool {
        match self {
            Side::Buy => candidate <= limit,
            Side::Sell => candidate >= limit,
        }
    }
}

/// How the order is to be priced. These are the FIX `OrdType(40)` values the venue
/// trades; every other standard value is rejected rather than reinterpreted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderType {
    /// `OrdType(40) = 1` — fill against whatever depth is quoted, at any price.
    Market,
    /// `OrdType(40) = 2` — fill only at `price` or better; depth worse than the
    /// limit is not eligible, so a limit order may partially fill or not fill at all.
    Limit {
        /// The taker's limit price.
        price: f64,
    },
    /// `OrdType(40) = D` — a lift of a specific streamed level at `price`. Treated
    /// as a limit at that level: the taker is entitled to the price it was shown,
    /// and to nothing worse, so a moved market produces an honest miss rather than
    /// a silent slip.
    PreviouslyQuoted {
        /// The streamed level being lifted.
        price: f64,
    },
}

impl OrderType {
    /// The FIX `OrdType(40)` byte.
    #[must_use]
    pub const fn fix_byte(self) -> u8 {
        match self {
            OrderType::Market => b'1',
            OrderType::Limit { .. } => b'2',
            OrderType::PreviouslyQuoted { .. } => b'D',
        }
    }

    /// The effective price ceiling/floor this order type imposes, if any.
    #[must_use]
    pub const fn limit_price(self) -> Option<f64> {
        match self {
            OrderType::Market => None,
            OrderType::Limit { price } | OrderType::PreviouslyQuoted { price } => Some(price),
        }
    }
}

/// The FIX `TimeInForce(59)` values, in the standard encoding.
///
/// The full standard set is modelled — including the ones this venue does not
/// support — so an inbound order naming a resting TIF is **recognised and rejected
/// with the right reason** rather than falling into an unknown-value bucket that
/// looks identical to a malformed message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeInForce {
    /// `0` — Day.
    Day,
    /// `1` — Good Till Cancel.
    GoodTillCancel,
    /// `2` — At the Opening.
    AtTheOpening,
    /// `3` — Immediate Or Cancel: take whatever is available now, cancel the rest.
    ImmediateOrCancel,
    /// `4` — Fill Or Kill: the full quantity, or nothing.
    FillOrKill,
    /// `5` — Good Till Crossing.
    GoodTillCrossing,
    /// `6` — Good Till Date.
    GoodTillDate,
    /// `7` — At the Close.
    AtTheClose,
}

impl TimeInForce {
    /// The FIX `TimeInForce(59)` byte.
    #[must_use]
    pub const fn fix_byte(self) -> u8 {
        match self {
            TimeInForce::Day => b'0',
            TimeInForce::GoodTillCancel => b'1',
            TimeInForce::AtTheOpening => b'2',
            TimeInForce::ImmediateOrCancel => b'3',
            TimeInForce::FillOrKill => b'4',
            TimeInForce::GoodTillCrossing => b'5',
            TimeInForce::GoodTillDate => b'6',
            TimeInForce::AtTheClose => b'7',
        }
    }

    /// Recover a TIF from its FIX byte. `None` for a byte outside the standard set.
    #[must_use]
    pub const fn from_fix_byte(b: u8) -> Option<Self> {
        match b {
            b'0' => Some(TimeInForce::Day),
            b'1' => Some(TimeInForce::GoodTillCancel),
            b'2' => Some(TimeInForce::AtTheOpening),
            b'3' => Some(TimeInForce::ImmediateOrCancel),
            b'4' => Some(TimeInForce::FillOrKill),
            b'5' => Some(TimeInForce::GoodTillCrossing),
            b'6' => Some(TimeInForce::GoodTillDate),
            b'7' => Some(TimeInForce::AtTheClose),
            _ => None,
        }
    }

    /// Whether this TIF asks the venue to **rest** unfilled quantity. A quote-driven
    /// counterparty cannot honour any of these (see the module docs).
    #[must_use]
    pub const fn rests(self) -> bool {
        !matches!(
            self,
            TimeInForce::ImmediateOrCancel | TimeInForce::FillOrKill
        )
    }

    /// A short stable label for reasons, logs and operator output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            TimeInForce::Day => "DAY",
            TimeInForce::GoodTillCancel => "GTC",
            TimeInForce::AtTheOpening => "OPG",
            TimeInForce::ImmediateOrCancel => "IOC",
            TimeInForce::FillOrKill => "FOK",
            TimeInForce::GoodTillCrossing => "GTX",
            TimeInForce::GoodTillDate => "GTD",
            TimeInForce::AtTheClose => "CLS",
        }
    }
}

/// One order presented to a simulated counterparty.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderRequest {
    /// The taker's `ClOrdID(11)` — echoed on every report so the taker can correlate.
    pub cl_ord_id: String,
    /// The canonical `instrument_id` / FIX `Symbol(55)` being traded.
    pub instrument_id: String,
    /// The taker's direction.
    pub side: Side,
    /// The requested quantity, in the instrument's quoted size units.
    pub quantity: f64,
    /// How the order is priced.
    pub ord_type: OrderType,
    /// The order's time-in-force.
    pub tif: TimeInForce,
}

/// One quoted price level: `size` available at `price`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthLevel {
    /// The level's price, in the instrument's quote convention.
    pub price: f64,
    /// The quantity firm at that price.
    pub size: f64,
}

/// One counterparty's quoted depth on **one side**, ordered best price first.
///
/// Built by [`DepthLadder::from_quote`] from that counterparty's streamed
/// top-of-book plus its own roster profile, so the ladder a taker walks is the same
/// liquidity the composite was built from — the sim never fills against depth it
/// never showed.
#[derive(Debug, Clone, PartialEq)]
pub struct DepthLadder {
    side: Side,
    levels: Vec<DepthLevel>,
}

impl DepthLadder {
    /// Build a ladder directly from explicit levels (best price first). Levels that
    /// are non-finite, non-positive in size, or out of price order are dropped:
    /// liquidity that improves as you go deeper is not liquidity, it is a bug, and
    /// admitting it would let an order fill better the more it took.
    #[must_use]
    pub fn new(side: Side, levels: impl IntoIterator<Item = DepthLevel>) -> Self {
        let mut kept: Vec<DepthLevel> = Vec::new();
        for lvl in levels {
            if !(lvl.price.is_finite() && lvl.size.is_finite() && lvl.size > 0.0) {
                continue;
            }
            if let Some(prev) = kept.last() {
                // Each successive level must be no better than the one in front.
                let ok = match side {
                    Side::Buy => lvl.price >= prev.price,
                    Side::Sell => lvl.price <= prev.price,
                };
                if !ok {
                    continue;
                }
            }
            kept.push(lvl);
        }
        Self { side, levels: kept }
    }

    /// Build the ladder a taker walks from the counterparty's own streamed
    /// top-of-book (`touch_price` firm for `touch_size`), its `half_spread`, and its
    /// roster `profile`.
    ///
    /// The touch is level 1. Each level behind it concedes
    /// [`depth_step_half_spreads`](SimLpProfile::depth_step_half_spreads) × the
    /// counterparty's own half-spread in price and retains
    /// [`depth_decay`](SimLpProfile::depth_decay) of the size in front of it. A
    /// counterparty showing `depth_levels = 0` is firm at the touch and nothing more.
    ///
    /// If `lot_size` is `Some`, every level's size is floored to a whole multiple of
    /// it (a listed contract does not trade in fractions of a contract) and levels
    /// that floor to zero are dropped.
    #[must_use]
    pub fn from_quote(
        side: Side,
        touch_price: f64,
        touch_size: f64,
        half_spread: f64,
        profile: &SimLpProfile,
        lot_size: Option<f64>,
    ) -> Self {
        let step = (profile.depth_step_half_spreads * half_spread).abs();
        let sign = match side {
            // Buying, each deeper level is a HIGHER (worse) offer.
            Side::Buy => 1.0,
            // Selling, each deeper level is a LOWER (worse) bid.
            Side::Sell => -1.0,
        };
        let mut levels = Vec::with_capacity(usize::from(profile.depth_levels) + 1);
        let mut size = touch_size;
        for i in 0..=u32::from(profile.depth_levels) {
            let price = touch_price + sign * step * f64::from(i);
            let firm = match lot_size {
                Some(lot) if lot > 0.0 => (size / lot).floor() * lot,
                _ => size,
            };
            if firm > 0.0 {
                levels.push(DepthLevel { price, size: firm });
            }
            size *= profile.depth_decay;
        }
        Self::new(side, levels)
    }

    /// The side this ladder quotes for the taker.
    #[must_use]
    pub fn side(&self) -> Side {
        self.side
    }

    /// The quoted levels, best price first.
    #[must_use]
    pub fn levels(&self) -> &[DepthLevel] {
        &self.levels
    }

    /// The best (touch) price, or `None` for an empty ladder.
    #[must_use]
    pub fn touch(&self) -> Option<f64> {
        self.levels.first().map(|l| l.price)
    }

    /// Total quantity available at or better than `limit` (all of it when `None`).
    #[must_use]
    pub fn available(&self, limit: Option<f64>) -> f64 {
        self.eligible(limit).map(|l| l.size).sum()
    }

    /// The levels eligible under `limit`, best first. Because levels are ordered
    /// worst-last, the first ineligible level ends the walk.
    fn eligible(&self, limit: Option<f64>) -> impl Iterator<Item = &DepthLevel> {
        let side = self.side;
        self.levels.iter().take_while(move |l| match limit {
            None => true,
            Some(px) => side.is_marketable(l.price, px),
        })
    }
}

/// Why a counterparty refused an order outright. Every rejection carries one — the
/// venue never drops an order silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// The requested quantity was zero, negative or not a number.
    InvalidQuantity,
    /// The order named an instrument this counterparty does not quote.
    InstrumentNotQuoted,
    /// The counterparty has no live two-way for the instrument right now (its quote
    /// aged out, or it is not currently making a market).
    NoMarket,
    /// The order's limit price was absent, non-finite or non-positive.
    InvalidLimitPrice,
    /// A limit / previously-quoted order whose price is not marketable against any
    /// quoted level — the market is away from it.
    NotMarketable,
    /// A fill-or-kill order whose full quantity is not available at an eligible
    /// price. Killed in its entirety; never partially filled.
    FillOrKillUnfillable,
    /// An immediate-or-cancel order that found no eligible liquidity at all, so
    /// there was nothing to fill before cancelling the remainder.
    NoLiquidity,
    /// A time-in-force asking the venue to rest unfilled quantity. This venue is
    /// quote-driven and holds no working-order book (see the module docs).
    RestingTifUnsupported,
    /// An `OrdType(40)` outside the set this venue trades (market / limit /
    /// previously-quoted).
    OrderTypeUnsupported,
    /// The order quantity is not a whole multiple of the instrument's lot size, on
    /// a market that only trades whole lots.
    NotAWholeLot,
}

impl RejectReason {
    /// The stable machine-readable reason code carried in the report's `Text(58)`.
    /// These strings are a contract: operators and tests match on them.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            RejectReason::InvalidQuantity => "INVALID_QUANTITY",
            RejectReason::InstrumentNotQuoted => "INSTRUMENT_NOT_QUOTED",
            RejectReason::NoMarket => "NO_MARKET",
            RejectReason::InvalidLimitPrice => "INVALID_LIMIT_PRICE",
            RejectReason::NotMarketable => "NOT_MARKETABLE",
            RejectReason::FillOrKillUnfillable => "FOK_UNFILLABLE",
            RejectReason::NoLiquidity => "NO_LIQUIDITY",
            RejectReason::RestingTifUnsupported => "RESTING_TIF_UNSUPPORTED",
            RejectReason::OrderTypeUnsupported => "ORDER_TYPE_UNSUPPORTED",
            RejectReason::NotAWholeLot => "NOT_A_WHOLE_LOT",
        }
    }

    /// A one-line operator-facing explanation.
    #[must_use]
    pub const fn detail(self) -> &'static str {
        match self {
            RejectReason::InvalidQuantity => "OrderQty(38) must be a positive, finite quantity",
            RejectReason::InstrumentNotQuoted => "this counterparty does not quote that Symbol(55)",
            RejectReason::NoMarket => "no live two-way for that instrument right now",
            RejectReason::InvalidLimitPrice => "Price(44) must be a positive, finite price",
            RejectReason::NotMarketable => "the limit price is away from the quoted market",
            RejectReason::FillOrKillUnfillable => {
                "the full quantity is not available at an eligible price; killed in full"
            }
            RejectReason::NoLiquidity => "no eligible quoted liquidity to fill against",
            RejectReason::RestingTifUnsupported => {
                "quote-driven venue: only IOC(3) and FOK(4) are accepted, \
                 a resting TimeInForce(59) cannot be honoured"
            }
            RejectReason::OrderTypeUnsupported => {
                "OrdType(40) must be market(1), limit(2) or previously-quoted(D)"
            }
            RejectReason::NotAWholeLot => {
                "this instrument trades in whole lots only; OrderQty(38) is not a whole multiple"
            }
        }
    }
}

/// Why an immediate-or-cancel order did not fill in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialReason {
    /// The quoted depth ran out before the order did.
    DepthExhausted,
    /// The eligible depth ran out because the deeper levels were through the
    /// order's limit price.
    LimitReached,
}

impl PartialReason {
    /// The stable machine-readable reason code carried in the report's `Text(58)`.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            PartialReason::DepthExhausted => "IOC_DEPTH_EXHAUSTED",
            PartialReason::LimitReached => "IOC_LIMIT_REACHED",
        }
    }
}

/// What a counterparty did with an order.
#[derive(Debug, Clone, PartialEq)]
pub enum Execution {
    /// The full requested quantity traded.
    Filled(Fill),
    /// Part of the requested quantity traded and the remainder was cancelled (only
    /// reachable under [`TimeInForce::ImmediateOrCancel`]).
    PartiallyFilled {
        /// The traded part.
        fill: Fill,
        /// The cancelled remainder (`requested − filled`), strictly positive.
        cancelled: f64,
        /// Why the order did not complete.
        reason: PartialReason,
    },
    /// Nothing traded.
    Rejected(RejectReason),
}

impl Execution {
    /// The quantity that traded (`0.0` on a rejection).
    #[must_use]
    pub fn filled_quantity(&self) -> f64 {
        match self {
            Execution::Filled(f) | Execution::PartiallyFilled { fill: f, .. } => f.quantity,
            Execution::Rejected(_) => 0.0,
        }
    }

    /// The size-weighted average traded price (`0.0` on a rejection).
    #[must_use]
    pub fn average_price(&self) -> f64 {
        match self {
            Execution::Filled(f) | Execution::PartiallyFilled { fill: f, .. } => f.average_price,
            Execution::Rejected(_) => 0.0,
        }
    }

    /// The stable machine-readable reason code for the outcome, or `None` for a
    /// clean complete fill.
    #[must_use]
    pub fn reason_code(&self) -> Option<&'static str> {
        match self {
            Execution::Filled(_) => None,
            Execution::PartiallyFilled { reason, .. } => Some(reason.code()),
            Execution::Rejected(r) => Some(r.code()),
        }
    }
}

/// The traded part of an execution.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    /// The quantity that traded (strictly positive).
    pub quantity: f64,
    /// The size-weighted average price across every level consumed.
    pub average_price: f64,
    /// The worst price touched — how far into the book the order reached.
    pub worst_price: f64,
    /// Each `(price, quantity)` consumed, best level first. The audit trail of the
    /// fill: a taker can reconcile the average price without trusting it.
    pub legs: Vec<DepthLevel>,
}

/// The instrument-level trading rules a counterparty applies before it matches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VenueRules {
    /// The minimum tradeable unit, if the instrument trades in whole lots only
    /// (a listed contract). `None` for an instrument that trades any quantity.
    pub lot_size: Option<f64>,
}

impl VenueRules {
    /// An over-the-counter instrument: any quantity trades.
    #[must_use]
    pub const fn over_the_counter() -> Self {
        Self { lot_size: None }
    }

    /// A listed instrument that trades in whole multiples of `lot_size` only.
    #[must_use]
    pub const fn whole_lots(lot_size: f64) -> Self {
        Self {
            lot_size: Some(lot_size),
        }
    }
}

impl Default for VenueRules {
    fn default() -> Self {
        Self::over_the_counter()
    }
}

/// Relative tolerance for the whole-lot check, so a quantity assembled in floating
/// point from a DV01 ratio is not rejected for being one ulp off a whole lot.
const LOT_TOLERANCE: f64 = 1e-9;

/// Match `order` against a counterparty's quoted `ladder` under `rules`.
///
/// Pure and total: every input produces exactly one [`Execution`], and every
/// non-fill carries a reason. The `ladder` must be the counterparty's depth on the
/// side the order takes — [`DepthLadder::side`] is asserted against the order's side
/// so a mis-wired caller is rejected rather than filled backwards.
#[must_use]
pub fn execute(order: &OrderRequest, ladder: &DepthLadder, rules: VenueRules) -> Execution {
    // ---- validate the order itself before touching the market -----------------
    if !(order.quantity.is_finite() && order.quantity > 0.0) {
        return Execution::Rejected(RejectReason::InvalidQuantity);
    }
    if let Some(lot) = rules.lot_size.filter(|l| *l > 0.0) {
        let lots = order.quantity / lot;
        if (lots - lots.round()).abs() > LOT_TOLERANCE * lots.abs().max(1.0) {
            return Execution::Rejected(RejectReason::NotAWholeLot);
        }
    }
    if order.tif.rests() {
        return Execution::Rejected(RejectReason::RestingTifUnsupported);
    }
    let limit = order.ord_type.limit_price();
    if let Some(px) = limit
        && !(px.is_finite() && px > 0.0)
    {
        return Execution::Rejected(RejectReason::InvalidLimitPrice);
    }

    // ---- the market must exist and be on the requested side -------------------
    if ladder.side != order.side || ladder.levels.is_empty() {
        return Execution::Rejected(RejectReason::NoMarket);
    }

    // A limit that is not marketable against even the TOUCH is a clean miss, and
    // is distinguishable from "there was liquidity, just not enough".
    if let (Some(px), Some(touch)) = (limit, ladder.touch())
        && !order.side.is_marketable(touch, px)
    {
        return Execution::Rejected(RejectReason::NotMarketable);
    }

    // ---- walk the eligible depth ---------------------------------------------
    let available = ladder.available(limit);
    match order.tif {
        // FOK is all-or-nothing: if the WHOLE quantity is not available at an
        // eligible price the order is killed entirely — never partially filled.
        TimeInForce::FillOrKill => {
            if available + LOT_TOLERANCE * available.max(1.0) < order.quantity {
                return Execution::Rejected(RejectReason::FillOrKillUnfillable);
            }
            let fill = consume(ladder, limit, order.quantity);
            match fill {
                Some(f) => Execution::Filled(f),
                None => Execution::Rejected(RejectReason::NoLiquidity),
            }
        }
        // IOC takes what is there now and cancels the rest — a PARTIAL fill is the
        // normal outcome against depth smaller than the clip.
        TimeInForce::ImmediateOrCancel => {
            if available <= 0.0 {
                return Execution::Rejected(RejectReason::NoLiquidity);
            }
            let want = order.quantity.min(available);
            let Some(fill) = consume(ladder, limit, want) else {
                return Execution::Rejected(RejectReason::NoLiquidity);
            };
            let cancelled = order.quantity - fill.quantity;
            if cancelled <= LOT_TOLERANCE * order.quantity.max(1.0) {
                return Execution::Filled(fill);
            }
            // Distinguish "the book ran out" from "the LIMIT ran out" — the taker
            // needs to know whether to widen its price or split its clip.
            let reason = if limit.is_some() && ladder.available(None) > available {
                PartialReason::LimitReached
            } else {
                PartialReason::DepthExhausted
            };
            Execution::PartiallyFilled {
                fill,
                cancelled,
                reason,
            }
        }
        // Unreachable: every resting TIF was rejected above. Kept exhaustive (no
        // wildcard) so adding a TIF to the enum forces a decision here.
        TimeInForce::Day
        | TimeInForce::GoodTillCancel
        | TimeInForce::AtTheOpening
        | TimeInForce::GoodTillCrossing
        | TimeInForce::GoodTillDate
        | TimeInForce::AtTheClose => Execution::Rejected(RejectReason::RestingTifUnsupported),
    }
}

/// Consume up to `want` from the eligible levels, best first. `None` when nothing
/// was consumable.
fn consume(ladder: &DepthLadder, limit: Option<f64>, want: f64) -> Option<Fill> {
    let mut remaining = want;
    let mut legs: Vec<DepthLevel> = Vec::new();
    let mut notional = 0.0;
    for level in ladder.eligible(limit) {
        if remaining <= 0.0 {
            break;
        }
        let take = remaining.min(level.size);
        if take <= 0.0 {
            continue;
        }
        legs.push(DepthLevel {
            price: level.price,
            size: take,
        });
        notional += level.price * take;
        remaining -= take;
    }
    let quantity = want - remaining.max(0.0);
    if quantity <= 0.0 || legs.is_empty() {
        return None;
    }
    let worst_price = legs.last().map_or(f64::NAN, |l| l.price);
    Some(Fill {
        quantity,
        average_price: notional / quantity,
        worst_price,
        legs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roster::{LISTED_ROSTER, OTC_ROSTER, profile_by_id};

    fn order(side: Side, qty: f64, ord_type: OrderType, tif: TimeInForce) -> OrderRequest {
        OrderRequest {
            cl_ord_id: "T-1".into(),
            instrument_id: "912810TZ1".into(),
            side,
            quantity: qty,
            ord_type,
            tif,
        }
    }

    /// A hand-built offer ladder: 1m at 100.00, 1m at 100.10, 1m at 100.20.
    fn offers() -> DepthLadder {
        DepthLadder::new(
            Side::Buy,
            [
                DepthLevel {
                    price: 100.00,
                    size: 1_000_000.0,
                },
                DepthLevel {
                    price: 100.10,
                    size: 1_000_000.0,
                },
                DepthLevel {
                    price: 100.20,
                    size: 1_000_000.0,
                },
            ],
        )
    }

    // ---------------------------------------------------------------- FOK ----

    /// FOK is all-or-nothing. A clip inside the quoted depth fills in FULL, at the
    /// size-weighted average of the levels it consumed.
    #[test]
    fn fok_fills_the_whole_clip_or_nothing() {
        let e = execute(
            &order(
                Side::Buy,
                2_500_000.0,
                OrderType::Market,
                TimeInForce::FillOrKill,
            ),
            &offers(),
            VenueRules::over_the_counter(),
        );
        let Execution::Filled(f) = e else {
            panic!("expected a complete fill, got {e:?}");
        };
        assert_eq!(f.quantity, 2_500_000.0);
        // 1m @ 100.00 + 1m @ 100.10 + 0.5m @ 100.20 = 250,200,000 over 2.5m = 100.08.
        assert!(
            (f.average_price - 100.08).abs() < 1e-9,
            "{}",
            f.average_price
        );
        assert_eq!(f.worst_price, 100.20);
        assert_eq!(f.legs.len(), 3);
    }

    /// A FOK larger than the whole quoted book is KILLED — never partially filled.
    /// This is the property that distinguishes FOK from IOC and the one a naive
    /// "fill what you can" implementation gets wrong.
    #[test]
    fn fok_beyond_the_book_is_killed_entirely_never_partial() {
        let e = execute(
            &order(
                Side::Buy,
                9_000_000.0,
                OrderType::Market,
                TimeInForce::FillOrKill,
            ),
            &offers(),
            VenueRules::over_the_counter(),
        );
        assert_eq!(e, Execution::Rejected(RejectReason::FillOrKillUnfillable));
        assert_eq!(e.filled_quantity(), 0.0);
        assert_eq!(e.reason_code(), Some("FOK_UNFILLABLE"));
    }

    /// A FOK whose LIMIT gates away the depth it would need is killed for the same
    /// reason — the gating is by eligible liquidity, not total liquidity.
    #[test]
    fn fok_is_killed_when_its_limit_gates_away_the_depth_it_needs() {
        // Only the 100.00 level is eligible ⇒ 1m available against a 2m clip.
        let e = execute(
            &order(
                Side::Buy,
                2_000_000.0,
                OrderType::Limit { price: 100.05 },
                TimeInForce::FillOrKill,
            ),
            &offers(),
            VenueRules::over_the_counter(),
        );
        assert_eq!(e, Execution::Rejected(RejectReason::FillOrKillUnfillable));
    }

    // ---------------------------------------------------------------- IOC ----

    /// IOC fills what is available NOW and cancels the remainder — a PARTIAL fill is
    /// the normal outcome, and it is reported as one with the reason attached.
    #[test]
    fn ioc_partially_fills_and_cancels_the_remainder() {
        let e = execute(
            &order(
                Side::Buy,
                5_000_000.0,
                OrderType::Market,
                TimeInForce::ImmediateOrCancel,
            ),
            &offers(),
            VenueRules::over_the_counter(),
        );
        let Execution::PartiallyFilled {
            fill,
            cancelled,
            reason,
        } = e
        else {
            panic!("expected a partial fill, got {e:?}");
        };
        assert_eq!(fill.quantity, 3_000_000.0);
        assert_eq!(cancelled, 2_000_000.0);
        assert_eq!(reason, PartialReason::DepthExhausted);
        assert!((fill.average_price - 100.10).abs() < 1e-9);
    }

    /// An IOC that fits inside the book is a COMPLETE fill, not a partial one with a
    /// zero remainder.
    #[test]
    fn ioc_inside_the_book_is_a_complete_fill() {
        let e = execute(
            &order(
                Side::Buy,
                800_000.0,
                OrderType::Market,
                TimeInForce::ImmediateOrCancel,
            ),
            &offers(),
            VenueRules::over_the_counter(),
        );
        let Execution::Filled(f) = e else {
            panic!("expected a complete fill, got {e:?}");
        };
        assert_eq!(f.quantity, 800_000.0);
        assert_eq!(f.average_price, 100.00);
    }

    /// An IOC stopped by its LIMIT rather than by the end of the book reports the
    /// distinct reason — the taker needs to know whether to widen or to split.
    #[test]
    fn ioc_stopped_by_its_limit_says_so() {
        let e = execute(
            &order(
                Side::Buy,
                3_000_000.0,
                OrderType::Limit { price: 100.10 },
                TimeInForce::ImmediateOrCancel,
            ),
            &offers(),
            VenueRules::over_the_counter(),
        );
        let Execution::PartiallyFilled {
            fill,
            cancelled,
            reason,
        } = e
        else {
            panic!("expected a partial fill, got {e:?}");
        };
        assert_eq!(fill.quantity, 2_000_000.0);
        assert_eq!(cancelled, 1_000_000.0);
        assert_eq!(reason, PartialReason::LimitReached);
        assert_eq!(fill.worst_price, 100.10);
    }

    // -------------------------------------------------------------- limits ----

    /// A limit away from the market does not fill at all, and says why.
    #[test]
    fn a_limit_through_the_touch_is_an_honest_miss() {
        let e = execute(
            &order(
                Side::Buy,
                1_000_000.0,
                OrderType::Limit { price: 99.50 },
                TimeInForce::ImmediateOrCancel,
            ),
            &offers(),
            VenueRules::over_the_counter(),
        );
        assert_eq!(e, Execution::Rejected(RejectReason::NotMarketable));
    }

    /// A limit order never fills WORSE than its limit, at any quantity — the
    /// invariant that makes a limit a limit.
    #[test]
    fn a_limit_order_never_trades_through_its_price() {
        for qty in [100_000.0, 1_500_000.0, 2_900_000.0, 10_000_000.0] {
            let limit = 100.10;
            let e = execute(
                &order(
                    Side::Buy,
                    qty,
                    OrderType::Limit { price: limit },
                    TimeInForce::ImmediateOrCancel,
                ),
                &offers(),
                VenueRules::over_the_counter(),
            );
            if let Execution::Filled(f) | Execution::PartiallyFilled { fill: f, .. } = &e {
                assert!(
                    f.worst_price <= limit + 1e-12,
                    "qty {qty} traded at {} through a {limit} limit",
                    f.worst_price
                );
            }
        }
    }

    /// A previously-quoted lift is priced at the level it was shown: if the market
    /// has moved away, it misses rather than slipping silently.
    #[test]
    fn a_previously_quoted_lift_does_not_slip() {
        let stale = OrderType::PreviouslyQuoted { price: 99.90 };
        let e = execute(
            &order(Side::Buy, 500_000.0, stale, TimeInForce::FillOrKill),
            &offers(),
            VenueRules::over_the_counter(),
        );
        assert_eq!(e, Execution::Rejected(RejectReason::NotMarketable));

        let current = OrderType::PreviouslyQuoted { price: 100.00 };
        let e = execute(
            &order(Side::Buy, 500_000.0, current, TimeInForce::FillOrKill),
            &offers(),
            VenueRules::over_the_counter(),
        );
        assert_eq!(e.filled_quantity(), 500_000.0);
        assert_eq!(e.average_price(), 100.00);
    }

    /// The sell side hits the bid, and "better" means HIGHER — the mirror image.
    #[test]
    fn selling_walks_the_bid_downwards() {
        let bids = DepthLadder::new(
            Side::Sell,
            [
                DepthLevel {
                    price: 99.90,
                    size: 1_000_000.0,
                },
                DepthLevel {
                    price: 99.80,
                    size: 1_000_000.0,
                },
            ],
        );
        let e = execute(
            &order(
                Side::Sell,
                1_500_000.0,
                OrderType::Limit { price: 99.85 },
                TimeInForce::ImmediateOrCancel,
            ),
            &bids,
            VenueRules::over_the_counter(),
        );
        let Execution::PartiallyFilled { fill, reason, .. } = e else {
            panic!("expected a partial fill");
        };
        assert_eq!(fill.quantity, 1_000_000.0);
        assert_eq!(fill.average_price, 99.90);
        assert_eq!(reason, PartialReason::LimitReached);
    }

    // ------------------------------------------------------- venue rules ----

    /// Every resting TIF is REJECTED with the reason, not silently downgraded to an
    /// IOC. Fabricating a cancel the taker never asked for is worse than refusing.
    #[test]
    fn every_resting_tif_is_rejected_with_a_reason() {
        for tif in [
            TimeInForce::Day,
            TimeInForce::GoodTillCancel,
            TimeInForce::AtTheOpening,
            TimeInForce::GoodTillCrossing,
            TimeInForce::GoodTillDate,
            TimeInForce::AtTheClose,
        ] {
            assert!(tif.rests(), "{} should rest", tif.label());
            let e = execute(
                &order(Side::Buy, 100_000.0, OrderType::Market, tif),
                &offers(),
                VenueRules::over_the_counter(),
            );
            assert_eq!(
                e,
                Execution::Rejected(RejectReason::RestingTifUnsupported),
                "{} was not refused",
                tif.label()
            );
        }
        assert!(!TimeInForce::ImmediateOrCancel.rests());
        assert!(!TimeInForce::FillOrKill.rests());
    }

    /// A whole-lot market refuses a fractional clip rather than rounding it.
    #[test]
    fn a_whole_lot_market_refuses_a_fractional_clip() {
        let rules = VenueRules::whole_lots(100_000.0);
        let e = execute(
            &order(
                Side::Buy,
                150_000.0,
                OrderType::Market,
                TimeInForce::FillOrKill,
            ),
            &offers(),
            rules,
        );
        assert_eq!(e, Execution::Rejected(RejectReason::NotAWholeLot));

        // A whole number of lots trades normally.
        let e = execute(
            &order(
                Side::Buy,
                200_000.0,
                OrderType::Market,
                TimeInForce::FillOrKill,
            ),
            &offers(),
            rules,
        );
        assert_eq!(e.filled_quantity(), 200_000.0);
    }

    /// Malformed orders are refused with a specific reason each.
    #[test]
    fn malformed_orders_are_refused_specifically() {
        for (qty, want) in [
            (0.0, RejectReason::InvalidQuantity),
            (-1.0, RejectReason::InvalidQuantity),
            (f64::NAN, RejectReason::InvalidQuantity),
        ] {
            let e = execute(
                &order(Side::Buy, qty, OrderType::Market, TimeInForce::FillOrKill),
                &offers(),
                VenueRules::over_the_counter(),
            );
            assert_eq!(e, Execution::Rejected(want));
        }
        let e = execute(
            &order(
                Side::Buy,
                1.0,
                OrderType::Limit { price: -1.0 },
                TimeInForce::FillOrKill,
            ),
            &offers(),
            VenueRules::over_the_counter(),
        );
        assert_eq!(e, Execution::Rejected(RejectReason::InvalidLimitPrice));

        // A ladder for the wrong side is a mis-wired caller, not a fill.
        let e = execute(
            &order(Side::Sell, 1.0, OrderType::Market, TimeInForce::FillOrKill),
            &offers(),
            VenueRules::over_the_counter(),
        );
        assert_eq!(e, Execution::Rejected(RejectReason::NoMarket));

        // An empty ladder has no market at all.
        let empty = DepthLadder::new(Side::Buy, []);
        let e = execute(
            &order(Side::Buy, 1.0, OrderType::Market, TimeInForce::FillOrKill),
            &empty,
            VenueRules::over_the_counter(),
        );
        assert_eq!(e, Execution::Rejected(RejectReason::NoMarket));
    }

    // -------------------------------------------------------- per-LP depth ----

    /// The whole point of the roster: the SAME order genuinely fills different
    /// amounts on different counterparties, because their depth shapes differ. This
    /// is what makes the LP panel meaningful rather than decorative.
    #[test]
    fn different_counterparties_fill_different_amounts_of_the_same_order() {
        let base_size = 1_000_000.0;
        let base_spread = 2.0e-2;
        let seed = 0x1234_5678u64;
        let clip = 6_000_000.0;

        let mut filled: Vec<(&str, f64)> = Vec::new();
        for p in OTC_ROSTER {
            let ladder = DepthLadder::from_quote(
                Side::Buy,
                100.0,
                p.firm_size(base_size, seed),
                p.half_spread(base_spread, seed),
                p,
                None,
            );
            let e = execute(
                &order(
                    Side::Buy,
                    clip,
                    OrderType::Market,
                    TimeInForce::ImmediateOrCancel,
                ),
                &ladder,
                VenueRules::over_the_counter(),
            );
            filled.push((p.id, e.filled_quantity()));
        }

        // Every counterparty fills SOMETHING, and no two fill the same amount.
        for (id, q) in &filled {
            assert!(*q > 0.0, "{id} filled nothing");
        }
        for (i, (ida, qa)) in filled.iter().enumerate() {
            for (idb, qb) in &filled[i + 1..] {
                assert!(
                    (qa - qb).abs() > 1.0,
                    "{ida} and {idb} both filled {qa} — the panel is not differentiated"
                );
            }
        }

        // The deep principal dealer provides the most; the thin platform the least.
        let deepest = filled
            .iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .expect("non-empty");
        assert_eq!(
            deepest.0, "citigroup-sim",
            "the deepest book is not the dealer's"
        );
    }

    /// A ladder built from a quote is monotonically worse and monotonically thinner
    /// as it goes deeper — the structural invariant the walk relies on.
    #[test]
    fn a_derived_ladder_degrades_monotonically() {
        for p in OTC_ROSTER.iter().chain(LISTED_ROSTER.iter()) {
            for side in [Side::Buy, Side::Sell] {
                let ladder = DepthLadder::from_quote(side, 100.0, 1_000_000.0, 2.0e-2, p, None);
                assert!(!ladder.levels().is_empty(), "{}: empty ladder", p.id);
                assert_eq!(ladder.levels().len(), usize::from(p.depth_levels) + 1);
                for w in ladder.levels().windows(2) {
                    let (a, b) = (w[0], w[1]);
                    match side {
                        Side::Buy => assert!(b.price > a.price, "{}: offers improve", p.id),
                        Side::Sell => assert!(b.price < a.price, "{}: bids improve", p.id),
                    }
                    assert!(b.size <= a.size, "{}: depth grows behind the touch", p.id);
                }
            }
        }
    }

    /// A listed ladder is quoted in WHOLE lots at every level — a level that would
    /// floor to a fraction of a contract is dropped, never rounded up.
    #[test]
    fn a_listed_ladder_quotes_whole_lots_at_every_level() {
        let p = profile_by_id(LISTED_ROSTER, "cme-sim").expect("listed roster");
        let lot = 100_000.0;
        let ladder = DepthLadder::from_quote(Side::Buy, 110.5, 4_650_000.0, 0.015625, p, Some(lot));
        assert!(!ladder.levels().is_empty());
        for l in ladder.levels() {
            let lots = l.size / lot;
            assert!(
                (lots - lots.round()).abs() < 1e-9 && lots >= 1.0,
                "level {l:?} is not a whole number of lots"
            );
        }
    }

    /// A ladder never admits liquidity that improves with depth, whatever it is
    /// handed — otherwise a bigger order could fill at a better average price.
    #[test]
    fn out_of_order_levels_are_dropped_not_reordered() {
        let l = DepthLadder::new(
            Side::Buy,
            [
                DepthLevel {
                    price: 100.0,
                    size: 1.0,
                },
                // Better than the touch: impossible behind it, so dropped.
                DepthLevel {
                    price: 99.0,
                    size: 5.0,
                },
                DepthLevel {
                    price: 100.5,
                    size: 2.0,
                },
                // Non-positive size and non-finite price are dropped too.
                DepthLevel {
                    price: 101.0,
                    size: 0.0,
                },
                DepthLevel {
                    price: f64::NAN,
                    size: 1.0,
                },
            ],
        );
        assert_eq!(l.levels().len(), 2);
        assert_eq!(l.available(None), 3.0);
        assert_eq!(l.touch(), Some(100.0));
    }

    /// FIX byte round-trips: every value this venue names maps to and from the
    /// standard encoding, and an out-of-set byte is `None` rather than a default.
    #[test]
    fn fix_encodings_round_trip_and_reject_the_unknown() {
        for tif in [
            TimeInForce::Day,
            TimeInForce::GoodTillCancel,
            TimeInForce::AtTheOpening,
            TimeInForce::ImmediateOrCancel,
            TimeInForce::FillOrKill,
            TimeInForce::GoodTillCrossing,
            TimeInForce::GoodTillDate,
            TimeInForce::AtTheClose,
        ] {
            assert_eq!(TimeInForce::from_fix_byte(tif.fix_byte()), Some(tif));
        }
        assert_eq!(TimeInForce::from_fix_byte(b'9'), None);
        assert_eq!(TimeInForce::from_fix_byte(b'X'), None);

        for side in [Side::Buy, Side::Sell] {
            assert_eq!(Side::from_fix_byte(side.fix_byte()), Some(side));
        }
        assert_eq!(Side::from_fix_byte(b'7'), None);

        assert_eq!(OrderType::Market.fix_byte(), b'1');
        assert_eq!(OrderType::Limit { price: 1.0 }.fix_byte(), b'2');
        assert_eq!(OrderType::PreviouslyQuoted { price: 1.0 }.fix_byte(), b'D');
    }

    /// Every reason code is unique and non-empty — they are a contract operators and
    /// tests match on.
    #[test]
    fn reason_codes_are_unique_and_populated() {
        let reasons = [
            RejectReason::InvalidQuantity,
            RejectReason::InstrumentNotQuoted,
            RejectReason::NoMarket,
            RejectReason::InvalidLimitPrice,
            RejectReason::NotMarketable,
            RejectReason::FillOrKillUnfillable,
            RejectReason::NoLiquidity,
            RejectReason::RestingTifUnsupported,
            RejectReason::OrderTypeUnsupported,
            RejectReason::NotAWholeLot,
        ];
        let codes: std::collections::HashSet<&str> = reasons.iter().map(|r| r.code()).collect();
        assert_eq!(codes.len(), reasons.len());
        for r in reasons {
            assert!(!r.code().is_empty() && !r.detail().is_empty());
        }
        let partials = [PartialReason::DepthExhausted, PartialReason::LimitReached];
        let codes: std::collections::HashSet<&str> = partials.iter().map(|r| r.code()).collect();
        assert_eq!(codes.len(), partials.len());
    }
}
