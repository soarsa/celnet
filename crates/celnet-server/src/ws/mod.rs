//! The WebSocket JSON mirror of the gRPC contract.
//!
//! A browser or any WebSocket client gets the **same single, current contract**
//! over WS that gRPC clients get over protobuf (`CLAUDE.md` rule 9: one contract,
//! no fork). The mirror is a second *encoding* — type-tagged JSON ([`codec`]) — of
//! the exact same [`celnet_proto`] messages, dispatched onto the **same** service
//! edges (`PricingEdge` / `QuoteEdge` / `StreamEdge` / `SurfaceEdge`) the gRPC
//! server hosts, driven by the **same** [`crate::core_link::CoreLink`] and shared
//! [`crate::surface_book::SurfaceBook`]. There is no second pricing path: a WS quote
//! is byte-for-byte the gRPC/direct price because it runs the identical code.
//!
//! # Framing
//!
//! Every client→server and server→client message is a single JSON text frame with a
//! `"type"` discriminator naming the operation (mirroring the gRPC method / stream
//! `oneof` variant):
//!
//! * **RFQ** — `request_quote` → `quote`, `request_multi_dealer_quote` →
//!   `multi_dealer_quote` (the ranked LP panel; the matching `accept_quote` may
//!   carry a panel row's `lp_id`), `accept_quote` → `execution`,
//!   `reject_quote` → `reject_ack`;
//! * **pricing** — `price` → `price_response`, `price_rates` → `rates_price_response`
//!   (the fixed-income linear-rates mirror of `PricingService::PriceRates`),
//!   `price_xva` → `price_xva_response` (the CVA/DVA/FVA valuation-adjustment
//!   mirror of `PricingService::PriceXva`);
//! * **surface** — `get_smile` → `smile`, `mark_surface` → `mark_surface_response`,
//!   `scenario` → `scenario_response`;
//! * **RFS** — `subscribe` / `modify` / `unsubscribe` / `resync` / `execute` /
//!   `heartbeat` drive the multiplexed [`crate::services::stream`] session, which
//!   emits `snapshot` / `update` / `heartbeat` / `stream_end` / `executed` /
//!   `stream_reject` frames. A WS connection IS one multiplexed RFS session, exactly
//!   like one gRPC `StreamSession` channel — so the same connection also serves
//!   request/response RFQ / pricing / surface calls inline.
//!
//! A malformed or out-of-contract frame is answered with a typed `error` frame
//! (echoing any `correlation_id`); it never tears the connection down.
//!
//! # Entitlements trust boundary (WS = same boundary as gRPC)
//!
//! The risk frames (`list_positions` / `aggregate_risk` / `drill_risk` /
//! `limit_status`) dispatch onto the **same** `RiskService` trait methods the
//! gRPC server hosts, so the deny-by-default authorization boundary
//! ([`crate::services::access`]) covers both encodings with one decision + one
//! audit record per request: a frame asserting **no** entitlement principal is
//! refused with a typed `error` frame carrying code `Unauthenticated` (the
//! `status_error` mapping), unless the edge runs the explicit, loudly-banner'd
//! permissive dev-mode. Transport-level authentication of the WS peer (TLS /
//! an authenticating gateway binding the asserted principal to a caller) is
//! deployment configuration — in-repo, the WS entry enforces the authorization
//! *decision* boundary, identically to gRPC.
//!
//! # Resource caps
//!
//! The accept path applies the explicit transport bounds in [`limits`] — never
//! the tungstenite defaults (64 MiB message / 16 MiB frame / unbounded write
//! buffer). A message over the 1 MiB contract cap is refused with a typed
//! `error` frame plus an RFC 6455 1009 "Message Too Big" close; a peer blowing
//! past the 2× hard transport backstop is torn down with a best-effort 1009.
//! Per-connection inbound memory is bounded at the backstop, outbound at the
//! write-buffer cap.
//!
//! # Not blocking the pinned core; readiness / drain
//!
//! The WS server runs entirely on the async edge. The RFS session is driven by the
//! same deterministic tick + bounded-ring discipline as gRPC (the pinned hot core is
//! never blocked by a slow WS consumer). New connections are gated by the shared
//! [`crate::readiness::ReadinessGate`]: a draining instance refuses the upgrade so
//! traffic steers to the warm replacement, and each live connection holds an
//! in-flight guard so the existing graceful drain ([`crate::Edge::shutdown`]) waits
//! for WS sessions exactly as it waits for gRPC calls.

pub mod codec;
mod codec_overrides;
pub mod generated_codec;
mod limits;

use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tonic::Request;

use celnet_proto::auth_service_server::AuthService;
use celnet_proto::fix_admin_service_server::FixAdminService;
use celnet_proto::pricing_service_server::PricingService;
use celnet_proto::quote_service_server::QuoteService;
use celnet_proto::rfq_desk_service_server::RfqDeskService;
use celnet_proto::risk_service_server::RiskService;
use celnet_proto::surface_service_server::SurfaceService;
use celnet_proto::{ClientStreamMessage, ServerStreamMessage, client_stream_message};

use crate::clock::Clock;
use crate::core_link::CoreLink;
use crate::readiness::{InFlightGuard, ReadinessGate};
use crate::services::auth::AuthEdge;
use crate::services::desk::RfqDeskEdge;
use crate::services::desk::notify::NotificationBroker;
use crate::services::fix_admin::FixAdminEdge;
use crate::services::pricing::PricingEdge;
use crate::services::quote::{LpPanelConfig, QuoteEdge};
use crate::services::risk::RiskEdge;
use crate::services::risk::store::PositionStore;
use crate::services::sessions::SessionRegistry;
use crate::services::stream::{StreamEdge, run_session};
use crate::services::surface::SurfaceEdge;
use crate::spread::SpreadModel;
use crate::surface_book::SurfaceBook;

/// The bounded depth of the per-connection server→client message channel. Matches
/// the gRPC RFS channel depth so a lagging WS consumer is paused (LAGGED) rather
/// than back-pressuring the shared driver — one slow socket never stalls the core.
const WS_CHANNEL_DEPTH: usize = 256;

