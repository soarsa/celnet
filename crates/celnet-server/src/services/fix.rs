//! The **live FIX 4.4 acceptor edge** — a quote venue that speaks the real
//! `celnet-fix` 4.4 protocol over a TCP socket, pricing every RFQ through the
//! **same** surface-book / engine path the gRPC `QuoteService` uses and booking
//! every lift through the **same** keyed-MAC click-to-trade / last-look token path
//! the RFS `StreamService` uses.
//!
//! # What this is (and what it deliberately reuses)
//!
//! `celnet-fix` is the real, gated FIX engine: SOH framing + `BodyLength`/`CheckSum`
//! validation, the FIXT/4.4 session FSM (logon / heartbeat / test-request / resend /
//! gap-fill), and the FX-options dialect ([`celnet_fix::dialect_fx`]). This module is
//! the **edge task** that drives that engine over a real `tokio` socket and joins it
//! to the live product:
//!
//! * **Pricing** — a `QuoteRequest(R)` is decoded + convention-checked by the
//!   dialect into an [`celnet_fix::dialect_fx::OptionDescriptor`], lifted into the
//!   canonical [`celnet_proto::Instrument`], and priced through [`crate::pricer`]
//!   against the live engine market (read via [`CoreLink`]) and the shared
//!   [`SurfaceBook`] pin — **byte-for-byte the same** path `QuoteService::request_quote`
//!   takes. The maker two-way comes from the same [`SpreadModel`]. No forked pricing.
//! * **Click-to-trade / last-look** — the two-way line stamps the **same**
//!   cryptographically-unforgeable keyed-MAC tokens ([`crate::services::clicktrade`])
//!   the RFS stream stamps. The wire carries them as the FIX `QuoteID(117)`; a
//!   `NewOrderSingle(D)` referencing that `QuoteID` + a `Side(54)` is validated by the
//!   **same** [`crate::services::clicktrade::TokenLedger::try_book`] — so a FIX lift is
//!   the identical execution math as a gRPC click (last-look expiry, replay rejection,
//!   forged-token rejection, idempotent re-lift). A reject answers with an
//!   `ExecutionReport(35=8, ExecType=8)` carrying a `Text(58)` reason, exactly as the
//!   gRPC path returns a typed reject.
//!
//! # Binding
//!
//! The acceptor is bound at edge boot from the `CELNET_FIX_ADDR` environment knob
//! (the same deploy-time-knob pattern as `CELNET_FLEET_MODE`): **absent ⇒ no FIX
//! listener is started and the edge is byte-identical to today**; present ⇒ a FIX 4.4
//! acceptor binds on that address and serves the live RFQ→Quote→lift→ExecutionReport
//! lifecycle. The bound address is read back via [`FixAcceptor::local_addr`] (so a
//! test can bind `:0` and dial the OS-assigned port).
//!
//! # Determinism
//!
//! Pricing routes entirely through [`crate::pricer`] (libm, no wall-clock). The only
//! wall-clock reads are the edge message timestamp ([`Clock`]) and the token
//! last-look deadline — neither is a priced number, exactly as on the gRPC/RFS edges.

#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};

use celnet_fix::dialect_fx::{self, ExerciseStyle, OptionDescriptor};
use celnet_fix::dialect_rates;
use celnet_fix::dictionary::MsgType;

use super::fix_monitor::{FixDirection, FixMonitor};
use celnet_fix::framing::FrameCursor;
use celnet_fix::messages::{self, EXEC_FILLED, EXEC_REJECTED, ExecReportParams, QuoteParams};
use celnet_fix::session::{InMemoryStore, Role, Session, SessionAction, SessionConfig};
use celnet_fix::transport::{FrameReader, write_frame};

use celnet_proto::{
    BondInstrument, CurveSet, DeskQuote, ManualInterventionReason, OisInstrument, RatesInstrument,
    rates_instrument,
};
use celnet_proto::{CcyPair, Instrument, MarketContext, Quantity, Side, StrikeOrDelta, Vanilla};
use celnet_proto::{instrument, strike_or_delta};

use celnet_types::{OptionType, Tenor};

use celnet_acceptance::AcceptanceDecision;

use crate::clock::Clock;
use crate::config::fix_connections::AcceptorKind;
use crate::config::identity::{
    DEFAULT_ASYNC_GIVEBACK_PCT, DEFAULT_BOOK_SKEW_WEIGHT, DEFAULT_LAST_LOOK_TOLERANCE_BPS,
    LastLookMode, PricingSourceMode,
};
use crate::core_link::CoreLink;
use crate::pricer::{ConventionSet, price_instrument};
use crate::services::clicktrade::{
    BookOutcome, MintedToken, TokenLedger, TokenMinter, TwoWayLine, mint_two_way,
    mint_two_way_no_clear,
};
use crate::services::desk::RfqIngestOutcome;
use crate::services::pin::{PinnedVol, resolve_pinned_vol};
use crate::services::risk::store::{BookedPosition, PositionStore, limit_breach_message};
use crate::spread::SpreadModel;
use crate::surface_book::SurfaceBook;

// The custom `expiry_years` dialect tag is owned by `celnet_fix::dialect_fx`
// ([`dialect_fx::TAG_EXPIRY_YEARS`]) — the single source of truth shared with the
// initiator/test client that encodes it. Referenced directly below.

/// How long a FIX quote's click-to-trade token stays valid for a lift, in nanoseconds
/// (5 seconds — the typical OTC last-look window, matching the RFQ deadline).
const QUOTE_VALIDITY_NANOS: i64 = 5_000_000_000;

/// The maker `SenderCompID` the venue presents (overridable via `CELNET_FIX_SENDER`).
const DEFAULT_VENUE_COMP_ID: &str = "CELNET";

/// The counterparty `TargetCompID` the venue expects the initiator to present
/// (overridable via `CELNET_FIX_TARGET`). FIX CompIDs are a pre-agreed pair, exactly
/// as the `celnet-fix` loopback tests configure them.
const DEFAULT_COUNTERPARTY_COMP_ID: &str = "CELNET-CPTY";

/// A live FIX quote the acceptor has streamed and will honour on a lift until its
/// tokens expire: the two per-side keyed-MAC tokens (SELL@bid, BUY@offer) and the
/// symbol the lift books, keyed by the wire `QuoteID(117)`.
#[derive(Debug, Clone)]
struct FixQuote {
    /// The wire symbol (echoed onto the `ExecutionReport`).
    symbol: Vec<u8>,
    /// The BUY (offer) token, if a positive offer was quoted.
    buy_token: Option<u64>,
    /// The SELL (bid) token, if a positive bid was quoted (a floored-zero bid mints
    /// none — that side is indicative only).
    sell_token: Option<u64>,
    /// The FX-vanilla pre-trade template captured at quote time (ADR-0016 A1): the
    /// BUY-side [`BookedPosition`] this line would book, so a `NewOrderSingle` lift can
    /// run the pre-trade limit gate without re-pricing (a `Side=SELL` lift negates the
    /// notional). `None` for a rates line — an OIS quote carries no canonical-vanilla
    /// risk leaf and so is never limit-gated.
    fx: Option<BookedPosition>,
    /// The desk request id an auto-quoted OIS RFQ / RFS stream created (if any). A
    /// `NewOrderSingle` lift of this quote books that request as a completed deal via
    /// [`crate::services::desk::RfqDeskEdge::book_fix_lift`] — so a FIX taker executing a
    /// rates auto-quote surfaces as a booked deal + live position. `None` for an FX line
    /// or a venue that is not desk-routed.
    rates_request_id: Option<String>,
    /// The clock time (nanos) this quote was minted — captured at
    /// [`Session::emit_two_way_quote`] alongside the token validity. A lift derives the
    /// quote's **age** (`now − mint_nanos`) for the incoming-quote-acceptance context, so a
    /// rule can reject / hold a stale lift.
    mint_nanos: i64,
}

/// A live **market-data** subscription on a fixed-income STREAM venue: opened by an
/// inbound `MarketDataRequest(V)` (subscribe), the venue re-prices this line every session
/// tick and pushes a fresh top-of-book `MarketDataSnapshotFullRefresh(W)` until the taker
/// unsubscribes (or the session drops). The per-tick liftable token is held symbol-keyed in
/// [`FixSession::live_md`] (a market-data lift names the `Symbol(55)`, not a `QuoteID`), so
/// the subscription itself carries only the re-price inputs and the desk-row correlation.
#[derive(Debug, Clone)]
struct MdSubscription {
    /// The wire `MDReqID(262)` correlating every streamed snapshot (and the unsubscribe).
    md_req_id: Vec<u8>,
    /// The instrument symbol echoed on each streamed snapshot (`Symbol(55)`).
    symbol: Vec<u8>,
    /// The subscribed size carried as the streamed entry size (`MDEntrySize(271)`).
    notional: f64,
    /// The pricing inputs to RE-PRICE this line each tick off the CURRENT live curve when
    /// no aggregated book covers it — so a live curve re-mark moves the stream (no
    /// fabricated movement). The composite path never consults this (it keys off
    /// [`symbol`](Self::symbol)); this drives only the curve fallback.
    line: RfsLine,
    /// The desk history row id this subscription opened, so a lift of any streamed update
    /// books it as a completed deal (`None` when the venue is not desk-routed).
    request_id: Option<String>,
}

/// The currently-liftable market-data quote for one streamed symbol, refreshed every tick:
/// the two per-side keyed-MAC tokens (SELL@bid, BUY@offer) a market-data lift authenticates
/// against, plus the desk-row id a fill books and the mint time a stale-quote acceptance
/// rule reads. Keyed by `Symbol(55)` in [`FixSession::live_md`] — a fresh tick OVERWRITES
/// the entry, so a lift always executes against the latest published top-of-book (the older
/// tokens simply lapse in the ledger's validity window).
#[derive(Debug, Clone)]
struct MdLiveQuote {
    /// The BUY (offer) token, if a positive offer was published.
    buy_token: Option<u64>,
    /// The SELL (bid) token, if a positive bid was published.
    sell_token: Option<u64>,
    /// The published Bid price (`MDEntryType=0`) this snapshot advertised — a SELL lift's
    /// `Price(44)` must match this (within tolerance) or the market has moved (last-look).
    bid: f64,
    /// The published Offer price (`MDEntryType=1`) this snapshot advertised — a BUY lift's
    /// `Price(44)` must match this (within tolerance) or the market has moved (last-look).
    offer: f64,
    /// The desk request id this streamed line opened (so a lift books the deal).
    request_id: Option<String>,
    /// The clock time (nanos) this snapshot was minted (for the quote-age acceptance gate).
    mint_nanos: i64,
}

/// The decoded inbound market-data instrument, tagged by arm so [`FixSession::on_market_data_request`]
/// records the correct desk-inbox row (bond vs OIS) after registering the subscription.
enum MdRecord {
    /// A cash-bond subscription: recorded via [`FixSession::record_bond_rfq`].
    Bond(dialect_rates::BondRfq),
    /// An OIS subscription: recorded via [`FixSession::record_rates_rfq`].
    Ois(dialect_rates::RatesRfq),
}

/// The pricing inputs a live RFS subscription retains so each re-price tick can
/// reconstruct the line off the CURRENT live curve (the honest alternative to
/// oscillating a frozen base rate — a curve re-mark then moves the stream). Only the
/// **curve fallback** uses this; the composite path re-prices off the book by symbol.
#[derive(Clone, Debug)]
enum RfsLine {
    /// An OIS line: re-priced as the par rate of `tenor_years` on the live curve.
    Ois { tenor_years: u32 },
    /// A cash-bond line: re-priced as the clean price off the live curve.
    Bond { instrument: Box<BondInstrument> },
}

impl RfsLine {
    /// Whether this line is a cash bond (the product the [`PricingSourceMode::ProductSplit`]
    /// policy routes to the composite).
    fn is_bond(&self) -> bool {
        matches!(self, RfsLine::Bond { .. })
    }
}

/// The shared, immutable pricing context every FIX session on this edge prices and
/// books against: the live engine bridge, the maker spread, the edge clock, and the
/// shared marked-surface registry. Cloning is cheap (all `Arc`/`Copy`).
#[derive(Clone)]
pub(crate) struct FixContext {
    link: Arc<CoreLink>,
    spread: SpreadModel,
    clock: Clock,
    surface_book: Arc<SurfaceBook>,
    /// The maker `SenderCompID` (pre-agreed venue identity).
    sender: Vec<u8>,
    /// The expected counterparty `TargetCompID` (pre-agreed initiator identity); the
    /// session FSM rejects any other peer with a CompID mismatch.
    counterparty: Vec<u8>,
    /// The shared session-traffic capture sink (the monitor screen reads it). Every
    /// inbound/outbound frame on this session is recorded against `connection_id`.
    monitor: Arc<FixMonitor>,
    /// The managing connection's id used to tag captured frames (a synthetic id for
    /// the legacy env-seeded acceptor).
    connection_id: String,
    /// The dialect this acceptor serves. A fixed-income kind routes inbound
    /// `QuoteRequest(R)` frames to the shared rates dialect and constrains the
    /// inbound `SubscriptionRequestType(263)` to the connection's intent (a
    /// quote/RFQ venue serves snapshots; a stream/RFS venue serves subscribes); the
    /// FX-options kind prices the FX block and still content-detects an OIS request
    /// for the legacy/demo venue.
    kind: AcceptorKind,
    /// The dealer-quoting desk inbox this acceptor records inbound rates RFQs into
    /// (so the GUI desk shows what a FIX venue received — live pending + processed
    /// history). `None` for the legacy env seed / tests that don't wire a desk.
    desk_edge: Option<Arc<crate::services::desk::RfqDeskEdge>>,
    /// The desk id an inbound rates RFQ is booked under (must be a desk the trader's
    /// session may see, or the reader is an all-desks admin). Empty when unrouted.
    desk: String,
    /// The auto-quote admission policy for a fixed-income venue: an RFQ it admits is
    /// auto-quoted (recorded QUOTED history); one it declines is routed to a human
    /// desk (recorded PENDING, no auto `Quote(S)` sent back).
    auto_quote: RatesAutoQuotePolicy,
    /// The shared live position book (ADR-0016 A1): a FIX lift consults the SAME
    /// pre-trade limit tree the RFS click-to-trade sink enforces, so a hard-limit-blown
    /// `NewOrderSingle` is refused with a rejected `ExecutionReport(ExecType=8)` — a
    /// `Text(58) = "limit breached: …"` — before the fill is emitted. Shared behind an
    /// `Arc` with every gRPC/WS edge, so the FIX venue sees the same firm/pair caps.
    store: Arc<PositionStore>,
    /// The live aggregated-book composite + pricing-group registry a fixed-income
    /// **stream** venue prices its outbound RFS/ESP two-way off (`docs/FI-PRICING-GROUPS-
    /// DESIGN.md` §5): when the streamed instrument is covered by an enabled aggregated
    /// book, the pushed two-way is the book's consolidated composite run through THIS
    /// connection's pricing-group pipeline (resolved by connection id, then desk) —
    /// closing the deferred rates-RFS group-pricing seam so the FIX stream is
    /// composite-based + tiered exactly like the gRPC/WS RFQ path
    /// ([`super::quote::QuoteEdge::book_composite_for_caller`]). `None` on the legacy env
    /// seed / tests that wire no hub — the stream then falls back to the standalone P0
    /// demo re-price, byte-identical to before. Resolution happens on the FIX session
    /// ticker task (never the pinned zero-alloc pricer), so guardrail 11 holds.
    aggregation_hub: Option<Arc<crate::services::aggregation::AggregationHub>>,
    /// The firm-wide **outbound** pricing kill-switch. Read (a cheap `Relaxed` load on
    /// the FIX session ticker task, never the pinned pricer) before every outbound
    /// pricing emission: when outbound is disabled, RFQ auto-quotes are suppressed and
    /// RFS/ESP streams pause (they re-check each tick, so they resume from live on
    /// re-enable). Inbound LP consumption, the desk-inbox RFQ recording, and internal
    /// book updates are untouched. Defaulted to a both-enabled control by the
    /// constructors (byte-identical to before the kill-switch); the boot path shares
    /// the one runtime control via [`FixContext::with_pricing_control`].
    pricing_control: Arc<crate::services::pricing_control::PricingControl>,
}

/// The admission policy that decides whether an inbound rates RFQ is **auto-quoted**
/// by the venue or **routed to a human desk**. A bank auto-quotes small, on-the-run
/// clips and works larger or off-the-run risk by voice/manual — modelled here as a
/// notional cap plus an on-the-run tenor set. Both are configurable; the defaults
/// give a realistic mix for the standard simulator flow.
#[derive(Debug, Clone)]
pub(crate) struct RatesAutoQuotePolicy {
    /// The largest notional the venue will auto-quote; above this it routes to a desk.
    pub max_notional: f64,
    /// The on-the-run tenors (whole years) the venue will auto-quote; any other tenor
    /// routes to a desk.
    pub tenors: std::collections::HashSet<u32>,
}

impl Default for RatesAutoQuotePolicy {
    fn default() -> Self {
        Self {
            max_notional: 25_000_000.0,
            tenors: [1, 2, 3, 5, 7, 10].into_iter().collect(),
        }
    }
}

/// The curve symbols (`Symbol(55)`) this venue can price on the OIS arm. An inbound RFQ
/// naming any other symbol is an **unknown security** the desk must handle by hand
/// (a `MANUAL_INTERVENTION_REQUIRED` alert), never a silently-dropped frame.
const KNOWN_RATES_SYMBOLS: &[&[u8]] = &[b"USD-OIS"];

/// Whether `symbol` is a rates curve this venue prices (see [`KNOWN_RATES_SYMBOLS`]).
fn is_known_rates_symbol(symbol: &[u8]) -> bool {
    KNOWN_RATES_SYMBOLS.contains(&symbol)
}

/// How the venue admits an inbound rates RFQ it does not fill on the wire itself — the
/// exception-vs-routine distinction the desk alerts on. Produced by
/// [`FixSession::classify_ois_rfq`] and consumed by [`FixSession::record_rates_rfq`].
enum RatesAdmission {
    /// On-the-run, within cap, priceable ⇒ auto-quote this two-way (quiet history).
    Auto(PricedLine),
    /// A routine RFQ a human prices in the normal flow (e.g. a large on-the-run clip):
    /// routed to the desk (PENDING), but NOT an alert.
    RoutedToDesk,
    /// Cannot be auto-priced and needs a human now: routed to the desk (PENDING) with an
    /// **alert-worthy** `MANUAL_INTERVENTION_REQUIRED` carrying the reason.
    Manual(ManualInterventionReason),
}

/// Decide how the venue admits an OIS RFQ, given the auto-quote `policy`, the request's
/// `symbol` / `tenor_years` / `notional`, and the engine's (already-attempted) two-way
/// `priced` line (`None` ⇒ either not attempted because a policy gate failed, or the
/// engine failed to price a gated-in request). Pure — no session/frame — so the
/// exception-vs-routine policy is directly unit-testable. The order is the policy:
///
/// 1. unknown security (symbol the venue does not price) — an exception alert;
/// 2. unconfigured (off-the-run) tenor — an exception alert;
/// 3. on-the-run but over the clip cap — routine large clip, routed quietly;
/// 4. on-the-run + within cap: a present price ⇒ auto-quote; an absent one ⇒ a genuine
///    pricing failure (exception alert).
fn classify_ois_admission(
    policy: &RatesAutoQuotePolicy,
    symbol: &[u8],
    tenor_years: u32,
    notional: f64,
    priced: Option<PricedLine>,
) -> RatesAdmission {
    if !is_known_rates_symbol(symbol) {
        return RatesAdmission::Manual(ManualInterventionReason::UnknownSecurity);
    }
    if !policy.tenors.contains(&tenor_years) {
        return RatesAdmission::Manual(ManualInterventionReason::UnconfiguredTenor);
    }
    if notional > policy.max_notional {
        return RatesAdmission::RoutedToDesk;
    }
    match priced {
        Some(priced) => RatesAdmission::Auto(priced),
        None => RatesAdmission::Manual(ManualInterventionReason::PricingFailure),
    }
}

