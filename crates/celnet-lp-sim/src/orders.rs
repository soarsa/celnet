//! The **order acceptor** every simulated counterparty binds — the half of a venue
//! that trades, as opposed to the half that quotes.
//!
//! [`crate::execution`] decides what a counterparty does with an order; this module
//! is how an order reaches it and how the answer gets back. It speaks the estate's
//! **existing** order contract and invents nothing: an inbound FIX
//! `NewOrderSingle(D)` is decoded through [`celnet_fix::messages::NewOrderView`],
//! matched by [`crate::execution::execute`], and answered with an
//! `ExecutionReport(8)` built by [`celnet_fix::messages::build_execution_report`] —
//! the same frames the FIX gateway already emits for a desk lift and the same ones
//! [`celnet_fix::initiator`] already parses.
//!
//! ```text
//!   taker                          simulated counterparty
//!     │  Logon(A)                        │
//!     ├─────────────────────────────────►│  session FSM (celnet_fix::session)
//!     │  NewOrderSingle(D)               │
//!     │   55=<instrument> 54=<side>      │
//!     │   38=<qty> 40=<type> 44=<px>     │
//!     │   59=<tif>                       │
//!     ├─────────────────────────────────►│  quote → DepthLadder → execute()
//!     │  ExecutionReport(8)              │
//!     │   150/39 32=<lastqty> 31=<lastpx>│
//!     │   58=<REASON_CODE: detail>       │
//!     │◄─────────────────────────────────┤
//! ```
//!
//! # Where the tradeable price comes from
//!
//! A venue must not fill against a price it never showed. The acceptor therefore
//! holds a [`QuotedMarket`] — the *same* top-of-book the simulator is streaming into
//! the server's `LpFeed` ingest — and derives the order's depth ladder from it via
//! that counterparty's own roster profile. The publisher updates the market on every
//! quote round; the acceptor reads whatever is current when an order lands. A market
//! that has never been published, or that has aged past
//! [`OrderVenue::max_quote_age`], is not tradeable and the order is declined with a
//! reason rather than filled off a stale level.
//!
//! # Every answer carries a reason
//!
//! There is no silent drop and no silent partial. A rejection carries
//! [`RejectReason::code`] + [`RejectReason::detail`] in `Text(58)`; an IOC partial
//! fill carries [`PartialReason::code`] plus the cancelled quantity. The
//! `OrdStatus(39)` distinguishes the three terminal states a quote-driven venue can
//! reach — filled, partially filled (remainder cancelled), and cancelled/rejected —
//! so a taker never has to infer an outcome from a missing message.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use celnet_fix::dictionary::MsgType;
use celnet_fix::framing::{FrameCursor, FrameEncoder};
use celnet_fix::messages::{
    self, EXEC_CANCELED, EXEC_FILLED, EXEC_REJECTED, ExecReportParams, NewOrderView,
    ORD_STATUS_CANCELED, ORD_STATUS_PARTIALLY_FILLED,
};
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig};
use celnet_fix::transport::{FrameReader, write_frame};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;

use crate::execution::{
    DepthLadder, Execution, OrderRequest, OrderType, RejectReason, Side, TimeInForce, VenueRules,
    execute,
};
use crate::lpsim::LpSimConfig;
use crate::roster::SimLpProfile;

/// One instrument's currently published two-way, as the venue will trade it.
///
/// This is deliberately the *streamed* top-of-book and not a re-derived price: the
/// level a taker sees in the composite is the level it trades against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuotedMarket {
    /// The published bid (the level a taker SELLS into).
    pub bid: f64,
    /// The published offer (the level a taker BUYS at).
    pub offer: f64,
    /// The firm size shown on the bid.
    pub bid_size: f64,
    /// The firm size shown on the offer.
    pub offer_size: f64,
    /// The observation instant (epoch nanoseconds) — the same stamp the quote
    /// carried onto the wire, so the venue ages its tradeability exactly as the
    /// consolidator ages its contribution.
    pub ts_nanos: i64,
    /// The instrument's minimum tradeable unit, if it trades in whole lots only.
    pub lot_size: Option<f64>,
}

impl QuotedMarket {
    /// The side of this market a taker on `side` executes against, as
    /// `(touch price, firm size)`.
    #[must_use]
    pub fn touch(&self, side: Side) -> (f64, f64) {
        match side {
            Side::Buy => (self.offer, self.offer_size),
            Side::Sell => (self.bid, self.bid_size),
        }
    }

    /// Whether this market is well-formed enough to trade on: finite, positive,
    /// uncrossed, and firm for something.
    #[must_use]
    pub fn is_tradeable(&self) -> bool {
        self.bid.is_finite()
            && self.offer.is_finite()
            && self.bid > 0.0
            && self.offer >= self.bid
            && self.bid_size > 0.0
            && self.offer_size > 0.0
    }
}

