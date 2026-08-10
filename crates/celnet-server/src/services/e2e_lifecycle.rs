//! End-to-end **incoming-order trade-lifecycle** suite.
//!
//! Every other test in the crate exercises one stage in isolation. This suite drives a
//! REAL incoming client order through the WHOLE pipeline and asserts every stage:
//!
//! ```text
//!   LAST-LOOK ─▶ ACCEPTANCE ─▶ BOOK ─▶ RISK ROUTING ─▶ INTERNALISE/HEDGE ─▶ risk-book aggregation
//! ```
//!
//! It drives the real server services — no pipeline mocks:
//!
//!  * **Desk seam** — [`RfqDeskEdge`] submit → auto-quote → [`RfqDeskEdge::evaluate_fix_acceptance`]
//!    (last-look + acceptance graph) → [`RfqDeskEdge::book_fix_lift`] (book → route →
//!    internalise). Reached through the shared [`Pipeline::submit_order`] fixture. This is
//!    the true incoming-order entrypoint, so acceptance / last-look / routing / deal
//!    stamping are asserted here.
//!  * **Store seam** — [`RatesPositionStore::book_with_routing`] driven directly with a
//!    controlled dealt price + reference mid ([`priced_attribution`]). This is the SAME
//!    booking chain `book_fix_lift` calls internally (`book_with_routing → stamp_internalise
//!    → internalise::verdict → AutoHedgeEngine::evaluate`), but with the dealer edge as an
//!    exact literal input rather than an engine-derived par mid — so the numeric
//!    internalise assertions (edge-bps sign, DV01 split, RAG band) are exact and
//!    non-circular. Used for the internalise / hedge-band / reconciliation scenarios.
//!
//! Both are real: the store seam is not a mock — it is the booking engine the desk seam
//! delegates to. The two seams together cover the full pipeline; each scenario asserts the
//! stages it faithfully reaches and comments any stage a given entry cannot.
//!
//! ## Numeric derivations (all re-derived from published formulas, not from the engine)
//!
//! * **Dealer edge** ([`crate::services::internalise::dealer_edge_bps`]):
//!   `edge_bps = dir·(dealt − mid) / bp_scale`, `dir = −1` pay-fixed (SIDE_BUY) /
//!   `+1` receive-fixed (SIDE_SELL); `bp_scale = 1e-4` for a rate, `1e-2` for a bond price.
//!   Pay-fixed 4.00% vs a 4.05% mid ⇒ `−1·(0.0400 − 0.0405)/1e-4 = +5.0 bp`.
//! * **DV01 proxy** ([`crate::services::rates_book::rates_linear_exposure`]):
//!   `|notional|·tenor_years·1e-4`, signed `+` pay-fixed / `−` receive-fixed. A 10mm 5y OIS
//!   ⇒ `10e6·5·1e-4 = 5000`; a 25mm 5y OIS ⇒ `12500`.
//! * **RAG band** ([`celnet_limits::LimitSpec::classify`], `ratio = |net|/cap`):
//!   `Breach` iff `ratio > 1.0`, else `Red` iff `ratio ≥ 0.9`, else `Amber` iff
//!   `ratio ≥ 0.8`, else `Green`. (At `ratio == 1.0` the band is Red, not Breach.)
//! * **Warehouse split** (`stamp_internalise` + `default_hedge_policy_graph`): while under
//!   tolerance the fill warehouses, shedding only the over-cap `overflow = (|net| −
//!   target).max(0)` externally (clamped to the fill DV01), `target = target_fraction·cap`.
//!   Below tolerance the whole fill goes to an advisory external back-to-back.

#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use celnet_acceptance::{
    AcceptanceAction, AcceptanceDecision, AcceptanceField, AcceptanceGraph, RouteOp, RouteValue,
};
use celnet_limits::{LimitMetric, LimitScope, LimitSpec};
use celnet_proto::{Deal, OisInstrument, RatesInstrument, RatesPosition, Side, rates_instrument};
use celnet_risk_routing::{RiskRoutingGraph, RouteField, RoutingNode};

use crate::config::identity::{IdentityStore, RiskBookEdit};
use crate::services::desk::RfqDeskEdge;
use crate::services::desk::tests::{edge_with_rates, gate_graph, quote_request};
use crate::services::rates_book::tests::{
    hedge_policy, position, priced_attribution, single_book_graph,
};
use crate::services::rates_book::{RatesPositionStore, RatesRoutingAttribution};
use crate::services::risk::book_risk::{RiskBookRisk, aggregate_risk_book};
use crate::services::risk::store::PositionStore;

// ---------------------------------------------------------------------------------------
// Shared fixture
// ---------------------------------------------------------------------------------------

