//! The initiator (price-taker / hedge) role: drives a [`Session`] over a real
//! socket, sending `Logon`, requesting quotes (`QuoteRequest`), and lifting a
//! returned `Quote` with a `NewOrderSingle` against its `QuoteID` (last-look
//! anchor), then collecting the `ExecutionReport`.
//!
//! The initiator is deliberately thin: it owns no pricing, only the session and
//! the request/lift workflow the hedge desk needs. It shares the session FSM
//! with the acceptor, so the two interoperate over a loopback socket exactly as
//! they would against the live Celer FIX edge.

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

/// The initiator driver.
pub struct Initiator<S: MessageStore> {
    session: Session<S>,
    policy: LiftPolicy,
    seq: u64,
}

impl<S: MessageStore> Initiator<S> {
    /// Build an initiator over a session and a lift policy.
    pub fn new(session: Session<S>, policy: LiftPolicy) -> Self {
        Self {
            session,
            policy,
            seq: 0,
        }
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

        // 1. Logon and wait for the mirror.
        let logon = self.session.start_logon(&sending_time, false);
        write_frame(&mut write_half, &logon).await?;
        loop {
            let Some(frame_bytes) = reader.next_frame().await? else {
                return Ok(InitiatorResult::default());
            };
            let action = self
                .drive(&frame_bytes, &sending_time, &mut write_half)
                .await?;
            if matches!(action, Some(MsgType::Logon))
                && self.session.state() == crate::session::SessionState::Active
            {
                break;
            }
        }

        // 2. Send the quote request.
        let req = self.session.send_app(&sending_time, build_request);
        write_frame(&mut write_half, &req).await?;

        // 3. Collect the quote, optionally lift, collect the exec report.
        let mut result = InitiatorResult::default();
        let mut awaiting_exec = false;
        loop {
            let Some(frame_bytes) = reader.next_frame().await? else {
                break;
            };
            let mt = self
                .drive(&frame_bytes, &sending_time, &mut write_half)
                .await?;
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
                            let order = self.session.send_app(&sending_time, |h, e| {
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
                            write_frame(&mut write_half, &order).await?;
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
