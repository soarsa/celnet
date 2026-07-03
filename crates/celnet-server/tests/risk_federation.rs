//! **Distributed risk federation — real cross-process gRPC reconciliation.**
//!
//! Boots N (>= 3) real `celnet-server` gRPC edges on ephemeral 127.0.0.1 ports, each
//! seeded with its HRW-owned slice of one shared master book (partitioned by
//! `celnet_risk_fleet::natural_owner_of` over a `(entity, pair)` partition key, so
//! every position of an `(entity, pair)` cell is co-resident on exactly one backend).
//! It then stands up a **Distributed** `RiskEdge` that is itself a *client* of those
//! backend edges (one `celnet_client::Client` per endpoint, real gRPC over loopback)
//! and asserts that the **federated** answer equals a **single-node oracle** edge
//! seeded with the WHOLE book:
//!
//! * federated `AggregateRisk(FIRM, +shocks +curvature)` == oracle: every additive
//!   Greek + the vega ladder to 1e-12, and the firm VaR/ES + FRTB curvature exact
//!   (re-gathered over the identical constituent union);
//! * a grouped dimension (by ccy-pair) reconciles node-for-node;
//! * federated `ListPositions` == oracle entitled set, under grant-all AND a scoped
//!   deny principal (entitlement-prune consistency at the backends);
//! * federated `DrillRisk` + `LimitStatus` reconcile;
//! * **scale-up** (add a 4th backend, re-partition + re-seed) is invariant;
//! * **scale-down** (remove a backend after re-homing its slice to survivors) is
//!   invariant;
//! * **failover** (a backend down with a hot standby holding its slice → invariant;
//!   down with no standby → `unavailable`, never a wrong number).
//!
//! Booting the backends as tokio tasks on ephemeral ports dialled via real gRPC over
//! loopback is the genuine federation proof. Every test body is hard wall-clock
//! bounded so a regression fails fast, never hangs.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use celnet_proto::risk_service_server::{RiskService, RiskServiceServer};
use celnet_proto::{
    AdditiveRisk, AggregateRiskRequest, AggregateRiskResponse, DrillRiskRequest, DrillRiskResponse,
    EntitlementPrincipal, EntitlementRule, LimitStatusRequest, LimitStatusResponse,
    ListPositionsRequest, ListPositionsResponse, NumeraireRate, ReportingNumeraire, RiskDimension,
    RiskScope,
};
use celnet_risk_cube::{
    BookId as CubeBookId, DeskId, EntityId, FactKey, FactMeasure, LocationId, PositionId, RiskFact,
    TraderId,
};
use celnet_risk_fleet::natural_owner_of;
use celnet_risk_normalize::{PositionRisk, canonicalize};
use celnet_router::{Health, PartitionMap, Replica, ReplicaId, ReplicaSet};
use celnet_server::services::risk::RiskEdge;
use celnet_server::services::risk::federate::Fleet;
use celnet_server::services::risk::store::{BookedPosition, PositionStore};
use celnet_server::{Clock, CoreLink, Edge, ReadinessGate, SpreadModel};
use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};
use tonic::{Request, Response, Status};

use common::TEST_DEADLINE;

// ---------------------------------------------------------------------------
// the shared master book
// ---------------------------------------------------------------------------

/// One master position: an id, a `(entity, book, pair)` placement, and its economics.
/// The entity + pair drive the HRW partition key, so spreading these over a few
/// entities/pairs fans the book across backends.
#[derive(Clone, Copy)]
struct Master {
    id: u64,
    entity: u32,
    book: u32,
    pair: CcyPair,
    option: OptionType,
    notional: f64,
}

fn pairs() -> [CcyPair; 3] {
    [
        CcyPair::new(Ccy::EUR, Ccy::USD),
        CcyPair::new(Ccy::GBP, Ccy::USD),
        CcyPair::new(Ccy::AUD, Ccy::USD),
    ]
}

/// The shared master book: 12 vanilla legs across 3 pairs × 2 legal entities × 2
/// books, mixing long/short so the firm VaR genuinely diversifies (a sum of shard
/// VaRs would be wrong — the federation re-gathers and re-derives once).
fn master_book() -> Vec<Master> {
    let [eurusd, gbpusd, audusd] = pairs();
    // (id, entity, book, pair, option, notional)
    let rows: &[(u64, u32, u32, CcyPair, OptionType, f64)] = &[
        (1, 1, 11, eurusd, OptionType::Call, 10_000_000.0),
        (2, 1, 11, eurusd, OptionType::Put, -4_000_000.0),
        (3, 1, 12, gbpusd, OptionType::Call, 7_000_000.0),
        (4, 1, 12, gbpusd, OptionType::Call, 3_000_000.0),
        (5, 1, 11, audusd, OptionType::Put, 5_000_000.0),
        (6, 2, 21, eurusd, OptionType::Call, 6_000_000.0),
        (7, 2, 21, eurusd, OptionType::Call, -2_000_000.0),
        (8, 2, 22, gbpusd, OptionType::Put, 8_000_000.0),
        (9, 2, 22, audusd, OptionType::Call, 9_000_000.0),
        (10, 2, 21, audusd, OptionType::Call, -3_000_000.0),
        (11, 1, 12, eurusd, OptionType::Call, 2_500_000.0),
        (12, 2, 22, gbpusd, OptionType::Call, 1_500_000.0),
    ];
    rows.iter()
        .map(|&(id, entity, book, pair, option, notional)| Master {
            id,
            entity,
            book,
            pair,
            option,
            notional,
        })
        .collect()
}

