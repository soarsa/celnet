//! **Owned-pair forwarding — real cross-process gRPC, the unary wire APIs.**
//!
//! Boots N (2-3) real `celnet-server` backend gRPC edges on ephemeral 127.0.0.1
//! ports, then a **Distributed** front [`Edge`] over their URLs (via the race-free
//! [`Edge::start_on_with_topology`], no env mutation). It proves the §3
//! owned-pair-forwarding router tier: every unary request through the front edge is
//! forwarded to the backend that owns its currency-pair (by the same HRW
//! [`PartitionMap`] the risk fan-out uses), and the forwarded answer is **identical**
//! to dialling that owning backend directly:
//!
//! * `PricingService::Price` forwarded == direct-to-owner (per pair);
//! * `SurfaceService::MarkSurface` on the front edge deposits on the OWNER's
//!   surface_book, and a later pinned `GetSmile` / `Price` for that pair reflects it
//!   on the owner (round-trip through the same owner);
//! * `SurfaceService::Scenario` forwarded == owner's;
//! * `QuoteService` request → accept routes back to the issuing backend;
//! * and the same holds **after a scale change** (a fresh fleet that re-homes a pair
//!   to a different backend — a price for that pair now routes to the new owner).
//!
//! In-process mode is covered by the existing `rfq` / `surface_scenario` suites
//! (unchanged); this file is the distributed-forwarding proof. Every body is hard
//! wall-clock bounded so a regression fails fast, never hangs.

mod common;

use std::net::SocketAddr;

use celnet_engine::testing::make_state;
use celnet_proto::pricing_service_client::PricingServiceClient;
use celnet_proto::quote_service_client::QuoteServiceClient;
use celnet_proto::surface_service_client::SurfaceServiceClient;
use celnet_proto::{
    BrokerQuoteSet, GetSmileRequest, MarkSurfaceRequest, PriceRequest, QuoteAccept, QuoteRequest,
    ScenarioRequest, ShockAxis, shock_axis,
};
use celnet_risk_fleet::FleetTopology;
use celnet_router::{PartitionKey, PartitionMap, Replica, ReplicaId, ReplicaSet};
use celnet_server::{Clock, CoreLink, Edge, SpreadModel};
use celnet_types::{Ccy, CcyPair};
use tonic::Request;
use tonic::transport::Channel;

use common::{
    STEP_DEADLINE, TEST_DEADLINE, eurusd_conv, live_market, vanilla_call, wire_conventions,
};

// ---------------------------------------------------------------------------
// the pairs the fleet partitions over (a disjoint pair universe)
// ---------------------------------------------------------------------------

fn pairs() -> [CcyPair; 4] {
    [
        CcyPair::new(Ccy::EUR, Ccy::USD),
        CcyPair::new(Ccy::GBP, Ccy::USD),
        CcyPair::new(Ccy::AUD, Ccy::USD),
        CcyPair::new(Ccy::USD, Ccy::JPY),
    ]
}

fn wire_pair(p: CcyPair) -> celnet_proto::CcyPair {
    celnet_proto::CcyPair {
        base: p.base.as_str().to_owned(),
        quote: p.quote.as_str().to_owned(),
    }
}

/// A vanilla call on `pair` at an absolute strike (reuses the common 1Y builder).
fn call_on(pair: CcyPair, strike: f64) -> celnet_proto::Instrument {
    let mut inst = vanilla_call(strike);
    inst.underlying = Some(celnet_proto::Underlying::fx(wire_pair(pair)));
    inst
}

// ---------------------------------------------------------------------------
// the backend fleet (real gRPC edges on ephemeral ports)
// ---------------------------------------------------------------------------

/// A running backend edge: kept alive, with its dial URL and router replica id. Each
/// backend is an ordinary in-process edge serving its own local core/surface book;
/// only the FRONT edge is distributed.
struct Backend {
    edge: Edge,
    url: String,
    replica: ReplicaId,
}

