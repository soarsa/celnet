//! The multi-dealer aggregation core: the [`QuoteSource`] panel abstraction and
//! the [`MultiDealerEngine`] that fans one [`RfqRequest`] to N sources
//! concurrently, then ranks the two-sided responses into an audited
//! [`RankedPanel`].
//!
//! # Ranking semantics (binding — `docs/W4-STRUCTURED-RFQ-PLAN.md` Track B)
//!
//! * **Best bid** = the **maximum** bid premium across responders (the client
//!   hits the highest bid). **Best offer** = the **minimum** offer premium (the
//!   client lifts the cheapest offer).
//! * **Tie-break (deterministic, so the panel is reproducible and auditable):**
//!   on an *equal best price*, prefer (1) the **earlier** `epoch_nanos`, then
//!   (2) the **lexicographically smallest** stable `lp_id`. The full key is
//!   therefore `(price, epoch_nanos, lp_id)` with `price` compared
//!   side-appropriately (max bid / min offer) and the latter two always
//!   ascending — a total order, so the winner is unique and stable.
//! * **Timeout / last-look:** each source is given a per-request deadline.
//!   Non-responders (deadline elapsed, or an explicit no-quote) are **dropped**,
//!   NOT errored, and are excluded from `lp_count`. A winning quote whose
//!   `valid_until_nanos` is already in the past at ranking time is **rejected**
//!   (last-look) and the next-best responder is promoted.
//! * **Consistency invariant (asserted by the engine, gated by the oracle):**
//!   `lp_count` == the number of responders; every `lp_won_*` references a real
//!   responder that is present in the ranked rows. A violation is a logic bug,
//!   not a data condition, so it surfaces as a returned [`PanelError`] (never a
//!   silent wrong winner).
//!
//! Pricing/quoting is the *source's* job; this module owns only the aggregation
//! algebra. The fan-out mirrors the server's `services/risk/federate.rs`
//! concurrent precedent (`futures_util::future::join_all` over a bounded panel),
//! so the panel latency is ≈ the slowest responder, never the sum.

use std::time::Duration;

use celnet_proto::{RatesInstrument, Side as WireSide};
use celnet_types::{CcyPair, OptionType, Tenor};
use futures_util::future::join_all;

/// One FX-option RFQ leg: the vendor-neutral descriptor the
/// [`crate::lp_fix::FixLpAdapter`] translates into a FIX `QuoteRequest(R)`
/// instrument block. Exactly the fields the FX route needs (pair, type, strike,
/// tenor).
#[derive(Debug, Clone, PartialEq)]
pub struct FxOptionLeg {
    /// The currency pair (`BASE/QUOTE`).
    pub pair: CcyPair,
    /// Call or put.
    pub option_type: OptionType,
    /// Strike (quote per base).
    pub strike: f64,
    /// Expiry tenor.
    pub tenor: Tenor,
}

/// One linear-rates (fixed-income) RFQ leg: the instrument the taker wants a
/// two-way on, at `notional` on `side`, plus the FIX `Symbol(55)` an external LP
/// is addressed with. The [`crate::lp_fix::FixLpAdapter`] translates it into a
/// rates FIX `QuoteRequest(R)` via the `celnet-fix` rates dialect (OIS / cash
/// bond); the native [`crate::internal::InternalPricerSource`] ignores it (its mid
/// is injected by the edge). This is the fixed-income analogue of [`FxOptionLeg`]
/// on the SAME [`QuoteSource`] / [`MultiDealerEngine`] ranking seam (ADR-0021
/// uniform-asset-class — generalize the seam, never fork a bespoke FI RFQ stack).
#[derive(Debug, Clone, PartialEq)]
pub struct RatesLeg {
    /// The FIX `Symbol(55)` an external LP is addressed with (e.g. `b"USD-OIS"`).
    pub symbol: Vec<u8>,
    /// The instrument to quote (OIS / vanilla IRS / FRA / cash bond).
    pub instrument: RatesInstrument,
    /// The RFQ size the two-way is good for (curve currency, strictly positive).
    pub notional: f64,
    /// The taker's directional intent (`SIDE_BUY` pay-fixed / long, `SIDE_SELL`
    /// receive-fixed / short, `SIDE_TWO_WAY` a two-way market with no firm side).
    pub side: WireSide,
}

