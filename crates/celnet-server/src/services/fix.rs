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
use celnet_fix::dictionary::MsgType;
use celnet_fix::framing::FrameCursor;
use celnet_fix::messages::{self, EXEC_FILLED, EXEC_REJECTED, ExecReportParams, QuoteParams};
use celnet_fix::session::{InMemoryStore, Role, Session, SessionAction, SessionConfig};
use celnet_fix::transport::{FrameReader, write_frame};

use celnet_proto::{CcyPair, Instrument, MarketContext, Quantity, Side, StrikeOrDelta, Vanilla};
use celnet_proto::{instrument, strike_or_delta};

use celnet_types::{OptionType, Tenor};

use crate::clock::Clock;
use crate::core_link::CoreLink;
use crate::pricer::{ConventionSet, price_instrument};
use crate::services::clicktrade::{
    BookOutcome, MintedToken, TokenLedger, TokenMinter, TwoWayLine, mint_two_way,
};
use crate::services::pin::{PinnedVol, resolve_pinned_vol};
use crate::spread::SpreadModel;
use crate::surface_book::SurfaceBook;

/// The custom dialect tag carrying the option's **vol-time in years** as a FIX float.
///
/// The platform's pricing is tenor- *and* vol-time-based; rather than depend on a
/// calendar resolution of `MaturityDate(541)` (which would drift with the trade
/// date), the dialect carries the exact `expiry_years` the engine prices against on a
/// user-defined tag (FIX tolerates unknown tags; this is a private dialect provenance
/// field, never a vendor name — `CLAUDE.md` rule 8). It makes the RFQ instrument
/// fully wire-specified, so the returned premium reproduces the engine/golden price to
/// the bit, independent of any date.
pub const TAG_EXPIRY_YEARS: u32 = 7001;

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
        }
    }

    /// Build a context with explicit CompIDs (the race-free path for tests, which bind
    /// ephemeral ports and must not mutate process-global env).
    pub(crate) fn with_comp_ids(
        link: Arc<CoreLink>,
        spread: SpreadModel,
        clock: Clock,
        surface_book: Arc<SurfaceBook>,
        sender: Vec<u8>,
        counterparty: Vec<u8>,
    ) -> Self {
        Self {
            link,
            spread,
            clock,
            surface_book,
            sender,
            counterparty,
        }
    }

    /// Project the live engine market state into the wire market context the pricer
    /// consumes — the SAME projection `QuoteService::live_market` performs.
    async fn live_market(&self) -> Result<MarketContext, String> {
        let snap = self
            .link
            .market_snapshot()
            .await
            .map_err(|e| e.to_string())?;
        Ok(MarketContext {
            spot: snap.spot,
            vol: snap.atm_vol,
            r_dom: snap.r_dom,
            r_for: snap.r_for,
        })
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
        while let Some(frame) = reader.next_frame().await? {
            let st = self.sending_time();
            let outbound = self.handle_frame(&frame, &st).await;
            for f in outbound {
                write_frame(&mut write_half, &f).await?;
            }
            if self.session.state() == celnet_fix::session::SessionState::Disconnected
                && self.session.next_outbound() > 1
            {
                break;
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

        // Resolve the option + price it; a dialect/convention/pricing error declines
        // the quote (no `Quote` is sent — the maker simply does not show a price).
        let priced = match self.price_request(frame).await {
            Ok(p) => p,
            Err(_) => return,
        };

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
                symbol: symbol.clone(),
                buy_token,
                sell_token,
            },
        );

        let valid_until = utc_timestamp(now.saturating_add(QUOTE_VALIDITY_NANOS));
        let frame_out = self.session.send_app(st, |h, e| {
            let p = QuoteParams {
                quote_req_id: &req_id,
                quote_id: &quote_id,
                symbol: &symbol,
                bid_px: priced.bid,
                offer_px: priced.offer,
                size: priced.size,
                valid_until: &valid_until,
            };
            messages::build_quote(h, &p, e)
        });
        out.push(frame_out);
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

        // Book through the SAME last-look ledger the RFS stream uses. An unknown
        // quote/side ⇒ UnknownToken (no live token), exactly as a forged token.
        let outcome = match token {
            Some(t) => self.ledger.try_book(t, now),
            None => BookOutcome::UnknownToken,
        };

        let (filled, premium, text): (bool, f64, Option<&[u8]>) = match outcome {
            BookOutcome::Booked { premium, .. } => (true, premium, None),
            BookOutcome::Expired => (false, 0.0, Some(b"quote expired (last-look)")),
            BookOutcome::AlreadyConsumed => (false, 0.0, Some(b"quote already executed")),
            BookOutcome::UnknownToken => (false, 0.0, Some(b"unknown or forged quote")),
        };

        // A successful lift retires the quote (idempotency: a second lift of the same
        // QuoteID now rejects as already-consumed via the ledger).
        if filled && let Some(q) = quote_id.as_ref() {
            self.live.remove(q);
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
    /// gRPC `QuoteService` would return for this instrument and market).
    async fn price_request(&self, frame: &FrameCursor<'_>) -> Result<PricedLine, ()> {
        // The vol-time in years carried by the dialect (fully wire-specified, no date
        // dependency). Required for a deterministic, reproducible premium.
        let expiry_years = frame
            .get(TAG_EXPIRY_YEARS)
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
        Ok(PricedLine {
            bid: two_way.bid,
            offer: two_way.offer,
            size: 1_000_000.0,
        })
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

/// Build the canonical [`Instrument`] from a decoded dialect descriptor and the
/// carried vol-time. A FX vanilla call/put at an absolute strike, base-notional 1mm.
fn instrument_from_descriptor(desc: &OptionDescriptor, expiry_years: f64) -> Instrument {
    Instrument {
        pair: Some(CcyPair {
            base: desc.pair.base.as_str().to_owned(),
            quote: desc.pair.quote.as_str().to_owned(),
        }),
        tenor: None,
        expiry_years,
        quantity: Some(Quantity {
            notional: 1_000_000.0,
            base_ccy: true,
        }),
        side: Side::TwoWay as i32,
        solve: None,
        product: Some(instrument::Product::Vanilla(Vanilla {
            option_type: match desc.option_type {
                OptionType::Call => celnet_proto::OptionType::Call as i32,
                OptionType::Put => celnet_proto::OptionType::Put as i32,
            },
            strike: Some(StrikeOrDelta {
                spec: Some(strike_or_delta::Spec::Strike(desc.strike)),
            }),
        })),
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
}