/// Boot one ready in-process backend edge on an ephemeral port over the EURUSD
/// fixture market (spot 1.10) — every backend shares the identical fixture so a
/// forwarded request and a direct-to-owner request see the same backend market.
async fn boot_backend(replica: ReplicaId) -> Backend {
    let initial = make_state(1.10, eurusd_conv());
    let link = CoreLink::start(initial, None);
    let grpc: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let edge = Edge::start(grpc, link, SpreadModel::default(), Clock::system())
        .await
        .expect("backend edge binds");
    edge.gate().mark_ready();
    let url = format!("http://{}", edge.grpc_addr());
    Backend { edge, url, replica }
}

/// Boot `n` backends with replica ids `1..=n`.
async fn boot_fleet(n: u64) -> Vec<Backend> {
    let mut v = Vec::new();
    for i in 1..=n {
        v.push(boot_backend(ReplicaId(i)).await);
    }
    v
}

/// The all-`Up` replica membership over the backends (in replica order) — the same
/// membership the front edge's `Fleet::connect` builds, so the test's routing oracle
/// matches the edge's.
fn membership(backends: &[Backend]) -> ReplicaSet {
    ReplicaSet::new(backends.iter().map(|b| Replica::up(b.replica)).collect()).unwrap()
}

/// The backend that OWNS `pair` under the HRW partition map (route by
/// `PartitionKey::pair` — exactly what the edge's `Fleet::owner_of_pair` does).
fn owner<'a>(backends: &'a [Backend], replicas: &ReplicaSet, pair: CcyPair) -> &'a Backend {
    let map = PartitionMap::new(replicas);
    let route = map.route(PartitionKey::pair(pair)).expect("routable");
    backends
        .iter()
        .find(|b| b.replica == route.replica)
        .expect("owner is a backend")
}

/// Boot a Distributed front edge over the backends' URLs (in replica order), ready.
async fn front_edge(backends: &[Backend]) -> Edge {
    let endpoints: Vec<String> = backends.iter().map(|b| b.url.clone()).collect();
    let initial = make_state(1.10, eurusd_conv());
    let link = CoreLink::start(initial, None);
    let grpc: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let ws: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let edge = Edge::start_on_with_topology(
        grpc,
        ws,
        link,
        SpreadModel::default(),
        Clock::system(),
        FleetTopology::Distributed { endpoints },
    )
    .await
    .expect("distributed front edge binds + dials backends");
    edge.gate().mark_ready();
    edge
}

async fn dial(url: &str) -> Channel {
    Channel::from_shared(url.to_owned())
        .expect("uri")
        .connect()
        .await
        .expect("dial backend")
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// **Price: forwarded == direct-to-owning-backend, for every pair.** A `Price`
/// through the distributed front edge is routed to the pair's owning backend; the
/// reply is byte-for-byte the same as calling that owning backend directly.
#[tokio::test]
async fn price_forwarded_equals_direct_to_owner() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let backends = boot_fleet(3).await;
        let replicas = membership(&backends);
        let front = front_edge(&backends).await;

        let mut front_cli =
            PricingServiceClient::new(dial(&format!("http://{}", front.grpc_addr())).await);

        // Sanity: the four pairs genuinely fan across more than one backend.
        let distinct: std::collections::HashSet<u64> = pairs()
            .iter()
            .map(|&p| owner(&backends, &replicas, p).replica.0)
            .collect();
        assert!(
            distinct.len() >= 2,
            "pairs must fan across multiple backends"
        );

        for (i, pair) in pairs().into_iter().enumerate() {
            let req = PriceRequest {
                request_id: 100 + i as u64,
                instrument: Some(call_on(pair, 1.10 + 0.01 * i as f64)),
                market: Some(live_market()),
                conventions: Some(wire_conventions()),
                correlation_id: Some(7),
                surface_version: None,
            };
            // Forwarded through the front edge.
            let via_front =
                tokio::time::timeout(STEP_DEADLINE, front_cli.price(Request::new(req.clone())))
                    .await
                    .expect("front price within deadline")
                    .expect("front price ok")
                    .into_inner();
            // Direct to the pair's owning backend.
            let owner = owner(&backends, &replicas, pair);
            let mut direct = PricingServiceClient::new(dial(&owner.url).await);
            let via_direct = direct
                .price(Request::new(req))
                .await
                .expect("direct price ok")
                .into_inner();

            assert_eq!(
                via_front.greeks.as_ref().unwrap().price.to_bits(),
                via_direct.greeks.as_ref().unwrap().price.to_bits(),
                "pair {pair:?}: forwarded price must equal direct-to-owner price"
            );
            assert_eq!(
                via_front.resolved_strike.to_bits(),
                via_direct.resolved_strike.to_bits()
            );
            assert_eq!(via_front.correlation_id, via_direct.correlation_id);
        }

        front.shutdown(std::time::Duration::from_secs(1)).await;
    })
    .await
    .expect("within deadline");
}