/// The canonical 1Y 10-vol mark every leg is booked under (the canonical leaf is
/// re-derived from these inputs server-side, independent of the live surface).
fn inputs() -> VanillaInputs {
    VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02)
}

impl Master {
    /// The `BookedPosition` for this master leg.
    fn booked(self) -> BookedPosition {
        BookedPosition {
            position_id: self.id,
            pair: self.pair,
            option: self.option,
            notional_base: self.notional,
            inputs: inputs(),
            quoted_delta: DeltaConvention::SpotUnadjusted,
            premium_style: PremiumStyle::DomesticPips,
            surface_version: 1,
        }
    }

    /// The firm-consistent `FactKey` for this master leg (the org handles are the same
    /// integers on every store because every store interns the same dictionary in the
    /// same order, then we address by raw handle). `desk`/`entity` are explicit (no
    /// parent-pointer resolution needed) so partition + grouping are unambiguous.
    fn key(self) -> FactKey {
        FactKey {
            trader: TraderId(self.entity * 100 + self.book), // a stable per-book trader
            book: CubeBookId(self.book),
            desk: DeskId(self.entity * 10), // one desk per entity
            underlying: celnet_types::Underlying::Fx(self.pair),
            location: LocationId(self.entity), // location == entity here
            entity: EntityId(self.entity),
        }
    }

    /// The `RiskFact` (for HRW routing at partition time).
    fn fact(self) -> RiskFact {
        let position = PositionRisk::fx(
            self.pair,
            self.option,
            self.notional,
            inputs(),
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        );
        RiskFact {
            position_id: PositionId(u32::try_from(self.id).unwrap()),
            key: self.key(),
            measure: FactMeasure {
                leaf: canonicalize(&position).unwrap(),
                position,
                exotic: None,
            },
            surface_version: 1,
        }
    }
}

/// The reporting numeraire (USD) with the spot rates the EUR/GBP/AUD base legs need.
fn usd_numeraire() -> ReportingNumeraire {
    ReportingNumeraire {
        numeraire: "USD".to_owned(),
        rates: vec![
            NumeraireRate {
                ccy: "EUR".to_owned(),
                rate: 1.10,
            },
            NumeraireRate {
                ccy: "GBP".to_owned(),
                rate: 1.27,
            },
            NumeraireRate {
                ccy: "AUD".to_owned(),
                rate: 0.66,
            },
        ],
    }
}

/// Seed `store` with the given master legs (each upserted under its firm-consistent
/// `FactKey`); attribution is omitted (the federation reconciles risk numbers, not
/// who's-trading provenance, which `ListPositions` carries orthogonally).
fn seed(store: &PositionStore, legs: impl IntoIterator<Item = Master>) {
    for m in legs {
        store
            .upsert(m.booked(), m.key(), None)
            .expect("upsert master leg");
    }
}

/// A ready single-node `RiskEdge` over a fresh store seeded with `legs`.
fn single_node_edge(legs: impl IntoIterator<Item = Master>) -> RiskEdge {
    let gate = Arc::new(ReadinessGate::new());
    gate.mark_ready();
    let store = Arc::new(PositionStore::new());
    seed(&store, legs);
    RiskEdge::new(store, gate)
}

// ---------------------------------------------------------------------------
// the backend fleet (real gRPC edges on ephemeral ports)
// ---------------------------------------------------------------------------

/// A running backend: its `Edge` (kept alive) and its bound `http://` URL.
struct Backend {
    edge: Edge,
    url: String,
    replica: ReplicaId,
    /// The temp dir rooting this backend's isolated persisted config — kept alive for
    /// the backend's full lifetime so parallel test edges never race one shared path.
    _data_dir: tempfile::TempDir,
}

/// Boot one ready backend `Edge` on an ephemeral 127.0.0.1 port, seed its store with
/// `legs`, and return its handle + dial URL. (The edge's own fleet topology is
/// in-process — it serves its local slice directly; only the federating edge is
/// distributed.)
async fn boot_backend(replica: ReplicaId, legs: Vec<Master>) -> Backend {
    let initial = celnet_engine::testing::make_state(1.10, common::eurusd_conv());
    let link = CoreLink::start(initial, None);
    let grpc = "127.0.0.1:0".parse().unwrap();
    let data_dir = tempfile::tempdir().expect("temp data dir for the backend edge config");
    let edge = Edge::start(
        grpc,
        link,
        SpreadModel::default(),
        Clock::system(),
        Some(data_dir.path()),
    )
    .await
    .expect("backend edge binds");
    edge.gate().mark_ready();
    seed(edge.store(), legs);
    let url = format!("http://{}", edge.grpc_addr());
    Backend {
        edge,
        url,
        replica,
        _data_dir: data_dir,
    }
}