impl FixContext {
    /// Build the FIX pricing context from the same shared services the gRPC edges use,
    /// reading the (optional) `CELNET_FIX_SENDER` / `CELNET_FIX_TARGET` CompID overrides
    /// from the environment (the same deploy-time-knob pattern as the fleet topology).
    pub(crate) fn new(
        link: Arc<CoreLink>,
        spread: SpreadModel,
        clock: Clock,
        surface_book: Arc<SurfaceBook>,
        monitor: Arc<FixMonitor>,
        connection_id: String,
        store: Arc<PositionStore>,
    ) -> Self {
        let sender = std::env::var("CELNET_FIX_SENDER")
            .unwrap_or_else(|_| DEFAULT_VENUE_COMP_ID.to_owned())
            .into_bytes();
        let counterparty = std::env::var("CELNET_FIX_TARGET")
            .unwrap_or_else(|_| DEFAULT_COUNTERPARTY_COMP_ID.to_owned())
            .into_bytes();
        Self {
            link,
            spread,
            clock,
            surface_book,
            sender,
            counterparty,
            monitor,
            connection_id,
            // The legacy env-seeded acceptor serves the FX-options dialect (and still
            // content-detects an OIS request, as it did before connection kinds).
            kind: AcceptorKind::Options,
            // The legacy env seed is not desk-routed (it predates the managed registry
            // and only ever serves the FX/legacy path).
            desk_edge: None,
            desk: String::new(),
            auto_quote: RatesAutoQuotePolicy::default(),
            store,
            aggregation_hub: None,
            pricing_control: crate::services::pricing_control::PricingControl::new(true, true),
        }
    }

    /// Build a context with explicit CompIDs (the race-free path for tests, which bind
    /// ephemeral ports and must not mutate process-global env) and an explicit dialect
    /// `kind` (the managed-registry path passes the connection's kind; the legacy
    /// attach path passes [`AcceptorKind::Options`]).
    #[allow(clippy::too_many_arguments)] // the shared component set + this acceptor's identity.
    pub(crate) fn with_comp_ids(
        link: Arc<CoreLink>,
        spread: SpreadModel,
        clock: Clock,
        surface_book: Arc<SurfaceBook>,
        sender: Vec<u8>,
        counterparty: Vec<u8>,
        monitor: Arc<FixMonitor>,
        connection_id: String,
        kind: AcceptorKind,
        store: Arc<PositionStore>,
    ) -> Self {
        Self {
            link,
            spread,
            clock,
            surface_book,
            sender,
            counterparty,
            monitor,
            connection_id,
            kind,
            desk_edge: None,
            desk: String::new(),
            auto_quote: RatesAutoQuotePolicy::default(),
            store,
            aggregation_hub: None,
            pricing_control: crate::services::pricing_control::PricingControl::new(true, true),
        }
    }

    /// Share the firm-wide runtime **pricing kill-switch** so this venue's outbound
    /// pricing (RFQ auto-quotes + RFS/ESP streams) is gated by `SetPricingControl`.
    /// Takes an `Option` so the managed registry can pass its set-once handle (or `None`
    /// in tests that never wire it, keeping the default both-enabled control). Builder-
    /// style; a no-op wire keeps existing callers/tests byte-identical.
    #[must_use]
    pub(crate) fn with_pricing_control(
        mut self,
        control: Option<Arc<crate::services::pricing_control::PricingControl>>,
    ) -> Self {
        if let Some(control) = control {
            self.pricing_control = control;
        }
        self
    }

    /// Wire the live aggregated-book composite + pricing-group registry a fixed-income
    /// **stream** venue prices its outbound RFS/ESP off (see the field docs). Builder-
    /// style so existing callers/tests that don't wire a hub are unchanged (the stream
    /// then keeps the standalone P0 demo re-price). A no-op wire for an FX-options
    /// acceptor, whose path never streams a composite.
    #[must_use]
    pub(crate) fn with_aggregation(
        mut self,
        hub: Option<Arc<crate::services::aggregation::AggregationHub>>,
    ) -> Self {
        self.aggregation_hub = hub;
        self
    }

    /// Wire the desk routing for a managed fixed-income venue: the inbox `desk_edge`
    /// an inbound rates RFQ is recorded into, the `desk` it is booked under, and the
    /// `auto_quote` admission policy (admitted ⇒ auto-quoted history; declined ⇒
    /// routed to a human desk as PENDING). A no-op for an FX-options acceptor (whose
    /// path never records to the desk). Builder-style so existing callers/tests that
    /// don't route to a desk are unchanged.
    #[must_use]
    pub(crate) fn with_desk_routing(
        mut self,
        desk_edge: Option<Arc<crate::services::desk::RfqDeskEdge>>,
        desk: String,
        auto_quote: RatesAutoQuotePolicy,
    ) -> Self {
        self.desk_edge = desk_edge;
        self.desk = desk;
        self.auto_quote = auto_quote;
        self
    }

    /// Project the live engine market state into the wire market context the pricer
    /// consumes — the SAME projection `QuoteService::live_market` performs.
    async fn live_market(&self) -> Result<MarketContext, String> {
        let snap = self
            .link
            .market_snapshot()
            .await
            .map_err(|e| e.to_string())?;
        Ok(MarketContext::fx(
            snap.spot,
            snap.atm_vol,
            snap.r_dom,
            snap.r_for,
        ))
    }
}

/// A running FIX 4.4 acceptor: an accept loop over a bound `TcpListener`, spawning one
/// session task per connected counterparty. Each session drives the real
/// [`celnet_fix::Session`] FSM and serves the RFQ→Quote→lift→ExecutionReport lifecycle.
#[derive(Debug)]
pub struct FixAcceptor {
    local_addr: SocketAddr,
    accept_task: tokio::task::JoinHandle<()>,
}

impl FixAcceptor {
    /// Bind a FIX 4.4 acceptor on `addr` and spawn its accept loop. Pass a `:0` port
    /// to let the OS assign one (read back via [`FixAcceptor::local_addr`]).
    ///
    /// # Errors
    /// Returns an [`std::io::Error`] if the listener cannot bind.
    pub(crate) async fn start(addr: SocketAddr, ctx: FixContext) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        let accept_task = tokio::spawn(async move {
            // A failed `accept` (the listener gone) ends the loop; an accepted
            // connection is served on its own task.
            while let Ok((stream, _peer)) = listener.accept().await {
                let ctx = ctx.clone();
                // One task per session; the per-session state is single-owner (no
                // locks), torn down when the peer closes.
                tokio::spawn(async move {
                    let _ = serve_connection(stream, ctx).await;
                });
            }
        });
        Ok(Self {
            local_addr,
            accept_task,
        })
    }

    /// The actually-bound FIX listener address (resolves an ephemeral `:0` port).
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Stop accepting new FIX connections (in-flight sessions run to their own close).
    pub fn abort(&self) {
        self.accept_task.abort();
    }
}

/// Drive one connected FIX counterparty session to completion: pump frames between the
/// socket and the [`FixSession`] until the peer closes.
async fn serve_connection(stream: TcpStream, ctx: FixContext) -> std::io::Result<()> {
    stream.set_nodelay(true).ok();
    let cfg = SessionConfig {
        sender: ctx.sender.clone(),
        // FIX CompIDs are a pre-agreed pair (as the `celnet-fix` loopback tests
        // configure them): the session FSM rejects any peer whose SenderCompID is not
        // the configured counterparty with a CompID mismatch.
        target: ctx.counterparty.clone(),
        heart_bt_int: 30,
        role: Role::Acceptor,
    };
    let mut session = FixSession::new(cfg, ctx);
    session.serve(stream).await
}

/// The per-connection FIX acceptor session: the real session FSM plus the live-quote
/// table keyed by `QuoteID(117)`, and the shared keyed-MAC token ledger.
struct FixSession {
    session: Session<InMemoryStore>,
    ctx: FixContext,
    /// The keyed-MAC minter (one fresh OS-CSPRNG key per session) — the SAME minter
    /// the RFS stream uses, so a FIX token is unforgeable on the identical construction.
    minter: TokenMinter,
    /// The last-look / replay ledger — the SAME [`TokenLedger`] the RFS click-to-trade
    /// path uses; a FIX lift books through its [`TokenLedger::try_book`].
    ledger: TokenLedger,
    /// Live FIX quotes keyed by the wire `QuoteID(117)` string — the RFQ / FX auto-quote
    /// path (a lift names the `QuoteID`). The market-data stream venue uses
    /// [`Self::live_md`] instead (a lift names the `Symbol`).
    live: HashMap<Vec<u8>, FixQuote>,
    /// Live market-data quotes keyed by `Symbol(55)`: on a fixed-income STREAM venue each
    /// tick refreshes the liftable top-of-book token for a symbol here, so an inbound
    /// `NewOrderSingle(D)` naming that symbol (no `QuoteID`) books through the SAME ledger.
    /// Empty on a quote/RFQ or FX venue.
    live_md: HashMap<Vec<u8>, MdLiveQuote>,
    /// A monotonic line ordinal the keyed-MAC binds (the FIX analogue of the RFS
    /// `subscription_id`); fresh per quote so each line's tokens are distinct.
    next_line: u64,
    /// A counter minting unique `QuoteID`/`OrderID`/`ExecID` strings.
    seq: u64,
    /// Live market-data subscriptions keyed by the wire `MDReqID(262)`: on a fixed-income
    /// STREAM venue, the session's periodic ticker re-prices each and pushes a fresh
    /// `MarketDataSnapshotFullRefresh(W)`. Empty on a quote/RFQ venue (and while an idle
    /// stream venue has no subscription) — so the ticker branch is dormant and the loop
    /// stays byte-identical to the request-response path.
    md_subs: HashMap<Vec<u8>, MdSubscription>,
}

impl FixSession {
    fn new(cfg: SessionConfig, ctx: FixContext) -> Self {
        Self {
            session: Session::new(cfg, InMemoryStore::new()),
            ctx,
            minter: TokenMinter::new(),
            ledger: TokenLedger::new(),
            live: HashMap::new(),
            live_md: HashMap::new(),
            next_line: 0,
            seq: 0,
            md_subs: HashMap::new(),
        }
    }

    fn mint_id(&mut self, prefix: &str) -> Vec<u8> {
        self.seq += 1;
        format!("{prefix}-{}", self.seq).into_bytes()
    }

    /// The edge wall-clock SendingTime bytes (UTC `YYYYMMDD-HH:MM:SS.sss`) the session
    /// stamps on outbound frames. Derived from the edge clock (never a priced number).
    fn sending_time(&self) -> Vec<u8> {
        utc_timestamp(self.ctx.clock.now_nanos())
    }

