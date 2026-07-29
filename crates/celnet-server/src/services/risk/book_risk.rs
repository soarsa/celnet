//! Per-risk-book risk aggregation, rolled up the book tree
//! (`docs/FI-RISK-ROUTING-REQUIREMENTS.md` §5, §8.5).
//!
//! A [`PositionStore`] buckets each routed fill under the `risk_book_id` the
//! firm-wide graph resolved for it ([`PositionStore::positions_in_risk_book`]).
//! This module reduces those per-book fact sets into the metrics the per-book
//! risk dashboard needs, and — because risk books form a tree ([`RiskBookDef`]'s
//! `parent_id`) — **rolls a parent up over its whole subtree**: a parent's risk is
//! its own routed positions PLUS every descendant's. The union is over
//! *distinct* book ids (a book, and each of its descendants, are distinct ids and
//! a fact is routed to exactly one), so no position is double-counted.
//!
//! # What is REAL here vs deferred
//!
//! Every metric this module emits is computed from the facts actually present in
//! the store — nothing is fabricated (guardrail #2):
//!
//! - **net / gross notional, position count** — summed from each fact's signed
//!   base notional ([`PositionRisk::notional_base`]).
//! - **delta / gamma / vega / theta** — summed from each fact's **canonical leaf**
//!   Greeks ([`CanonicalGreeks`]), which are the *additive* leaf measures (§2.5):
//!   they are already scaled by the position's notional and carry a single
//!   convention, so the cube (and this roll-up) sums them directly. The summed
//!   units are therefore the canonical-leaf units: delta is a **base-leg hedge
//!   amount** (units of the base/asset leg), gamma is `∂²V/∂S² × notional`, vega is
//!   `∂V/∂σ` per `1.0` absolute vol in **premium currency × notional**, theta is
//!   `∂V/∂t` per year `× notional`. (Vega across mixed premium currencies is summed
//!   nominally here; a currency-correct vega netting is a presentation-layer concern
//!   tracked on the cube, §2.3 — flagged, not silently wrong, because the FI books
//!   this view serves are single-numeraire in practice.)
//! - **limit utilization** — for each cap present on the book's [`RiskLimits`] that
//!   is computable at this seam (net / gross notional), `used / limit` as a fraction
//!   plus a green/amber/red band.
//!
//! Two metric families are **genuinely not evaluable** at this seam and are carried
//! as ABSENT ([`Option::None`]) rather than a fabricated zero:
//!
//! - **DV01 / curve buckets** — FX-vanilla positions have no rates DV01; that is the
//!   rates-book seam (`services/rates_book.rs` + the `celnet-bond` risk leaf, §5.3).
//!   The `max_dv01` cap is therefore also skipped (no utilization without a DV01).
//! - **live mark-to-market PnL** — needs a mark pass that does not exist at this
//!   booking-seam view (§5.4).

use celnet_risk_cube::RiskFact;

use crate::config::identity::{IdentityStore, RiskBookDef, RiskLimits};

use super::store::PositionStore;

/// The traffic-light band for a limit utilization (`docs/FI-RISK-ROUTING-REQUIREMENTS.md`
/// §5): GREEN below 0.8, AMBER in `[0.8, 1.0)`, RED at or above 1.0 (a breach).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RagBand {
    /// `used / limit < 0.8` — comfortable headroom.
    Green,
    /// `used / limit` in `[0.8, 1.0)` — approaching the cap.
    Amber,
    /// `used / limit >= 1.0` — at or over the cap.
    Red,
}

/// The AMBER threshold: utilization at or above this (but below [`RAG_RED`]) is amber.
const RAG_AMBER: f64 = 0.8;
/// The RED threshold: utilization at or above this is a breach.
const RAG_RED: f64 = 1.0;

impl RagBand {
    /// The band for a utilization fraction. `NaN` never reaches here (the fraction is
    /// computed defensively, never `0/0`); a `+inf` fraction (used > 0 against a zero
    /// cap) is `>= RAG_RED` and so correctly RED.
    #[must_use]
    fn from_fraction(fraction: f64) -> Self {
        if fraction >= RAG_RED {
            Self::Red
        } else if fraction >= RAG_AMBER {
            Self::Amber
        } else {
            Self::Green
        }
    }
}