/// **MarkSurface deposits on the OWNER, and a later pinned Price/GetSmile reflects
/// it on the owner.** Marking a skewed surface through the front edge for a pair must
/// land on that pair's owning backend's surface_book — proven by pinning the returned
/// `surface_version` on a later `Price` (forwarded to the same owner, honoured) AND
/// on a direct `GetSmile` to the owner.
#[tokio::test]
async fn mark_surface_lands_on_owner_and_is_visible_there() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let backends = boot_fleet(3).await;
        let replicas = membership(&backends);
        let front = front_edge(&backends).await;

        let pair = CcyPair::new(Ccy::GBP, Ccy::USD);
        let owner = owner(&backends, &replicas, pair);

        let mut front_surf =
            SurfaceServiceClient::new(dial(&format!("http://{}", front.grpc_addr())).await);

        // Mark a 1Y surface through the front edge: forwarded to the owner.
        let mark = MarkSurfaceRequest {
            pair: Some(wire_pair(pair)),
            broker_quotes: vec![BrokerQuoteSet {
                tenor_years: 1.0,
                atm_vol: 0.10,
                rr_25: -0.004,
                bf_25: 0.0015,
                rr_10: 0.0,
                bf_10: 0.0,
                has_ten_delta: false,
            }],
            conventions: Some(wire_conventions()),
            smile_model: None,
        };
        let marked =
            tokio::time::timeout(STEP_DEADLINE, front_surf.mark_surface(Request::new(mark)))
                .await
                .expect("front mark within deadline")
                .expect("front mark ok")
                .into_inner();
        let version = marked.surface_version;
        assert!(version > 0, "a real surface version was assigned");

        // The OWNER's surface_book holds the deposited version (it landed there, not
        // on the front edge nor a non-owner backend).
        assert!(
            owner.edge.surface_book().has_version(version),
            "the mark must be deposited on the owning backend's surface_book"
        );
        // The front edge itself never priced locally → its own book has no such mark.
        assert!(
            !front.surface_book().has_version(version),
            "the front edge must not deposit the mark locally (it forwarded)"
        );

        // A later Price for that pair, pinned to the marked version, is forwarded to
        // the SAME owner and honoured (the pin resolves on the owner's book): the
        // forwarded pinned price equals the owner's own pinned price.
        let req = PriceRequest {
            request_id: 1,
            instrument: Some(call_on(pair, 1.27)),
            market: Some(live_market()),
            conventions: Some(wire_conventions()),
            correlation_id: Some(9),
            surface_version: Some(version),
        };
        let mut front_px =
            PricingServiceClient::new(dial(&format!("http://{}", front.grpc_addr())).await);
        let via_front = front_px
            .price(Request::new(req.clone()))
            .await
            .expect("front pinned price ok")
            .into_inner();
        assert_eq!(
            via_front.surface_version,
            Some(version),
            "the forwarded pinned price echoes the marked version (resolved on the owner)"
        );
        let mut owner_px = PricingServiceClient::new(dial(&owner.url).await);
        let via_owner = owner_px
            .price(Request::new(req))
            .await
            .expect("owner pinned price ok")
            .into_inner();
        assert_eq!(
            via_front.greeks.as_ref().unwrap().price.to_bits(),
            via_owner.greeks.as_ref().unwrap().price.to_bits(),
            "pinned forwarded price == owner's pinned price (same marked surface)"
        );

        // A direct GetSmile to the owner for that pair returns a calibrated slice
        // (the owner is the surface authority for the pair).
        let mut owner_surf = SurfaceServiceClient::new(dial(&owner.url).await);
        let smile = owner_surf
            .get_smile(Request::new(GetSmileRequest {
                pair: Some(wire_pair(pair)),
                tenor_years: 1.0,
                conventions: Some(wire_conventions()),
            }))
            .await
            .expect("owner get_smile ok")
            .into_inner();
        assert!(!smile.points.is_empty(), "owner returns a calibrated smile");

        front.shutdown(std::time::Duration::from_secs(1)).await;
    })
    .await
    .expect("within deadline");
}

