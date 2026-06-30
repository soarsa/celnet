//! Trader-workflow integration tests for the firm-scale hierarchical-risk SDK
//! surface (`RiskService`): list the open book, roll it up over an org dimension,
//! drill a node to its constituents, and read limit RAG — every one a single
//! round-trip whose aggregation runs SERVER-SIDE (the API-first parity rule: an SDK
//! user gets the SAME risk surface the GUI Book view does, never looping positions
//! and summing client-side).
//!
//! Each test starts a real in-process `celnet-server` edge, seeds its shared live
//! position book + org hierarchy + limit tree through the edge's public `store()`
//! admin surface (the SAME book the RFS click-to-trade path feeds and `RiskService`
//! aggregates), dials it with the typed [`celnet_client`] SDK over gRPC, and asserts
//! the rolled-up results against the cube's own invariants — the additive roll-up
//! consistency (`firm == Σ children`), VaR diversification + presence tracking,
//! entitlement pruning, and limit-breach RAG. Never a mock. Every body is hard
//! wall-clock bounded and every network await is bounded, so a regression fails
//! fast, never hangs.

mod common;

use celnet_client::{
    AggregateQuery, DrillQuery, EntitlementScope, Entitlements, LimitQuery, Numeraire,
    OrgDimension, PositionQuery, Rag, Scope,
};
use celnet_limits::{LimitMetric, LimitScope, LimitSpec};
use celnet_risk_cube::DeskId;
use celnet_server::services::risk::store::{BookedPosition, PositionStore};
use celnet_types::{DeltaConvention, OptionType, PremiumStyle, VanillaInputs};

use celnet_proto::owner::Seat;
use celnet_proto::{AttributionRecord, BookId, Owner};

use common::{STEP_DEADLINE, TEST_DEADLINE, eurusd, start_edge_and_client};

/// The reporting numeraire the tests collapse to: USD, with the EUR→USD spot rate
/// the seeded EURUSD positions need to convert their base (EUR) leg. (USD's own
/// rate is implicitly 1.0.)
fn usd_numeraire() -> Numeraire {
    Numeraire::new("USD").rate("EUR", 1.10)
}

/// A booked vanilla position record: an EURUSD `option` of signed base notional
/// `notional_base`, marked under the standard interbank conventions.
fn booked(position_id: u64, option: OptionType, notional_base: f64) -> BookedPosition {
    BookedPosition {
        position_id,
        pair: eurusd(),
        option,
        notional_base,
        // A 1Y 10-vol EURUSD mark — the canonical leaf is re-derived from these
        // inputs server-side, independent of the live surface.
        inputs: VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        quoted_delta: DeltaConvention::SpotUnadjusted,
        premium_style: PremiumStyle::DomesticPips,
        surface_version: 1,
    }
}

/// An attribution chain holding a line in `book` under human trader `trader`.
fn held_by(book: &str, trader: &str) -> AttributionRecord {
    let id = BookId {
        book: book.to_owned(),
        owner: Some(Owner {
            seat: Some(Seat::Trader(trader.to_owned())),
        }),
    };
    AttributionRecord {
        quoted_by: Some(id.clone()),
        held_by: Some(id),
        won: Some(true),
        lp_count: Some(1),
    }
}

/// Seed a two-book desk into the store: book `"EUR-VOL-A"` (trader `"alice"`) holds
/// a 1mm long call + a 2mm long call; book `"EUR-VOL-B"` (trader `"bob"`) holds a
/// 1mm short call (offsetting). Both books hang off one desk handle. Returns the two
/// interned book handles and the desk handle.
fn seed_two_book_desk(store: &PositionStore) -> (u32, u32, u32) {
    let book_a = store.intern("EUR-VOL-A");
    let book_b = store.intern("EUR-VOL-B");
    let desk_h = store.intern("G10-VOL-DESK");
    store.set_book_desk(book_a, desk_h);
    store.set_book_desk(book_b, desk_h);

    store
        .book_from_attribution(
            booked(1, OptionType::Call, 1_000_000.0),
            &held_by("EUR-VOL-A", "alice"),
        )
        .expect("book 1");
    store
        .book_from_attribution(
            booked(2, OptionType::Call, 2_000_000.0),
            &held_by("EUR-VOL-A", "alice"),
        )
        .expect("book 2");
    store
        .book_from_attribution(
            booked(3, OptionType::Call, -1_000_000.0),
            &held_by("EUR-VOL-B", "bob"),
        )
        .expect("book 3");

    (book_a, book_b, desk_h)
}

