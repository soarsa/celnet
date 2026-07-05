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

use celnet_proto::{CcyPair, Instrument, MarketContext, Quantity, Side, StrikeOrDelta, Vanilla};
use celnet_proto::{CurveSet, DeskQuote, OisInstrument, RatesInstrument, rates_instrument};
use celnet_proto::{instrument, strike_or_delta};

use celnet_types::{OptionType, Tenor};

use crate::clock::Clock;
use crate::config::fix_connections::AcceptorKind;
use crate::core_link::CoreLink;
use crate::pricer::{ConventionSet, price_instrument};
use crate::services::clicktrade::{
    BookOutcome, MintedToken, TokenLedger, TokenMinter, TwoWayLine, mint_two_way,
};
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
}

/// A live **RFS** (request-for-stream) subscription on a fixed-income STREAM venue: the
/// venue re-prices this line off the P0 curve on every session tick and pushes a fresh
/// two-way `Quote(S)` until the taker Unsubscribes (or the session drops). Each streamed
/// update supersedes the prior one — only `last_quote_id` stays liftable, which keeps the
/// live-quote table bounded to one entry per subscription rather than growing per tick.
#[derive(Debug, Clone)]
struct RfsSubscription {
    /// The wire `QuoteReqID(131)` correlating every streamed update (and the Unsubscribe).
    req_id: Vec<u8>,
    /// The curve symbol echoed on each streamed `Quote(S)`.
    symbol: Vec<u8>,
    /// The RFQ notional carried as the streamed quote size.
    notional: f64,
    /// The static P0 par rate this stream oscillates a small demo movement around.
    base_par: f64,
    /// The desk history row id this subscription opened, so a lift of any streamed update
    /// books it as a completed deal (`None` when the venue is not desk-routed).
    request_id: Option<String>,
    /// A monotonic tick ordinal driving the deterministic (no wall-clock/RNG) movement.
    tick: u64,
    /// The most recent streamed quote's `QuoteID(117)`; retired when the next update is
    /// emitted so only the latest update is liftable (bounded live-quote table).
    last_quote_id: Option<Vec<u8>>,
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

impl RatesAutoQuotePolicy {
    /// Whether an RFQ of this `notional` and `tenor_years` is auto-quoted (`true`) or
    /// routed to a human desk (`false`).
    #[must_use]
    pub(crate) fn admits(&self, notional: f64, tenor_years: u32) -> bool {
        notional <= self.max_notional && self.tenors.contains(&tenor_years)
    }