/// Partition the master book across `n` replicas (ids `1..=n`) by HRW natural owner,
/// returning, per replica, the legs it owns — a disjoint cover of the whole book.
fn partition(n: u64) -> BTreeMap<u64, Vec<Master>> {
    let replicas = ReplicaSet::new((1..=n).map(|i| Replica::up(ReplicaId(i))).collect()).unwrap();
    let map = PartitionMap::new(&replicas);
    let mut by_owner: BTreeMap<u64, Vec<Master>> = BTreeMap::new();
    for m in master_book() {
        let owner = natural_owner_of(&m.fact(), &map).expect("non-empty set has an owner");
        by_owner.entry(owner.0).or_default().push(m);
    }
    by_owner
}

/// Boot `n` backends seeded with their HRW slice of the master book; assert the
/// partition is a disjoint cover (every leg lands once). Returns the backends in
/// replica order.
async fn boot_fleet(n: u64) -> Vec<Backend> {
    let parts = partition(n);
    let total: usize = parts.values().map(Vec::len).sum();
    assert_eq!(total, master_book().len(), "partition is a disjoint cover");
    let mut backends = Vec::new();
    for i in 1..=n {
        let legs = parts.get(&i).cloned().unwrap_or_default();
        backends.push(boot_backend(ReplicaId(i), legs).await);
    }
    backends
}

/// A distributed `RiskEdge` federating an all-`Up` fleet over the backends' URLs,
/// carrying an empty firm config store (the master facts live on the backends; the
/// federated edge only stages gathered unions against its hierarchy/limits, which are
/// the firm defaults here — every fact carries explicit handles so grouping is
/// unambiguous without parent pointers).
async fn federated_edge(backends: &[Backend]) -> RiskEdge {
    let members: Vec<(String, Replica)> = backends
        .iter()
        .map(|b| (b.url.clone(), Replica::up(b.replica)))
        .collect();
    let fleet = Arc::new(
        Fleet::connect_with_membership(&members)
            .await
            .expect("fleet connects"),
    );
    let gate = Arc::new(ReadinessGate::new());
    gate.mark_ready();
    RiskEdge::with_fleet(Arc::new(PositionStore::new()), gate, fleet)
}

// ---------------------------------------------------------------------------
// request builders
// ---------------------------------------------------------------------------

/// An explicitly **asserted** grant-all principal. The deny-by-default trust
/// boundary refuses a request with NO principal (`tests/entitlements_boundary.rs`
/// proves that), so every federation request here asserts the firm-wide view it
/// always meant — and the federated frontend forwards the same assertion to the
/// backends, which re-authorize it at their own trait entry.
fn asserted_grant_all() -> Option<EntitlementPrincipal> {
    Some(EntitlementPrincipal {
        grant_all: true,
        grants: vec![],
        denies: vec![],
    })
}

fn firm_request_with_risk() -> AggregateRiskRequest {
    AggregateRiskRequest {
        dimension: RiskDimension::Firm as i32,
        numeraire: Some(usd_numeraire()),
        principal: asserted_grant_all(),
        scope: None,
        vega_pillars: vec![],
        var_spot_shocks: vec![-0.02, -0.01, 0.0, 0.01, 0.02],
        var_alpha: 0.99,
        curvature_risk_weight: 0.18,
        correlation_id: Some(7),
        session_token: None,
    }
}

fn ccy_pair_request() -> AggregateRiskRequest {
    AggregateRiskRequest {
        dimension: RiskDimension::Underlying as i32,
        numeraire: Some(usd_numeraire()),
        principal: asserted_grant_all(),
        scope: None,
        vega_pillars: vec![],
        var_spot_shocks: vec![-0.02, 0.0, 0.02],
        var_alpha: 0.975,
        curvature_risk_weight: 0.15,
        correlation_id: None,
        session_token: None,
    }
}

// ---------------------------------------------------------------------------
// reconciliation assertions
// ---------------------------------------------------------------------------

const TOL: f64 = 1e-12;