    /// Run the session loop over a connected stream until the peer closes.
    async fn serve<RW>(&mut self, stream: RW) -> std::io::Result<()>
    where
        RW: AsyncRead + AsyncWrite + Unpin,
    {
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = FrameReader::new(read_half);
        // The market-data streaming cadence. A fixed-income STREAM venue re-prices every
        // live subscription on this interval and pushes fresh top-of-book snapshots; the
        // ticker branch is armed only while at least one subscription is live
        // (`!self.md_subs.is_empty()`), so a quote/RFQ venue — or an idle stream venue —
        // stays byte-identical to the pure request-response loop. `next_frame` is
        // cancellation-safe (its only await
        // commits the socket read into a struct-field buffer before any further await), so
        // `select!` dropping the read future on a tick loses no bytes.
        let mut ticker = tokio::time::interval(RFS_STREAM_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await; // consume the immediate first tick (interval fires at t=0)
        loop {
            tokio::select! {
                frame = reader.next_frame() => {
                    let Some(frame) = frame? else { break };
                    // Capture the inbound frame for the monitor screen, then the responses
                    // we emit — both tagged with this acceptor's connection id (best-effort
                    // observability off the pricing core; see `fix_monitor`).
                    let now = self.ctx.clock.now_nanos();
                    self.ctx.monitor.record(
                        &self.ctx.connection_id,
                        FixDirection::Inbound,
                        &frame,
                        now,
                    );
                    let st = self.sending_time();
                    let outbound = self.handle_frame(&frame, &st).await;
                    for f in outbound {
                        self.ctx.monitor.record(
                            &self.ctx.connection_id,
                            FixDirection::Outbound,
                            &f,
                            self.ctx.clock.now_nanos(),
                        );
                        write_frame(&mut write_half, &f).await?;
                    }
                    if self.session.state() == celnet_fix::session::SessionState::Disconnected
                        && self.session.next_outbound() > 1
                    {
                        break;
                    }
                }
                // Re-price + push every live market-data subscription. Dormant (guard
                // false, never polled) while no subscription is live, so an RFQ venue is
                // unaffected.
                _ = ticker.tick(), if !self.md_subs.is_empty() => {
                    let st = self.sending_time();
                    let frames = self.tick_md_stream(&st);
                    for f in frames {
                        self.ctx.monitor.record(
                            &self.ctx.connection_id,
                            FixDirection::Outbound,
                            &f,
                            self.ctx.clock.now_nanos(),
                        );
                        write_frame(&mut write_half, &f).await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Process one inbound frame and return all frames to transmit.
    async fn handle_frame(&mut self, raw: &[u8], st: &[u8]) -> Vec<Vec<u8>> {
        let action: SessionAction = match self.session.on_inbound(raw, st) {
            Ok(a) => a,
            // A protocol/session fault is surfaced as no application reply; the
            // session layer already emitted any admin reject. We never panic.
            Err(_) => return Vec::new(),
        };
        let mut out = action.outbound;
        if let Some(mt) = action.deliver {
            let frame = match FrameCursor::parse(raw) {
                Ok(f) => f,
                Err(_) => return out,
            };
            match mt {
                MsgType::QuoteRequest => self.on_quote_request(&frame, st, &mut out).await,
                MsgType::MarketDataRequest => {
                    self.on_market_data_request(&frame, st, &mut out).await;
                }
                MsgType::NewOrderSingle | MsgType::NewOrderMultileg => {
                    self.on_new_order(&frame, st, &mut out);
                }
                _ => {}
            }
        }
        out
    }

    /// Answer a `QuoteRequest(R)`: decode + convention-check the option, price it
    /// through the shared engine/surface path, stamp the keyed-MAC tokens, and reply a
    /// two-sided `Quote(S)` whose `QuoteID(117)` carries the line's tokens.
    async fn on_quote_request(
        &mut self,
        frame: &FrameCursor<'_>,
        st: &[u8],
        out: &mut Vec<Vec<u8>>,
    ) {
        let Some(req_id) = frame.get(131).map(<[u8]>::to_vec) else {
            return;
        };
        let Some(symbol) = frame.get(55).map(<[u8]>::to_vec) else {
            return;
        };

        // A dedicated fixed-income venue routes an inbound `QuoteRequest(R)` by intent:
        //  * the RFQ venue records it into the desk inbox and auto-quotes / routes-to-human
        //    (the FX-options path and the legacy content-detected OIS path below are
        //    unchanged / byte-identical);
        //  * the STREAM venue does NOT serve `QuoteRequest(R)` at all — it speaks Market
        //    Data, so streaming is opened by a `MarketDataRequest(V)`
        //    ([`Self::on_market_data_request`]); a stray `35=R` on the stream venue is
        //    out-of-contract and ignored.
        match rates_intent_for_kind(self.ctx.kind) {
            Some(RatesIntent::Rfq) => {
                self.on_rates_quote_request(frame, st, &req_id, &symbol, out)
                    .await;
                return;
            }
            Some(RatesIntent::Rfs) => return,
            None => {}
        }

        // Resolve the option + price it; a dialect/convention/pricing error declines
        // the quote (no `Quote` is sent — the maker simply does not show a price). The
        // FX-vanilla pre-trade template (ADR-0016 A1) rides alongside the priced line so
        // a lift can run the limit gate; a rates line carries `None`.
        let (priced, fx) = match self.price_request(frame).await {
            Ok(p) => p,
            Err(_) => return,
        };
        // Firm-wide OUTBOUND kill-switch: while outbound pricing is halted, suppress the
        // auto-quote frame (never push it to the client). Internal book state / limits
        // are untouched; nothing else in this path records history for an FX RFQ.
        if !self.ctx.pricing_control.outbound_enabled() {
            tracing::debug!(
                connection_id = %self.ctx.connection_id,
                "outbound pricing halted: suppressing FX RFQ auto-quote"
            );
            return;
        }
        self.emit_two_way_quote(st, &req_id, &symbol, &priced, fx, None, out);
    }

    /// Mint the keyed-MAC two-way tokens for a priced line and send the `Quote(S)`,
    /// registering the live quote so a subsequent lift books through the SAME ledger.
    /// Shared by the FX-options path and the fixed-income auto-quote path — the FX
    /// numbers/wire are byte-identical to before this extraction. `fx` is the BUY-side
    /// pre-trade [`BookedPosition`] template (ADR-0016 A1) an FX line carries for the
    /// lift-time limit gate; a rates line passes `None` (no canonical-vanilla risk leaf).
    ///
    /// `rates_request_id` is the desk history row a rates auto-quote/stream opened, so a
    /// lift of this quote books it as a completed deal (an FX line passes `None`). Returns
    /// the wire `QuoteID(117)` of the emitted quote so a streaming caller can retire the
    /// prior update.
    #[allow(clippy::too_many_arguments)] // the shared quote-emit inputs + the pre-trade/desk hooks.
    fn emit_two_way_quote(
        &mut self,
        st: &[u8],
        req_id: &[u8],
        symbol: &[u8],
        priced: &PricedLine,
        fx: Option<BookedPosition>,
        rates_request_id: Option<String>,
        out: &mut Vec<Vec<u8>>,
    ) -> Vec<u8> {
        // Best-order timer O1 (outbound quote publish, `OpKind::StreamPublish` —
        // "Quote publish (tick→quote)"): bracket the quote-ready → frame-emitted span (mint the
        // two-way tokens, build the `Quote(S)` wire frame, push it) with a monotonic `Instant`,
        // recorded into the shared telemetry hub before returning. Shared by the FX-options RFQ
        // auto-quote, the rates RFQ auto-quote, and the RFS/ESP stream tick, so every outbound
        // maker quote folds into this stage. The async FIX edge, never the pinned pricing core.
        let publish_t0 = std::time::Instant::now();
        let now = self.ctx.clock.now_nanos();
        let line_id = {
            self.next_line += 1;
            self.next_line
        };
        // Stamp the SAME keyed-MAC two-way tokens the RFS stream stamps (SELL@bid,
        // BUY@offer), registered in the SAME ledger that books a lift.
        let minted: Vec<MintedToken> = mint_two_way(
            &mut self.ledger,
            &self.minter,
            TwoWayLine {
                line_id,
                sequence: 1,
                bid: priced.bid,
                offer: priced.offer,
            },
            now,
            QUOTE_VALIDITY_NANOS,
        );
        let mut buy_token = None;
        let mut sell_token = None;
        for m in &minted {
            match m.side {
                Side::Buy => buy_token = Some(m.token),
                Side::Sell => sell_token = Some(m.token),
                Side::TwoWay => {}
            }
        }

        // The wire `QuoteID(117)` is the BUY token (always minted); the SELL token is
        // recorded alongside so a `Side=SELL` lift books the bid leg. Both authenticate
        // through the identical MAC ledger; the wire id just names the live quote.
        let quote_id = buy_token
            .map(|t| t.to_string().into_bytes())
            .unwrap_or_else(|| self.mint_id("Q"));
        self.live.insert(
            quote_id.clone(),
            FixQuote {
                symbol: symbol.to_vec(),
                buy_token,
                sell_token,
                fx,
                rates_request_id,
                mint_nanos: now,
            },
        );

        let valid_until = utc_timestamp(now.saturating_add(QUOTE_VALIDITY_NANOS));
        let frame_out = self.session.send_app(st, |h, e| {
            let p = QuoteParams {
                quote_req_id: req_id,
                quote_id: &quote_id,
                symbol,
                bid_px: priced.bid,
                offer_px: priced.offer,
                size: priced.size,
                valid_until: &valid_until,
            };
            messages::build_quote(h, &p, e)
        });
        out.push(frame_out);
        // O1: record the tick→quote publish latency into the per-`OpKind` store, off the hot core.
        self.ctx.link.telemetry().record_edge(
            celnet_observability::OpKind::StreamPublish,
            u64::try_from(publish_t0.elapsed().as_nanos()).unwrap_or(u64::MAX),
        );
        quote_id
    }

    /// Handle an inbound rates `QuoteRequest(R)` on a dedicated fixed-income venue:
    /// decode + intent-check the RFQ, record it into the desk inbox, and either
    /// auto-quote it (admitted by the venue policy → `Quote(S)` + QUOTED history) or
    /// route it to a human desk (declined → PENDING, no auto `Quote(S)`). The taker
    /// runs an Observe policy, so a routed RFQ receiving no immediate quote is expected.
    async fn on_rates_quote_request(
        &mut self,
        frame: &FrameCursor<'_>,
        st: &[u8],
        req_id: &[u8],
        symbol: &[u8],
        out: &mut Vec<Vec<u8>>,
    ) {
        let Some(intent) = rates_intent_for_kind(self.ctx.kind) else {
            return;
        };
        // A fixed-income venue serves BOTH arms; `SecurityType(167)=BOND` selects the
        // cash-bond arm, every other rates request is the OIS arm.
        if frame.get(167) == Some(dialect_rates::SEC_TYPE_BOND) {
            self.on_bond_quote_request(frame, st, req_id, symbol, out)
                .await;
            return;
        }
        let Ok(rfq) = dialect_rates::decode_rates_rfq(frame) else {
            return;
        };
        if !subscription_matches_intent(intent, rfq.subscription) {
            return;
        }
        let side = rates_side_to_side(rfq.side);
        // The CURRENT live curve (operator's most recent `MarkCurve`, else the static P0
        // default) — desk display + the curve arm of the pricing-source policy both price
        // off it, so editing SOFR pillars moves this venue's OIS quotes.
        let curve = self.live_rates_curve();
        // Classify the RFQ (exception-vs-routine). An admitted, priceable on-the-run clip
        // is auto-quoted; a routine large clip is routed quietly; an unknown security /
        // unconfigured tenor / pricing failure is routed as an ALERT-worthy manual
        // intervention. Every case records to the desk inbox — nothing is dropped.
        // Best-order timer O1 (rates quote CONSTRUCTION, `OpKind::RfqQuote`): bracket the desk
        // quote build — the classify + engine price step that constructs the par-rate / PV
        // two-way for this inbound RFQ — with a monotonic `Instant`, recorded into the shared
        // telemetry hub. The FIX session runs on the async edge (allocation-OK), never the
        // pinned pricing core (guardrail 11).
        let price_t0 = std::time::Instant::now();
        let admission = self.classify_ois_rfq(&rfq, intent, frame, &curve);
        self.ctx.link.telemetry().record_edge(
            celnet_observability::OpKind::RfqQuote,
            u64::try_from(price_t0.elapsed().as_nanos()).unwrap_or(u64::MAX),
        );
        // The display counterparty rides the RFQ's PartyID(448) when the initiator names one
        // (a SIM rotates realistic names per request), else the authenticated CompID.
        let counterparty = self.display_counterparty(frame);
        // Record the inbox row first (its id rides an auto-quote so a lift books the deal).
        let request_id = self.record_rates_rfq(&rfq, &counterparty, side, &curve, &admission);
        // Only an auto-quote shows a `Quote(S)`; routed / manual-intervention RFQs carry no
        // price (the desk prices them).
        if let RatesAdmission::Auto(priced) = &admission {
            let mid = 0.5 * (priced.bid + priced.offer);
            // Firm-wide OUTBOUND kill-switch: while outbound pricing is halted, suppress
            // the outbound `Quote(S)` frame (do NOT push it) — the desk-inbox row above
            // was already recorded, so nothing internal is lost.
            if self.ctx.pricing_control.outbound_enabled() {
                let quote_id = self.emit_two_way_quote(
                    st,
                    req_id,
                    symbol,
                    priced,
                    None,
                    request_id.clone(),
                    out,
                );
                // PRICING (class=Pricing): the outbound rates auto-quote CONSTRUCTED +
                // emitted on the async edge — the priced two-way (bid/offer/mid par) and
                // the quote id, correlated to the desk request. The pinned pricing core
                // never logs; this is the edge that published the maker quote.
                tracing::info!(
                    class = celnet_observability::LogClass::Pricing.label(),
                    connection_id = %self.ctx.connection_id,
                    counterparty = %counterparty,
                    request_id = request_id.as_deref(),
                    symbol = %String::from_utf8_lossy(symbol),
                    quote_id = %String::from_utf8_lossy(&quote_id),
                    bid = priced.bid,
                    offer = priced.offer,
                    mid,
                    size = priced.size,
                    good_for_nanos = QUOTE_VALIDITY_NANOS,
                    "rates RFQ auto-quote priced (outbound)",
                );
            } else {
                tracing::debug!(
                    connection_id = %self.ctx.connection_id,
                    "outbound pricing halted: suppressing OIS RFQ auto-quote"
                );
            }
        }
    }

    /// Classify an inbound OIS RFQ into the venue's admission decision (see
    /// [`RatesAdmission`]). The order encodes the exception-vs-routine policy: an unknown
    /// security and an unconfigured (off-the-run) tenor are exceptions a human is alerted
    /// to; a large on-the-run clip over the auto-quote cap is routine desk work (routed,
    /// quiet); an on-the-run, within-cap request that the engine still fails to price is a
    /// genuine pricing failure (alert).
    fn classify_ois_rfq(
        &self,
        rfq: &dialect_rates::RatesRfq,
        intent: RatesIntent,
        frame: &FrameCursor<'_>,
        curve: &CurveSet,
    ) -> RatesAdmission {
        // Only touch the engine when the RFQ clears the cheap policy gates (known symbol,
        // on-the-run tenor, within the clip cap); otherwise the pure classifier reports the
        // exact reason without pricing. A cleared-but-unpriceable RFQ reports a genuine
        // pricing failure (engine `Err`), distinct from an off-the-run tenor. The gated-in
        // price is sourced under this session's [`PricingSourceMode`]: the aggregated-book
        // composite drives the INITIAL quote (not just the RFS re-price) when the policy
        // selects it and a book covers the symbol, else the live curve. For a plain OIS
        // curve no book covers it, so every fallback mode resolves to the curve — an
        // ungrouped/`CurveOnly` session is byte-identical to before.
        let priced = if is_known_rates_symbol(&rfq.symbol)
            && self.ctx.auto_quote.tenors.contains(&rfq.tenor_years)
            && rfq.notional <= self.ctx.auto_quote.max_notional
        {
            self.priced_under_policy(&rfq.symbol, rfq.notional, false, || {
                rates_line(frame, Some(intent), curve).ok()
            })
        } else {
            None
        };
        classify_ois_admission(
            &self.ctx.auto_quote,
            &rfq.symbol,
            rfq.tenor_years,
            rfq.notional,
            priced,
        )
    }

    /// Resolve the aggregated-book composite two-way for a streamed instrument, applying
    /// **this connection's** pricing group when one resolves — the seam that makes the FIX
    /// RFS/ESP outbound composite-based **and tiered**, matching the gRPC/WS RFQ path
    /// ([`super::quote::QuoteEdge::book_composite_for_caller`]). A grouped connection prices
    /// off the book's RAW consolidated composite through its own effective RFS/RFQ pipeline
    /// (`share_pipeline ? esp : rfq`); an ungrouped one receives the book-default composite.
    ///
    /// Returns `None` when no hub is wired, no enabled book covers `symbol`, or the
    /// composite is degenerate — the RFS ticker then falls back to the standalone P0 demo
    /// re-price, so an instrument no book covers (e.g. a plain OIS curve) and the legacy
    /// env seed stay byte-identical. Runs on the FIX **session ticker task**, never the
    /// pinned zero-alloc pricer, so the composite/pipeline lookup (which takes the hub
    /// read locks) respects guardrail 11.
    fn composite_two_way(&self, symbol: &[u8], size: f64) -> Option<PricedLine> {
        let hub = self.ctx.aggregation_hub.as_ref()?;
        // Key off the FIX `Symbol(55)` TRIMMED — the book publishes each line under the
        // canonical `instrument_id` the LP fed (a bond's CUSIP / a govvie slug), and the
        // FIX client sends that same id as the symbol; the only benign divergence is
        // padding whitespace, which we strip here so a covered book is actually found.
        // (There is no case-fold: CUSIPs are case-significant and slugs are lower-case,
        // so folding could conflate distinct ids.)
        let instrument_id = std::str::from_utf8(symbol).ok()?.trim();
        let resolver = hub.pricing_groups();
        let desk = self.ctx.desk.trim();
        let comp = match resolver
            .resolve_for_connection(&self.ctx.connection_id, (!desk.is_empty()).then_some(desk))
        {
            Some(group) => hub.resolve_rfq_composite_priced(
                instrument_id,
                &group.id,
                group.rfq_effective_pipeline(),
            ),
            None => hub.resolve_rfq_composite(instrument_id),
        };
        match comp {
            Some(comp) => Some(PricedLine {
                bid: comp.best_bid,
                offer: comp.best_offer,
                size,
            }),
            None => {
                // Distinguish an UNFED book (covered but no live line / below quorum /
                // degenerate) from a KEY-MISMATCH (no enabled book's scope admits the id)
                // — the current silent `None` is a diagnosis trap. Off the pinned pricer
                // (guardrail 11); `debug` so it never spams a production `info` stream.
                tracing::debug!(
                    class = celnet_observability::LogClass::Pricing.label(),
                    connection_id = %self.ctx.connection_id,
                    instrument_id,
                    reason = hub.diagnose_composite_miss(instrument_id).label(),
                    "no aggregated-book composite for FIX symbol — falling back to curve",
                );
                None
            }
        }
    }

    /// This connection's resolved **pricing-source policy** `(mode, book_skew_weight)`:
    /// the mode + blend weight of the pricing group this FIX session resolves to (by
    /// connection id, then desk fallback), or the platform default when no hub is wired
    /// or no group claims this session. Read off the pinned pricer (guardrail 11) — a
    /// short read-lock on the hub's cached resolver.
    fn pricing_policy(&self) -> (PricingSourceMode, f64) {
        let Some(hub) = self.ctx.aggregation_hub.as_ref() else {
            return (PricingSourceMode::default(), DEFAULT_BOOK_SKEW_WEIGHT);
        };
        let resolver = hub.pricing_groups();
        let desk = self.ctx.desk.trim();
        match resolver
            .resolve_for_connection(&self.ctx.connection_id, (!desk.is_empty()).then_some(desk))
        {
            Some(group) => (group.pricing_source_mode, group.book_skew_weight),
            None => (PricingSourceMode::default(), DEFAULT_BOOK_SKEW_WEIGHT),
        }
    }

    /// This session's resolved **market-data (ESP) last-look policy**
    /// `(mode, tolerance_bps, giveback_pct)`: the last-look settings of the pricing group
    /// this FIX session resolves to (by connection id, then desk fallback), or the platform
    /// defaults when no hub is wired or no group claims this session. Read on the async FIX
    /// edge (a short read-lock on the hub's cached resolver), never the pinned pricer
    /// (guardrail 11).
    fn last_look_policy(&self) -> (LastLookMode, f64, f64) {
        let Some(hub) = self.ctx.aggregation_hub.as_ref() else {
            return (
                LastLookMode::default(),
                DEFAULT_LAST_LOOK_TOLERANCE_BPS,
                DEFAULT_ASYNC_GIVEBACK_PCT,
            );
        };
        let resolver = hub.pricing_groups();
        let desk = self.ctx.desk.trim();
        match resolver
            .resolve_for_connection(&self.ctx.connection_id, (!desk.is_empty()).then_some(desk))
        {
            Some(group) => (
                group.last_look_mode,
                group.last_look_tolerance_bps,
                group.async_giveback_pct,
            ),
            None => (
                LastLookMode::default(),
                DEFAULT_LAST_LOOK_TOLERANCE_BPS,
                DEFAULT_ASYNC_GIVEBACK_PCT,
            ),
        }
    }

    /// Price a rates/bond line under this session's [`PricingSourceMode`], the seam that
    /// makes the aggregated book drive the price **from the initial quote** — not just the
    /// RFS re-price tick. `curve_line` is the internal-curve two-way (lazily evaluated, so
    /// a mode that never needs the curve never prices it); `is_bond` selects the product
    /// arm for [`PricingSourceMode::ProductSplit`]. Returns `None` only when the selected
    /// source(s) all fail to produce a well-formed two-way (e.g. an unpriceable tenor with
    /// no covering book) — the caller then declines/quotes nothing, never a fabricated
    /// price. Runs on the async FIX edge, never the pinned pricer (guardrail 11).
    fn priced_under_policy(
        &self,
        symbol: &[u8],
        size: f64,
        is_bond: bool,
        curve_line: impl FnOnce() -> Option<PricedLine>,
    ) -> Option<PricedLine> {
        let (mode, weight) = self.pricing_policy();
        match mode {
            PricingSourceMode::CurveOnly => curve_line(),
            PricingSourceMode::CompositeFirstCurveFallback => {
                self.composite_two_way(symbol, size).or_else(curve_line)
            }
            PricingSourceMode::ProductSplit => {
                if is_bond {
                    self.composite_two_way(symbol, size).or_else(curve_line)
                } else {
                    curve_line()
                }
            }
            PricingSourceMode::CurveAnchoredBookSkew => {
                let curve = curve_line();
                match (self.composite_two_way(symbol, size), curve) {
                    // Both present: curve backbone, mid skewed `weight` of the way toward
                    // the composite, spread taken from the (real-liquidity) composite.
                    (Some(comp), Some(curve)) => {
                        Some(blend_curve_toward_composite(&curve, &comp, weight))
                    }
                    // Only one source priced: use it (no book ⇒ pure curve; no curve ⇒
                    // the composite is still a real price, never fabricated).
                    (None, Some(curve)) => Some(curve),
                    (Some(comp), None) => Some(comp),
                    (None, None) => None,
                }
            }
        }
    }

    /// The live rates curve this FIX session prices off: the operator's most recent
    /// `MarkCurve` for the supported currency (so editing SOFR pillars in the GUI moves
    /// outbound OIS/bond FIX quotes), or the static P0 default when none has been marked.
    /// A snapshot clone taken off the pinned pricer (guardrail 11).
    fn live_rates_curve(&self) -> CurveSet {
        self.ctx
            .surface_book
            .live_curve(crate::rates_pricing::SUPPORTED_CURRENCY)
            .unwrap_or_else(crate::rates_pricing::default_usd_sofr_curve_set)
    }

    /// Re-price every live market-data subscription and return the fresh
    /// `MarketDataSnapshotFullRefresh(W)` frames to push. Each subscription is priced under
    /// this connection's [`PricingSourceMode`](Self::priced_under_policy): the aggregated-
    /// book composite (group-tiered) where a book covers it, else the **current live curve**
    /// re-resolved each tick (so a `MarkCurve` moves the stream — no fabricated movement).
    /// Each snapshot refreshes the symbol's liftable token in [`Self::live_md`] (overwriting
    /// the prior one — the latest top-of-book is what a lift hits). Driven by the session
    /// loop's streaming ticker; returns an empty vec when no subscription is live.
    fn tick_md_stream(&mut self, st: &[u8]) -> Vec<Vec<u8>> {
        // Firm-wide OUTBOUND kill-switch: while outbound pricing is halted, pause every
        // live market-data stream (push no updates). Cheap `Relaxed` load off the pinned
        // pricer; the ticker re-checks each tick, so streams resume from live on
        // re-enable (the subscriptions stay registered, never torn down).
        if !self.ctx.pricing_control.outbound_enabled() {
            return Vec::new();
        }
        // The operator's most recent curve, snapshot once for this whole tick (every
        // subscription that falls back to the curve prices off the same mark).
        let curve = self.live_rates_curve();
        let mut frames = Vec::new();
        // Snapshot the keys so the loop can re-borrow `self` (live table, quote minter)
        // between subscriptions while mutating the subscription set in place.
        let keys: Vec<Vec<u8>> = self.md_subs.keys().cloned().collect();
        for key in keys {
            let (md_req_id, symbol, notional, request_id, line) = {
                let Some(sub) = self.md_subs.get(&key) else {
                    continue;
                };
                (
                    sub.md_req_id.clone(),
                    sub.symbol.clone(),
                    sub.notional,
                    sub.request_id.clone(),
                    sub.line.clone(),
                )
            };
            // Price under the session policy: composite where a book covers the streamed
            // instrument (design §5), else the current live curve re-priced from the
            // retained line inputs. A tick that cannot price (e.g. a re-marked curve now
            // missing this tenor) skips this symbol, leaving its existing live token intact
            // rather than blanking the stream.
            let Some(priced) = self.priced_under_policy(&symbol, notional, line.is_bond(), || {
                curve_line_for(&line, &curve, notional)
            }) else {
                continue;
            };
            self.emit_md_snapshot(st, &md_req_id, &symbol, &priced, request_id, &mut frames);
        }
        frames
    }

    /// Mint the keyed-MAC two-way tokens for a streamed line and push a
    /// `MarketDataSnapshotFullRefresh(W)`, refreshing the symbol's liftable token in
    /// [`Self::live_md`]. The market-data analogue of [`Self::emit_two_way_quote`]: the
    /// SAME token minting + ledger registration (so a `NewOrderSingle(D)` naming the symbol
    /// books through the identical last-look ledger), but the wire frame is a top-of-book
    /// snapshot keyed by `Symbol(55)` rather than a `Quote(S)` keyed by `QuoteID(117)`. A
    /// fresh snapshot OVERWRITES the symbol's entry — the latest published top-of-book is
    /// what a lift executes against; the superseded token simply lapses in the ledger's
    /// validity window. Runs on the async FIX edge, never the pinned pricer (guardrail 11).
    fn emit_md_snapshot(
        &mut self,
        st: &[u8],
        md_req_id: &[u8],
        symbol: &[u8],
        priced: &PricedLine,
        request_id: Option<String>,
        out: &mut Vec<Vec<u8>>,
    ) {
        // Best-order timer O1 (outbound snapshot publish, `OpKind::StreamPublish`): bracket
        // the snapshot-ready → frame-emitted span with a monotonic `Instant`.
        let publish_t0 = std::time::Instant::now();
        let now = self.ctx.clock.now_nanos();
        let line_id = {
            self.next_line += 1;
            self.next_line
        };
        // Supersede this SYMBOL's prior tokens individually (retire only its own — NOT the
        // whole-ledger `clear_live`, which would wipe every OTHER live symbol's token and is
        // the root cause of the multi-instrument lift-reject bug). The shared session ledger
        // holds one liftable two-way per subscribed symbol simultaneously.
        if let Some(prev) = self.live_md.get(symbol) {
            if let Some(t) = prev.buy_token {
                self.ledger.retire(t);
            }
            if let Some(t) = prev.sell_token {
                self.ledger.retire(t);
            }
        }
        // The SAME keyed-MAC two-way tokens the RFQ auto-quote stamps (SELL@bid, BUY@offer),
        // registered in the SAME ledger that books a lift — via the MULTI-line mint that does
        // NOT clear sibling symbols' live tokens.
        let minted: Vec<MintedToken> = mint_two_way_no_clear(
            &mut self.ledger,
            &self.minter,
            TwoWayLine {
                line_id,
                sequence: 1,
                bid: priced.bid,
                offer: priced.offer,
            },
            now,
            QUOTE_VALIDITY_NANOS,
        );
        let mut buy_token = None;
        let mut sell_token = None;
        for m in &minted {
            match m.side {
                Side::Buy => buy_token = Some(m.token),
                Side::Sell => sell_token = Some(m.token),
                Side::TwoWay => {}
            }
        }
        // Refresh (overwrite) the symbol's liftable top-of-book token + the published prices
        // a by-symbol lift's `Price(44)` must match (last-look on price).
        self.live_md.insert(
            symbol.to_vec(),
            MdLiveQuote {
                buy_token,
                sell_token,
                bid: priced.bid,
                offer: priced.offer,
                request_id,
                mint_nanos: now,
            },
        );
        let frame_out = self.session.send_app(st, |h, e| {
            let p = messages::MarketDataSnapshotParams {
                md_req_id,
                symbol,
                bid_px: priced.bid,
                offer_px: priced.offer,
                size: priced.size,
            };
            messages::build_market_data_snapshot(h, &p, e)
        });
        out.push(frame_out);
        self.ctx.link.telemetry().record_edge(
            celnet_observability::OpKind::StreamPublish,
            u64::try_from(publish_t0.elapsed().as_nanos()).unwrap_or(u64::MAX),
        );
    }

    /// Handle an inbound `MarketDataRequest(V)` on a fixed-income STREAM venue: a subscribe
    /// (`SubscriptionRequestType(263)=1`) decodes the single instrument block (bond or OIS),
    /// records a desk-inbox row (so a lift books a deal), registers the subscription so the
    /// session ticker re-prices + pushes `MarketDataSnapshotFullRefresh(W)` updates, and
    /// emits an INITIAL snapshot; an unsubscribe (`263=2`) tears the subscription down. Only
    /// a STREAM venue serves this — a quote/RFQ or FX venue ignores it.
    async fn on_market_data_request(
        &mut self,
        frame: &FrameCursor<'_>,
        st: &[u8],
        out: &mut Vec<Vec<u8>>,
    ) {
        // Market-data streaming is the STREAM venue's contract only.
        if rates_intent_for_kind(self.ctx.kind) != Some(RatesIntent::Rfs) {
            return;
        }
        let view = messages::MarketDataRequestView::new(*frame);
        let Some(md_req_id) = view.md_req_id().map(<[u8]>::to_vec) else {
            return;
        };
        let Some(sub_type) = view.subscription_type() else {
            return;
        };
        // An unsubscribe tears down the live stream for this correlation id (no snapshot,
        // no desk row) — the ticker no longer re-prices it, and its liftable token lapses.
        if sub_type == messages::SUBSCRIPTION_DISABLE[0] {
            if let Some(sub) = self.md_subs.remove(&md_req_id)
                && let Some(md) = self.live_md.remove(&sub.symbol)
            {
                if let Some(t) = md.buy_token {
                    self.ledger.retire(t);
                }
                if let Some(t) = md.sell_token {
                    self.ledger.retire(t);
                }
            }
            return;
        }
        // Only a subscribe (snapshot+updates) opens a stream; any other type is ignored.
        if sub_type != messages::SUBSCRIPTION_SNAPSHOT_UPDATES[0] {
            return;
        }
        let Some(symbol) = view.symbol().map(<[u8]>::to_vec) else {
            return;
        };
        // The CURRENT live curve (operator's most recent `MarkCurve`, else static P0) — the
        // curve arm of the pricing-source policy + the desk display curve.
        let curve = self.live_rates_curve();
        // Decode the single instrument block. `SecurityType(167)=BOND` selects the cash-bond
        // arm (the family an aggregated book covers); every other request is the OIS arm.
        let (line, notional, is_bond, record) = if frame.get(167)
            == Some(dialect_rates::SEC_TYPE_BOND)
        {
            let Ok((instrument, notional)) = dialect_rates::decode_bond_instrument(frame) else {
                return;
            };
            let rfq = dialect_rates::BondRfq {
                quote_req_id: md_req_id.clone(),
                symbol: symbol.clone(),
                notional,
                subscription: dialect_rates::SubscriptionRequest::Subscribe,
                instrument,
            };
            let line = RfsLine::Bond {
                instrument: Box::new(instrument),
            };
            (line, notional, true, MdRecord::Bond(rfq))
        } else {
            let Ok((tenor_years, notional, side)) = dialect_rates::decode_ois_instrument(frame)
            else {
                return;
            };
            let rfq = dialect_rates::RatesRfq {
                quote_req_id: md_req_id.clone(),
                symbol: symbol.clone(),
                tenor_years,
                notional,
                side,
                subscription: dialect_rates::SubscriptionRequest::Subscribe,
            };
            let line = RfsLine::Ois { tenor_years };
            (line, notional, false, MdRecord::Ois(rfq))
        };
        // Price the initial line under this session's pricing-source policy — the SAME path
        // the streaming ticker uses ([`Self::priced_under_policy`] + [`curve_line_for`]):
        // the aggregated-book composite where a book covers the instrument, else the live
        // curve re-priced from the decoded line inputs. A priceable line is auto-quoted
        // (QUOTED desk row + initial snapshot); an unpriceable one routes to a human
        // (PENDING, no snapshot until the book/curve can price it).
        let admission = match self.priced_under_policy(&symbol, notional, is_bond, || {
            curve_line_for(&line, &curve, notional)
        }) {
            Some(priced) => RatesAdmission::Auto(priced),
            None => RatesAdmission::Manual(ManualInterventionReason::PricingFailure),
        };
        let counterparty = self.display_counterparty(frame);
        // Record the desk-inbox row first (its id rides the stream so a lift books a deal).
        let request_id = match &record {
            MdRecord::Bond(rfq) => self.record_bond_rfq(rfq, &counterparty, &curve, &admission),
            MdRecord::Ois(rfq) => {
                let side = rates_side_to_side(rfq.side);
                self.record_rates_rfq(rfq, &counterparty, side, &curve, &admission)
            }
        };
        // Register the subscription so the ticker re-prices + pushes updates until an
        // unsubscribe (or session close), keyed by the MDReqID.
        self.md_subs.insert(
            md_req_id.clone(),
            MdSubscription {
                md_req_id: md_req_id.clone(),
                symbol: symbol.clone(),
                notional,
                line,
                request_id: request_id.clone(),
            },
        );
        // Emit an INITIAL snapshot when the line priced and outbound pricing is enabled
        // (the kill-switch suppresses the snapshot but keeps the subscription registered, so
        // the ticker resumes streaming from live on re-enable).
        if let RatesAdmission::Auto(priced) = &admission {
            if self.ctx.pricing_control.outbound_enabled() {
                self.emit_md_snapshot(st, &md_req_id, &symbol, priced, request_id, out);
                tracing::info!(
                    class = celnet_observability::LogClass::Pricing.label(),
                    connection_id = %self.ctx.connection_id,
                    counterparty = %counterparty,
                    md_req_id = %String::from_utf8_lossy(&md_req_id),
                    symbol = %String::from_utf8_lossy(&symbol),
                    bid = priced.bid,
                    offer = priced.offer,
                    size = priced.size,
                    "market-data subscribe: initial snapshot published (outbound)",
                );
            } else {
                tracing::debug!(
                    connection_id = %self.ctx.connection_id,
                    "outbound pricing halted: suppressing initial market-data snapshot"
                );
            }
        }
    }

    /// Record an inbound rates RFQ into the desk inbox under this venue's desk, mapping the
    /// [`RatesAdmission`] to the desk-edge [`RfqIngestOutcome`]:
    /// [`Auto`](RatesAdmission::Auto) ⇒ QUOTED at the two-way mid;
    /// [`RoutedToDesk`](RatesAdmission::RoutedToDesk) ⇒ PENDING (quiet);
    /// [`Manual`](RatesAdmission::Manual) ⇒ PENDING with an alert-worthy reason. Returns
    /// the stored request id (so a lift can book an auto-quote), or `None` when the
    /// acceptor is not desk-routed (legacy env seed / tests) or carries no desk.
    /// The DISPLAY counterparty for an inbound RFQ: the `PartyID(448)` the initiator named
    /// (the client on whose behalf the RFQ was entered — see
    /// [`celnet_fix::messages::push_originating_party`]), falling back to the session's
    /// authenticated `TargetCompID` when the party block is absent/empty. A managed SIM
    /// varies `PartyID(448)` per RFQ so the blotter shows realistic, varied names over one
    /// FIX session and counterparty-keyed risk-routing rules become exercisable; the real
    /// gateway path names no party, so `counterparty` stays the authenticated CompID.
    fn display_counterparty(&self, frame: &FrameCursor<'_>) -> String {
        frame
            .get(celnet_fix::messages::TAG_PARTY_ID)
            .filter(|id| !id.is_empty())
            .map_or_else(
                || String::from_utf8_lossy(&self.ctx.counterparty).into_owned(),
                |id| String::from_utf8_lossy(id).into_owned(),
            )
    }

    fn record_rates_rfq(
        &self,
        rfq: &dialect_rates::RatesRfq,
        counterparty: &str,
        side: Side,
        curve: &CurveSet,
        admission: &RatesAdmission,
    ) -> Option<String> {
        let edge = self.ctx.desk_edge.as_ref()?;
        if self.ctx.desk.trim().is_empty() {
            return None;
        }
        let (fixed_rate, outcome) = match admission {
            RatesAdmission::Auto(priced) => {
                let mid = 0.5 * (priced.bid + priced.offer);
                (
                    mid,
                    RfqIngestOutcome::AutoQuoted(DeskQuote {
                        price: mid,
                        notional: rfq.notional,
                        valid_for_ms: RATES_AUTO_QUOTE_VALID_MS,
                        trader: "auto".to_owned(),
                    }),
                )
            }
            RatesAdmission::RoutedToDesk => (0.0, RfqIngestOutcome::RoutedToDesk),
            RatesAdmission::Manual(reason) => (0.0, RfqIngestOutcome::ManualIntervention(*reason)),
        };
        let instrument = RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                tenor_years: rfq.tenor_years,
                fixed_rate,
                notional: rfq.notional,
                side: side as i32,
            })),
        };
        let stored = edge.ingest_fix_rfq(
            &self.ctx.desk,
            counterparty,
            instrument,
            curve.clone(),
            side,
            rfq.notional,
            outcome,
        );
        Some(stored.request_id)
    }

    /// Handle an inbound cash-bond `QuoteRequest(R)` on a dedicated fixed-income venue —
    /// the bond analogue of [`Self::on_rates_quote_request`]'s OIS body: decode +
    /// intent-check the bond RFQ, record it into the desk inbox, and either auto-quote it
    /// (admitted by the notional policy → `Quote(S)` + QUOTED history) or route it to a
    /// human desk (declined → PENDING). The auto-quote runs through the SAME
    /// [`bond_line`] pricing + [`Self::emit_two_way_quote`] token/quote machinery as an
    /// OIS or FX line — a bond carries no canonical-vanilla risk leaf, so no FX pre-trade
    /// template.
    async fn on_bond_quote_request(
        &mut self,
        frame: &FrameCursor<'_>,
        st: &[u8],
        req_id: &[u8],
        symbol: &[u8],
        out: &mut Vec<Vec<u8>>,
    ) {
        let Some(intent) = rates_intent_for_kind(self.ctx.kind) else {
            return;
        };
        let Ok(rfq) = dialect_rates::decode_bond_rfq(frame) else {
            return;
        };
        if !subscription_matches_intent(intent, rfq.subscription) {
            return;
        }
        // The CURRENT live curve (operator's most recent `MarkCurve`, else static P0) — the
        // curve arm of the pricing-source policy + the desk display curve.
        let curve = self.live_rates_curve();
        // Classify (bond arm has no whole-year tenor to gate, so the clip cap is the only
        // routine route): over the cap ⇒ routed quietly to the desk; within the cap and
        // priceable ⇒ auto-quote; within the cap but the engine cannot price it (e.g. a
        // maturity that does not resolve) ⇒ an ALERT-worthy pricing failure. The gated-in
        // price is sourced under this session's [`PricingSourceMode`]: a bond is the arm an
        // aggregated book actually covers, so the composite drives the INITIAL quote (not
        // just the RFS re-price) when the policy selects it and a book covers the symbol,
        // else the live curve.
        let admission = if rfq.notional > self.ctx.auto_quote.max_notional {
            RatesAdmission::RoutedToDesk
        } else {
            match self.priced_under_policy(symbol, rfq.notional, true, || {
                bond_line(frame, Some(intent), &curve).ok()
            }) {
                Some(priced) => RatesAdmission::Auto(priced),
                None => RatesAdmission::Manual(ManualInterventionReason::PricingFailure),
            }
        };
        // Record the inbox row first so its id can ride an auto-quote (a lift then books
        // the deal), exactly as the OIS arm does.
        let counterparty = self.display_counterparty(frame);
        let request_id = self.record_bond_rfq(&rfq, &counterparty, &curve, &admission);
        if let RatesAdmission::Auto(priced) = &admission {
            // Firm-wide OUTBOUND kill-switch: suppress the outbound `Quote(S)` while halted
            // (the desk-inbox row above already recorded it).
            if self.ctx.pricing_control.outbound_enabled() {
                self.emit_two_way_quote(st, req_id, symbol, priced, None, request_id, out);
            } else {
                tracing::debug!(
                    connection_id = %self.ctx.connection_id,
                    "outbound pricing halted: suppressing bond RFQ auto-quote"
                );
            }
        }
    }

    /// Record an inbound cash-bond RFQ into the desk inbox under this venue's desk — the
    /// bond analogue of [`Self::record_rates_rfq`]. When `auto_level` is `Some`, the
    /// venue auto-quoted → stored QUOTED at that clean-price level; otherwise stored
    /// PENDING for a human trader. A no-op when the acceptor is not desk-routed or
    /// carries no desk. The recorded instrument is the LANDED [`RatesInstrument`] `Bond`
    /// arm the decode produced (its `side` is the request's side, `SIDE_TWO_WAY` for a
    /// two-way request).
    fn record_bond_rfq(
        &self,
        rfq: &dialect_rates::BondRfq,
        counterparty: &str,
        curve: &CurveSet,
        admission: &RatesAdmission,
    ) -> Option<String> {
        let edge = self.ctx.desk_edge.as_ref()?;
        if self.ctx.desk.trim().is_empty() {
            return None;
        }
        let side = Side::try_from(rfq.instrument.side).unwrap_or(Side::TwoWay);
        let instrument = RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Bond(rfq.instrument)),
        };
        let outcome = match admission {
            RatesAdmission::Auto(priced) => RfqIngestOutcome::AutoQuoted(DeskQuote {
                price: 0.5 * (priced.bid + priced.offer),
                notional: rfq.notional,
                valid_for_ms: RATES_AUTO_QUOTE_VALID_MS,
                trader: "auto".to_owned(),
            }),
            RatesAdmission::RoutedToDesk => RfqIngestOutcome::RoutedToDesk,
            RatesAdmission::Manual(reason) => RfqIngestOutcome::ManualIntervention(*reason),
        };
        let stored = edge.ingest_fix_rfq(
            &self.ctx.desk,
            counterparty,
            instrument,
            curve.clone(),
            side,
            rfq.notional,
            outcome,
        );
        Some(stored.request_id)
    }

