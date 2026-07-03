//! Oracle + codec tests for the `RiskService.CombinedTailRisk` RPC — the C2c unified
//! joint options+FI tail cube (`celnet_risk_cube::combined_tail_risk`) exposed on the
//! wire.
//!
//! The RPC is a FAITHFUL TRANSPORT of the engine: the handler decodes the inline
//! portfolio + scenario config, calls `combined_tail_risk` once, and maps the result
//! back to the wire. Every oracle here builds the domain inputs INDEPENDENTLY, runs an
//! independent reference (the engine called directly, or `node_var_es_combined` /
//! `rate_scenario_var_es` / a hand-reconstructed summed-P&L reduction), then builds the
//! wire request from the SAME logical portfolio and asserts the transported result is
//! bit-identical (`to_bits`). Non-circular: the reference is never the RPC re-run as
//! its own check. The C2c joint-VaR identities are re-established THROUGH the wire
//! (options-only ⇒ options VaR; FI-only ⇒ rate VaR; a mixed book is the ONE reduction
//! of the summed P&L, not a marginal aggregation), and the WS codec is proven to
//! round-trip the messages byte-stable in both directions.

use celnet_proto::risk_service_server::RiskService;
use celnet_proto::{
    CcyPair as WireCcyPair, CombinedTailRiskRequest, CombinedTailRiskResponse, JointTailScenario,
    OisFixedPeriod, OisSwapLeg, TailRiskCurvePillar, TailRiskFiPosition, TailRiskKeyRate,
    TailRiskOptionLeg, VarEs as WireVarEs, tail_risk_fi_position,
};
use celnet_rates::{FixedPeriod, OisSchedule};
use celnet_rates_risk::{
    FiPosition, RatePillars, RateShock, rate_scenario_var_es, scenario_pnls,
    standard_bump_scenarios,
};
use celnet_risk_cube::{
    FiBook, JointScenario, Scenario, combined_tail_risk, node_pnl, node_var_es_combined,
};
use celnet_risk_normalize::{AssetPricer, PositionRisk};
use celnet_server::services::risk::combined_tail::combined_tail_risk_response;
use celnet_server::services::risk::exotic_pricer::ExoticEngine;
use celnet_types::{
    Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, Rate, Time, VanillaInputs,
};

// ---------------------------------------------------------------------------
// portfolio builders — the domain side and its byte-for-byte wire mirror
// ---------------------------------------------------------------------------

/// The EUR/USD pair the worked options book is marked on.
fn eurusd() -> CcyPair {
    CcyPair::new(Ccy::EUR, Ccy::USD)
}

/// The single at-the-money vanilla mark the option legs share.
fn vanilla() -> VanillaInputs {
    VanillaInputs::new(1.10, 1.10, 0.10, 0.5, 0.03, 0.01)
}

/// One domain option position of signed base notional `n`.
fn option_pos(n: f64) -> PositionRisk {
    PositionRisk::fx(
        eurusd(),
        OptionType::Call,
        n,
        vanilla(),
        DeltaConvention::SpotUnadjusted,
        PremiumStyle::DomesticPips,
    )
}

/// The wire mirror of [`option_pos`] — the identical economics as a `TailRiskOptionLeg`.
fn wire_leg(n: f64) -> TailRiskOptionLeg {
    TailRiskOptionLeg {
        pair: Some(WireCcyPair {
            base: "EUR".to_owned(),
            quote: "USD".to_owned(),
        }),
        option_type: celnet_proto::OptionType::from(OptionType::Call) as i32,
        notional_base: n,
        spot: 1.10,
        strike: 1.10,
        vol: 0.10,
        t: 0.5,
        r_dom: 0.03,
        r_for: 0.01,
        quoted_delta: celnet_proto::DeltaConvention::from(DeltaConvention::SpotUnadjusted) as i32,
        premium_style: celnet_proto::PremiumStyle::from(PremiumStyle::DomesticPips) as i32,
    }
}

/// The five `(time, zero rate)` discount-curve pillars the FI legs reprice off.
const PILLARS: [(f64, f64); 5] = [
    (1.0, 0.030),
    (2.0, 0.032),
    (3.0, 0.033),
    (4.0, 0.034),
    (5.0, 0.035),
];

/// The domain base curve.
fn fi_base() -> RatePillars {
    RatePillars::new(PILLARS.iter().map(|&(t, z)| (Time(t), Rate(z))).collect()).expect("pillars")
}

/// The wire mirror of [`fi_base`].
fn wire_base() -> Vec<TailRiskCurvePillar> {
    PILLARS
        .iter()
        .map(|&(t, z)| TailRiskCurvePillar { t, zero_rate: z })
        .collect()
}

