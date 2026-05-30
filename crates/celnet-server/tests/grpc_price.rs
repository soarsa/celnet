//! In-process gRPC integration test: start the pricing-edge service on an
//! ephemeral port, dial it with the generated client, and assert the priced
//! result matches `celnet-vanilla` evaluated directly at the live smile vol.
//!
//! This exercises the full edge path — tonic codec, the async⇄core bridge, the
//! SPSC rings, the pricing core, and the readiness gate — against the same
//! closed-form Garman-Kohlhagen math the core uses, so a regression anywhere on
//! the path is caught against a first-principles reference rather than a snapshot.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use celnet_core::is_close;
use celnet_engine::testing::make_state;
use celnet_server::proto::pricing_edge_client::PricingEdgeClient;
use celnet_server::proto::{
    self, DeltaConvention as WireDeltaConvention, OptionType as WireOptionType,
};
use celnet_server::{CoreLink, Edge, TickSource};
use celnet_types::{CcyPair, OptionType, Tenor, VanillaInputs};

fn eurusd_conv() -> celnet_conventions::ConventionRecord {
    celnet_conventions::resolve(CcyPair::parse("EURUSD").unwrap(), Tenor::Years(1)).record
}

/// Start an `Edge` on ephemeral gRPC + WS ports, marked ready, with a quiet
/// (constant-spot) tick source so the RFS driver does not perturb the state the
/// gRPC test queries.
async fn start_quiet_edge(spot: f64) -> Edge {
    let conv = eurusd_conv();
    let initial = make_state(spot, conv);
    let link = CoreLink::start(initial.clone(), None);
    // bump = 0.0 ⇒ constant spot; the gRPC test prices against a stable state.
    let tick = TickSource::new(initial, 1, 0.0);
    let grpc: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let ws: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let edge = Edge::start(grpc, ws, Arc::clone(&link), tick)
        .await
        .expect("edge binds on ephemeral ports");
    edge.gate().mark_ready();
    edge
}

/// The hard wall-clock ceiling for any single server integration test. A
/// correctness failure must surface as a *fast* failure, never an infinite hang,
/// so every test body runs inside this bound and every network/stream `.await` is
/// itself bounded. If the bridge regresses into a stall, the suite fails in
/// seconds with a clear message instead of wedging CI.
const TEST_DEADLINE: Duration = Duration::from_secs(10);

/// Bound a single network/response `.await` so a never-arriving reply fails fast.
const STEP_DEADLINE: Duration = Duration::from_secs(5);

#[tokio::test]
async fn grpc_price_matches_direct_vanilla() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let spot = 1.10;
        let edge = start_quiet_edge(spot).await;
        let addr = edge.grpc_addr();

        // Dial the generated client over the real TCP socket.
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            PricingEdgeClient::connect(format!("http://{addr}")),
        )
        .await
        .expect("client connects within the step deadline")
        .expect("client connects to the ephemeral gRPC port");

        // Build a price request. The edge prices `strike` on the core against the
        // *published* market state; we mirror that to form the reference.
        let strike = 1.12;
        let inputs = proto::VanillaInputs {
            spot,
            strike,
            vol: 0.10, // request-supplied vol (used only for the convention delta)
            t: 1.0,
            r_dom: 0.02,
            r_for: 0.01,
        };
        let req = proto::PriceVanillaRequest {
            request_id: 7,
            pair: Some(proto::CcyPair {
                base: "EUR".to_owned(),
                quote: "USD".to_owned(),
            }),
            option_type: WireOptionType::Call as i32,
            inputs: Some(inputs),
            delta_convention: WireDeltaConvention::SpotPremiumAdjusted as i32,
            premium_style: proto::PremiumStyle::DomesticPips as i32,
        };

        let resp = tokio::time::timeout(STEP_DEADLINE, client.price_vanilla(req))
            .await
            .expect("price_vanilla RPC returns within the step deadline")
            .expect("price_vanilla RPC succeeds")
            .into_inner();
        assert_eq!(resp.request_id, 7);
        let greeks = resp.greeks.expect("greeks present");

        // Reference: replicate exactly what the core does — read the live published
        // state, take the smile vol at the strike, and price the GK greeks directly.
        let state = make_state(spot, eurusd_conv());
        let forward = state.forward();
        let smile_vol = celnet_core::Smile::implied_vol(&state.smile, strike, forward, state.t).0;
        let direct = celnet_vanilla::greeks(
            OptionType::Call,
            &VanillaInputs::new(spot, strike, smile_vol, state.t, state.r_dom, state.r_for),
        );

        assert!(
            is_close(greeks.price, direct.price, 1e-12, 1e-12),
            "edge price {} != direct {}",
            greeks.price,
            direct.price
        );
        assert!(is_close(greeks.vega, direct.vega, 1e-12, 1e-12));
        assert!(is_close(greeks.delta_spot, direct.delta_spot, 1e-12, 1e-12));
        assert!(is_close(greeks.gamma, direct.gamma, 1e-12, 1e-12));

        // The convention delta is resolved against the request inputs (vol 0.10).
        let conv_delta = celnet_vanilla::convention_delta(
            celnet_types::DeltaConvention::SpotPremiumAdjusted,
            OptionType::Call,
            &VanillaInputs::new(spot, strike, 0.10, 1.0, 0.02, 0.01),
        );
        assert!(is_close(
            resp.delta_convention_value,
            conv_delta,
            1e-12,
            1e-12
        ));

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