fn assert_additive_eq(fed: &AdditiveRisk, oracle: &AdditiveRisk) {
    let close = |a: f64, b: f64, what: &str| {
        assert!(
            (a - b).abs() <= TOL + TOL * b.abs(),
            "additive {what} federated {a} vs oracle {b} (delta {})",
            (a - b).abs()
        );
    };
    close(
        fed.delta_numeraire,
        oracle.delta_numeraire,
        "delta_numeraire",
    );
    close(fed.gamma, oracle.gamma, "gamma");
    close(fed.vega_numeraire, oracle.vega_numeraire, "vega_numeraire");
    close(fed.theta, oracle.theta, "theta");
    close(fed.vanna, oracle.vanna, "vanna");
    close(fed.volga, oracle.volga, "volga");
    close(fed.charm, oracle.charm, "charm");
    close(fed.speed, oracle.speed, "speed");
    close(fed.zomma, oracle.zomma, "zomma");
    close(fed.color, oracle.color, "color");
    close(
        fed.premium_numeraire,
        oracle.premium_numeraire,
        "premium_numeraire",
    );

    // Per-ccy delta vector: same legs, same amounts.
    let leg = |v: &[celnet_proto::CcyExposureLeg], ccy: &str| {
        v.iter().find(|l| l.ccy == ccy).map_or(0.0, |l| l.amount)
    };
    for ccy in ["EUR", "GBP", "AUD", "USD"] {
        close(
            leg(&fed.delta_vector, ccy),
            leg(&oracle.delta_vector, ccy),
            &format!("delta_vector[{ccy}]"),
        );
    }

    // Vega ladder: same buckets, same vega per pillar.
    let pillar_vega = |v: &[celnet_proto::VegaLadderBucket],
                       p: Option<celnet_proto::VegaPillar>| {
        v.iter().find(|b| b.pillar == p).map_or(0.0, |b| b.vega)
    };
    for b in &oracle.vega_ladder {
        close(
            pillar_vega(&fed.vega_ladder, b.pillar),
            b.vega,
            "vega_ladder bucket",
        );
    }
    assert_eq!(
        fed.vega_ladder.len(),
        oracle.vega_ladder.len(),
        "same number of vega-ladder buckets"
    );
}

fn assert_nonadditive_eq(
    fed: &celnet_proto::NonAdditiveRisk,
    oracle: &celnet_proto::NonAdditiveRisk,
) {
    let close_opt = |a: Option<f64>, b: Option<f64>, what: &str| match (a, b) {
        (Some(a), Some(b)) => assert!(
            (a - b).abs() <= 1e-9 + 1e-9 * b.abs(),
            "non-additive {what} federated {a} vs oracle {b}"
        ),
        (None, None) => {}
        _ => panic!("non-additive {what} presence mismatch: federated {a:?} vs oracle {b:?}"),
    };
    close_opt(fed.var, oracle.var, "var");
    close_opt(fed.es, oracle.es, "es");
    close_opt(fed.curvature_spot, oracle.curvature_spot, "curvature_spot");
}

/// Federated vs oracle for one aggregate request: same node set keyed by group, each
/// node's additive (1e-12) and non-additive (re-gathered, exact-ish) reconcile.
async fn assert_aggregate_reconciles(
    fed_edge: &RiskEdge,
    oracle: &RiskEdge,
    req: &AggregateRiskRequest,
) {
    let federated = RiskService::aggregate_risk(fed_edge, Request::new(req.clone()))
        .await
        .expect("federated aggregate")
        .into_inner();
    let single = RiskService::aggregate_risk(oracle, Request::new(req.clone()))
        .await
        .expect("oracle aggregate")
        .into_inner();
    assert_nodes_eq(&federated, &single);
}

fn assert_nodes_eq(federated: &AggregateRiskResponse, single: &AggregateRiskResponse) {
    assert_eq!(federated.dimension, single.dimension);
    assert_eq!(federated.numeraire, single.numeraire);
    let by_group = |r: &AggregateRiskResponse| {
        r.nodes
            .iter()
            .map(|n| (n.group, n.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    let fed = by_group(federated);
    let orc = by_group(single);
    assert_eq!(
        fed.keys().collect::<Vec<_>>(),
        orc.keys().collect::<Vec<_>>(),
        "same node groups"
    );
    for (g, on) in &orc {
        let fnode = &fed[g];
        assert_eq!(
            fnode.position_count, on.position_count,
            "group {g} position count"
        );
        assert_additive_eq(
            fnode.additive.as_ref().unwrap(),
            on.additive.as_ref().unwrap(),
        );
        assert_nonadditive_eq(
            fnode.nonadditive.as_ref().unwrap(),
            on.nonadditive.as_ref().unwrap(),
        );
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

/// **Core reconciliation across 3 real gRPC backends.** The federated FIRM aggregate
/// (additive + VaR/ES + curvature) and a grouped by-ccy-pair aggregate both equal the
/// single-node oracle over the whole book.
#[tokio::test]
async fn federated_aggregate_equals_single_node_oracle() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let backends = boot_fleet(3).await;
        // Sanity: the book genuinely fanned across >1 backend.
        let nonempty = backends
            .iter()
            .filter(|b| !b.edge.store().is_empty())
            .count();
        assert!(
            nonempty >= 2,
            "master book must fan across multiple backends"
        );

        let fed = federated_edge(&backends).await;
        let oracle = single_node_edge(master_book());

        assert_aggregate_reconciles(&fed, &oracle, &firm_request_with_risk()).await;
        assert_aggregate_reconciles(&fed, &oracle, &ccy_pair_request()).await;
    })
    .await
    .expect("within deadline");
}

/// **Federated ListPositions == oracle entitled set**, under grant-all AND a scoped
/// deny principal — entitlement pruning is consistent at the backends (deny wins) and
/// the union de-duplicates by id.
#[tokio::test]
async fn federated_list_positions_equals_oracle() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let backends = boot_fleet(3).await;
        let fed = federated_edge(&backends).await;
        let oracle = single_node_edge(master_book());

        let ids = |r: &celnet_proto::ListPositionsResponse| {
            let mut v: Vec<u64> = r.positions.iter().map(|p| p.position_id).collect();
            v.sort_unstable();
            v
        };

        // Asserted grant-all: the whole book.
        let req = ListPositionsRequest {
            scope: None,
            principal: asserted_grant_all(),
            correlation_id: Some(1),
            session_token: None,
        };
        let f = RiskService::list_positions(&fed, Request::new(req.clone()))
            .await
            .unwrap()
            .into_inner();
        let o = RiskService::list_positions(&oracle, Request::new(req))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(ids(&f), ids(&o), "grant-all union == oracle");
        assert_eq!(ids(&f).len(), master_book().len());

        // Scoped deny: wall off entity 1's desk (desk handle = entity*10 = 10).
        let walled = EntitlementPrincipal {
            grant_all: true,
            grants: vec![],
            denies: vec![EntitlementRule {
                scopes: vec![RiskScope {
                    dimension: RiskDimension::Desk as i32,
                    value: 10,
                }],
            }],
        };
        let req = ListPositionsRequest {
            scope: None,
            principal: Some(walled),
            correlation_id: None,
            session_token: None,
        };
        let f = RiskService::list_positions(&fed, Request::new(req.clone()))
            .await
            .unwrap()
            .into_inner();
        let o = RiskService::list_positions(&oracle, Request::new(req))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(ids(&f), ids(&o), "deny-walled union == oracle");
        assert!(
            ids(&f).len() < master_book().len(),
            "the wall actually cut positions"
        );
    })
    .await
    .expect("within deadline");
}

