//! Resource caps for the WebSocket edge transport.
//!
//! The WS mirror serves untrusted network peers, so the transport must bound the
//! memory any single connection can pin. Left at the tungstenite defaults a peer
//! may stream a **64 MiB** message (16 MiB per frame) and the edge will buffer
//! all of it before the contract layer ever sees a byte — an IB-sized fleet of
//! such sockets is an allocation amplifier. This module is the one seam where
//! the WS transport bounds live; [`super`] applies them on the accept path and
//! enforces the contract cap with a typed refusal.
//!
//! # Sizing (measured against the contract corpus)
//!
//! Every legitimate frame is one type-tagged JSON message mirroring a
//! `celnet_proto` message ([`super::codec`]). The widest shapes in the golden
//! corpus and the live contract are:
//!
//! * the largest golden-corpus instrument frame (a fully-specified `vanilla` /
//!   multi-leg `strategy` entry with market + conventions,
//!   `crates/celnet-golden/vectors/`) — ≈ 5 KiB pretty-printed, less compact;
//! * a full `mark_surface` mark — a ~20-pillar tenor ladder of per-tenor
//!   ATM/RR/BF broker-quote sets at ≈ 150 B each — ≈ 4 KiB;
//! * the widest panel frame — a `multi_dealer_quote` of ~100 ranked dealer
//!   lines at ≈ 300 B each — ≈ 30 KiB.
//!
//! [`MAX_CONTRACT_MESSAGE_BYTES`] (1 MiB) is therefore ≥ 30× the most generous
//! legitimate frame while remaining 64× below the tungstenite default.
//!
//! # Two tiers: graceful contract cap + hard transport backstop
//!
//! A graceful, typed refusal must *assemble* the oversize message: tungstenite's
//! own capacity error aborts mid-payload, leaving the protocol parser pointed at
//! raw payload bytes, so the connection can only be torn down — and the typed
//! close then races the TCP reset from unread inbound data. The transport
//! backstop ([`TRANSPORT_MESSAGE_CAP_BYTES`], 2× the contract cap) bounds that
//! assembly:
//!
//! * `len ≤ contract cap` — dispatched normally;
//! * `contract cap < len ≤ backstop` — fully read (parser stays coherent), then
//!   refused with a typed `error` frame plus an RFC 6455 §7.4.1 **1009
//!   "Message Too Big"** close, delivered over a clean closing handshake;
//! * `len > backstop` — tungstenite's hard capacity error fires before the
//!   payload is buffered; the edge answers with a best-effort 1009 close and
//!   drops the connection. Memory stays bounded at the backstop regardless.

use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::error::CapacityError;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};

/// The largest assembled inbound message the contract accepts, in bytes (1 MiB).
///
/// ≥ 30× headroom over the widest measured legitimate frame (see the module
/// sizing notes). A message over this cap is refused with a typed `error` frame
/// and a 1009 close.
pub(super) const MAX_CONTRACT_MESSAGE_BYTES: usize = 1 << 20;

/// The hard transport bound on one message *and* one frame, in bytes (2 MiB —
/// 2× the contract cap).
///
/// Messages between the contract cap and this backstop are fully assembled so
/// they can be refused gracefully (typed `error` + clean 1009 close); anything
/// larger trips tungstenite's capacity error before the payload is buffered.
/// This is the per-connection inbound memory ceiling.
pub(super) const TRANSPORT_MESSAGE_CAP_BYTES: usize = MAX_CONTRACT_MESSAGE_BYTES * 2;

/// The cap on the in-flight outbound write buffer, in bytes (4 MiB).
///
/// The buffer only grows past tungstenite's 128 KiB write target while writes
/// to the socket are failing, so this bounds what a peer with a broken/stalled
/// socket can pin: the 128 KiB target plus several maximum-size outbound frames
/// (tungstenite requires headroom of at least one message above the target).
/// On overflow the queued send fails and the writer task ends the connection —
/// bounded-memory backpressure instead of unbounded buffering.
pub(super) const MAX_WRITE_BUFFER_BYTES: usize = 4 << 20;

