//! The acceptor (quote-venue) role: drives a [`Session`] over a real socket,
//! answering `QuoteRequest(R)` with a priced `Quote(S)` (or `MassQuote(i)`),
//! and executing `NewOrderSingle(D)` / `NewOrderMultileg(AB)` against a live
//! quote with last-look — the lifted `QuoteID(117)` must still be valid.
//!
//! The acceptor is convention-correct: every inbound option block is decoded
//! and convention-checked via [`crate::dialect_fx`] before it is priced off the
//! supplied [`crate::dialect_fx::MarketSnapshot`] (one consistent surface
//! snapshot per request) using the `celnet-vanilla` engine.

use std::collections::HashMap;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::dialect_fx::{self, MarketSnapshot, SecurityDef};
use crate::dictionary::MsgType;
use crate::framing::FrameCursor;
use crate::messages::{self, EXEC_FILLED, EXEC_REJECTED, ExecReportParams, QuoteParams};
use crate::session::{MessageStore, Session, SessionAction};
use crate::transport::{FrameReader, write_frame};

/// A live quote the acceptor has streamed and will honour on a lift, until it
/// expires. Keyed by `QuoteID`.
#[derive(Debug, Clone)]
struct LiveQuote {
    symbol: Vec<u8>,
    bid_px: f64,
    offer_px: f64,
    /// Monotonic validity deadline (caller-supplied tick units).
    valid_until_tick: u64,
}

/// Resolves a market snapshot + tenor for an inbound option request. In
/// production this consults the live surface; the trait keeps the acceptor
/// independent of the surface source and lets tests supply a fixed snapshot
/// (a *real* pricing input, not a mock of acceptor behaviour).
pub trait QuoteSource {
    /// Provide the snapshot + tenor to price the option described by `frame`.
    fn snapshot_for(&self, frame: &FrameCursor<'_>) -> (MarketSnapshot, celnet_types::Tenor);
    /// The half-spread (in premium units) applied around the mid to form the
    /// two-sided quote.
    fn half_spread(&self) -> f64;
    /// Quote validity window in tick units (last-look horizon).
    fn validity_ticks(&self) -> u64;
    /// The vanilla pricer the venue uses to value the option off the snapshot.
    fn pricer(&self) -> dialect_fx::VanillaPricer;
    /// The venue's authoritative projection of the tradable-securities universe
    /// it can quote. Answered verbatim to a `SecurityListRequest(x)` as a
    /// `SecurityList(y)`, so a client can download exactly what the venue
    /// prices before it ever sends an RFQ.
    fn securities(&self) -> Vec<SecurityDef>;
}

/// The acceptor state machine driving a session and a live-quote table.
pub struct Acceptor<S: MessageStore, Q: QuoteSource> {
    session: Session<S>,
    quote_source: Q,
    live: HashMap<Vec<u8>, LiveQuote>,
    /// Monotonic tick counter advanced by the caller per inbound event.
    tick: u64,
    /// Counter for minting unique QuoteIDs/OrderIDs/ExecIDs.
    seq: u64,
}

impl<S: MessageStore, Q: QuoteSource> Acceptor<S, Q> {
    /// Build an acceptor over a session and a quote source.
    pub fn new(session: Session<S>, quote_source: Q) -> Self {
        Self {
            session,
            quote_source,
            live: HashMap::new(),
            tick: 0,
            seq: 0,
        }
    }

    /// Advance the acceptor's logical clock (e.g. on each heartbeat tick). Used
    /// to expire stale quotes for last-look.
    pub fn advance(&mut self, ticks: u64) {
        self.tick += ticks;
    }

    fn mint(&mut self, prefix: &str) -> Vec<u8> {
        self.seq += 1;
        format!("{prefix}-{}", self.seq).into_bytes()
    }