/// Create an ENABLED risk book (optionally under `parent`) and return the id the store
/// mints from the name — so routing graphs and aggregation target the exact stamp the
/// store writes (never a guessed slug).
fn enabled_book(identity: &mut IdentityStore, name: &str, parent: Option<&str>) -> String {
    identity
        .create_risk_book(RiskBookEdit {
            name: name.to_owned(),
            parent_id: parent.map(str::to_owned),
            desk_id: None,
            description: String::new(),
            limits: None,
            enabled: true,
        })
        .expect("create enabled risk book")
        .id
}

/// The dashboard roll-up the `ListRiskBookRisk` RPC serves for one book: the real
/// [`aggregate_risk_book`] over the routed rates fills (with an empty FX book), rolling up
/// the book's whole subtree.
fn aggregate(rates: &RatesPositionStore, identity: &IdentityStore, book_id: &str) -> RiskBookRisk {
    let book = identity.risk_book(book_id).expect("known risk book");
    aggregate_risk_book(&PositionStore::new(), Some(rates), identity, book)
}

/// A routing attribution carrying only the routing keys (counterparty / ccy) and no priced
/// dealt/mid — so routing + booking + aggregation run but no internalise decision is stamped.
fn attribution(counterparty: &str, ccy: &str) -> RatesRoutingAttribution {
    RatesRoutingAttribution {
        counterparty: counterparty.to_owned(),
        ccy: ccy.to_owned(),
        dealt_price: None,
        reference_mid: None,
        request_id: None,
    }
}

/// A receive-fixed (SIDE_SELL) 5y OIS fill — the opposite netting sign to [`position`]'s
/// pay-fixed (SIDE_BUY) fill: signed notional and DV01 are both negative.
fn sell_fill(notional: f64) -> RatesPosition {
    RatesPosition {
        position_id: 0,
        entity: 1,
        book: 10,
        instrument: Some(RatesInstrument {
            instrument: Some(rates_instrument::Instrument::Ois(OisInstrument {
                tenor_years: 5,
                fixed_rate: 0.0405,
                notional,
                side: Side::Sell as i32,
            })),
        }),
    }
}

/// A single-condition risk-routing graph: `counterparty == cp ? on_match : on_else`.
fn cp_routing(cp: &str, on_match: &str, on_else: &str) -> RiskRoutingGraph {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        0u32,
        RoutingNode::Condition {
            field: RouteField::Counterparty,
            op: RouteOp::Eq,
            value: RouteValue::Text(cp.to_owned()),
            on_true: 1,
            on_false: 2,
        },
    );
    nodes.insert(
        1u32,
        RoutingNode::Book {
            risk_book_id: on_match.to_owned(),
        },
    );
    nodes.insert(
        2u32,
        RoutingNode::Book {
            risk_book_id: on_else.to_owned(),
        },
    );
    RiskRoutingGraph { entry: 0, nodes }
}

/// A two-level routing graph: `counterparty == cp → cp_book; else ccy == eur_ccy →
/// eur_book; else default_book`.
fn two_way_routing(
    cp: &str,
    cp_book: &str,
    eur_ccy: &str,
    eur_book: &str,
    default_book: &str,
) -> RiskRoutingGraph {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        0u32,
        RoutingNode::Condition {
            field: RouteField::Counterparty,
            op: RouteOp::Eq,
            value: RouteValue::Text(cp.to_owned()),
            on_true: 1,
            on_false: 2,
        },
    );
    nodes.insert(
        1u32,
        RoutingNode::Book {
            risk_book_id: cp_book.to_owned(),
        },
    );
    nodes.insert(
        2u32,
        RoutingNode::Condition {
            field: RouteField::Ccy,
            op: RouteOp::Eq,
            value: RouteValue::Text(eur_ccy.to_owned()),
            on_true: 3,
            on_false: 4,
        },
    );
    nodes.insert(
        3u32,
        RoutingNode::Book {
            risk_book_id: eur_book.to_owned(),
        },
    );
    nodes.insert(
        4u32,
        RoutingNode::Book {
            risk_book_id: default_book.to_owned(),
        },
    );
    RiskRoutingGraph { entry: 0, nodes }
}

/// The outcome of one incoming order through the desk seam.
struct OrderOutcome {
    /// The QUOTED desk-request / QuoteID the order minted.
    request_id: String,
    /// The last-look + acceptance-graph verdict.
    decision: AcceptanceDecision,
    /// The booked deal — `None` when acceptance refused it, OR when it cleared acceptance
    /// but the routed booking was rejected (a hard-limit breach in `book_with_routing`).
    deal: Option<Deal>,
}

/// The shared pipeline fixture: a real [`RatesPositionStore`] wired with a given acceptance
/// graph + routing graph + hedge policy + firm limits, and the real [`RfqDeskEdge`] over it.
struct Pipeline {
    rates: Arc<RatesPositionStore>,
    edge: RfqDeskEdge,
    identity: IdentityStore,
}