    /// Execute a `NewOrderSingle(D)` / `NewOrderMultileg(AB)` against a live quote:
    /// select the per-side token by `Side(54)`, present it to the SAME keyed-MAC
    /// last-look ledger ([`TokenLedger::try_book`]), and reply a fill or reject
    /// `ExecutionReport(8)` — rejecting a stale / forged / already-consumed / unknown
    /// token exactly as the gRPC click-to-trade path declines.
    fn on_new_order(&mut self, frame: &FrameCursor<'_>, st: &[u8], out: &mut Vec<Vec<u8>>) {
        let cl_ord_id = frame.get(11).map(<[u8]>::to_vec).unwrap_or_default();
        let side_byte = frame
            .get(54)
            .and_then(|v| v.first().copied())
            .unwrap_or(b'1');
        let quote_id = frame.get(117).map(<[u8]>::to_vec);
        let now = self.ctx.clock.now_nanos();

        // Resolve the lift target. A `QuoteID(117)` naming a live RFQ/FX quote takes the
        // QuoteID path (`self.live` — byte-identical to the RFQ/FX flow). Otherwise a
        // market-data STREAM lift names the `Symbol(55)` (a snapshot carries no QuoteID):
        // resolve the CURRENT top-of-book token published for that symbol from
        // [`Self::live_md`]. Either way the per-side token books through the SAME ledger.
        let live_quote = quote_id.as_ref().and_then(|q| self.live.get(q));
        // A market-data lift carries the streamed level in `Price(44)` (`q`, the price the
        // taker hit). This session's configurable last-look policy compares it to the
        // current published side price (`c`) at order arrival: an adverse move beyond the
        // tolerance band rejects the lift as superseded; a favorable move is kept (Sync) or
        // partly returned to the client (Async). Resolved once off the pricing group (async
        // FIX edge, guardrail 11).
        let presented_px = frame.get(44).and_then(dialect_fx::parse_float);
        let (ll_mode, ll_tolerance_bps, ll_giveback_pct) = self.last_look_policy();
        let is_sell = side_byte == dialect_fx::SIDE_SELL;
        let (
            mut token,
            symbol,
            fx_line,
            rates_request_id,
            quote_mint_nanos,
            via_md,
            md_superseded,
            md_fill_px,
        ) = match live_quote {
            Some(lq) => {
                let token = if is_sell { lq.sell_token } else { lq.buy_token };
                (
                    token,
                    lq.symbol.clone(),
                    lq.fx,
                    lq.rates_request_id.clone(),
                    Some(lq.mint_nanos),
                    false,
                    false,
                    None,
                )
            }
            None => {
                let sym = frame.get(55).map(<[u8]>::to_vec).unwrap_or_default();
                match self.live_md.get(&sym) {
                    Some(md) => {
                        let (token, cur_px) = if is_sell {
                            (md.sell_token, md.bid)
                        } else {
                            (md.buy_token, md.offer)
                        };
                        // Configurable market-data last-look. A client that omits `Price(44)`
                        // books at the current price (lenient fallback — no `q` to compare, so
                        // no adverse/favorable move and no supersede).
                        let (superseded, fill_px) = match presented_px {
                            Some(q) => match md_last_look_decision(
                                is_sell,
                                q,
                                cur_px,
                                ll_mode,
                                ll_tolerance_bps,
                                ll_giveback_pct,
                            ) {
                                MdLastLook::Fill(px) => (false, Some(px)),
                                MdLastLook::Superseded => (true, None),
                            },
                            None => (false, None),
                        };
                        // A market-data line carries no FX pre-trade template (rates).
                        (
                            token,
                            sym,
                            None,
                            md.request_id.clone(),
                            Some(md.mint_nanos),
                            true,
                            superseded,
                            fill_px,
                        )
                    }
                    None => (None, sym, None, None, None, false, false, None),
                }
            }
        };
        // A superseded market-data lift books nothing: drop the token so the ledger is never
        // consumed (the live token stays liftable at the CURRENT price for a fresh re-lift).
        if md_superseded {
            token = None;
        }

        // Book through the SAME last-look ledger the RFQ/stream path uses. An unknown
        // quote/side ⇒ UnknownToken (no live token), exactly as a forged token.
        let outcome = match token {
            Some(t) => self.ledger.try_book(t, now),
            None => BookOutcome::UnknownToken,
        };
        // Last-look verdict label for the incoming-order log below (async edge).
        let last_look = match &outcome {
            BookOutcome::Booked { .. } => "accepted",
            BookOutcome::Expired => "expired",
            BookOutcome::AlreadyConsumed => "replayed",
            BookOutcome::UnknownToken => "unknown",
        };

        let (mut filled, mut premium, mut text): (bool, f64, Option<&[u8]>) = match outcome {
            BookOutcome::Booked { premium, .. } => (true, premium, None),
            BookOutcome::Expired => (false, 0.0, Some(b"quote expired (last-look)")),
            BookOutcome::AlreadyConsumed => (false, 0.0, Some(b"quote already executed")),
            BookOutcome::UnknownToken => (false, 0.0, Some(b"unknown or forged quote")),
        };
        // A superseded market-data lift is a moved-market last-look decline, not a forged
        // token — surface the accurate reason (filled is already false: token was dropped).
        if md_superseded {
            text = Some(b"quote superseded: market moved (last-look)");
        }
        // The ACTUAL fill price of a market-data lift is the last-look policy result (the
        // client's requested price, honored or improved toward the current price under an
        // Async giveback) — NOT the raw token premium (the current published price). Report
        // and book THAT dealt price so the wire `35=8`, the booked position, the internalise
        // edge, risk routing and hedging all see the real economics.
        if filled && let Some(px) = md_fill_px {
            premium = px;
        }

        // Pre-trade limit gate (ADR-0016 A1): a filled FX-vanilla lift consults the SAME
        // shared limit tree the RFS click-to-trade sink enforces. A hard breach converts
        // the fill into a rejected `ExecutionReport(ExecType=8)` with a `Text(58)` reason
        // BEFORE the fill is emitted. The last-look token was already consumed by the
        // ledger above, so the line is dead (a re-lift now rejects as already-consumed)
        // and the trader re-requests — mirroring the gRPC/RFS reject-after-consume path.
        // A rates line (no `fx` template) or a canonicalization miss is not gated.
        let limit_reason: Option<String> = if filled {
            fx_line.and_then(|template| {
                let mut booked = template;
                if side_byte == dialect_fx::SIDE_SELL {
                    booked.notional_base = -booked.notional_base;
                }
                match self.ctx.store.evaluate_pre_trade(&booked) {
                    Ok(res) if res.decision == celnet_limits::PreTradeDecision::Reject => {
                        Some(limit_breach_message(&res))
                    }
                    _ => None,
                }
            })
        } else {
            None
        };
        if let Some(reason) = &limit_reason {
            filled = false;
            text = Some(reason.as_bytes());
        }

        // A filled rates auto-quote / RFS-stream lift books the desk request it created as
        // a completed deal (dealt position + Deal + ACCEPTED) and closes any live RFS
        // stream for it — so the FIX taker's execution shows in the deal blotter and the
        // rates Book, exactly like a GUI desk accept. `book_fix_lift` runs the rates
        // pre-trade + per-risk-book limit gates AND the internalisation/hedge rules; if it
        // REJECTS (limit / risk-book breach) it books NOTHING and returns `None`. We MUST
        // reflect that on the wire: report REJECTED instead of a phantom FILLED with no deal
        // behind it. (The last-look token is already consumed, so a re-lift still rejects.)
        //
        // Incoming-quote-acceptance gate (the third trader-configurable rule engine —
        // `celnet-acceptance`): AFTER last-look validity passes (the ledger `Booked` above)
        // and BEFORE `book_fix_lift`, the acceptance graph gates the lift. `Accept` (the
        // seeded ACCEPT-ALL default, so a NO-OP until configured) books exactly as today;
        // `Reject` refuses the lift with its reason (EXEC_REJECTED, nothing booked); `Hold`
        // routes it to the desk inbox for manual accept (the request stays QUOTED so a human
        // accepts it later via `accept_desk_quote`) and does NOT close the RFS stream. The
        // last-look token was already consumed by `try_book`, so a re-lift re-rejects
        // (consistent with the existing last-look ordering) — a Hold relies on the GUI
        // desk-accept path, not a FIX re-lift.
        let mut acceptance_reason: Option<String> = None;
        if filled && let Some(request_id) = rates_request_id.as_ref() {
            if let Some(edge) = self.ctx.desk_edge.as_ref() {
                let quote_age_ms = quote_mint_nanos
                    .map(|mint| ((now.saturating_sub(mint)).max(0) as f64) / 1_000_000.0)
                    .unwrap_or(0.0);
                match edge.evaluate_fix_acceptance(request_id, quote_age_ms) {
                    AcceptanceDecision::Accept => {
                        // Book at the ACTUAL last-look fill price (`premium`, already the
                        // policy result for a market-data lift), so the booked position and
                        // deal carry the real dealt level, not the raw streamed quote.
                        if edge.book_fix_lift_priced(request_id, premium).is_none() {
                            filled = false;
                            const REJECT: &[u8] =
                                b"rates lift rejected: pre-trade / risk-book limit breach";
                            text = text.or(Some(REJECT));
                        }
                    }
                    AcceptanceDecision::Reject(reason) => {
                        // Refuse the lift — book NOTHING. Leave the RFS stream live so the
                        // counterparty can re-request (a fresh quote mints a new token).
                        filled = false;
                        acceptance_reason = Some(format!("acceptance rejected: {reason}"));
                    }
                    AcceptanceDecision::Hold(reason) => {
                        // Route to the desk inbox for MANUAL accept: the request stays QUOTED
                        // (never booked here), so a human accepts it via the GUI desk-accept
                        // path. Do NOT retire the quote / close the RFS stream on a Hold.
                        filled = false;
                        acceptance_reason = Some(format!("held for manual review: {reason}"));
                    }
                }
            }
            // Only a genuinely-booked (or no-edge test) lift closes the stream; a
            // rejected / held one leaves it live so the counterparty can re-request.
            if filled {
                self.md_subs
                    .retain(|_, sub| sub.request_id.as_deref() != Some(request_id.as_str()));
            }
        }
        if let Some(reason) = &acceptance_reason {
            text = Some(reason.as_bytes());
        }

        // A successful lift retires the quote (idempotency: a second lift of the same
        // QuoteID / symbol now rejects as already-consumed via the ledger). A rejected lift
        // is NOT retired here, but its token is already consumed, so a re-lift still rejects.
        if filled {
            if let Some(q) = quote_id.as_ref() {
                self.live.remove(q);
            }
            // The booked token is already consumed by `try_book`; retire the symbol's
            // OTHER-side token too so a booked line leaves no liftable residue.
            if via_md && let Some(md) = self.live_md.remove(&symbol) {
                if let Some(t) = md.buy_token {
                    self.ledger.retire(t);
                }
                if let Some(t) = md.sell_token {
                    self.ledger.retire(t);
                }
            }
        }

        // --- Order-lifecycle structured logging (async FIX edge; the pinned pricing
        //     core is never on this path — guardrail 11). Shared string views for the
        //     incoming-order (class=Order) and execution-report (class=Execution) events.
        let symbol_str = String::from_utf8_lossy(&symbol).into_owned();
        let cl_ord_str = String::from_utf8_lossy(&cl_ord_id).into_owned();
        let quote_id_str = quote_id
            .as_deref()
            .map(|q| String::from_utf8_lossy(q).into_owned());
        let counterparty = String::from_utf8_lossy(&self.ctx.counterparty).into_owned();
        let side_label = if side_byte == dialect_fx::SIDE_SELL {
            "sell"
        } else {
            "buy"
        };
        let reason_str = text.map(|t| String::from_utf8_lossy(t).into_owned());
        // INCOMING ORDER (class=Order): the inbound lift + last-look verdict + the
        // acceptance-rule decision (INFO when the lift is accepted, WARN on a
        // reject/hold/last-look-fail).
        if filled {
            tracing::info!(
                class = celnet_observability::LogClass::Order.label(),
                connection_id = %self.ctx.connection_id,
                counterparty = %counterparty,
                cl_ord_id = %cl_ord_str,
                quote_id = quote_id_str.as_deref(),
                symbol = %symbol_str,
                side = side_label,
                last_look,
                rates_request_id = rates_request_id.as_deref(),
                "FIX new-order lift accepted",
            );
        } else {
            tracing::warn!(
                class = celnet_observability::LogClass::Order.label(),
                connection_id = %self.ctx.connection_id,
                counterparty = %counterparty,
                cl_ord_id = %cl_ord_str,
                quote_id = quote_id_str.as_deref(),
                symbol = %symbol_str,
                side = side_label,
                last_look,
                rates_request_id = rates_request_id.as_deref(),
                reason = reason_str.as_deref(),
                "FIX new-order lift rejected/held",
            );
        }

        let order_id = self.mint_id("O");
        let exec_id = self.mint_id("E");
        let frame_out = self.session.send_app(st, |h, e| {
            let p = ExecReportParams {
                order_id: &order_id,
                exec_id: &exec_id,
                cl_ord_id: &cl_ord_id,
                exec_type: if filled { EXEC_FILLED } else { EXEC_REJECTED },
                ord_status: if filled { EXEC_FILLED } else { EXEC_REJECTED },
                symbol: &symbol,
                side: side_byte,
                last_qty: if filled { 1_000_000.0 } else { 0.0 },
                last_px: premium,
                multileg_type: None,
                text,
            };
            messages::build_execution_report(h, &p, e)
        });
        // EXECUTION (class=Execution): the ExecutionReport actually emitted on the wire
        // — the fill (INFO) or the reject (WARN), with the dealt price + reason.
        let order_id_str = String::from_utf8_lossy(&order_id).into_owned();
        let exec_id_str = String::from_utf8_lossy(&exec_id).into_owned();
        if filled {
            tracing::info!(
                class = celnet_observability::LogClass::Execution.label(),
                connection_id = %self.ctx.connection_id,
                counterparty = %counterparty,
                order_id = %order_id_str,
                exec_id = %exec_id_str,
                cl_ord_id = %cl_ord_str,
                symbol = %symbol_str,
                side = side_label,
                exec_type = "filled",
                last_qty = 1_000_000.0_f64,
                last_px = premium,
                rates_request_id = rates_request_id.as_deref(),
                "FIX execution report — filled",
            );
        } else {
            tracing::warn!(
                class = celnet_observability::LogClass::Execution.label(),
                connection_id = %self.ctx.connection_id,
                counterparty = %counterparty,
                order_id = %order_id_str,
                exec_id = %exec_id_str,
                cl_ord_id = %cl_ord_str,
                symbol = %symbol_str,
                side = side_label,
                exec_type = "rejected",
                last_qty = 0.0_f64,
                last_px = premium,
                reason = reason_str.as_deref(),
                "FIX execution report — rejected",
            );
        }
        out.push(frame_out);
    }

