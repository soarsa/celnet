//! SOTA Scalability Phase 4 Verification Suite:
//! 1. Decentralized Scenario Grid Vector Reduction (4 KB wire transfer vs 50 MB trade re-gathering).
//! 2. Dean & Barroso Hedged Fan-In neutralizing tail-at-scale amplification.

use std::time::Duration;
use celnet_risk_cube::test_support::DigitalTestPricer;
use celnet_risk_cube::{
    BookId as CubeBookId, Cube, DeskId, EntityId, FactKey, FactMeasure, LocationId, PositionId,
    RiskFact, Scenario, TraderId, VegaPillar, VegaPillarMap,
};
use celnet_risk_fleet::{
    ExpectedShortfallAggregator, FrtbCurvatureAggregator, HedgedFanInCoordinator, HedgedPolicy,
    LogicalShard, ScenarioFleetReducer, ScenarioGridVector, WinningSource,
};
use celnet_risk_normalize::{CanonicalLeaf, PositionRisk, canonicalize};
use celnet_router::ReplicaId;
use celnet_types::{
    Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs,
};

struct DaysPillar;
impl VegaPillarMap for DaysPillar {
    fn pillar_of(&self, _leaf: &CanonicalLeaf, position: &PositionRisk) -> VegaPillar {
        let days = (position.inputs.t * 365.0).round() as u32;
        VegaPillar::new(days, 5000)
    }
}

fn pos(p: CcyPair, opt: OptionType, notional: f64, inputs: VanillaInputs) -> PositionRisk {
    PositionRisk::fx(
        p,
        opt,
        notional,
        inputs,
        DeltaConvention::SpotUnadjusted,
        PremiumStyle::DomesticPips,
    )
}

fn make_fact(
    id: u32,
    trader: u32,
    book: u32,
    desk: u32,
    loc: u32,
    entity: u32,
    position: PositionRisk,
) -> RiskFact {
    let leaf = canonicalize(&position).unwrap();
    RiskFact {
        position_id: PositionId(id),
        key: FactKey {
            trader: TraderId(trader),
            book: CubeBookId(book),
            desk: DeskId(desk),
            underlying: position.underlying.clone(),
            location: LocationId(loc),
            entity: EntityId(entity),
        },
        measure: FactMeasure {
            leaf,
            position,
            exotic: None,
        },
        surface_version: 1,
    }
}

#[test]
fn test_scenario_grid_vector_reduction_mathematical_equivalence() {
    let eurusd = CcyPair::new(Ccy::EUR, Ccy::USD);
    let gbpusd = CcyPair::new(Ccy::GBP, Ccy::USD);
    let usdjpy = CcyPair::new(Ccy::USD, Ccy::JPY);
    let audusd = CcyPair::new(Ccy::AUD, Ccy::USD);

    let pairs = [eurusd, gbpusd, usdjpy, audusd];
    let num_shards = 4;
    let mut shards: Vec<LogicalShard> = (0..num_shards)
        .map(|i| LogicalShard::new(ReplicaId(i as u64)))
        .collect();

    // Populate shards with facts
    let mut full_cube = Cube::new();
    let mut fact_id = 1;
    for (shard_idx, pair) in pairs.iter().enumerate() {
        for k in 0..25 {
            let strike = 1.05 + (k as f64) * 0.01;
            let notional = 100_000.0 * ((k % 5) + 1) as f64;
            let inputs = VanillaInputs {
                spot: strike,
                strike,
                vol: 0.12,
                t: 1.0,
                r_dom: 0.03,
                r_for: 0.02,
            };
            let p = pos(*pair, OptionType::Call, notional, inputs);
            let fact = make_fact(fact_id, 1, 1, 1, 1, 1, p);
            shards[shard_idx].upsert(fact.clone());
            full_cube.upsert(fact);
            fact_id += 1;
        }
    }

    // Build 500 scenarios
    let scenarios: Vec<Scenario> = (0..500)
        .map(|i| {
            let shock = ((i as f64 - 250.0) / 250.0) * 0.05; // -5% to +5%
            let vol_shock = ((i as f64 - 250.0) / 250.0) * 0.02;
            Scenario {
                spot_rel: shock,
                vol_abs: vol_shock,
                discount_abs: shock * 0.1,
                carry_abs: shock * 0.1,
            }
        })
        .collect();

    // 1. Central reference evaluation: single-node cube evaluates all positions
    let full_firm_node = full_cube.firm_aggregate(&DaysPillar);
    let reference_var_es =
        Cube::node_var_es_sensitivity(&celnet_risk_normalize::AssetPricer, &DigitalTestPricer, &full_firm_node, &scenarios, 0.99);

    // 2. SOTA Decentralized Reduction: each shard produces a 4 KB ScenarioGridVector
    let mut reducer = ScenarioFleetReducer::new();
    for shard in &shards {
        let node = shard.local_aggregate(&DaysPillar);
        let grid = ScenarioGridVector::from_node_sensitivity(
            shard.replica(),
            &node,
            &scenarios,
            Ccy::USD,
        );

        // Assert wire size: exactly 500 floats = 4,000 bytes (4 KB)
        assert_eq!(grid.wire_size_bytes(), 500 * 8);
        assert_eq!(grid.scenario_count(), 500);

        reducer.add_shard(grid).expect("shard vector addition must succeed");
    }

    assert_eq!(reducer.shard_count(), 4);
    assert_eq!(reducer.total_wire_bytes(), 4 * 4000); // 16 KB total for 4 shards!

    // Reduce firm grid and derive VaR / ES
    let decentralized_var_es = reducer.firm_var_es(0.99).expect("firm var_es calculation must succeed");

    // 3. Prove exact mathematical equivalence
    println!(
        "Central Reference: VaR = {}, ES = {}",
        reference_var_es.var, reference_var_es.es
    );
    println!(
        "Decentralized Vector Reduction: VaR = {}, ES = {}",
        decentralized_var_es.var, decentralized_var_es.es
    );

    let var_diff = (decentralized_var_es.var - reference_var_es.var).abs();
    let es_diff = (decentralized_var_es.es - reference_var_es.es).abs();

    assert!(
        var_diff < 1e-10,
        "VaR must be bit-identical: diff = {}",
        var_diff
    );
    assert!(
        es_diff < 1e-10,
        "ES must be bit-identical: diff = {}",
        es_diff
    );

    // 4. Prove pluggable non-additive risk aggregator SPI
    let es_aggregator = ExpectedShortfallAggregator::frtb_standard();
    let es_975 = reducer.evaluate_non_additive(&es_aggregator).expect("ES evaluation must succeed");
    assert!(es_975.is_finite());
    assert!(es_975 > 0.0);

    let frtb_cvr = FrtbCurvatureAggregator::new(0.60);
    let curvature_capital = reducer.evaluate_non_additive(&frtb_cvr).expect("FRTB curvature evaluation must succeed");
    assert!(curvature_capital.is_finite());
    assert!(curvature_capital > 0.0);
}

