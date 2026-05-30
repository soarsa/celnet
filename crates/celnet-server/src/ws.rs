//! The WebSocket RFS (request-for-stream) streaming endpoint
//! (`tokio-tungstenite`).
//!
//! A counterparty subscribes once and then receives a continuous stream of
//! price/Greek updates as the market moves — the FX-options *RFS* model. The
//! server:
//!
//! 1. runs a single **tick-driver** task that pulls deterministic market ticks
//!    from the [`TickSource`], publishes each new [`celnet_engine::MarketState`]
//!    to the pricing core (lock-free, via the [`CoreLink`] control plane), prices
//!    a reference strike on the hot ring, and **fans the update out** to every
//!    subscriber over a `tokio::sync::broadcast` channel;
//! 2. accepts WebSocket connections and, per connection, forwards each broadcast
//!    update as a JSON text frame until the client disconnects or the server
//!    drains.
//!
//! The async edge never blocks the core: ticks and prices flow over the same
//! rings/channels the gRPC path uses. Updates are serialized with `serde_json`
//! into the neutral [`PriceUpdate`] shape (no vendor wire format leaks into the
//! public stream).

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, oneshot};
use tokio_tungstenite::tungstenite::Message;

use celnet_types::OptionType;

use crate::core_link::CoreLink;
use crate::readiness::ReadinessGate;
use crate::tick::TickSource;

/// One streamed price/Greek update pushed to RFS subscribers.
///
/// A vendor-neutral, purpose-named projection of a priced tick: the sequence
/// number (monotonic per stream, so a consumer detects gaps), the spot that drove
/// it, the reference strike priced, and the headline risk. Serialized as JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceUpdate {
    /// Monotonic per-stream sequence number.
    pub sequence: u64,
    /// The spot that drove this update.
    pub spot: f64,
    /// The reference strike priced for the stream.
    pub strike: f64,
    /// Present value (domestic premium per 1 unit of base notional).
    pub price: f64,
    /// Spot delta (premium-unadjusted).
    pub delta_spot: f64,
    /// Vega (per 1.0 absolute vol).
    pub vega: f64,
    /// The Black vol the smile assigned to the priced strike.
    pub vol: f64,
}

/// The depth of the broadcast fan-out buffer. A slow subscriber that lags by more
/// than this many updates is lagged (it receives a `Lagged` error and resyncs
/// from the next update) rather than back-pressuring the tick driver — the RFS
/// stream never blocks on one slow consumer.
const BROADCAST_DEPTH: usize = 256;

/// The interval between synthesized ticks. Small but nonzero so a test observes
/// several updates quickly while the loop stays cooperative.
const TICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(5);

/// The WebSocket RFS streaming server.
///
/// Owns the [`CoreLink`] (to publish ticks and price), the [`ReadinessGate`] (so
/// it refuses new subscriptions while draining), and the [`TickSource`] that
/// drives the stream. Constructed by [`crate::Edge::start`].
#[derive(Debug)]
pub struct RfsServer {
    link: Arc<CoreLink>,
    gate: Arc<ReadinessGate>,
    tick: TickSource,
}

impl RfsServer {
    /// Construct the RFS server.
    #[must_use]
    pub fn new(link: Arc<CoreLink>, gate: Arc<ReadinessGate>, tick: TickSource) -> Self {
        Self { link, gate, tick }
    }