/// A domain 5y annual receive-fixed OIS at 3.30% on 1mm notional.
fn recv_ois() -> FiPosition {
    let periods = (1..=5)
        .map(|i| FixedPeriod {
            pay: Time(i as f64),
            accrual: Time(1.0),
        })
        .collect();
    FiPosition::OisSwap {
        schedule: OisSchedule::new(Time(0.0), periods).expect("schedule"),
        fixed_rate: Rate(0.033),
        notional: 1_000_000.0,
        receive_fixed: true,
    }
}

/// The wire mirror of [`recv_ois`].
fn wire_ois() -> TailRiskFiPosition {
    let periods = (1..=5)
        .map(|i| OisFixedPeriod {
            pay: i as f64,
            accrual: 1.0,
        })
        .collect();
    TailRiskFiPosition {
        position: Some(tail_risk_fi_position::Position::OisSwap(OisSwapLeg {
            start: 0.0,
            periods,
            fixed_rate: 0.033,
            notional: 1_000_000.0,
            receive_fixed: true,
        })),
    }
}

/// The wire mirror of a domain [`JointScenario`] (option shock + aligned rate shock).
fn wire_scenario(opt: Scenario, shock: &RateShock) -> JointTailScenario {
    JointTailScenario {
        spot_rel: opt.spot_rel,
        vol_abs: opt.vol_abs,
        discount_abs: opt.discount_abs,
        carry_abs: opt.carry_abs,
        rate_shifts: shock.shifts().to_vec(),
    }
}

/// A pure spot-shock option scenario.
fn spot(rel: f64) -> Scenario {
    Scenario {
        spot_rel: rel,
        vol_abs: 0.0,
        discount_abs: 0.0,
        carry_abs: 0.0,
    }
}

// ---------------------------------------------------------------------------
// oracle 1 — faithful transport (RPC == direct combined_tail_risk, to_bits)
// ---------------------------------------------------------------------------

#[test]
fn rpc_is_a_faithful_transport_of_combined_tail_risk() {
    let positions = vec![option_pos(1_000_000.0)];
    let fi = vec![recv_ois()];
    let base = fi_base();
    let shocks = standard_bump_scenarios(5, 1e-4);
    let opt: Vec<Scenario> = shocks
        .iter()
        .enumerate()
        .map(|(i, _)| spot(0.01 * (i as f64 - 4.0)))
        .collect();
    let joints: Vec<JointScenario> = opt
        .iter()
        .zip(&shocks)
        .map(|(o, s)| JointScenario::new(*o, s.clone()))
        .collect();
    let alpha = 0.95;

    // The independent oracle: the engine called directly on the domain portfolio.
    let direct = combined_tail_risk(
        &AssetPricer,
        &ExoticEngine,
        &positions,
        &[],
        FiBook::new(&fi, &base),
        &joints,
        alpha,
    )
    .expect("direct");

    // The same portfolio, transported through the wire request.
    let req = CombinedTailRiskRequest {
        option_legs: vec![wire_leg(1_000_000.0)],
        fi_positions: vec![wire_ois()],
        base_curve: wire_base(),
        scenarios: opt
            .iter()
            .zip(&shocks)
            .map(|(o, s)| wire_scenario(*o, s))
            .collect(),
        alpha,
        correlation_id: Some(77),
        session_token: None,
    };
    let resp = combined_tail_risk_response(&req).expect("rpc");
    let jve = resp.joint_var_es.expect("var_es present");

    assert_eq!(jve.var.to_bits(), direct.joint_var_es.var.to_bits());
    assert_eq!(jve.es.to_bits(), direct.joint_var_es.es.to_bits());
    assert_eq!(
        resp.fi_parallel_dv01.to_bits(),
        direct.fi_parallel_dv01.to_bits()
    );
    assert_eq!(resp.key_rate.len(), direct.key_rate.points().len());
    for (w, d) in resp.key_rate.iter().zip(direct.key_rate.points()) {
        assert_eq!(w.tenor_years.to_bits(), d.tenor_years.to_bits());
        assert_eq!(w.dv01.to_bits(), d.dv01.to_bits());
    }
    assert_eq!(resp.correlation_id, Some(77));
}

// ---------------------------------------------------------------------------
// oracle 2 — options-only reduces to the options VaR (through the wire)
// ---------------------------------------------------------------------------