/// One book's utilization of a single limit cap (§5).
#[derive(Debug, Clone, PartialEq)]
pub struct LimitUtilization {
    /// The metric this cap governs (`net_notional` / `gross_notional`).
    pub metric: &'static str,
    /// The book's used magnitude for this metric (net uses the absolute of the signed sum).
    pub used: f64,
    /// The configured cap from the book's [`RiskLimits`].
    pub limit: f64,
    /// `used / limit` (0 when the cap is 0 and nothing is used; `+inf` when the cap is
    /// 0 and something is used — a breach).
    pub fraction: f64,
    /// The traffic-light band derived from [`fraction`](Self::fraction).
    pub band: RagBand,
}

impl LimitUtilization {
    /// Build a utilization row from a `used` magnitude and a cap. The fraction is
    /// computed defensively so a zero cap never yields `NaN`: `0/0` is 0 (nothing used,
    /// no capacity, still green), `x/0` for `x > 0` is `+inf` (a breach → red).
    #[must_use]
    fn new(metric: &'static str, used: f64, limit: f64) -> Self {
        let fraction = if limit > 0.0 {
            used / limit
        } else if used > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };
        Self {
            metric,
            used,
            limit,
            fraction,
            band: RagBand::from_fraction(fraction),
        }
    }
}

/// One risk book's aggregated risk, rolled up its subtree (§5).
#[derive(Debug, Clone, PartialEq)]
pub struct RiskBookRisk {
    /// The risk book this row is for ([`RiskBookDef::id`]).
    pub book_id: String,
    /// The book's human-friendly name ([`RiskBookDef::name`]).
    pub name: String,
    /// Net (signed sum) base-currency notional across the subtree's positions.
    pub net_notional: f64,
    /// Gross (sum of absolute) base-currency notional across the subtree.
    pub gross_notional: f64,
    /// The number of positions rolled into this book (own + descendants').
    pub position_count: u32,
    /// Canonical, premium-excluded delta (base-leg hedge amount) × notional, summed.
    pub delta: f64,
    /// Canonical gamma (`∂²V/∂S²`) × notional, summed.
    pub gamma: f64,
    /// Canonical vega (`∂V/∂σ` per 1.0 vol, premium-ccy) × notional, summed.
    pub vega: f64,
    /// Canonical theta (`∂V/∂t` per year) × notional, summed.
    pub theta: f64,
    /// Net DV01 (PV per +1bp) — ABSENT: FX-vanilla positions carry no rates DV01
    /// (the rates-book seam, §5.3).
    pub dv01: Option<f64>,
    /// Live mark-to-market PnL — ABSENT: no mark pass exists at this seam (§5.4).
    pub pnl: Option<f64>,
    /// Per-cap limit utilization for the caps present on the book AND computable now.
    pub limits: Vec<LimitUtilization>,
}

/// Reduce a book's rolled-up fact set into its [`RiskBookRisk`], computing limit
/// utilization against the book's own [`RiskLimits`]. Pure — the `facts` are already
/// the union of the book's own + descendants' positions (see [`aggregate_risk_book`]);
/// this is the numeric core, split out so it is testable without a store.
///
/// Notional is summed in the position's **base-leg currency** ([`PositionRisk::notional_base`]);
/// Greeks are summed in canonical-leaf units (see the module docs). DV01 and PnL are
/// not evaluable at this seam and are left `None`.
#[must_use]
fn aggregate_facts(book: &RiskBookDef, facts: &[RiskFact]) -> RiskBookRisk {
    let mut net_notional = 0.0;
    let mut gross_notional = 0.0;
    let mut delta = 0.0;
    let mut gamma = 0.0;
    let mut vega = 0.0;
    let mut theta = 0.0;
    for f in facts {
        let n = f.measure.position.notional_base;
        net_notional += n;
        gross_notional += n.abs();
        let g = &f.measure.leaf.greeks;
        delta += g.delta_base;
        gamma += g.gamma;
        vega += g.vega;
        theta += g.theta;
    }
    // `usize -> u32`: a live risk book is far under 4 billion routed lines; saturate
    // rather than wrap on the theoretical overflow (never a silent truncation).
    let position_count = u32::try_from(facts.len()).unwrap_or(u32::MAX);

    let limits = book
        .limits
        .as_ref()
        .map(|l| limit_utilizations(l, net_notional, gross_notional))
        .unwrap_or_default();

    RiskBookRisk {
        book_id: book.id.clone(),
        name: book.name.clone(),
        net_notional,
        gross_notional,
        position_count,
        delta,
        gamma,
        vega,
        theta,
        // Not evaluable at this seam — carried absent, never a fabricated zero.
        dv01: None,
        pnl: None,
        limits,
    }
}

