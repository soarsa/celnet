//! In-process WebSocket RFS integration test: start the streaming edge on an
//! ephemeral port, subscribe over a real WebSocket, and assert at least one
//! price/Greek update is received and that it carries a self-consistent priced
//! line (price equals a direct vanilla pricing at the streamed spot and vol).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use celnet_core::is_close;
use celnet_engine::testing::make_state;
use celnet_server::ws::PriceUpdate;
use celnet_server::{CoreLink, Edge, TickSource};
use celnet_types::{CcyPair, OptionType, Tenor, VanillaInputs};
use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message;

fn eurusd_conv() -> celnet_conventions::ConventionRecord {
    celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record
}

/// The hard wall-clock ceiling for the WS streaming integration test: a stalled
/// stream must fail fast, never hang the suite.
const TEST_DEADLINE: Duration = Duration::from_secs(15);

#[tokio::test]
async fn ws_subscriber_receives_streamed_update() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let spot = 1.10;
        let conv = eurusd_conv();
        let initial = make_state(spot, conv);
        let link = CoreLink::start(initial.clone(), None);
        // A small deterministic bump so the spot path is non-trivial; updates flow
        // every tick regardless of the bump.
        let tick = TickSource::new(initial, 1234, 0.002);

        let grpc: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let ws: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let edge = Edge::start(grpc, ws, Arc::clone(&link), tick)
            .await
            .expect("edge binds");
        edge.gate().mark_ready();

        let url = format!("ws://{}", edge.ws_addr());
        let (mut stream, _resp) = tokio::time::timeout(
            Duration::from_secs(5),
            tokio_tungstenite::connect_async(url),
        )
        .await
        .expect("WebSocket handshake completes within the step deadline")
        .expect("WebSocket handshake succeeds");

        // Pull the first streamed update within a bounded time.
        let update: PriceUpdate = loop {
            let msg = tokio::time::timeout(Duration::from_secs(5), stream.next())
                .await
                .expect("an update arrives before the timeout")
                .expect("the stream is open")
                .expect("the frame is well-formed");
            match msg {
                Message::Text(text) => break serde_json::from_str(&text).expect("update is JSON"),
                // Ignore protocol frames (ping/pong) and keep waiting for a payload.
                _ => continue,
            }
        };

        assert!(update.sequence >= 1, "sequence must be monotonic from 1");
        assert!(update.spot > 0.0);
        // Within the bump band of base spot.
        assert!((update.spot - spot).abs() <= spot * 0.002 + 1e-12);

        // The streamed line is self-consistent: price == direct vanilla pricing at
        // the streamed spot and the streamed (smile) vol.
        let direct = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(update.spot, update.strike, update.vol, 1.0, 0.02, 0.01),
        );
        assert!(
            is_close(update.price, direct, 1e-9, 1e-12),
            "streamed price {} != direct {}",
            update.price,
            direct
        );

        // Receive a second update to confirm the stream is continuous.
        let next = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("a second update arrives");
        assert!(next.is_some(), "the RFS stream keeps producing updates");

        drop(stream);
        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
