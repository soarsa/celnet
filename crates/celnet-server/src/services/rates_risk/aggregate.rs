//! The rates portfolio-risk **aggregation engine** — the pure core of the
//! `RiskService.AggregateRatesRisk` edge RPC, the linear-rates analogue of
//! [`super::super::risk::aggregate`].
//!
//! The vertical is *positions → price → shard → rollup*:
//!
//! 1. **Price** each [`RatesPosition`] against the request's [`CurveSet`] into one
//!    additive [`RatesRiskFact`] ([`super::convert::fact_from_position`]) — reusing
//!    the `PricingService.PriceRates` engine entry, never re-deriving OIS math.
//! 2. **Scope** the facts by the optional `(entity, book, ccy)` filter before the
//!    rollup (the rates-native analogue of the options pre-aggregation prune).
//! 3. **Shard** the facts across the firm HRW [`PartitionMap`] by their
//!    `(entity, ccy)` cell ([`partition_rates_facts`]) and **fan them in**
//!    additively ([`RatesFleetReducer::fan_in_additive`]).
//! 4. **Convert** the per-currency [`RatesFirmRollup`] to the wire response
//!    ([`super::convert::rollup_to_response`]).
//!
//! # Why the partition fan-in (not a bare single-node sum)
//!
//! Rates risk is *purely additive*, and the F5 reducer pins a fixed
//! `(ccy, entity, tenor)` summation order that is a pure function of the facts, so
//! the sharded fan-in equals the single-node [`firm_aggregate_rates`] **bit-for-bit**
//! regardless of how the HRW map lays the cells out (`celnet-risk-fleet`). Driving
//! the rollup through the partition map — rather than a bare local sum — exercises
//! the real firm fan-in path the GUI Risk/Book workspaces consume, and the
//! bit-exact invariant makes the logical-shard count immaterial to the answer.
//!
//! # Honest scope
//!
//! The cross-node transport is designed-only (`docs/SCALE-OUT.md` §0), exactly as
//! for the options fleet: the fan-out runs over an **in-process logical-shard**
//! `ReplicaSet` derived from the edge [`FleetTopology`] (one logical replica per
//! configured backend, or a fixed in-process fabric when co-resident). Positions
//! travel inline on the request; a persisted, execution-fed firm rates position
//! store is a separate slice (`docs/fixed-income/FI-ARCHITECTURE.md`).

// `tonic::Status` is a large error type carried by value across the whole
// `RiskService` surface (mirrors `services::risk`).
#![allow(clippy::result_large_err)]

use celnet_proto::{AggregateRatesRiskRequest, AggregateRatesRiskResponse, RatesRiskScope};
use celnet_risk_fleet::{
    FleetTopology, RatesRiskFact, firm_aggregate_rates, partition_rates_facts,
};
use celnet_router::{PartitionMap, Replica, ReplicaId, ReplicaSet};
use tonic::Status;

use super::convert::{fact_from_position, rollup_to_response};

/// The number of in-process **logical shards** a co-resident ([`FleetTopology::InProcess`])
/// rates fan-out spreads its facts across. Any value ≥ 1 yields the identical
/// rollup (the fan-in is bit-for-bit equal to the single-node sum), so this only
/// governs how many shards genuinely participate in the fan-in path; a small fan
/// keeps the in-process simulation cheap while still exercising a multi-shard merge.
const IN_PROCESS_RATES_SHARDS: u64 = 8;

/// Whether a [`RatesRiskFact`] passes the request's optional `(entity, book, ccy)`
/// scope: each present filter must match (absent fields do not constrain).
fn fact_in_scope(fact: &RatesRiskFact, scope: &RatesRiskScope) -> bool {
    if scope.entity.is_some_and(|e| fact.key.entity.0 != e) {
        return false;
    }
    if scope.book.is_some_and(|b| fact.key.book.0 != b) {
        return false;
    }
    if scope
        .ccy
        .as_deref()
        .is_some_and(|c| !fact.key.ccy.as_str().eq_ignore_ascii_case(c))
    {
        return false;
    }
    true
}

/// The in-process logical-shard membership the rates fan-out partitions over,
/// derived from the edge [`FleetTopology`]: one logical replica per configured
/// backend for a distributed edge (the same membership the options fan-out dials),
/// or a fixed [`IN_PROCESS_RATES_SHARDS`]-wide fabric when co-resident. Replica ids
/// are `1..=N` in order, mirroring [`super::super::risk::federate`].
///
/// # Errors
/// [`Status::internal`] if the assembled membership is invalid (never reached for
/// the fixed `1..=N` ids, which are always distinct and non-empty).
fn rates_partition_replicas(topology: &FleetTopology) -> Result<ReplicaSet, Status> {
    let count = match topology {
        FleetTopology::InProcess => IN_PROCESS_RATES_SHARDS,
        // One logical shard per backend endpoint (≥ 1); the physical cross-node
        // transport is designed-only, so the fan-in still runs in-process here.
        FleetTopology::Distributed { endpoints } => (endpoints.len() as u64).max(1),
    };
    let replicas = (1..=count).map(|i| Replica::up(ReplicaId(i))).collect();
    ReplicaSet::new(replicas)
        .map_err(|e| Status::internal(format!("invalid rates fan-out membership: {e}")))
}