    /// Run the acceptor loop over a connected stream until the peer closes.
    ///
    /// # Errors
    /// Propagates transport I/O errors.
    pub async fn run<RW>(&mut self, stream: RW, sending_time: Vec<u8>) -> std::io::Result<()>
    where
        RW: AsyncRead + AsyncWrite + Unpin,
    {
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = FrameReader::new(read_half);
        while let Some(frame) = reader.next_frame().await? {
            self.tick += 1;
            let outbound = self.handle_frame(&frame, &sending_time);
            for f in outbound {
                write_frame(&mut write_half, &f).await?;
            }
            if self.session.state() == crate::session::SessionState::Disconnected
                && self.session.next_outbound() > 1
            {
                break;
            }
        }
        Ok(())
    }

    /// Process one inbound frame and return all frames to transmit. Pure (no
    /// I/O) so it is unit-testable without a socket.
    pub fn handle_frame(&mut self, raw: &[u8], sending_time: &[u8]) -> Vec<Vec<u8>> {
        let action: SessionAction = match self.session.on_inbound(raw, sending_time) {
            Ok(a) => a,
            // A protocol fault is surfaced as a session-level reject; we do not
            // panic. Returning no frames lets the caller decide to disconnect.
            Err(_) => return Vec::new(),
        };
        let mut out = action.outbound;
        if let Some(mt) = action.deliver {
            let frame = match FrameCursor::parse(raw) {
                Ok(f) => f,
                Err(_) => return out,
            };
            match mt {
                MsgType::QuoteRequest => self.on_quote_request(&frame, sending_time, &mut out),
                MsgType::NewOrderSingle => self.on_new_order(&frame, sending_time, &mut out),
                MsgType::NewOrderMultileg => self.on_new_order(&frame, sending_time, &mut out),
                MsgType::SecurityListRequest => {
                    self.on_security_list_request(&frame, sending_time, &mut out);
                }
                _ => {}
            }
        }
        out
    }

    fn on_quote_request(&mut self, frame: &FrameCursor<'_>, st: &[u8], out: &mut Vec<Vec<u8>>) {
        let req_id = match frame.get(131) {
            Some(v) => v.to_vec(),
            None => return,
        };
        let symbol = match frame.get(55) {
            Some(v) => v.to_vec(),
            None => return,
        };
        let (snap, tenor) = self.quote_source.snapshot_for(frame);

        // Decode + convention-check, then price off the snapshot.
        let mid = match dialect_fx::decode_option(frame, tenor) {
            Ok(desc) => dialect_fx::price_leg(&desc, &snap, self.quote_source.pricer()),
            Err(_) => return, // convention/dialect error: do not quote
        };

        let hs = self.quote_source.half_spread();
        let bid = mid - hs;
        let offer = mid + hs;
        let quote_id = self.mint("Q");
        let valid_until_tick = self.tick + self.quote_source.validity_ticks();
        self.live.insert(
            quote_id.clone(),
            LiveQuote {
                symbol: symbol.clone(),
                bid_px: bid,
                offer_px: offer,
                valid_until_tick,
            },
        );

        let frame_out = self.session.send_app(st, |h, e| {
            let p = QuoteParams {
                quote_req_id: &req_id,
                quote_id: &quote_id,
                symbol: &symbol,
                bid_px: bid,
                offer_px: offer,
                size: 1_000_000.0,
                valid_until: b"20260530-12:00:05.000",
            };
            messages::build_quote(h, &p, e)
        });
        out.push(frame_out);
    }