/// The asset-class-specific payload of an RFQ leg (ADR-0021 uniform-asset-class):
/// exactly one variant is set. The aggregation / ranking / tie-break / last-look /
/// booking machinery in [`MultiDealerEngine`] below stays **asset-class-agnostic**
/// (it ranks [`TwoWay`] + timestamps only), so a fixed-income RFQ rides the SAME
/// ranking seam as an FX-option RFQ; each [`QuoteSource`] interprets the leg it
/// understands. This is the "generalize the seam, don't fork" contract.
#[derive(Debug, Clone, PartialEq)]
pub enum RfqLeg {
    /// An FX-option leg (pair / call-put / strike / tenor).
    FxOption(FxOptionLeg),
    /// A linear-rates (fixed-income) leg.
    Rates(RatesLeg),
}

/// A Celnet RFQ — the request fanned to every panel source. This is the
/// vendor-neutral, asset-class-correct descriptor of the instrument the client
/// wants a two-way market on; the [`crate::lp_fix::FixLpAdapter`] translates the
/// [`RfqLeg`] into a FIX `QuoteRequest(R)` and the
/// [`crate::internal::InternalPricerSource`] prices it in-process. It carries the
/// class-specific leg plus a stable client-minted request id for audit correlation.
#[derive(Debug, Clone, PartialEq)]
pub struct RfqRequest {
    /// A stable, client-minted request identifier (audit correlation key). Every
    /// [`DealerQuote`] in the resulting panel echoes the request this answered.
    pub request_id: String,
    /// The class-specific leg (FX option or fixed-income).
    pub leg: RfqLeg,
}

impl RfqRequest {
    /// Construct an RFQ for one FX-option leg (unchanged signature — the FX RFQ
    /// path is byte-identical to before the [`RfqLeg`] generalization).
    #[must_use]
    pub fn new(
        request_id: impl Into<String>,
        pair: CcyPair,
        option_type: OptionType,
        strike: f64,
        tenor: Tenor,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            leg: RfqLeg::FxOption(FxOptionLeg {
                pair,
                option_type,
                strike,
                tenor,
            }),
        }
    }

    /// Construct an RFQ for one linear-rates (fixed-income) leg — the FI analogue
    /// of [`RfqRequest::new`], routed to a rates FIX `QuoteRequest(R)` by the
    /// [`crate::lp_fix::FixLpAdapter`].
    #[must_use]
    pub fn new_rates(
        request_id: impl Into<String>,
        symbol: impl Into<Vec<u8>>,
        instrument: RatesInstrument,
        notional: f64,
        side: WireSide,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            leg: RfqLeg::Rates(RatesLeg {
                symbol: symbol.into(),
                instrument,
                notional,
                side,
            }),
        }
    }

    /// The FX-option leg, if this RFQ is an FX-option request; else `None`.
    #[must_use]
    pub fn fx(&self) -> Option<&FxOptionLeg> {
        match &self.leg {
            RfqLeg::FxOption(l) => Some(l),
            RfqLeg::Rates(_) => None,
        }
    }

    /// The linear-rates leg, if this RFQ is a fixed-income request; else `None`.
    #[must_use]
    pub fn rates(&self) -> Option<&RatesLeg> {
        match &self.leg {
            RfqLeg::Rates(l) => Some(l),
            RfqLeg::FxOption(_) => None,
        }
    }
}

/// A two-sided market: a bid and an offer premium (domestic per unit base).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwoWay {
    /// Bid premium (the price at which the LP buys from the client).
    pub bid: f64,
    /// Offer premium (the price at which the LP sells to the client).
    pub offer: f64,
}

impl TwoWay {
    /// Project onto the canonical wire [`celnet_proto::TwoWayPrice`] so the
    /// coordinator can surface a panel row over the contract without re-deriving
    /// the field mapping. The bid/offer are carried verbatim (full f64).
    #[must_use]
    pub fn to_wire(self) -> celnet_proto::TwoWayPrice {
        celnet_proto::TwoWayPrice {
            bid: self.bid,
            offer: self.offer,
        }
    }
}

impl From<celnet_proto::TwoWayPrice> for TwoWay {
    fn from(p: celnet_proto::TwoWayPrice) -> Self {
        Self {
            bid: p.bid,
            offer: p.offer,
        }
    }
}