/// The utilization rows for the caps present on `limits` that are computable at this
/// seam. `max_net_notional` uses the **absolute** of the signed net (a short book still
/// consumes net capacity); `max_gross_notional` uses the gross. `max_dv01` is
/// intentionally omitted while DV01 is a later rates seam (§5.3) — a utilization with no
/// numerator would be a fabricated number.
fn limit_utilizations(
    limits: &RiskLimits,
    net_notional: f64,
    gross_notional: f64,
) -> Vec<LimitUtilization> {
    let mut out = Vec::new();
    if let Some(cap) = limits.max_net_notional {
        out.push(LimitUtilization::new(
            "net_notional",
            net_notional.abs(),
            cap,
        ));
    }
    if let Some(cap) = limits.max_gross_notional {
        out.push(LimitUtilization::new("gross_notional", gross_notional, cap));
    }
    // `max_dv01`: deliberately skipped — DV01 is absent (§5.3), so it has no numerator.
    out
}

/// Aggregate a risk book's risk rolled up its subtree (§5): union the book's own routed
/// positions with every descendant's, then reduce via [`aggregate_facts`]. Pure and
/// cycle-safe (the descendant walk is [`IdentityStore::risk_book_descendants`]).
///
/// The union is over distinct book ids, and the store routes each fact to exactly one
/// book id, so a position is counted once.
#[must_use]
pub fn aggregate_risk_book(
    store: &PositionStore,
    identity: &IdentityStore,
    book: &RiskBookDef,
) -> RiskBookRisk {
    let mut facts = store.positions_in_risk_book(&book.id);
    for descendant in identity.risk_book_descendants(&book.id) {
        facts.extend(store.positions_in_risk_book(&descendant.id));
    }
    aggregate_facts(book, &facts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_risk_cube::{
        BookId as CubeBookId, DeskId, EntityId, FactKey, FactMeasure, LocationId, PositionId,
        TraderId,
    };
    use celnet_risk_normalize::{CanonicalGreeks, CanonicalLeaf, PositionRisk};
    use celnet_types::{
        Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, Underlying, VanillaInputs,
    };

    use crate::config::identity::{IdentityStore, RiskBookEdit, RiskLimits};
    use crate::services::risk::store::PositionStore;

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    /// A hand-built vanilla fact with an explicit notional and a delta/vega set proportional
    /// to it — enough to exercise the sums and RAG bands without invoking a pricer.
    fn hand_fact(id: u32, notional: f64) -> RiskFact {
        let position = PositionRisk::fx(
            eurusd(),
            OptionType::Call,
            notional,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        );
        let greeks = CanonicalGreeks {
            delta_base: notional * 0.5,
            gamma: notional * 1e-6,
            vega: notional * 2e-3,
            theta: notional * -1e-4,
            vanna: 0.0,
            volga: 0.0,
            charm: 0.0,
            speed: 0.0,
            zomma: 0.0,
            color: 0.0,
        };
        let leaf = CanonicalLeaf {
            underlying: Underlying::Fx(eurusd()),
            spot: 1.10,
            greeks,
            premium_quote: 0.0,
            vega_premium_ccy: Ccy::USD,
            quoted_was_premium_adjusted: false,
        };
        RiskFact {
            position_id: PositionId(id),
            key: FactKey {
                trader: TraderId(1),
                book: CubeBookId(1),
                desk: DeskId(1),
                underlying: Underlying::Fx(eurusd()),
                location: LocationId(1),
                entity: EntityId(1),
            },
            measure: FactMeasure {
                leaf,
                position,
                exotic: None,
            },
            surface_version: 1,
        }
    }

    fn book_with_limits(id: &str, limits: Option<RiskLimits>) -> RiskBookDef {
        RiskBookDef {
            id: id.to_owned(),
            name: id.to_uppercase(),
            parent_id: None,
            desk_id: None,
            description: String::new(),
            limits,
            enabled: true,
        }
    }

    /// The additive sums are the plain sum of the facts' notionals and canonical Greeks;
    /// net is signed, gross is absolute, and DV01/PnL stay ABSENT (never a fabricated 0).
    #[test]
    fn aggregate_facts_sums_notional_and_greeks() {
        let book = book_with_limits("b", None);
        // +100 long and -40 short: net 60, gross 140.
        let facts = [hand_fact(1, 100.0), hand_fact(2, -40.0)];
        let agg = aggregate_facts(&book, &facts);
        assert_eq!(agg.position_count, 2);
        assert!((agg.net_notional - 60.0).abs() < 1e-9);
        assert!((agg.gross_notional - 140.0).abs() < 1e-9);
        // delta_base = 0.5 * notional summed: 50 + (-20) = 30.
        assert!((agg.delta - 30.0).abs() < 1e-9);
        // vega = 2e-3 * notional summed: 0.2 + (-0.08) = 0.12.
        assert!((agg.vega - 0.12).abs() < 1e-12);
        // Genuinely-unavailable metrics are absent, not zeroed.
        assert_eq!(agg.dv01, None);
        assert_eq!(agg.pnl, None);
        // No limits configured ⇒ no utilization rows.
        assert!(agg.limits.is_empty());
    }

    /// RAG bands land exactly at the 0.8 / 1.0 boundaries: 0.8 ⇒ AMBER, 1.0 ⇒ RED,
    /// below 0.8 ⇒ GREEN. Net uses |signed sum|; gross uses the absolute sum.
    #[test]
    fn limit_utilization_rag_bands_at_boundaries() {
        let limits = RiskLimits {
            max_net_notional: Some(100.0),
            max_gross_notional: Some(100.0),
            max_dv01: Some(5.0),
        };
        let book = book_with_limits("b", Some(limits));

        // net = gross = 79 ⇒ 0.79 < 0.8 ⇒ GREEN.
        let green = aggregate_facts(&book, &[hand_fact(1, 79.0)]);
        assert_eq!(green.limits.len(), 2, "net + gross; dv01 skipped (no DV01)");
        assert!(green.limits.iter().all(|u| u.band == RagBand::Green));

        // net = gross = 80 ⇒ exactly 0.8 ⇒ AMBER (the lower boundary is inclusive).
        let amber = aggregate_facts(&book, &[hand_fact(1, 80.0)]);
        for u in &amber.limits {
            assert!((u.fraction - 0.8).abs() < 1e-12);
            assert_eq!(u.band, RagBand::Amber);
        }

        // net = gross = 100 ⇒ exactly 1.0 ⇒ RED (the cap is a breach at equality).
        let red = aggregate_facts(&book, &[hand_fact(1, 100.0)]);
        for u in &red.limits {
            assert!((u.fraction - 1.0).abs() < 1e-12);
            assert_eq!(u.band, RagBand::Red);
        }

        // A short book still consumes NET capacity via the absolute of the signed sum.
        let short = aggregate_facts(&book, &[hand_fact(1, -100.0)]);
        let net = short
            .limits
            .iter()
            .find(|u| u.metric == "net_notional")
            .expect("net cap present");
        assert!((net.used - 100.0).abs() < 1e-9);
        assert_eq!(net.band, RagBand::Red);
        // The dv01 cap is present on the book but NOT emitted (DV01 is absent).
        assert!(short.limits.iter().all(|u| u.metric != "max_dv01"));
    }

    /// A firm-wide graph: notional > 50 → PARENT, else → CHILD (a 2-leaf split).
    fn split_graph() -> celnet_risk_routing::RiskRoutingGraph {
        use celnet_risk_routing::{RouteField, RouteOp, RouteValue, RoutingNode};
        use std::collections::BTreeMap;
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0u32,
            RoutingNode::Condition {
                field: RouteField::Notional,
                op: RouteOp::Gt,
                value: RouteValue::Num(50.0),
                on_true: 1,
                on_false: 2,
            },
        );
        nodes.insert(
            1u32,
            RoutingNode::Book {
                risk_book_id: "parent".to_owned(),
            },
        );
        nodes.insert(
            2u32,
            RoutingNode::Book {
                risk_book_id: "child".to_owned(),
            },
        );
        celnet_risk_routing::RiskRoutingGraph { entry: 0, nodes }
    }

    fn booked(id: u64, notional: f64) -> crate::services::risk::store::BookedPosition {
        crate::services::risk::store::BookedPosition {
            position_id: id,
            pair: eurusd(),
            option: OptionType::Call,
            notional_base: notional,
            inputs: VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
            quoted_delta: DeltaConvention::SpotUnadjusted,
            premium_style: PremiumStyle::DomesticPips,
            surface_version: 1,
        }
    }

    /// A parent book aggregates its OWN routed positions PLUS its child's — the tree
    /// roll-up. Ground truth is the store's own per-book fact sets, so the roll-up must
    /// be exactly their union (count + notional + summed delta).
    #[test]
    fn aggregate_risk_book_rolls_up_the_subtree() {
        // Identity: CHILD nested under PARENT (a 2-level tree).
        let mut identity = IdentityStore::default();
        let parent = identity
            .create_risk_book(RiskBookEdit {
                name: "Parent".to_owned(),
                parent_id: None,
                desk_id: None,
                description: String::new(),
                limits: None,
                enabled: true,
            })
            .expect("create parent");
        let _child = identity
            .create_risk_book(RiskBookEdit {
                name: "Child".to_owned(),
                parent_id: Some(parent.id.clone()),
                desk_id: None,
                description: String::new(),
                limits: None,
                enabled: true,
            })
            .expect("create child");
        assert_eq!(parent.id, "parent");

        // Store: route one fill (60 > 50) to PARENT, one (10) to CHILD.
        let store = PositionStore::new();
        store.set_routing(Some(split_graph()));
        store
            .book_from_attribution(booked(1, 60.0), &attribution("PARENT"))
            .expect("book parent fill");
        store
            .book_from_attribution(booked(2, 10.0), &attribution("CHILD"))
            .expect("book child fill");

        // Ground truth: the raw per-book fact sets (the roll-up must be their union).
        let own: Vec<RiskFact> = store.positions_in_risk_book("parent");
        let child_facts: Vec<RiskFact> = store.positions_in_risk_book("child");
        assert_eq!(own.len(), 1, "one fill routed to parent");
        assert_eq!(child_facts.len(), 1, "one fill routed to child");
        let expect_delta: f64 = own
            .iter()
            .chain(child_facts.iter())
            .map(|f| f.measure.leaf.greeks.delta_base)
            .sum();

        // The child alone rolls up ONLY its own fill.
        let child_def = identity.risk_book("child").expect("child def");
        let child_agg = aggregate_risk_book(&store, &identity, child_def);
        assert_eq!(child_agg.position_count, 1);
        assert!((child_agg.net_notional - 10.0).abs() < 1e-9);

        // The parent rolls up BOTH: count 2, notional 70, delta = sum of both leaves.
        let parent_agg = aggregate_risk_book(&store, &identity, &parent);
        assert_eq!(parent_agg.position_count, 2);
        assert!((parent_agg.net_notional - 70.0).abs() < 1e-9);
        assert!((parent_agg.gross_notional - 70.0).abs() < 1e-9);
        assert!((parent_agg.delta - expect_delta).abs() < 1e-9);
        assert_eq!(parent_agg.dv01, None);
        assert_eq!(parent_agg.pnl, None);
    }

    fn attribution(book: &str) -> celnet_proto::AttributionRecord {
        use celnet_proto::{BookId, Owner, owner};
        celnet_proto::AttributionRecord {
            quoted_by: Some(BookId {
                book: "AUTO-MM".to_owned(),
                owner: Some(Owner {
                    seat: Some(owner::Seat::AutoPricer("celnet-auto-pricer".to_owned())),
                }),
            }),
            held_by: Some(BookId {
                book: book.to_owned(),
                owner: Some(Owner {
                    seat: Some(owner::Seat::Trader("jdoe".to_owned())),
                }),
            }),
            won: Some(true),
            lp_count: Some(3),
        }
    }
}