/// The explicit transport bounds applied on the accept path — never the
/// tungstenite defaults (64 MiB message / 16 MiB frame / unbounded write
/// buffer).
pub(super) fn transport_config() -> WebSocketConfig {
    WebSocketConfig {
        max_message_size: Some(TRANSPORT_MESSAGE_CAP_BYTES),
        max_frame_size: Some(TRANSPORT_MESSAGE_CAP_BYTES),
        max_write_buffer_size: MAX_WRITE_BUFFER_BYTES,
        ..WebSocketConfig::default()
    }
}

/// The typed in-contract refusal text for an oversize message: names the
/// offending size and the documented cap so the caller can act on it.
pub(super) fn oversize_reject_text(message_bytes: usize) -> String {
    format!(
        "message of {message_bytes} bytes exceeds the {MAX_CONTRACT_MESSAGE_BYTES}-byte WS edge \
         message cap; no contract frame legitimately approaches this bound — reduce the payload"
    )
}

/// The typed RFC 6455 §7.4.1 close (1009 "Message Too Big") for an oversize
/// message. A close frame is a CONTROL frame: its payload is capped at 125
/// bytes (2-byte code + ≤123-byte reason, RFC 6455 §5.5) — a longer reason
/// makes the close itself malformed (`ControlFrameTooBig` at the peer) and
/// breaks the clean closing handshake. The reason therefore carries only the
/// compact size/cap facts; the FULL explanatory text travels in the preceding
/// typed `error` DATA frame ([`oversize_reject_text`]), which has no such cap.
pub(super) fn oversize_close(message_bytes: usize) -> CloseFrame<'static> {
    let reason = format!("{message_bytes} bytes > {MAX_CONTRACT_MESSAGE_BYTES}-byte message cap");
    debug_assert!(
        reason.len() <= 123,
        "close reason must fit the RFC 6455 control-frame budget: {reason}"
    );
    CloseFrame {
        code: CloseCode::Size,
        reason: reason.into(),
    }
}