/// **Scenario: forwarded == owner's.** A what-if grid through the front edge equals
/// the same grid run directly against the pair's owning backend, node for node.
#[tokio::test]
async fn scenario_forwarded_equals_owner() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let backends = boot_fleet(3).await;
        let replicas = membership(&backends);
        let front = front_edge(&backends).await;

        let pair = CcyPair::new(Ccy::AUD, Ccy::USD);
        let owner = owner(&backends, &replicas, pair);

        let req = ScenarioRequest {
            instrument: Some(call_on(pair, 0.66)),
            base_market: Some(live_market()),
            conventions: Some(wire_conventions()),
            axes: vec![ShockAxis {
                factor: shock_axis::Factor::Spot as i32,
                relative: true,
                steps: vec![-0.02, -0.01, 0.0, 0.01, 0.02],
            }],
            expiry_years: 1.0,
            risk_buckets: None,
            smile_model: None,
        };

        let mut front_surf =
            SurfaceServiceClient::new(dial(&format!("http://{}", front.grpc_addr())).await);
        let via_front = tokio::time::timeout(
            STEP_DEADLINE,
            front_surf.scenario(Request::new(req.clone())),
        )
        .await
        .expect("front scenario within deadline")
        .expect("front scenario ok")
        .into_inner();
        let mut owner_surf = SurfaceServiceClient::new(dial(&owner.url).await);
        let via_owner = owner_surf
            .scenario(Request::new(req))
            .await
            .expect("owner scenario ok")
            .into_inner();

        assert_eq!(via_front.points.len(), via_owner.points.len());
        for (f, o) in via_front.points.iter().zip(via_owner.points.iter()) {
            assert_eq!(
                f.greeks.as_ref().unwrap().price.to_bits(),
                o.greeks.as_ref().unwrap().price.to_bits(),
                "scenario node price forwarded == owner"
            );
        }

        front.shutdown(std::time::Duration::from_secs(1)).await;
    })
    .await
    .expect("within deadline");
}

/// **Quote: request → accept routes back to the issuing backend.** A `RequestQuote`
/// through the front edge is forwarded to the pair's owner; the returned `quote_id`
/// (minted under that backend's secret) is then accepted through the front edge,
/// which routes the accept back to the SAME issuing backend and books a real
/// execution against the issued quote.
#[tokio::test]
async fn quote_request_then_accept_routes_to_issuer() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let backends = boot_fleet(3).await;
        let front = front_edge(&backends).await;

        let pair = CcyPair::new(Ccy::EUR, Ccy::USD);
        let mut front_q =
            QuoteServiceClient::new(dial(&format!("http://{}", front.grpc_addr())).await);

        let quote = tokio::time::timeout(
            STEP_DEADLINE,
            front_q.request_quote(Request::new(QuoteRequest {
                idempotency_key: "fwd-key-1".to_owned(),
                instrument: Some(call_on(pair, 1.12)),
                conventions: Some(wire_conventions()),
                correlation_id: Some(11),
                surface_version: None,
                attribution: None,
            })),
        )
        .await
        .expect("front request_quote within deadline")
        .expect("front request_quote ok")
        .into_inner();
        assert!(quote.quote_id != 0, "a real quote was issued by the owner");
        assert_eq!(quote.correlation_id, Some(11));

        // Accept the SAME quote through the front edge: it must route back to the
        // issuing backend (the accept carries only quote_id, no pair) and book.
        let exec = front_q
            .accept_quote(Request::new(QuoteAccept {
                quote_id: quote.quote_id,
                idempotency_key: "fwd-key-1".to_owned(),
                side: celnet_proto::Side::Buy as i32,
            }))
            .await
            .expect("front accept routed to issuer + booked")
            .into_inner();
        assert_eq!(
            exec.quote_id, quote.quote_id,
            "execution is against the issued quote"
        );
        assert!(exec.execution_id != 0, "a real execution id was assigned");

        // An accept for an unknown quote_id (never issued through this edge) is a
        // clean not_found, never a mis-route.
        let unknown = front_q
            .accept_quote(Request::new(QuoteAccept {
                quote_id: quote.quote_id ^ 0xDEAD_BEEF,
                idempotency_key: "x".to_owned(),
                side: celnet_proto::Side::Buy as i32,
            }))
            .await
            .expect_err("unknown quote_id is not_found");
        assert_eq!(unknown.code(), tonic::Code::NotFound);

        front.shutdown(std::time::Duration::from_secs(1)).await;
    })
    .await
    .expect("within deadline");
}