/// One transmission queued to the per-connection writer task.
enum Outbound {
    /// A type-tagged contract frame, serialized to one WS text message.
    Frame(Value),
    /// A typed terminal close (e.g. the RFC 6455 1009 oversize refusal from
    /// [`limits`]); the writer sends it and ends the connection.
    Close(CloseFrame<'static>),
}

/// The shared, transport-neutral service set every WS connection dispatches onto —
/// the **same** edges the gRPC server hosts, behind one [`CoreLink`] / one
/// [`SurfaceBook`]. Cheap to clone behind [`Arc`]s; one instance backs the whole WS
/// listener.
#[derive(Clone)]
pub struct WsServices {
    pricing: Arc<PricingEdge>,
    quote: Arc<QuoteEdge>,
    stream: Arc<StreamEdge>,
    surface: Arc<SurfaceEdge>,
    risk: Arc<RiskEdge>,
    fix_admin: Arc<FixAdminEdge>,
    auth: Arc<AuthEdge>,
    /// The dealer-quoting desk edge (RFQ/IOI capture + response + accept + reads) and
    /// the publisher behind the notification push channel.
    rfq_desk: Arc<RfqDeskEdge>,
    gate: Arc<ReadinessGate>,
}

impl WsServices {
    /// Construct the WS service set from the same shared components the gRPC edge is
    /// built from, so both fronts speak one contract over one pricing path — now
    /// including `RiskService` over the same shared live position book and the shared
    /// backend [`Fleet`](crate::services::risk::federate::Fleet) for owned-pair
    /// forwarding.
    #[must_use]
    #[allow(clippy::too_many_arguments)] // the shared component set the edge is built from.
    pub fn new(
        link: Arc<CoreLink>,
        gate: Arc<ReadinessGate>,
        spread: SpreadModel,
        clock: Clock,
        surface_book: Arc<SurfaceBook>,
        store: Arc<PositionStore>,
        sessions: Arc<SessionRegistry>,
        risk: Arc<RiskEdge>,
        fix_admin: Arc<FixAdminEdge>,
        auth: Arc<AuthEdge>,
        rfq_desk: Arc<RfqDeskEdge>,
        fleet: Option<Arc<crate::services::risk::federate::Fleet>>,
        panel: LpPanelConfig,
    ) -> Self {
        // The SAME shared backend fleet the gRPC edges use (or `None` in-process), so
        // the WS unary mirror forwards owned-pair requests identically (API-first
        // parity). The WS RFS path drives the local session loop (`session_driver`);
        // the implemented distributed stream relay is the gRPC `StreamSession` path
        // (`crate::services::stream::stream_session`) — the WS stream relay is a
        // parallel refinement, not faked here.
        let pricing = Arc::new(PricingEdge::with_fleet(
            Arc::clone(&gate),
            Arc::clone(&surface_book),
            fleet.clone(),
        ));
        let quote = Arc::new(
            QuoteEdge::with_fleet(
                Arc::clone(&link),
                Arc::clone(&gate),
                spread,
                clock.clone(),
                Arc::clone(&surface_book),
                fleet.clone(),
                panel,
            )
            // The WS RFQ caller gate (item B §2) shares the SAME session registry +
            // access store the gRPC quote edge uses, so the WS mirror enforces one
            // coherent policy (the caller rides in the unary body — no router change).
            .with_session_access(Arc::clone(&sessions), Arc::clone(&store)),
        );
        let stream = Arc::new(
            StreamEdge::with_store(
                Arc::clone(&link),
                Arc::clone(&gate),
                spread,
                clock.clone(),
                Arc::clone(&surface_book),
                Arc::clone(&store),
            )
            // Install the SAME edge-wide session registry the `auth` edge mints
            // login sessions into (and the WS quote/risk gates resolve against), so
            // a WS `StreamAuth` frame's `session_token` validates against the
            // sessions actually issued by `AuthService.Login`. Without this the WS
            // `StreamEdge` defaulted to a fresh EMPTY registry (`default_sessions`),
            // so under `Enforce` every tokened stream auth was rejected
            // "invalid or expired session token" — closing the connection — and the
            // capability gate (`Stream`/`Execute·FxOptions`, finding #3: caps come
            // only from an authenticated session) could never be satisfied over WS.
            // The gRPC `StreamEdge` was already wired this way (lib.rs); this brings
            // the WS mirror to the same one coherent session policy.
            .with_sessions(Arc::clone(&sessions)),
        );
        let surface = Arc::new(SurfaceEdge::with_fleet(
            Arc::clone(&link),
            Arc::clone(&gate),
            clock,
            surface_book,
            fleet,
        ));
        // `risk` is the SAME connected edge the gRPC server uses (a distributed edge's
        // backend fleet is connected once at boot and shared behind an `Arc`).
        Self {
            pricing,
            quote,
            stream,
            surface,
            risk,
            fix_admin,
            auth,
            rfq_desk,
            gate,
        }
    }
}

/// A running WebSocket mirror: the bound listener address and the accept-loop task
/// handle. Constructed by [`WsMirror::start`]; stopped by dropping the owning
/// [`crate::Edge`] (which aborts the accept loop) after the shared readiness drain.
#[derive(Debug)]
pub struct WsMirror {
    addr: SocketAddr,
    accept_task: tokio::task::JoinHandle<()>,
}

impl WsMirror {
    /// Bind the WS listener on `addr` (pass port `0` for an ephemeral port, read
    /// back via [`WsMirror::addr`]) and spawn the accept loop over the shared
    /// services. Each accepted connection becomes one multiplexed session task.
    ///
    /// # Errors
    ///
    /// Returns an [`std::io::Error`] if the listener cannot bind.
    pub async fn start(addr: SocketAddr, services: WsServices) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let bound = listener.local_addr()?;
        let accept_task = tokio::spawn(accept_loop(listener, services));
        Ok(Self {
            addr: bound,
            accept_task,
        })
    }