/// Map a transport read error to the typed 1009 close if it is the hard
/// capacity backstop tripping (the peer blew past [`TRANSPORT_MESSAGE_CAP_BYTES`]);
/// any other read error is an ordinary disconnect with no close to send.
pub(super) fn backstop_close(error: &WsError) -> Option<CloseFrame<'static>> {
    match error {
        WsError::Capacity(CapacityError::MessageTooLong { size, .. }) => {
            Some(oversize_close(*size))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use futures_util::{SinkExt, StreamExt};
    use serde_json::{Value, json};
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use tokio_tungstenite::tungstenite::error::CapacityError;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

    use celnet_types::{CcyPair, Tenor};

    use super::*;
    use crate::clock::Clock;
    use crate::core_link::CoreLink;
    use crate::readiness::ReadinessGate;
    use crate::services::quote::LpPanelConfig;
    use crate::services::risk::RiskEdge;
    use crate::services::risk::store::PositionStore;
    use crate::spread::SpreadModel;
    use crate::surface_book::SurfaceBook;
    use crate::ws::{WsMirror, WsServices};

    /// The hard wall-clock ceiling for a test body — a regression fails fast.
    const TEST_DEADLINE: Duration = Duration::from_secs(10);

    /// A bound on any single socket send/receive so a never-arriving frame
    /// fails fast rather than hanging.
    const STEP: Duration = Duration::from_secs(5);

    /// Start a ready WS mirror over the calibrated EURUSD engine fixture — the
    /// same real service set [`crate::Edge`] wires (in-process fleet, native
    /// panel only), bound on an ephemeral loopback port.
    async fn start_ready_mirror() -> WsMirror {
        let conv = celnet_conventions::resolve(
            CcyPair::parse("EURUSD").expect("EURUSD parses"),
            Tenor::Years(1),
        )
        .record;
        let initial = celnet_engine::testing::make_state(1.10, conv);
        let link = CoreLink::start(initial, None);
        let gate = Arc::new(ReadinessGate::new());
        assert!(gate.mark_ready(), "a fresh gate promotes to ready");
        let store = Arc::new(PositionStore::new());
        let risk = Arc::new(RiskEdge::new(Arc::clone(&store), Arc::clone(&gate)));
        let surface_book = Arc::new(SurfaceBook::new());
        // An empty managed-acceptor registry over a throwaway config path: the cap
        // test never exercises fix-admin, it just needs the edge the WS set requires.
        let fix_monitor = Arc::new(crate::services::fix_monitor::FixMonitor::new());
        let fix_registry = Arc::new(
            crate::services::fix_registry::FixAcceptorRegistry::load(
                Arc::clone(&link),
                SpreadModel::default(),
                Clock::system(),
                Arc::clone(&surface_book),
                Arc::clone(&fix_monitor),
                Arc::clone(&store),
                None,
                std::env::temp_dir().join(format!("celnet-ws-cap-fix-{}.json", std::process::id())),
            )
            .expect("a missing config loads as an empty registry"),
        );
        let fix_admin = Arc::new(crate::services::fix_admin::FixAdminEdge::new(
            fix_registry,
            Arc::clone(&gate),
            Arc::clone(&store),
            fix_monitor,
        ));
        // A throwaway seeded identity store + session registry: the cap test never
        // exercises auth, it just needs the edge the WS set now requires. The SAME
        // registry is shared into the WS set (the quote gate now requires it).
        let mut identity = crate::config::identity::IdentityStore::default();
        identity.ensure_seed_admin().expect("seed admin hashes");
        let sessions = Arc::new(crate::services::sessions::SessionRegistry::new(
            Clock::system(),
        ));
        let auth = Arc::new(crate::services::auth::AuthEdge::new(
            Arc::new(std::sync::Mutex::new(identity)),
            std::env::temp_dir().join(format!(
                "celnet-ws-cap-identity-{}.json",
                std::process::id()
            )),
            Arc::clone(&sessions),
            Arc::clone(&gate),
            Clock::system(),
        ));
        // A throwaway desk edge over fresh stores: the cap test never exercises the
        // dealer-quoting desk, it just needs the edge the WS set now requires.
        let rfq_desk = Arc::new(crate::services::desk::RfqDeskEdge::new(
            Arc::clone(&store),
            Arc::new(crate::services::sessions::SessionRegistry::new(
                Clock::system(),
            )),
            Arc::clone(&gate),
            Arc::new(crate::services::desk::store::DeskRequestStore::new()),
            Arc::new(crate::services::desk::store::DealStore::new()),
            Arc::new(crate::services::rates_book::RatesPositionStore::new()),
            Arc::new(crate::services::desk::notify::NotificationBroker::new()),
            Clock::system(),
        ));
        let services = WsServices::new(
            link,
            gate,
            SpreadModel::default(),
            Clock::system(),
            surface_book,
            store,
            sessions,
            risk,
            fix_admin,
            auth,
            rfq_desk,
            None,
            LpPanelConfig { synthetic_lps: 0 },
            crate::services::aggregation::AggregationHub::new(Clock::system()),
        );
        WsMirror::start(
            "127.0.0.1:0".parse().expect("loopback bind addr parses"),
            services,
        )
        .await
        .expect("the WS mirror binds an ephemeral port")
    }

    /// A syntactically valid contract frame padded to exactly `total_bytes` —
    /// an unknown `type` so the dispatcher answers it with a typed `error`
    /// frame, proving the message traversed the transport caps end-to-end.
    fn probe_frame(total_bytes: usize) -> String {
        let envelope = r#"{"type":"cap_probe","pad":""}"#;
        let pad = total_bytes
            .checked_sub(envelope.len())
            .expect("probe budget exceeds the envelope");
        let frame = format!(r#"{{"type":"cap_probe","pad":"{}"}}"#, "x".repeat(pad));
        assert_eq!(frame.len(), total_bytes, "probe pads to the exact size");
        frame
    }

    /// Receive the next WS message within the step deadline.
    async fn next_message<S>(ws: &mut S) -> WsMessage
    where
        S: StreamExt<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>> + Unpin,
    {
        tokio::time::timeout(STEP, ws.next())
            .await
            .expect("a WS message arrives before the deadline")
            .expect("the socket stays open")
            .expect("the message is well-formed")
    }

    /// Receive the next JSON text frame within the step deadline.
    async fn next_json<S>(ws: &mut S) -> Value
    where
        S: StreamExt<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>> + Unpin,
    {
        match next_message(ws).await {
            WsMessage::Text(t) => serde_json::from_str(&t).expect("frame is valid JSON"),
            other => panic!("expected a text frame, got: {other:?}"),
        }
    }

    /// Send a WS text frame within the step deadline.
    async fn send_text<S>(ws: &mut S, text: String)
    where
        S: SinkExt<WsMessage, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
    {
        tokio::time::timeout(STEP, ws.send(WsMessage::Text(text)))
            .await
            .expect("the send completes in time")
            .expect("the send succeeds");
    }

    /// A corpus-shaped `price` frame (the golden-corpus vanilla entry as the WS
    /// JSON contract carries it) prices normally under the caps, and a probe at
    /// exactly the contract cap still traverses the transport and reaches the
    /// dispatcher — the cap admits every legitimate frame with full headroom.
    #[tokio::test]
    async fn corpus_frame_and_cap_boundary_frame_pass() {
        tokio::time::timeout(TEST_DEADLINE, async {
            let mirror = start_ready_mirror().await;
            let (mut ws, _resp) =
                tokio::time::timeout(STEP, connect_async(format!("ws://{}", mirror.addr())))
                    .await
                    .expect("WS connects in time")
                    .expect("WS connects");

            // The corpus-shaped frame: a 1Y EURUSD vanilla call with explicit
            // market + conventions — the same shape the golden vectors carry.
            let price_frame = json!({
                "type": "price",
                "request_id": 7,
                "instrument": {
                    "pair": { "base": "EUR", "quote": "USD" },
                    "tenor": { "unit": 3, "count": 1 },
                    "expiry_years": 1.0,
                    "quantity": { "notional": 1_000_000.0, "base_ccy": true },
                    "side": 2,
                    "vanilla": { "option_type": 0, "strike": { "strike": 1.12 } }
                },
                "market": { "spot": 1.10, "vol": 0.105, "r_dom": 0.015, "r_for": 0.0035 },
                "conventions": {
                    "delta_convention": 0,
                    "atm_convention": 0,
                    "premium_style": 0,
                    "cut": 0,
                    "day_count": 0,
                    "settlement": 0
                }
            });
            send_text(&mut ws, price_frame.to_string()).await;
            let reply = next_json(&mut ws).await;
            assert_eq!(reply["type"], "price_response", "frame priced: {reply}");
            let price = reply["greeks"]["price"]
                .as_f64()
                .expect("the response carries a numeric price");
            assert!(
                price.is_finite() && price > 0.0,
                "an OTM call has a positive finite premium, got {price}"
            );

            // A frame at exactly the contract cap reaches the dispatcher (the
            // typed unknown-type error proves it was fully read and parsed).
            send_text(&mut ws, probe_frame(MAX_CONTRACT_MESSAGE_BYTES)).await;
            let reply = next_json(&mut ws).await;
            assert_eq!(reply["type"], "error", "boundary frame dispatched: {reply}");
            assert!(
                reply["message"]
                    .as_str()
                    .expect("error frames carry a message")
                    .contains("cap_probe"),
                "the dispatcher saw the probe's type tag: {reply}"
            );

            let _ = tokio::time::timeout(STEP, ws.close(None)).await;
            mirror.abort();
        })
        .await
        .expect("test completes within the deadline");
    }

    /// A message one byte over the contract cap is refused with the typed
    /// `error` frame naming the cap, followed by the RFC 6455 1009
    /// "Message Too Big" close — delivered over a clean closing handshake
    /// (the backstop assembled the message, so the parser never broke).
    #[tokio::test]
    async fn frame_over_the_cap_gets_a_typed_error_and_a_1009_close() {
        tokio::time::timeout(TEST_DEADLINE, async {
            let mirror = start_ready_mirror().await;
            let (mut ws, _resp) =
                tokio::time::timeout(STEP, connect_async(format!("ws://{}", mirror.addr())))
                    .await
                    .expect("WS connects in time")
                    .expect("WS connects");

            let oversize = MAX_CONTRACT_MESSAGE_BYTES + 1;
            send_text(&mut ws, probe_frame(oversize)).await;

            let reply = next_json(&mut ws).await;
            assert_eq!(reply["type"], "error", "typed refusal first: {reply}");
            let message = reply["message"]
                .as_str()
                .expect("error frames carry a message");
            assert!(
                message.contains(&oversize.to_string())
                    && message.contains(&MAX_CONTRACT_MESSAGE_BYTES.to_string()),
                "the refusal names the offending size and the cap: {message}"
            );

            match next_message(&mut ws).await {
                WsMessage::Close(Some(frame)) => {
                    assert_eq!(frame.code, CloseCode::Size, "1009 Message Too Big");
                    assert!(
                        frame
                            .reason
                            .contains(&MAX_CONTRACT_MESSAGE_BYTES.to_string()),
                        "the close reason names the cap: {}",
                        frame.reason
                    );
                }
                other => panic!("expected the typed 1009 close, got: {other:?}"),
            }

            mirror.abort();
        })
        .await
        .expect("test completes within the deadline");
    }

    /// The hard transport backstop maps to the same typed 1009 close (the
    /// `> 2× cap` tear-down path), and an ordinary disconnect maps to none.
    #[test]
    fn backstop_error_maps_to_the_typed_1009_close() {
        let error = WsError::Capacity(CapacityError::MessageTooLong {
            size: TRANSPORT_MESSAGE_CAP_BYTES + 1,
            max_size: TRANSPORT_MESSAGE_CAP_BYTES,
        });
        let close = backstop_close(&error).expect("the capacity backstop yields a typed close");
        assert_eq!(close.code, CloseCode::Size);
        assert!(
            close
                .reason
                .contains(&(TRANSPORT_MESSAGE_CAP_BYTES + 1).to_string()),
            "the close names the offending size: {}",
            close.reason
        );
        assert!(
            backstop_close(&WsError::ConnectionClosed).is_none(),
            "an ordinary disconnect carries no oversize close"
        );
    }

    /// The documented cap ordering the two-tier design relies on: contract cap
    /// strictly inside the transport backstop, write buffer above tungstenite's
    /// 128 KiB target plus at least one maximum-size outbound message.
    #[test]
    fn cap_ordering_holds() {
        let config = transport_config();
        assert_eq!(config.max_message_size, Some(TRANSPORT_MESSAGE_CAP_BYTES));
        assert_eq!(config.max_frame_size, Some(TRANSPORT_MESSAGE_CAP_BYTES));
        assert_eq!(config.max_write_buffer_size, MAX_WRITE_BUFFER_BYTES);
        // Compile-time law: the largest legitimate contract frame must fit under the
        // transport cap with headroom (a const relation — checked statically so the
        // cap can never be tightened below the contract's own maximum).
        const _CONTRACT_FITS_UNDER_CAP: () =
            assert!(MAX_CONTRACT_MESSAGE_BYTES < TRANSPORT_MESSAGE_CAP_BYTES);
        assert!(MAX_WRITE_BUFFER_BYTES > config.write_buffer_size + MAX_CONTRACT_MESSAGE_BYTES);
    }
}