/// The live, shared book of published markets one simulated counterparty will trade
/// on: `instrument_id → QuotedMarket`.
///
/// The quote publisher writes it each round; the order acceptor reads it per order.
/// A `RwLock` (not a channel) because the acceptor needs the *current* market, not a
/// history — a queued quote would let an order fill at a level the market has
/// already left.
pub type LiveMarkets = Arc<RwLock<BTreeMap<String, QuotedMarket>>>;

/// A simulated counterparty's tradeable side: its identity, its trading rules, and
/// the markets it is currently showing.
#[derive(Debug, Clone)]
pub struct OrderVenue {
    profile: &'static SimLpProfile,
    /// The counterparty's own half-spread, which sets the price concession between
    /// its quoted depth levels.
    half_spread: f64,
    markets: LiveMarkets,
    /// How old a published market may be and still be tradeable (nanoseconds).
    max_quote_age_nanos: i64,
    /// The taker `CompID` this venue accepts a session from.
    taker_comp_id: String,
}

/// The default taker `CompID` a simulated counterparty accepts an order session
/// from — the Celnet edge itself, which is the only taker in the deployed topology.
pub const DEFAULT_TAKER_COMP_ID: &str = "CELNET";

/// The default tradeability horizon: a published market older than this is not
/// executable. Two seconds is comfortably longer than every roster refresh cadence
/// (the slowest is 400 ms) and far shorter than a book's staleness cutoff, so a
/// healthy counterparty is always tradeable and a wedged one stops trading long
/// before the consolidator has finished decaying it out of the composite.
pub const DEFAULT_MAX_QUOTE_AGE_NANOS: i64 = 2_000_000_000;

impl OrderVenue {
    /// Build the tradeable side of the counterparty at panel position `member` of
    /// `cfg`, sharing `markets` with that counterparty's quote publisher.
    ///
    /// Returns `None` for a panel position outside the roster.
    #[must_use]
    pub fn new(cfg: &LpSimConfig, member: usize, markets: LiveMarkets) -> Option<Self> {
        let profile = cfg.member_profile(member)?;
        Some(Self {
            profile,
            half_spread: profile.half_spread(cfg.half_spread, cfg.seed),
            markets,
            max_quote_age_nanos: DEFAULT_MAX_QUOTE_AGE_NANOS,
            taker_comp_id: DEFAULT_TAKER_COMP_ID.to_owned(),
        })
    }

    /// Build a venue directly from a profile and half-spread — the constructor a
    /// simulator with its own (non-`LpSimConfig`) fleet model uses.
    #[must_use]
    pub fn from_profile(
        profile: &'static SimLpProfile,
        half_spread: f64,
        markets: LiveMarkets,
    ) -> Self {
        Self {
            profile,
            half_spread,
            markets,
            max_quote_age_nanos: DEFAULT_MAX_QUOTE_AGE_NANOS,
            taker_comp_id: DEFAULT_TAKER_COMP_ID.to_owned(),
        }
    }

    /// Override the taker `CompID` this venue accepts a session from (chainable).
    #[must_use]
    pub fn with_taker(mut self, comp_id: impl Into<String>) -> Self {
        self.taker_comp_id = comp_id.into();
        self
    }

    /// The taker `CompID` this venue accepts a session from.
    #[must_use]
    pub fn taker_comp_id(&self) -> &str {
        &self.taker_comp_id
    }

    /// Override the tradeability horizon (chainable).
    #[must_use]
    pub fn with_max_quote_age(mut self, nanos: i64) -> Self {
        self.max_quote_age_nanos = nanos.max(0);
        self
    }

    /// The counterparty's stable connection id — the venue's `SenderCompID` and the
    /// `lp_won` a booked fill is attributed to.
    #[must_use]
    pub fn id(&self) -> &'static str {
        self.profile.id
    }

    /// The counterparty's roster profile.
    #[must_use]
    pub fn profile(&self) -> &'static SimLpProfile {
        self.profile
    }

    /// The tradeability horizon in nanoseconds.
    #[must_use]
    pub fn max_quote_age(&self) -> i64 {
        self.max_quote_age_nanos
    }

    /// The shared handle the quote publisher writes its markets into.
    #[must_use]
    pub fn markets(&self) -> &LiveMarkets {
        &self.markets
    }

    /// Publish (or refresh) the market this counterparty is showing for
    /// `instrument_id`. Called by the quote publisher on every round.
    pub fn publish(&self, instrument_id: &str, market: QuotedMarket) {
        if let Ok(mut w) = self.markets.write() {
            w.insert(instrument_id.to_owned(), market);
        }
    }

    /// Match `order` against this counterparty's currently published market at
    /// `now_nanos`. The whole tradeable behaviour of the venue, with no I/O.
    #[must_use]
    pub fn handle(&self, order: &OrderRequest, now_nanos: i64) -> Execution {
        let market = {
            let Ok(r) = self.markets.read() else {
                // A poisoned lock means a publisher panicked. Refusing to trade is
                // the only safe answer; filling off a possibly-torn book is not.
                return Execution::Rejected(RejectReason::NoMarket);
            };
            match r.get(&order.instrument_id) {
                Some(m) => *m,
                None => return Execution::Rejected(RejectReason::InstrumentNotQuoted),
            }
        };
        if !market.is_tradeable() {
            return Execution::Rejected(RejectReason::NoMarket);
        }
        // Age the published market exactly as the consolidator ages the contribution
        // it was built from: a future-dated stamp is treated as age 0, never negative.
        if now_nanos.saturating_sub(market.ts_nanos).max(0) > self.max_quote_age_nanos {
            return Execution::Rejected(RejectReason::NoMarket);
        }

        let (touch_price, touch_size) = market.touch(order.side);
        let ladder = DepthLadder::from_quote(
            order.side,
            touch_price,
            touch_size,
            self.half_spread,
            self.profile,
            market.lot_size,
        );
        let rules = market
            .lot_size
            .map_or_else(VenueRules::over_the_counter, VenueRules::whole_lots);
        execute(order, &ladder, rules)
    }
}