    /// The actually-bound WS socket address (resolves an ephemeral `:0` port).
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Abort the accept loop (live connections finish under the readiness drain
    /// before the owning edge stops the runtime).
    pub fn abort(&self) {
        self.accept_task.abort();
    }
}

/// Accept WebSocket connections until the listener is dropped, spawning one session
/// task per connection.
async fn accept_loop(listener: TcpListener, services: WsServices) {
    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                let services = services.clone();
                tokio::spawn(async move {
                    serve_connection(stream, services).await;
                });
            }
            // A transient accept error (e.g. fd exhaustion) should not kill the
            // listener; yield and retry. A permanently-closed listener returns
            // errors continuously, but the task is aborted on edge shutdown.
            Err(_) => {
                tokio::task::yield_now().await;
            }
        }
    }
}

/// Serve one WebSocket connection: complete the upgrade, gate on readiness, then
/// run the multiplexed session loop (request/response calls inline + the RFS driver).
async fn serve_connection(tcp: TcpStream, services: WsServices) {
    // Accept under the explicit transport caps ([`limits`]) — never the
    // tungstenite defaults — so one connection's inbound/outbound buffering is
    // hard-bounded before the first frame is read.
    let ws =
        match tokio_tungstenite::accept_async_with_config(tcp, Some(limits::transport_config()))
            .await
        {
            Ok(ws) => ws,
            Err(_) => return, // not a valid WebSocket handshake; drop.
        };

    // Gate new connections on readiness, and hold an in-flight guard for the whole
    // connection so the graceful drain waits for live WS sessions just like gRPC.
    let guard: InFlightGuard = services.gate.enter();
    if !services.gate.is_ready() {
        let mut ws = ws;
        let _ = ws
            .send(WsMessage::Text(
                codec::error_frame(
                    "edge not ready (starting or draining); steer to the active instance",
                    None,
                )
                .to_string(),
            ))
            .await;
        let _ = ws.close(None).await;
        return;
    }

    let (mut ws_tx, mut ws_rx) = ws.split();

    // The outbound server→client channel: every reply and every RFS server message
    // is queued here and pumped to the socket by a dedicated writer task, so the
    // request/response path and the RFS driver share one ordered, bounded outbound
    // sink without either blocking the other.
    let (out_tx, mut out_rx) = mpsc::channel::<Outbound>(WS_CHANNEL_DEPTH);

    // The RFS session is driven lazily: the connection is also an RFS session, so we
    // bridge inbound stream-control frames into the shared `run_session` driver via
    // an inbound `ClientStreamMessage` channel and a server-message channel that the
    // writer task forwards as JSON.
    let (rfs_in_tx, rfs_in_rx) = mpsc::channel::<ClientStreamMessage>(WS_CHANNEL_DEPTH);
    let (rfs_out_tx, mut rfs_out_rx) =
        mpsc::channel::<Result<ServerStreamMessage, tonic::Status>>(WS_CHANNEL_DEPTH);

    // Start the shared RFS session driver on this connection. It is the SAME driver
    // the gRPC `StreamSession` runs — one streaming/pricing path, one contract.
    let driver = services.stream.session_driver();
    let rfs_in_stream = ClientStreamRx { rx: rfs_in_rx };
    let rfs_task = tokio::spawn(async move {
        run_session(driver, rfs_in_stream, rfs_out_tx).await;
    });

    // The writer task: serialize every outbound frame to a WS text message. Forwards
    // both the request/response replies (`out_rx`) and the RFS server messages
    // (`rfs_out_rx`), closing the socket when both dry up.
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                reply = out_rx.recv() => match reply {
                    Some(Outbound::Frame(v)) => {
                        if ws_tx.send(WsMessage::Text(v.to_string())).await.is_err() {
                            break;
                        }
                    }
                    // A typed terminal close (oversize refusal): send it, then
                    // stop writing — the trailing `close()` completes the
                    // handshake flush.
                    Some(Outbound::Close(frame)) => {
                        let _ = ws_tx.send(WsMessage::Close(Some(frame))).await;
                        break;
                    }
                    None => break,
                },
                server_msg = rfs_out_rx.recv() => match server_msg {
                    Some(Ok(msg)) => {
                        if let Some(v) = codec::server_stream_message_to_json(&msg)
                            && ws_tx.send(WsMessage::Text(v.to_string())).await.is_err()
                        {
                            break;
                        }
                    }
                    Some(Err(status)) => {
                        let frame = codec::error_frame(status.message(), None);
                        if ws_tx.send(WsMessage::Text(frame.to_string())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                },
            }
        }
        let _ = ws_tx.close().await;
    });

    // The per-connection notification subscription (at most one; CLAUDE.md §11
    // bounded-queue offload). Registered on the shared broker when the client sends a
    // `subscribe_notifications` frame, drained by a spawned task onto `out_tx`, and
    // torn down on `unsubscribe_notifications` or socket close.
    let mut conn_notif = ConnNotif::default();

    // The reader loop: dispatch each inbound frame. Request/response calls reply on
    // `out_tx`; stream-control frames are forwarded to the RFS driver via `rfs_in_tx`.
    while let Some(frame) = ws_rx.next().await {
        let msg = match frame {
            Ok(WsMessage::Text(t)) => t,
            Ok(WsMessage::Binary(b)) => match String::from_utf8(b) {
                Ok(t) => t,
                Err(_) => {
                    let _ = out_tx
                        .send(Outbound::Frame(codec::error_frame(
                            "binary frame is not valid UTF-8 JSON",
                            None,
                        )))
                        .await;
                    continue;
                }
            },
            Ok(WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Frame(_)) => continue,
            Ok(WsMessage::Close(_)) => break,
            Err(e) => {
                // The hard transport backstop tripped (> 2× the contract cap):
                // answer with a best-effort typed 1009 close. Any other read
                // error is an ordinary disconnect.
                if let Some(close) = limits::backstop_close(&e) {
                    let _ = out_tx.send(Outbound::Close(close)).await;
                }
                break;
            }
        };
        // The contract cap: a message over the documented bound is refused with
        // a typed `error` frame plus the RFC 6455 1009 close. The transport
        // backstop assembled it in full, so the protocol parser is coherent and
        // the closing handshake is clean (no reset racing the refusal).
        if msg.len() > limits::MAX_CONTRACT_MESSAGE_BYTES {
            let _ = out_tx
                .send(Outbound::Frame(codec::error_frame(
                    &limits::oversize_reject_text(msg.len()),
                    None,
                )))
                .await;
            let _ = out_tx
                .send(Outbound::Close(limits::oversize_close(msg.len())))
                .await;
            break;
        }
        if !dispatch(&services, &msg, &out_tx, &rfs_in_tx, &mut conn_notif).await {
            break;
        }
    }

    // Socket closed / reader ended: deregister any notification subscription so it
    // never leaks (the drain task also self-deregisters when its broker rx closes).
    conn_notif.shutdown(services.rfq_desk.broker());

    // Reader ended: tear the RFS session down (dropping `rfs_in_tx` closes its
    // inbound), let the writer drain, and release the in-flight guard.
    drop(rfs_in_tx);
    drop(out_tx);
    rfs_task.abort();
    let _ = rfs_task.await;
    let _ = writer.await;
    drop(guard);
}

