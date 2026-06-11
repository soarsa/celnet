//! **Entitlements trust-boundary suite** (`docs/RISK-HIERARCHY.md` §4): the
//! deny-by-default authorization decision + per-decision audit on every
//! entitlement-gated service.
//!
//! What is proven here, through the public service surface (the same `tonic`
//! trait methods the gRPC server hosts and the WS mirror dispatches onto — one
//! boundary, two encodings):
//!
//! 1. a request asserting **no** principal is denied with a typed
//!    `unauthenticated` error on **every** entitlement-gated RPC — the complete
//!    set being the four `RiskService` methods (`ListPositions`,
//!    `AggregateRisk`, `DrillRisk`, `LimitStatus`; no other service's request
//!    carries an `EntitlementPrincipal` in `celnet.proto`);
//! 2. the **explicit permissive dev-mode** (the demo-edge affordance) admits an
//!    absent principal as grant-all — opt-in, never the default;
//! 3. **every decision is audited**, allow and deny alike, as a structured
//!    security-class record carrying principal / resource / decision / reason
//!    (+ correlation id), captured here through the real JSON subscriber;
//! 4. the boundary's doc comment states the trust model **honestly**:
//!    transport-level authentication is ENV/deploy configuration; in-repo code
//!    enforces the authorization *decision* boundary.

use std::sync::{Arc, Mutex};

use celnet_entitlements::AccessMode;
use celnet_observability::{LogConfig, build_json_subscriber};
use celnet_proto::risk_service_server::RiskService;
use celnet_proto::{
    AggregateRiskRequest, AttributionRecord, BookId as WireBookId, DrillRiskRequest,
    EntitlementPrincipal, EntitlementRule, LimitStatusRequest, ListPositionsRequest,
    NumeraireRate, Owner, ReportingNumeraire, RiskDimension, RiskScope, owner,
};
use celnet_server::ReadinessGate;
use celnet_server::services::risk::RiskEdge;
use celnet_server::services::risk::store::{BookedPosition, PositionStore};
use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};
use tonic::{Code, Request};

// ---------------------------------------------------------------------------
// harness: a ready in-process edge over a two-book live book
// ---------------------------------------------------------------------------

fn usd_numeraire() -> ReportingNumeraire {
    ReportingNumeraire {
        numeraire: "USD".to_owned(),
        rates: vec![NumeraireRate {
            ccy: "EUR".to_owned(),
            rate: 1.10,
        }],
    }
}

fn attribution(book: &str, trader: &str) -> AttributionRecord {
    AttributionRecord {
        quoted_by: Some(WireBookId {
            book: "AUTO-MM".to_owned(),
            owner: Some(Owner {
                seat: Some(owner::Seat::AutoPricer("celnet-auto-pricer".to_owned())),
            }),
        }),
        held_by: Some(WireBookId {
            book: book.to_owned(),
            owner: Some(Owner {
                seat: Some(owner::Seat::Trader(trader.to_owned())),
            }),
        }),
        won: Some(true),
        lp_count: Some(2),
    }
}

fn booked(id: u64, notional: f64) -> BookedPosition {
    BookedPosition {
        position_id: id,
        pair: CcyPair::new(Ccy::EUR, Ccy::USD),
        option: OptionType::Call,
        notional_base: notional,
        inputs: VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        quoted_delta: DeltaConvention::SpotUnadjusted,
        premium_style: PremiumStyle::DomesticPips,
        surface_version: 1,
    }
}

/// A ready, in-process `RiskEdge` (construction-default access mode:
/// deny-by-default `Enforce`) over a live book with one position in each of two
/// holder books. Returns the edge plus the interned handle of `"BOOK-A"` for
/// scoped-principal assertions.
fn ready_edge() -> (RiskEdge, u32) {
    let gate = Arc::new(ReadinessGate::new());
    gate.mark_ready();
    let store = Arc::new(PositionStore::new());
    store
        .book_from_attribution(booked(1, 10_000_000.0), &attribution("BOOK-A", "jdoe"))
        .expect("book position 1");
    store
        .book_from_attribution(booked(2, 5_000_000.0), &attribution("BOOK-B", "asmith"))
        .expect("book position 2");
    let book_a = store.intern("BOOK-A");
    (RiskEdge::new(store, gate), book_a)
}

