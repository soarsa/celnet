//! The initiator (price-taker / hedge) role: drives a [`Session`] over a real
//! socket, sending `Logon`, requesting quotes (`QuoteRequest`), and lifting a
//! returned `Quote` with a `NewOrderSingle` against its `QuoteID` (last-look
//! anchor), then collecting the `ExecutionReport`.
//!
//! The initiator is deliberately thin: it owns no pricing, only the session and
//! the request/lift workflow the hedge desk needs. It shares the session FSM
//! with the acceptor, so the two interoperate over a loopback socket exactly as
//! they would against the live Celer FIX edge.

use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::dictionary::MsgType;
use crate::framing::FrameCursor;
use crate::messages::{self, Header, NewOrderParams, QuoteView};
use crate::session::{MessageStore, Session, SessionAction};
use crate::transport::{FrameReader, write_frame};

/// What the initiator should do after receiving a `Quote`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiftPolicy {
    /// Always lift the offer (BUY) on the first valid quote.
    LiftOffer,
    /// Always hit the bid (SELL) on the first valid quote.
    HitBid,
    /// Observe quotes without lifting (indicative RFQ).
    Observe,
}

/// Outcome of running an initiator request/lift cycle.
#[derive(Debug, Clone, Default)]
pub struct InitiatorResult {
    /// The `QuoteID` of the quote received (if any).
    pub quote_id: Option<Vec<u8>>,
    /// The bid / offer the quote carried.
    pub bid: Option<f64>,
    /// The offer the quote carried.
    pub offer: Option<f64>,
    /// The fill price from the `ExecutionReport` (if filled).
    pub fill_px: Option<f64>,
    /// Whether the execution report reported a fill.
    pub filled: bool,
}

/// The default time the initiator waits for a `Quote` before giving up: a venue that
/// routes an RFQ to a human desk (rather than auto-quoting) sends no reply, so without
/// a bound the request/lift cycle would block forever. Five seconds comfortably covers
/// an auto-quote round-trip while keeping a routed (no-quote) request responsive.
pub const DEFAULT_QUOTE_TIMEOUT: Duration = Duration::from_secs(5);

/// The initiator driver.
pub struct Initiator<S: MessageStore> {
    session: Session<S>,
    policy: LiftPolicy,
    seq: u64,
    /// How long to wait for a `Quote` (and its exec report, when lifting) before the
    /// cycle returns no-quote. Bounds the wait so a venue that routes the RFQ to a human
    /// desk — and therefore never auto-quotes — does not hang the caller.
    quote_timeout: Duration,
}

impl<S: MessageStore> Initiator<S> {
    /// Build an initiator over a session and a lift policy (default quote timeout).
    pub fn new(session: Session<S>, policy: LiftPolicy) -> Self {
        Self {
            session,
            policy,
            seq: 0,
            quote_timeout: DEFAULT_QUOTE_TIMEOUT,
        }
    }

    /// Override the quote-wait timeout (e.g. a longer window for a slow manual desk, or
    /// a shorter one for a snappy demo). Builder-style; leaves every other field intact.
    #[must_use]
    pub fn with_quote_timeout(mut self, quote_timeout: Duration) -> Self {
        self.quote_timeout = quote_timeout;
        self
    }

    fn mint(&mut self, prefix: &str) -> Vec<u8> {
        self.seq += 1;
        format!("{prefix}-{}", self.seq).into_bytes()
    }

    /// Borrow the session for introspection.
    pub fn session(&self) -> &Session<S> {
        &self.session
    }