/// Decode an inbound `NewOrderSingle(D)` into an [`OrderRequest`].
///
/// Returns `Err(reason)` for a frame that is well-formed FIX but names something the
/// venue does not trade, so the caller can answer with that exact reason. A frame
/// missing a *mandatory* field is `Err(None)`: the session layer's `Reject(3)` is the
/// right answer to a malformed message, not a business-level execution report.
///
/// Nothing is defaulted. An absent `TimeInForce(59)` is **not** silently read as Day
/// or as IOC — it means "the taker stated none", which for a quote-driven venue is a
/// lift of the streamed level and is therefore treated as
/// [`TimeInForce::FillOrKill`]: the taker asked for a specific clip at a specific
/// shown price, and a venue that half-filled that without being told it could would
/// be inventing a cancel.
#[allow(clippy::result_large_err)]
pub fn decode_order(view: &NewOrderView<'_>) -> Result<OrderRequest, Option<RejectReason>> {
    let cl_ord_id = view.cl_ord_id().ok_or(None)?;
    let symbol = view.symbol().ok_or(None)?;
    let side_byte = view.side().ok_or(None)?;
    let quantity = view.order_qty().ok_or(None)?;

    let side = Side::from_fix_byte(side_byte).ok_or(Some(RejectReason::OrderTypeUnsupported))?;

    let ord_type = match view.ord_type() {
        // No OrdType at all: a bare lift of the streamed level. Priced at what was
        // shown when the taker also supplied a price, otherwise a market order.
        None => view
            .price()
            .map_or(OrderType::Market, |price| OrderType::PreviouslyQuoted {
                price,
            }),
        Some(messages::ord_type::MARKET) => OrderType::Market,
        Some(messages::ord_type::LIMIT) => OrderType::Limit {
            price: view.price().ok_or(Some(RejectReason::InvalidLimitPrice))?,
        },
        Some(messages::ord_type::PREVIOUSLY_QUOTED) => OrderType::PreviouslyQuoted {
            price: view.price().ok_or(Some(RejectReason::InvalidLimitPrice))?,
        },
        // A recognised-but-untraded or unknown OrdType is declined by name, never
        // reinterpreted as one of the three the venue does trade.
        Some(_) => return Err(Some(RejectReason::OrderTypeUnsupported)),
    };

    let tif = match view.time_in_force() {
        None => TimeInForce::FillOrKill,
        Some(b) => TimeInForce::from_fix_byte(b)
            // A byte outside the standard TimeInForce set is a malformed field, not
            // a business decline — the session layer answers that.
            .ok_or(None)?,
    };

    Ok(OrderRequest {
        cl_ord_id: String::from_utf8_lossy(cl_ord_id).into_owned(),
        instrument_id: String::from_utf8_lossy(symbol).into_owned(),
        side,
        quantity,
        ord_type,
        tif,
    })
}

/// The `(ExecType(150), OrdStatus(39))` pair reporting `execution`.
///
/// The distinction that matters: a **cancel** means the venue accepted the order and
/// nothing traded (an IOC that found no liquidity, a killed FOK, a limit away from
/// the market); a **reject** means the venue would not accept the order at all (a
/// malformed quantity, an unsupported order type or time-in-force, an instrument it
/// does not quote). Collapsing the two would tell a taker to fix its order when in
/// fact it only needed to try again.
#[must_use]
pub fn report_status(execution: &Execution) -> (u8, u8) {
    match execution {
        Execution::Filled(_) => (EXEC_FILLED, EXEC_FILLED),
        Execution::PartiallyFilled { .. } => (EXEC_FILLED, ORD_STATUS_PARTIALLY_FILLED),
        Execution::Rejected(reason) => match reason {
            // Accepted, but the market could not satisfy it ⇒ cancelled.
            RejectReason::NoMarket
            | RejectReason::NotMarketable
            | RejectReason::FillOrKillUnfillable
            | RejectReason::NoLiquidity => (EXEC_CANCELED, ORD_STATUS_CANCELED),
            // The venue would not accept the order in the first place ⇒ rejected.
            RejectReason::InvalidQuantity
            | RejectReason::InstrumentNotQuoted
            | RejectReason::InvalidLimitPrice
            | RejectReason::RestingTifUnsupported
            | RejectReason::OrderTypeUnsupported
            | RejectReason::NotAWholeLot => (EXEC_REJECTED, EXEC_REJECTED),
        },
    }
}