fn asserted_grant_all() -> Option<EntitlementPrincipal> {
    Some(EntitlementPrincipal {
        grant_all: true,
        grants: vec![],
        denies: vec![],
    })
}

/// A principal asserting visibility over exactly one book subtree.
fn asserted_book(book: u32) -> Option<EntitlementPrincipal> {
    Some(EntitlementPrincipal {
        grant_all: false,
        grants: vec![EntitlementRule {
            scopes: vec![RiskScope {
                dimension: RiskDimension::Book as i32,
                value: u64::from(book),
            }],
        }],
        denies: vec![],
    })
}

fn list_request(principal: Option<EntitlementPrincipal>) -> ListPositionsRequest {
    ListPositionsRequest {
        scope: None,
        principal,
        correlation_id: Some(42),
    }
}

fn aggregate_request(principal: Option<EntitlementPrincipal>) -> AggregateRiskRequest {
    AggregateRiskRequest {
        dimension: RiskDimension::Firm as i32,
        numeraire: Some(usd_numeraire()),
        principal,
        scope: None,
        vega_pillars: vec![],
        var_spot_shocks: vec![],
        var_alpha: 0.0,
        curvature_risk_weight: 0.0,
        correlation_id: Some(42),
    }
}

fn drill_request(principal: Option<EntitlementPrincipal>) -> DrillRiskRequest {
    DrillRiskRequest {
        node: Some(RiskScope {
            dimension: RiskDimension::Firm as i32,
            value: 0,
        }),
        child_dimension: RiskDimension::Underlying as i32,
        numeraire: Some(usd_numeraire()),
        principal,
        vega_pillars: vec![],
        include_children: true,
        include_positions: true,
        correlation_id: Some(42),
    }
}

fn limit_request(principal: Option<EntitlementPrincipal>, book: u32) -> LimitStatusRequest {
    LimitStatusRequest {
        scope: Some(RiskScope {
            dimension: RiskDimension::Book as i32,
            value: u64::from(book),
        }),
        numeraire: Some(usd_numeraire()),
        principal,
        vega_pillars: vec![],
        var_spot_shocks: vec![],
        var_alpha: 0.0,
        correlation_id: Some(42),
    }
}

// ---------------------------------------------------------------------------
// 1. deny-by-default: an absent principal is refused on every gated service
// ---------------------------------------------------------------------------

/// An absent principal is denied with a typed `unauthenticated` error on
/// **each** of the four entitlement-gated RPCs — the complete gated set of the
/// contract (no other service's request carries an `EntitlementPrincipal`).
/// The error message names the boundary so a caller knows what to do.
#[tokio::test]
async fn absent_principal_denied_on_every_entitlement_gated_service() {
    let (edge, book_a) = ready_edge();

    let list = RiskService::list_positions(&edge, Request::new(list_request(None)))
        .await
        .expect_err("ListPositions with no principal must be denied");
    let agg = RiskService::aggregate_risk(&edge, Request::new(aggregate_request(None)))
        .await
        .expect_err("AggregateRisk with no principal must be denied");
    let drill = RiskService::drill_risk(&edge, Request::new(drill_request(None)))
        .await
        .expect_err("DrillRisk with no principal must be denied");
    let limit = RiskService::limit_status(&edge, Request::new(limit_request(None, book_a)))
        .await
        .expect_err("LimitStatus with no principal must be denied");

    for (rpc, err) in [
        ("ListPositions", &list),
        ("AggregateRisk", &agg),
        ("DrillRisk", &drill),
        ("LimitStatus", &limit),
    ] {
        assert_eq!(
            err.code(),
            Code::Unauthenticated,
            "{rpc}: an absent principal is an authentication-shaped refusal, got {err:?}"
        );
        assert!(
            err.message().contains("denied by default"),
            "{rpc}: the error names the deny-by-default boundary: {}",
            err.message()
        );
        assert!(
            err.message().contains(rpc),
            "{rpc}: the error names the refused resource: {}",
            err.message()
        );
    }
}

// ---------------------------------------------------------------------------
// 2. the explicit permissive dev-mode (the demo-edge affordance)
// ---------------------------------------------------------------------------