#[tokio::test]
async fn test_dean_barroso_hedged_fan_in_primary_win() {
    let coordinator = HedgedFanInCoordinator::new(HedgedPolicy {
        hedging_delay: Duration::from_millis(5),
        hard_timeout: Duration::from_millis(50),
    });

    // Primary returns quickly in 1 ms
    let primary_fn = || async {
        tokio::time::sleep(Duration::from_millis(1)).await;
        Ok("primary_quote_ok".to_string())
    };

    // Secondary would take longer
    let secondary_fn = || async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        Ok("secondary_quote_ok".to_string())
    };

    let response = coordinator
        .query_shard(primary_fn, secondary_fn)
        .await
        .expect("query must succeed");

    assert_eq!(response.source, WinningSource::Primary);
    assert_eq!(response.value, "primary_quote_ok");

    let metrics = coordinator.metrics().snapshot();
    assert_eq!(metrics.primary_wins, 1);
    assert_eq!(metrics.secondary_hedges_dispatched, 0);
    assert_eq!(metrics.hedged_wins, 0);
}

#[tokio::test]
async fn test_dean_barroso_hedged_fan_in_speculative_secondary_neutralizes_tail() {
    let coordinator = HedgedFanInCoordinator::new(HedgedPolicy {
        hedging_delay: Duration::from_millis(5),
        hard_timeout: Duration::from_millis(50),
    });

    // Primary stalls with a simulated GC tail latency of 40 ms
    let primary_fn = || async {
        tokio::time::sleep(Duration::from_millis(40)).await;
        Ok("primary_tail_stalled".to_string())
    };

    // Secondary executes promptly in 2 ms once dispatched
    let secondary_fn = || async {
        tokio::time::sleep(Duration::from_millis(2)).await;
        Ok("hedged_secondary_recovered".to_string())
    };

    let response = coordinator
        .query_shard(primary_fn, secondary_fn)
        .await
        .expect("hedged query must succeed");

    // The speculative secondary request must win because primary stalled!
    assert_eq!(response.source, WinningSource::HedgedSecondary);
    assert_eq!(response.value, "hedged_secondary_recovered");
    // Elapsed should be ~7ms (5ms delay + 2ms secondary), NOT 40ms!
    assert!(response.elapsed < Duration::from_millis(25));

    let metrics = coordinator.metrics().snapshot();
    assert_eq!(metrics.primary_wins, 0);
    assert_eq!(metrics.secondary_hedges_dispatched, 1);
    assert_eq!(metrics.hedged_wins, 1);
    assert_eq!(metrics.tail_avoidance_ratio, 1.0);
}