impl Pipeline {
    /// Drive one incoming order through the full desk chain: submit → auto-quote (QUOTED) →
    /// LAST-LOOK + ACCEPTANCE → (on Accept) BOOK → ROUTE → INTERNALISE.
    ///
    /// A Reject or a Hold does NOT auto-book (a Hold leaves the request QUOTED for a human,
    /// mirroring the FIX venue orchestration in `services::fix`).
    async fn submit_order(
        &self,
        counterparty: &str,
        notional: f64,
        price: f64,
        quote_age_ms: f64,
    ) -> OrderOutcome {
        let request_id = quote_request(&self.edge, counterparty, notional, price).await;
        let decision = self.edge.evaluate_fix_acceptance(&request_id, quote_age_ms);
        let deal = if decision.is_accept() {
            self.edge.book_fix_lift(&request_id)
        } else {
            None
        };
        OrderOutcome {
            request_id,
            decision,
            deal,
        }
    }
}

/// Builder for [`Pipeline`] — installs the acceptance/routing/hedge/limit state a scenario
/// needs and mints its enabled risk books.
struct PipelineBuilder {
    identity: IdentityStore,
    acceptance: Option<AcceptanceGraph>,
    routing: Option<RiskRoutingGraph>,
    hedge: Option<crate::services::rates_book::RatesHedgePolicy>,
    limits: Vec<(LimitScope, LimitSpec)>,
}

impl PipelineBuilder {
    fn new() -> Self {
        Self {
            identity: IdentityStore::default(),
            acceptance: None,
            routing: None,
            hedge: None,
            limits: Vec::new(),
        }
    }

    fn risk_book(&mut self, name: &str, parent: Option<&str>) -> String {
        enabled_book(&mut self.identity, name, parent)
    }

    fn acceptance(mut self, graph: AcceptanceGraph) -> Self {
        self.acceptance = Some(graph);
        self
    }

    fn routing(mut self, graph: RiskRoutingGraph) -> Self {
        self.routing = Some(graph);
        self
    }

    fn hedge(mut self, policy: crate::services::rates_book::RatesHedgePolicy) -> Self {
        self.hedge = Some(policy);
        self
    }

    fn limit(mut self, scope: LimitScope, spec: LimitSpec) -> Self {
        self.limits.push((scope, spec));
        self
    }

    fn build(self) -> Pipeline {
        let rates = Arc::new(RatesPositionStore::new());
        if let Some(a) = self.acceptance {
            rates.set_acceptance(Some(a));
        }
        if let Some(r) = self.routing {
            rates.set_routing(Some(r));
        }
        if let Some(h) = self.hedge {
            rates.set_hedge_policy(Some(h));
        }
        for (scope, spec) in self.limits {
            rates.set_limit(scope, spec);
        }
        let edge = edge_with_rates(Arc::clone(&rates));
        Pipeline {
            rates,
            edge,
            identity: self.identity,
        }
    }
}

