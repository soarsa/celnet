//! The desk-backed FIX↔gRPC gateway: a FIX acceptor that maps the rates quoting
//! lifecycle onto the Celnet dealer desk ([`crate::backend::DeskBackend`]).
//!
//! This is the fixed-income complement to [`crate::acceptor`] (which prices FX
//! options locally). Where that venue quotes off a market snapshot, the gateway
//! quotes off the *human desk*: an inbound rates `QuoteRequest(R)` is submitted
//! into the desk as an RFQ (or IOI), the desk trader's response is awaited and
//! relayed back as a `Quote(S)`, and the counterparty's lift
//! (`NewOrderSingle(D)` referencing the `QuoteID`) books the deal via
//! `AcceptDeskQuote` and is reported as an `ExecutionReport(8)`.
//!
//! The session layer ([`Session`]) handles all admin messages
//! (Logon/Logout/Heartbeat/TestRequest/ResendRequest/Reject/SequenceReset) with
//! correct sequence-number, `BodyLength` and `CheckSum` framing; the gateway only
//! adds the application lifecycle on top. Every desk round-trip is async; the
//! whole connection is driven from one task so the session FSM stays single-owner.
//!
//! ## RFQ vs IOI
//!
//! A counterparty signals an indication-of-interest by setting `QuoteType(537)=0`
//! (Indicative) on the `QuoteRequest`; absent or `1` (Tradeable) is a firm RFQ.
//! Both submit into the desk and relay the desk's firm level back — the desk
//! decides whether to work or price the axe.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::backend::{DeskBackend, DeskRfq, DeskSide, RequestKind, ResponseOutcome};
use crate::dialect_rates::{self, RatesSide};
use crate::dictionary::MsgType;
use crate::framing::FrameCursor;
use crate::messages::{self, EXEC_FILLED, EXEC_REJECTED, ExecReportParams, QuoteParams};
use crate::session::{MessageStore, Session, SessionState};
use crate::transport::{FrameReader, write_frame};

/// `QuoteType(537)`.
const TAG_QUOTE_TYPE: u32 = 537;
/// `QuoteType(537)=0` — indicative (mapped to a desk IOI).
const QUOTE_TYPE_INDICATIVE: &[u8] = b"0";
/// `QuoteRequestRejectReason(658)=99` — "Other" (the reason text carries detail).
const QRR_REASON_OTHER: i64 = 99;

/// Identity + routing configuration for a [`DeskGateway`].
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// The desk inbound requests route to (the desk's entitlement + notification
    /// scope, matched against the backend's desk binding).
    pub desk: String,
    /// The counterparty label stamped on submitted requests (display /
    /// attribution) — typically the peer's `SenderCompID`.
    pub counterparty: String,
}

/// A gateway run-loop failure (transport I/O). Session protocol faults are
/// handled inline (surfaced as session-level rejects or a clean teardown) and
/// never abort the loop; backend faults become outbound FIX rejects.
#[derive(Debug)]
pub enum GatewayError {
    /// An I/O error on the underlying socket.
    Io(std::io::Error),
}

impl core::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GatewayError::Io(e) => write!(f, "gateway transport error: {e}"),
        }
    }
}

impl std::error::Error for GatewayError {}

impl From<std::io::Error> for GatewayError {
    fn from(e: std::io::Error) -> Self {
        GatewayError::Io(e)
    }
}

/// The desk-backed FIX acceptor, generic over the [`MessageStore`] (resend
/// replay) and the [`DeskBackend`] (the venue seam).
pub struct DeskGateway<S: MessageStore, B: DeskBackend> {
    session: Session<S>,
    backend: Arc<B>,
    config: GatewayConfig,
    /// `QuoteID(117)` → `Symbol(55)`, so the `ExecutionReport(8)` for a lift
    /// carries the dealt instrument's symbol even if the order omits it.
    live: HashMap<Vec<u8>, Vec<u8>>,
}

impl<S: MessageStore, B: DeskBackend> DeskGateway<S, B> {
    /// Build a gateway over an acceptor-role [`Session`], a shared
    /// [`DeskBackend`], and its routing config.
    pub fn new(session: Session<S>, backend: Arc<B>, config: GatewayConfig) -> Self {
        Self {
            session,
            backend,
            config,
            live: HashMap::new(),
        }
    }

    /// Borrow the underlying session (state introspection / tests).
    pub fn session(&self) -> &Session<S> {
        &self.session
    }

    /// Drive the gateway over a connected stream until the peer closes (or logs
    /// out). Reads complete frames, runs the session + desk lifecycle, and writes
    /// every resulting frame back.
    ///
    /// # Errors
    /// Propagates transport I/O errors.
    pub async fn run<RW>(&mut self, stream: RW) -> Result<(), GatewayError>
    where
        RW: AsyncRead + AsyncWrite + Unpin,
    {
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = FrameReader::new(read_half);
        while let Some(frame) = reader.next_frame().await? {
            let outbound = self.handle_frame(&frame).await;
            for f in &outbound {
                write_frame(&mut write_half, f).await?;
            }
            if self.session.state() == SessionState::Disconnected
                && self.session.next_outbound() > 1
            {
                break;
            }
        }
        Ok(())
    }