/// The `Text(58)` an execution carries: the machine-readable code, a human detail,
/// and — for a partial fill — the quantity that was cancelled.
///
/// Empty for a clean complete fill, which needs no explanation.
#[must_use]
pub fn report_text(execution: &Execution) -> String {
    match execution {
        Execution::Filled(_) => String::new(),
        Execution::PartiallyFilled {
            cancelled, reason, ..
        } => format!(
            "{}: filled at the touch, {cancelled} cancelled",
            reason.code()
        ),
        Execution::Rejected(r) => format!("{}: {}", r.code(), r.detail()),
    }
}

/// Build the `ExecutionReport(8)` answering `order` with `execution`.
///
/// `order_id` and `exec_id` are the venue-minted identities (`OrderID(37)` /
/// `ExecID(17)`); the caller mints them so the sequence is the venue's, not this
/// function's.
#[must_use]
pub fn build_report(
    hdr: &messages::Header<'_>,
    order: &OrderRequest,
    execution: &Execution,
    order_id: &[u8],
    exec_id: &[u8],
    enc: &mut FrameEncoder,
) -> Vec<u8> {
    let (exec_type, ord_status) = report_status(execution);
    let text = report_text(execution);
    let params = ExecReportParams {
        order_id,
        exec_id,
        cl_ord_id: order.cl_ord_id.as_bytes(),
        exec_type,
        ord_status,
        symbol: order.instrument_id.as_bytes(),
        side: order.side.fix_byte(),
        last_qty: execution.filled_quantity(),
        last_px: execution.average_price(),
        multileg_type: None,
        text: if text.is_empty() {
            None
        } else {
            Some(text.as_bytes())
        },
    };
    messages::build_execution_report(hdr, &params, enc)
}

/// Drive one connected taker session to completion on `stream`.
///
/// The session FSM ([`celnet_fix::session`]) owns logon, sequencing, heartbeats and
/// malformed-frame `Reject(3)`s exactly as it does for the quote acceptor; this loop
/// adds one application behaviour on top — answer a `NewOrderSingle(D)` with an
/// `ExecutionReport(8)`. Any other application message is ignored (the venue quotes
/// over the `LpFeed` ingest, not over this session).
///
/// `now_nanos` is supplied by the caller's clock so a test can drive the venue
/// deterministically without a wall clock.
///
/// # Errors
/// Propagates a socket read/write failure. A protocol-level problem is answered
/// in-band by the session FSM and never surfaces here.
pub async fn serve_orders<RW, C>(
    venue: &OrderVenue,
    stream: RW,
    sending_time: Vec<u8>,
    mut now_nanos: C,
) -> std::io::Result<()>
where
    RW: AsyncRead + AsyncWrite + Unpin,
    C: FnMut() -> i64,
{
    let cfg = SessionConfig {
        sender: venue.id().as_bytes().to_vec(),
        target: venue.taker_comp_id().as_bytes().to_vec(),
        heart_bt_int: SESSION_HEARTBEAT_SECS,
        role: Role::Acceptor,
    };
    let mut session = Session::new(cfg, InMemoryStore::new());
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = FrameReader::new(read_half);
    let mut exec_seq: u64 = 0;

    while let Some(raw) = reader.next_frame().await? {
        // Session-level handling first: logon mirror, heartbeats, resend, reject.
        // A protocol fault is answered in-band by the FSM; a frame it will not even
        // parse is dropped here exactly as the quote acceptor drops it.
        let Ok(action) = session.on_inbound(&raw, &sending_time) else {
            continue;
        };
        for frame in &action.outbound {
            write_frame(&mut write_half, frame).await?;
        }
        if action.deliver != Some(MsgType::NewOrderSingle) {
            continue;
        }
        let Ok(frame) = FrameCursor::parse(&raw) else {
            continue;
        };
        let view = NewOrderView::new(frame);

        let (order, execution) = match decode_order(&view) {
            Ok(order) => {
                let execution = venue.handle(&order, now_nanos());
                tracing::info!(
                    lp = venue.id(),
                    instrument = %order.instrument_id,
                    qty = order.quantity,
                    tif = order.tif.label(),
                    filled = execution.filled_quantity(),
                    reason = execution.reason_code().unwrap_or("FILLED"),
                    "order executed"
                );
                (order, execution)
            }
            // A well-formed order naming something the venue does not trade gets a
            // business rejection carrying the reason.
            Err(Some(reason)) => {
                tracing::warn!(lp = venue.id(), reason = reason.code(), "order declined");
                (declined_order(&view), Execution::Rejected(reason))
            }
            // A malformed order is a SESSION-level fault: the FSM's `Reject(3)` is
            // the correct answer and has already been queued above. Answering it
            // with a business `ExecutionReport(8)` would tell the taker its order
            // was considered when in fact it was never understood.
            Err(None) => continue,
        };

        exec_seq += 1;
        let report = emit_report(
            venue,
            &mut session,
            &sending_time,
            exec_seq,
            &order,
            &execution,
        );
        write_frame(&mut write_half, &report).await?;
    }
    Ok(())
}

