//! Trader-workflow integration tests for the surface + risk SDK.
//!
//! * **Scenario 3 — a quant marks the surface from broker ATM/RR/BF, then reads
//!   the smile across deltas and the arbitrage report.** Driven through the typed
//!   [`celnet_client::Client::mark_surface`] / [`celnet_client::Client::get_smile`]:
//!   the calibrated 50Δ pillar reproduces the marked ATM vol, the wings carry the
//!   marked skew, and the typed [`celnet_client::ArbReport`] reports
//!   butterfly-arbitrage-freedom — exactly what the server's own surface tests
//!   assert, but through the SDK's typed [`celnet_client::Smile`].
//! * **Scenario 4 — a risk manager runs a spot/vol shock grid.** Driven through
//!   [`celnet_client::Client::scenario`]: every node's repriced price equals a
//!   first-principles `celnet-vanilla` price at the node's shocked market, and the
//!   unshocked node equals the base price.
//!
//! Every body is hard wall-clock bounded and every network await is bounded.

mod common;

use std::time::Duration;

use celnet_client::{BrokerQuoteSet, ShockAxis, ShockFactor};
use celnet_core::is_close;
use celnet_types::{OptionType, VanillaInputs};

use common::{
    STEP_DEADLINE, TEST_DEADLINE, conventions, eurusd, live_market, start_edge_and_client,
    vanilla_call,
};

/// Scenario 3: a quant marks the surface from a broker ATM/RR/BF quote set and
/// reads back a typed calibrated smile with an arbitrage report. The 50Δ pillar
/// reproduces the marked ATM vol and the calibrated smile is butterfly-arb-free.
#[tokio::test]
async fn quant_marks_surface_reads_smile_and_arb_report() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;

        let broker = BrokerQuoteSet::three_point(1.0, 0.105, -0.0040, 0.0020);
        let marked = tokio::time::timeout(
            STEP_DEADLINE,
            client.mark_surface(eurusd(), &[broker], conventions()),
        )
        .await
        .expect("mark_surface returns in time")
        .expect("mark_surface succeeds");

        assert!(marked.surface_version >= 1, "a surface version is stamped");
        assert_eq!(marked.smiles.len(), 1, "one smile per broker quote set");
        let smile = &marked.smiles[0];
        assert!(is_close(smile.tenor_years, 1.0, 1e-12, 1e-12));
        assert_eq!(smile.points.len(), 5, "delta-axis pillars reported");

        // The arbitrage report is typed and a quant branches on it directly.
        assert!(
            smile.arbitrage.butterfly_arbitrage_free,
            "a mild EURUSD smile is butterfly-arbitrage-free"
        );

        // The 50Δ (ATM) pillar reproduces the marked ATM vol, via the typed helper.
        let atm = smile.atm_vol().expect("a 50Δ pillar exists");
        assert!(
            is_close(atm, broker.atm_vol, 5e-3, 5e-3),
            "ATM pillar vol {atm} ~ marked {}",
            broker.atm_vol
        );

        // The skew is present: with a negative 25Δ RR the 25Δ put wing vol exceeds
        // the 25Δ call wing vol (puts richer than calls).
        let call_25 = smile.vol_at_delta(0.25).expect("25Δ call pillar");
        let put_25 = smile.vol_at_delta(-0.25).expect("25Δ put pillar");
        assert!(
            put_25 > call_25,
            "negative RR ⇒ 25Δ put vol {put_25} > 25Δ call vol {call_25}"
        );

        // A subsequent GetSmile returns a calibrated, sane delta-axis slice.
        let read = tokio::time::timeout(
            STEP_DEADLINE,
            client.get_smile(eurusd(), 1.0, conventions()),
        )
        .await
        .expect("get_smile returns in time")
        .expect("get_smile succeeds");
        assert_eq!(read.points.len(), 5);
        for p in &read.points {
            assert!(p.vol > 0.0 && p.vol < 1.0, "vol {} in range", p.vol);
        }

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// Scenario 4: a risk manager runs a spot × vol shock grid. Every node's repriced
/// price equals a first-principles `celnet-vanilla` price at the node's shocked
/// market, and the unshocked `[0, 0]` node equals the base price.
#[tokio::test]
async fn risk_manager_runs_spot_vol_shock_grid() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;

        let strike = 1.12;
        let base = live_market();
        let spot_steps = vec![-0.10, 0.0, 0.10];
        let vol_steps = vec![0.0, 0.50];
        let axes = vec![
            ShockAxis::relative(ShockFactor::Spot, spot_steps.clone()),
            ShockAxis::relative(ShockFactor::Vol, vol_steps.clone()),
        ];

        let grid = tokio::time::timeout(
            STEP_DEADLINE,
            client.scenario(&vanilla_call(strike), base, &axes, conventions()),
        )
        .await
        .expect("scenario returns in time")
        .expect("scenario succeeds");

        assert_eq!(
            grid.nodes.len(),
            spot_steps.len() * vol_steps.len(),
            "the grid is the Cartesian product of the axes"
        );

        // Every node reprices exactly against a direct GK price at its shocked
        // (spot, vol) — the SDK surfaces the maker's deterministic value verbatim.
        for node in &grid.nodes {
            let shocked = node.shocked_market;
            let direct = celnet_vanilla::price(
                OptionType::Call,
                &VanillaInputs::new(
                    shocked.spot,
                    strike,
                    shocked.vol,
                    1.0,
                    shocked.r_dom,
                    shocked.r_for,
                ),
            );
            assert!(
                is_close(node.greeks.price, direct, 1e-10, 1e-10),
                "node price {} != direct {direct} at spot {} vol {}",
                node.greeks.price,
                shocked.spot,
                shocked.vol
            );
        }

        // The unshocked node equals the base price, via the typed lookup helper.
        let base_node = grid
            .node_with_shocks(&[0.0, 0.0])
            .expect("the [0,0] node exists");
        let base_direct = celnet_vanilla::price(
            OptionType::Call,
            &VanillaInputs::new(base.spot, strike, base.vol, 1.0, base.r_dom, base.r_for),
        );
        assert!(is_close(base_node.greeks.price, base_direct, 1e-12, 1e-12));

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}

/// One-shot pricing through the SDK equals a direct GK price at the supplied
/// market context — the `Client::price` path surfaces the maker's deterministic
/// computation verbatim.
#[tokio::test]
async fn one_shot_price_equals_direct() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client) = start_edge_and_client().await;

        let strike = 1.08;
        let market = live_market();
        let priced = tokio::time::timeout(
            STEP_DEADLINE,
            client.price(&vanilla_call(strike), market, conventions()),
        )
        .await
        .expect("price returns in time")
        .expect("price succeeds");

        let direct = celnet_vanilla::greeks(
            OptionType::Call,
            &VanillaInputs::new(
                market.spot,
                strike,
                market.vol,
                1.0,
                market.r_dom,
                market.r_for,
            ),
        );
        assert!(is_close(priced.greeks.price, direct.price, 1e-12, 1e-12));
        assert!(is_close(priced.greeks.vega, direct.vega, 1e-12, 1e-12));
        assert!(is_close(priced.greeks.gamma, direct.gamma, 1e-12, 1e-12));
        assert!(is_close(priced.resolved_strike, strike, 1e-12, 1e-12));

        edge.shutdown(Duration::from_secs(5)).await;
    })
    .await
    .expect("test must not hang");
}