/// **Federated DrillRisk reconciles.** Drilling the FIRM apex into its ccy-pair
/// children (with positions) merges the per-child additive across backends + unions
/// the contributing positions, equal to the single-node drill.
#[tokio::test]
async fn federated_drill_reconciles() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let backends = boot_fleet(3).await;
        let fed = federated_edge(&backends).await;
        let oracle = single_node_edge(master_book());

        let req = DrillRiskRequest {
            node: Some(RiskScope {
                dimension: RiskDimension::Firm as i32,
                value: 0,
            }),
            child_dimension: RiskDimension::Underlying as i32,
            numeraire: Some(usd_numeraire()),
            principal: asserted_grant_all(),
            vega_pillars: vec![],
            include_children: true,
            include_positions: true,
            correlation_id: Some(9),
            session_token: None,
        };
        let f = RiskService::drill_risk(&fed, Request::new(req.clone()))
            .await
            .unwrap()
            .into_inner();
        let o = RiskService::drill_risk(&oracle, Request::new(req))
            .await
            .unwrap()
            .into_inner();

        // Same contributing positions (union de-duplicated == whole book).
        let pids = |r: &celnet_proto::DrillRiskResponse| {
            let mut v: Vec<u64> = r.positions.iter().map(|p| p.position_id).collect();
            v.sort_unstable();
            v
        };
        assert_eq!(pids(&f), pids(&o), "drill positions union == oracle");
        assert_eq!(pids(&f).len(), master_book().len());

        // Same child nodes, each additive reconciled.
        let by_group = |r: &celnet_proto::DrillRiskResponse| {
            r.children
                .iter()
                .map(|n| (n.group, n.clone()))
                .collect::<BTreeMap<_, _>>()
        };
        let fc = by_group(&f);
        let oc = by_group(&o);
        assert_eq!(
            fc.keys().collect::<Vec<_>>(),
            oc.keys().collect::<Vec<_>>(),
            "same child groups"
        );
        for (g, on) in &oc {
            assert_additive_eq(
                fc[g].additive.as_ref().unwrap(),
                on.additive.as_ref().unwrap(),
            );
        }
    })
    .await
    .expect("within deadline");
}

/// **Federated LimitStatus reconciles.** The limit tree is firm-level policy at the
/// aggregating edge; the federation re-derives utilization over the gathered union
/// against the SAME limit tree the oracle holds, so a configured book-level vega cap
/// reads the same exposure / RAG on both.
#[tokio::test]
async fn federated_limit_status_reconciles() {
    tokio::time::timeout(TEST_DEADLINE, async {
        use celnet_limits::{LimitMetric, LimitScope, LimitSpec};
        let backends = boot_fleet(3).await;
        let fed = federated_edge(&backends).await;
        let oracle = single_node_edge(master_book());

        // Configure the SAME book-22 vega cap on BOTH the federated edge and the oracle
        // (firm-level policy lives at the aggregating edge).
        let cap = LimitSpec::hard(LimitMetric::Vega, 5.0e5);
        let scope = LimitScope::Book(CubeBookId(22));
        fed.store().set_limit(scope, cap);
        oracle.store().set_limit(scope, cap);

        let req = LimitStatusRequest {
            scope: Some(RiskScope {
                dimension: RiskDimension::Book as i32,
                value: 22,
            }),
            numeraire: Some(usd_numeraire()),
            principal: asserted_grant_all(),
            vega_pillars: vec![],
            var_spot_shocks: vec![],
            var_alpha: 0.0,
            correlation_id: Some(3),
            session_token: None,
        };
        let f = RiskService::limit_status(&fed, Request::new(req.clone()))
            .await
            .unwrap()
            .into_inner();
        let o = RiskService::limit_status(&oracle, Request::new(req))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(f.limits.len(), o.limits.len(), "same number of limits");
        assert_eq!(f.worst, o.worst, "same worst RAG");
        assert_eq!(f.hard_breach, o.hard_breach, "same hard-breach flag");
        for (fl, ol) in f.limits.iter().zip(o.limits.iter()) {
            assert!(
                (fl.exposure - ol.exposure).abs() <= 1e-6 + 1e-9 * ol.exposure.abs(),
                "limit exposure federated {} vs oracle {}",
                fl.exposure,
                ol.exposure
            );
            assert_eq!(fl.status, ol.status, "limit RAG");
        }
    })
    .await
    .expect("within deadline");
}