// ---------------------------------------------------------------------------------------
// 1. ACCEPT → INTERNALISE (happy path)
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn s01_accept_then_internalise_happy_path() {
    // -- Store seam: the exact internalise numerics on the real book→route→internalise chain.
    let mut identity = IdentityStore::default();
    let wh = enabled_book(&mut identity, "Warehouse", None);
    let store = RatesPositionStore::new();
    store.set_routing(Some(single_book_graph(&wh)));
    store.set_hedge_policy(Some(hedge_policy(&wh, 100_000.0, 0.5)));

    // Desk pays fixed (OIS SIDE_BUY) a 5y swap at 4.00% vs a 4.05% fair mid.
    //   edge_bps = (mid − dealt)/1bp = (0.0405 − 0.0400)/1e-4 = +5.0 bp  (≥ 0.5 floor ⇒ within tolerance)
    //   DV01     = |10mm|·5·1e-4 = 5000; pay-fixed ⇒ +5000; |net| 5000 ≪ 100k cap ⇒ green ⇒ warehouse ⇒ shed 0
    let booked = store
        .book_with_routing(position(0, 1, 10), priced_attribution(0.0400, 0.0405))
        .expect("books");
    let prov = store
        .internalise_of(booked.position_id)
        .expect("a priced fill under a hedge policy is stamped");
    assert!(prov.within_tolerance);
    assert!(
        prov.internalised,
        "under cap + making money ⇒ fully internalised"
    );
    assert!(
        (prov.edge_bps - 5.0).abs() < 1e-6,
        "pay-fixed 5bp under mid = +5bp dealer edge, got {}",
        prov.edge_bps
    );
    assert!((prov.internal_dv01 - 5000.0).abs() < 1e-6);
    assert_eq!(prov.external_dv01, 0.0, "nothing shed to the street");
    assert_eq!(prov.hedge_band, "green");
    // Routed into the enabled book …
    assert_eq!(
        store.risk_book_of(booked.position_id).as_deref(),
        Some(wh.as_str())
    );
    // … and the dashboard roll-up shows the right NET / DV01.
    let agg = aggregate(&store, &identity, &wh);
    assert_eq!(agg.position_count, 1);
    assert!((agg.dv01.expect("rates dv01") - 5000.0).abs() < 1e-6);
    assert!(
        (agg.net_notional - 10_000_000.0).abs() < 1e-3,
        "pay-fixed 10mm signed notional"
    );

    // -- Desk seam: the SAME chain reached from the real FIX-lift entrypoint. The desk mid is
    // the engine par rate (asserted exactly at the store seam above), so here we prove the
    // ACCEPTED lift books a deal that routes into the book and carries the internalise stamp.
    let mut b = PipelineBuilder::new();
    let wh2 = b.risk_book("Warehouse", None);
    let pipe = b
        .routing(single_book_graph(&wh2))
        .hedge(hedge_policy(&wh2, 100_000_000.0, 0.0))
        .build();
    let out = pipe
        .submit_order("cp-bank", 25_000_000.0, 0.0411, 0.0)
        .await;
    assert!(out.decision.is_accept(), "acceptance passes");
    let deal = out.deal.expect("an accepted lift books a deal");
    assert_eq!(
        deal.risk_book_id.as_deref(),
        Some(wh2.as_str()),
        "deal carries the routed book"
    );
    let deal_prov = deal
        .internalise
        .clone()
        .expect("the booked deal carries an internalise decision");
    assert!(deal_prov.edge_bps.is_finite());
    assert!(matches!(
        deal_prov.hedge_band.as_str(),
        "green" | "amber" | "red" | "breach"
    ));
    // The deal's stamp matches the store's own record for the booked position.
    assert_eq!(
        Some(deal_prov),
        pipe.rates
            .internalise_of(deal.position_id.expect("position id"))
    );
    let agg = aggregate(&pipe.rates, &pipe.identity, &wh2);
    assert_eq!(
        agg.position_count, 1,
        "the routed fill shows on the dashboard"
    );
    assert!(agg.dv01.is_some(), "a routed rates fill surfaces a DV01");
}

// ---------------------------------------------------------------------------------------
// 2. ACCEPT → BACK-TO-BACK (thin / sub-floor edge)
// ---------------------------------------------------------------------------------------

#[test]
fn s02_accept_then_back_to_back_thin_edge() {
    let mut identity = IdentityStore::default();
    let wh = enabled_book(&mut identity, "Warehouse", None);
    let store = RatesPositionStore::new();
    store.set_routing(Some(single_book_graph(&wh)));
    store.set_hedge_policy(Some(hedge_policy(&wh, 100_000.0, 0.5)));

    // Pay-fixed 4.049% vs 4.05% mid → +0.1bp edge = (0.0405 − 0.04049)/1e-4, BELOW the 0.5bp floor.
    let booked = store
        .book_with_routing(position(0, 1, 10), priced_attribution(0.04049, 0.0405))
        .expect("books");
    let prov = store.internalise_of(booked.position_id).expect("stamped");
    assert!(!prov.within_tolerance, "0.1bp is below the 0.5bp floor");
    assert!(!prov.internalised, "a thin fill is not warehoused");
    assert_eq!(prov.internal_dv01, 0.0);
    assert!(
        (prov.external_dv01 - 5000.0).abs() < 1e-6,
        "the whole fill goes to an advisory external back-to-back"
    );
    // Live execution (the default LP-then-composite mode): the thin fill's full back-to-back
    // executes on the composite venue and books an OFFSETTING leg into the same book, so the
    // warehoused net reduces to ~0 — the risk is genuinely shed, not merely flagged. The
    // original fill's risk-book stamp is unchanged.
    assert_eq!(
        store.risk_book_of(booked.position_id).as_deref(),
        Some(wh.as_str())
    );
    let agg = aggregate(&store, &identity, &wh);
    assert_eq!(agg.position_count, 2, "the fill + its offsetting hedge leg");
    assert!(
        agg.dv01.expect("dv01").abs() < 1e-6,
        "the back-to-back hedges the net flat"
    );
}

// ---------------------------------------------------------------------------------------
// 3. ACCEPT → OVER-CAP SPLIT
// ---------------------------------------------------------------------------------------