    /// Whether a cash-bond RFQ of this `notional` is auto-quoted (`true`) or routed to a
    /// human desk (`false`). A bond carries no whole-year tenor to match the on-the-run
    /// set against (its schedule is a maturity date), so the bond admission is the
    /// notional cap alone — the same clip-size gate the OIS path applies.
    #[must_use]
    pub(crate) fn admits_notional(&self, notional: f64) -> bool {
        notional <= self.max_notional
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
        }
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
    /// Live FIX quotes keyed by the wire `QuoteID(117)` string.
    live: HashMap<Vec<u8>, FixQuote>,
    /// A monotonic line ordinal the keyed-MAC binds (the FIX analogue of the RFS
    /// `subscription_id`); fresh per quote so each line's tokens are distinct.
    next_line: u64,
    /// A counter minting unique `QuoteID`/`OrderID`/`ExecID` strings.
    seq: u64,
    /// Live RFS subscriptions keyed by the wire `QuoteReqID(131)`: on a fixed-income
    /// STREAM venue, the session's periodic ticker re-prices each and pushes a fresh
    /// `Quote(S)`. Empty on a quote/RFQ venue (and while an idle stream venue has no
    /// subscription) — so the ticker branch is dormant and the loop stays byte-identical
    /// to the request-response path.
    rfs: HashMap<Vec<u8>, RfsSubscription>,
}

impl FixSession {
    fn new(cfg: SessionConfig, ctx: FixContext) -> Self {
        Self {
            session: Session::new(cfg, InMemoryStore::new()),
            ctx,
            minter: TokenMinter::new(),
            ledger: TokenLedger::new(),
            live: HashMap::new(),
            next_line: 0,
            seq: 0,
            rfs: HashMap::new(),
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
        // The RFS streaming cadence. A fixed-income STREAM venue re-prices every live
        // subscription on this interval and pushes fresh two-way quotes; the ticker branch
        // is armed only while at least one subscription is live (`!self.rfs.is_empty()`),
        // so a quote/RFQ venue — or an idle stream venue — stays byte-identical to the
        // pure request-response loop. `next_frame` is cancellation-safe (its only await
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
                // Re-price + push every live RFS subscription. Dormant (guard false, never
                // polled) while no subscription is live, so an RFQ venue is unaffected.
                _ = ticker.tick(), if !self.rfs.is_empty() => {
                    let st = self.sending_time();
                    let frames = self.tick_rfs_stream(&st);
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

        // A dedicated fixed-income venue records every inbound RFQ into the desk inbox
        // (so the GUI shows inbound RFQs — live pending + processed history) and decides
        // auto-quote vs route-to-human. The FX-options path and the legacy
        // content-detected OIS path below are unchanged (byte-identical).
        if rates_intent_for_kind(self.ctx.kind).is_some() {
            self.on_rates_quote_request(frame, st, &req_id, &symbol, out)
                .await;
            return;
        }

        // Resolve the option + price it; a dialect/convention/pricing error declines
        // the quote (no `Quote` is sent — the maker simply does not show a price). The
        // FX-vanilla pre-trade template (ADR-0016 A1) rides alongside the priced line so
        // a lift can run the limit gate; a rates line carries `None`.
        let (priced, fx) = match self.price_request(frame).await {
            Ok(p) => p,
            Err(_) => return,
        };
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
        // An RFS Unsubscribe tears down the live stream for this correlation id and stops
        // (no quote, no desk row) — the session ticker no longer re-prices it.
        if matches!(
            rfq.subscription,
            dialect_rates::SubscriptionRequest::Unsubscribe
        ) {
            self.rfs.remove(req_id);
            return;
        }
        let side = rates_side_to_side(rfq.side);
        let curve = crate::rates_pricing::default_usd_sofr_curve_set();
        // Auto-quote only a policy-admitted RFQ that the venue can actually price; a
        // request the policy declines (over the cap / off-the-run) OR a tenor the venue
        // cannot price (e.g. one that doesn't exist for this curve) routes to a human
        // desk as PENDING — never dropped — so it always surfaces in the desk inbox.
        let priced = if self.ctx.auto_quote.admits(rfq.notional, rfq.tenor_years) {
            rates_line(frame, Some(intent)).ok()
        } else {
            None
        };
        match priced {
            Some(priced) => {
                // Auto-quote: the SAME shared rates line + `Quote(S)`, plus a QUOTED
                // history row at the two-way mid whose id rides the live quote so a lift
                // books it as a completed deal. A rates line carries no FX pre-trade
                // template (no canonical-vanilla leaf).
                let mid = 0.5 * (priced.bid + priced.offer);
                let request_id = self.record_rates_rfq(&rfq, side, &curve, Some(mid));
                let quote_id = self.emit_two_way_quote(
                    st,
                    req_id,
                    symbol,
                    &priced,
                    None,
                    request_id.clone(),
                    out,
                );
                // On a STREAM (RFS) venue a Subscribe opens a CONTINUOUS stream: register
                // the subscription so the session ticker re-prices and pushes updates until
                // an Unsubscribe (or session close). The quote just emitted is the first
                // update; the desk row id rides every update so a lift of any one books the
                // same deal and closes the stream.
                if intent == RatesIntent::Rfs {
                    self.rfs.insert(
                        req_id.to_vec(),
                        RfsSubscription {
                            req_id: req_id.to_vec(),
                            symbol: symbol.to_vec(),
                            notional: rfq.notional,
                            base_par: mid,
                            request_id,
                            tick: 0,
                            last_quote_id: Some(quote_id),
                        },
                    );
                }
            }
            None => {
                // Route to a human desk: PENDING, no auto `Quote(S)` (the desk prices it).
                self.record_rates_rfq(&rfq, side, &curve, None);
            }
        }
    }

    /// Re-price every live RFS subscription off the P0 curve (with a small deterministic
    /// demo movement) and return the fresh two-way `Quote(S)` frames to push. Each update
    /// retires the subscription's prior live quote so only the latest stays liftable,
    /// keeping the live-quote table bounded to one entry per subscription. Driven by the
    /// session loop's streaming ticker; returns an empty vec when no subscription is live.
    fn tick_rfs_stream(&mut self, st: &[u8]) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        // Snapshot the keys so the loop can re-borrow `self` (live table, quote minter)
        // between subscriptions while mutating the subscription set in place.
        let keys: Vec<Vec<u8>> = self.rfs.keys().cloned().collect();
        for key in keys {
            let (req_id, symbol, notional, base_par, request_id, tick, prior) = {
                let Some(sub) = self.rfs.get_mut(&key) else {
                    continue;
                };
                sub.tick += 1;
                (
                    sub.req_id.clone(),
                    sub.symbol.clone(),
                    sub.notional,
                    sub.base_par,
                    sub.request_id.clone(),
                    sub.tick,
                    sub.last_quote_id.take(),
                )
            };
            // Retire the superseded quote — only the latest streamed update is liftable.
            if let Some(prior) = prior {
                self.live.remove(&prior);
            }
            let moved_par = rfs_streamed_rate(base_par, tick);
            let (bid, offer) = dialect_rates::two_way_rates(moved_par, RATES_HALF_SPREAD);
            let priced = PricedLine {
                bid,
                offer,
                size: notional,
            };
            let quote_id = self.emit_two_way_quote(
                st,
                &req_id,
                &symbol,
                &priced,
                None,
                request_id,
                &mut frames,
            );
            if let Some(sub) = self.rfs.get_mut(&key) {
                sub.last_quote_id = Some(quote_id);
            }
        }
        frames
    }

    /// Record an inbound rates RFQ into the desk inbox under this venue's desk. When
    /// `auto_level` is `Some`, the venue auto-quoted → stored QUOTED at that level;
    /// otherwise stored PENDING for a human trader. Returns the stored request id (so a
    /// lift can book it), or `None` when the acceptor is not desk-routed (legacy env seed /
    /// tests) or carries no desk.
    fn record_rates_rfq(
        &self,
        rfq: &dialect_rates::RatesRfq,
        side: Side,
        curve: &CurveSet,
        auto_level: Option<f64>,
    ) -> Option<String> {
        let edge = self.ctx.desk_edge.as_ref()?;
        if self.ctx.desk.trim().is_empty() {
            return None;
        }
        let counterparty = String::from_utf8_lossy(&self.ctx.counterparty).into_owned();
        let instrument = RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                tenor_years: rfq.tenor_years,
                fixed_rate: auto_level.unwrap_or(0.0),
                notional: rfq.notional,
                side: side as i32,
            })),
        };
        let quote = auto_level.map(|lvl| DeskQuote {
            price: lvl,
            notional: rfq.notional,
            valid_for_ms: RATES_AUTO_QUOTE_VALID_MS,
            trader: "auto".to_owned(),
        });
        let stored = edge.ingest_fix_rfq(
            &self.ctx.desk,
            &counterparty,
            instrument,
            curve.clone(),
            side,
            rfq.notional,
            quote,
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
        let curve = crate::rates_pricing::default_usd_sofr_curve_set();
        // Auto-quote only a policy-admitted RFQ the venue can actually price; a request
        // over the notional cap OR one the engine cannot price (e.g. a maturity that does
        // not resolve) routes to a human desk as PENDING — never dropped.
        let priced = if self.ctx.auto_quote.admits_notional(rfq.notional) {
            bond_line(frame, Some(intent)).ok()
        } else {
            None
        };
        match priced {
            Some(priced) => {
                self.emit_two_way_quote(st, req_id, symbol, &priced, None, None, out);
                let mid = 0.5 * (priced.bid + priced.offer);
                self.record_bond_rfq(&rfq, &curve, Some(mid));
            }
            None => {
                self.record_bond_rfq(&rfq, &curve, None);
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
        curve: &CurveSet,
        auto_level: Option<f64>,
    ) {
        let Some(edge) = self.ctx.desk_edge.as_ref() else {
            return;
        };
        if self.ctx.desk.trim().is_empty() {
            return;
        }
        let counterparty = String::from_utf8_lossy(&self.ctx.counterparty).into_owned();
        let side = Side::try_from(rfq.instrument.side).unwrap_or(Side::TwoWay);
        let instrument = RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Bond(rfq.instrument)),
        };
        let quote = auto_level.map(|lvl| DeskQuote {
            price: lvl,
            notional: rfq.notional,
            valid_for_ms: RATES_AUTO_QUOTE_VALID_MS,
            trader: "auto".to_owned(),
        });
        edge.ingest_fix_rfq(
            &self.ctx.desk,
            &counterparty,
            instrument,
            curve.clone(),
            side,
            rfq.notional,
            quote,
        );
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

        // Resolve the per-side token for this lift from the named quote.
        let token = quote_id
            .as_ref()
            .and_then(|q| self.live.get(q))
            .and_then(|lq| {
                if side_byte == dialect_fx::SIDE_SELL {
                    lq.sell_token
                } else {
                    lq.buy_token
                }
            });
        let symbol = quote_id
            .as_ref()
            .and_then(|q| self.live.get(q))
            .map(|lq| lq.symbol.clone())
            .or_else(|| frame.get(55).map(<[u8]>::to_vec))
            .unwrap_or_default();
        // The FX-vanilla pre-trade template for this line (ADR-0016 A1), captured at
        // quote time; `None` for a rates line or an unknown quote.
        let fx_line = quote_id
            .as_ref()
            .and_then(|q| self.live.get(q))
            .and_then(|lq| lq.fx);
        // The desk request id a rates auto-quote / RFS stream created for this line; a
        // filled lift books it as a completed deal (below). `None` for an FX/unknown line.
        let rates_request_id = quote_id
            .as_ref()
            .and_then(|q| self.live.get(q))
            .and_then(|lq| lq.rates_request_id.clone());

        // Book through the SAME last-look ledger the RFS stream uses. An unknown
        // quote/side ⇒ UnknownToken (no live token), exactly as a forged token.
        let outcome = match token {
            Some(t) => self.ledger.try_book(t, now),
            None => BookOutcome::UnknownToken,
        };

        let (mut filled, premium, mut text): (bool, f64, Option<&[u8]>) = match outcome {
            BookOutcome::Booked { premium, .. } => (true, premium, None),
            BookOutcome::Expired => (false, 0.0, Some(b"quote expired (last-look)")),
            BookOutcome::AlreadyConsumed => (false, 0.0, Some(b"quote already executed")),
            BookOutcome::UnknownToken => (false, 0.0, Some(b"unknown or forged quote")),
        };

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

        // A successful lift retires the quote (idempotency: a second lift of the same
        // QuoteID now rejects as already-consumed via the ledger). A limit-rejected lift
        // is NOT retired, but its token is already consumed, so a re-lift still rejects.
        if filled && let Some(q) = quote_id.as_ref() {
            self.live.remove(q);
        }

        // A filled rates auto-quote / RFS-stream lift books the desk request it created as
        // a completed deal (dealt position + Deal + ACCEPTED) and closes any live RFS
        // stream for it — so the FIX taker's execution shows in the deal blotter and the
        // rates Book, exactly like a GUI desk accept.
        if filled && let Some(request_id) = rates_request_id {
            if let Some(edge) = self.ctx.desk_edge.as_ref() {
                let _ = edge.book_fix_lift(&request_id);
            }
            self.rfs
                .retain(|_, sub| sub.request_id.as_deref() != Some(request_id.as_str()));
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
        if let Some(intent) = rates_intent_for_kind(self.ctx.kind) {
            // A rates line has no canonical-vanilla risk leaf ⇒ no pre-trade template.
            // The bond arm is selected by `SecurityType(167)=BOND`; every other rates
            // request is the OIS arm.
            if frame.get(167) == Some(dialect_rates::SEC_TYPE_BOND) {
                return bond_line(frame, Some(intent)).map(|line| (line, None));
            }
            return rates_line(frame, Some(intent)).map(|line| (line, None));
        }
        // The FX-options / legacy-demo acceptor still content-detects a fixed-income
        // request by `SecurityType(167)` (BOND before OIS), so a mixed acceptor prices
        // cash bonds and OIS alongside FX options.
        if frame.get(167) == Some(dialect_rates::SEC_TYPE_BOND) {
            return bond_line(frame, None).map(|line| (line, None));
        }
        if frame.get(167) == Some(dialect_rates::SEC_TYPE_OIS) {
            return rates_line(frame, None).map(|line| (line, None));
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

/// The demo movement amplitude a streamed rate oscillates around its static P0 par —
/// ±2bp, a realistic intraday wiggle that keeps the live stream visibly moving without
/// misrepresenting the curve level.
const RFS_MOVE_AMPLITUDE: f64 = 0.0002;

/// A deterministic small oscillation of a streamed par rate driven by the subscription
/// tick ordinal — a triangle wave in `[-1, 1]` over an 8-tick period, scaled by
/// [`RFS_MOVE_AMPLITUDE`]. Pure arithmetic (no wall-clock, no RNG, no libm), so the
/// stream is reproducible and stays off the pricing hot path while looking alive.
fn rfs_streamed_rate(base_par: f64, tick: u64) -> f64 {
    const PERIOD: u64 = 8;
    let phase = (tick % PERIOD) as f64 / PERIOD as f64; // [0, 1)
    let saw = 2.0 * phase - 1.0; // [-1, 1)
    let triangle = 1.0 - 2.0 * saw.abs(); // [-1, 1], peak at phase 0.5
    base_par + RFS_MOVE_AMPLITUDE * triangle
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

fn rates_line(frame: &FrameCursor<'_>, expected: Option<RatesIntent>) -> Result<PricedLine, ()> {
    let rfq = dialect_rates::decode_rates_rfq(frame).map_err(|_| ())?;
    if let Some(intent) = expected
        && !subscription_matches_intent(intent, rfq.subscription)
    {
        return Err(());
    }
    let curve = crate::rates_pricing::default_usd_sofr_curve_set();
    let par = crate::rates_pricing::par_rate_for(&curve, rfq.tenor_years).map_err(|_| ())?;
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
fn bond_line(frame: &FrameCursor<'_>, expected: Option<RatesIntent>) -> Result<PricedLine, ()> {
    let rfq = dialect_rates::decode_bond_rfq(frame).map_err(|_| ())?;
    if let Some(intent) = expected
        && !subscription_matches_intent(intent, rfq.subscription)
    {
        return Err(());
    }
    let curve = crate::rates_pricing::default_usd_sofr_curve_set();
    let quote = crate::rates_pricing::quote_bond(&rfq.instrument, &curve).map_err(|_| ())?;
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

    /// The auto-quote policy admits small, on-the-run clips and routes larger or
    /// off-the-run risk to a human desk — the mix the desk inbox surfaces.
    #[test]
    fn auto_quote_policy_admits_small_on_the_run_only() {
        let p = RatesAutoQuotePolicy::default();
        assert!(p.admits(10_000_000.0, 5)); // small + on-the-run ⇒ auto
        assert!(p.admits(25_000_000.0, 2)); // at the cap ⇒ auto
        assert!(!p.admits(50_000_000.0, 5)); // over the cap ⇒ desk
        assert!(!p.admits(10_000_000.0, 30)); // off-the-run tenor ⇒ desk
    }

    /// A dialect rates side maps to the canonical proto side (pay⇒buy, receive⇒sell).
    #[test]
    fn rates_side_maps_to_proto_side() {
        assert_eq!(rates_side_to_side(RatesSide::PayFixed), Side::Buy);
        assert_eq!(rates_side_to_side(RatesSide::ReceiveFixed), Side::Sell);
        assert_eq!(rates_side_to_side(RatesSide::TwoWay), Side::TwoWay);
    }

    /// The streamed RFS rate oscillates a small bounded movement around the static P0 par:
    /// deterministic (same tick ⇒ same rate), centred, within ±the amplitude, and actually
    /// moving — the wiggle that makes the live stream visibly alive without misquoting the
    /// curve level.
    #[test]
    fn rfs_streamed_rate_oscillates_within_amplitude() {
        let base = 0.0405;
        for tick in 0..64u64 {
            let r = rfs_streamed_rate(base, tick);
            assert!((r - base).abs() <= RFS_MOVE_AMPLITUDE + 1e-12);
        }
        // Deterministic + periodic (period 8).
        assert_eq!(rfs_streamed_rate(base, 3), rfs_streamed_rate(base, 11));
        // The wave actually moves (not a constant line).
        assert_ne!(rfs_streamed_rate(base, 0), rfs_streamed_rate(base, 4));
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

        // A quote venue: snapshot prices, subscribe is refused.
        assert!(rates_line(&snap, Some(RatesIntent::Rfq)).is_ok());
        assert!(rates_line(&sub, Some(RatesIntent::Rfq)).is_err());

        // A stream venue: subscribe prices, snapshot is refused.
        assert!(rates_line(&sub, Some(RatesIntent::Rfs)).is_ok());
        assert!(rates_line(&snap, Some(RatesIntent::Rfs)).is_err());

        // The legacy/demo content-detect path (no intent) prices either.
        assert!(rates_line(&snap, None).is_ok());
        assert!(rates_line(&sub, None).is_ok());
    }
}
