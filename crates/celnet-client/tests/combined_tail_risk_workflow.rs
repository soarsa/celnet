//! Live-loopback trader-workflow tests for the combined (joint options + fixed-income)
//! tail-risk SDK method (`Client::combined_tail_risk`) — the client parity of the
//! landed `RiskService.CombinedTailRisk` C2c unified risk cube, and the last piece of FI
//! SDK risk/scenario parity.
//!
//! Each test boots a real in-process `celnet-server` edge under the production `Enforce`
//! posture, authenticates as the seed admin (the RPC is `ReadAny` but carries NO
//! asserted-principal field, so under `Enforce` a validated session is the only
//! admission — the token authorizes and never changes the number), builds the request
//! with the typed SDK builders, and dispatches it end-to-end over the gRPC wire. The
//! whole portfolio + scenario config travels inline; the tail is reduced SERVER-SIDE.
//!
//! The C2c joint-VaR identities are re-established THROUGH the wire, against INDEPENDENT
//! references built first-principles from the underlying engines (a DIFFERENT code path
//! than the RPC's cube), never the RPC re-run as its own oracle:
//!
//! * **FI-only reduces to the standalone rate VaR** — the SDK FI-only book's
//!   `joint_var_es` is bit-identical (`to_bits`) to `celnet_rates_risk::rate_scenario_var_es`
//!   over the same shocks, proving the SDK routes an FI-only portfolio to the FI engine
//!   and reduces it faithfully; plus the model-independent coherence (`es ≥ var ≥ 0`) and
//!   the signed key-rate axis (one negative point per pillar for a receive-fixed swap,
//!   totalling to the parallel DV01 to first order).
//! * **A mixed book diversifies** — the joint VaR is strictly below the naive sum of the
//!   options-only and FI-only marginal VaRs (the anti-aligned scenarios net within each
//!   market state), is not equal to either marginal, and is bit-identical to an
//!   independent reconstruction of the summed-P&L tail reduction (`node_pnl` +
//!   `scenario_pnls` + the one platform `tail_var_es`).
//!
//! Never a mock. Every body is hard wall-clock bounded and every network await is
//! itself bounded, so a regression fails fast, never hangs.

mod common;

use celnet_client::{
    CombinedTailRiskQuery, DiscountCurve, JointShock, MarketContext, TailFiPosition, TailOptionLeg,
};
use celnet_types::{
    Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, Rate, Time, VanillaInputs,
};

use celnet_rates::{FixedPeriod, OisSchedule};
use celnet_rates_risk::{
    FiPosition, RatePillars, RateShock, rate_scenario_var_es, scenario_pnls,
    standard_bump_scenarios,
};
use celnet_risk_cube::{Scenario, node_pnl};
use celnet_risk_normalize::{AssetPricer, PositionRisk};

use common::{STEP_DEADLINE, TEST_DEADLINE, start_edge_and_authed_client};

/// The five `(time, continuously-compounded zero rate)` discount-curve pillars the FI
/// legs reprice off — the same known-good shape the server's own C2c tests use.
const PILLARS: [(f64, f64); 5] = [
    (1.0, 0.030),
    (2.0, 0.032),
    (3.0, 0.033),
    (4.0, 0.034),
    (5.0, 0.035),
];

/// The EUR/USD pair the worked option leg is marked on.
fn eurusd() -> CcyPair {
    CcyPair::new(Ccy::EUR, Ccy::USD)
}

/// The market the option leg is valued off (spot 1.10, 10 vol, 3%/1% carry).
fn market() -> MarketContext {
    MarketContext {
        spot: 1.10,
        vol: 0.10,
        r_dom: 0.03,
        r_for: 0.01,
    }
}

/// The SDK base discount curve (zero-rate pillars).
fn sdk_curve() -> DiscountCurve {
    PILLARS
        .iter()
        .fold(DiscountCurve::new(), |c, &(t, z)| c.pillar(t, z))
}