/// Dispatch one decoded JSON frame. Request/response operations reply on `out_tx`;
/// RFS stream-control operations are forwarded to the session driver via `rfs_in_tx`.
/// Returns `false` only on a fatal outbound-channel close (connection gone).
async fn dispatch(
    services: &WsServices,
    raw: &str,
    out_tx: &mpsc::Sender<Outbound>,
    rfs_in_tx: &mpsc::Sender<ClientStreamMessage>,
    conn_notif: &mut ConnNotif,
) -> bool {
    let value: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => {
            return out_tx
                .send(Outbound::Frame(codec::error_frame(
                    &format!("invalid JSON: {e}"),
                    None,
                )))
                .await
                .is_ok();
        }
    };
    let Some(o) = value.as_object() else {
        return out_tx
            .send(Outbound::Frame(codec::error_frame(
                "frame must be a JSON object",
                None,
            )))
            .await
            .is_ok();
    };
    let Some(kind) = o.get("type").and_then(Value::as_str) else {
        return out_tx
            .send(Outbound::Frame(codec::error_frame(
                "frame missing a `type` discriminator",
                None,
            )))
            .await
            .is_ok();
    };
    let correlation_id = o.get("correlation_id").and_then(Value::as_u64);

    // Stream-control frames are forwarded to the shared `run_session` driver via
    // `rfs_in_tx`; everything else is a request/response RPC handled inline. The
    // classification has ONE source of truth — [`is_stream_control`] — so a new
    // stream-control verb added to `decode_stream_control` can never again be
    // forgotten here (the `authenticate` frame was: it fell through to
    // `handle_unary`, the session stayed anonymous, and under `AccessMode::Enforce`
    // the following `subscribe` was rejected `unauthenticated` so no two-way
    // streamed — the WS-mirror counterpart of the gRPC stream's `Authenticate` arm).
    if is_stream_control(kind) {
        match decode_stream_control(kind, o) {
            Ok(msg) => {
                // If the RFS driver has gone, the connection is being torn down.
                rfs_in_tx.send(msg).await.is_ok()
            }
            Err(e) => out_tx
                .send(Outbound::Frame(codec::error_frame(
                    &e.to_string(),
                    correlation_id,
                )))
                .await
                .is_ok(),
        }
    } else if kind == "subscribe_notifications" {
        // ---- notification push: subscribe this connection --------------------
        handle_subscribe_notifications(services, o, out_tx, conn_notif).await
    } else if kind == "unsubscribe_notifications" {
        // ---- notification push: unsubscribe this connection ------------------
        conn_notif.shutdown(services.rfq_desk.broker());
        out_tx
            .send(Outbound::Frame(serde_json::json!({
                "type": "unsubscribe_notifications_response",
                "correlation_id": correlation_id,
            })))
            .await
            .is_ok()
    } else {
        // ---- request/response RPCs: run the same edge, reply inline ----------
        let reply = handle_unary(services, kind, o, correlation_id).await;
        out_tx.send(Outbound::Frame(reply)).await.is_ok()
    }
}

/// One connection's notification subscription: the broker-assigned id and the drain
/// task that forwards bounded-queue notifications onto the connection's outbound sink.
#[derive(Default)]
struct ConnNotif {
    sub: Option<NotifSub>,
}

/// A live per-connection notification subscription handle.
struct NotifSub {
    id: u64,
    task: tokio::task::JoinHandle<()>,
}

impl ConnNotif {
    /// Tear down the current subscription (if any): abort the drain task and
    /// deregister from the broker. Idempotent.
    fn shutdown(&mut self, broker: &Arc<NotificationBroker>) {
        if let Some(sub) = self.sub.take() {
            sub.task.abort();
            broker.unsubscribe(sub.id);
        }
    }
}

/// Handle an inbound `subscribe_notifications` frame: resolve + authorize the caller
/// against the SAME deny-by-default boundary the gRPC `StreamNotifications` uses,
/// register a desk-scoped subscriber on the shared broker, and spawn a task that
/// drains its bounded queue onto THIS connection's outbound sink as
/// `{"type":"notification", …}` frames. A new subscription supersedes any prior one
/// on the connection. Returns `false` only on a fatal outbound-channel close.
async fn handle_subscribe_notifications(
    services: &WsServices,
    o: &Map<String, Value>,
    out_tx: &mpsc::Sender<Outbound>,
    conn_notif: &mut ConnNotif,
) -> bool {
    let correlation_id = o
        .get("correlation_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let req = match codec::stream_notifications_request_from_json(o) {
        Ok(r) => r,
        Err(e) => {
            return out_tx
                .send(Outbound::Frame(codec::error_frame(&e.to_string(), None)))
                .await
                .is_ok();
        }
    };
    // Supersede any prior subscription on this connection (one per connection).
    conn_notif.shutdown(services.rfq_desk.broker());
    let subscription = match services.rfq_desk.subscribe_notifications(&req) {
        Ok(s) => s,
        Err(status) => {
            return out_tx
                .send(Outbound::Frame(status_error(&status, None)))
                .await
                .is_ok();
        }
    };
    let id = subscription.id;
    let mut rx = subscription.rx;
    let drain_tx = out_tx.clone();
    let task = tokio::spawn(async move {
        while let Some(n) = rx.recv().await {
            if drain_tx
                .send(Outbound::Frame(codec::notification_to_json(&n)))
                .await
                .is_err()
            {
                break; // the connection's writer is gone.
            }
        }
    });
    conn_notif.sub = Some(NotifSub { id, task });
    out_tx
        .send(Outbound::Frame(serde_json::json!({
            "type": "subscribe_notifications_response",
            "subscribed": true,
            "correlation_id": correlation_id,
        })))
        .await
        .is_ok()
}