/// What a [`QuoteSource`] returns for one [`RfqRequest`]: either a firm two-way
/// quote with its validity window, or an explicit declination (the source chose
/// not to quote — treated identically to a timeout: dropped, excluded from
/// `lp_count`).
#[derive(Debug, Clone, PartialEq)]
pub enum QuoteSourceReply {
    /// A firm two-sided quote.
    Quote {
        /// The two-way market.
        price: TwoWay,
        /// When the source produced this quote (nanoseconds since the Unix
        /// epoch in the engine's logical clock). Used as the first tie-break.
        epoch_nanos: u64,
        /// The last instant (same clock) at which the quote may be lifted; a
        /// winner past this at ranking time is rejected (last-look).
        valid_until_nanos: u64,
    },
    /// The source declined to quote this request (no market). Dropped from the
    /// panel exactly like a non-responder.
    NoQuote,
}

/// A panel quote source: an object-safe dealer the engine can fan an RFQ to.
///
/// Object-safe by design — the engine holds `&[Box<dyn QuoteSource>]` so the
/// panel can mix a native [`crate::internal::InternalPricerSource`] with any
/// number of [`crate::lp_fix::FixLpAdapter`]s (external LPs over loopback FIX)
/// behind one trait. The single method is the whole contract: answer `request`
/// within `deadline`, or be treated as a non-responder.
pub trait QuoteSource: Send + Sync {
    /// A stable identifier for this source — the audit `lp_id`. Stable across
    /// requests (so the deterministic tie-break is reproducible) and unique
    /// within a panel.
    fn lp_id(&self) -> &str;

    /// Answer `request` within `deadline`. Implementations MUST honour the
    /// deadline internally (the engine also imposes it as a hard outer bound, so
    /// a wedged source can never hang the panel). Returning [`QuoteSourceReply::NoQuote`]
    /// or exceeding the deadline both drop the source from the panel.
    fn request<'a>(
        &'a self,
        request: &'a RfqRequest,
        deadline: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = QuoteSourceReply> + Send + 'a>>;
}

/// One responder's audited row in the ranked panel: the source's `lp_id`, its
/// firm two-way, and the timestamps that drive the tie-break and last-look.
#[derive(Debug, Clone, PartialEq)]
pub struct DealerQuote {
    /// The responding source's stable `lp_id`.
    pub lp_id: String,
    /// The firm two-way market it quoted.
    pub price: TwoWay,
    /// When the source produced the quote (tie-break key 1).
    pub epoch_nanos: u64,
    /// Last instant the quote may be lifted (last-look horizon).
    pub valid_until_nanos: u64,
}

/// The audited, ranked multi-dealer panel returned by [`MultiDealerEngine::request`].
///
/// `rows` holds **only responders** (timed-out / no-quote sources are absent),
/// in a deterministic order (best-bid-first is not imposed; rows are sorted by
/// `lp_id` for a reproducible audit trail). `lp_count == rows.len()`. The two
/// winners are each `Some(lp_id)` of a row in `rows`, or `None` only when no
/// responder offered a *liftable* (not last-look-rejected) price on that side.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedPanel {
    /// The original request's id, echoed for audit correlation.
    pub request_id: String,
    /// Every responder's row (responders only; deterministic `lp_id` order).
    pub rows: Vec<DealerQuote>,
    /// The number of sources that responded in time with a firm quote.
    pub lp_count: usize,
    /// The best bid premium across liftable responders (max bid), if any.
    pub best_bid: Option<f64>,
    /// The best offer premium across liftable responders (min offer), if any.
    pub best_offer: Option<f64>,
    /// The `lp_id` that won the bid side (`⊆` responders), if any.
    pub lp_won_bid: Option<String>,
    /// The `lp_id` that won the offer side (`⊆` responders), if any.
    pub lp_won_offer: Option<String>,
    /// Responders whose firm quote was **rejected on last-look** — its
    /// `valid_until_nanos` had already lapsed at ranking time, so it was excluded
    /// from winning on both sides (`⊆` `rows`, deterministic `lp_id` order). Empty
    /// in the common case; surfaced so street-side LP analytics can grade an LP's
    /// last-look renege rate (`docs/ANALYTICS-REQUIREMENTS.md` §2.4). A responder is
    /// listed here iff it responded firm **and** its validity had lapsed — it may
    /// still appear in `rows` (it responded), but it cannot win.
    pub last_look_rejected: Vec<String>,
}