/// Price every request position into its additive [`RatesRiskFact`] and narrow by
/// the optional `(entity, book, ccy)` scope **before** the rollup — the shared
/// front half of both the sharded ([`aggregate_rates_risk`]) and the single-node
/// ([`single_node_aggregate`]) paths, so they roll up the *identical* fact set.
///
/// # Errors
/// `invalid_argument` for a missing `curve_set`, an invalid curve currency, or a
/// position with no instrument; `internal` for a numeric bootstrap failure.
pub fn priced_facts(req: &AggregateRatesRiskRequest) -> Result<Vec<RatesRiskFact>, Status> {
    let curve_set = req
        .curve_set
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("AggregateRatesRisk missing `curve_set`"))?;
    let mut facts: Vec<RatesRiskFact> = Vec::with_capacity(req.positions.len());
    for position in &req.positions {
        let fact = fact_from_position(position, curve_set)?;
        if req.scope.as_ref().is_none_or(|s| fact_in_scope(&fact, s)) {
            facts.push(fact);
        }
    }
    Ok(facts)
}

/// Price, scope, shard, and roll up a rates portfolio into the per-currency firm
/// rollup response. Pure with respect to the edge: the market is the
/// request-supplied [`CurveSet`], so every replica computes the identical result.
///
/// # Errors
/// * `invalid_argument` if the request carries no `curve_set`, an invalid curve
///   currency, or a position with no instrument / malformed economics.
/// * `internal` for a numeric bootstrap failure or an unroutable partition.
pub fn aggregate_rates_risk(
    req: &AggregateRatesRiskRequest,
    topology: &FleetTopology,
) -> Result<AggregateRatesRiskResponse, Status> {
    let facts = priced_facts(req)?;

    // Shard across the firm HRW partition map and fan in additively. The fan-in is
    // bit-for-bit equal to `firm_aggregate_rates` over the same facts (asserted by
    // the federation integration test and the `celnet-risk-fleet` invariant).
    let replicas = rates_partition_replicas(topology)?;
    let map = PartitionMap::new(&replicas);
    let reducer = partition_rates_facts(&facts, &map)
        .map_err(|e| Status::internal(format!("rates fan-out routing failed: {e}")))?;
    let rollup = reducer.fan_in_additive();

    Ok(rollup_to_response(&rollup, req.correlation_id))
}