#[test]
fn s03_accept_then_over_cap_split() {
    let mut identity = IdentityStore::default();
    let wh = enabled_book(&mut identity, "Warehouse", None);
    let store = RatesPositionStore::new();
    store.set_routing(Some(single_book_graph(&wh)));
    // cap 4000 DV01: the 5000-DV01 fill breaches (util 1.25 > 1.0 ⇒ breach). target =
    // 0.8·4000 = 3200 → overflow 5000 − 3200 = 1800 shed externally (clamped to the 5000
    // fill DV01), the remaining 3200 warehoused.
    store.set_hedge_policy(Some(hedge_policy(&wh, 4000.0, 0.5)));
    let booked = store
        .book_with_routing(position(0, 1, 10), priced_attribution(0.0400, 0.0405))
        .expect("books");
    let prov = store.internalise_of(booked.position_id).expect("stamped");
    assert!(prov.within_tolerance, "5bp clears the floor");
    assert!(
        !prov.internalised,
        "an over-cap fill is not fully internalised"
    );
    assert!(
        (prov.external_dv01 - 1800.0).abs() < 1e-6,
        "overflow shed externally"
    );
    assert!(
        (prov.internal_dv01 - 3200.0).abs() < 1e-6,
        "target warehoused"
    );
    assert_eq!(prov.hedge_band, "breach");
}

// ---------------------------------------------------------------------------------------
// 4. ACCEPTANCE REJECT — counterparty deny rule
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn s04_acceptance_reject_by_counterparty() {
    let mut b = PipelineBuilder::new();
    let wh = b.risk_book("Warehouse", None);
    let pipe = b
        .acceptance(gate_graph(
            AcceptanceField::Counterparty,
            RouteOp::Eq,
            RouteValue::Text("cp-bank".to_owned()),
            AcceptanceAction::Reject {
                reason: "blocked name".to_owned(),
            },
        ))
        .routing(single_book_graph(&wh))
        .build();

    let blocked = pipe
        .submit_order("cp-bank", 25_000_000.0, 0.0411, 0.0)
        .await;
    assert_eq!(
        blocked.decision,
        AcceptanceDecision::Reject("blocked name".to_owned())
    );
    assert!(blocked.deal.is_none(), "a rejected order books no deal");
    assert_eq!(pipe.rates.len(), 0, "no rates position booked");
    assert_eq!(
        aggregate(&pipe.rates, &pipe.identity, &wh).position_count,
        0,
        "the dashboard is unchanged by a rejected order"
    );

    // A DIFFERENT counterparty on the same instrument still fills.
    let other = pipe
        .submit_order("other-bank", 25_000_000.0, 0.0411, 0.0)
        .await;
    assert_eq!(other.decision, AcceptanceDecision::Accept);
    assert!(
        other.deal.is_some(),
        "a non-blocked counterparty still fills"
    );
    assert_eq!(pipe.rates.len(), 1);
}

// ---------------------------------------------------------------------------------------
// 5. ACCEPTANCE REJECT — notional-cap rule
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn s05_acceptance_reject_notional_cap() {
    let pipe = PipelineBuilder::new()
        .acceptance(gate_graph(
            AcceptanceField::NotionalUsd,
            RouteOp::Gt,
            RouteValue::Num(20_000_000.0),
            AcceptanceAction::Reject {
                reason: "over notional cap".to_owned(),
            },
        ))
        .build();

    let big = pipe
        .submit_order("cp-bank", 25_000_000.0, 0.0411, 0.0)
        .await;
    assert_eq!(
        big.decision,
        AcceptanceDecision::Reject("over notional cap".to_owned())
    );
    assert!(big.deal.is_none(), "an over-cap notional is turned away");
    assert_eq!(pipe.rates.len(), 0);

    let small = pipe.submit_order("cp-bank", 5_000_000.0, 0.0411, 0.0).await;
    assert_eq!(small.decision, AcceptanceDecision::Accept);
    assert!(small.deal.is_some(), "an under-cap notional books");
}