/// A panel-level logic error. Distinct from a *data* condition (a source
/// declining or timing out is normal and never an error) — a [`PanelError`] is
/// only raised if the consistency invariant could not be upheld, which is a bug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelError {
    /// A winner was selected that is not present among the responders. The
    /// ranking produced an inconsistent panel — never returned in correct
    /// operation; surfaced as a hard error rather than silently shipped.
    WinnerNotAResponder {
        /// The side (`"bid"` / `"offer"`) whose winner was inconsistent.
        side: &'static str,
        /// The offending `lp_id`.
        lp_id: String,
    },
}

impl std::fmt::Display for PanelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PanelError::WinnerNotAResponder { side, lp_id } => {
                write!(f, "ranked {side} winner '{lp_id}' is not a responder")
            }
        }
    }
}

impl std::error::Error for PanelError {}

/// The multi-dealer aggregation engine. Holds a bounded panel of object-safe
/// [`QuoteSource`]s and fans each RFQ to all of them concurrently.
pub struct MultiDealerEngine {
    sources: Vec<Box<dyn QuoteSource>>,
}

impl MultiDealerEngine {
    /// Build an engine over a bounded panel. The panel should carry at least one
    /// native dealer (an [`crate::internal::InternalPricerSource`]) so a market
    /// always exists; this is the caller's responsibility (the server wires it).
    #[must_use]
    pub fn new(sources: Vec<Box<dyn QuoteSource>>) -> Self {
        Self { sources }
    }

    /// The number of sources on the panel (responders + non-responders).
    #[must_use]
    pub fn panel_size(&self) -> usize {
        self.sources.len()
    }

    /// Fan `request` to every panel source concurrently with a per-source
    /// `deadline`, collect the responders, and rank them into a [`RankedPanel`].
    ///
    /// `now_nanos` is the engine's logical "now" used for the last-look check (a
    /// quote with `valid_until_nanos < now_nanos` is rejected). Passing the wall
    /// clock makes last-look real; tests pass a fixed value to make it
    /// deterministic.
    ///
    /// # Errors
    /// Returns [`PanelError`] only if the internal consistency invariant fails
    /// (a selected winner is not a responder) — a logic bug, never a normal data
    /// condition. Timeouts and declinations are not errors.
    pub async fn request(
        &self,
        request: &RfqRequest,
        deadline: Duration,
        now_nanos: u64,
    ) -> Result<RankedPanel, PanelError> {
        // Concurrent fan-out: dial every source at once (the `federate.rs`
        // precedent). Each future is bounded by the hard outer deadline so a
        // wedged source is dropped, never allowed to hang the panel.
        let futures = self.sources.iter().map(|src| {
            let req = request;
            async move {
                let id = src.lp_id().to_owned();
                let reply = match tokio::time::timeout(deadline, src.request(req, deadline)).await {
                    Ok(reply) => reply,
                    // Outer deadline elapsed: treat exactly as a non-responder.
                    Err(_elapsed) => QuoteSourceReply::NoQuote,
                };
                (id, reply)
            }
        });
        let replies = join_all(futures).await;

        // Keep only firm quotes (responders). NoQuote / timeout are dropped.
        let mut rows: Vec<DealerQuote> = replies
            .into_iter()
            .filter_map(|(lp_id, reply)| match reply {
                QuoteSourceReply::Quote {
                    price,
                    epoch_nanos,
                    valid_until_nanos,
                } => Some(DealerQuote {
                    lp_id,
                    price,
                    epoch_nanos,
                    valid_until_nanos,
                }),
                QuoteSourceReply::NoQuote => None,
            })
            .collect();

        // Deterministic audit order, independent of fan-out completion order.
        rows.sort_by(|a, b| a.lp_id.cmp(&b.lp_id));
        let lp_count = rows.len();

        // Rank each side over the liftable (not last-look-rejected) responders.
        let lp_won_bid = rank_side(&rows, now_nanos, Side::Bid);
        let lp_won_offer = rank_side(&rows, now_nanos, Side::Offer);

        let best_bid = lp_won_bid
            .as_ref()
            .and_then(|id| rows.iter().find(|r| &r.lp_id == id))
            .map(|r| r.price.bid);
        let best_offer = lp_won_offer
            .as_ref()
            .and_then(|id| rows.iter().find(|r| &r.lp_id == id))
            .map(|r| r.price.offer);

        // Consistency invariant: a declared winner must be a real responder.
        check_winner(&rows, "bid", lp_won_bid.as_deref())?;
        check_winner(&rows, "offer", lp_won_offer.as_deref())?;

        // Responders whose validity had lapsed at ranking — excluded from winning
        // on both sides (last-look). `rows` is already `lp_id`-sorted, so this
        // preserves the deterministic audit order.
        let last_look_rejected: Vec<String> = rows
            .iter()
            .filter(|r| r.valid_until_nanos < now_nanos)
            .map(|r| r.lp_id.clone())
            .collect();

        Ok(RankedPanel {
            request_id: request.request_id.clone(),
            rows,
            lp_count,
            best_bid,
            best_offer,
            lp_won_bid,
            lp_won_offer,
            last_look_rejected,
        })
    }
}