/// Whether a frame `type` is an RFS stream-control verb (forwarded to the shared
/// session driver) rather than a request/response RPC. The SINGLE source of truth
/// for the routing decision in [`dispatch`].
///
/// The verb set is the descriptor-derived [`celnet_proto::wire_contract::STREAM_CONTROL_VERBS`]
/// — the `ClientStreamMessage` oneof arm names, generated at build time from
/// `celnet.proto` (arch item G — `ws-codec-from-proto`). Sourcing it from the
/// schema means the router can no longer drift from the contract: a new oneof arm
/// added to `ClientStreamMessage` is classified here automatically, with no hand
/// edit to this function (the `authenticate` omission — which left WS sessions
/// anonymous under `Enforce` — is now structurally impossible). The matching
/// [`decode_stream_control`] arm is still required, and the lockstep test in this
/// module asserts the decoder keeps up with the generated verb set.
fn is_stream_control(kind: &str) -> bool {
    celnet_proto::wire_contract::STREAM_CONTROL_VERBS.contains(&kind)
}

/// Decode a stream-control frame into the [`ClientStreamMessage`] the shared RFS
/// driver consumes (the same message a gRPC client sends on its `StreamSession`).
fn decode_stream_control(
    kind: &str,
    o: &Map<String, Value>,
) -> Result<ClientStreamMessage, codec::CodecError> {
    let message = match kind {
        "authenticate" => {
            client_stream_message::Message::Authenticate(codec::stream_auth_from_json(o)?)
        }
        "subscribe" => client_stream_message::Message::Subscribe(codec::subscribe_from_json(o)?),
        "rates_subscribe" => {
            client_stream_message::Message::RatesSubscribe(codec::rates_subscribe_from_json(o)?)
        }
        "modify" => client_stream_message::Message::Modify(codec::modify_from_json(o)?),
        "unsubscribe" => {
            client_stream_message::Message::Unsubscribe(codec::unsubscribe_from_json(o)?)
        }
        "resync" => client_stream_message::Message::Resync(codec::resync_from_json(o)?),
        "execute" => client_stream_message::Message::Execute(codec::execute_from_json(o)?),
        "market_series_subscribe" => client_stream_message::Message::MarketSeriesSubscribe(
            codec::market_series_subscribe_from_json(o)?,
        ),
        "market_series_unsubscribe" => client_stream_message::Message::MarketSeriesUnsubscribe(
            codec::market_series_unsubscribe_from_json(o)?,
        ),
        "heartbeat" => client_stream_message::Message::Heartbeat(celnet_proto::Heartbeat {
            subscription: None,
            sequence: 0,
            epoch_nanos: 0,
            ..Default::default()
        }),
        other => {
            return Err(codec::CodecError(format!(
                "unknown stream-control type `{other}`"
            )));
        }
    };
    Ok(ClientStreamMessage {
        message: Some(message),
    })
}