#[test]
fn wire_options_only_reduces_to_the_options_var() {
    let positions = vec![option_pos(1_000_000.0), option_pos(-500_000.0)];
    let opt: Vec<Scenario> = (-5..=5)
        .filter(|i| *i != 0)
        .map(|i| spot(f64::from(i) * 0.01))
        .collect();
    let alpha = 0.9;

    // Independent oracle: the pure options non-additive VaR/ES.
    let want = node_var_es_combined(&AssetPricer, &ExoticEngine, &positions, &[], &opt, alpha);

    let req = CombinedTailRiskRequest {
        option_legs: vec![wire_leg(1_000_000.0), wire_leg(-500_000.0)],
        fi_positions: vec![],
        base_curve: wire_base(),
        // The rate arm is irrelevant (no FI legs); an empty shift vector is fine.
        scenarios: opt
            .iter()
            .map(|o| wire_scenario(*o, &RateShock::new(vec![])))
            .collect(),
        alpha,
        correlation_id: None,
        session_token: None,
    };
    let resp = combined_tail_risk_response(&req).expect("rpc");
    let jve = resp.joint_var_es.expect("var_es present");

    assert_eq!(jve.var.to_bits(), want.var.to_bits());
    assert_eq!(jve.es.to_bits(), want.es.to_bits());
    // An empty FI book ⇒ an all-zero key-rate ladder (one bucket per base pillar) and a
    // zero signed parallel DV01 — never a spurious FI number.
    assert_eq!(resp.key_rate.len(), 5);
    assert!(resp.key_rate.iter().all(|k| k.dv01 == 0.0));
    assert_eq!(resp.fi_parallel_dv01, 0.0);
}

// ---------------------------------------------------------------------------
// oracle 3 — FI-only reduces to the standalone rate VaR (through the wire)
// ---------------------------------------------------------------------------

#[test]
fn wire_fi_only_reduces_to_the_rate_var() {
    let fi = vec![recv_ois()];
    let base = fi_base();
    let shocks = standard_bump_scenarios(5, 1e-4);
    let alpha = 0.95;

    // Independent oracle: the standalone linear-FI scenario VaR/ES.
    let want = rate_scenario_var_es(&fi, &base, &shocks, alpha)
        .expect("rate var")
        .var_es;

    let req = CombinedTailRiskRequest {
        option_legs: vec![],
        fi_positions: vec![wire_ois()],
        base_curve: wire_base(),
        // The option arm is the base (no shock); the rate arm carries the shocks.
        scenarios: shocks.iter().map(|s| wire_scenario(spot(0.0), s)).collect(),
        alpha,
        correlation_id: None,
        session_token: None,
    };
    let jve = combined_tail_risk_response(&req)
        .expect("rpc")
        .joint_var_es
        .expect("var_es present");

    assert_eq!(jve.var.to_bits(), want.var.to_bits());
    assert_eq!(jve.es.to_bits(), want.es.to_bits());
}

// ---------------------------------------------------------------------------
// oracle 4 — a mixed book is the joint reduction, not a marginal sum
// ---------------------------------------------------------------------------

#[test]
fn wire_mixed_book_is_the_joint_reduction_not_a_marginal_sum() {
    let positions = vec![option_pos(1_000_000.0)];
    let fi = vec![recv_ois()];
    let base = fi_base();
    let shocks = standard_bump_scenarios(5, 1e-4);
    let opt: Vec<Scenario> = shocks
        .iter()
        .enumerate()
        .map(|(i, _)| spot(0.01 * (i as f64 - 4.0)))
        .collect();
    let alpha = 0.9;

    let req = CombinedTailRiskRequest {
        option_legs: vec![wire_leg(1_000_000.0)],
        fi_positions: vec![wire_ois()],
        base_curve: wire_base(),
        scenarios: opt
            .iter()
            .zip(&shocks)
            .map(|(o, s)| wire_scenario(*o, s))
            .collect(),
        alpha,
        correlation_id: None,
        session_token: None,
    };
    let jve = combined_tail_risk_response(&req)
        .expect("rpc")
        .joint_var_es
        .expect("var_es present");

    // Independent reconstruction of the joint model: option P&L + FI P&L summed
    // element-wise (ONE market state per index), reduced by the SAME tail primitive.
    let fi_pnl = scenario_pnls(&fi, &base, &shocks).expect("fi pnl");
    let mut summed: Vec<f64> = opt
        .iter()
        .zip(&fi_pnl)
        .map(|(o, &f)| node_pnl(&AssetPricer, &positions, *o) + f)
        .collect();
    let want = celnet_core::tail_var_es(&mut summed, alpha);
    assert_eq!(jve.var.to_bits(), want.var.to_bits());
    assert_eq!(jve.es.to_bits(), want.es.to_bits());

    // And it is genuinely JOINT — the FI leg moves the tail: the joint VaR is NOT the
    // options-only tail over the same option scenarios (a bug that dropped the FI book
    // would collapse the two). The rigorous joint-vs-marginal diversification is the
    // summed-P&L-reduction identity asserted above; this guards against a degenerate
    // one-sided cube.
    let options_only =
        node_var_es_combined(&AssetPricer, &ExoticEngine, &positions, &[], &opt, alpha);
    assert_ne!(
        jve.var.to_bits(),
        options_only.var.to_bits(),
        "the FI leg must genuinely contribute to the joint tail"
    );
}