// ---------------------------------------------------------------------------------------
// 6. ACCEPTANCE REJECT — edge-floor UP FRONT (contrast with scenario 2)
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn s06_acceptance_reject_edge_floor_up_front() {
    // An acceptance edge-floor turns away a thin/negative-edge lift BEFORE booking. The desk
    // side is receive-fixed (opposite the SIDE_BUY request), so the acceptance edge is
    // (dealt − mid)/1bp. Dealing at 0.0 ⇒ edge = −mid/1e-4 < 0 for any positive par mid ⇒
    // below the 0.2bp floor ⇒ Reject. This CONTRASTS scenario 2, where an equivalently thin
    // edge IS booked then hedged out: the internalise floor only splits internal/external
    // AFTER booking, whereas the acceptance edge-floor refuses the trade entirely.
    let pipe = PipelineBuilder::new()
        .acceptance(gate_graph(
            AcceptanceField::EdgeBps,
            RouteOp::Lt,
            RouteValue::Num(0.2),
            AcceptanceAction::Reject {
                reason: "unprofitable".to_owned(),
            },
        ))
        .build();

    let turned_away = pipe.submit_order("cp-bank", 25_000_000.0, 0.0, 0.0).await;
    assert_eq!(
        turned_away.decision,
        AcceptanceDecision::Reject("unprofitable".to_owned())
    );
    assert!(
        turned_away.deal.is_none(),
        "an edge-floor reject is NOT booked"
    );
    assert_eq!(pipe.rates.len(), 0, "turned away up front — nothing booked");

    // A genuinely profitable lift (dealt far above any par mid, mid < 1.0) clears the floor.
    let profitable = pipe.submit_order("cp-bank", 25_000_000.0, 1.0, 0.0).await;
    assert_eq!(profitable.decision, AcceptanceDecision::Accept);
    assert!(
        profitable.deal.is_some(),
        "clearing the edge floor books the deal"
    );
}

// ---------------------------------------------------------------------------------------
// 7. ACCEPTANCE HOLD — not auto-filled, then manually booked
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn s07_acceptance_hold_then_manual_accept() {
    let mut b = PipelineBuilder::new();
    let wh = b.risk_book("Warehouse", None);
    let pipe = b
        .acceptance(gate_graph(
            AcceptanceField::NotionalUsd,
            RouteOp::Gt,
            RouteValue::Num(20_000_000.0),
            AcceptanceAction::HoldForReview {
                reason: "desk review".to_owned(),
            },
        ))
        .routing(single_book_graph(&wh))
        .build();

    let out = pipe
        .submit_order("cp-bank", 25_000_000.0, 0.0411, 0.0)
        .await;
    assert_eq!(
        out.decision,
        AcceptanceDecision::Hold("desk review".to_owned())
    );
    assert!(out.deal.is_none(), "a held lift is not auto-filled");
    assert_eq!(pipe.rates.len(), 0, "nothing booked on a hold");

    // A Hold does not consume the request — it stays QUOTED, so a human books it via the
    // platform lift path.
    let booked = pipe
        .edge
        .book_fix_lift(&out.request_id)
        .expect("the held-but-QUOTED request remains manually liftable");
    assert!(booked.position_id.is_some());
    assert_eq!(
        pipe.rates.len(),
        1,
        "the manual accept books the held quote"
    );
    assert_eq!(
        aggregate(&pipe.rates, &pipe.identity, &wh).position_count,
        1,
        "the manually-booked fill now shows on the dashboard"
    );
}

// ---------------------------------------------------------------------------------------
// 8. RISK ROUTING — per-book isolation + hierarchical roll-up
// ---------------------------------------------------------------------------------------

#[test]
fn s08_risk_routing_isolation_and_hierarchy() {
    // Books: BOOK-A (top level), EUR-RATES (top level), WAREHOUSE (child of PARENT).
    let mut identity = IdentityStore::default();
    let parent = enabled_book(&mut identity, "Parent", None);
    let wh = enabled_book(&mut identity, "Warehouse", Some(&parent));
    let book_a = enabled_book(&mut identity, "Book A", None);
    let eur = enabled_book(&mut identity, "EUR Rates", None);

    let store = RatesPositionStore::new();
    // counterparty == cp-a → BOOK-A; else ccy == EUR → EUR-RATES; else → WAREHOUSE.
    store.set_routing(Some(two_way_routing("cp-a", &book_a, "EUR", &eur, &wh)));

    let to_a = store
        .book_with_routing(position(0, 1, 10), attribution("cp-a", "USD"))
        .expect("books to book-a");
    let to_eur = store
        .book_with_routing(position(0, 1, 10), attribution("other", "EUR"))
        .expect("books to eur");
    let to_wh = store
        .book_with_routing(position(0, 1, 10), attribution("other", "USD"))
        .expect("books to warehouse");

    // Routing stamped each fill into its resolved book.
    assert_eq!(
        store.risk_book_of(to_a.position_id).as_deref(),
        Some(book_a.as_str())
    );
    assert_eq!(
        store.risk_book_of(to_eur.position_id).as_deref(),
        Some(eur.as_str())
    );
    assert_eq!(
        store.risk_book_of(to_wh.position_id).as_deref(),
        Some(wh.as_str())
    );

    // Each enabled book aggregates ONLY its own routed fill (10mm 5y pay-fixed ⇒ +5000 DV01).
    for book in [&book_a, &eur, &wh] {
        let agg = aggregate(&store, &identity, book);
        assert_eq!(agg.position_count, 1, "book {book} holds only its own fill");
        assert!((agg.dv01.expect("dv01") - 5000.0).abs() < 1e-6);
    }

    // The hierarchical PARENT rolls up its child WAREHOUSE's DV01 (parent has no own fills;
    // BOOK-A / EUR-RATES are NOT under it, so they do not roll in).
    let parent_agg = aggregate(&store, &identity, &parent);
    assert_eq!(
        parent_agg.position_count, 1,
        "parent rolls up its child's single fill"
    );
    assert!((parent_agg.dv01.expect("rolled-up dv01") - 5000.0).abs() < 1e-6);
}