/// Which side of the two-way is being ranked.
#[derive(Clone, Copy)]
enum Side {
    Bid,
    Offer,
}

/// Select the winning `lp_id` for one side over the liftable responders.
///
/// "Liftable" = `valid_until_nanos >= now_nanos` (last-look: a stale quote is
/// excluded from winning, and the next-best is therefore promoted naturally).
/// The winner is the extremum of a **total** comparison key so it is unique and
/// reproducible:
/// * bid: maximise `bid`, then (tie) minimise `epoch_nanos`, then (tie) minimise
///   `lp_id`;
/// * offer: minimise `offer`, then (tie) minimise `epoch_nanos`, then (tie)
///   minimise `lp_id`.
fn rank_side(rows: &[DealerQuote], now_nanos: u64, side: Side) -> Option<String> {
    rows.iter()
        // Last-look: a quote whose validity has lapsed cannot win.
        .filter(|r| r.valid_until_nanos >= now_nanos)
        .min_by(|a, b| side_key(a, side).partial_cmp_total(&side_key(b, side)))
        .map(|r| r.lp_id.clone())
}

/// The total comparison key for one side. The price component is oriented so a
/// *smaller* key always means *better* (we negate the bid so "max bid" becomes
/// "min key"); the tie-breaks (`epoch_nanos`, then `lp_id`) are naturally
/// ascending = better. Comparing this key with the strict total order below
/// yields the documented deterministic winner.
fn side_key(row: &DealerQuote, side: Side) -> SideKey<'_> {
    let price = match side {
        // Negate so that the largest bid sorts first under a "min" selection.
        Side::Bid => -row.price.bid,
        Side::Offer => row.price.offer,
    };
    SideKey {
        price,
        epoch_nanos: row.epoch_nanos,
        lp_id: &row.lp_id,
    }
}

/// A side's comparison key: `(price, epoch_nanos, lp_id)`, all ascending = better.
struct SideKey<'a> {
    price: f64,
    epoch_nanos: u64,
    lp_id: &'a str,
}

impl SideKey<'_> {
    /// A strict **total** order over the key. The price field is finite by
    /// construction (sources never quote NaN — see the adapters), but we still
    /// fold any non-finite price to "worst" so a malformed source can never win
    /// and the order stays total (`partial_cmp` never returns `None`). The
    /// `epoch_nanos` then `lp_id` tie-breaks make distinct rows always
    /// distinguishable, so the selected minimum is unique.
    fn partial_cmp_total(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        // Treat a non-finite price as strictly worse than any finite price.
        let by_price = match (self.price.is_finite(), other.price.is_finite()) {
            (true, true) => self
                .price
                .partial_cmp(&other.price)
                .unwrap_or(Ordering::Equal),
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => Ordering::Equal,
        };
        by_price
            .then_with(|| self.epoch_nanos.cmp(&other.epoch_nanos))
            .then_with(|| self.lp_id.cmp(other.lp_id))
    }
}