/// The **single-node reference** rollup for a request: price + scope the identical
/// facts, then roll them up directly via [`firm_aggregate_rates`] (no partition
/// fan-out). This is the ground truth the sharded [`aggregate_rates_risk`] reconciles
/// against bit-for-bit — the federation integration test asserts the two agree
/// end-to-end over the same positions.
///
/// # Errors
/// Identical to [`priced_facts`].
pub fn single_node_aggregate(
    req: &AggregateRatesRiskRequest,
) -> Result<AggregateRatesRiskResponse, Status> {
    let facts = priced_facts(req)?;
    Ok(rollup_to_response(
        &firm_aggregate_rates(&facts),
        req.correlation_id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{
        BrokenDate, CurveSet, OisInstrument, OisPillar, RatesInstrument, RatesPosition, Side,
        rates_instrument,
    };

    fn curve() -> CurveSet {
        CurveSet {
            currency: "USD".to_owned(),
            reference_date: Some(BrokenDate {
                year: 2026,
                month: 6,
                day: 15,
            }),
            ois_pillars: [(1, 0.0405), (2, 0.0410), (5, 0.0420), (10, 0.0430)]
                .into_iter()
                .map(|(years, par_rate)| OisPillar {
                    tenor: Some(crate::rates_pricing::years_pillar(years)),
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
                    notional: 100_000_000.0,
                    side: side as i32,
                })),
            }),
        }
    }

    fn request(
        positions: Vec<RatesPosition>,
        scope: Option<RatesRiskScope>,
    ) -> AggregateRatesRiskRequest {
        AggregateRatesRiskRequest {
            curve_set: Some(curve()),
            positions,
            scope,
            principal: None,
            correlation_id: Some(77),
            session_token: None,
        }
    }

    /// The sharded endpoint rollup equals the direct single-node rollup over the
    /// same priced facts, bit-for-bit (the F5 invariant, surfaced end-to-end).
    #[test]
    fn endpoint_equals_single_node_rollup() {
        let positions = vec![
            position(1, 1, 100, 5, 0.041, Side::Sell),
            position(2, 1, 101, 2, 0.040, Side::Buy),
            position(3, 2, 200, 10, 0.043, Side::Sell),
        ];
        let req = request(positions.clone(), None);

        let single = single_node_aggregate(&req).unwrap();
        let sharded = aggregate_rates_risk(&req, &FleetTopology::InProcess).unwrap();

        assert_eq!(sharded.correlation_id, Some(77));
        assert_eq!(sharded.nodes.len(), single.nodes.len());
        for (s, d) in sharded.nodes.iter().zip(&single.nodes) {
            assert_eq!(s.ccy, d.ccy);
            assert_eq!(s.net_pv.to_bits(), d.net_pv.to_bits());
            assert_eq!(s.net_pv01.to_bits(), d.net_pv01.to_bits());
            assert_eq!(s.net_dv01.to_bits(), d.net_dv01.to_bits());
            assert_eq!(s.key_rate_ladder.len(), d.key_rate_ladder.len());
            for (sb, db) in s.key_rate_ladder.iter().zip(&d.key_rate_ladder) {
                assert_eq!(sb.tenor_years, db.tenor_years);
                assert_eq!(sb.dv01.to_bits(), db.dv01.to_bits());
            }
        }
    }

    /// A `(payer, receiver)` pair on the SAME schedule and curve nets to ~zero
    /// PV/DV01 — the additive rollup genuinely sums opposing positions.
    #[test]
    fn payer_and_receiver_offset() {
        let req = request(
            vec![
                position(1, 1, 100, 5, 0.041, Side::Sell),
                position(2, 1, 100, 5, 0.041, Side::Buy),
            ],
            None,
        );
        let resp = aggregate_rates_risk(&req, &FleetTopology::InProcess).unwrap();
        assert_eq!(resp.nodes.len(), 1);
        let usd = &resp.nodes[0];
        assert_eq!(usd.ccy, "USD");
        assert!(usd.net_pv.abs() < 1e-6, "net_pv {}", usd.net_pv);
        assert!(usd.net_dv01.abs() < 1e-6, "net_dv01 {}", usd.net_dv01);
        for bucket in &usd.key_rate_ladder {
            assert!(
                bucket.dv01.abs() < 1e-6,
                "{}y dv01 {}",
                bucket.tenor_years,
                bucket.dv01
            );
        }
    }

    /// The scope filter narrows the rollup before netting: an entity filter keeps
    /// only that entity's positions.
    #[test]
    fn entity_scope_narrows_rollup() {
        let positions = vec![
            position(1, 1, 100, 5, 0.041, Side::Sell),
            position(2, 2, 200, 10, 0.043, Side::Sell),
        ];
        let scoped = aggregate_rates_risk(
            &request(
                positions.clone(),
                Some(RatesRiskScope {
                    entity: Some(1),
                    book: None,
                    ccy: None,
                }),
            ),
            &FleetTopology::InProcess,
        )
        .unwrap();
        let entity1_only = single_node_aggregate(&request(vec![positions[0]], None)).unwrap();
        assert_eq!(scoped.nodes.len(), 1);
        assert_eq!(
            scoped.nodes[0].net_pv.to_bits(),
            entity1_only.nodes[0].net_pv.to_bits()
        );
    }

    /// The fan-out is topology-agnostic on the answer: a distributed membership
    /// fans across more shards but produces the identical rollup.
    #[test]
    fn distributed_topology_same_answer() {
        let req = request(
            vec![
                position(1, 1, 100, 5, 0.041, Side::Sell),
                position(2, 2, 200, 10, 0.043, Side::Buy),
            ],
            None,
        );
        let in_process = aggregate_rates_risk(&req, &FleetTopology::InProcess).unwrap();
        let distributed = aggregate_rates_risk(
            &req,
            &FleetTopology::Distributed {
                endpoints: vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            },
        )
        .unwrap();
        assert_eq!(in_process.nodes.len(), distributed.nodes.len());
        for (a, b) in in_process.nodes.iter().zip(&distributed.nodes) {
            assert_eq!(a.net_pv.to_bits(), b.net_pv.to_bits());
            assert_eq!(a.net_dv01.to_bits(), b.net_dv01.to_bits());
        }
    }

    /// An empty book rolls up to no currency nodes (never a spurious zero node).
    #[test]
    fn empty_book_is_empty_rollup() {
        let resp = aggregate_rates_risk(&request(vec![], None), &FleetTopology::InProcess).unwrap();
        assert!(resp.nodes.is_empty());
    }

    /// A missing curve set fails loudly as `invalid_argument`.
    #[test]
    fn missing_curve_set_is_invalid_argument() {
        let req = AggregateRatesRiskRequest {
            curve_set: None,
            positions: vec![position(1, 1, 100, 5, 0.041, Side::Sell)],
            scope: None,
            principal: None,
            correlation_id: None,
            session_token: None,
        };
        let err = aggregate_rates_risk(&req, &FleetTopology::InProcess).unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }
}