    /// Process one inbound frame and return all frames to transmit. Awaits the
    /// desk backend for the application lifecycle. Exposed (not just used by
    /// [`DeskGateway::run`]) so a conformance test can replay a recorded FIX
    /// conversation through the codec + lifecycle with no socket.
    pub async fn handle_frame(&mut self, raw: &[u8]) -> Vec<Vec<u8>> {
        let st = now_fix_utc();
        let action = match self.session.on_inbound(raw, &st) {
            Ok(a) => a,
            // A protocol fault: no app processing, no panic. The session has
            // already emitted any reject it deems necessary.
            Err(_) => return Vec::new(),
        };
        let mut out = action.outbound;
        if let Some(mt) = action.deliver {
            let frame = match FrameCursor::parse(raw) {
                Ok(f) => f,
                Err(_) => return out,
            };
            match mt {
                MsgType::QuoteRequest => self.on_quote_request(&frame, &st, &mut out).await,
                MsgType::NewOrderSingle | MsgType::NewOrderMultileg => {
                    self.on_new_order(&frame, &st, &mut out).await;
                }
                _ => {}
            }
        }
        out
    }

    /// Map an inbound rates `QuoteRequest(R)` onto a desk RFQ/IOI and relay the
    /// desk's response (a `Quote(S)`, or a `QuoteRequestReject(AG)` on decline).
    async fn on_quote_request(
        &mut self,
        frame: &FrameCursor<'_>,
        st: &[u8],
        out: &mut Vec<Vec<u8>>,
    ) {
        let rfq = match dialect_rates::decode_rates_rfq(frame) {
            Ok(r) => r,
            Err(e) => {
                // A malformed RFQ with an identifiable id is rejected by id; one
                // without even a `QuoteReqID` is dropped (nothing to address).
                if let Some(qid) = frame.get(131) {
                    let symbol = frame.get(55).unwrap_or(b"");
                    self.push_reject(st, qid, symbol, &format!("malformed rates RFQ: {e:?}"), out);
                }
                return;
            }
        };

        let kind = match frame.get(TAG_QUOTE_TYPE) {
            Some(QUOTE_TYPE_INDICATIVE) => RequestKind::Ioi,
            _ => RequestKind::Rfq,
        };
        let side = match rfq.side {
            RatesSide::PayFixed => DeskSide::PayFixed,
            RatesSide::ReceiveFixed => DeskSide::ReceiveFixed,
            RatesSide::TwoWay => DeskSide::TwoWay,
        };
        let desk_rfq = DeskRfq {
            kind,
            counterparty: self.config.counterparty.clone(),
            desk: self.config.desk.clone(),
            symbol: String::from_utf8_lossy(&rfq.symbol).into_owned(),
            tenor_years: rfq.tenor_years,
            notional: rfq.notional,
            side,
        };

        let request_id = match self.backend.submit(desk_rfq).await {
            Ok(id) => id,
            Err(e) => {
                self.push_reject(st, &rfq.quote_req_id, &rfq.symbol, &e.to_string(), out);
                return;
            }
        };

        match self.backend.await_response(request_id).await {
            Ok(ResponseOutcome::Quoted(q)) => {
                let quote_id = q.request_id.into_bytes();
                self.live.insert(quote_id.clone(), rfq.symbol.clone());
                let valid_until = fix_utc_plus_ms(q.valid_for_ms);
                let req_id = rfq.quote_req_id.clone();
                let symbol = rfq.symbol.clone();
                let frame_out = self.session.send_app(st, |h, e| {
                    let p = QuoteParams {
                        quote_req_id: &req_id,
                        quote_id: &quote_id,
                        symbol: &symbol,
                        // The desk shows one firm all-in level; the FIX two-way
                        // carries it on both sides (a firm, non-indicative price).
                        bid_px: q.price,
                        offer_px: q.price,
                        size: q.notional,
                        valid_until: &valid_until,
                    };
                    messages::build_quote(h, &p, e)
                });
                out.push(frame_out);
            }
            Ok(ResponseOutcome::Declined(reason)) => {
                self.push_reject(st, &rfq.quote_req_id, &rfq.symbol, &reason, out);
            }
            Err(e) => {
                self.push_reject(st, &rfq.quote_req_id, &rfq.symbol, &e.to_string(), out);
            }
        }
    }

