//! Celnet **limits / entitlements** layer (`docs/RISK-HIERARCHY.md` §5,
//! `docs/EXPERIENCE-ARCHITECTURE.md` P2-7).
//!
//! # What this crate is
//!
//! The pre/post-trade **limit framework** that sits **above** the
//! [`celnet_risk_cube`] aggregation engine. The cube answers *"what is the netted /
//! re-derived risk at this node?"*; this crate answers *"is that within the limits
//! set at this node, and what happens if a proposed trade pushes it over?"*.
//!
//! Limits cascade **down the same hierarchy** as risk (trader → book → desk →
//! ccy-pair → location → entity → firm, RH §5.2) and are checked **at multiple
//! nodes simultaneously**: a single trade consumes limit at its trader, book, desk,
//! ccy-pair, and entity nodes at once. That is the operational reason the cube must
//! aggregate in real time — and why this crate addresses limits by a
//! [`LimitScope`] that maps 1:1 onto a cube group, so the same `group_by` /
//! `firm_aggregate` results drive both the risk display and the limit check.
//!
//! The three layers:
//!
//! 1. **Taxonomy & thresholds** ([`limit`]) — the industry-standard limit set
//!    (greek / bucketed-vega / tenor-bucket / concentration / VaR-ES / stop-loss,
//!    RH §5.1), each **soft** (warn) or **hard** (block), with **utilization** and
//!    **RAG status** as first-class node measures (RH §5.2).
//! 2. **The limit tree** ([`tree`]) — limits configured at any hierarchy scope, and
//!    the [`ScopePath`] a position is simultaneously constrained on (resolved
//!    through the cube's `book→desk` / `location→entity` parent pointers).
//! 3. **The checks** ([`check`]) — exposure extraction off a cube node, **pre-trade**
//!    projection + accept/warn/reject, and **post-trade** continuous monitoring with
//!    breach detection + escalation status (RH §5.3).
//!
//! # Additive vs non-additive (RH §2.5/§5.3)
//!
//! The crate inherits the cube's correctness split. **Additive** limits (greeks,
//! bucketed/tenor vega, concentration) read the node's already-summed
//! `NetGreeks`/`VegaLadder` in O(1), so the pre-trade additive-Greek path stays in
//! the µs-class budget (RH §3.5). **Non-additive** limits (VaR / ES / stop-loss)
//! consume a loss number **re-derived per node** by the cube's bump-and-revalue
//! reducers (RH §2.5), evaluated on the slower recompute-trigger cadence (RH §3.3);
//! the check layer takes that number as input ([`NonAdditiveExposure`]) and never
//! summed-child-VaRs (which would be wrong — VaR diversifies).
//!
//! # Scope & honesty
//!
//! - **Pure & deterministic, no IO / no market data.** The limit tree holds only
//!   the constraints; exposures are read from caller-supplied cube nodes; every
//!   classification is a pure ratio comparison. For a fixed tree + node set the
//!   result is bit-reproducible.
//! - **The crate evaluates limits; it does not own the cascade *constraint
//!   solver*.** RH §5.2's "child limits constrained by parents" (a desk's limit
//!   bounding the sum of its books' limits) is a **configuration-time** validation
//!   that belongs to the admin/entitlements path; this crate stores and evaluates
//!   whatever tree it is given. That cascade-consistency check is the named,
//!   **deferred** companion (it would live in `celnet-entitlements` / the admin
//!   workflow, P2-8) — not faked here.
//! - **Limit thresholds are external data**, never compiled-in: caps and the
//!   amber/red warning bands (RH §5.2's illustrative 80 %/90 %) are
//!   [`LimitSpec`] fields, so a re-calibration is data, not a recompile.
//! - **Entitlements vs limits.** This crate is the *limits* half of the
//!   limits/entitlements layer; the entitlement pre-aggregation predicate (EA §3,
//!   P2-8) is the sibling `celnet-entitlements` crate. A [`LimitScope`] is exactly
//!   the granularity an entitlement grant is expressed over, so the two compose
//!   cleanly.
//!
//! # Provenance
//!
//! The limit taxonomy, soft/hard cascade and pre/post-trade + escalation workflow
//! follow `docs/RISK-HIERARCHY.md` §5 (which cites the MiFID II / SEC 15c3-5
//! pre-trade mandate and the Murex/Calypso escalation workflows). Provenance is in
//! doc comments only; no method/person/vendor name appears in any identifier
//! (guardrail #8).