// ---------------------------------------------------------------------------------------
// 9. PHANTOM-FILL GUARD — passes last-look + acceptance, breaches the hard cap → REJECTED
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn s09_phantom_fill_guard_rejects_hard_cap_breach() {
    let mut b = PipelineBuilder::new();
    let wh = b.risk_book("Warehouse", None);
    // A hard firm Delta cap of 1 base unit: a 25mm 5y OIS charges 25mm·5·1e-4 = 12500 DV01
    // against it — a hard breach that `book_with_routing` rejects BEFORE any mutation.
    let pipe = b
        .routing(single_book_graph(&wh))
        .limit(LimitScope::Firm, LimitSpec::hard(LimitMetric::Delta, 1.0))
        .build();

    let out = pipe
        .submit_order("cp-bank", 25_000_000.0, 0.0411, 0.0)
        .await;
    // It PASSES acceptance (and last-look) …
    assert!(out.decision.is_accept(), "clears last-look + acceptance");
    // … but the routed booking breaches the hard cap → book_fix_lift returns None: the
    // verdict is REJECTED, not FILLED (the 2f5ee56 fix — a phantom fill never lands).
    assert!(out.deal.is_none(), "a hard-limit-blown lift books no deal");
    assert_eq!(pipe.rates.len(), 0, "no phantom position added");
    assert_eq!(
        aggregate(&pipe.rates, &pipe.identity, &wh).position_count,
        0,
        "the dashboard is unchanged"
    );
}

// ---------------------------------------------------------------------------------------
// 10. LAST-LOOK PRECEDENCE — refused before the acceptance graph is consulted
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn s10_last_look_precedes_acceptance() {
    let mut b = PipelineBuilder::new();
    let wh = b.risk_book("Warehouse", None);
    // Accept-all acceptance so the ONLY thing that can refuse a lift is the last-look guard.
    let pipe = b.routing(single_book_graph(&wh)).build();

    // (a) A fresh, valid QuoteID proceeds THROUGH acceptance and books.
    let good = pipe
        .submit_order("cp-bank", 25_000_000.0, 0.0411, 0.0)
        .await;
    assert!(good.decision.is_accept());
    assert!(good.deal.is_some(), "a valid QuoteID fills");

    // (b) An UNKNOWN QuoteID is refused by the QUOTED-state last-look guard BEFORE the
    // acceptance graph could book anything. Acceptance on a missing request defaults to
    // Accept — proving it is LAST-LOOK, not acceptance, that turns the lift away.
    assert_eq!(
        pipe.edge.evaluate_fix_acceptance("no-such-quote", 0.0),
        AcceptanceDecision::Accept
    );
    assert!(
        pipe.edge.book_fix_lift("no-such-quote").is_none(),
        "an unknown QuoteID is turned away by last-look"
    );

    // (c) A REPLAYED (already-consumed) QuoteID: the first lift in (a) consumed it (state →
    // Accepted); a second lift is refused even though acceptance still says Accept.
    assert!(
        pipe.edge.book_fix_lift(&good.request_id).is_none(),
        "a replayed QuoteID is turned away by last-look"
    );
    assert_eq!(pipe.rates.len(), 1, "the replay adds no second position");

    // NOTE: true quote-TTL *expiry* is enforced at the FIX venue layer, not this RfqDeskEdge
    // seam (whose lift carries no wall clock). The nearest faithful expiry control HERE is a
    // distinct, acceptance-layer QuoteAgeMs floor:
    let stale_pipe = PipelineBuilder::new()
        .acceptance(gate_graph(
            AcceptanceField::QuoteAgeMs,
            RouteOp::Gt,
            RouteValue::Num(800.0),
            AcceptanceAction::Reject {
                reason: "stale quote".to_owned(),
            },
        ))
        .build();
    let stale = stale_pipe
        .submit_order("cp-bank", 25_000_000.0, 0.0411, 1_500.0)
        .await;
    assert_eq!(
        stale.decision,
        AcceptanceDecision::Reject("stale quote".to_owned())
    );
    assert!(stale.deal.is_none(), "a past-TTL quote age is rejected");
    let fresh = stale_pipe
        .submit_order("cp-bank", 25_000_000.0, 0.0411, 100.0)
        .await;
    assert_eq!(
        fresh.decision,
        AcceptanceDecision::Accept,
        "a fresh quote age proceeds"
    );
}