/// **Scale UP: add a 4th backend, re-partition + re-seed, assert the aggregate is
/// unchanged.** More shards, same union book ⇒ same firm number.
#[tokio::test]
async fn scale_up_is_invariant() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let oracle = single_node_edge(master_book());

        let three = boot_fleet(3).await;
        let fed3 = federated_edge(&three).await;
        assert_aggregate_reconciles(&fed3, &oracle, &firm_request_with_risk()).await;

        // Add a 4th backend: re-partition over 4 replicas, re-seed, re-federate.
        let four = boot_fleet(4).await;
        // The 4-way partition must still cover the whole book.
        let covered: usize = four.iter().map(|b| b.edge.store().len()).sum();
        assert_eq!(covered, master_book().len());
        let fed4 = federated_edge(&four).await;
        assert_aggregate_reconciles(&fed4, &oracle, &firm_request_with_risk()).await;
    })
    .await
    .expect("within deadline");
}

/// **Scale DOWN: remove a backend after re-homing its slice to survivors, assert the
/// aggregate is unchanged.** We model the re-home by re-partitioning the WHOLE book
/// over the smaller replica set (so every surviving backend owns its new slice) — the
/// union is still the whole book ⇒ same firm number.
#[tokio::test]
async fn scale_down_is_invariant() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let oracle = single_node_edge(master_book());

        // Start at 4, then re-home to 3 survivors (re-partition over 3 replicas).
        let four = boot_fleet(4).await;
        let fed4 = federated_edge(&four).await;
        assert_aggregate_reconciles(&fed4, &oracle, &firm_request_with_risk()).await;

        let three = boot_fleet(3).await; // the re-homed survivor fleet
        let covered: usize = three.iter().map(|b| b.edge.store().len()).sum();
        assert_eq!(
            covered,
            master_book().len(),
            "re-home covers the whole book"
        );
        let fed3 = federated_edge(&three).await;
        assert_aggregate_reconciles(&fed3, &oracle, &firm_request_with_risk()).await;
    })
    .await
    .expect("within deadline");
}

/// **Failover.** (a) A backend DOWN with a hot **standby** holding its slice → the
/// aggregate is unchanged (the standby is fanned to in its place). (b) A backend DOWN
/// with **no** standby → the federation returns `unavailable`, never a wrong (too
/// small) firm number.
#[tokio::test]
async fn failover_standby_then_unavailable() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let oracle = single_node_edge(master_book());

        // Boot 4 backends over a 3-way partition: replicas 1..=3 own the book; backend
        // 4 is a STANDBY pre-seeded with replica-2's exact slice (a hot shadow shard).
        let parts = partition(3);
        let b1 = boot_backend(ReplicaId(1), parts.get(&1).cloned().unwrap_or_default()).await;
        let b2 = boot_backend(ReplicaId(2), parts.get(&2).cloned().unwrap_or_default()).await;
        let b3 = boot_backend(ReplicaId(3), parts.get(&3).cloned().unwrap_or_default()).await;
        let standby_legs = parts.get(&2).cloned().unwrap_or_default();
        let b4 = boot_backend(ReplicaId(4), standby_legs).await;

        // (a) Membership: replica 2 DOWN with declared hot standby 4 (Up). Backend 2's
        // endpoint is not dialled; its slice is served by the standby (backend 4).
        let members_a: Vec<(String, Replica)> = vec![
            (b1.url.clone(), Replica::up(ReplicaId(1))),
            (
                b2.url.clone(),
                Replica::up(ReplicaId(2)).with_standby(ReplicaId(4)).down(),
            ),
            (b3.url.clone(), Replica::up(ReplicaId(3))),
            (b4.url.clone(), Replica::up(ReplicaId(4))),
        ];
        let fleet_a = Arc::new(
            Fleet::connect_with_membership(&members_a)
                .await
                .expect("standby fleet connects (down primary not dialled)"),
        );
        let gate = Arc::new(ReadinessGate::new());
        gate.mark_ready();
        let fed_a =
            RiskEdge::with_fleet(Arc::new(PositionStore::new()), Arc::clone(&gate), fleet_a);
        assert_aggregate_reconciles(&fed_a, &oracle, &firm_request_with_risk()).await;

        // (b) Membership: replica 2 DOWN with NO standby → uncovered slice → unavailable.
        let members_b: Vec<(String, Replica)> = vec![
            (b1.url.clone(), Replica::up(ReplicaId(1))),
            (b2.url.clone(), Replica::up(ReplicaId(2)).down()),
            (b3.url.clone(), Replica::up(ReplicaId(3))),
        ];
        let fleet_b = Arc::new(
            Fleet::connect_with_membership(&members_b)
                .await
                .expect("partial fleet connects (down replica not dialled)"),
        );
        let fed_b = RiskEdge::with_fleet(Arc::new(PositionStore::new()), gate, fleet_b);
        let err = RiskService::aggregate_risk(&fed_b, Request::new(firm_request_with_risk()))
            .await
            .expect_err("a down replica with no standby must fail, never a partial number");
        assert_eq!(
            err.code(),
            tonic::Code::Unavailable,
            "uncovered slice ⇒ unavailable (never a silently-smaller firm number)"
        );

        // Keep the standby health asserted at the type level.
        assert_eq!(b4.replica, ReplicaId(4));
    })
    .await
    .expect("within deadline");
}