/// The heartbeat interval the venue advertises on its logon mirror.
const SESSION_HEARTBEAT_SECS: u32 = 30;

/// Reconstruct enough of a declined order to address the rejection back to the
/// taker: the fields it did supply, with placeholders only where it supplied none.
/// The economics are irrelevant on a rejection (nothing traded), but the
/// `ClOrdID(11)` / `Symbol(55)` / `Side(54)` must echo or the taker cannot correlate.
fn declined_order(view: &NewOrderView<'_>) -> OrderRequest {
    OrderRequest {
        cl_ord_id: view
            .cl_ord_id()
            .map(|v| String::from_utf8_lossy(v).into_owned())
            .unwrap_or_default(),
        instrument_id: view
            .symbol()
            .map(|v| String::from_utf8_lossy(v).into_owned())
            .unwrap_or_default(),
        side: view
            .side()
            .and_then(Side::from_fix_byte)
            .unwrap_or(Side::Buy),
        quantity: view.order_qty().unwrap_or_default(),
        ord_type: OrderType::Market,
        tif: TimeInForce::FillOrKill,
    }
}

/// Mint the venue's `OrderID(37)` / `ExecID(17)` for execution `seq` and emit the
/// stamped `ExecutionReport(8)` through the session (so it is stored for resend).
fn emit_report(
    venue: &OrderVenue,
    session: &mut Session<InMemoryStore>,
    sending_time: &[u8],
    seq: u64,
    order: &OrderRequest,
    execution: &Execution,
) -> Vec<u8> {
    let order_id = format!("{}-O{seq:06}", venue.id());
    let exec_id = format!("{}-E{seq:06}", venue.id());
    session.send_app(sending_time, |h, e| {
        build_report(
            h,
            order,
            execution,
            order_id.as_bytes(),
            exec_id.as_bytes(),
            e,
        )
    })
}

