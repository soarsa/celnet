//! End-to-end federation test for the `RiskService.AggregateRatesRisk` edge — the
//! fixed-income rates portfolio-risk rollup.
//!
//! This drives the **wired gRPC handler** on a ready [`RiskEdge`] (entitlement guard
//! and all) over a multi-entity, multi-book rates portfolio and proves the headline
//! invariant end-to-end: the sharded endpoint result equals the single-node
//! [`firm_aggregate_rates`](celnet_risk_fleet::firm_aggregate_rates) rollup over the
//! identically priced facts, **bit-for-bit** (the `celnet-risk-fleet` F5 reducer
//! guarantees the fan-in equals the single-node sum exactly, and this test surfaces
//! that through the real price → shard → rollup vertical).
//!
//! It also asserts the deny-by-default entitlement boundary (an unauthenticated
//! caller under `Enforce` is rejected, mirroring `AggregateRisk`), scope narrowing,
//! and per-currency isolation of the additive reducer.
//!
//! # Currency scope
//!
//! The P0 rates arm prices USD-SOFR OIS only (`crate::rates_pricing` rejects a
//! non-USD `CurveSet`), so an end-to-end request produces a single USD rollup node.
//! Per-currency *isolation* of the rollup algebra is therefore exercised directly
//! against the additive reducer with multi-currency facts (the path a multi-currency
//! curve arm will feed); the endpoint vertical covers the multi-entity USD book.

use std::sync::Arc;

use celnet_proto::risk_service_server::RiskService;
use celnet_proto::{
    AggregateRatesRiskRequest, BrokenDate, CurveSet, EntitlementPrincipal, OisInstrument,
    OisPillar, PillarTenor, RatesInstrument, RatesPosition, RatesRiskScope, Side, pillar_tenor,
    rates_instrument,
};
use celnet_risk_cube::{BookId, EntityId};
use celnet_risk_fleet::{KeyRateBucket, RatesFactKey, RatesRiskFact, firm_aggregate_rates};
use celnet_server::AccessMode;
use celnet_server::services::rates_risk::aggregate::single_node_aggregate;
use celnet_server::services::risk::RiskEdge;
use celnet_server::services::risk::store::PositionStore;
use celnet_server::{ReadinessGate, services};
use celnet_types::Ccy;
use tonic::{Code, Request};

/// A USD-SOFR curve on the 1/2/5/10y grid (the P0 calibrating pillars).
fn usd_curve() -> CurveSet {
    CurveSet {
        currency: "USD".to_owned(),
        reference_date: Some(BrokenDate {
            year: 2026,
            month: 6,
            day: 15,
        }),
        ois_pillars: [(1, 0.0402), (2, 0.0408), (5, 0.0421), (10, 0.0435)]
            .into_iter()
            .map(|(years, par_rate)| OisPillar {
                tenor: Some(PillarTenor {
                    point: Some(pillar_tenor::Point::Years(years)),
                }),
                par_rate,
            })
            .collect(),
    }
}

fn position(
    id: u64,
    entity: u32,
    book: u32,
    tenor: u32,
    fixed: f64,
    notional: f64,
    side: Side,
) -> RatesPosition {
    RatesPosition {
        position_id: id,
        entity,
        book,
        instrument: Some(RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                tenor_years: tenor,
                fixed_rate: fixed,
                notional,
                side: side as i32,
            })),
        }),
    }
}

/// A multi-entity, multi-book USD portfolio: 3 entities, payer + receiver legs at
/// 2/5/10y, so the rollup genuinely nets across entities and the facts fan across
/// more than one HRW shard.
fn firm_book() -> Vec<RatesPosition> {
    vec![
        position(1, 1, 100, 5, 0.041, 100_000_000.0, Side::Sell),
        position(2, 1, 101, 2, 0.040, 50_000_000.0, Side::Buy),
        position(3, 2, 200, 10, 0.043, 75_000_000.0, Side::Sell),
        position(4, 2, 201, 5, 0.042, 120_000_000.0, Side::Buy),
        position(5, 3, 300, 2, 0.0395, 90_000_000.0, Side::Sell),
        position(6, 3, 301, 10, 0.0445, 60_000_000.0, Side::Buy),
    ]
}