/// Run one request/response RPC against the shared service edge and produce the
/// type-tagged JSON reply (or a typed `error` frame). The call goes through the
/// SAME `tonic` service-trait method the gRPC server invokes — one pricing path.
async fn handle_unary(
    services: &WsServices,
    kind: &str,
    o: &Map<String, Value>,
    correlation_id: Option<u64>,
) -> Value {
    // Map a decode error or a tonic `Status` to a typed `error` frame.
    macro_rules! decode {
        ($e:expr) => {
            match $e {
                Ok(v) => v,
                Err(e) => return codec::error_frame(&e.to_string(), correlation_id),
            }
        };
    }
    macro_rules! call {
        ($fut:expr, $tag:expr, $to_json:expr) => {
            match $fut.await {
                Ok(resp) => codec::tagged($tag, $to_json(&resp.into_inner())),
                Err(status) => status_error(&status, correlation_id),
            }
        };
    }

    match kind {
        // The Instrument-consuming Price family runs on the descriptor-driven
        // generated codec (arch item G — `ws-codec-from-proto`, increment 4): the
        // `ws_codec_differential` harness proves, over the full price/rates/xva
        // conformance corpus, that the generated request-decode + response-encode is
        // byte-identical to the hand codec (retained as the frozen oracle in
        // `codec::diff_support`), so this swap is contract-preserving.
        "price" => {
            let req = decode!(generated_codec::decode_price_request(o));
            call!(
                services.pricing.price(Request::new(req)),
                "price_response",
                generated_codec::encode_price_response
            )
        }
        "price_rates" => {
            let req = decode!(generated_codec::decode_rates_price_request(o));
            call!(
                services.pricing.price_rates(Request::new(req)),
                "rates_price_response",
                generated_codec::encode_rates_price_response
            )
        }
        "price_xva" => {
            let req = decode!(generated_codec::decode_price_xva_request(o));
            call!(
                services.pricing.price_xva(Request::new(req)),
                "price_xva_response",
                generated_codec::encode_price_xva_response
            )
        }
        "request_quote" => {
            let req = decode!(codec::quote_request_from_json(o));
            call!(
                services.quote.request_quote(Request::new(req)),
                "quote",
                codec::quote_to_json
            )
        }
        "request_multi_dealer_quote" => {
            let req = decode!(codec::quote_request_from_json(o));
            call!(
                services.quote.request_multi_dealer_quote(Request::new(req)),
                "multi_dealer_quote",
                codec::multi_dealer_quote_to_json
            )
        }
        "accept_quote" => {
            let req = decode!(codec::quote_accept_from_json(o));
            call!(
                services.quote.accept_quote(Request::new(req)),
                "execution",
                codec::execution_to_json
            )
        }
        "reject_quote" => {
            let req = decode!(codec::quote_reject_from_json(o));
            call!(
                services.quote.reject_quote(Request::new(req)),
                "reject_ack",
                codec::reject_ack_to_json
            )
        }
        "get_smile" => {
            let req = decode!(codec::get_smile_request_from_json(o));
            call!(
                services.surface.get_smile(Request::new(req)),
                "smile",
                codec::smile_reply_to_json
            )
        }
        "mark_surface" => {
            let req = decode!(codec::mark_surface_request_from_json(o));
            call!(
                services.surface.mark_surface(Request::new(req)),
                "mark_surface_response",
                codec::mark_surface_response_to_json
            )
        }
        "scenario" => {
            let req = decode!(codec::scenario_request_from_json(o));
            call!(
                services.surface.scenario(Request::new(req)),
                "scenario_response",
                codec::scenario_response_to_json
            )
        }
        // ---- risk: server-side hierarchical risk over the live book ----------
        "list_positions" => {
            let req = decode!(codec::list_positions_request_from_json(o));
            call!(
                services.risk.list_positions(Request::new(req)),
                "list_positions_response",
                codec::list_positions_response_to_json
            )
        }
        "aggregate_risk" => {
            let req = decode!(codec::aggregate_risk_request_from_json(o));
            call!(
                services.risk.aggregate_risk(Request::new(req)),
                "aggregate_risk_response",
                codec::aggregate_risk_response_to_json
            )
        }
        "aggregate_rates_risk" => {
            let req = decode!(codec::aggregate_rates_risk_request_from_json(o));
            call!(
                services.risk.aggregate_rates_risk(Request::new(req)),
                "aggregate_rates_risk_response",
                codec::aggregate_rates_risk_response_to_json
            )
        }
        // The C2c unified joint options+FI tail-risk cube. Its request/response run on
        // the descriptor-driven generated codec (arch item G — `ws-codec-from-proto`):
        // the new messages carry no FX-legacy quirks, so they decode/encode purely
        // from the field tables, proven round-trip byte-stable by the differential
        // harness (`tests/ws_codec_differential.rs`).
        "combined_tail_risk" => {
            let req = decode!(generated_codec::decode_combined_tail_risk_request(o));
            call!(
                services.risk.combined_tail_risk(Request::new(req)),
                "combined_tail_risk_response",
                generated_codec::encode_combined_tail_risk_response
            )
        }
        "drill_risk" => {
            let req = decode!(codec::drill_risk_request_from_json(o));
            call!(
                services.risk.drill_risk(Request::new(req)),
                "drill_risk_response",
                codec::drill_risk_response_to_json
            )
        }
        "limit_status" => {
            let req = decode!(codec::limit_status_request_from_json(o));
            call!(
                services.risk.limit_status(Request::new(req)),
                "limit_status_response",
                codec::limit_status_response_to_json
            )
        }
        // ---- linear-rates Book/List (RiskService rates positions) ------------
        "book_rates_position" => {
            let req = decode!(codec::book_rates_position_request_from_json(o));
            call!(
                services.risk.book_rates_position(Request::new(req)),
                "book_rates_position_response",
                codec::book_rates_position_response_to_json
            )
        }
        "list_rates_positions" => {
            let req = decode!(codec::list_rates_positions_request_from_json(o));
            call!(
                services.risk.list_rates_positions(Request::new(req)),
                "list_rates_positions_response",
                codec::list_rates_positions_response_to_json
            )
        }
        // ---- dealer-quoting desk (RfqDeskService) ----------------------------
        "submit_desk_request" => {
            let req = decode!(codec::submit_desk_request_from_json(o));
            call!(
                services.rfq_desk.submit_desk_request(Request::new(req)),
                "submit_desk_request_response",
                codec::submit_desk_request_response_to_json
            )
        }
        "respond_desk_request" => {
            let req = decode!(codec::respond_desk_request_from_json(o));
            call!(
                services.rfq_desk.respond_desk_request(Request::new(req)),
                "respond_desk_request_response",
                codec::respond_desk_request_response_to_json
            )
        }
        "accept_desk_quote" => {
            let req = decode!(codec::accept_desk_quote_from_json(o));
            call!(
                services.rfq_desk.accept_desk_quote(Request::new(req)),
                "accept_desk_quote_response",
                codec::accept_desk_quote_response_to_json
            )
        }
        "list_desk_requests" => {
            let req = decode!(codec::list_desk_requests_from_json(o));
            call!(
                services.rfq_desk.list_desk_requests(Request::new(req)),
                "list_desk_requests_response",
                codec::list_desk_requests_response_to_json
            )
        }
        "list_deals" => {
            let req = decode!(codec::list_deals_from_json(o));
            call!(
                services.rfq_desk.list_deals(Request::new(req)),
                "list_deals_response",
                codec::list_deals_response_to_json
            )
        }
        // ---- fix-admin: manage the inbound FIX acceptor connections ----------
        "list_fix_connections" => {
            let req = decode!(codec::list_fix_connections_request_from_json(o));
            call!(
                services.fix_admin.list_connections(Request::new(req)),
                "fix_connections",
                codec::list_fix_connections_response_to_json
            )
        }
        "create_fix_connection" => {
            let req = decode!(codec::create_fix_connection_request_from_json(o));
            call!(
                services.fix_admin.create_connection(Request::new(req)),
                "fix_connection_created",
                codec::create_fix_connection_response_to_json
            )
        }
        "update_fix_connection" => {
            let req = decode!(codec::update_fix_connection_request_from_json(o));
            call!(
                services.fix_admin.update_connection(Request::new(req)),
                "fix_connection_updated",
                codec::update_fix_connection_response_to_json
            )
        }
        "delete_fix_connection" => {
            let req = decode!(codec::delete_fix_connection_request_from_json(o));
            call!(
                services.fix_admin.delete_connection(Request::new(req)),
                "fix_connection_deleted",
                codec::delete_fix_connection_response_to_json
            )
        }
        "set_fix_connection_enabled" => {
            let req = decode!(codec::set_fix_connection_enabled_request_from_json(o));
            call!(
                services.fix_admin.set_enabled(Request::new(req)),
                "fix_connection_enabled",
                codec::set_fix_connection_enabled_response_to_json
            )
        }
        "list_fix_messages" => {
            let req = decode!(codec::list_fix_messages_request_from_json(o));
            call!(
                services.fix_admin.list_messages(Request::new(req)),
                "fix_messages",
                codec::list_fix_messages_response_to_json
            )
        }
        "login" => {
            let req = decode!(codec::login_request_from_json(o));
            call!(
                services.auth.login(Request::new(req)),
                "login_result",
                codec::login_response_to_json
            )
        }
        "logout" => {
            let req = decode!(codec::logout_request_from_json(o));
            call!(
                services.auth.logout(Request::new(req)),
                "logout_result",
                codec::logout_response_to_json
            )
        }
        "list_users" => {
            let req = decode!(codec::list_users_request_from_json(o));
            call!(
                services.auth.list_users(Request::new(req)),
                "users",
                codec::list_users_response_to_json
            )
        }
        "create_user" => {
            let req = decode!(codec::create_user_request_from_json(o));
            call!(
                services.auth.create_user(Request::new(req)),
                "user_created",
                codec::create_user_response_to_json
            )
        }
        "update_user" => {
            let req = decode!(codec::update_user_request_from_json(o));
            call!(
                services.auth.update_user(Request::new(req)),
                "user_updated",
                codec::update_user_response_to_json
            )
        }
        "delete_user" => {
            let req = decode!(codec::delete_user_request_from_json(o));
            call!(
                services.auth.delete_user(Request::new(req)),
                "user_deleted",
                codec::delete_user_response_to_json
            )
        }
        "reset_password" => {
            let req = decode!(codec::reset_password_request_from_json(o));
            call!(
                services.auth.reset_password(Request::new(req)),
                "password_reset",
                codec::reset_password_response_to_json
            )
        }
        "get_user_capabilities" => {
            let req = decode!(codec::get_user_capabilities_request_from_json(o));
            call!(
                services.auth.get_user_capabilities(Request::new(req)),
                "user_capabilities",
                codec::get_user_capabilities_response_to_json
            )
        }
        "set_user_capabilities" => {
            let req = decode!(codec::set_user_capabilities_request_from_json(o));
            call!(
                services.auth.set_user_capabilities(Request::new(req)),
                "user_capabilities_set",
                codec::set_user_capabilities_response_to_json
            )
        }
        "get_role_capabilities" => {
            let req = decode!(codec::get_role_capabilities_request_from_json(o));
            call!(
                services.auth.get_role_capabilities(Request::new(req)),
                "role_capabilities",
                codec::get_role_capabilities_response_to_json
            )
        }
        "set_role_capabilities" => {
            let req = decode!(codec::set_role_capabilities_request_from_json(o));
            call!(
                services.auth.set_role_capabilities(Request::new(req)),
                "role_capabilities_set",
                codec::set_role_capabilities_response_to_json
            )
        }
        "list_desks" => {
            let req = decode!(codec::list_desks_request_from_json(o));
            call!(
                services.auth.list_desks(Request::new(req)),
                "desks",
                codec::list_desks_response_to_json
            )
        }
        "create_desk" => {
            let req = decode!(codec::create_desk_request_from_json(o));
            call!(
                services.auth.create_desk(Request::new(req)),
                "desk_created",
                codec::create_desk_response_to_json
            )
        }
        "delete_desk" => {
            let req = decode!(codec::delete_desk_request_from_json(o));
            call!(
                services.auth.delete_desk(Request::new(req)),
                "desk_deleted",
                codec::delete_desk_response_to_json
            )
        }
        "list_entities" => {
            let req = decode!(codec::list_entities_request_from_json(o));
            call!(
                services.auth.list_entities(Request::new(req)),
                "entities",
                codec::list_entities_response_to_json
            )
        }
        "create_entity" => {
            let req = decode!(codec::create_entity_request_from_json(o));
            call!(
                services.auth.create_entity(Request::new(req)),
                "entity_created",
                codec::create_entity_response_to_json
            )
        }
        "update_entity" => {
            let req = decode!(codec::update_entity_request_from_json(o));
            call!(
                services.auth.update_entity(Request::new(req)),
                "entity_updated",
                codec::update_entity_response_to_json
            )
        }
        "delete_entity" => {
            let req = decode!(codec::delete_entity_request_from_json(o));
            call!(
                services.auth.delete_entity(Request::new(req)),
                "entity_deleted",
                codec::delete_entity_response_to_json
            )
        }
        "list_books" => {
            let req = decode!(codec::list_books_request_from_json(o));
            call!(
                services.auth.list_books(Request::new(req)),
                "books",
                codec::list_books_response_to_json
            )
        }
        "create_book" => {
            let req = decode!(codec::create_book_request_from_json(o));
            call!(
                services.auth.create_book(Request::new(req)),
                "book_created",
                codec::create_book_response_to_json
            )
        }
        "update_book" => {
            let req = decode!(codec::update_book_request_from_json(o));
            call!(
                services.auth.update_book(Request::new(req)),
                "book_updated",
                codec::update_book_response_to_json
            )
        }
        "delete_book" => {
            let req = decode!(codec::delete_book_request_from_json(o));
            call!(
                services.auth.delete_book(Request::new(req)),
                "book_deleted",
                codec::delete_book_response_to_json
            )
        }
        "list_instruments" => {
            let req = decode!(codec::list_instruments_request_from_json(o));
            call!(
                services.auth.list_instruments(Request::new(req)),
                "instruments",
                codec::list_instruments_response_to_json
            )
        }
        "get_instrument" => {
            let req = decode!(codec::get_instrument_request_from_json(o));
            call!(
                services.auth.get_instrument(Request::new(req)),
                "instrument",
                codec::get_instrument_response_to_json
            )
        }
        "create_instrument" => {
            let req = decode!(codec::create_instrument_request_from_json(o));
            call!(
                services.auth.create_instrument(Request::new(req)),
                "instrument_created",
                codec::create_instrument_response_to_json
            )
        }
        "update_instrument" => {
            let req = decode!(codec::update_instrument_request_from_json(o));
            call!(
                services.auth.update_instrument(Request::new(req)),
                "instrument_updated",
                codec::update_instrument_response_to_json
            )
        }
        "delete_instrument" => {
            let req = decode!(codec::delete_instrument_request_from_json(o));
            call!(
                services.auth.delete_instrument(Request::new(req)),
                "instrument_deleted",
                codec::delete_instrument_response_to_json
            )
        }
        "build_curve" => {
            let req = decode!(codec::build_curve_request_from_json(o));
            call!(
                services.auth.build_curve(Request::new(req)),
                "calibrated_curve",
                codec::calibrated_curve_to_json
            )
        }
        other => codec::error_frame(&format!("unknown request type `{other}`"), correlation_id),
    }
}

