//! **Architectural invariant (behavioural): the per-tick price path resolves locally,
//! never through the router/federation.**
//!
//! The structural code-path proof lives in `celnet-server` (`services/forward.rs`
//! `per_tick_price_path_never_crosses_the_router` — `serve_mode(None) == Serve::Local`).
//! This is its **behavioural** counterpart, exercised over the real loopback gRPC
//! edge: an **in-process** [`Edge`] holds **zero fleet endpoints** (no router, no
//! backend `Fleet` at all), and yet a `Price` for an owned pair (the per-tick hot
//! path) returns a real, finite price.
//!
//! The load-bearing argument: if the per-tick price path crossed the router /
//! federation, an in-process edge — which has **no** fleet endpoints to route to —
//! could not resolve a price at all (a forward would have nowhere to go). It prices
//! fine ⇒ the price path is node-local, the router is off it. This is `docs/SCALE-OUT.md`
//! §11's "**never** on the critical price path — routing is connection-setup/
//! subscription, not per-tick", proven by behaviour, not asserted.
//!
//! It is **not** a latency claim (the absolute §11 wire SLOs stay deploy-gated); it
//! is a *routing-topology* assertion. Bounded by a hard deadline so a regression
//! fails fast, never hangs.

use std::time::Duration;

use celnet_bench::wire::{start_ready_edge, vanilla_call, wire_conventions};
use celnet_proto::pricing_service_client::PricingServiceClient;
use celnet_proto::{MarketContext, PriceRequest};

use tonic::transport::Channel;

/// An in-process edge (no fleet, no router) prices an owned pair on the per-tick
/// path — proving the price path is node-local and never crosses the router.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_process_price_resolves_locally_without_any_router() {
    tokio::time::timeout(Duration::from_secs(20), async {
        // `start_ready_edge` builds a plain in-process `Edge` (FleetTopology::InProcess
        // by construction — no `CELNET_FLEET_BACKENDS`, no fleet handle): there is NO
        // router and NO backend to forward to.
        let (edge, addr) = start_ready_edge().await.expect("in-process edge binds");

        let channel = Channel::from_shared(format!("http://{addr}"))
            .expect("valid endpoint")
            .connect()
            .await
            .expect("loopback channel connects");
        let mut client = PricingServiceClient::new(channel);

        // A `Price` for the edge's owned EUR/USD pair — the per-tick hot path. With no
        // fleet, the ONLY way this can succeed is the node-local price path (the
        // `Serve::Local` arm). A router-crossing price path would have nowhere to
        // forward to and would fail.
        let req = PriceRequest {
            request_id: 1,
            instrument: Some(vanilla_call(1.10)),
            market: Some(MarketContext {
                spot: 1.10,
                vol: 0.095,
                r_dom: 0.025,
                r_for: 0.015,
            }),
            conventions: Some(wire_conventions()),
            correlation_id: None,
            surface_version: None,
        };
        let resp = client
            .price(req)
            .await
            .expect(
                "in-process edge MUST price an owned pair locally (no router on the price path)",
            )
            .into_inner();
        let greeks = resp.greeks.expect("a real priced result");
        assert!(
            greeks.price.is_finite() && greeks.price > 0.0,
            "the node-local price path returned a real finite price ({}), \
             proving the per-tick price path never needed the router/federation",
            greeks.price
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("architectural-invariant test must not hang");
}