fn grant_all() -> EntitlementPrincipal {
    EntitlementPrincipal {
        grant_all: true,
        grants: vec![],
        denies: vec![],
    }
}

fn request(
    positions: Vec<RatesPosition>,
    scope: Option<RatesRiskScope>,
) -> AggregateRatesRiskRequest {
    AggregateRatesRiskRequest {
        curve_set: Some(usd_curve()),
        positions,
        scope,
        principal: Some(grant_all()),
        correlation_id: Some(4242),
        session_token: None,
    }
}

/// A ready [`RiskEdge`] under an explicit [`AccessMode`]. The rates risk rollup is
/// store-independent (positions travel inline), so the store carries only the
/// access posture the entitlement guard reads.
fn edge(mode: AccessMode) -> RiskEdge {
    let gate = Arc::new(ReadinessGate::new());
    gate.mark_ready();
    let store = Arc::new(PositionStore::new());
    store.set_access_mode(mode);
    RiskEdge::new(store, gate)
}

/// **THE HEADLINE INVARIANT, END-TO-END.** The wired gRPC endpoint's sharded rollup
/// over a multi-entity book equals the direct single-node `firm_aggregate_rates`
/// over the identically priced facts — bit-for-bit.
#[tokio::test]
async fn endpoint_equals_firm_aggregate_rates() {
    let req = request(firm_book(), None);

    let resp = edge(AccessMode::Enforce)
        .aggregate_rates_risk(Request::new(req.clone()))
        .await
        .expect("authorized aggregate")
        .into_inner();

    // The single-node reference rolls up the SAME priced + scoped facts directly via
    // firm_aggregate_rates (no partition fan-out).
    let reference = single_node_aggregate(&req).expect("single-node reference");

    assert_eq!(resp.correlation_id, Some(4242));
    assert_eq!(
        resp.nodes.len(),
        reference.nodes.len(),
        "currency node count"
    );
    assert_eq!(resp.nodes.len(), 1, "USD-only P0 book ⇒ one currency node");
    for (got, want) in resp.nodes.iter().zip(&reference.nodes) {
        assert_eq!(got.ccy, want.ccy);
        assert_eq!(got.net_pv.to_bits(), want.net_pv.to_bits(), "net_pv");
        assert_eq!(got.net_pv01.to_bits(), want.net_pv01.to_bits(), "net_pv01");
        assert_eq!(got.net_dv01.to_bits(), want.net_dv01.to_bits(), "net_dv01");
        assert_eq!(
            got.key_rate_ladder.len(),
            want.key_rate_ladder.len(),
            "ladder length"
        );
        for (gb, wb) in got.key_rate_ladder.iter().zip(&want.key_rate_ladder) {
            assert_eq!(gb.tenor_years, wb.tenor_years, "ladder tenor");
            assert_eq!(gb.dv01.to_bits(), wb.dv01.to_bits(), "ladder dv01");
        }
    }

    // The USD node is non-trivial: a real net DV01 and a 1/2/5/10y ladder.
    let usd = &resp.nodes[0];
    assert_eq!(usd.ccy, "USD");
    let tenors: Vec<u32> = usd.key_rate_ladder.iter().map(|b| b.tenor_years).collect();
    assert_eq!(tenors, vec![1, 2, 5, 10]);
    // The ladder sums (to first order) to the parallel DV01 of the netting set.
    let ladder_sum: f64 = usd.key_rate_ladder.iter().map(|b| b.dv01).sum();
    assert!(
        (ladder_sum - usd.net_dv01).abs() < 1e-3 * usd.net_dv01.abs().max(1.0),
        "ladder Σ {ladder_sum} ≉ net_dv01 {}",
        usd.net_dv01
    );
}