/// Flipping the shared store to the **explicit** permissive dev-mode admits an
/// absent principal as grant-all on all four RPCs — the demo edge's documented
/// affordance, opt-in only (the construction default is proven deny-by-default
/// above and re-checked here).
#[tokio::test]
async fn permissive_dev_mode_explicitly_grants_absent_principal() {
    let (edge, book_a) = ready_edge();
    assert_eq!(
        edge.store().access_mode(),
        AccessMode::Enforce,
        "the construction default is deny-by-default; permissive is opt-in only"
    );
    edge.store().set_access_mode(AccessMode::Permissive);
    assert_eq!(edge.store().access_mode(), AccessMode::Permissive);

    let list = RiskService::list_positions(&edge, Request::new(list_request(None)))
        .await
        .expect("permissive dev-mode admits the absent principal")
        .into_inner();
    assert_eq!(
        list.positions.len(),
        2,
        "the dev-mode grant-all substitution sees the whole demo book"
    );

    let agg = RiskService::aggregate_risk(&edge, Request::new(aggregate_request(None)))
        .await
        .expect("AggregateRisk under permissive dev-mode")
        .into_inner();
    assert_eq!(agg.nodes.len(), 1, "one firm apex node over the whole book");

    let drill = RiskService::drill_risk(&edge, Request::new(drill_request(None)))
        .await
        .expect("DrillRisk under permissive dev-mode")
        .into_inner();
    assert_eq!(drill.positions.len(), 2);

    RiskService::limit_status(&edge, Request::new(limit_request(None, book_a)))
        .await
        .expect("LimitStatus under permissive dev-mode");

    // Flipping back restores the deny-by-default boundary (runtime-coherent).
    edge.store().set_access_mode(AccessMode::Enforce);
    let err = RiskService::list_positions(&edge, Request::new(list_request(None)))
        .await
        .expect_err("enforce restored ⇒ absent principal denied again");
    assert_eq!(err.code(), Code::Unauthenticated);
}

// ---------------------------------------------------------------------------
// 3. asserted principals are honored (and pruned) under enforcement
// ---------------------------------------------------------------------------

/// Under the deny-by-default mode, an **asserted** principal is authorized and
/// applied as the pre-aggregation pruning predicate: an asserted grant-all sees
/// the whole book; a principal asserting only `BOOK-A` sees exactly its book —
/// the boundary composes with the entitlement predicate, end to end through the
/// service surface. (Binding the assertion to an authenticated caller identity
/// is the transport/deploy layer — the model the boundary documents honestly.)
#[tokio::test]
async fn asserted_principals_are_authorized_and_pruned_under_enforcement() {
    let (edge, book_a) = ready_edge();

    let all = RiskService::list_positions(&edge, Request::new(list_request(asserted_grant_all())))
        .await
        .expect("asserted grant-all is authorized")
        .into_inner();
    assert_eq!(all.positions.len(), 2, "asserted grant-all sees both books");

    let scoped =
        RiskService::list_positions(&edge, Request::new(list_request(asserted_book(book_a))))
            .await
            .expect("asserted scoped principal is authorized")
            .into_inner();
    assert_eq!(scoped.positions.len(), 1, "the BOOK-A principal sees exactly its book");
    assert_eq!(scoped.positions[0].position_id, 1);

    // A malformed assertion (unknown dimension) is refused loudly at the
    // boundary — typed `invalid_argument`, never partially honored.
    let malformed = Some(EntitlementPrincipal {
        grant_all: false,
        grants: vec![EntitlementRule {
            scopes: vec![RiskScope {
                dimension: 9999,
                value: 1,
            }],
        }],
        denies: vec![],
    });
    let err = RiskService::list_positions(&edge, Request::new(list_request(malformed)))
        .await
        .expect_err("a malformed principal is rejected at the boundary");
    assert_eq!(err.code(), Code::InvalidArgument);
}

// ---------------------------------------------------------------------------
// 4. per-decision audit: allow AND deny each emit one structured record
// ---------------------------------------------------------------------------