    /// Connect (logon), send a single `QuoteRequest` built by `build_request`,
    /// then react per the lift policy, returning once the cycle completes (a
    /// `Quote` observed and, if lifting, its `ExecutionReport` received).
    ///
    /// # Errors
    /// Propagates transport I/O errors.
    pub async fn request_and_lift<RW>(
        &mut self,
        stream: RW,
        sending_time: Vec<u8>,
        build_request: impl FnOnce(&Header<'_>, &mut crate::framing::FrameEncoder) -> Vec<u8>,
    ) -> std::io::Result<InitiatorResult>
    where
        RW: AsyncRead + AsyncWrite + Unpin,
    {
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = FrameReader::new(read_half);
        if !self.do_logon(&mut reader, &mut write_half, &sending_time).await? {
            return Ok(InitiatorResult::default());
        }
        self.send_request(&mut write_half, &sending_time, build_request)
            .await?;
        self.collect(&mut reader, &mut write_half, &sending_time).await
    }

    /// Open a **persistent** session: log on ONCE and return an [`InitiatorSession`]
    /// handle that sends many requests over the SAME FIX session (the sequence number
    /// increments; no re-logon per request). The session stays up until the handle is
    /// dropped — this is how the quote simulator stays logged in across a stream of
    /// RFQs instead of a logon/logout churn per request.
    ///
    /// # Errors
    /// Propagates transport I/O errors, or fails if the peer closes before logon completes.
    pub async fn open<RW>(
        &mut self,
        stream: RW,
        sending_time: &[u8],
    ) -> std::io::Result<InitiatorSession<'_, S, RW>>
    where
        RW: AsyncRead + AsyncWrite + Unpin,
    {
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = FrameReader::new(read_half);
        if !self.do_logon(&mut reader, &mut write_half, sending_time).await? {
            return Err(std::io::Error::other("peer closed before logon completed"));
        }
        Ok(InitiatorSession {
            initiator: self,
            reader,
            write_half,
        })
    }

    /// Logon and wait for the mirror. Returns `Ok(true)` once the session is Active,
    /// `Ok(false)` if the peer closed before logon completed. Shared by the one-shot
    /// [`Self::request_and_lift`] and the persistent [`Self::open`].
    async fn do_logon<R, W>(
        &mut self,
        reader: &mut FrameReader<R>,
        write_half: &mut W,
        sending_time: &[u8],
    ) -> std::io::Result<bool>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let logon = self.session.start_logon(sending_time, false);
        write_frame(write_half, &logon).await?;
        loop {
            let Some(frame_bytes) = reader.next_frame().await? else {
                return Ok(false);
            };
            let action = self.drive(&frame_bytes, sending_time, write_half).await?;
            if matches!(action, Some(MsgType::Logon))
                && self.session.state() == crate::session::SessionState::Active
            {
                return Ok(true);
            }
        }
    }

    /// Send one application quote request over the (already logged-on) session.
    async fn send_request<W>(
        &mut self,
        write_half: &mut W,
        sending_time: &[u8],
        build_request: impl FnOnce(&Header<'_>, &mut crate::framing::FrameEncoder) -> Vec<u8>,
    ) -> std::io::Result<()>
    where
        W: AsyncWrite + Unpin,
    {
        let req = self.session.send_app(sending_time, build_request);
        write_frame(write_half, &req).await
    }

    /// Collect the quote, optionally lift, collect the exec report. Bounded by a
    /// deadline so a venue that routes the RFQ to a human desk (no auto-quote reply —
    /// only periodic heartbeats) returns no-quote instead of blocking forever.
    /// Heartbeats do not extend the deadline.
    async fn collect<R, W>(
        &mut self,
        reader: &mut FrameReader<R>,
        write_half: &mut W,
        sending_time: &[u8],
    ) -> std::io::Result<InitiatorResult>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut result = InitiatorResult::default();
        let mut awaiting_exec = false;
        let deadline = tokio::time::Instant::now() + self.quote_timeout;
        loop {
            let frame_bytes = match tokio::time::timeout_at(deadline, reader.next_frame()).await {
                // Deadline hit — the venue is not going to auto-quote (routed to a desk).
                Err(_elapsed) => break,
                Ok(Ok(Some(bytes))) => bytes,
                // Peer closed the session.
                Ok(Ok(None)) => break,
                // A transport error propagates as before.
                Ok(Err(e)) => return Err(e),
            };
            let mt = self.drive(&frame_bytes, sending_time, write_half).await?;
            match mt {
                Some(MsgType::Quote) => {
                    let frame = FrameCursor::parse(&frame_bytes)
                        .map_err(|_| std::io::Error::other("bad quote frame"))?;
                    let view = QuoteView::new(frame);
                    result.quote_id = view.quote_id().map(<[u8]>::to_vec);
                    result.bid = view.bid();
                    result.offer = view.offer();
                    match self.policy {
                        LiftPolicy::Observe => break,
                        LiftPolicy::LiftOffer | LiftPolicy::HitBid => {
                            let side = if self.policy == LiftPolicy::LiftOffer {
                                crate::dialect_fx::SIDE_BUY
                            } else {
                                crate::dialect_fx::SIDE_SELL
                            };
                            let cl = self.mint("C");
                            let qid = result.quote_id.clone().unwrap_or_default();
                            let symbol = view
                                .symbol()
                                .map(<[u8]>::to_vec)
                                .unwrap_or_else(|| b"EURUSD".to_vec());
                            let order = self.session.send_app(sending_time, |h, e| {
                                let p = NewOrderParams {
                                    cl_ord_id: &cl,
                                    quote_id: &qid,
                                    symbol: &symbol,
                                    side,
                                    qty: 1_000_000.0,
                                    transact_time: b"20260530-12:00:01.000",
                                };
                                messages::build_new_order_single(h, &p, e)
                            });
                            write_frame(write_half, &order).await?;
                            awaiting_exec = true;
                        }
                    }
                }
                Some(MsgType::ExecutionReport) if awaiting_exec => {
                    let frame = FrameCursor::parse(&frame_bytes)
                        .map_err(|_| std::io::Error::other("bad exec frame"))?;
                    let view = messages::ExecReportView::new(frame);
                    result.filled = view.exec_type() == Some(messages::EXEC_FILLED);
                    result.fill_px = view.last_px();
                    break;
                }
                _ => {}
            }
        }
        Ok(result)
    }