/// Deny-by-default: under `Enforce`, a caller presenting neither a session token nor
/// an entitlement principal is rejected — exactly the guard `AggregateRisk` applies.
#[tokio::test]
async fn unauthenticated_caller_is_denied_under_enforce() {
    let req = AggregateRatesRiskRequest {
        curve_set: Some(usd_curve()),
        positions: firm_book(),
        scope: None,
        principal: None,
        correlation_id: Some(1),
        session_token: None,
    };
    let err = edge(AccessMode::Enforce)
        .aggregate_rates_risk(Request::new(req))
        .await
        .expect_err("unauthenticated caller must be denied");
    assert_eq!(err.code(), Code::Unauthenticated);
}

/// The same anonymous request is admitted under `Permissive` (the explicit
/// absent-⇒-grant-all posture) — proving the denial above is the mode-gated guard,
/// not an unrelated failure.
#[tokio::test]
async fn anonymous_caller_admitted_under_permissive() {
    let req = AggregateRatesRiskRequest {
        curve_set: Some(usd_curve()),
        positions: firm_book(),
        scope: None,
        principal: None,
        correlation_id: Some(2),
        session_token: None,
    };
    let resp = edge(AccessMode::Permissive)
        .aggregate_rates_risk(Request::new(req))
        .await
        .expect("permissive admits anonymous")
        .into_inner();
    assert_eq!(resp.nodes.len(), 1);
}

/// A scope filter narrows the rollup before netting: an entity filter rolls up only
/// that entity's positions, matching the single-node rollup of that subset.
#[tokio::test]
async fn entity_scope_narrows_endpoint_rollup() {
    let scoped_req = request(
        firm_book(),
        Some(RatesRiskScope {
            entity: Some(2),
            book: None,
            ccy: None,
        }),
    );
    let resp = edge(AccessMode::Enforce)
        .aggregate_rates_risk(Request::new(scoped_req.clone()))
        .await
        .expect("scoped aggregate")
        .into_inner();

    // Reference: only entity-2 positions, rolled single-node.
    let entity2_only = request(
        firm_book().into_iter().filter(|p| p.entity == 2).collect(),
        None,
    );
    let reference = single_node_aggregate(&entity2_only).expect("entity-2 reference");

    assert_eq!(resp.nodes.len(), 1);
    assert_eq!(
        resp.nodes[0].net_pv.to_bits(),
        reference.nodes[0].net_pv.to_bits(),
        "scoped net_pv must equal the entity-2 subset rollup"
    );
    assert_eq!(
        resp.nodes[0].net_dv01.to_bits(),
        reference.nodes[0].net_dv01.to_bits(),
    );
}

/// Per-currency isolation of the additive rollup algebra: currencies never
/// cross-net (the path a multi-currency curve arm feeds). Exercised directly against
/// the reducer with USD + EUR facts, since the P0 endpoint is USD-only.
#[test]
fn currencies_do_not_cross_net() {
    let usd = Ccy::USD;
    let eur = Ccy::parse("EUR").unwrap();
    let facts = vec![
        RatesRiskFact {
            key: RatesFactKey {
                entity: EntityId(1),
                ccy: usd,
                book: BookId(1),
            },
            pv: 12_500.0,
            pv01: 830.0,
            dv01: -829.4,
            key_rate_ladder: vec![KeyRateBucket {
                tenor_years: 5,
                dv01: -829.4,
            }],
        },
        RatesRiskFact {
            key: RatesFactKey {
                entity: EntityId(1),
                ccy: eur,
                book: BookId(2),
            },
            pv: 4_100.0,
            pv01: 560.0,
            dv01: -559.6,
            key_rate_ladder: vec![KeyRateBucket {
                tenor_years: 2,
                dv01: -559.6,
            }],
        },
    ];
    let rollup = firm_aggregate_rates(&facts);
    assert_eq!(rollup.books().len(), 2, "USD and EUR net separately");
    let usd_node = rollup.book(usd).expect("USD node");
    let eur_node = rollup.book(eur).expect("EUR node");
    assert!((usd_node.net_pv - 12_500.0).abs() < 1e-9);
    assert!((eur_node.net_pv - 4_100.0).abs() < 1e-9);

    // Renders to two wire nodes via the same converter the endpoint uses.
    let resp = services::rates_risk::convert::rollup_to_response(&rollup, None);
    assert_eq!(resp.nodes.len(), 2);
}