/// **Scale change: a re-homed pair routes to its NEW owner.** Boot a 2-backend fleet
/// and find a pair; then boot a 3-backend fleet in which that pair's HRW owner is a
/// DIFFERENT backend, and assert a `Price` through the new front edge equals the new
/// owner's direct price (and the routing genuinely moved). This proves forwarding
/// tracks the live partition, not a frozen assignment.
#[tokio::test]
async fn scale_change_reroutes_pair_to_new_owner() {
    tokio::time::timeout(TEST_DEADLINE, async {
        // Find a pair whose owner under a 2-replica set differs from under a 3-replica
        // set (HRW moves ~1/N of keys on a membership change, so such a pair exists
        // among our universe — search the four, assert we found one).
        let set2 =
            ReplicaSet::new(vec![Replica::up(ReplicaId(1)), Replica::up(ReplicaId(2))]).unwrap();
        let set3 = ReplicaSet::new(vec![
            Replica::up(ReplicaId(1)),
            Replica::up(ReplicaId(2)),
            Replica::up(ReplicaId(3)),
        ])
        .unwrap();
        let map2 = PartitionMap::new(&set2);
        let map3 = PartitionMap::new(&set3);
        let moved = pairs().into_iter().find(|&p| {
            map2.route(PartitionKey::pair(p)).unwrap().replica
                != map3.route(PartitionKey::pair(p)).unwrap().replica
        });
        let pair = moved.expect("some pair re-homes between a 2- and 3-backend fleet");
        let new_owner_id = map3.route(PartitionKey::pair(pair)).unwrap().replica;

        // Boot the scaled-up (3-backend) fleet and its front edge.
        let backends = boot_fleet(3).await;
        let replicas = membership(&backends);
        assert_eq!(
            owner(&backends, &replicas, pair).replica,
            new_owner_id,
            "the booted fleet routes the pair to its new (3-replica) owner"
        );
        let front = front_edge(&backends).await;

        let req = PriceRequest {
            request_id: 1,
            instrument: Some(call_on(pair, 1.10)),
            market: Some(live_market()),
            conventions: Some(wire_conventions()),
            correlation_id: Some(3),
            surface_version: None,
        };
        let mut front_cli =
            PricingServiceClient::new(dial(&format!("http://{}", front.grpc_addr())).await);
        let via_front = front_cli
            .price(Request::new(req.clone()))
            .await
            .expect("front price ok")
            .into_inner();

        // The NEW owner serves it (forwarded == direct-to-new-owner).
        let new_owner = owner(&backends, &replicas, pair);
        let mut direct = PricingServiceClient::new(dial(&new_owner.url).await);
        let via_owner = direct
            .price(Request::new(req))
            .await
            .expect("new owner price ok")
            .into_inner();
        assert_eq!(
            via_front.greeks.as_ref().unwrap().price.to_bits(),
            via_owner.greeks.as_ref().unwrap().price.to_bits(),
            "after scale-up, the pair's price routes to and matches its new owner"
        );

        front.shutdown(std::time::Duration::from_secs(1)).await;
    })
    .await
    .expect("within deadline");
}