/// Bind `bind_addr` and serve every connecting taker against `venue` until the task
/// is cancelled. Returns the bound address (useful when `bind_addr` names port `0`).
///
/// # Errors
/// Returns the bind failure. A per-connection failure is logged and the listener
/// keeps accepting — one taker dropping its socket must not take the venue down.
pub async fn run_order_acceptor(
    venue: OrderVenue,
    bind_addr: &str,
) -> std::io::Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind(bind_addr).await?;
    let local = listener.local_addr()?;
    tracing::info!(lp = venue.id(), addr = %local, "order acceptor listening");
    let handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let v = venue.clone();
                    tokio::spawn(async move {
                        tracing::info!(lp = v.id(), %peer, "taker connected");
                        if let Err(e) = serve_orders(&v, stream, fix_now(), wall_nanos).await {
                            tracing::warn!(lp = v.id(), %peer, error = %e, "taker session ended");
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!(error = %e, "order acceptor accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }
    });
    Ok((local, handle))
}

/// The valuation clock in epoch nanoseconds: wall time and nothing else.
///
/// Deliberately identical to the feed's clock (`crate::net`), which once summed two
/// advancing sources and ran at 2×, silently disabling every age-based gate that
/// consumed it. The tradeability horizon here is exactly such a gate.
#[must_use]
pub fn wall_nanos() -> i64 {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(since_epoch.as_nanos()).unwrap_or(i64::MAX)
}

/// A `SendingTime(52)` stamp for the venue's outbound frames.
#[must_use]
fn fix_now() -> Vec<u8> {
    // The session layer only requires a well-formed UTC timestamp; the venue's
    // authoritative execution instant is its own `wall_nanos` clock.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = secs / 86_400;
    let rem = secs % 86_400;
    // Days since the Unix epoch → a civil date, via the standard days-from-civil
    // inverse (Howard Hinnant, "chrono-Compatible Low-Level Date Algorithms").
    let z = i64::try_from(days).unwrap_or(0) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}{m:02}{d:02}-{:02}:{:02}:{:02}.000",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roster::{LISTED_ROSTER, OTC_ROSTER, profile_by_id};

    const NOW: i64 = 1_700_000_000_000_000_000;

    fn venue(id: &str) -> OrderVenue {
        let profile = profile_by_id(OTC_ROSTER, id)
            .or_else(|| profile_by_id(LISTED_ROSTER, id))
            .expect("roster member");
        OrderVenue::from_profile(profile, 2.0e-2, Arc::new(RwLock::new(BTreeMap::new())))
    }

    fn market() -> QuotedMarket {
        QuotedMarket {
            bid: 99.90,
            offer: 100.10,
            bid_size: 1_000_000.0,
            offer_size: 1_000_000.0,
            ts_nanos: NOW,
            lot_size: None,
        }
    }

    fn order(qty: f64, tif: TimeInForce) -> OrderRequest {
        OrderRequest {
            cl_ord_id: "C-1".into(),
            instrument_id: "912810TZ1".into(),
            side: Side::Buy,
            quantity: qty,
            ord_type: OrderType::Market,
            tif,
        }
    }

    /// A venue only trades what it has published, and says so by name when it has
    /// not — never a generic "no market".
    #[test]
    fn an_unpublished_instrument_is_declined_by_name() {
        let v = venue("jpm-sim");
        let e = v.handle(&order(100_000.0, TimeInForce::FillOrKill), NOW);
        assert_eq!(e, Execution::Rejected(RejectReason::InstrumentNotQuoted));
    }

    /// A published market inside the tradeability horizon fills; the SAME market
    /// past it does not. This is the gate that stops a wedged simulator filling off
    /// a level the market left minutes ago.
    #[test]
    fn a_stale_published_market_stops_trading() {
        let v = venue("jpm-sim");
        v.publish("912810TZ1", market());

        let fresh = v.handle(&order(100_000.0, TimeInForce::FillOrKill), NOW);
        assert_eq!(fresh.filled_quantity(), 100_000.0);
        assert_eq!(fresh.average_price(), 100.10);

        let aged = NOW + v.max_quote_age() + 1;
        let stale = v.handle(&order(100_000.0, TimeInForce::FillOrKill), aged);
        assert_eq!(stale, Execution::Rejected(RejectReason::NoMarket));

        // A future-dated stamp is age 0, never negative — the bug that once made
        // every staleness gate in the estate silently inert.
        let early = NOW - 10 * v.max_quote_age();
        assert!(
            v.handle(&order(100_000.0, TimeInForce::FillOrKill), early)
                .filled_quantity()
                > 0.0
        );
    }

    /// A crossed or malformed published market is not tradeable at any age.
    #[test]
    fn a_malformed_market_is_never_tradeable() {
        let v = venue("jpm-sim");
        for bad in [
            QuotedMarket {
                bid: 100.20,
                ..market()
            }, // crossed
            QuotedMarket {
                offer: f64::NAN,
                ..market()
            },
            QuotedMarket {
                offer_size: 0.0,
                ..market()
            },
        ] {
            v.publish("912810TZ1", bad);
            assert_eq!(
                v.handle(&order(100_000.0, TimeInForce::FillOrKill), NOW),
                Execution::Rejected(RejectReason::NoMarket),
                "{bad:?} should not be tradeable"
            );
        }
    }

    /// The SAME order against the SAME published market fills different amounts on
    /// different counterparties — the end-to-end version of the roster's promise,
    /// now going through the venue rather than a hand-built ladder.
    #[test]
    fn the_same_order_fills_differently_across_the_panel() {
        let clip = 5_000_000.0;
        let mut results: Vec<(&str, f64)> = Vec::new();
        for p in OTC_ROSTER {
            let v = venue(p.id);
            v.publish("912810TZ1", market());
            let e = v.handle(&order(clip, TimeInForce::ImmediateOrCancel), NOW);
            results.push((p.id, e.filled_quantity()));
        }
        for (i, (ida, qa)) in results.iter().enumerate() {
            assert!(*qa > 0.0, "{ida} filled nothing");
            for (idb, qb) in &results[i + 1..] {
                assert!((qa - qb).abs() > 1.0, "{ida} and {idb} both filled {qa}");
            }
        }
    }

    // ------------------------------------------------------- wire decoding ----

    fn decode(bytes: &[u8]) -> Result<OrderRequest, Option<RejectReason>> {
        let frame = FrameCursor::parse(bytes).expect("frame parses");
        decode_order(&NewOrderView::new(frame))
    }

    fn build(ord_type: u8, tif: Option<u8>, price: f64, qty: f64, side: u8) -> Vec<u8> {
        let mut enc = FrameEncoder::new();
        let hdr = messages::Header {
            sender: b"TAKER",
            target: b"jpm-sim",
            seq_num: 2,
            sending_time: b"20260814-09:00:00.000",
        };
        let p = messages::MarketOrderParams {
            cl_ord_id: b"C-9",
            symbol: b"912810TZ1",
            quote_id: b"",
            security_type: b"",
            side,
            qty,
            price,
            ord_type,
            tif,
            transact_time: b"20260814-09:00:00.000",
        };
        messages::build_new_order_by_symbol(&hdr, &p, &mut enc)
    }

    /// Every order type and TIF the venue trades survives a real encode → decode
    /// round-trip through the estate's own builders and views.
    #[test]
    fn orders_round_trip_through_the_real_wire_builders() {
        let raw = build(
            messages::ord_type::LIMIT,
            Some(messages::time_in_force::IMMEDIATE_OR_CANCEL),
            100.05,
            2_000_000.0,
            b'1',
        );
        let o = decode(&raw).expect("decodes");
        assert_eq!(o.cl_ord_id, "C-9");
        assert_eq!(o.instrument_id, "912810TZ1");
        assert_eq!(o.side, Side::Buy);
        assert_eq!(o.quantity, 2_000_000.0);
        assert_eq!(o.ord_type, OrderType::Limit { price: 100.05 });
        assert_eq!(o.tif, TimeInForce::ImmediateOrCancel);

        let raw = build(
            messages::ord_type::MARKET,
            Some(messages::time_in_force::FILL_OR_KILL),
            0.0,
            500_000.0,
            b'2',
        );
        let o = decode(&raw).expect("decodes");
        assert_eq!(o.side, Side::Sell);
        assert_eq!(
            o.ord_type,
            OrderType::Market,
            "a market order carries no price"
        );
        assert_eq!(o.tif, TimeInForce::FillOrKill);

        let raw = build(
            messages::ord_type::PREVIOUSLY_QUOTED,
            Some(messages::time_in_force::DAY),
            99.90,
            100_000.0,
            b'2',
        );
        let o = decode(&raw).expect("decodes");
        assert_eq!(o.ord_type, OrderType::PreviouslyQuoted { price: 99.90 });
        assert_eq!(
            o.tif,
            TimeInForce::Day,
            "a Day order is decoded, then declined"
        );
    }

    /// An order with no `TimeInForce(59)` is a lift of the streamed level: it is
    /// treated as FOK, NOT as an IOC. Half-filling a bare lift would invent a cancel
    /// the taker never asked for.
    #[test]
    fn an_absent_tif_is_a_lift_not_an_ioc() {
        let raw = build(
            messages::ord_type::PREVIOUSLY_QUOTED,
            None,
            100.10,
            9_000_000.0,
            b'1',
        );
        let o = decode(&raw).expect("decodes");
        assert_eq!(o.tif, TimeInForce::FillOrKill);

        let v = venue("marketaccess-sim");
        v.publish("912810TZ1", market());
        // Beyond the platform's thin book ⇒ killed in full, not half-done.
        let e = v.handle(&o, NOW);
        assert_eq!(e, Execution::Rejected(RejectReason::FillOrKillUnfillable));
    }

    /// An untraded `OrdType(40)` is declined by name rather than reinterpreted as
    /// one of the three the venue does trade.
    #[test]
    fn an_untraded_order_type_is_declined_not_reinterpreted() {
        // `3` = Stop — a real FIX value this venue does not trade.
        let raw = build(
            b'3',
            Some(messages::time_in_force::FILL_OR_KILL),
            100.0,
            1.0,
            b'1',
        );
        assert_eq!(decode(&raw), Err(Some(RejectReason::OrderTypeUnsupported)));

        // A TimeInForce byte outside the standard set is MALFORMED, not a decline:
        // the session layer answers that with a Reject(3).
        let raw = build(messages::ord_type::MARKET, Some(b'Z'), 0.0, 1.0, b'1');
        assert_eq!(decode(&raw), Err(None));

        // As is an unrecognised Side.
        let raw = build(messages::ord_type::MARKET, None, 0.0, 1.0, b'7');
        assert_eq!(decode(&raw), Err(Some(RejectReason::OrderTypeUnsupported)));
    }

    // ------------------------------------------------------------ reports ----

    /// The three terminal states map to distinct `(ExecType, OrdStatus)` pairs, and
    /// a cancel is never reported as a reject: one tells the taker to retry, the
    /// other to fix its order.
    #[test]
    fn a_cancel_is_never_reported_as_a_reject() {
        let filled = Execution::Filled(crate::execution::Fill {
            quantity: 1.0,
            average_price: 100.0,
            worst_price: 100.0,
            legs: vec![],
        });
        assert_eq!(report_status(&filled), (EXEC_FILLED, EXEC_FILLED));
        assert!(
            report_text(&filled).is_empty(),
            "a clean fill needs no text"
        );

        for accepted in [
            RejectReason::NoMarket,
            RejectReason::NotMarketable,
            RejectReason::FillOrKillUnfillable,
            RejectReason::NoLiquidity,
        ] {
            assert_eq!(
                report_status(&Execution::Rejected(accepted)),
                (EXEC_CANCELED, ORD_STATUS_CANCELED),
                "{} should cancel, not reject",
                accepted.code()
            );
        }
        for refused in [
            RejectReason::InvalidQuantity,
            RejectReason::InstrumentNotQuoted,
            RejectReason::InvalidLimitPrice,
            RejectReason::RestingTifUnsupported,
            RejectReason::OrderTypeUnsupported,
            RejectReason::NotAWholeLot,
        ] {
            assert_eq!(
                report_status(&Execution::Rejected(refused)),
                (EXEC_REJECTED, EXEC_REJECTED),
                "{} should reject, not cancel",
                refused.code()
            );
        }
    }

    /// Every non-fill carries a machine-readable reason in `Text(58)` — nothing is
    /// ever dropped silently.
    #[test]
    fn every_non_fill_carries_its_reason_on_the_wire() {
        let v = venue("marketaccess-sim");
        v.publish("912810TZ1", market());
        let mut enc = FrameEncoder::new();
        let hdr = messages::Header {
            sender: b"marketaccess-sim",
            target: b"TAKER",
            seq_num: 3,
            sending_time: b"20260814-09:00:00.000",
        };

        for (o, want) in [
            (
                order(9_000_000.0, TimeInForce::FillOrKill),
                "FOK_UNFILLABLE",
            ),
            (order(100.0, TimeInForce::Day), "RESTING_TIF_UNSUPPORTED"),
            (
                order(9_000_000.0, TimeInForce::ImmediateOrCancel),
                "IOC_DEPTH_EXHAUSTED",
            ),
        ] {
            let e = v.handle(&o, NOW);
            assert_eq!(e.reason_code(), Some(want), "wrong reason for {o:?}");
            let raw = build_report(&hdr, &o, &e, b"O-1", b"E-1", &mut enc);
            let frame = FrameCursor::parse(&raw).expect("report parses");
            assert_eq!(
                celnet_fix::dictionary::validate(&frame),
                Ok(MsgType::ExecutionReport),
                "the report is not a valid ExecutionReport"
            );
            let text = frame.get(58).expect("Text(58) present");
            assert!(
                String::from_utf8_lossy(text).starts_with(want),
                "Text(58) {:?} does not lead with {want}",
                String::from_utf8_lossy(text)
            );
            // A partial fill also reports what actually traded.
            if matches!(e, Execution::PartiallyFilled { .. }) {
                let last_qty = frame.get(32).expect("LastQty(32)");
                assert_ne!(String::from_utf8_lossy(last_qty), "0");
            }
        }
    }

    /// A clean complete fill is reported with the traded quantity and price and NO
    /// `Text(58)` — an operator scanning for text is scanning for problems.
    #[test]
    fn a_complete_fill_reports_quantity_and_price_with_no_text() {
        let v = venue("citigroup-sim");
        v.publish("912810TZ1", market());
        let o = order(1_000_000.0, TimeInForce::FillOrKill);
        let e = v.handle(&o, NOW);
        assert!(matches!(e, Execution::Filled(_)), "{e:?}");

        let mut enc = FrameEncoder::new();
        let hdr = messages::Header {
            sender: b"citigroup-sim",
            target: b"TAKER",
            seq_num: 4,
            sending_time: b"20260814-09:00:00.000",
        };
        let raw = build_report(&hdr, &o, &e, b"O-2", b"E-2", &mut enc);
        let frame = FrameCursor::parse(&raw).expect("report parses");
        assert_eq!(
            celnet_fix::dictionary::validate(&frame),
            Ok(MsgType::ExecutionReport)
        );
        assert_eq!(frame.get(58), None, "a clean fill must carry no Text(58)");
        assert_eq!(
            frame.get(150).and_then(|v| v.first().copied()),
            Some(EXEC_FILLED)
        );
        assert_eq!(frame.get(11), Some(&b"C-1"[..]));
        assert_eq!(frame.get(55), Some(&b"912810TZ1"[..]));
    }

    /// The venue's `SendingTime(52)` stamp is a well-formed FIX UTC timestamp — a
    /// malformed one is rejected by the counterparty's own dictionary validation.
    #[test]
    fn the_sending_time_stamp_is_well_formed() {
        let s = fix_now();
        let s = String::from_utf8(s).expect("ascii");
        assert_eq!(s.len(), 21, "{s} is not YYYYMMDD-HH:MM:SS.sss");
        assert_eq!(&s[8..9], "-");
        assert_eq!(&s[11..12], ":");
        assert_eq!(&s[17..18], ".");
        let year: i32 = s[0..4].parse().expect("year");
        assert!((2020..2200).contains(&year), "implausible year in {s}");
        let month: u32 = s[4..6].parse().expect("month");
        assert!((1..=12).contains(&month), "bad month in {s}");
        let day: u32 = s[6..8].parse().expect("day");
        assert!((1..=31).contains(&day), "bad day in {s}");
    }
}