    fn on_new_order(&mut self, frame: &FrameCursor<'_>, st: &[u8], out: &mut Vec<Vec<u8>>) {
        let cl_ord_id = frame.get(11).map(<[u8]>::to_vec).unwrap_or_default();
        let side = frame
            .get(54)
            .and_then(|v| v.first().copied())
            .unwrap_or(b'1');
        let quote_id = frame.get(117).map(<[u8]>::to_vec);

        // Last-look: the lifted QuoteID must exist and not have expired.
        let (filled, px, symbol) = match quote_id.as_ref().and_then(|q| self.live.get(q)) {
            Some(lq) if self.tick <= lq.valid_until_tick => {
                // Fill at the side-appropriate price (BUY @ offer, SELL @ bid).
                let px = if side == dialect_fx::SIDE_BUY {
                    lq.offer_px
                } else {
                    lq.bid_px
                };
                (true, px, lq.symbol.clone())
            }
            _ => (
                false,
                0.0,
                frame.get(55).map(<[u8]>::to_vec).unwrap_or_default(),
            ),
        };

        // Consume the quote on a successful lift (idempotency: a second lift of
        // the same QuoteID will now reject).
        if filled && let Some(q) = quote_id.as_ref() {
            self.live.remove(q);
        }

        let order_id = self.mint("O");
        let exec_id = self.mint("E");
        let frame_out = self.session.send_app(st, |h, e| {
            let p = ExecReportParams {
                order_id: &order_id,
                exec_id: &exec_id,
                cl_ord_id: &cl_ord_id,
                exec_type: if filled { EXEC_FILLED } else { EXEC_REJECTED },
                ord_status: if filled { EXEC_FILLED } else { EXEC_REJECTED },
                symbol: &symbol,
                side,
                last_qty: if filled { 1_000_000.0 } else { 0.0 },
                last_px: px,
                multileg_type: None,
                text: if filled {
                    None
                } else {
                    Some(b"quote expired or unknown")
                },
            };
            messages::build_execution_report(h, &p, e)
        });
        out.push(frame_out);
    }

    /// Answer a `SecurityListRequest(x)`: echo the `SecurityReqID(320)` and
    /// stream the venue's authoritative tradable-securities universe (from
    /// [`QuoteSource::securities`]) back as a single `SecurityList(y)` fragment.
    fn on_security_list_request(
        &mut self,
        frame: &FrameCursor<'_>,
        st: &[u8],
        out: &mut Vec<Vec<u8>>,
    ) {
        let req_id = match frame.get(320) {
            Some(v) => v.to_vec(),
            None => return,
        };
        let securities = self.quote_source.securities();
        let frame_out = self.session.send_app(st, |h, e| {
            let p = dialect_fx::SecurityListParams {
                security_req_id: &req_id,
                securities: &securities,
            };
            dialect_fx::build_security_list(h, &p, e)
        });
        out.push(frame_out);
    }

    /// Borrow the underlying session (state introspection / tests).
    pub fn session(&self) -> &Session<S> {
        &self.session
    }

    /// Mutable session access (so a caller can drive heartbeats).
    pub fn session_mut(&mut self) -> &mut Session<S> {
        &mut self.session
    }
}

/// A simple [`QuoteSource`] backed by a fixed snapshot and spread — the real
/// pricing inputs a surface would supply, with no acceptor behaviour faked.
#[derive(Debug, Clone)]
pub struct FixedQuoteSource {
    /// The market snapshot used for every request.
    pub snapshot: MarketSnapshot,
    /// The tenor resolved for every request.
    pub tenor: celnet_types::Tenor,
    /// Half-spread around the mid.
    pub half_spread: f64,
    /// Quote validity in ticks.
    pub validity_ticks: u64,
    /// The injected vanilla pricer (the async edge supplies `celnet_vanilla::price`).
    pub pricer: dialect_fx::VanillaPricer,
    /// The tradable-securities universe this venue advertises on a
    /// `SecurityListRequest(x)` — the securities it can quote.
    pub securities: Vec<SecurityDef>,
}

impl QuoteSource for FixedQuoteSource {
    fn snapshot_for(&self, _frame: &FrameCursor<'_>) -> (MarketSnapshot, celnet_types::Tenor) {
        (self.snapshot, self.tenor)
    }
    fn half_spread(&self) -> f64 {
        self.half_spread
    }
    fn validity_ticks(&self) -> u64 {
        self.validity_ticks
    }
    fn pricer(&self) -> dialect_fx::VanillaPricer {
        self.pricer
    }
    fn securities(&self) -> Vec<SecurityDef> {
        self.securities.clone()
    }
}
