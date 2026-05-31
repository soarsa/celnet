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
//! * **RFQ** — `request_quote` → `quote`, `accept_quote` → `execution`,
//!   `reject_quote` → `reject_ack`;
//! * **pricing** — `price` → `price_response`;
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

use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tonic::Request;

use celnet_proto::pricing_service_server::PricingService;
use celnet_proto::quote_service_server::QuoteService;
use celnet_proto::risk_service_server::RiskService;
use celnet_proto::surface_service_server::SurfaceService;
use celnet_proto::{ClientStreamMessage, ServerStreamMessage, client_stream_message};

use crate::clock::Clock;
use crate::core_link::CoreLink;
use crate::readiness::{InFlightGuard, ReadinessGate};
use crate::services::pricing::PricingEdge;
use crate::services::quote::QuoteEdge;
use crate::services::risk::RiskEdge;
use crate::services::risk::store::PositionStore;
use crate::services::stream::{StreamEdge, run_session};
use crate::services::surface::SurfaceEdge;
use crate::spread::SpreadModel;
use crate::surface_book::SurfaceBook;

/// The bounded depth of the per-connection server→client message channel. Matches
/// the gRPC RFS channel depth so a lagging WS consumer is paused (LAGGED) rather
/// than back-pressuring the shared driver — one slow socket never stalls the core.
const WS_CHANNEL_DEPTH: usize = 256;

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
    gate: Arc<ReadinessGate>,
}

impl WsServices {
    /// Construct the WS service set from the same shared components the gRPC edge is
    /// built from, so both fronts speak one contract over one pricing path — now
    /// including `RiskService` over the same shared live position book.
    #[must_use]
    pub fn new(
        link: Arc<CoreLink>,
        gate: Arc<ReadinessGate>,
        spread: SpreadModel,
        clock: Clock,
        surface_book: Arc<SurfaceBook>,
        store: Arc<PositionStore>,
    ) -> Self {
        let pricing = Arc::new(PricingEdge::new(
            Arc::clone(&gate),
            Arc::clone(&surface_book),
        ));
        let quote = Arc::new(QuoteEdge::new(
            Arc::clone(&link),
            Arc::clone(&gate),
            spread,
            clock.clone(),
            Arc::clone(&surface_book),
        ));
        let stream = Arc::new(StreamEdge::with_store(
            Arc::clone(&link),
            Arc::clone(&gate),
            spread,
            clock.clone(),
            Arc::clone(&surface_book),
            Arc::clone(&store),
        ));
        let surface = Arc::new(SurfaceEdge::new(
            Arc::clone(&link),
            Arc::clone(&gate),
            clock,
            surface_book,
        ));
        let risk = Arc::new(RiskEdge::new(store, Arc::clone(&gate)));
        Self {
            pricing,
            quote,
            stream,
            surface,
            risk,
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
    let ws = match tokio_tungstenite::accept_async(tcp).await {
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
    let (out_tx, mut out_rx) = mpsc::channel::<Value>(WS_CHANNEL_DEPTH);

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
                    Some(v) => {
                        if ws_tx.send(WsMessage::Text(v.to_string())).await.is_err() {
                            break;
                        }
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

    // The reader loop: dispatch each inbound frame. Request/response calls reply on
    // `out_tx`; stream-control frames are forwarded to the RFS driver via `rfs_in_tx`.
    while let Some(frame) = ws_rx.next().await {
        let msg = match frame {
            Ok(WsMessage::Text(t)) => t,
            Ok(WsMessage::Binary(b)) => match String::from_utf8(b) {
                Ok(t) => t,
                Err(_) => {
                    let _ = out_tx
                        .send(codec::error_frame(
                            "binary frame is not valid UTF-8 JSON",
                            None,
                        ))
                        .await;
                    continue;
                }
            },
            Ok(WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Frame(_)) => continue,
            Ok(WsMessage::Close(_)) | Err(_) => break,
        };
        if !dispatch(&services, &msg, &out_tx, &rfs_in_tx).await {
            break;
        }
    }

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
    out_tx: &mpsc::Sender<Value>,
    rfs_in_tx: &mpsc::Sender<ClientStreamMessage>,
) -> bool {
    let value: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => {
            return out_tx
                .send(codec::error_frame(&format!("invalid JSON: {e}"), None))
                .await
                .is_ok();
        }
    };
    let Some(o) = value.as_object() else {
        return out_tx
            .send(codec::error_frame("frame must be a JSON object", None))
            .await
            .is_ok();
    };
    let Some(kind) = o.get("type").and_then(Value::as_str) else {
        return out_tx
            .send(codec::error_frame(
                "frame missing a `type` discriminator",
                None,
            ))
            .await
            .is_ok();
    };
    let correlation_id = o.get("correlation_id").and_then(Value::as_u64);

    match kind {
        // ---- RFS stream control: forward to the shared session driver --------
        "subscribe"
        | "modify"
        | "unsubscribe"
        | "resync"
        | "execute"
        | "heartbeat"
        | "market_series_subscribe"
        | "market_series_unsubscribe" => {
            match decode_stream_control(kind, o) {
                Ok(msg) => {
                    // If the RFS driver has gone, the connection is being torn down.
                    rfs_in_tx.send(msg).await.is_ok()
                }
                Err(e) => out_tx
                    .send(codec::error_frame(&e.to_string(), correlation_id))
                    .await
                    .is_ok(),
            }
        }
        // ---- request/response RPCs: run the same edge, reply inline ----------
        _ => {
            let reply = handle_unary(services, kind, o, correlation_id).await;
            out_tx.send(reply).await.is_ok()
        }
    }
}

/// Decode a stream-control frame into the [`ClientStreamMessage`] the shared RFS
/// driver consumes (the same message a gRPC client sends on its `StreamSession`).
fn decode_stream_control(
    kind: &str,
    o: &Map<String, Value>,
) -> Result<ClientStreamMessage, codec::CodecError> {
    let message = match kind {
        "subscribe" => client_stream_message::Message::Subscribe(codec::subscribe_from_json(o)?),
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
        "price" => {
            let req = decode!(codec::price_request_from_json(o));
            call!(
                services.pricing.price(Request::new(req)),
                "price_response",
                codec::price_response_to_json
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