/// The SDK FI leg: a 5y annual receive-fixed OIS at 3.30% on 1mm notional.
fn sdk_recv_ois() -> TailFiPosition {
    TailFiPosition::receive_fixed_annual(5, 0.033, 1_000_000.0)
}

/// The independent-reference base curve (the same pillars in domain coordinates).
fn ref_base() -> RatePillars {
    RatePillars::new(PILLARS.iter().map(|&(t, z)| (Time(t), Rate(z))).collect()).expect("pillars")
}

/// The independent-reference FI leg — the domain mirror of [`sdk_recv_ois`].
fn ref_recv_ois() -> FiPosition {
    let periods = (1..=5)
        .map(|i| FixedPeriod {
            pay: Time(f64::from(i)),
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

/// The independent-reference option position — the domain mirror of the SDK option leg
/// (a long 1mm EUR/USD 6M ATM call).
fn ref_option() -> PositionRisk {
    PositionRisk::fx(
        eurusd(),
        OptionType::Call,
        1_000_000.0,
        VanillaInputs::new(1.10, 1.10, 0.10, 0.5, 0.03, 0.01),
        DeltaConvention::SpotUnadjusted,
        PremiumStyle::DomesticPips,
    )
}

/// An FI-only book reduces to the standalone rate VaR through the SDK, carries the
/// coherent (`es ≥ var ≥ 0`) tail, and surfaces the signed FI key-rate axis.
#[tokio::test]
async fn fi_only_reduces_to_the_rate_var_over_the_sdk() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _dir) = start_edge_and_authed_client().await;
        let alpha = 0.95;

        // The prescribed ±1bp bump ladder (parallel ± then per-pillar ±) as the rate
        // shocks; each SDK scenario is the base option state (no spot/vol shock) plus
        // that shift vector.
        let shocks = standard_bump_scenarios(5, 1e-4);
        let scenarios: Vec<JointShock> = shocks
            .iter()
            .map(|s| JointShock::base().rate_shifts(s.shifts().to_vec()))
            .collect();
        let query =
            CombinedTailRiskQuery::fi_only(sdk_curve(), [sdk_recv_ois()], scenarios, alpha);

        let got = step(client.combined_tail_risk(&query)).await;

        // Independent oracle: the standalone linear-FI scenario VaR/ES (a DIFFERENT code
        // path than the RPC's joint cube). The C2c FI-only identity, over the wire.
        let want = rate_scenario_var_es(&[ref_recv_ois()], &ref_base(), &shocks, alpha)
            .expect("rate var")
            .var_es;
        assert_eq!(
            got.joint_var_es.var.to_bits(),
            want.var.to_bits(),
            "FI-only VaR reduces to the standalone rate VaR"
        );
        assert_eq!(
            got.joint_var_es.es.to_bits(),
            want.es.to_bits(),
            "FI-only ES reduces to the standalone rate ES"
        );

        // Coherence: both are non-negative loss magnitudes and ES ≥ VaR (ES is the mean
        // loss in the tail at or beyond the VaR quantile — the platform tail convention).
        assert!(
            got.joint_var_es.var >= 0.0,
            "VaR is a loss magnitude: {}",
            got.joint_var_es.var
        );
        assert!(
            got.joint_var_es.es >= got.joint_var_es.var - 1e-9,
            "ES {} >= VaR {}",
            got.joint_var_es.es,
            got.joint_var_es.var
        );
        assert!(
            got.joint_var_es.var > 0.0,
            "a receive-fixed swap under a ±1bp bump ladder carries real tail risk: {}",
            got.joint_var_es.var
        );

        // The FI signed key-rate axis: one point per base-curve pillar, each a loss on a
        // rate rise (receive-fixed) ⇒ negative, at the matching pillar tenor; the ladder
        // totals to the parallel DV01 to first order.
        assert_eq!(
            got.key_rate.len(),
            PILLARS.len(),
            "one key-rate point per base-curve pillar"
        );
        for (kr, &(t, _)) in got.key_rate.iter().zip(PILLARS.iter()) {
            assert_eq!(kr.tenor_years, t, "key-rate tenor matches the pillar time");
            assert!(
                kr.dv01 < 0.0,
                "receive-fixed loses on a rate rise ⇒ negative key-rate DV01 at {t}y: {}",
                kr.dv01
            );
        }
        assert!(
            got.fi_parallel_dv01 < 0.0,
            "receive-fixed has a negative signed parallel DV01: {}",
            got.fi_parallel_dv01
        );
        let ladder_total: f64 = got.key_rate.iter().map(|k| k.dv01).sum();
        assert!(
            (ladder_total - got.fi_parallel_dv01).abs() / got.fi_parallel_dv01.abs() < 5e-3,
            "the key-rate ladder totals to the parallel DV01 to first order: Σ {ladder_total} vs {}",
            got.fi_parallel_dv01
        );

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// A mixed options + FI book diversifies through the SDK: the joint VaR is strictly below
/// the naive sum of the two marginal VaRs, differs from either marginal, and is the exact
/// summed-P&L reduction (the C2c joint identity, over the wire).
#[tokio::test]
async fn joint_book_diversifies_and_is_the_joint_reduction_over_the_sdk() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _dir) = start_edge_and_authed_client().await;
        let alpha = 0.9;

        // Deliberately ANTI-ALIGNED scenarios so the joint tail diversifies: a long call
        // loses when spot falls; a receive-fixed swap loses when rates rise. Pairing a
        // spot move with a SAME-SIGN parallel rate move (spot down ⟺ rates down) puts the
        // worst option loss (spot −4%) in the same market state as an FI GAIN (rates
        // −40bp) and vice-versa, so neither marginal worst-loss survives in the joint.
        let scenarios: Vec<JointShock> = (-4..=4)
            .map(|m| {
                let m = f64::from(m);
                JointShock::base()
                    .spot_rel(0.01 * m)
                    .parallel_rate(PILLARS.len(), 0.0010 * m)
            })
            .collect();

        let leg = TailOptionLeg::call(eurusd(), 1_000_000.0, 1.10, 0.5, market());

        let joint = step(client.combined_tail_risk(&CombinedTailRiskQuery::joint(
            sdk_curve(),
            [leg],
            [sdk_recv_ois()],
            scenarios.clone(),
            alpha,
        )))
        .await;
        // The two marginals are the projections of the SAME joint scenarios onto each
        // risk class (the ignored rate shifts / spot shocks are simply not applied).
        let opts = step(
            client.combined_tail_risk(&CombinedTailRiskQuery::options_only(
                sdk_curve(),
                [leg],
                scenarios.clone(),
                alpha,
            )),
        )
        .await;
        let fi = step(client.combined_tail_risk(&CombinedTailRiskQuery::fi_only(
            sdk_curve(),
            [sdk_recv_ois()],
            scenarios.clone(),
            alpha,
        )))
        .await;

        // Both marginals carry real tail risk, and so does the joint book.
        assert!(
            opts.joint_var_es.var > 0.0,
            "options-only VaR positive: {}",
            opts.joint_var_es.var
        );
        assert!(
            fi.joint_var_es.var > 0.0,
            "FI-only VaR positive: {}",
            fi.joint_var_es.var
        );
        assert!(
            joint.joint_var_es.var > 0.0,
            "joint VaR positive: {}",
            joint.joint_var_es.var
        );

        // Diversification: the joint VaR is strictly below the NAIVE SUM of the two
        // marginal VaRs — the cross-risk-class benefit — and is NOT equal to either
        // marginal (both legs move the joint tail; a dropped leg would collapse it).
        let naive_sum = opts.joint_var_es.var + fi.joint_var_es.var;
        assert!(
            joint.joint_var_es.var < naive_sum,
            "joint VaR {} diversifies below the naive marginal sum {naive_sum}",
            joint.joint_var_es.var
        );
        assert!(
            joint.joint_var_es.var.to_bits() != opts.joint_var_es.var.to_bits(),
            "the FI leg moves the joint tail (joint != options-only)"
        );
        assert!(
            joint.joint_var_es.var.to_bits() != fi.joint_var_es.var.to_bits(),
            "the option leg moves the joint tail (joint != FI-only)"
        );

        // The SDK routes each shape to the right cube arm: an options-only book carries NO
        // FI number, the FI-only book carries the full signed FI axis, and the joint book's
        // FI axis is that SAME book (the FI sensitivities are independent of the options).
        assert_eq!(
            opts.fi_parallel_dv01, 0.0,
            "an options-only book has a zero parallel DV01"
        );
        assert!(
            opts.key_rate.iter().all(|k| k.dv01 == 0.0),
            "an options-only book has an all-zero key-rate ladder"
        );
        assert!(
            fi.fi_parallel_dv01 < 0.0,
            "the FI-only receive-fixed book has a negative parallel DV01: {}",
            fi.fi_parallel_dv01
        );
        assert_eq!(
            joint.fi_parallel_dv01.to_bits(),
            fi.fi_parallel_dv01.to_bits(),
            "the joint FI parallel DV01 equals the FI-only one (the same FI book)"
        );

        // Exact identity: the joint tail is the ONE tail reduction of the element-wise SUM
        // of the option-leg and FI-leg per-scenario P&L, reconstructed here from
        // independent first-principles engines (never the RPC as its own oracle).
        let ref_positions = vec![ref_option()];
        let ref_fi = vec![ref_recv_ois()];
        let base = ref_base();
        let ref_shocks: Vec<RateShock> = (-4..=4)
            .map(|m| RateShock::new(vec![0.0010 * f64::from(m); PILLARS.len()]))
            .collect();
        let ref_opt: Vec<Scenario> = (-4..=4)
            .map(|m| Scenario {
                spot_rel: 0.01 * f64::from(m),
                vol_abs: 0.0,
                discount_abs: 0.0,
                carry_abs: 0.0,
            })
            .collect();
        let fi_pnl = scenario_pnls(&ref_fi, &base, &ref_shocks).expect("fi pnl");
        let mut summed: Vec<f64> = ref_opt
            .iter()
            .zip(&fi_pnl)
            .map(|(o, &f)| node_pnl(&AssetPricer, &ref_positions, *o) + f)
            .collect();
        let want = celnet_core::tail_var_es(&mut summed, alpha);
        assert_eq!(
            joint.joint_var_es.var.to_bits(),
            want.var.to_bits(),
            "joint VaR is the summed-P&L reduction"
        );
        assert_eq!(
            joint.joint_var_es.es.to_bits(),
            want.es.to_bits(),
            "joint ES is the summed-P&L reduction"
        );

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// The caller correlation id round-trips on the response (the observability handle).
#[tokio::test]
async fn correlation_id_round_trips_over_the_sdk() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _dir) = start_edge_and_authed_client().await;
        let shocks = standard_bump_scenarios(5, 1e-4);
        let scenarios: Vec<JointShock> = shocks
            .iter()
            .map(|s| JointShock::base().rate_shifts(s.shifts().to_vec()))
            .collect();
        let query = CombinedTailRiskQuery::fi_only(sdk_curve(), [sdk_recv_ois()], scenarios, 0.99)
            .correlation_id(4242);

        let got = step(client.combined_tail_risk(&query)).await;
        assert_eq!(got.correlation_id, Some(4242));

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// Bound a single risk call so a never-arriving reply fails fast.
async fn step<T>(fut: impl std::future::Future<Output = celnet_client::ClientResult<T>>) -> T {
    tokio::time::timeout(STEP_DEADLINE, fut)
        .await
        .expect("combined-tail-risk call resolves in time")
        .expect("combined-tail-risk call succeeds")
}