    /// Decode + convention-check the inbound option block and price it through the
    /// SHARED engine/surface path, returning the maker two-way (the SAME numbers the
    /// gRPC `QuoteService` would return for this instrument and market) plus, for an
    /// FX-vanilla line, the BUY-side pre-trade [`BookedPosition`] template (ADR-0016 A1)
    /// a lift will run the limit gate against. A rates line has no vanilla risk leaf and
    /// so carries `None`.
    async fn price_request(
        &self,
        frame: &FrameCursor<'_>,
    ) -> Result<(PricedLine, Option<BookedPosition>), ()> {
        // Dialect dispatch keyed on the connection's kind:
        //
        // * a fixed-income venue routes EVERY inbound `QuoteRequest(R)` to the shared
        //   rates dialect and enforces that the inbound `SubscriptionRequestType(263)`
        //   matches the venue's intent — a quote/RFQ venue serves a one-shot snapshot,
        //   a stream/RFS venue serves a subscribe/unsubscribe;
        // * an FX-options venue prices the FX block, and (as before connection kinds)
        //   still content-detects an OIS request by `SecurityType(167)` so the
        //   legacy/demo mixed acceptor keeps working.
        //
        // Either way the rate-valued two-way line flows through the SAME token / Quote
        // / lift / fill machinery as an FX option line.
        // The CURRENT live curve (operator's most recent `MarkCurve`, else static P0) so
        // this content-detect rates path also tracks live SOFR pillar edits (item C).
        let curve = self.live_rates_curve();
        if let Some(intent) = rates_intent_for_kind(self.ctx.kind) {
            // A rates line has no canonical-vanilla risk leaf ⇒ no pre-trade template.
            // The bond arm is selected by `SecurityType(167)=BOND`; every other rates
            // request is the OIS arm.
            if frame.get(167) == Some(dialect_rates::SEC_TYPE_BOND) {
                return bond_line(frame, Some(intent), &curve).map(|line| (line, None));
            }
            return rates_line(frame, Some(intent), &curve).map(|line| (line, None));
        }
        // The FX-options / legacy-demo acceptor still content-detects a fixed-income
        // request by `SecurityType(167)` (BOND before OIS), so a mixed acceptor prices
        // cash bonds and OIS alongside FX options.
        if frame.get(167) == Some(dialect_rates::SEC_TYPE_BOND) {
            return bond_line(frame, None, &curve).map(|line| (line, None));
        }
        if frame.get(167) == Some(dialect_rates::SEC_TYPE_OIS) {
            return rates_line(frame, None, &curve).map(|line| (line, None));
        }

        // The vol-time in years carried by the dialect (fully wire-specified, no date
        // dependency). Required for a deterministic, reproducible premium.
        let expiry_years = frame
            .get(dialect_fx::TAG_EXPIRY_YEARS)
            .and_then(dialect_fx::parse_float)
            .filter(|t| *t > 0.0 && t.is_finite())
            .ok_or(())?;

        // Resolve a tenor for the convention cross-check from the carried expiry: the
        // dialect's convention guard needs a tenor; we resolve the convention record
        // for the pair at the nearest standard tenor and price at the exact
        // `expiry_years`. (The premium is governed by `expiry_years`, not the tenor
        // label.)
        let tenor = tenor_for_years(expiry_years);
        let desc: OptionDescriptor = dialect_fx::decode_option(frame, tenor).map_err(|_| ())?;
        // American exercise is not a vanilla-GK leaf; the FIX edge prices European
        // vanillas (the dialect's `decode_option` already defaults to European).
        if desc.exercise != ExerciseStyle::European {
            return Err(());
        }

        let instrument = instrument_from_descriptor(&desc, expiry_years);
        let conv = convention_set_for(&desc, tenor);

        let market = self.ctx.live_market().await.map_err(|_| ())?;
        // Honour an optional pinned surface_version exactly as the gRPC path does.
        let surface_version = frame
            .get(TAG_SURFACE_VERSION)
            .and_then(celnet_fix::framing::parse_uint);
        let PinnedVol {
            market: effective_market,
            ..
        } = resolve_pinned_vol(
            &self.ctx.surface_book,
            surface_version,
            &instrument,
            &market,
        )
        .map_err(|_| ())?;

        let priced = price_instrument(&instrument, &effective_market, &conv).map_err(|_| ())?;
        let two_way = self.ctx.spread.two_way(priced.greeks.price, &priced.greeks);

        // ADR-0016 A1: the BUY-side pre-trade template a lift will limit-gate. The FIX
        // venue books a fixed 1mm base (the `last_qty` an `ExecutionReport` fill carries),
        // marked at the resolved strike + vol of the priced market — the SAME risk leaf
        // the RFS click-to-trade sink records. A `Side=SELL` lift negates the notional.
        let inputs = celnet_types::VanillaInputs::new(
            effective_market.spot,
            priced.resolved_strike,
            priced.vol,
            expiry_years,
            effective_market.r_dom(),
            effective_market.r_for(),
        );
        let template = BookedPosition {
            position_id: 0,
            pair: desc.pair,
            option: desc.option_type,
            notional_base: 1_000_000.0,
            inputs,
            quoted_delta: conv.delta,
            premium_style: conv.premium,
            surface_version: surface_version.unwrap_or(0),
        };

        Ok((
            PricedLine {
                bid: two_way.bid,
                offer: two_way.offer,
                size: 1_000_000.0,
            },
            Some(template),
        ))
    }
}

/// The custom dialect tag carrying an optional pinned `surface_version` (a marked
/// surface selector), so a FIX RFQ can reproduce a desk-marked surface to the bit —
/// the same pin the gRPC/RFS/scenario paths carry on the wire.
pub const TAG_SURFACE_VERSION: u32 = 7002;

/// The maker two-way for one quoted FIX line.
struct PricedLine {
    bid: f64,
    offer: f64,
    size: f64,
}

/// The P0 maker half-spread for a fixed-income two-way rate market — single-homed
/// in [`crate::rates_pricing::RATES_RFQ_HALF_SPREAD`], shared with the WS/gRPC
/// taker RFQ two-way so the FIX and contract FI RFQ markets are struck identically.
use crate::rates_pricing::RATES_RFQ_HALF_SPREAD as RATES_HALF_SPREAD;

/// The RFS streaming re-price cadence: a fixed-income STREAM venue pushes a fresh two-way
/// on this interval for a lively demo, well within the 30s session heartbeat window.
const RFS_STREAM_INTERVAL_SECS: u64 = 5;
const RFS_STREAM_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(RFS_STREAM_INTERVAL_SECS);

/// The verdict of the configurable market-data (ESP) last-look policy for one lift.
#[derive(Debug, Clone, Copy, PartialEq)]
enum MdLastLook {
    /// Book the lift at this **fill price** — the client's requested price `q`, possibly
    /// improved toward the current published price under an Async giveback.
    Fill(f64),
    /// The published price moved AGAINST the dealer beyond the tolerance band: reject the
    /// lift as superseded. The caller preserves the live token so a fresh re-lift at the
    /// current price can still book.
    Superseded,
}

/// Resolve the configurable market-data last-look for a single lift.
///
/// `q` is the client's requested (lifted) price carried in `Price(44)`; `c` is the current
/// published price on the SAME side at order arrival. `is_sell` is `true` when the client
/// **sells into our bid** (the dealer BUYS), `false` when the client **buys our offer** (the
/// dealer SELLS). The relative `tolerance_bps` is the adverse-move band (bps of the current
/// price); `giveback_pct ∈ [0, 100]` is the Async share of the favorable move returned to
/// the client.
///
/// Sign convention — the dealer's signed benefit of the current price over the requested
/// price (`> 0` favorable to the dealer, `< 0` adverse):
/// * client sells into our bid (dealer buys, wants a LOWER price): favorable when the bid
///   ROSE, i.e. `c > q` ⇒ `gain = c - q`;
/// * client buys our offer (dealer sells, wants a HIGHER price): favorable when the offer
///   FELL, i.e. `c < q` ⇒ `gain = q - c`.
///
/// * **Adverse beyond tolerance** (`-gain > band`) ⇒ [`MdLastLook::Superseded`].
/// * **Adverse within tolerance** ⇒ fill at exactly `q` (the dealer honors, absorbing the
///   small adverse move).
/// * **Favorable / neutral** ⇒ Sync fills at `q` (the dealer keeps the whole gain); Async
///   fills at `q + f·(c − q)` with `f = giveback_pct/100` (that share of the favorable move
///   passed back to the client, the dealer keeps the rest). Both sides collapse to the one
///   formula because `c − q` carries the correct improvement sign per side.
fn md_last_look_decision(
    is_sell: bool,
    q: f64,
    c: f64,
    mode: LastLookMode,
    tolerance_bps: f64,
    giveback_pct: f64,
) -> MdLastLook {
    let dealer_gain = if is_sell { c - q } else { q - c };
    if dealer_gain < 0.0 {
        // Adverse: reject only beyond the relative band (bps of the current published
        // price). Within the band the dealer honors the client's requested price `q`.
        let band = tolerance_bps * 1e-4 * c.abs();
        if -dealer_gain > band {
            return MdLastLook::Superseded;
        }
        return MdLastLook::Fill(q);
    }
    // Favorable (or exactly neutral): Sync keeps the whole move; Async returns a share.
    match mode {
        LastLookMode::Sync => MdLastLook::Fill(q),
        LastLookMode::Async => {
            let f = (giveback_pct / 100.0).clamp(0.0, 1.0);
            MdLastLook::Fill(q + f * (c - q))
        }
    }
}