/// Map a tonic [`Status`](tonic::Status) into a typed `error` frame carrying the
/// gRPC status code name and message, so a WS client sees the same failure taxonomy
/// a gRPC client does (one contract, two encodings).
fn status_error(status: &tonic::Status, correlation_id: Option<u64>) -> Value {
    let mut frame = codec::error_frame(status.message(), correlation_id);
    if let Value::Object(ref mut m) = frame {
        m.insert(
            "code".to_owned(),
            Value::String(format!("{:?}", status.code())),
        );
    }
    frame
}

/// A [`futures_util::Stream`] of [`ClientStreamMessage`]s backing the WS side of the
/// shared RFS session driver — the WS analogue of the gRPC `Streaming` inbound.
struct ClientStreamRx {
    rx: mpsc::Receiver<ClientStreamMessage>,
}

impl futures_util::Stream for ClientStreamRx {
    type Item = ClientStreamMessage;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `authenticate` frame — the session-pinning FIRST frame every WS client
    /// sends — MUST route to the RFS session driver, not `handle_unary`. Omitting
    /// it from the routing classification left WS sessions anonymous, so under
    /// `AccessMode::Enforce` the following `subscribe` was rejected `unauthenticated`
    /// and no live two-way ever streamed (the GUI/Excel headline-stream defect this
    /// guards against forever).
    #[test]
    fn authenticate_routes_to_the_stream_driver() {
        assert!(
            is_stream_control("authenticate"),
            "`authenticate` must be a stream-control frame so it reaches run_session"
        );
    }