/// Enforce the consistency invariant: a non-`None` winner must be present in the
/// ranked responder rows.
fn check_winner(
    rows: &[DealerQuote],
    side: &'static str,
    winner: Option<&str>,
) -> Result<(), PanelError> {
    if let Some(id) = winner {
        let present = rows.iter().any(|r| r.lp_id == id);
        if !present {
            return Err(PanelError::WinnerNotAResponder {
                side,
                lp_id: id.to_owned(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(lp_id: &str, bid: f64, offer: f64, epoch: u64, valid_until: u64) -> DealerQuote {
        DealerQuote {
            lp_id: lp_id.to_owned(),
            price: TwoWay { bid, offer },
            epoch_nanos: epoch,
            valid_until_nanos: valid_until,
        }
    }

    #[test]
    fn rank_side_picks_max_bid_and_min_offer() {
        let rows = vec![
            row("A", 0.0090, 0.0102, 100, u64::MAX),
            row("B", 0.0095, 0.0099, 200, u64::MAX),
            row("C", 0.0088, 0.0101, 300, u64::MAX),
        ];
        assert_eq!(rank_side(&rows, 0, Side::Bid).as_deref(), Some("B")); // max bid
        assert_eq!(rank_side(&rows, 0, Side::Offer).as_deref(), Some("B")); // min offer
    }

    #[test]
    fn tie_break_epoch_then_lp_id() {
        // Equal best bid 0.0090: earliest epoch wins.
        let rows = vec![
            row("LATE", 0.0090, 0.02, 500, u64::MAX),
            row("EARLY", 0.0090, 0.02, 100, u64::MAX),
        ];
        assert_eq!(rank_side(&rows, 0, Side::Bid).as_deref(), Some("EARLY"));

        // Equal bid AND epoch: smallest lp_id wins.
        let rows = vec![
            row("BBB", 0.0090, 0.02, 100, u64::MAX),
            row("AAA", 0.0090, 0.02, 100, u64::MAX),
        ];
        assert_eq!(rank_side(&rows, 0, Side::Bid).as_deref(), Some("AAA"));
    }

    #[test]
    fn last_look_excludes_stale_rows_from_winning() {
        // STALE has the best bid but expired before `now`.
        let rows = vec![
            row("STALE", 0.0099, 0.0090, 100, 500),
            row("FRESH", 0.0095, 0.0099, 200, u64::MAX),
        ];
        assert_eq!(rank_side(&rows, 1_000, Side::Bid).as_deref(), Some("FRESH"));
        assert_eq!(
            rank_side(&rows, 1_000, Side::Offer).as_deref(),
            Some("FRESH")
        );
        // At an earlier `now` STALE is still liftable and wins.
        assert_eq!(rank_side(&rows, 400, Side::Bid).as_deref(), Some("STALE"));
    }

    #[test]
    fn all_stale_yields_no_winner() {
        let rows = vec![
            row("A", 0.0099, 0.0090, 100, 100),
            row("B", 0.0095, 0.0092, 200, 200),
        ];
        assert_eq!(rank_side(&rows, 1_000, Side::Bid), None);
        assert_eq!(rank_side(&rows, 1_000, Side::Offer), None);
    }

    #[test]
    fn non_finite_price_never_wins() {
        // A malformed NaN bid must not win over a finite bid; the order stays
        // total (no panic, deterministic).
        let rows = vec![
            row("NAN", f64::NAN, 0.0099, 100, u64::MAX),
            row("OK", 0.0090, 0.0100, 200, u64::MAX),
        ];
        assert_eq!(rank_side(&rows, 0, Side::Bid).as_deref(), Some("OK"));
    }

    #[test]
    fn check_winner_rejects_phantom() {
        let rows = vec![row("A", 0.009, 0.010, 1, u64::MAX)];
        // A winner not among the rows is an invariant violation.
        let err = check_winner(&rows, "bid", Some("GHOST")).unwrap_err();
        assert_eq!(
            err,
            PanelError::WinnerNotAResponder {
                side: "bid",
                lp_id: "GHOST".to_owned(),
            }
        );
        // A real responder passes.
        assert!(check_winner(&rows, "bid", Some("A")).is_ok());
        assert!(check_winner(&rows, "bid", None).is_ok());
    }

    #[test]
    fn two_way_wire_round_trip() {
        let tw = TwoWay {
            bid: 0.0081,
            offer: 0.0089,
        };
        let wire = tw.to_wire();
        assert_eq!(wire.bid, 0.0081);
        assert_eq!(wire.offer, 0.0089);
        assert_eq!(TwoWay::from(wire), tw);
    }
}