    /// Feed one inbound frame to the session and transmit any session-level
    /// outbound. Returns the inbound application `MsgType` (if delivered).
    async fn drive<W>(
        &mut self,
        frame_bytes: &[u8],
        sending_time: &[u8],
        write_half: &mut W,
    ) -> std::io::Result<Option<MsgType>>
    where
        W: AsyncWrite + Unpin,
    {
        // Logon mirror is processed by on_inbound; detect it for the caller.
        let pre_state = self.session.state();
        let action: SessionAction = self
            .session
            .on_inbound(frame_bytes, sending_time)
            .map_err(|_| std::io::Error::other("session protocol error"))?;
        for f in &action.outbound {
            write_frame(write_half, f).await?;
        }
        if let Some(mt) = action.deliver {
            return Ok(Some(mt));
        }
        // The logon mirror has no `deliver`; surface it by state transition.
        if pre_state == crate::session::SessionState::LogonPending
            && self.session.state() == crate::session::SessionState::Active
        {
            return Ok(Some(MsgType::Logon));
        }
        Ok(None)
    }
}

/// A **persistent, logged-on** initiator session: send many quote requests over ONE
/// FIX session (no logon/logout per request). Created by [`Initiator::open`]; the
/// session stays up until this handle is dropped. The simulator uses it to log on once
/// and stream RFQs with incrementing sequence numbers.
pub struct InitiatorSession<'a, S: MessageStore, RW: AsyncRead + AsyncWrite + Unpin> {
    initiator: &'a mut Initiator<S>,
    reader: FrameReader<tokio::io::ReadHalf<RW>>,
    write_half: tokio::io::WriteHalf<RW>,
}

impl<S: MessageStore, RW: AsyncRead + AsyncWrite + Unpin> InitiatorSession<'_, S, RW> {
    /// Send one quote request over the already-open session and collect the quote (and
    /// lift, per the initiator's [`LiftPolicy`]). Reuses the SAME session — the sequence
    /// number increments, no new logon. Bounded by the initiator's quote timeout so a
    /// desk-routed (no-quote) RFQ returns cleanly and the caller can send the next one.
    ///
    /// # Errors
    /// Propagates transport I/O errors.
    pub async fn request(
        &mut self,
        sending_time: &[u8],
        build_request: impl FnOnce(&Header<'_>, &mut crate::framing::FrameEncoder) -> Vec<u8>,
    ) -> std::io::Result<InitiatorResult> {
        self.initiator
            .send_request(&mut self.write_half, sending_time, build_request)
            .await?;
        self.initiator
            .collect(&mut self.reader, &mut self.write_half, sending_time)
            .await
    }
}