    /// Map an inbound `NewOrderSingle(D)` lift (against a `QuoteID`) onto
    /// `AcceptDeskQuote`, reporting the booked deal as an `ExecutionReport(8)`
    /// (or a rejected report if the desk declines / the quote is gone).
    async fn on_new_order(&mut self, frame: &FrameCursor<'_>, st: &[u8], out: &mut Vec<Vec<u8>>) {
        let cl_ord_id = frame.get(11).map(<[u8]>::to_vec).unwrap_or_default();
        let side = frame
            .get(54)
            .and_then(|v| v.first().copied())
            .unwrap_or(b'1');
        let quote_id = frame.get(117).map(<[u8]>::to_vec);
        let symbol = quote_id
            .as_ref()
            .and_then(|q| self.live.get(q).cloned())
            .or_else(|| frame.get(55).map(<[u8]>::to_vec))
            .unwrap_or_default();

        let Some(qid) = quote_id.clone() else {
            self.push_exec_reject(st, &cl_ord_id, &symbol, side, b"missing QuoteID", out);
            return;
        };
        let request_id = String::from_utf8_lossy(&qid).into_owned();

        match self.backend.accept(request_id).await {
            Ok(fill) => {
                self.live.remove(&qid);
                let order_id = fill.deal_id.clone().into_bytes();
                let exec_id = fill.deal_id.into_bytes();
                let frame_out = self.session.send_app(st, |h, e| {
                    let p = ExecReportParams {
                        order_id: &order_id,
                        exec_id: &exec_id,
                        cl_ord_id: &cl_ord_id,
                        exec_type: EXEC_FILLED,
                        ord_status: EXEC_FILLED,
                        symbol: &symbol,
                        side,
                        last_qty: fill.notional,
                        last_px: fill.price,
                        multileg_type: None,
                        text: None,
                    };
                    messages::build_execution_report(h, &p, e)
                });
                out.push(frame_out);
            }
            Err(e) => {
                let reason = e.to_string();
                self.push_exec_reject(st, &cl_ord_id, &symbol, side, reason.as_bytes(), out);
            }
        }
    }

    /// Emit a `QuoteRequestReject(AG)` addressing `quote_req_id`.
    fn push_reject(
        &mut self,
        st: &[u8],
        quote_req_id: &[u8],
        symbol: &[u8],
        reason: &str,
        out: &mut Vec<Vec<u8>>,
    ) {
        let frame_out = self.session.send_app(st, |h, e| {
            messages::build_quote_request_reject(
                h,
                quote_req_id,
                symbol,
                QRR_REASON_OTHER,
                reason.as_bytes(),
                e,
            )
        });
        out.push(frame_out);
    }

    /// Emit a rejected `ExecutionReport(8)` for a lift that could not book.
    fn push_exec_reject(
        &mut self,
        st: &[u8],
        cl_ord_id: &[u8],
        symbol: &[u8],
        side: u8,
        text: &[u8],
        out: &mut Vec<Vec<u8>>,
    ) {
        let order_id = b"REJ".to_vec();
        let exec_id = b"REJ".to_vec();
        let frame_out = self.session.send_app(st, |h, e| {
            let p = ExecReportParams {
                order_id: &order_id,
                exec_id: &exec_id,
                cl_ord_id,
                exec_type: EXEC_REJECTED,
                ord_status: EXEC_REJECTED,
                symbol,
                side,
                last_qty: 0.0,
                last_px: 0.0,
                multileg_type: None,
                text: Some(text),
            };
            messages::build_execution_report(h, &p, e)
        });
        out.push(frame_out);
    }
}

/// The current UTC instant as a FIX `SendingTime(52)` string
/// (`YYYYMMDD-HH:MM:SS.sss`).
#[must_use]
pub fn now_fix_utc() -> Vec<u8> {
    fix_timestamp(time::OffsetDateTime::now_utc())
}

/// `now + ms` as a FIX UTC timestamp — the `ValidUntilTime(62)` of a quote whose
/// desk validity window is `ms` milliseconds.
#[must_use]
fn fix_utc_plus_ms(ms: u32) -> Vec<u8> {
    fix_timestamp(time::OffsetDateTime::now_utc() + time::Duration::milliseconds(i64::from(ms)))
}

/// Format an [`OffsetDateTime`](time::OffsetDateTime) as the FIX UTC timestamp
/// `YYYYMMDD-HH:MM:SS.sss` (the only timestamp shape the engine emits).
fn fix_timestamp(t: time::OffsetDateTime) -> Vec<u8> {
    format!(
        "{:04}{:02}{:02}-{:02}:{:02}:{:02}.{:03}",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second(),
        t.millisecond(),
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_timestamp_is_well_formed() {
        // 2026-06-29T12:34:56.789Z
        let dt = time::OffsetDateTime::from_unix_timestamp_nanos(1_782_563_696_789_000_000)
            .expect("valid instant");
        let s = fix_timestamp(dt);
        let text = std::str::from_utf8(&s).unwrap();
        assert_eq!(text.len(), 21, "YYYYMMDD-HH:MM:SS.sss is 21 chars: {text}");
        assert_eq!(&text[8..9], "-");
        assert_eq!(&text[11..12], ":");
        assert_eq!(&text[17..18], ".");
    }

    #[test]
    fn now_fix_utc_has_the_canonical_shape() {
        let s = now_fix_utc();
        assert_eq!(s.len(), 21);
        assert_eq!(s[8], b'-');
    }
}