/// The open book lists through the SDK with the real attribution chain, and the
/// grant-all default shows every seeded position.
#[tokio::test]
async fn list_positions_returns_the_entitled_open_book() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;
        seed_two_book_desk(edge.store());

        let listed = step(client.list_positions(&PositionQuery::new())).await;

        assert_eq!(listed.positions.len(), 3, "all three seeded positions list");
        // Each carries its org placement (a real ccy pair) and attribution chain.
        for p in &listed.positions {
            assert_eq!(p.org.ccy_pair, eurusd());
            let attr = p.attribution.as_ref().expect("attributed");
            assert!(
                attr.quoted_by.book == "EUR-VOL-A" || attr.quoted_by.book == "EUR-VOL-B",
                "attribution names the seeded book, got {:?}",
                attr.quoted_by.book
            );
        }
        // One position is the short leg.
        assert_eq!(
            listed
                .positions
                .iter()
                .filter(|p| p.notional_base < 0.0)
                .count(),
            1,
            "exactly one short position"
        );

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// The firm aggregate equals the sum of its per-book aggregates — the cube's core
/// additive roll-up invariant, validated end-to-end through the SDK. The offsetting
/// short book pulls the firm delta below the long book's, and the net stays long.
#[tokio::test]
async fn firm_aggregate_equals_sum_of_book_aggregates() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;
        seed_two_book_desk(edge.store());

        let by_book =
            step(client.aggregate_risk(&AggregateQuery::new(OrgDimension::Book, usd_numeraire())))
                .await;
        let firm =
            step(client.aggregate_risk(&AggregateQuery::new(OrgDimension::Firm, usd_numeraire())))
                .await;

        assert_eq!(by_book.dimension, OrgDimension::Book);
        assert_eq!(firm.dimension, OrgDimension::Firm);
        assert_eq!(firm.numeraire, "USD");
        assert_eq!(by_book.nodes.len(), 2, "two book nodes");
        assert_eq!(firm.nodes.len(), 1, "a single firm-apex node");

        let firm_node = &firm.nodes[0];

        // Additive roll-up: firm delta == Σ book delta, firm count == Σ book counts.
        let sum_delta: f64 = by_book
            .nodes
            .iter()
            .map(|n| n.additive.delta_numeraire)
            .sum();
        let sum_premium: f64 = by_book
            .nodes
            .iter()
            .map(|n| n.additive.premium_numeraire)
            .sum();
        let sum_count: u32 = by_book.nodes.iter().map(|n| n.position_count).sum();

        assert!(
            celnet_core::is_close(firm_node.additive.delta_numeraire, sum_delta, 1e-9, 1e-6),
            "firm delta {} == Σ book delta {}",
            firm_node.additive.delta_numeraire,
            sum_delta
        );
        assert!(
            celnet_core::is_close(
                firm_node.additive.premium_numeraire,
                sum_premium,
                1e-9,
                1e-6
            ),
            "firm premium == Σ book premium"
        );
        assert_eq!(firm_node.position_count, sum_count);
        assert_eq!(firm_node.position_count, 3);

        // The net book is long (1mm + 2mm − 1mm = +2mm base): positive base delta.
        assert!(
            firm_node.additive.delta_numeraire > 0.0,
            "net-long book has positive delta, got {}",
            firm_node.additive.delta_numeraire
        );

        // The per-ccy delta vector collapses back to the scalar through the rates:
        // a USD leg (the funding leg) + a EUR leg (the base leg) ⇒ scalar.
        assert!(
            !firm_node.additive.delta_vector.is_empty(),
            "the delta vector is reported leg-by-leg"
        );

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// VaR/ES are presence-tracked (absent without shocks, present with) and diversify:
/// a desk holding offsetting long+short books has a firm VaR below the sum of the
/// per-book VaRs (the non-additive re-derivation, not a sum).
#[tokio::test]
async fn value_at_risk_is_presence_tracked_and_diversifies() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;
        seed_two_book_desk(edge.store());

        // No shocks ⇒ the non-additive block is absent (never a spurious zero).
        let plain =
            step(client.aggregate_risk(&AggregateQuery::new(OrgDimension::Firm, usd_numeraire())))
                .await;
        assert!(
            plain.nodes[0].nonadditive.var.is_none(),
            "VaR is absent without shocks"
        );

        let shocks = [-0.02, -0.01, 0.0, 0.01, 0.02];

        // Firm VaR (the offsetting books net) at 99%.
        let firm = step(client.aggregate_risk(
            &AggregateQuery::new(OrgDimension::Firm, usd_numeraire()).value_at_risk(shocks, 0.99),
        ))
        .await;
        let firm_var = firm.nodes[0]
            .nonadditive
            .var
            .expect("VaR is present with shocks");
        assert!(firm_var >= 0.0, "VaR is a loss magnitude");
        assert_eq!(
            firm.nodes[0].nonadditive.var_alpha,
            Some(0.99),
            "the confidence level is echoed alongside the VaR"
        );

        // Per-book VaRs summed — the un-diversified figure.
        let by_book = step(client.aggregate_risk(
            &AggregateQuery::new(OrgDimension::Book, usd_numeraire()).value_at_risk(shocks, 0.99),
        ))
        .await;
        let sum_book_var: f64 = by_book
            .nodes
            .iter()
            .map(|n| n.nonadditive.var.expect("per-book VaR present"))
            .sum();

        assert!(
            firm_var <= sum_book_var + 1e-6,
            "firm VaR {firm_var} diversifies below the sum of book VaRs {sum_book_var}"
        );

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// A scoped entitlement principal sees only its granted book, never the whole firm —
/// the pre-aggregation pruning, proven through the SDK: a Book-A-only principal's
/// firm aggregate counts only Book A's positions, and a deny barrier cuts a book.
#[tokio::test]
async fn entitlement_principal_prunes_before_aggregation() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;
        let (book_a, book_b, _desk) = seed_two_book_desk(edge.store());

        // Grant-all firm view sees all three positions.
        let all =
            step(client.aggregate_risk(&AggregateQuery::new(OrgDimension::Firm, usd_numeraire())))
                .await;
        assert_eq!(all.nodes[0].position_count, 3);

        // A principal granted ONLY Book A sees just Book A's two positions.
        let only_a = Entitlements::scoped().grant(EntitlementScope::covering(Scope::at(
            OrgDimension::Book,
            u64::from(book_a),
        )));
        let scoped = step(client.aggregate_risk(
            &AggregateQuery::new(OrgDimension::Firm, usd_numeraire()).entitled(only_a),
        ))
        .await;
        assert_eq!(
            scoped.nodes[0].position_count, 2,
            "the Book-A-only principal sees exactly Book A's positions"
        );
        // The directional exposure lives in the per-currency legs (the collapsed
        // scalar is near-zero for a forward-hedged book — the EUR base leg and the
        // USD funding leg net through the numeraire rate). The Book-A-only view, with
        // no offsetting short, carries a strictly larger long EUR (base) leg.
        let eur_leg = |a: &celnet_client::AdditiveRisk| {
            a.delta_vector
                .iter()
                .find(|l| l.ccy == "EUR")
                .map(|l| l.amount)
                .expect("a EUR base-currency delta leg is reported")
        };
        assert!(
            eur_leg(&scoped.nodes[0].additive) > eur_leg(&all.nodes[0].additive),
            "without Book B's offsetting short, the scoped long-EUR delta leg is larger: \
             scoped {} vs firm {}",
            eur_leg(&scoped.nodes[0].additive),
            eur_leg(&all.nodes[0].additive)
        );

        // A grant-all firm view with a deny on Book B is the same restricted view.
        let deny_b = Entitlements::grant_all().deny(EntitlementScope::covering(Scope::at(
            OrgDimension::Book,
            u64::from(book_b),
        )));
        let walled = step(client.aggregate_risk(
            &AggregateQuery::new(OrgDimension::Firm, usd_numeraire()).entitled(deny_b),
        ))
        .await;
        assert_eq!(
            walled.nodes[0].position_count, 2,
            "deny wins — Book B is cut from the firm view"
        );

        // The listing is pruned identically (no leakage on the position list either).
        let listed = step(client.list_positions(&PositionQuery::new().entitled(
            Entitlements::scoped().grant(EntitlementScope::covering(Scope::at(
                OrgDimension::Book,
                u64::from(book_a),
            ))),
        )))
        .await;
        assert_eq!(listed.positions.len(), 2);

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// Drilling the firm node into its books reconciles to the by-book aggregate, and a
/// position drill returns the contributing leaves — the Book → Risk drill.
#[tokio::test]
async fn drill_firm_into_books_and_positions() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;
        let (book_a, _book_b, _desk) = seed_two_book_desk(edge.store());

        // Drill the firm apex into its book children + all contributing positions.
        let drill = step(
            client.drill_risk(
                &DrillQuery::new(Scope::firm(), OrgDimension::Book, usd_numeraire())
                    .children()
                    .positions(),
            ),
        )
        .await;
        assert_eq!(drill.node.dimension, OrgDimension::Firm);
        assert_eq!(drill.children.len(), 2, "two book children");
        assert_eq!(drill.positions.len(), 3, "all three leaves");

        // The drill children equal the standalone by-book aggregate.
        let by_book =
            step(client.aggregate_risk(&AggregateQuery::new(OrgDimension::Book, usd_numeraire())))
                .await;
        let mut child_deltas: Vec<f64> = drill
            .children
            .iter()
            .map(|n| n.additive.delta_numeraire)
            .collect();
        let mut agg_deltas: Vec<f64> = by_book
            .nodes
            .iter()
            .map(|n| n.additive.delta_numeraire)
            .collect();
        child_deltas.sort_by(|a, b| a.partial_cmp(b).unwrap());
        agg_deltas.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(child_deltas.len(), agg_deltas.len());
        for (c, a) in child_deltas.iter().zip(agg_deltas.iter()) {
            assert!(
                celnet_core::is_close(*c, *a, 1e-9, 1e-6),
                "drill child delta {c} == by-book aggregate delta {a}"
            );
        }

        // A position-only drill of Book A returns exactly its two leaves.
        let book_a_leaves = step(
            client.drill_risk(
                &DrillQuery::new(
                    Scope::at(OrgDimension::Book, u64::from(book_a)),
                    OrgDimension::Trader,
                    usd_numeraire(),
                )
                .positions(),
            ),
        )
        .await;
        assert_eq!(book_a_leaves.positions.len(), 2);
        assert!(
            book_a_leaves.children.is_empty(),
            "children were not requested"
        );

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// A hard vega cap below the desk's exposure breaches (BREACH + hard_breach +
/// negative headroom); a generous cap is GREEN with headroom to spare — the limit
/// RAG, server-evaluated and surfaced through the SDK. (Vega is the clean breach
/// exercise for a single-pair book: the collapsed delta self-funds to ~0 in the
/// common numeraire, whereas vega is a substantial non-zero exposure.)
#[tokio::test]
async fn limit_status_reports_breach_and_headroom() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;
        let (_book_a, _book_b, desk_h) = seed_two_book_desk(edge.store());
        let desk_scope = Scope::at(OrgDimension::Desk, u64::from(desk_h));

        // A generous desk vega cap ⇒ GREEN with positive headroom.
        edge.store().set_limit(
            LimitScope::Desk(DeskId(desk_h)),
            LimitSpec::hard(LimitMetric::Vega, 1.0e12),
        );
        let green = step(client.limit_status(&LimitQuery::new(desk_scope, usd_numeraire()))).await;
        assert_eq!(green.scope.dimension, OrgDimension::Desk);
        let green_vega = green
            .limits
            .iter()
            .find(|l| l.metric == celnet_client::LimitMetric::Vega)
            .expect("a vega limit is configured at the desk");
        assert_eq!(
            green_vega.status,
            Rag::Green,
            "a huge cap is comfortably green"
        );
        assert!(
            green_vega.headroom > 0.0,
            "headroom is positive under a huge cap"
        );
        assert!(!green.hard_breach, "no hard breach under a huge cap");

        // Supersede with a hard cap far below the desk's vega exposure ⇒ BREACH.
        edge.store().set_limit(
            LimitScope::Desk(DeskId(desk_h)),
            LimitSpec::hard(LimitMetric::Vega, 1.0),
        );
        let breached =
            step(client.limit_status(&LimitQuery::new(desk_scope, usd_numeraire()))).await;
        let breach_vega = breached
            .limits
            .iter()
            .find(|l| l.metric == celnet_client::LimitMetric::Vega)
            .expect("a vega limit is configured at the desk");
        assert_eq!(breach_vega.status, Rag::Breach, "a tiny cap is breached");
        assert!(
            breach_vega.headroom < 0.0,
            "headroom is negative over the cap"
        );
        assert!(breach_vega.ratio > 1.0, "exposure exceeds the cap");
        assert_eq!(breached.worst, Rag::Breach, "the worst RAG is the breach");
        assert!(
            breached.hard_breach,
            "a hard cap breach sets the escalation flag"
        );

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// A correlation id round-trips on every risk call (the observability handle).
#[tokio::test]
async fn correlation_id_round_trips() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let (edge, client, _data_dir) = start_edge_and_client().await;
        seed_two_book_desk(edge.store());

        let listed = step(client.list_positions(&PositionQuery::new().correlation_id(7))).await;
        assert_eq!(listed.correlation_id, Some(7));

        let agg = step(client.aggregate_risk(
            &AggregateQuery::new(OrgDimension::Firm, usd_numeraire()).correlation_id(11),
        ))
        .await;
        assert_eq!(agg.correlation_id, Some(11));

        edge.shutdown(STEP_DEADLINE).await;
    })
    .await
    .expect("test completes within the deadline");
}

/// Bound a single risk call so a never-arriving reply fails fast.
async fn step<T>(fut: impl std::future::Future<Output = celnet_client::ClientResult<T>>) -> T {
    tokio::time::timeout(STEP_DEADLINE, fut)
        .await
        .expect("risk call resolves in time")
        .expect("risk call succeeds")
}