#[tokio::test]
async fn grpc_surface_and_barrier_round_trip() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let spot = 1.10;
        let edge = start_quiet_edge(spot).await;
        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            PricingEdgeClient::connect(format!("http://{}", edge.grpc_addr())),
        )
        .await
        .expect("client connects within the step deadline")
        .expect("client connects");

        // --- Surface vol read ---------------------------------------------------
        let strike = 1.15;
        let sresp = tokio::time::timeout(
            STEP_DEADLINE,
            client.surface_vol(proto::SurfaceVolRequest {
                request_id: 11,
                pair: Some(proto::CcyPair {
                    base: "EUR".to_owned(),
                    quote: "USD".to_owned(),
                }),
                strike,
                tenor_years: 1.0,
            }),
        )
        .await
        .expect("surface_vol RPC returns within the step deadline")
        .expect("surface_vol RPC succeeds")
        .into_inner();
        assert_eq!(sresp.request_id, 11);

        let state = make_state(spot, eurusd_conv());
        let forward = state.forward();
        let ref_vol = celnet_core::Smile::implied_vol(&state.smile, strike, forward, state.t).0;
        assert!(is_close(sresp.vol, ref_vol, 1e-12, 1e-12));
        assert!(is_close(sresp.forward, forward, 1e-12, 1e-12));

        // --- Single-barrier exotic read ----------------------------------------
        let inputs = proto::VanillaInputs {
            spot,
            strike: 1.10,
            vol: ref_vol,
            t: 1.0,
            r_dom: 0.02,
            r_for: 0.01,
        };
        let bresp = tokio::time::timeout(
            STEP_DEADLINE,
            client.price_barrier(proto::PriceBarrierRequest {
                request_id: 12,
                pair: Some(proto::CcyPair {
                    base: "EUR".to_owned(),
                    quote: "USD".to_owned(),
                }),
                option_type: WireOptionType::Call as i32,
                inputs: Some(inputs),
                barrier_kind: proto::BarrierKind::DownAndOut as i32,
                barrier: 0.95,
                rebate: 0.0,
            }),
        )
        .await
        .expect("price_barrier RPC returns within the step deadline")
        .expect("price_barrier RPC succeeds")
        .into_inner();
        assert_eq!(bresp.request_id, 12);

        // Reference: the same closed-form single-barrier price.
        let spec = celnet_exotics::SingleBarrier {
            kind: celnet_exotics::BarrierKind {
                up: false,
                style: celnet_exotics::BarrierStyle::KnockOut,
                option: OptionType::Call,
            },
            strike: 1.10,
            barrier: 0.95,
            rebate: 0.0,
        };
        let direct = celnet_exotics::single_barrier_price(
            &VanillaInputs::new(spot, 1.10, ref_vol, 1.0, 0.02, 0.01),
            spec,
        );
        assert!(
            is_close(bresp.price, direct, 1e-10, 1e-12),
            "barrier edge {} != direct {}",
            bresp.price,
            direct
        );

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

#[tokio::test]
async fn grpc_rejects_requests_until_ready() {
    tokio::time::timeout(TEST_DEADLINE, async {
        // An edge that has NOT been marked ready must refuse pricing with UNAVAILABLE
        // (the `/readyz` contract): new traffic only flows once warm.
        let conv = eurusd_conv();
        let initial = make_state(1.10, conv);
        let link = CoreLink::start(initial.clone(), None);
        let tick = TickSource::new(initial, 1, 0.0);
        let edge = Edge::start(
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.1:0".parse().unwrap(),
            Arc::clone(&link),
            tick,
        )
        .await
        .expect("edge binds");
        // Deliberately NOT marked ready.

        let mut client = tokio::time::timeout(
            STEP_DEADLINE,
            PricingEdgeClient::connect(format!("http://{}", edge.grpc_addr())),
        )
        .await
        .expect("client connects within the step deadline")
        .expect("client connects");

        let status = tokio::time::timeout(
            STEP_DEADLINE,
            client.price_vanilla(proto::PriceVanillaRequest {
                request_id: 1,
                pair: None,
                option_type: WireOptionType::Call as i32,
                inputs: Some(proto::VanillaInputs {
                    spot: 1.10,
                    strike: 1.10,
                    vol: 0.10,
                    t: 1.0,
                    r_dom: 0.02,
                    r_for: 0.01,
                }),
                delta_convention: WireDeltaConvention::SpotUnadjusted as i32,
                premium_style: proto::PremiumStyle::DomesticPips as i32,
            }),
        )
        .await
        .expect("price_vanilla RPC returns within the step deadline")
        .expect_err("a not-ready edge must reject pricing");
        assert_eq!(status.code(), tonic::Code::Unavailable);

        // The readiness probe itself always answers (it reports, never gates).
        let probe =
            tokio::time::timeout(STEP_DEADLINE, client.readiness(proto::ReadinessRequest {}))
                .await
                .expect("readiness probe returns within the step deadline")
                .expect("readiness probe always answers")
                .into_inner();
        assert_eq!(probe.state, proto::ServiceState::Starting as i32);
        assert!(!probe.ready);

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