    /// Every descriptor-derived stream-control verb routes to the driver — the live
    /// RFS control surface, none of it treated as a request/response RPC. Driving
    /// the generated [`celnet_proto::wire_contract::STREAM_CONTROL_VERBS`] (the
    /// `ClientStreamMessage` oneof arms) makes this a forward guard: a new oneof
    /// arm added to the proto is asserted to route here automatically.
    #[test]
    fn every_stream_control_verb_is_classified() {
        assert!(
            !celnet_proto::wire_contract::STREAM_CONTROL_VERBS.is_empty(),
            "the generated stream-control verb set must be populated"
        );
        for &kind in celnet_proto::wire_contract::STREAM_CONTROL_VERBS {
            assert!(is_stream_control(kind), "`{kind}` must route to the driver");
        }
    }

    /// The router's stream-control classification is byte-identical to the verb set
    /// the WS clients have always sent — the no-regression proof for sourcing it
    /// from the proto descriptor (arch item G). If the generated set ever differs
    /// from this frozen list, the wire-routing contract changed and this fails.
    #[test]
    fn is_stream_control_matches_generated_contract() {
        let mut generated: Vec<&str> = celnet_proto::wire_contract::STREAM_CONTROL_VERBS.to_vec();
        generated.sort_unstable();
        let mut frozen = [
            "authenticate",
            "subscribe",
            "modify",
            "unsubscribe",
            "resync",
            "execute",
            "heartbeat",
            "market_series_subscribe",
            "market_series_unsubscribe",
            // The fixed-income streaming line folded onto the SAME multiplexed
            // session (rates-stream-ws): a WS client sends `{"type":"rates_subscribe"}`.
            "rates_subscribe",
        ];
        frozen.sort_unstable();
        assert_eq!(
            generated, frozen,
            "the descriptor-derived stream-control verbs must match the frozen wire set"
        );
    }

    /// `is_stream_control` and `decode_stream_control` stay in LOCKSTEP: a verb the
    /// decoder accepts must be classified as stream-control (else `dispatch` would
    /// mis-route it to `handle_unary`), and a verb it rejects must not be (else
    /// `dispatch` would try to decode a non-control frame). This catches the exact
    /// class of bug the `authenticate` omission was — a new control verb added to
    /// the decoder but forgotten in the router.
    #[test]
    fn classification_matches_the_decoder() {
        let empty = serde_json::Map::new();
        // The descriptor-derived control verbs (every one must be decoder-recognized:
        // a proto oneof arm added without its `decode_stream_control` arm fails here),
        // plus request/response RPCs and a nonsense verb that must NOT be control.
        let control = celnet_proto::wire_contract::STREAM_CONTROL_VERBS
            .iter()
            .copied();
        let non_control = [
            "price",
            "request_quote",
            "accept_quote",
            "login",
            "not_a_real_frame",
        ];
        for kind in control.chain(non_control) {
            // `decode_stream_control` rejects ONLY with the "unknown stream-control
            // type" error for a non-control verb; a control verb either decodes or
            // fails on a missing field (still "recognized"). Distinguish on the error.
            let decoded = decode_stream_control(kind, &empty);
            let recognized = match &decoded {
                Ok(_) => true,
                Err(e) => !e.0.contains("unknown stream-control type"),
            };
            assert_eq!(
                is_stream_control(kind),
                recognized,
                "`{kind}`: router classification must match the decoder's recognition",
            );
        }
    }

    /// A request/response RPC name is NOT a stream-control frame — it must reach
    /// `handle_unary`, never the driver.
    #[test]
    fn unary_rpcs_are_not_stream_control() {
        for kind in [
            "price",
            "request_quote",
            "accept_quote",
            "login",
            "aggregate_risk",
        ] {
            assert!(
                !is_stream_control(kind),
                "`{kind}` is a request/response RPC, not stream control"
            );
        }
    }
}
