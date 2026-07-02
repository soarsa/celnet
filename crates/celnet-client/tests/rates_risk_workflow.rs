//! Trader-workflow integration tests for the linear-rates (fixed-income) risk SDK
//! surface (`RiskService` FI arm): book a rates position, list the booked book, and
//! roll an inline OIS book up into the firm per-currency aggregate — every one a
//! single round-trip whose pricing / netting runs SERVER-SIDE (the api-first parity
//! rule: an SDK user gets the SAME FI-risk surface the GUI Book view does, never
//! pricing or summing client-side).
//!
//! Each test starts a real in-process `celnet-server` edge under the production
//! `Enforce` posture, authenticates as the seed admin (the booking write carries the
//! `Book·FixedIncome` capability, which resolves ONLY from an authenticated session),
//! and asserts the new typed FI-risk methods against the SDK's own already-validated
//! `price_rates` path (the single-position rollup equals the single price, and an
//! offsetting pay/receive book nets to zero). Never a mock. Every body is hard
//! wall-clock bounded and every network await is bounded, so a regression fails fast,
//! never hangs.

mod common;

use celnet_client::{
    CivilDate, Ois, RatesAggregateQuery, RatesPosition, RatesPositionQuery, RatesRiskScope,
    UsdSofrCurve,
};

use common::{STEP_DEADLINE, TEST_DEADLINE, start_edge_and_authed_client};

/// A calibrated USD-SOFR curve (three dated par-OIS pillars) the tests price against.
fn curve() -> UsdSofrCurve {
    UsdSofrCurve::new(CivilDate::new(2026, 6, 25))
        .pillar(1, 0.0432)
        .pillar(2, 0.0418)
        .pillar(5, 0.0405)
}

/// The single-position firm rollup equals the SDK's own `price_rates` of that same
/// OIS bit-for-bit — the additive fan-in over one leaf is the leaf itself. This
/// validates the NEW typed `aggregate_rates_risk` against the already-validated
/// typed rates-price path (both server-computed, neither summed client-side).
#[tokio::test]
async fn single_position_aggregate_equals_the_price() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_authed_client().await;
        let curve = curve();
        let ois = Ois::receive_fixed(5, 0.0405).notional(100_000_000.0);

        let priced = step(client.price_rates(&curve, &ois)).await;

        let pos = RatesPosition::new(1, 10, ois);
        let agg = step(client.aggregate_rates_risk(&RatesAggregateQuery::new(curve, [pos]))).await;

        assert_eq!(agg.nodes.len(), 1, "one settlement-currency node");
        let node = &agg.nodes[0];
        assert_eq!(node.ccy, "USD");
        assert!(
            celnet_core::is_close(node.net_pv, priced.pv, 1e-9, 1e-4),
            "aggregate net_pv {} == price_rates pv {}",
            node.net_pv,
            priced.pv
        );
        assert!(
            celnet_core::is_close(node.net_pv01, priced.pv01, 1e-9, 1e-6),
            "aggregate net_pv01 {} == price_rates pv01 {}",
            node.net_pv01,
            priced.pv01
        );
        assert!(
            celnet_core::is_close(node.net_dv01, priced.dv01, 1e-9, 1e-6),
            "aggregate net_dv01 {} == price_rates dv01 {}",
            node.net_dv01,
            priced.dv01
        );
        assert!(
            !node.key_rate_ladder.is_empty(),
            "the key-rate DV01 ladder is bucketed by tenor"
        );

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// An offsetting pay-fixed + receive-fixed book of the SAME swap nets to zero PV /
/// PV01 / DV01 in the firm rollup — the additive-netting invariant, proven through
/// the new typed method.
#[tokio::test]
async fn offsetting_book_nets_to_zero() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_authed_client().await;
        let curve = curve();
        let n = 50_000_000.0;
        let pay = RatesPosition::new(1, 10, Ois::pay_fixed(5, 0.041).notional(n));
        let recv = RatesPosition::new(1, 20, Ois::receive_fixed(5, 0.041).notional(n));

        let agg =
            step(client.aggregate_rates_risk(&RatesAggregateQuery::new(curve, [pay, recv]))).await;

        assert_eq!(agg.nodes.len(), 1, "a single USD netting node");
        let node = &agg.nodes[0];
        assert!(
            node.net_pv.abs() < 1e-3,
            "the equal-and-opposite book nets PV to ~0, got {}",
            node.net_pv
        );
        assert!(
            node.net_dv01.abs() < 1e-6,
            "the equal-and-opposite book nets DV01 to ~0, got {}",
            node.net_dv01
        );

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// A rates position books into the firm rates store (an authenticated write carrying
/// the `Book·FixedIncome` capability), gets a server-assigned id, and lists back
/// through the typed listing — scoped by `(entity, book, ccy)`.
#[tokio::test]
async fn book_then_list_rates_positions() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_authed_client().await;

        let a = step(client.book_rates_position(&RatesPosition::new(
            1,
            10,
            Ois::pay_fixed(5, 0.0405).notional(75_000_000.0),
        )))
        .await;
        assert!(a.position_id > 0, "the server assigns a fresh id");
        assert_eq!(a.entity, 1);
        assert_eq!(a.book, 10);
        assert_eq!(a.instrument.tenor_years(), 5);

        let b = step(client.book_rates_position(&RatesPosition::new(
            2,
            20,
            Ois::receive_fixed(2, 0.0418).notional(40_000_000.0),
        )))
        .await;
        assert!(b.position_id > a.position_id, "ids are monotone");

        // The whole booked book lists back.
        let all = step(client.list_rates_positions(&RatesPositionQuery::new())).await;
        assert_eq!(all.len(), 2, "both booked positions list");

        // A scoped listing prunes to the matching entity.
        let entity_1 = step(client.list_rates_positions(
            &RatesPositionQuery::new().scoped(RatesRiskScope::new().entity(1)),
        ))
        .await;
        assert_eq!(entity_1.len(), 1, "only entity 1's position");
        assert_eq!(entity_1[0].entity, 1);
        assert_eq!(entity_1[0].position_id, a.position_id);

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// Bound a single rates call so a never-arriving reply fails fast.
async fn step<T>(fut: impl std::future::Future<Output = celnet_client::ClientResult<T>>) -> T {
    tokio::time::timeout(STEP_DEADLINE, fut)
        .await
        .expect("rates call resolves in time")
        .expect("rates call succeeds")
}