/// Price a retained [`RfsLine`] off `curve` to the two-way its arm centres a market on —
/// an OIS around its par rate ([`crate::rates_pricing::par_rate_for`], split
/// [`RATES_HALF_SPREAD`]) or a cash bond around its clean price
/// ([`crate::rates_pricing::quote_bond`], split [`BOND_HALF_SPREAD`]) — with `notional`
/// as the quote size. The RFS re-price fallback: because `curve` is the CURRENT live
/// curve, a re-mark moves the stream, and an unpriceable line (`Err`) yields `None` so the
/// ticker skips it rather than emitting a bad price. Mirrors [`rates_line`] / [`bond_line`]
/// exactly (the same engine bodies), differing only in that the inputs are retained on the
/// subscription rather than re-decoded from a frame.
fn curve_line_for(line: &RfsLine, curve: &CurveSet, notional: f64) -> Option<PricedLine> {
    match line {
        RfsLine::Ois { tenor_years } => {
            let par = crate::rates_pricing::par_rate_for(curve, *tenor_years).ok()?;
            let (bid, offer) = dialect_rates::two_way_rates(par, RATES_HALF_SPREAD);
            Some(PricedLine {
                bid,
                offer,
                size: notional,
            })
        }
        RfsLine::Bond { instrument } => {
            let quote = crate::rates_pricing::quote_bond(instrument, curve).ok()?;
            let (bid, offer) = dialect_rates::two_way_rates(quote.clean_price, BOND_HALF_SPREAD);
            Some(PricedLine {
                bid,
                offer,
                size: notional,
            })
        }
    }
}

/// The [`PricingSourceMode::CurveAnchoredBookSkew`] blend: the internal `curve` two-way is
/// the backbone, its mid pulled a fraction `w ∈ [0, 1]` of the way toward the composite
/// mid, and the half-spread taken from the (real-liquidity) composite. Precisely:
///
/// ```text
///   curve_mid = 0.5·(curve.bid + curve.offer)
///   comp_mid  = 0.5·(comp.bid  + comp.offer)
///   skew_mid  = curve_mid + w·(comp_mid − curve_mid)   // convex blend, bounded by the gap
///   half      = 0.5·(comp.offer − comp.bid)            // composite spread
///   bid,offer = skew_mid ∓ half
/// ```
///
/// `w` is clamped to `[0, 1]` (also validated at the admin write), so `w = 0` is the pure
/// curve mid, `w = 1` the pure composite mid, and the move can never overshoot the
/// curve→composite gap. The result is non-crossed because the composite is non-crossed
/// (`half ≥ 0`, guaranteed by the aggregation degenerate-composite guard).
fn blend_curve_toward_composite(curve: &PricedLine, comp: &PricedLine, w: f64) -> PricedLine {
    let w = w.clamp(0.0, 1.0);
    let curve_mid = 0.5 * (curve.bid + curve.offer);
    let comp_mid = 0.5 * (comp.bid + comp.offer);
    let skew_mid = curve_mid + w * (comp_mid - curve_mid);
    let half = 0.5 * (comp.offer - comp.bid);
    PricedLine {
        bid: skew_mid - half,
        offer: skew_mid + half,
        size: curve.size,
    }
}

/// The rates request intent a fixed-income acceptor serves: a one-shot RFQ vs a
/// streaming RFS. Derived from the connection's [`AcceptorKind`] and matched
/// against the inbound `SubscriptionRequestType(263)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RatesIntent {
    /// A one-shot quote venue: `SubscriptionRequestType(263)=0` (snapshot).
    Rfq,
    /// A streaming venue: `SubscriptionRequestType(263)=1`/`2` (subscribe/unsubscribe).
    Rfs,
}

/// The rates intent a fixed-income acceptor kind serves, or `None` for the
/// FX-options kind (which is not a rates venue).
fn rates_intent_for_kind(kind: AcceptorKind) -> Option<RatesIntent> {
    match kind {
        AcceptorKind::Options => None,
        AcceptorKind::FixedIncomeQuote => Some(RatesIntent::Rfq),
        AcceptorKind::FixedIncomeStream => Some(RatesIntent::Rfs),
    }
}

/// Whether an inbound `SubscriptionRequestType(263)` matches the venue's intent: a
/// quote/RFQ venue admits only a snapshot; a stream/RFS venue admits a
/// subscribe/unsubscribe. A mismatch (e.g. a streaming subscribe sent to a
/// one-shot quote venue) is refused upstream as an unquotable line.
fn subscription_matches_intent(
    intent: RatesIntent,
    subscription: dialect_rates::SubscriptionRequest,
) -> bool {
    use dialect_rates::SubscriptionRequest::{Snapshot, Subscribe, Unsubscribe};
    match intent {
        RatesIntent::Rfq => matches!(subscription, Snapshot),
        RatesIntent::Rfs => matches!(subscription, Subscribe | Unsubscribe),
    }
}

/// Price an inbound OIS RFQ to a two-way **rate** line: the par rate of the
/// requested tenor on the P0 static USD-SOFR curve, split [`RATES_HALF_SPREAD`]
/// either side, with the RFQ notional as the quote size. The rate-valued
/// `bid`/`offer` reuse [`PricedLine`] unchanged, so the inbound RFQ is quoted,
/// token-minted, lifted and filled exactly like an FX line — that reuse is what
/// makes "the FIX API supports fixed income" true end-to-end without a parallel
/// quote/order path.
///
/// `expected` is the venue's rates intent when this is a dedicated fixed-income
/// connection (`Some`), or `None` for the legacy/demo FX-options acceptor's
/// content-detected OIS path (which imposes no `263` constraint). When set, an
/// inbound subscription type that does not match the intent is refused.
/// The validity a venue auto-quote is recorded with in the desk history (ms).
const RATES_AUTO_QUOTE_VALID_MS: u32 = 30_000;

/// Map a dialect [`RatesSide`] to the canonical proto [`Side`]: pay-fixed is a
/// (fixed-rate) buy, receive-fixed a sell; a two-way request carries no firm side.
fn rates_side_to_side(side: dialect_rates::RatesSide) -> Side {
    match side {
        dialect_rates::RatesSide::PayFixed => Side::Buy,
        dialect_rates::RatesSide::ReceiveFixed => Side::Sell,
        dialect_rates::RatesSide::TwoWay => Side::TwoWay,
    }
}

/// Price an inbound OIS RFQ to a two-way **rate** line off `curve` (the caller passes the
/// current live curve, so editing SOFR pillars moves the quote). Split
/// [`RATES_HALF_SPREAD`] either side of the par rate, with the RFQ notional as the size.
fn rates_line(
    frame: &FrameCursor<'_>,
    expected: Option<RatesIntent>,
    curve: &CurveSet,
) -> Result<PricedLine, ()> {
    let rfq = dialect_rates::decode_rates_rfq(frame).map_err(|_| ())?;
    if let Some(intent) = expected
        && !subscription_matches_intent(intent, rfq.subscription)
    {
        return Err(());
    }
    let par = crate::rates_pricing::par_rate_for(curve, rfq.tenor_years).map_err(|_| ())?;
    let (bid, offer) = dialect_rates::two_way_rates(par, RATES_HALF_SPREAD);
    Ok(PricedLine {
        bid,
        offer,
        size: rfq.notional,
    })
}

/// The P0 maker half-spread for a fixed-income two-way **bond price** market —
/// single-homed in [`crate::rates_pricing::BOND_RFQ_HALF_SPREAD`], shared with the
/// WS/gRPC taker bond RFQ two-way.
use crate::rates_pricing::BOND_RFQ_HALF_SPREAD as BOND_HALF_SPREAD;

/// Price an inbound cash-bond RFQ to a two-way **clean-price** line: the clean (quoted)
/// price of the bond discounted off the P0 static USD-SOFR curve — the LANDED
/// `price_rates`(Bond) / `celnet_bond` result (via [`crate::rates_pricing::quote_bond`])
/// — split [`BOND_HALF_SPREAD`] either side, with the RFQ notional as the quote size.
/// A bond's clean price and DV01 are side-independent magnitudes, so the two-way market
/// is struck around the magnitude exactly as [`rates_line`] centres an OIS market on the
/// side-independent par rate. The rate-valued `PricedLine` is reused unchanged, so the
/// inbound bond RFQ is quoted, token-minted, lifted and filled exactly like an OIS or FX
/// line — that reuse is what makes cash bonds first-class on the FIX edge with no
/// bond-specific quote/order path.
///
/// `expected` is the venue's rates intent when this is a dedicated fixed-income
/// connection (`Some`), or `None` for the content-detected path (which imposes no `263`
/// constraint). When set, an inbound subscription type that does not match the intent is
/// refused.
fn bond_line(
    frame: &FrameCursor<'_>,
    expected: Option<RatesIntent>,
    curve: &CurveSet,
) -> Result<PricedLine, ()> {
    let rfq = dialect_rates::decode_bond_rfq(frame).map_err(|_| ())?;
    if let Some(intent) = expected
        && !subscription_matches_intent(intent, rfq.subscription)
    {
        return Err(());
    }
    let quote = crate::rates_pricing::quote_bond(&rfq.instrument, curve).map_err(|_| ())?;
    let (bid, offer) = dialect_rates::two_way_rates(quote.clean_price, BOND_HALF_SPREAD);
    Ok(PricedLine {
        bid,
        offer,
        size: rfq.notional,
    })
}

/// Build the canonical [`Instrument`] from a decoded dialect descriptor and the
/// carried vol-time. A FX vanilla call/put at an absolute strike, base-notional 1mm.
fn instrument_from_descriptor(desc: &OptionDescriptor, expiry_years: f64) -> Instrument {
    Instrument {
        underlying: Some(celnet_proto::Underlying::fx(CcyPair {
            base: desc.pair.base.as_str().to_owned(),
            quote: desc.pair.quote.as_str().to_owned(),
        })),
        tenor: None,
        expiry_years,
        quantity: Some(Quantity {
            notional: 1_000_000.0,
            base_ccy: true,
        }),
        side: Side::TwoWay as i32,
        solve: None,
        pricing_model: celnet_proto::PricingModel::Default as i32,
        product: Some(instrument::Product::Vanilla(Vanilla {
            option_type: match desc.option_type {
                OptionType::Call => celnet_proto::OptionType::Call as i32,
                OptionType::Put => celnet_proto::OptionType::Put as i32,
            },
            strike: Some(StrikeOrDelta {
                spec: Some(strike_or_delta::Spec::Strike(desc.strike)),
            }),
        })),
        ..Default::default()
    }
}

/// Resolve the [`ConventionSet`] for a `(pair, tenor)` from the registry record — the
/// FIX dialect carries no explicit conventions, so the venue applies the canonical
/// resolved convention for the pair/tenor (the same record the rest of the platform
/// uses). The settlement style is taken from the dialect descriptor (FXVO/FXNO),
/// which `decode_option` already cross-checked against the resolved convention.
fn convention_set_for(desc: &OptionDescriptor, tenor: Tenor) -> ConventionSet {
    let record = celnet_conventions::resolve(desc.pair, tenor).record;
    ConventionSet {
        delta: record.delta,
        atm: record.atm,
        premium: record.premium_style,
        cut: record.cut,
        day_count: record.day_count_vol,
        settlement: record.settlement,
    }
}

/// Resolve a standard [`Tenor`] label nearest the carried vol-time in years, used only
/// for the convention-record lookup (the premium is governed by `expiry_years`, not
/// the label). Common round tenors map exactly; anything else picks the nearest
/// whole-month/year bucket.
fn tenor_for_years(years: f64) -> Tenor {
    // Nearest whole month, clamped to a sane range; whole years use `Years`.
    let months = (years * 12.0).round() as i64;
    if months <= 0 {
        Tenor::Weeks(1)
    } else if months % 12 == 0 {
        Tenor::Years((months / 12) as u16)
    } else {
        Tenor::Months(months as u16)
    }
}