// ---------------------------------------------------------------------------
// concurrency proof: a delaying backend (latency ~ max, not Σ)
// ---------------------------------------------------------------------------

/// A `RiskService` that **sleeps `delay` before every RPC**, then delegates to an
/// inner single-node [`RiskEdge`]. Standing N of these up and federating across them
/// makes the per-backend latency observable: a SEQUENTIAL fan-out would cost ~N×delay
/// (each await serialized), whereas the CONCURRENT fan-out under test costs ~delay
/// (all backends sleep at once) — which the latency test below asserts directly.
///
/// The delegate answers are real (the same algebra the reconciliation tests use), so
/// the response is a genuine slice, not a stub — only the *timing* is injected.
struct DelayingBackend {
    inner: RiskEdge,
    delay: Duration,
}

#[tonic::async_trait]
impl RiskService for DelayingBackend {
    async fn list_positions(
        &self,
        request: Request<ListPositionsRequest>,
    ) -> Result<Response<ListPositionsResponse>, Status> {
        tokio::time::sleep(self.delay).await;
        RiskService::list_positions(&self.inner, request).await
    }

    async fn aggregate_risk(
        &self,
        request: Request<AggregateRiskRequest>,
    ) -> Result<Response<AggregateRiskResponse>, Status> {
        tokio::time::sleep(self.delay).await;
        RiskService::aggregate_risk(&self.inner, request).await
    }

    async fn drill_risk(
        &self,
        request: Request<DrillRiskRequest>,
    ) -> Result<Response<DrillRiskResponse>, Status> {
        tokio::time::sleep(self.delay).await;
        RiskService::drill_risk(&self.inner, request).await
    }

    async fn limit_status(
        &self,
        request: Request<LimitStatusRequest>,
    ) -> Result<Response<LimitStatusResponse>, Status> {
        tokio::time::sleep(self.delay).await;
        RiskService::limit_status(&self.inner, request).await
    }

    async fn aggregate_rates_risk(
        &self,
        request: Request<celnet_proto::AggregateRatesRiskRequest>,
    ) -> Result<Response<celnet_proto::AggregateRatesRiskResponse>, Status> {
        tokio::time::sleep(self.delay).await;
        RiskService::aggregate_rates_risk(&self.inner, request).await
    }

    async fn book_rates_position(
        &self,
        request: Request<celnet_proto::BookRatesPositionRequest>,
    ) -> Result<Response<celnet_proto::BookRatesPositionResponse>, Status> {
        tokio::time::sleep(self.delay).await;
        RiskService::book_rates_position(&self.inner, request).await
    }

    async fn list_rates_positions(
        &self,
        request: Request<celnet_proto::ListRatesPositionsRequest>,
    ) -> Result<Response<celnet_proto::ListRatesPositionsResponse>, Status> {
        tokio::time::sleep(self.delay).await;
        RiskService::list_rates_positions(&self.inner, request).await
    }

    async fn combined_tail_risk(
        &self,
        request: Request<celnet_proto::CombinedTailRiskRequest>,
    ) -> Result<Response<celnet_proto::CombinedTailRiskResponse>, Status> {
        tokio::time::sleep(self.delay).await;
        RiskService::combined_tail_risk(&self.inner, request).await
    }
}

/// Boot a delaying `RiskService` backend (seeded with `legs`, sleeping `delay` before
/// each RPC) on an ephemeral 127.0.0.1 port over real gRPC, returning its dial URL.
/// The server task runs until the test process ends (the test is deadline-bounded).
async fn boot_delaying_backend(legs: Vec<Master>, delay: Duration) -> String {
    let gate = Arc::new(ReadinessGate::new());
    gate.mark_ready();
    let store = Arc::new(PositionStore::new());
    seed(&store, legs);
    let svc = DelayingBackend {
        inner: RiskEdge::new(store, gate),
        delay,
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let incoming =
        tonic::transport::server::TcpIncoming::from_listener(listener, true, None).unwrap();
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(RiskServiceServer::new(svc))
            .serve_with_incoming(incoming)
            .await
            .ok();
    });
    format!("http://{addr}")
}