/// A cloneable line sink the real JSON subscriber writes into, so the test
/// reads back exactly what an operator's log pipeline would receive.
#[derive(Clone)]
struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("log buffer lock").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Parse the captured line-delimited JSON and return the entitlement-decision
/// audit records (each as a parsed JSON object).
fn decision_records(buf: &Arc<Mutex<Vec<u8>>>) -> Vec<serde_json::Value> {
    let bytes = buf.lock().expect("log buffer lock").clone();
    String::from_utf8(bytes)
        .expect("the JSON subscriber emits UTF-8")
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| v["audit"] == "entitlement_decision")
        .collect()
}

/// Every authorization decision — deny (enforce + absent), allow (asserted),
/// and the permissive dev-mode absent-admission — emits exactly one structured
/// `security`-class audit record carrying principal / resource / decision /
/// reason and the request correlation id, through the real structured-logging
/// pipeline (the same `tracing` → JSON-subscriber path the trade-lifecycle
/// audit mirror uses). The drained log is the independent oracle here: the
/// assertions read what an operator would, not internal counters.
#[tokio::test]
async fn audit_records_emitted_for_allow_and_deny() {
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
    let sink = CapturedLog(Arc::clone(&buf));
    let subscriber = build_json_subscriber(
        &LogConfig {
            filter: Some("info".to_owned()),
            ..LogConfig::default()
        },
        move || sink.clone(),
    );
    // Thread-scoped install: the current-thread test runtime executes the
    // service futures on this thread, so every decision lands in `buf`.
    let _guard = tracing::subscriber::set_default(subscriber);

    let (edge, _book_a) = ready_edge();

    // DENY: enforce + absent principal.
    let _ = RiskService::list_positions(&edge, Request::new(list_request(None)))
        .await
        .expect_err("denied under enforce");
    // ALLOW: an asserted grant-all principal.
    let _ =
        RiskService::aggregate_risk(&edge, Request::new(aggregate_request(asserted_grant_all())))
            .await
            .expect("allowed under enforce with an asserted principal");
    // ALLOW (dev-mode): permissive + absent principal — audited per decision,
    // so a permissive edge is visible in the security log on every request.
    edge.store().set_access_mode(AccessMode::Permissive);
    let _ = RiskService::list_positions(&edge, Request::new(list_request(None)))
        .await
        .expect("allowed under permissive dev-mode");

    let records = decision_records(&buf);
    assert_eq!(records.len(), 3, "exactly one audit record per decision: {records:#?}");

    let deny = &records[0];
    assert_eq!(deny["class"], "security");
    assert_eq!(deny["resource"], "RiskService/ListPositions");
    assert_eq!(deny["principal"], "absent");
    assert_eq!(deny["decision"], "deny");
    assert_eq!(deny["reason"], "principal_absent");
    assert_eq!(deny["correlation_id"], 42);

    let allow = &records[1];
    assert_eq!(allow["class"], "security");
    assert_eq!(allow["resource"], "RiskService/AggregateRisk");
    assert_eq!(allow["principal"], "grant-all(+0 denies)");
    assert_eq!(allow["decision"], "allow");
    assert_eq!(allow["reason"], "principal_asserted");

    let dev = &records[2];
    assert_eq!(dev["class"], "security");
    assert_eq!(dev["resource"], "RiskService/ListPositions");
    assert_eq!(dev["principal"], "absent");
    assert_eq!(dev["decision"], "allow");
    assert_eq!(dev["reason"], "permissive_dev_mode_absent_principal");
}

// ---------------------------------------------------------------------------
// 5. the boundary documents the trust model honestly
// ---------------------------------------------------------------------------

/// The boundary's module documentation states the trust model without
/// overclaiming: transport-level authentication (binding the asserted principal
/// to a real caller) is the deployment environment's job; what the repo
/// enforces is the authorization DECISION boundary, deny-by-default, with the
/// permissive dev-mode explicit and loud. A doc drift here is a compliance
/// defect, so the suite pins the load-bearing sentences.
#[test]
fn boundary_doc_states_the_trust_model_honestly() {
    let boundary_source = include_str!("../src/services/access.rs");
    for load_bearing in [
        "Transport-level authentication is the deployment environment's job.",
        "What the repo enforces is the authorization DECISION boundary.",
        "denied by default",
        "Permissive dev-mode is explicit and loud, never silent.",
        "Every decision is audited — allow and deny.",
    ] {
        assert!(
            boundary_source.contains(load_bearing),
            "the trust-boundary doc must state: {load_bearing:?}"
        );
    }
}