#![forbid(unsafe_code)]

pub mod check;
pub mod limit;
pub mod tree;

pub use check::{
    EscalationStatus, IncrementalTrade, LimitCheck, NonAdditiveExposure, PreTradeDecision,
    PreTradeResult, ScopeMonitor, check_scope, exposure_of, post_trade_check, pre_trade_check,
};
pub use limit::{ConcentrationMetric, Enforcement, LimitMetric, LimitSpec, RagStatus, Utilization};
pub use tree::{LimitScope, LimitTree, ScopePath};

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_risk_cube::{
        BookId, Cube, DeskId, DimensionId, EntityId, FactKey, FactMeasure, Hierarchy, LocationId,
        NetGreeks, NodeAggregate, PositionId, RiskFact, Scenario, TraderId, VegaLadder, VegaPillar,
        VegaPillarMap,
    };
    use celnet_risk_normalize::{CanonicalLeaf, PositionRisk, canonicalize};
    use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }

    /// A pillar map bucketing by integer-day tenor and a fixed 0.50Δ pillar.
    struct DaysPillar;
    impl VegaPillarMap for DaysPillar {
        fn pillar_of(&self, _leaf: &CanonicalLeaf, position: &PositionRisk) -> VegaPillar {
            VegaPillar::new((position.inputs.t * 365.0).round() as u32, 5000)
        }
    }

    fn pos(opt: OptionType, notional: f64, inputs: VanillaInputs) -> PositionRisk {
        PositionRisk::new(
            eurusd(),
            opt,
            notional,
            inputs,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        )
    }

    fn fact(id: u32, trader: u32, book: u32, desk: u32, position: PositionRisk) -> RiskFact {
        RiskFact {
            position_id: PositionId(id),
            key: FactKey {
                trader: TraderId(trader),
                book: BookId(book),
                desk: DeskId(desk),
                ccy_pair: position.pair,
                location: LocationId(1),
                entity: EntityId(1),
            },
            measure: FactMeasure {
                leaf: canonicalize(&position),
                position,
                exotic: None,
            },
            surface_version: 1,
        }
    }

    /// An empty node aggregate (all fields public) — a scope with no current risk.
    fn empty_node() -> NodeAggregate {
        NodeAggregate {
            group: 0,
            net_greeks: NetGreeks::zero(),
            vega_ladder: VegaLadder::new(),
            positions: Vec::new(),
            leaves: Vec::new(),
            exotic_legs: Vec::new(),
        }
    }

    fn one_book_cube(positions: &[(u32, PositionRisk)]) -> Cube {
        let mut cube = Cube::new();
        for (id, p) in positions {
            cube.upsert(fact(*id, 1, 1, 1, *p));
        }
        cube
    }

    // ---- limit::utilization / RAG math --------------------------------------

    /// **Utilization is `|exposure| / cap`, and RAG bands classify it exactly.**
    #[test]
    fn utilization_ratio_and_rag_bands() {
        let lim = LimitSpec::hard(LimitMetric::Delta, 10_000_000.0); // 80/90 bands.
        // 5mm of a 10mm cap → 0.5 → green.
        let u = lim.classify(5_000_000.0);
        assert_eq!(u.ratio, 0.5);
        assert_eq!(u.status, RagStatus::Green);
        assert_eq!(u.headroom(), 5_000_000.0);
        // Sign-agnostic: -8.5mm → 0.85 → amber (≥0.80, <0.90).
        assert_eq!(lim.classify(-8_500_000.0).status, RagStatus::Amber);
        // 9.5mm → 0.95 → red (≥0.90, ≤1.0).
        assert_eq!(lim.classify(9_500_000.0).status, RagStatus::Red);
        // 10mm exactly → 1.0 → still red, not a breach (at the cap, not over).
        assert_eq!(lim.classify(10_000_000.0).status, RagStatus::Red);
        // 10.0000001mm → breach.
        let over = lim.classify(10_000_001.0);
        assert_eq!(over.status, RagStatus::Breach);
        assert!(over.status.is_breach());
        assert!(over.headroom() < 0.0);
    }

    /// A **non-positive cap** is never silently unbounded: any exposure is a breach.
    #[test]
    fn zero_cap_is_a_breach_not_unbounded() {
        let lim = LimitSpec::hard(LimitMetric::Vega, 0.0);
        assert_eq!(lim.classify(0.0).status, RagStatus::Green); // no exposure, no breach.
        let u = lim.classify(1.0);
        assert!(u.ratio.is_infinite());
        assert_eq!(u.status, RagStatus::Breach);
    }

    /// **Custom warning bands** are honoured and clamped to a valid ordering.
    #[test]
    fn custom_bands_clamp_and_apply() {
        let lim = LimitSpec::soft(LimitMetric::Vega, 100.0).with_bands(0.5, 0.7);
        assert_eq!(lim.classify(60.0).status, RagStatus::Amber); // 0.60 ≥ 0.50.
        assert_eq!(lim.classify(80.0).status, RagStatus::Red); // 0.80 ≥ 0.70.
        // An inverted band (red < amber) is clamped so red ≥ amber, never inverting.
        let bad = LimitSpec::hard(LimitMetric::Delta, 10.0).with_bands(0.9, 0.1);
        assert!(bad.red >= bad.amber);
    }

    // ---- check::exposure_of off a cube node ---------------------------------

    /// **Exposure extraction reads the right additive measure off a node.** A delta
    /// limit sees `net_greeks.delta_base`; a vega-bucket limit sees the matching
    /// ladder bucket; a tenor-vega limit sums all delta pillars in that tenor.
    #[test]
    fn exposure_extraction_matches_node_aggregate() {
        let p1 = pos(
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02),
        );
        let p2 = pos(
            OptionType::Put,
            5_000_000.0,
            VanillaInputs::new(1.10, 1.08, 0.11, 1.0, 0.04, 0.02),
        );
        let cube = one_book_cube(&[(1, p1), (2, p2)]);
        let node = cube.firm_aggregate(&DaysPillar);
        let na = NonAdditiveExposure::default();

        let want_delta = canonicalize(&p1).greeks.delta_base + canonicalize(&p2).greeks.delta_base;
        assert_eq!(exposure_of(&node, LimitMetric::Delta, &na), want_delta);

        let pillar_1y = VegaPillar::new(365, 5000);
        let want_vega = canonicalize(&p1).greeks.vega + canonicalize(&p2).greeks.vega;
        assert_eq!(
            exposure_of(&node, LimitMetric::VegaBucket(pillar_1y), &na),
            want_vega
        );
        // Both positions are 1Y → the tenor cut equals the bucket here.
        assert_eq!(
            exposure_of(&node, LimitMetric::TenorVega { tenor_days: 365 }, &na),
            want_vega
        );
    }

    /// **Concentration uses gross (un-netted) magnitude.** A long+short pair that
    /// nets to ~0 delta still has a large gross delta concentration — the
    /// concentration metric charges it where the net greek would not.
    #[test]
    fn concentration_is_gross_not_net() {
        let inputs = VanillaInputs::new(1.10, 1.10, 0.10, 0.5, 0.03, 0.01);
        let long = pos(OptionType::Call, 10_000_000.0, inputs);
        let short = pos(OptionType::Call, -10_000_000.0, inputs);
        let cube = one_book_cube(&[(1, long), (2, short)]);
        let node = cube.firm_aggregate(&DaysPillar);
        let na = NonAdditiveExposure::default();

        // Net delta ≈ 0 (offsetting legs).
        let net = exposure_of(&node, LimitMetric::Delta, &na).abs();
        // Gross concentration ≈ 2× one leg's |delta|.
        let gross = exposure_of(
            &node,
            LimitMetric::Concentration(ConcentrationMetric::Delta),
            &na,
        );
        assert!(net < 1e-6, "net delta should net to ~0, got {net}");
        assert!(
            gross > 1e6,
            "gross concentration should be large, got {gross}"
        );
    }

    /// **Non-additive VaR exposure** is read from the supplied loss number (and is
    /// `0` when not evaluated this cycle — never spuriously breaching).
    #[test]
    fn nonadditive_var_exposure_from_reducer() {
        let inputs = VanillaInputs::new(1.10, 1.10, 0.10, 0.5, 0.03, 0.01);
        let cube = one_book_cube(&[(1, pos(OptionType::Call, 10_000_000.0, inputs))]);
        let node = cube.firm_aggregate(&DaysPillar);
        let scen: Vec<Scenario> = (-10..=10)
            .filter(|i| *i != 0)
            .map(|i| Scenario::spot(f64::from(i) * 0.005))
            .collect();
        let na = NonAdditiveExposure::from_scenarios(&node, &scen, 0.99);
        let var = exposure_of(&node, LimitMetric::Var, &na);
        assert!(var > 0.0);
        assert_eq!(var, na.var.unwrap());
        // Unevaluated VaR reads 0 → can't breach.
        let empty = NonAdditiveExposure::default();
        assert_eq!(exposure_of(&node, LimitMetric::Var, &empty), 0.0);
    }

    // ---- tree + scope path --------------------------------------------------

    /// **The limit tree stores per-scope limits and supersedes by metric.** Setting
    /// a desk delta cap twice keeps one current limit; a different metric coexists.
    #[test]
    fn tree_set_supersedes_by_metric() {
        let mut tree = LimitTree::new();
        let desk = LimitScope::Desk(DeskId(7));
        tree.set(desk, LimitSpec::hard(LimitMetric::Delta, 10_000_000.0));
        tree.set(desk, LimitSpec::hard(LimitMetric::Vega, 500_000.0));
        tree.set(desk, LimitSpec::hard(LimitMetric::Delta, 20_000_000.0)); // supersede.
        let limits = tree.at(desk);
        assert_eq!(limits.len(), 2, "delta superseded, vega coexists");
        let delta = limits
            .iter()
            .find(|l| l.metric == LimitMetric::Delta)
            .unwrap();
        assert_eq!(delta.cap, 20_000_000.0);
        assert_eq!(tree.iter().count(), 2);
    }

    /// **A position's scope path resolves desk/entity through the hierarchy parent
    /// pointers**, so a limit set at the desk is on the path even when the fact's
    /// own desk key is unset.
    #[test]
    fn scope_path_resolves_desk_via_parent_pointer() {
        let mut h = Hierarchy::new();
        h.set_book_desk(BookId(10), DeskId(99));
        let key = FactKey {
            trader: TraderId(1),
            book: BookId(10),
            desk: DeskId(0), // deliberately unset; the parent pointer wins.
            ccy_pair: eurusd(),
            location: LocationId(1),
            entity: EntityId(1),
        };
        let path = ScopePath::resolve(&key, &h);
        let scopes: Vec<_> = path.scopes().collect();
        assert!(scopes.contains(&LimitScope::Desk(DeskId(99))));
        assert!(scopes.contains(&LimitScope::Trader(TraderId(1))));
        assert_eq!(scopes.last(), Some(&LimitScope::Firm));
    }

    // ---- pre-trade -----------------------------------------------------------

    /// **Pre-trade hard breach rejects.** A book sits just under a delta cap; a
    /// proposed trade whose incremental delta pushes it over the cap is **rejected**
    /// by the pre-trade check, and the offending hard breach is reported.
    #[test]
    fn pre_trade_hard_breach_rejects() {
        // Current book: a long call with ~+delta.
        let current_pos = pos(
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02),
        );
        let cube = one_book_cube(&[(1, current_pos)]);
        let book_node = cube.group_by(DimensionId::Book, &DaysPillar)[0].clone();
        let current_delta = book_node.net_greeks.delta_base;

        // A delta cap set just above current exposure, so any further +delta breaches.
        let cap = current_delta.abs() + 1_000_000.0;
        let mut tree = LimitTree::new();
        tree.set(
            LimitScope::Book(BookId(1)),
            LimitSpec::hard(LimitMetric::Delta, cap),
        );

        // The proposed trade: another long call adding +delta beyond the headroom.
        let proposed = pos(
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02),
        );
        let leaf = canonicalize(&proposed);
        let incremental = IncrementalTrade::from_leaf(&leaf, VegaPillar::new(365, 5000));

        let path = ScopePath::resolve(
            &FactKey {
                trader: TraderId(1),
                book: BookId(1),
                desk: DeskId(1),
                ccy_pair: eurusd(),
                location: LocationId(1),
                entity: EntityId(1),
            },
            &Hierarchy::new(),
        );

        let result = pre_trade_check(
            &tree,
            &path,
            &incremental,
            |scope| {
                if scope == LimitScope::Book(BookId(1)) {
                    book_node.clone()
                } else {
                    empty_node()
                }
            },
            |_| NonAdditiveExposure::default(),
        );
        assert_eq!(result.decision, PreTradeDecision::Reject);
        assert!(!result.allowed());
        assert_eq!(result.hard_breaches().count(), 1);
        let breach = result.hard_breaches().next().unwrap();
        assert_eq!(breach.scope, LimitScope::Book(BookId(1)));
        assert_eq!(breach.limit.metric, LimitMetric::Delta);
    }

    /// **Pre-trade within headroom accepts; a soft over-cap warns (does not block).**
    #[test]
    fn pre_trade_accept_and_soft_warn() {
        let current_pos = pos(
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02),
        );
        let cube = one_book_cube(&[(1, current_pos)]);
        let book_node = cube.group_by(DimensionId::Book, &DaysPillar)[0].clone();
        let current_delta = book_node.net_greeks.delta_base.abs();

        let proposed = pos(
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02),
        );
        let leaf = canonicalize(&proposed);
        let incr_delta = leaf.greeks.delta_base.abs();
        let incremental = IncrementalTrade::from_leaf(&leaf, VegaPillar::new(365, 5000));
        let path = ScopePath::resolve(
            &FactKey {
                trader: TraderId(1),
                book: BookId(1),
                desk: DeskId(1),
                ccy_pair: eurusd(),
                location: LocationId(1),
                entity: EntityId(1),
            },
            &Hierarchy::new(),
        );

        // (a) A generous cap → accept.
        let mut accept_tree = LimitTree::new();
        accept_tree.set(
            LimitScope::Book(BookId(1)),
            LimitSpec::hard(LimitMetric::Delta, (current_delta + incr_delta) * 10.0),
        );
        let node_at = |scope: LimitScope| {
            if scope == LimitScope::Book(BookId(1)) {
                book_node.clone()
            } else {
                empty_node()
            }
        };
        let accept = pre_trade_check(&accept_tree, &path, &incremental, node_at, |_| {
            NonAdditiveExposure::default()
        });
        assert_eq!(accept.decision, PreTradeDecision::Accept);
        assert!(accept.allowed());

        // (b) A *soft* cap the trade exceeds → warn, but still allowed.
        let mut soft_tree = LimitTree::new();
        soft_tree.set(
            LimitScope::Book(BookId(1)),
            LimitSpec::soft(LimitMetric::Delta, current_delta), // incremental pushes over.
        );
        let warn = pre_trade_check(&soft_tree, &path, &incremental, node_at, |_| {
            NonAdditiveExposure::default()
        });
        assert_eq!(warn.decision, PreTradeDecision::Warn);
        assert!(warn.allowed(), "a soft breach warns but never blocks");
    }

    // ---- post-trade + escalation --------------------------------------------

    /// **Post-trade monitoring detects a breach and drives escalation (RAG).** A
    /// booked node over a hard cap reports `HardBreach` and a red/breach RAG; a node
    /// within band is `Clear`/green.
    #[test]
    fn post_trade_breach_detection_and_escalation() {
        let p = pos(
            OptionType::Call,
            10_000_000.0,
            VanillaInputs::new(1.10, 1.10, 0.10, 1.0, 0.04, 0.02),
        );
        let cube = one_book_cube(&[(1, p)]);
        let node = cube.group_by(DimensionId::Book, &DaysPillar)[0].clone();
        let exposure = node.net_greeks.delta_base.abs();
        let na = NonAdditiveExposure::default();

        // Hard limit below current exposure → breach + hard escalation.
        let mut tree = LimitTree::new();
        tree.set(
            LimitScope::Book(BookId(1)),
            LimitSpec::hard(LimitMetric::Delta, exposure * 0.5),
        );
        let mon = post_trade_check(&tree, LimitScope::Book(BookId(1)), &node, &na);
        assert_eq!(mon.worst, RagStatus::Breach);
        assert_eq!(mon.escalation, EscalationStatus::HardBreach);
        assert_eq!(mon.checks.len(), 1);

        // Same exposure under a soft limit → soft breach escalation, still surfaced.
        let mut soft_tree = LimitTree::new();
        soft_tree.set(
            LimitScope::Book(BookId(1)),
            LimitSpec::soft(LimitMetric::Delta, exposure * 0.5),
        );
        let soft = post_trade_check(&soft_tree, LimitScope::Book(BookId(1)), &node, &na);
        assert_eq!(soft.escalation, EscalationStatus::SoftBreach);

        // A generous cap → clear / green.
        let mut clear_tree = LimitTree::new();
        clear_tree.set(
            LimitScope::Book(BookId(1)),
            LimitSpec::hard(LimitMetric::Delta, exposure * 10.0),
        );
        let clear = post_trade_check(&clear_tree, LimitScope::Book(BookId(1)), &node, &na);
        assert_eq!(clear.worst, RagStatus::Green);
        assert_eq!(clear.escalation, EscalationStatus::Clear);
    }
}