/// **Genuine concurrency proof: federated fan-out latency ≈ slowest backend, NOT the
/// sum.** Stand up N real gRPC backends that EACH sleep `DELAY` before responding,
/// federate across them, and assert the federated `AggregateRisk` completes in well
/// under `N × DELAY` (it would take ≥ `N × DELAY` under the old sequential fan-out)
/// — and within a small multiple of a single `DELAY` (proving all backends were
/// dialled at once). Hard wall-clock bounded so a regression fails fast, never hangs.
#[tokio::test]
async fn federated_fan_out_latency_is_max_not_sum() {
    tokio::time::timeout(TEST_DEADLINE, async {
        const DELAY: Duration = Duration::from_millis(300);
        let n = 4u64;

        // Each backend owns its HRW slice and sleeps DELAY before answering.
        let parts = partition(n);
        let mut members: Vec<(String, Replica)> = Vec::new();
        for i in 1..=n {
            let legs = parts.get(&i).cloned().unwrap_or_default();
            let url = boot_delaying_backend(legs, DELAY).await;
            members.push((url, Replica::up(ReplicaId(i))));
        }

        let fleet = Arc::new(
            Fleet::connect_with_membership(&members)
                .await
                .expect("delaying fleet connects"),
        );
        let gate = Arc::new(ReadinessGate::new());
        gate.mark_ready();
        let fed = RiskEdge::with_fleet(Arc::new(PositionStore::new()), gate, fleet);

        // A non-additive request fans out TWICE (additive aggregate, then a re-gather
        // ListPositions per node) — the strongest sequential-cost case. Concurrency
        // must still keep each fan-out wave ~ one DELAY.
        let started = Instant::now();
        let resp = RiskService::aggregate_risk(&fed, Request::new(firm_request_with_risk()))
            .await
            .expect("federated aggregate over delaying backends");
        let elapsed = started.elapsed();

        // Correctness still holds against the single-node oracle (the delay is timing
        // only — the slices are real).
        let oracle = single_node_edge(master_book());
        let single = RiskService::aggregate_risk(&oracle, Request::new(firm_request_with_risk()))
            .await
            .expect("oracle aggregate")
            .into_inner();
        assert_nodes_eq(&resp.into_inner(), &single);

        // Sequential fan-out would cost ≥ n × DELAY for the additive wave alone, plus
        // another n × DELAY for the re-gather wave (≥ 2 n × DELAY ≈ 2.4 s). Concurrent
        // fan-out collapses each wave to ~one DELAY; with a couple of fan-out waves +
        // gRPC overhead we bound generously at 4 × DELAY and STRICTLY below the
        // sequential floor (n × DELAY) to prove genuine concurrency, not luck.
        let sequential_floor = DELAY * u32::try_from(n).unwrap(); // 1.2 s
        let concurrent_ceiling = DELAY * 4; // 1.2 s budget for ~2 concurrent waves + overhead
        assert!(
            elapsed < sequential_floor + DELAY, // < (n+1)×DELAY: cannot be the Σ-cost path
            "federated fan-out took {elapsed:?}; a sequential per-backend fan-out would be \
             ≥ {sequential_floor:?} (n×DELAY) per wave — concurrency regressed"
        );
        assert!(
            elapsed < concurrent_ceiling,
            "federated fan-out took {elapsed:?}, expected ≈ DELAY ({DELAY:?}) per concurrent \
             wave (ceiling {concurrent_ceiling:?})"
        );
    })
    .await
    .expect("within deadline");
}

/// **The HRW partition is a disjoint cover that genuinely fans across replicas.** For
/// 3 and 4 replicas, every master leg lands on exactly one owner and the book spreads
/// across more than one shard (so the reconciliation tests above are not vacuously
/// over a single backend). Also exercises the `Health` enum the reach model branches
/// on.
#[tokio::test]
async fn partition_is_a_disjoint_multi_shard_cover() {
    tokio::time::timeout(TEST_DEADLINE, async {
        assert_ne!(Health::Up, Health::Down); // the health states the reach model branches on
        for n in [3u64, 4] {
            let parts = partition(n);
            let total: usize = parts.values().map(Vec::len).sum();
            assert_eq!(
                total,
                master_book().len(),
                "{n}-way partition covers the book"
            );
            assert!(
                parts.values().filter(|v| !v.is_empty()).count() >= 2,
                "{n}-way partition fans across multiple shards"
            );
            // No leg appears on two shards.
            let mut seen = std::collections::HashSet::new();
            for legs in parts.values() {
                for m in legs {
                    assert!(seen.insert(m.id), "leg {} owned by exactly one shard", m.id);
                }
            }
        }
    })
    .await
    .expect("within deadline");
}