/// Format a nanosecond UTC instant as a FIX `SendingTime` / `ValidUntilTime` timestamp
/// (`YYYYMMDD-HH:MM:SS.sss`). Deterministic, allocation-light, and never on the
/// pricing path (a message timestamp, not a priced number).
fn utc_timestamp(nanos: i64) -> Vec<u8> {
    // Convert epoch-nanos to civil date-time via `time` (a workspace dep of the
    // calendar). Fall back to the epoch on an out-of-range instant (never panics).
    let secs = nanos.div_euclid(1_000_000_000);
    let millis = (nanos.rem_euclid(1_000_000_000) / 1_000_000) as u32;
    let dt =
        time::OffsetDateTime::from_unix_timestamp(secs).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let date = dt.date();
    let t = dt.time();
    format!(
        "{:04}{:02}{:02}-{:02}:{:02}:{:02}.{:03}",
        date.year(),
        u8::from(date.month()),
        date.day(),
        t.hour(),
        t.minute(),
        t.second(),
        millis,
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenor_for_years_maps_round_tenors() {
        assert_eq!(tenor_for_years(1.0), Tenor::Years(1));
        assert_eq!(tenor_for_years(2.0), Tenor::Years(2));
        assert_eq!(tenor_for_years(0.25), Tenor::Months(3));
        assert_eq!(tenor_for_years(0.5), Tenor::Months(6));
    }

    #[test]
    fn utc_timestamp_is_fix_shaped() {
        // 2026-06-05T00:00:00Z = 1_780_963_200 s.
        let ts = utc_timestamp(1_780_963_200_000_000_000);
        let s = String::from_utf8(ts).unwrap();
        assert_eq!(s.len(), "YYYYMMDD-HH:MM:SS.sss".len());
        assert_eq!(&s[8..9], "-");
        assert!(s.starts_with("2026"));
    }

    /// A hub with one flat book covering `BND-5Y` (raw composite 99.50/99.60 ⇒ mid 99.55)
    /// and one enabled pricing group `grp-fix` whose member connection is `conn-grouped`,
    /// pricing its RFS/RFQ outbound through a Flat ±50 price-bps tier. The SAME construction
    /// pattern the `aggregation`/`quote` tests use, kept local so this test owns its fixture.
    fn grouped_stream_hub() -> Arc<crate::services::aggregation::AggregationHub> {
        use crate::config::identity::{
            AggregatedBookDef, AggregationParams, IdentityStore, PricingGroupDef, Scope,
        };
        use celnet_tiering::{
            FeaturePipeline, Guardrails, PricingFeature, SpreadUnit, StalePolicy, StrategySpec,
            TieringConfig,
        };
        const HUB_NOW: i64 = 1_700_000_000_000_000_000;
        let flat_50 = FeaturePipeline::new(
            vec![PricingFeature::Tiering {
                config: TieringConfig {
                    unit: SpreadUnit::PriceBps,
                    strategies: vec![StrategySpec::FlatMarkup { half_spread: 50.0 }],
                    guardrails: Guardrails::new(0.0, 1_000.0, 1_000.0, 1e-9),
                    stale_policy: StalePolicy::Suppress,
                },
            }],
            Guardrails::new(0.0, 1_000.0, 1_000.0, 1e-9),
        );
        let hub =
            crate::services::aggregation::AggregationHub::new(crate::clock::Clock::manual(HUB_NOW));
        let mut store = IdentityStore::default();
        store.aggregated_books.push(AggregatedBookDef {
            id: "agg-book".to_string(),
            name: "agg-book".to_string(),
            member_connection_ids: vec!["LP-1".to_string()],
            instrument_scope: Scope::Explicit(vec!["BND-5Y".to_string()]),
            params: AggregationParams {
                staleness_tau_ms: 30_000,
                max_quote_age_ms: 86_400_000,
                divergence_gating: false,
                min_contributors: 1,
                depth_levels: 1,
            },
            enabled: true,
        });
        store.pricing_groups.push(PricingGroupDef {
            id: "grp-fix".to_string(),
            name: "GRP-FIX".to_string(),
            description: String::new(),
            member_connection_ids: vec!["conn-grouped".to_string()],
            member_user_ids: vec![],
            member_desks: vec![],
            esp_pipeline: flat_50.clone(),
            rfq_pipeline: flat_50,
            share_pipeline: false,
            pricing_source_mode: PricingSourceMode::default(),
            book_skew_weight: DEFAULT_BOOK_SKEW_WEIGHT,
            last_look_mode: LastLookMode::default(),
            last_look_tolerance_bps: DEFAULT_LAST_LOOK_TOLERANCE_BPS,
            async_giveback_pct: DEFAULT_ASYNC_GIVEBACK_PCT,
            enabled: true,
        });
        hub.reconcile(&store);
        assert!(
            hub.ingest(&celnet_proto::LpQuote {
                lp_name: "LP-1".to_string(),
                instrument_id: "BND-5Y".to_string(),
                bid: 99.50,
                offer: 99.60,
                bid_size: 1_000_000.0,
                offer_size: 2_000_000.0,
                ts_nanos: HUB_NOW,
            }),
            "LP-1 ingested into the book"
        );
        hub
    }

    /// A hub whose pricing group binds connection `conn-async` to a market-data last-look
    /// policy of [`LastLookMode::Async`] with a 50% giveback — so a favorable lift on that
    /// session is improved half-way toward the current price. No aggregated book / pipeline
    /// is needed: the last-look test publishes an explicit snapshot via `emit_md_snapshot`.
    fn async_lastlook_hub() -> Arc<crate::services::aggregation::AggregationHub> {
        use crate::config::identity::{IdentityStore, PricingGroupDef};
        use celnet_tiering::{FeaturePipeline, Guardrails};
        const HUB_NOW: i64 = 1_700_000_000_000_000_000;
        let empty = FeaturePipeline::new(vec![], Guardrails::new(0.0, 1_000.0, 1_000.0, 1e-9));
        let hub =
            crate::services::aggregation::AggregationHub::new(crate::clock::Clock::manual(HUB_NOW));
        let mut store = IdentityStore::default();
        store.pricing_groups.push(PricingGroupDef {
            id: "grp-async".to_string(),
            name: "GRP-ASYNC".to_string(),
            description: String::new(),
            member_connection_ids: vec!["conn-async".to_string()],
            member_user_ids: vec![],
            member_desks: vec![],
            esp_pipeline: empty.clone(),
            rfq_pipeline: empty,
            share_pipeline: false,
            pricing_source_mode: PricingSourceMode::default(),
            book_skew_weight: DEFAULT_BOOK_SKEW_WEIGHT,
            last_look_mode: LastLookMode::Async,
            last_look_tolerance_bps: DEFAULT_LAST_LOOK_TOLERANCE_BPS,
            async_giveback_pct: 50.0,
            enabled: true,
        });
        hub.reconcile(&store);
        hub
    }

    /// Build a fixed-income STREAM session on `connection_id` wired to `hub` (or none) —
    /// the minimal context [`FixSession::composite_two_way`] reads (hub + connection id +
    /// desk); the engine collaborators are inert for a composite lookup.
    fn stream_session(
        connection_id: &str,
        hub: Option<Arc<crate::services::aggregation::AggregationHub>>,
    ) -> FixSession {
        stream_session_with_control(connection_id, hub, None)
    }

    /// Build a fixed-income STREAM session, optionally sharing an explicit firm-wide
    /// pricing kill-switch `control` so the outbound-gate tests can flip it (absent ⇒
    /// the default both-enabled control).
    fn stream_session_with_control(
        connection_id: &str,
        hub: Option<Arc<crate::services::aggregation::AggregationHub>>,
        control: Option<Arc<crate::services::pricing_control::PricingControl>>,
    ) -> FixSession {
        let link = {
            let initial = celnet_engine::testing::make_state(
                1.10,
                celnet_conventions::resolve(
                    celnet_types::CcyPair::parse("EURUSD").unwrap(),
                    celnet_types::Tenor::Years(1),
                )
                .record,
            );
            CoreLink::start(initial, None)
        };
        let ctx = FixContext::with_comp_ids(
            link,
            SpreadModel::default(),
            Clock::system(),
            Arc::new(SurfaceBook::new()),
            b"CELNET".to_vec(),
            b"CPTY".to_vec(),
            Arc::new(FixMonitor::new()),
            connection_id.to_string(),
            AcceptorKind::FixedIncomeStream,
            Arc::new(PositionStore::new()),
        )
        .with_aggregation(hub)
        .with_pricing_control(control);
        let cfg = SessionConfig {
            sender: ctx.sender.clone(),
            target: ctx.counterparty.clone(),
            heart_bt_int: 30,
            role: Role::Acceptor,
        };
        FixSession::new(cfg, ctx)
    }

    /// The composite-based + tiered outbound seam (design §5): a fixed-income STREAM
    /// session prices a streamed instrument OFF the aggregated-book composite, and a
    /// GROUPED connection's two-way is its group's tier applied to the RAW composite —
    /// DIFFERENT from the raw two-way an ungrouped connection receives. Raw mid 99.55,
    /// Flat ±50 price-bps ⇒ grouped 99.05/100.05; ungrouped = raw 99.50/99.60.
    #[tokio::test]
    async fn stream_prices_off_composite_and_tiers_per_pricing_group() {
        let hub = grouped_stream_hub();

        // Grouped connection ⇒ the group's Flat ±50 tier off the RAW mid 99.55.
        let grouped = stream_session("conn-grouped", Some(Arc::clone(&hub)));
        let g = grouped
            .composite_two_way(b"BND-5Y", 5_000_000.0)
            .expect("a covered instrument prices off the composite");
        assert!((g.bid - 99.05).abs() < 1e-9, "grouped bid={}", g.bid);
        assert!((g.offer - 100.05).abs() < 1e-9, "grouped offer={}", g.offer);
        assert!((g.size - 5_000_000.0).abs() < 1e-9);

        // Ungrouped connection ⇒ the RAW composite verbatim (tiering is group-only).
        let plain = stream_session("conn-plain", Some(Arc::clone(&hub)));
        let p = plain
            .composite_two_way(b"BND-5Y", 5_000_000.0)
            .expect("a covered instrument prices off the composite");
        assert!((p.bid - 99.50).abs() < 1e-9, "ungrouped bid={}", p.bid);
        assert!(
            (p.offer - 99.60).abs() < 1e-9,
            "ungrouped offer={}",
            p.offer
        );

        // The grouped two-way genuinely DIFFERS from the raw (the seam actually tiers).
        assert!(
            (g.bid - p.bid).abs() > 1e-6 && (g.offer - p.offer).abs() > 1e-6,
            "grouped tier must differ from the raw composite"
        );

        // An instrument NO book covers falls back (None) — the ticker keeps the P0 demo
        // re-price for it (e.g. a plain OIS curve), so nothing regresses.
        assert!(
            plain.composite_two_way(b"USD-OIS", 5_000_000.0).is_none(),
            "an uncovered instrument yields no composite (standalone fallback)"
        );

        // No hub wired ⇒ always None (the legacy env seed / tests stay byte-identical).
        let no_hub = stream_session("conn-grouped", None);
        assert!(no_hub.composite_two_way(b"BND-5Y", 5_000_000.0).is_none());
    }

    /// A small, auto-quotable OIS Subscribe RFQ (within the clip cap + on-the-run) for
    /// the outbound-kill-switch test — 10 mm at 5 y, which `classify_ois_rfq` admits.
    fn small_ois_subscribe_frame() -> Vec<u8> {
        let hdr = Header {
            sender: b"CELNET",
            target: b"CELNET-CPTY",
            seq_num: 7,
            sending_time: b"20260625-12:00:00.000",
        };
        let p = RatesQuoteRequestParams {
            quote_req_id: b"RFQ-KS",
            symbol: b"USD-OIS",
            tenor_years: 5,
            notional: 10_000_000.0,
            side: RatesSide::TwoWay,
            subscription: SubscriptionRequest::Subscribe,
        };
        let mut enc = FrameEncoder::new();
        build_rates_quote_request(&hdr, &p, &mut enc)
    }

    /// A small, auto-quotable OIS `MarketDataRequest(V)` subscribe (within the clip cap +
    /// on-the-run) for the market-data stream tests — 10 mm at 5 y, which the venue admits.
    fn small_ois_md_subscribe_frame() -> Vec<u8> {
        let hdr = Header {
            sender: b"CELNET",
            target: b"CELNET-CPTY",
            seq_num: 7,
            sending_time: b"20260625-12:00:00.000",
        };
        let p = dialect_rates::OisMarketDataRequestParams {
            md_req_id: b"MDR-KS",
            symbol: b"USD-OIS",
            tenor_years: 5,
            notional: 10_000_000.0,
            side: RatesSide::TwoWay,
            subscription: SubscriptionRequest::Subscribe,
        };
        let mut enc = FrameEncoder::new();
        dialect_rates::build_ois_market_data_request(&hdr, &p, &mut enc)
    }

    /// The firm-wide OUTBOUND kill-switch (test i + the stream-resume behaviour): with
    /// outbound pricing halted, an inbound `MarketDataRequest(V)` subscribe produces NO
    /// outbound `MarketDataSnapshotFullRefresh(W)` frame, yet the market-data subscription
    /// is still registered so it resumes from live on re-enable. The desk-inbox recording
    /// (None here — no desk wired) is untouched.
    #[tokio::test]
    async fn outbound_kill_switch_suppresses_snapshot_but_keeps_stream() {
        use crate::services::pricing_control::PricingControl;
        let control = PricingControl::new(true, true);
        let mut session = stream_session_with_control("conn-ks", None, Some(Arc::clone(&control)));
        let st = session.sending_time();
        let frame_bytes = small_ois_md_subscribe_frame();
        let frame = FrameCursor::parse(&frame_bytes).expect("frame parses");

        // Outbound HALTED: no outbound snapshot frame, but the market-data subscription
        // registers so it can resume from live later (no live token while halted).
        control.set(false, true);
        let mut out = Vec::new();
        session.on_market_data_request(&frame, &st, &mut out).await;
        assert!(out.is_empty(), "outbound halted: no snapshot frame pushed");
        assert_eq!(
            session.md_subs.len(),
            1,
            "the market-data subscription is still registered"
        );
        assert!(session.live_md.is_empty(), "no live token while halted");
        // A tick while halted pushes nothing.
        assert!(
            session.tick_md_stream(&st).is_empty(),
            "the ticker pauses while outbound is halted"
        );

        // Re-enable: the ticker now streams a fresh snapshot from live (resume, no replay).
        control.set(true, true);
        let frames = session.tick_md_stream(&st);
        assert!(!frames.is_empty(), "streams resume from live on re-enable");
        assert_eq!(session.live_md.len(), 1, "a live token is now published");
    }

    /// Build a stable OIS `MarketDataRequest(V)` with the given `md_req_id` + subscription
    /// type (for the subscribe/unsubscribe round-trip test).
    fn ois_md_request(md_req_id: &[u8], subscription: SubscriptionRequest) -> Vec<u8> {
        let hdr = Header {
            sender: b"CELNET",
            target: b"CELNET-CPTY",
            seq_num: 9,
            sending_time: b"20260625-12:00:01.000",
        };
        let p = dialect_rates::OisMarketDataRequestParams {
            md_req_id,
            symbol: b"USD-OIS",
            tenor_years: 5,
            notional: 10_000_000.0,
            side: RatesSide::TwoWay,
            subscription,
        };
        let mut enc = FrameEncoder::new();
        dialect_rates::build_ois_market_data_request(&hdr, &p, &mut enc)
    }

    /// The 35=V → 35=W round-trip: an inbound `MarketDataRequest(V)` subscribe registers the
    /// subscription, publishes an INITIAL `MarketDataSnapshotFullRefresh(W)` two-way, the
    /// streaming ticker pushes fresh snapshots, and a `263=2` unsubscribe tears it all down.
    #[tokio::test]
    async fn md_subscribe_streams_snapshots_and_unsubscribe_tears_down() {
        let mut session = stream_session("conn-md", None);
        let st = session.sending_time();

        let sub = ois_md_request(b"MDR-1", SubscriptionRequest::Subscribe);
        let sframe = FrameCursor::parse(&sub).expect("V frame parses");
        let mut out = Vec::new();
        session.on_market_data_request(&sframe, &st, &mut out).await;

        assert_eq!(session.md_subs.len(), 1, "subscription registered");
        assert_eq!(session.live_md.len(), 1, "a liftable token published");
        assert_eq!(out.len(), 1, "an initial snapshot is pushed");

        let snap = FrameCursor::parse(&out[0]).expect("W frame parses");
        assert_eq!(
            snap.msg_type(),
            MsgType::MarketDataSnapshotFullRefresh.as_bytes()
        );
        let view = messages::MarketDataSnapshotView::new(snap);
        assert_eq!(view.symbol(), Some(&b"USD-OIS"[..]));
        assert_eq!(view.md_req_id(), Some(&b"MDR-1"[..]));
        let tob = view.top_of_book();
        assert!(
            tob.bid_px.unwrap() <= tob.offer_px.unwrap(),
            "a two-way top-of-book (bid <= offer)"
        );

        // The ticker re-prices and pushes a fresh snapshot for the live subscription.
        let frames = session.tick_md_stream(&st);
        assert_eq!(frames.len(), 1, "one snapshot per live subscription");
        let tick = FrameCursor::parse(&frames[0]).unwrap();
        assert_eq!(
            tick.msg_type(),
            MsgType::MarketDataSnapshotFullRefresh.as_bytes()
        );

        // An unsubscribe tears the stream down (no more snapshots, token dropped).
        let unsub = ois_md_request(b"MDR-1", SubscriptionRequest::Unsubscribe);
        let uframe = FrameCursor::parse(&unsub).unwrap();
        let mut out2 = Vec::new();
        session
            .on_market_data_request(&uframe, &st, &mut out2)
            .await;
        assert!(
            session.md_subs.is_empty(),
            "unsubscribe tears the stream down"
        );
        assert!(session.live_md.is_empty(), "and drops the live token");
        assert!(
            session.tick_md_stream(&st).is_empty(),
            "nothing left to stream"
        );
    }

    /// The BUY (offer) top-of-book price the most recent snapshot in `frames` advertised —
    /// so a lift can present the EXACT streamed `Price(44)` and clear the price last-look.
    fn snapshot_offer(frames: &[Vec<u8>]) -> f64 {
        let snap = FrameCursor::parse(frames.last().expect("a snapshot was pushed")).unwrap();
        messages::MarketDataSnapshotView::new(snap)
            .top_of_book()
            .offer_px
            .expect("the snapshot carried an offer")
    }

    /// Build a by-symbol BUY `NewOrderSingle(D)` lifting `symbol` at `price` (the streamed
    /// offer) — the market-data lift the ESP sends (no `QuoteID(117)`).
    fn md_buy_order(cl_ord_id: &[u8], symbol: &[u8], price: f64) -> Vec<u8> {
        let hdr = Header {
            sender: b"CELNET",
            target: b"CELNET-CPTY",
            seq_num: 11,
            sending_time: b"20260625-12:00:02.000",
        };
        let p = messages::MarketOrderParams {
            cl_ord_id,
            symbol,
            side: dialect_fx::SIDE_BUY,
            qty: 1_000_000.0,
            price,
            transact_time: b"20260625-12:00:02.000",
        };
        let mut enc = FrameEncoder::new();
        messages::build_new_order_by_symbol(&hdr, &p, &mut enc)
    }

    fn exec_type_of(frames: &[Vec<u8>]) -> Option<u8> {
        let exec = FrameCursor::parse(frames.first().expect("an ExecutionReport")).unwrap();
        assert_eq!(exec.msg_type(), MsgType::ExecutionReport.as_bytes());
        messages::ExecReportView::new(exec).exec_type()
    }

    /// The 35=D → 35=8 book: after a subscribe publishes a liftable top-of-book, an inbound
    /// `NewOrderSingle(D)` naming the SYMBOL (no `QuoteID`) at the streamed `Price(44)` books
    /// through the SAME last-look ledger and FILLS — an `ExecutionReport(150=F)`, retiring the
    /// symbol's live token; a re-lift of the consumed line then rejects (idempotency).
    #[tokio::test]
    async fn md_lift_by_symbol_books_and_fills_then_rejects_replay() {
        let mut session = stream_session("conn-md-lift", None);
        let st = session.sending_time();

        let sub = ois_md_request(b"MDR-2", SubscriptionRequest::Subscribe);
        let sframe = FrameCursor::parse(&sub).unwrap();
        let mut out = Vec::new();
        session.on_market_data_request(&sframe, &st, &mut out).await;
        assert_eq!(session.live_md.len(), 1);
        let offer = snapshot_offer(&out);

        // Lift the offer (BUY) by symbol at the streamed price — no QuoteID(117).
        let order = md_buy_order(b"C-MD-1", b"USD-OIS", offer);
        let oframe = FrameCursor::parse(&order).unwrap();
        assert_eq!(
            oframe.get(117),
            None,
            "a market-data lift carries no QuoteID"
        );
        let mut exec_out = Vec::new();
        session.on_new_order(&oframe, &st, &mut exec_out);

        assert_eq!(exec_out.len(), 1, "one ExecutionReport emitted");
        assert_eq!(
            exec_type_of(&exec_out),
            Some(EXEC_FILLED),
            "the symbol lift fills"
        );
        assert!(
            session.live_md.is_empty(),
            "a filled lift retires the symbol's live token"
        );

        // A re-lift of the same (now-retired) symbol rejects — the token is consumed.
        let mut exec_out2 = Vec::new();
        session.on_new_order(&oframe, &st, &mut exec_out2);
        assert_eq!(
            exec_type_of(&exec_out2),
            Some(EXEC_REJECTED),
            "a re-lift of a consumed line rejects"
        );
    }

    /// Regression for the multi-instrument lift bug (UAT `912797UU9` et al. rejecting as
    /// "unknown or forged quote"): N symbols stream into ONE session ledger, so a per-symbol
    /// snapshot mint must NOT wipe sibling symbols' liftable tokens (the old whole-ledger
    /// `clear_live` did). Publish TWO symbols' snapshots, then lift the FIRST — the one the
    /// second mint's `clear_live` would have wiped — and confirm it BOOKS, not rejects.
    #[tokio::test]
    async fn md_lift_of_a_non_last_symbol_still_books() {
        let mut session = stream_session("conn-md-multi", None);
        let st = session.sending_time();
        let a = PricedLine {
            bid: 99.40,
            offer: 99.60,
            size: 5_000_000.0,
        };
        let b = PricedLine {
            bid: 0.0401,
            offer: 0.0403,
            size: 5_000_000.0,
        };

        // Publish symbol A, then symbol B — into the SAME session ledger.
        let mut out_a = Vec::new();
        session.emit_md_snapshot(&st, b"MDR-A", b"912797UU9", &a, None, &mut out_a);
        let a_offer = snapshot_offer(&out_a);
        session.emit_md_snapshot(&st, b"MDR-B", b"ACME-5Y-CORP", &b, None, &mut Vec::new());
        assert_eq!(
            session.live_md.len(),
            2,
            "both symbols are live simultaneously"
        );

        // Lift the FIRST symbol (A) at its streamed offer — under the old whole-ledger
        // clear_live A's token was wiped by B's mint → UnknownToken; now it must BOOK.
        let order = md_buy_order(b"C-A", b"912797UU9", a_offer);
        let mut exec_out = Vec::new();
        session.on_new_order(&FrameCursor::parse(&order).unwrap(), &st, &mut exec_out);
        assert_eq!(
            exec_type_of(&exec_out),
            Some(EXEC_FILLED),
            "a non-last symbol's lift must BOOK, not reject as unknown-token"
        );
    }

    /// The BUY `last_px` (dealt price) the emitted `ExecutionReport(8)` carried — so a test
    /// can assert the ACTUAL fill price the last-look policy produced (Async improvement).
    fn exec_last_px_of(frames: &[Vec<u8>]) -> Option<f64> {
        let exec = FrameCursor::parse(frames.first().expect("an ExecutionReport")).unwrap();
        assert_eq!(exec.msg_type(), MsgType::ExecutionReport.as_bytes());
        messages::ExecReportView::new(exec).last_px()
    }

    /// Build a by-symbol SELL `NewOrderSingle(D)` hitting `symbol`'s bid at `price` — the
    /// market-data lift the ESP sends to sell into the streamed bid (no `QuoteID(117)`).
    fn md_sell_order(cl_ord_id: &[u8], symbol: &[u8], price: f64) -> Vec<u8> {
        let hdr = Header {
            sender: b"CELNET",
            target: b"CELNET-CPTY",
            seq_num: 11,
            sending_time: b"20260625-12:00:02.000",
        };
        let p = messages::MarketOrderParams {
            cl_ord_id,
            symbol,
            side: dialect_fx::SIDE_SELL,
            qty: 1_000_000.0,
            price,
            transact_time: b"20260625-12:00:02.000",
        };
        let mut enc = FrameEncoder::new();
        messages::build_new_order_by_symbol(&hdr, &p, &mut enc)
    }

    /// The market-data last-look policy on a by-symbol lift, ADVERSE side: a BUY whose
    /// `Price(44)` is BELOW the current published offer (the client wants to pay less than
    /// the market — the dealer would sell below the current level) beyond the tolerance band
    /// is REJECTED as superseded, while the live token stays liftable for a fresh re-lift at
    /// the current price (the reject consumed nothing).
    #[tokio::test]
    async fn md_adverse_lift_beyond_tolerance_is_rejected_token_preserved() {
        let mut session = stream_session("conn-md-stale", None);
        let st = session.sending_time();

        let sub = ois_md_request(b"MDR-S", SubscriptionRequest::Subscribe);
        let mut out = Vec::new();
        session
            .on_market_data_request(&FrameCursor::parse(&sub).unwrap(), &st, &mut out)
            .await;
        let offer = snapshot_offer(&out);

        // BUY at an ADVERSE price (offer − 1bp of price, well beyond the 1bp tolerance band):
        // the offer would have to be LOWER than the current market ⇒ superseded reject.
        let stale = md_buy_order(b"C-STALE", b"USD-OIS", offer - 0.0001);
        let mut exec_stale = Vec::new();
        session.on_new_order(&FrameCursor::parse(&stale).unwrap(), &st, &mut exec_stale);
        assert_eq!(
            exec_type_of(&exec_stale),
            Some(EXEC_REJECTED),
            "an adverse-beyond-tolerance lift is rejected (last-look)"
        );
        assert_eq!(
            session.live_md.len(),
            1,
            "the live token is untouched by an adverse reject"
        );

        // A fresh lift at the CURRENT price still books (the adverse reject consumed nothing).
        let fresh = md_buy_order(b"C-FRESH", b"USD-OIS", offer);
        let mut exec_fresh = Vec::new();
        session.on_new_order(&FrameCursor::parse(&fresh).unwrap(), &st, &mut exec_fresh);
        assert_eq!(
            exec_type_of(&exec_fresh),
            Some(EXEC_FILLED),
            "a fresh lift at the current price books"
        );
    }

    /// The FAVORABLE side is NEVER superseded: a BUY whose `Price(44)` is ABOVE the current
    /// offer (the client offered to overpay — a move in the DEALER's favor) FILLS under the
    /// default Sync policy at exactly the client's requested price `q` (the dealer keeps the
    /// whole favorable move). This is the asymmetry the old ±exact-match reject lacked.
    #[tokio::test]
    async fn md_favorable_lift_fills_at_requested_price_under_sync() {
        let mut session = stream_session("conn-md-fav", None);
        let st = session.sending_time();
        let line = PricedLine {
            bid: 99.40,
            offer: 99.60,
            size: 5_000_000.0,
        };
        let mut out = Vec::new();
        session.emit_md_snapshot(&st, b"MDR-F", b"BND-5Y", &line, None, &mut out);

        // BUY at q = 99.70 > current offer 99.60 ⇒ favorable to the dealer. Sync ⇒ fill @ q.
        let q = 99.70;
        let order = md_buy_order(b"C-FAV", b"BND-5Y", q);
        let mut exec = Vec::new();
        session.on_new_order(&FrameCursor::parse(&order).unwrap(), &st, &mut exec);
        assert_eq!(
            exec_type_of(&exec),
            Some(EXEC_FILLED),
            "favorable BUY fills"
        );
        assert!(
            exec_last_px_of(&exec).is_some_and(|px| (px - q).abs() < 1e-9),
            "Sync fills at exactly the requested price q"
        );
    }

    /// The SELL (hit-bid) mirror: a SELL whose `Price(44)` is BELOW the current bid (the
    /// client sells for less than the current market — favorable to the dealer, who buys
    /// cheaper) FILLS under Sync at exactly the requested price `q`.
    #[tokio::test]
    async fn md_favorable_sell_fills_at_requested_price_under_sync() {
        let mut session = stream_session("conn-md-sell", None);
        let st = session.sending_time();
        let line = PricedLine {
            bid: 99.40,
            offer: 99.60,
            size: 5_000_000.0,
        };
        let mut out = Vec::new();
        session.emit_md_snapshot(&st, b"MDR-SL", b"BND-5Y", &line, None, &mut out);

        // SELL at q = 99.30 < current bid 99.40 ⇒ favorable to the dealer. Sync ⇒ fill @ q.
        let q = 99.30;
        let order = md_sell_order(b"C-SELL", b"BND-5Y", q);
        let mut exec = Vec::new();
        session.on_new_order(&FrameCursor::parse(&order).unwrap(), &st, &mut exec);
        assert_eq!(
            exec_type_of(&exec),
            Some(EXEC_FILLED),
            "favorable SELL fills"
        );
        assert!(
            exec_last_px_of(&exec).is_some_and(|px| (px - q).abs() < 1e-9),
            "Sync fills the SELL at exactly the requested price q"
        );
    }

    /// End-to-end Async improvement on the wire: a session whose pricing group is
    /// `LastLookMode::Async` with a 50% giveback fills a FAVORABLE BUY at `q + 50%·(c − q)`
    /// — the client's requested price improved half-way toward the current published price
    /// — and reports THAT dealt price in the `ExecutionReport(8)` `LastPx(31)`.
    #[tokio::test]
    async fn md_async_lift_improves_client_price_on_the_wire() {
        let hub = async_lastlook_hub();
        let mut session = stream_session("conn-async", Some(hub));
        let st = session.sending_time();
        let line = PricedLine {
            bid: 99.40,
            offer: 99.60,
            size: 5_000_000.0,
        };
        let mut out = Vec::new();
        session.emit_md_snapshot(&st, b"MDR-AS", b"BND-5Y", &line, None, &mut out);

        // BUY at q = 99.70 while the current offer c = 99.60 (favorable, Δ = q − c = 0.10).
        // Async 50% ⇒ fill = q + 0.5·(c − q) = 99.70 + 0.5·(−0.10) = 99.65 (client keeps half
        // the favorable move; the dealer keeps the other half over the current price).
        let q = 99.70;
        let order = md_buy_order(b"C-AS", b"BND-5Y", q);
        let mut exec = Vec::new();
        session.on_new_order(&FrameCursor::parse(&order).unwrap(), &st, &mut exec);
        assert_eq!(exec_type_of(&exec), Some(EXEC_FILLED), "async BUY fills");
        assert!(
            exec_last_px_of(&exec).is_some_and(|px| (px - 99.65).abs() < 1e-9),
            "Async fills at q + 50%·(c − q) = 99.65, not the raw {q} or the current 99.60"
        );
    }

    /// Pure last-look decision math — the sign convention and the Async improvement formula,
    /// asserted with EXACT numerics for BOTH sides (client buys our offer / sells into our
    /// bid) and every branch (adverse-beyond ⇒ supersede, adverse-within ⇒ honor `q`,
    /// favorable Sync ⇒ keep, favorable Async ⇒ share).
    #[test]
    fn md_last_look_decision_covers_both_sides_and_all_branches() {
        // Reference prices ~100 ⇒ the default 1bp band is ~0.01 (1e-4·100).
        let tol = DEFAULT_LAST_LOOK_TOLERANCE_BPS; // 1.0
        let give = 50.0;

        // --- Client BUYS our offer (dealer SELLS; favorable when the offer FELL, c < q). ---
        // Adverse (c > q) beyond band ⇒ supersede. q=100.50, c=100.52 (0.02 > ~0.010052).
        assert_eq!(
            md_last_look_decision(false, 100.50, 100.52, LastLookMode::Sync, tol, give),
            MdLastLook::Superseded
        );
        // Adverse within band ⇒ honor q. q=100.50, c=100.505 (0.005 < band).
        assert_eq!(
            md_last_look_decision(false, 100.50, 100.505, LastLookMode::Sync, tol, give),
            MdLastLook::Fill(100.50)
        );
        // Favorable (c < q) Sync ⇒ keep the whole move, fill @ q. q=100.50, c=100.40.
        assert_eq!(
            md_last_look_decision(false, 100.50, 100.40, LastLookMode::Sync, tol, give),
            MdLastLook::Fill(100.50)
        );
        // Favorable Async 50% ⇒ fill = q + 0.5·(c − q) = 100.50 + 0.5·(−0.10) = 100.45.
        assert_eq!(
            md_last_look_decision(false, 100.50, 100.40, LastLookMode::Async, tol, give),
            MdLastLook::Fill(100.45)
        );

        // --- Client SELLS into our bid (dealer BUYS; favorable when the bid ROSE, c > q). ---
        // Adverse (c < q) beyond band ⇒ supersede. q=99.50, c=99.48 (0.02 > ~0.009948).
        assert_eq!(
            md_last_look_decision(true, 99.50, 99.48, LastLookMode::Sync, tol, give),
            MdLastLook::Superseded
        );
        // Adverse within band ⇒ honor q. q=99.50, c=99.495 (0.005 < band).
        assert_eq!(
            md_last_look_decision(true, 99.50, 99.495, LastLookMode::Sync, tol, give),
            MdLastLook::Fill(99.50)
        );
        // Favorable (c > q) Sync ⇒ fill @ q. q=99.40, c=99.60.
        assert_eq!(
            md_last_look_decision(true, 99.40, 99.60, LastLookMode::Sync, tol, give),
            MdLastLook::Fill(99.40)
        );
        // Favorable Async 50% ⇒ fill = q + 0.5·(c − q) = 99.40 + 0.5·(0.20) = 99.50.
        assert_eq!(
            md_last_look_decision(true, 99.40, 99.60, LastLookMode::Async, tol, give),
            MdLastLook::Fill(99.50)
        );
    }

    /// Latency instrumentation (docs/LATENCY-AND-HEDGING-ANALYTICS-REQUIREMENTS.md): driving an
    /// inbound auto-quotable rates RFQ through the FIX edge records the quote CONSTRUCTION stage
    /// (`RfqQuote` — the classify/price step) AND the outbound quote PUBLISH stage
    /// (`StreamPublish` — the `emit_two_way_quote` frame emit) into the shared telemetry hub the
    /// `CoreLink` owns, both with `count > 0` in `stage_stats()`.
    #[tokio::test]
    async fn rates_rfq_records_construction_and_publish_stages() {
        use celnet_observability::OpKind;
        // Default (both-enabled) control ⇒ an admitted clip is auto-quoted (emits a Quote(S)).
        let mut session = stream_session("conn-lat", None);
        let st = session.sending_time();
        let frame_bytes = small_ois_subscribe_frame();
        let frame = FrameCursor::parse(&frame_bytes).expect("frame parses");
        let req_id = frame
            .get(131)
            .map(<[u8]>::to_vec)
            .expect("QuoteReqID present");
        let symbol = frame.get(55).map(<[u8]>::to_vec).expect("Symbol present");

        let mut out = Vec::new();
        session
            .on_rates_quote_request(&frame, &st, &req_id, &symbol, &mut out)
            .await;
        assert!(
            !out.is_empty(),
            "an admitted, on-the-run clip emits an outbound Quote(S)"
        );

        let stats = session.ctx.link.telemetry().stage_stats();
        let count_of = |k: OpKind| {
            stats
                .iter()
                .find(|s| s.kind == k)
                .map_or(0, |s| s.snapshot.count)
        };
        assert!(
            count_of(OpKind::RfqQuote) > 0,
            "the rates quote construction records the RfqQuote stage, got {stats:?}"
        );
        assert!(
            count_of(OpKind::StreamPublish) > 0,
            "the outbound emit records the StreamPublish stage, got {stats:?}"
        );
    }

    /// The auto-quote policy admits small, on-the-run clips and routes larger or
    /// off-the-run risk to a human desk — the mix the desk inbox surfaces. Asserts the
    /// exact predicate `classify_ois_rfq` reads (tenor in the on-the-run set AND notional
    /// within the clip cap).
    #[test]
    fn auto_quote_policy_admits_small_on_the_run_only() {
        let p = RatesAutoQuotePolicy::default();
        let admits =
            |notional: f64, tenor: u32| p.tenors.contains(&tenor) && notional <= p.max_notional;
        assert!(admits(10_000_000.0, 5)); // small + on-the-run ⇒ auto
        assert!(admits(25_000_000.0, 2)); // at the cap ⇒ auto
        assert!(!admits(50_000_000.0, 5)); // over the cap ⇒ desk
        assert!(!admits(10_000_000.0, 30)); // off-the-run tenor ⇒ desk
    }

    /// The exception-only classification: an on-the-run, within-cap, priceable RFQ
    /// auto-quotes (quiet); an unknown security / unconfigured tenor / pricing failure is
    /// a manual-intervention with the right reason; a large on-the-run clip is routed
    /// quietly (not an exception).
    #[test]
    fn classify_ois_admission_maps_every_reason() {
        let p = RatesAutoQuotePolicy::default();
        let line = || PricedLine {
            bid: 0.0400,
            offer: 0.0410,
            size: 10_000_000.0,
        };

        // On-the-run + within cap + priced ⇒ auto-quote (quiet).
        assert!(matches!(
            classify_ois_admission(&p, b"USD-OIS", 5, 10_000_000.0, Some(line())),
            RatesAdmission::Auto(_)
        ));
        // Unknown security (bogus symbol) ⇒ manual UNKNOWN_SECURITY — checked before the
        // tenor, even for an on-the-run tenor.
        assert!(matches!(
            classify_ois_admission(&p, b"XXX-UNKNOWN", 5, 10_000_000.0, Some(line())),
            RatesAdmission::Manual(ManualInterventionReason::UnknownSecurity)
        ));
        // Off-the-run tenor on the valid curve ⇒ manual UNCONFIGURED_TENOR.
        assert!(matches!(
            classify_ois_admission(&p, b"USD-OIS", 15, 10_000_000.0, None),
            RatesAdmission::Manual(ManualInterventionReason::UnconfiguredTenor)
        ));
        // On-the-run + within cap but the engine could not price it ⇒ manual
        // PRICING_FAILURE (distinct from an off-the-run tenor).
        assert!(matches!(
            classify_ois_admission(&p, b"USD-OIS", 5, 10_000_000.0, None),
            RatesAdmission::Manual(ManualInterventionReason::PricingFailure)
        ));
        // On-the-run but over the clip cap ⇒ routed to the desk quietly (NOT an alert).
        assert!(matches!(
            classify_ois_admission(&p, b"USD-OIS", 5, 500_000_000.0, Some(line())),
            RatesAdmission::RoutedToDesk
        ));
    }

    /// A dialect rates side maps to the canonical proto side (pay⇒buy, receive⇒sell).
    #[test]
    fn rates_side_maps_to_proto_side() {
        assert_eq!(rates_side_to_side(RatesSide::PayFixed), Side::Buy);
        assert_eq!(rates_side_to_side(RatesSide::ReceiveFixed), Side::Sell);
        assert_eq!(rates_side_to_side(RatesSide::TwoWay), Side::TwoWay);
    }

    /// The [`PricingSourceMode::CurveAnchoredBookSkew`] blend — a worked numerical example
    /// plus its defining properties. Curve two-way `[0.0400, 0.0440]` (mid 0.0420, half
    /// 0.0020); composite `[0.0426, 0.0434]` (mid 0.0430, half 0.0004). The skew takes the
    /// COMPOSITE half-spread and pulls the mid `w` of the way from the curve mid toward the
    /// composite mid.
    #[test]
    fn curve_anchored_book_skew_blends_precisely() {
        let curve = PricedLine {
            bid: 0.0400,
            offer: 0.0440,
            size: 5_000_000.0,
        };
        let comp = PricedLine {
            bid: 0.0426,
            offer: 0.0434,
            size: 5_000_000.0,
        };
        let curve_mid = 0.0420;
        let comp_mid = 0.0430;
        let comp_half = 0.0004;

        // w = 0.5 → mid halfway (0.0425), composite half-spread.
        let b = blend_curve_toward_composite(&curve, &comp, 0.5);
        let mid = 0.5 * (b.bid + b.offer);
        assert!((mid - 0.0425).abs() < 1e-12, "mid {mid}");
        assert!((0.5 * (b.offer - b.bid) - comp_half).abs() < 1e-12);
        assert!(b.bid <= b.offer, "non-crossed");
        assert!((b.size - 5_000_000.0).abs() < 1e-9);

        // w = 0 → pure curve mid; w = 1 → pure composite mid; both keep the composite spread.
        let at0 = blend_curve_toward_composite(&curve, &comp, 0.0);
        assert!((0.5 * (at0.bid + at0.offer) - curve_mid).abs() < 1e-12);
        let at1 = blend_curve_toward_composite(&curve, &comp, 1.0);
        assert!((0.5 * (at1.bid + at1.offer) - comp_mid).abs() < 1e-12);

        // Out-of-range weights are clamped to [0, 1] (belt-and-braces vs the write-time check).
        let hi = blend_curve_toward_composite(&curve, &comp, 5.0);
        assert!((0.5 * (hi.bid + hi.offer) - comp_mid).abs() < 1e-12);
        let lo = blend_curve_toward_composite(&curve, &comp, -3.0);
        assert!((0.5 * (lo.bid + lo.offer) - curve_mid).abs() < 1e-12);
    }

    /// The RFS curve fallback re-prices a retained line off whatever curve it is handed —
    /// so marking a different curve moves the stream (item C), with no fabricated movement.
    /// An OIS line on the P0 default reproduces `par_rate_for` exactly; a curve whose pillars
    /// are all shifted up moves the quoted par strictly up; an unpriceable (zero) tenor is
    /// `None` (the ticker then skips, never emits a bad price).
    #[test]
    fn curve_line_for_reprices_off_the_supplied_curve() {
        let base = crate::rates_pricing::default_usd_sofr_curve_set();
        let line = RfsLine::Ois { tenor_years: 5 };
        let priced = curve_line_for(&line, &base, 5_000_000.0).expect("prices on P0");
        let mid = 0.5 * (priced.bid + priced.offer);
        let par = crate::rates_pricing::par_rate_for(&base, 5).expect("par");
        assert!((mid - par).abs() < 1e-12, "mid {mid} vs par {par}");
        assert!((priced.size - 5_000_000.0).abs() < 1e-9);

        // A uniformly +50bp-shifted curve lifts the 5y par strictly (monotone in the pillars).
        let mut bumped = base.clone();
        for p in &mut bumped.ois_pillars {
            p.par_rate += 0.005;
        }
        let bumped_mid = {
            let l = curve_line_for(&line, &bumped, 5_000_000.0).expect("prices bumped");
            0.5 * (l.bid + l.offer)
        };
        assert!(bumped_mid > mid, "bumped {bumped_mid} !> base {mid}");

        // A zero tenor cannot price → None (skip, never a fabricated price).
        assert!(curve_line_for(&RfsLine::Ois { tenor_years: 0 }, &base, 1.0).is_none());
    }

    // --- fixed-income dialect dispatch --------------------------------------

    use celnet_fix::dialect_rates::{
        RatesQuoteRequestParams, RatesSide, SubscriptionRequest, build_rates_quote_request,
    };
    use celnet_fix::framing::FrameEncoder;
    use celnet_fix::messages::Header;

    /// The FX-options kind is not a rates venue; the two FI kinds each map to their
    /// intent.
    #[test]
    fn rates_intent_follows_the_connection_kind() {
        assert_eq!(rates_intent_for_kind(AcceptorKind::Options), None);
        assert_eq!(
            rates_intent_for_kind(AcceptorKind::FixedIncomeQuote),
            Some(RatesIntent::Rfq)
        );
        assert_eq!(
            rates_intent_for_kind(AcceptorKind::FixedIncomeStream),
            Some(RatesIntent::Rfs)
        );
    }

    /// A quote venue admits only a one-shot snapshot; a stream venue admits only a
    /// subscribe/unsubscribe — the `SubscriptionRequestType(263)` must match.
    #[test]
    fn subscription_must_match_the_venue_intent() {
        assert!(subscription_matches_intent(
            RatesIntent::Rfq,
            SubscriptionRequest::Snapshot
        ));
        assert!(!subscription_matches_intent(
            RatesIntent::Rfq,
            SubscriptionRequest::Subscribe
        ));
        assert!(subscription_matches_intent(
            RatesIntent::Rfs,
            SubscriptionRequest::Subscribe
        ));
        assert!(subscription_matches_intent(
            RatesIntent::Rfs,
            SubscriptionRequest::Unsubscribe
        ));
        assert!(!subscription_matches_intent(
            RatesIntent::Rfs,
            SubscriptionRequest::Snapshot
        ));
    }

    /// Build an OIS `QuoteRequest(R)` frame carrying `subscription` for the tests.
    fn ois_rfq_frame(subscription: SubscriptionRequest) -> Vec<u8> {
        let hdr = Header {
            sender: b"CELNET",
            target: b"CELNET-CPTY",
            seq_num: 7,
            sending_time: b"20260625-12:00:00.000",
        };
        let p = RatesQuoteRequestParams {
            quote_req_id: b"RFQ-1",
            symbol: b"USD-OIS",
            tenor_years: 5,
            notional: 100_000_000.0,
            side: RatesSide::TwoWay,
            subscription,
        };
        let mut enc = FrameEncoder::new();
        build_rates_quote_request(&hdr, &p, &mut enc)
    }

    /// A quote (RFQ) venue prices a snapshot request and refuses a streaming
    /// subscribe; a stream (RFS) venue does the mirror — the dispatch enforces the
    /// dialect's intent on the wire, reusing the shared rates decode/price path.
    #[test]
    fn rates_line_enforces_the_venue_intent() {
        let snapshot = ois_rfq_frame(SubscriptionRequest::Snapshot);
        let subscribe = ois_rfq_frame(SubscriptionRequest::Subscribe);

        let snap = FrameCursor::parse(&snapshot).unwrap();
        let sub = FrameCursor::parse(&subscribe).unwrap();
        let curve = crate::rates_pricing::default_usd_sofr_curve_set();

        // A quote venue: snapshot prices, subscribe is refused.
        assert!(rates_line(&snap, Some(RatesIntent::Rfq), &curve).is_ok());
        assert!(rates_line(&sub, Some(RatesIntent::Rfq), &curve).is_err());

        // A stream venue: subscribe prices, snapshot is refused.
        assert!(rates_line(&sub, Some(RatesIntent::Rfs), &curve).is_ok());
        assert!(rates_line(&snap, Some(RatesIntent::Rfs), &curve).is_err());

        // The legacy/demo content-detect path (no intent) prices either.
        assert!(rates_line(&snap, None, &curve).is_ok());
        assert!(rates_line(&sub, None, &curve).is_ok());
    }
}