// ---------------------------------------------------------------------------------------
// 11. HEDGE BAND TRANSITIONS — green → amber → red → breach at the derived boundaries
// ---------------------------------------------------------------------------------------

#[test]
fn s11_hedge_band_transitions() {
    let mut identity = IdentityStore::default();
    let wh = enabled_book(&mut identity, "Warehouse", None);
    let store = RatesPositionStore::new();
    store.set_routing(Some(single_book_graph(&wh)));
    // cap 25_000 DV01, within-tolerance fills of +5000 DV01 each. util = |net|/25000:
    //   amber ≥ 0.8 (net ≥ 20000), red ≥ 0.9 (net ≥ 22500), breach > 1.0 (net > 25000);
    //   at net == 25000 (util 1.0) the band is Red, not Breach.
    // Pin ADVISORY execution so this band-classification walk is not perturbed by live hedging
    // shedding the accumulating net at the edge (the live-execution path is covered by s02 + the
    // rates unit tests); here we assert the RAG band each fill stamps as the net climbs.
    let mut band_policy = hedge_policy(&wh, 25_000.0, 0.5);
    band_policy.config.execution = crate::config::hedge_policy::HedgeExecutionMode::Advisory;
    store.set_hedge_policy(Some(band_policy));

    // Book one +5000-DV01 pay-fixed fill (+5bp edge, within tolerance) and read the band it
    // stamped against the post-fill net.
    let book_one = || {
        let b = store
            .book_with_routing(position(0, 1, 10), priced_attribution(0.0400, 0.0405))
            .expect("books");
        store
            .internalise_of(b.position_id)
            .expect("stamped")
            .hedge_band
    };

    assert_eq!(book_one(), "green", "net 5000 · util 0.20");
    assert_eq!(book_one(), "green", "net 10000 · util 0.40");
    assert_eq!(book_one(), "green", "net 15000 · util 0.60");
    assert_eq!(
        book_one(),
        "amber",
        "net 20000 · util 0.80 → amber boundary"
    );
    assert_eq!(
        book_one(),
        "red",
        "net 25000 · util 1.00 → red (ratio == 1.0, not breach)"
    );
    assert_eq!(book_one(), "breach", "net 30000 · util 1.20 → breach");
}

// ---------------------------------------------------------------------------------------
// 12. MULTI-FILL RECONCILIATION — each book's NET/GROSS/POSITIONS/DV01 == the summed fills
// ---------------------------------------------------------------------------------------

#[test]
fn s12_multi_fill_reconciliation() {
    let mut identity = IdentityStore::default();
    let book_a = enabled_book(&mut identity, "Book A", None);
    let wh = enabled_book(&mut identity, "Warehouse", None);
    let store = RatesPositionStore::new();
    store.set_routing(Some(cp_routing("cp-a", &book_a, &wh)));

    // BOOK-A: two pay-fixed 10mm fills from cp-a → +10mm signed, +5000 DV01 each.
    store
        .book_with_routing(position(0, 1, 10), attribution("cp-a", "USD"))
        .expect("books");
    store
        .book_with_routing(position(0, 1, 10), attribution("cp-a", "USD"))
        .expect("books");
    // WAREHOUSE: two pay-fixed (+10mm/+5000) + one receive-fixed (−10mm/−5000) fill.
    store
        .book_with_routing(position(0, 1, 10), attribution("other", "USD"))
        .expect("books");
    store
        .book_with_routing(position(0, 1, 10), attribution("other", "USD"))
        .expect("books");
    store
        .book_with_routing(sell_fill(10_000_000.0), attribution("other", "USD"))
        .expect("books");

    // BOOK-A reconciles to the summed fills: 2 positions, net = gross = +20mm, DV01 = +10000.
    let a = aggregate(&store, &identity, &book_a);
    assert_eq!(a.position_count, 2);
    assert!((a.net_notional - 20_000_000.0).abs() < 1e-3);
    assert!((a.gross_notional - 20_000_000.0).abs() < 1e-3);
    assert!((a.dv01.expect("dv01") - 10_000.0).abs() < 1e-6);

    // WAREHOUSE nets the receive-fixed leg: 3 positions, net = +10mm (20 − 10), gross = 30mm,
    // DV01 = 5000 + 5000 − 5000 = +5000.
    let w = aggregate(&store, &identity, &wh);
    assert_eq!(w.position_count, 3);
    assert!((w.net_notional - 10_000_000.0).abs() < 1e-3);
    assert!((w.gross_notional - 30_000_000.0).abs() < 1e-3);
    assert!((w.dv01.expect("dv01") - 5_000.0).abs() < 1e-6);
}