    /// Serve WebSocket subscriptions on `listener` until `shutdown` fires.
    ///
    /// Spawns the tick driver and then accepts connections, handing each to a
    /// per-connection task that forwards broadcast updates. On `shutdown` it stops
    /// accepting; in-flight connection tasks finish their current send and then
    /// observe the closed broadcast channel and exit.
    pub async fn serve(self, listener: TcpListener, mut shutdown: oneshot::Receiver<()>) {
        let (tx, _rx) = broadcast::channel::<PriceUpdate>(BROADCAST_DEPTH);

        // Tick driver: deterministic ticks → publish → price → broadcast.
        let driver_tx = tx.clone();
        let driver_link = Arc::clone(&self.link);
        let mut tick = self.tick;
        let reference_strike = tick.base_spot(); // ATM-forward-ish reference.
        let (driver_stop_tx, mut driver_stop_rx) = oneshot::channel::<()>();
        let driver = tokio::spawn(async move {
            let mut sequence: u64 = 0;
            let mut interval = tokio::time::interval(TICK_INTERVAL);
            loop {
                tokio::select! {
                    _ = &mut driver_stop_rx => break,
                    _ = interval.tick() => {
                        let state = tick.next_state();
                        let spot = state.spot;
                        // Publish the new market state to the core and wait for it
                        // to be applied, so the price below is computed against
                        // exactly this spot (keeps the streamed line consistent).
                        if driver_link.publish_acked(state).await.is_err() {
                            break; // core gone
                        }
                        // Price the reference strike on the hot path.
                        let Ok(resp) = driver_link
                            .price(OptionType::Call, reference_strike)
                            .await
                        else {
                            break; // core gone
                        };
                        sequence += 1;
                        let update = PriceUpdate {
                            sequence,
                            spot,
                            strike: reference_strike,
                            price: resp.greeks.price,
                            delta_spot: resp.greeks.delta_spot,
                            vega: resp.greeks.vega,
                            vol: resp.vol,
                        };
                        // Fan out; `Err` only means *currently* no subscribers,
                        // which is fine — the next subscriber gets later updates.
                        let _ = driver_tx.send(update);
                    }
                }
            }
        });

        // Accept loop.
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                accepted = listener.accept() => {
                    let Ok((stream, _peer)) = accepted else { continue };
                    // Refuse new subscriptions once draining.
                    if !self.gate.is_ready() {
                        // Drop the connection: a draining instance steers new
                        // streams to the warm replacement.
                        drop(stream);
                        continue;
                    }
                    let sub = tx.subscribe();
                    let guard = self.gate.enter();
                    tokio::spawn(async move {
                        serve_connection(stream, sub, guard).await;
                    });
                }
            }
        }

        // Stop the driver and let it (and its broadcast sender) drop, which closes
        // the per-connection receivers so those tasks exit.
        let _ = driver_stop_tx.send(());
        let _ = driver.await;
    }
}

/// Serve a single subscribed WebSocket connection: forward each broadcast
/// [`PriceUpdate`] as a JSON text frame until the client closes, the channel
/// closes, or a send fails.
///
/// `guard` holds the in-flight slot on the readiness gate for the connection's
/// lifetime, so a graceful drain waits for streaming connections to finish.
async fn serve_connection(
    stream: TcpStream,
    mut sub: broadcast::Receiver<PriceUpdate>,
    guard: crate::readiness::InFlightGuard,
) {
    let _guard = guard; // held for the connection lifetime.
    let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    let (mut writer, mut reader) = ws.split();

    loop {
        tokio::select! {
            // Drain inbound frames so pings are answered by the library and a
            // client Close terminates the stream promptly.
            inbound = reader.next() => {
                match inbound {
                    Some(Ok(msg)) if msg.is_close() => break,
                    Some(Ok(_)) => {} // ignore client text/ping (lib auto-pongs)
                    Some(Err(_)) | None => break,
                }
            }
            // Forward the next broadcast update.
            update = sub.recv() => {
                match update {
                    Ok(u) => {
                        // Serialize to JSON; a serialization error is unreachable
                        // for this plain-data struct but is handled, not panicked.
                        let Ok(json) = serde_json::to_string(&u) else { continue };
                        if writer.send(Message::text(json)).await.is_err() {
                            break; // peer gone
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // Slow consumer fell behind; resync from the next update.
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    let _ = writer.close().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_update_round_trips_json() {
        let u = PriceUpdate {
            sequence: 9,
            spot: 1.1003,
            strike: 1.10,
            price: 0.0123,
            delta_spot: 0.51,
            vega: 0.30,
            vol: 0.105,
        };
        let json = serde_json::to_string(&u).unwrap();
        let back: PriceUpdate = serde_json::from_str(&json).unwrap();
        assert_eq!(u, back);
    }
}
