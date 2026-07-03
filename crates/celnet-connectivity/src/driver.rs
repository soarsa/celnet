//! Live FIX session driver — runs a certification cycle for a venue adapter over
//! the `celnet-fix` engine, feeding every observed wire event into a
//! [`CertLedger`](crate::cert::CertLedger).
//!
//! This is the **initiator-side** counterpart to `celnet-server`'s inbound
//! `FixAcceptorRegistry`. It reuses `celnet-fix`'s [`Session`] state machine —
//! no FIX framing/session logic is re-implemented here. The caller supplies the
//! app-flow message via a builder closure (exactly the `celnet-fix`
//! [`Session::send_app`] idiom, and the same shape as `celnet-rfq`'s
//! `FixLpAdapter`); the driver owns the Logon → app-flow → Logout IO loop and
//! auto-passes certification checks as the matching frames fly past.

use std::time::Duration;

use celnet_fix::framing::{FrameCursor, FrameEncoder};
use celnet_fix::messages::{Header, build_logout};
use celnet_fix::session::{InMemoryStore, Role, Session, SessionConfig, SessionState};
use celnet_fix::transport::{FrameReader, write_frame};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::cert::CertLedger;
use crate::descriptor::WireDirection;

/// How long to wait for the next inbound frame before treating the peer as quiet
/// and moving on. A slow/flaky venue leaves the affected checks `Pending` rather
/// than erroring — mirroring `celnet-rfq`'s `FixLpAdapter` no-quote-not-error
/// policy.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Endpoint + identity for an outbound (initiator) venue session. Generalises
/// `celnet-rfq`'s per-vendor `FixLpConfig` for the connectivity control-plane.
#[derive(Debug, Clone)]
pub struct VenueEndpoint {
    /// Our `SenderCompID(49)`.
    pub sender_comp_id: Vec<u8>,
    /// The counterparty's `TargetCompID(56)`.
    pub target_comp_id: Vec<u8>,
    /// `HeartBtInt(108)` in seconds.
    pub heartbeat_secs: u32,
    /// `SendingTime(52)` bytes (the caller's clock).
    pub sending_time: Vec<u8>,
}

/// Outcome of a certification cycle.
#[derive(Debug, Clone, Default)]
pub struct CycleReport {
    /// Number of inbound frames observed.
    pub frames_observed: usize,
    /// Certification check keys that auto-passed during the cycle, in order.
    pub flipped: Vec<&'static str>,
}

/// Drive one certification cycle over `stream`: Logon → send the caller's
/// app-flow request → observe the response → Logout, feeding every wire event
/// into `ledger`.
///
/// `app_msg_type` is the FIX `MsgType` we send (for the outbound cert record);
/// `build_app` lays that message onto the wire via a `celnet-fix` builder.
///
/// # Errors
/// Propagates socket IO errors. A non-responding peer is **not** an error: the
/// affected checks simply stay `Pending` and the cycle returns what it observed.
pub async fn run_cert_cycle<RW, F>(
    endpoint: &VenueEndpoint,
    stream: RW,
    ledger: &mut CertLedger,
    app_msg_type: &str,
    build_app: F,
) -> std::io::Result<CycleReport>
where
    RW: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce(&Header<'_>, &mut FrameEncoder) -> Vec<u8>,
{
    let cfg = SessionConfig {
        sender: endpoint.sender_comp_id.clone(),
        target: endpoint.target_comp_id.clone(),
        heart_bt_int: endpoint.heartbeat_secs,
        role: Role::Initiator,
    };
    let mut session = Session::new(cfg, InMemoryStore::new());
    let st: &[u8] = &endpoint.sending_time;
    let (rd, mut wr) = tokio::io::split(stream);
    let mut reader = FrameReader::new(rd);
    let mut report = CycleReport::default();

    // Phase 1 — Logon.
    let logon = session.start_logon(st, true);
    write_frame(&mut wr, &logon).await?;
    record(ledger, &mut report, WireDirection::Outbound, "A");

    // Phase 2 — pump inbound until the session is Active.
    while session.state() != SessionState::Active {
        let Some(raw) = read_next(&mut reader).await? else {
            break;
        };
        pump_inbound(&mut session, &mut wr, &raw, st, ledger, &mut report).await?;
    }
    if session.state() != SessionState::Active {
        // Handshake never completed — return what we observed; checks reflect it.
        return Ok(report);
    }

    // Phase 3 — send the caller's app-flow request.
    let app = session.send_app(st, build_app);
    write_frame(&mut wr, &app).await?;
    record(ledger, &mut report, WireDirection::Outbound, app_msg_type);

    // Phase 4 — observe until an application response is delivered (bounded).
    loop {
        let Some(raw) = read_next(&mut reader).await? else {
            break;
        };
        let delivered = pump_inbound(&mut session, &mut wr, &raw, st, ledger, &mut report).await?;
        if delivered {
            break;
        }
    }

    // Phase 5 — Logout.
    let logout = session.send_app(st, |h, e| build_logout(h, None, e));
    write_frame(&mut wr, &logout).await?;
    record(ledger, &mut report, WireDirection::Outbound, "5");

    // Phase 6 — drain to teardown (bounded).
    while session.state() != SessionState::Disconnected {
        let Some(raw) = read_next(&mut reader).await? else {
            break;
        };
        pump_inbound(&mut session, &mut wr, &raw, st, ledger, &mut report).await?;
    }

    Ok(report)
}

/// Read the next frame, treating a `READ_TIMEOUT` lull as end-of-stream.
async fn read_next<R: AsyncRead + Unpin>(
    reader: &mut FrameReader<R>,
) -> std::io::Result<Option<Vec<u8>>> {
    match tokio::time::timeout(READ_TIMEOUT, reader.next_frame()).await {
        Ok(res) => res,
        Err(_elapsed) => Ok(None),
    }
}

/// Feed one inbound frame into the ledger + session, flush any session-generated
/// outbound (Logon ack, Heartbeat, Logout ack). Returns whether the frame was an
/// application message (the session delivered it upward).
async fn pump_inbound<W: AsyncWrite + Unpin>(
    session: &mut Session<InMemoryStore>,
    wr: &mut W,
    raw: &[u8],
    sending_time: &[u8],
    ledger: &mut CertLedger,
    report: &mut CycleReport,
) -> std::io::Result<bool> {
    report.frames_observed += 1;
    let mt_bytes = FrameCursor::parse(raw)
        .map(|c| c.msg_type().to_vec())
        .unwrap_or_default();
    let mt = std::str::from_utf8(&mt_bytes).unwrap_or_default();
    record(ledger, report, WireDirection::Inbound, mt);
    match session.on_inbound(raw, sending_time) {
        Ok(action) => {
            for out in &action.outbound {
                write_frame(wr, out).await?;
            }
            Ok(action.deliver.is_some())
        }
        // A malformed / out-of-state frame ends the cycle cleanly.
        Err(_) => Ok(false),
    }
}

fn record(ledger: &mut CertLedger, report: &mut CycleReport, dir: WireDirection, mt: &str) {
    if let Some(key) = ledger.record_event(dir, mt) {
        report.flipped.push(key);
    }
}