// ---------------------------------------------------------------------------
// end-to-end — the async gRPC handler (auth + gate) returns the compute-core result
// ---------------------------------------------------------------------------

#[tokio::test]
async fn grpc_handler_matches_the_compute_core() {
    use celnet_server::services::risk::RiskEdge;
    use celnet_server::services::risk::store::PositionStore;
    use celnet_server::{AccessMode, ReadinessGate};
    use std::sync::Arc;
    use tonic::Request;

    let store = Arc::new(PositionStore::new());
    // A dev/test edge admits the grant-all default caller (no session token); the
    // handler's numbers are independent of the trust posture (faithful transport).
    store.set_access_mode(AccessMode::Permissive);
    let gate = Arc::new(ReadinessGate::new());
    gate.mark_ready();
    let edge = RiskEdge::new(store, gate);

    let shocks = standard_bump_scenarios(5, 1e-4);
    let opt: Vec<Scenario> = shocks
        .iter()
        .enumerate()
        .map(|(i, _)| spot(0.01 * (i as f64 - 4.0)))
        .collect();
    let req = CombinedTailRiskRequest {
        option_legs: vec![wire_leg(1_000_000.0)],
        fi_positions: vec![wire_ois()],
        base_curve: wire_base(),
        scenarios: opt
            .iter()
            .zip(&shocks)
            .map(|(o, s)| wire_scenario(*o, s))
            .collect(),
        alpha: 0.95,
        correlation_id: Some(9001),
        session_token: None,
    };

    let via_rpc = RiskService::combined_tail_risk(&edge, Request::new(req.clone()))
        .await
        .expect("rpc ok")
        .into_inner();
    let via_compute = combined_tail_risk_response(&req).expect("compute ok");
    assert_eq!(via_rpc, via_compute);
}

// ---------------------------------------------------------------------------
// codec — the WS request/response round-trip byte-stable in both directions
// ---------------------------------------------------------------------------

#[test]
fn ws_codec_round_trips_request_and_response() {
    use celnet_server::ws::generated_codec as gc;

    // A representative mixed request (both correlation_id + session_token set).
    let req = CombinedTailRiskRequest {
        option_legs: vec![wire_leg(1_000_000.0), wire_leg(-250_000.0)],
        fi_positions: vec![wire_ois()],
        base_curve: wire_base(),
        scenarios: vec![
            JointTailScenario {
                spot_rel: 0.01,
                vol_abs: -0.02,
                discount_abs: 0.001,
                carry_abs: -0.0005,
                rate_shifts: vec![1e-4, 2e-4, -1e-4, 0.0, 3e-4],
            },
            JointTailScenario {
                spot_rel: -0.03,
                vol_abs: 0.0,
                discount_abs: 0.0,
                carry_abs: 0.0,
                rate_shifts: vec![-2e-4, 0.0, 1e-4, 5e-5, -3e-4],
            },
        ],
        alpha: 0.975,
        correlation_id: Some(4242),
        session_token: Some("tok-abc".to_owned()),
    };
    let j = gc::encode_combined_tail_risk_request(&req);
    let back =
        gc::decode_combined_tail_risk_request(j.as_object().expect("obj")).expect("decode req");
    assert_eq!(req, back);

    // The response envelope round-trips too.
    let resp = CombinedTailRiskResponse {
        joint_var_es: Some(WireVarEs {
            var: 12345.678,
            es: 23456.789,
        }),
        key_rate: vec![
            TailRiskKeyRate {
                tenor_years: 1.0,
                dv01: -12.5,
            },
            TailRiskKeyRate {
                tenor_years: 2.5,
                dv01: -30.25,
            },
        ],
        fi_parallel_dv01: -512.75,
        correlation_id: Some(4242),
    };
    let jr = gc::encode_combined_tail_risk_response(&resp);
    let rback =
        gc::decode_combined_tail_risk_response(jr.as_object().expect("obj")).expect("decode resp");
    assert_eq!(resp, rback);

    // An options-only request (empty fi_positions, no session token, empty shifts) is
    // the other structural shape — it round-trips identically.
    let opt_only = CombinedTailRiskRequest {
        option_legs: vec![wire_leg(500_000.0)],
        fi_positions: vec![],
        base_curve: wire_base(),
        scenarios: vec![JointTailScenario {
            spot_rel: 0.02,
            vol_abs: 0.0,
            discount_abs: 0.0,
            carry_abs: 0.0,
            rate_shifts: vec![],
        }],
        alpha: 0.99,
        correlation_id: None,
        session_token: None,
    };
    let j2 = gc::encode_combined_tail_risk_request(&opt_only);
    let back2 = gc::decode_combined_tail_risk_request(j2.as_object().expect("obj"))
        .expect("decode opt-only");
    assert_eq!(opt_only, back2);
}
