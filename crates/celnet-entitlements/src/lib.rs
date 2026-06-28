//! Celnet **entitlements**: the principal model + server-side pre-aggregation
//! pruning predicate (`docs/RISK-HIERARCHY.md` §2.6/§4,
//! `docs/EXPERIENCE-ARCHITECTURE.md` P2-8/§3).
//!
//! # What this crate is
//!
//! The "who sees what" layer above the risk cube. A [`Principal`] carries **grant**
//! rules (the dimension subtrees it may read) and **deny** rules (information
//! barriers — Chinese walls). The [`EntitlementFilter`] applies that principal as a
//! **predicate over the cube's fact stream, *before* any roll-up**, so a node total
//! can never leak the magnitude of a subtree the principal cannot see (the
//! *aggregate-leakage* hazard, §2.6). The decision is pure and deterministic:
//!
//! ```text
//! admitted  ⇔  (grant-all  ∨  some grant covers the fact)  ∧  no deny covers it
//! ```
//!
//! # Deny by default — grant-all is an explicit, audited choice
//!
//! The crate default principal is [`Principal::scoped`] with no grants: it admits
//! **nothing** until a grant is added (the §4 separation-of-duties posture).
//! [`Principal::grant_all`] still exists — it is the firm-wide view a caller may
//! *explicitly assert* (and the substitute the server's **explicit permissive
//! dev-mode** applies for a demo edge) — but it is never an implicit fallback.
//! The [`decision`] module carries the trust-boundary vocabulary the server's
//! service edge decides and audits with: [`AccessMode`] (deny-by-default
//! [`AccessMode::Enforce`] vs the loud dev-only [`AccessMode::Permissive`]),
//! [`AccessDecision`] (allow/deny) and the typed [`AccessReason`] behind it.
//! Because every aggregation path flows the fact stream through
//! [`EntitlementFilter`], slotting a differently-scoped principal in changes
//! *only which facts the predicate admits* — no aggregation call site and no wire
//! contract moves.
//!
//! # How it composes with the cube's group-by
//!
//! [`celnet_risk_cube::Cube`] reduces facts with `group_by` / `firm_aggregate`.
//! Pruning sits **upstream** of ingestion:
//!
//! ```text
//! fact stream ─▶ EntitlementFilter::entitled_cube(principal, hierarchy, facts)
//!             ─▶ Cube (admitted facts only) ─▶ group_by / firm_aggregate / VaR …
//! ```
//!
//! A [`Scope`] pins one cube dimension to a value (a subtree, e.g. `Desk = 99`),
//! and resolves coverage through the **same** [`Hierarchy`](celnet_risk_cube::Hierarchy)
//! parent pointers (`Book → Desk`, `Location → Entity`) the cube rolls up with — so
//! a scope covers exactly the facts that roll into the matching node. The scope key
//! space *is* the group-by key space ([`FactKey::group_value`](celnet_risk_cube::FactKey::group_value)),
//! so there is no separate scope encoding to drift out of sync with the cube.
//!
//! # Honest scope
//!
//! This crate owns the **model + predicate + decision vocabulary** only. The
//! **audit** of every entitlement decision (§4: "every entitlement decision
//! logged via `celnet-observability`") is implemented at the server's service
//! boundary (`celnet-server::services::access`), which makes the allow/deny
//! decision in this crate's [`decision`] vocabulary and emits one structured
//! security-class record per decision — this crate is pure and does **no** IO,
//! so it neither logs nor allocates a logger; it defines the decision the server
//! records. Transport-level **authentication** (binding the asserted principal
//! to a real caller identity) is the deployment environment's job — mTLS / an
//! authenticating gateway — and is deliberately not faked here.
//! Role↔principal *assignment* and user-admin (P2-8's GUI half) live above this
//! crate at the server/GUI edge; this crate is the deterministic kernel they
//! build on.
//!
//! # Determinism
//!
//! Every function is pure and allocation-disciplined; for a fixed principal,
//! hierarchy and fact order the admitted set and its order are reproducible. No
//! float math is involved (the predicate is over integer dimension keys), so there
//! is no `libm`/ULP concern here.
//!
//! # Provenance
//!
//! The server-side-pruning-before-aggregation mechanism and the deny-wins
//! information-barrier semantics follow `docs/RISK-HIERARCHY.md` §2.6/§4; the
//! grant-all default mirrors `docs/EXPERIENCE-ARCHITECTURE.md` §3. Provenance is in
//! doc comments only; no method/person/vendor name appears in any identifier
//! (guardrail #8).

#![forbid(unsafe_code)]

pub mod capability;
pub mod decision;
pub mod filter;
pub mod principal;
pub mod scope;

pub use capability::{Action, AssetClass, Capability, CapabilitySet};
pub use decision::{AccessDecision, AccessMode, AccessReason};
pub use filter::EntitlementFilter;
pub use principal::Principal;
pub use scope::{Rule, Scope};

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_risk_cube::{
        BookId, Cube, DeskId, DimensionId, EntityId, FactKey, FactMeasure, Hierarchy, LocationId,
        NodeAggregate, PositionId, RiskFact, TraderId, VegaPillar, VegaPillarMap,
    };
    use celnet_risk_normalize::{CanonicalLeaf, PositionRisk, canonicalize};
    use celnet_types::{Ccy, CcyPair, DeltaConvention, OptionType, PremiumStyle, VanillaInputs};

    fn eurusd() -> CcyPair {
        CcyPair::new(Ccy::EUR, Ccy::USD)
    }
    fn usdjpy() -> CcyPair {
        CcyPair::new(Ccy::USD, Ccy::JPY)
    }

    /// A pillar map bucketing by rounded tenor-days and a single 0.50Δ pillar —
    /// enough to drive the cube's additive ladder deterministically.
    struct DaysPillar;
    impl VegaPillarMap for DaysPillar {
        fn pillar_of(&self, _leaf: &CanonicalLeaf, position: &PositionRisk) -> VegaPillar {
            let days = (position.inputs.t * 365.0).round() as u32;
            VegaPillar::new(days, 5000)
        }
    }

    fn pos(pair: CcyPair, opt: OptionType, notional: f64, inputs: VanillaInputs) -> PositionRisk {
        PositionRisk::fx(
            pair,
            opt,
            notional,
            inputs,
            DeltaConvention::SpotUnadjusted,
            PremiumStyle::DomesticPips,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn fact(
        id: u32,
        trader: u32,
        book: u32,
        desk: u32,
        loc: u32,
        ent: u32,
        position: PositionRisk,
    ) -> RiskFact {
        RiskFact {
            position_id: PositionId(id),
            key: FactKey {
                trader: TraderId(trader),
                book: BookId(book),
                desk: DeskId(desk),
                underlying: position.underlying.clone(),
                location: LocationId(loc),
                entity: EntityId(ent),
            },
            measure: FactMeasure {
                leaf: canonicalize(&position).unwrap(),
                position,
                exotic: None,
            },
            surface_version: 1,
        }
    }

    fn standard_inputs() -> VanillaInputs {
        VanillaInputs::new(1.10, 1.12, 0.10, 1.0, 0.04, 0.02)
    }

    /// Three facts spanning two desks: desk 1 = {pos1 (book 10), pos2 (book 11)},
    /// desk 2 = {pos3 (book 20)}. Books mapped to desks via the hierarchy so a
    /// `Desk` scope must resolve the `Book → Desk` parent pointer.
    fn sample() -> (Hierarchy, Vec<RiskFact>) {
        let mut h = Hierarchy::new();
        h.set_book_desk(BookId(10), DeskId(1));
        h.set_book_desk(BookId(11), DeskId(1));
        h.set_book_desk(BookId(20), DeskId(2));
        let facts = vec![
            // desk 1
            fact(
                1,
                100,
                10,
                0,
                1,
                1,
                pos(eurusd(), OptionType::Call, 10_000_000.0, standard_inputs()),
            ),
            fact(
                2,
                101,
                11,
                0,
                1,
                1,
                pos(
                    usdjpy(),
                    OptionType::Put,
                    8_000_000.0,
                    VanillaInputs::new(156.0, 154.0, 0.11, 0.5, 0.01, 0.05),
                ),
            ),
            // desk 2
            fact(
                3,
                102,
                20,
                0,
                2,
                2,
                pos(
                    eurusd(),
                    OptionType::Call,
                    7_000_000.0,
                    VanillaInputs::new(1.10, 1.15, 0.09, 1.0, 0.04, 0.02),
                ),
            ),
        ];
        (h, facts)
    }

    fn firm_delta(cube: &Cube) -> f64 {
        cube.firm_aggregate(&DaysPillar).net_greeks.delta_base
    }

    /// The grant-all principal admits EVERYTHING: the entitled cube is byte-for-byte
    /// the unfiltered cube, and the firm aggregate is unchanged. This is the
    /// zero-rework identity — flowing facts through the filter today changes nothing.
    #[test]
    fn grant_all_sees_everything() {
        let (h, facts) = sample();
        let principal = Principal::grant_all();
        assert!(principal.is_grant_all());

        let filter = EntitlementFilter::new(&principal, &h);
        let pruned = filter.prune(&facts);
        assert_eq!(pruned.len(), facts.len(), "grant-all prunes nothing");
        assert_eq!(
            pruned, facts,
            "grant-all preserves the exact fact set + order"
        );

        // The entitled cube == an unfiltered cube built from the same facts.
        let entitled = filter.entitled_cube(facts.iter().cloned());
        let mut unfiltered = Cube::with_hierarchy(h.clone());
        for f in &facts {
            unfiltered.upsert(f.clone());
        }
        assert_eq!(entitled.len(), unfiltered.len());
        assert_eq!(firm_delta(&entitled), firm_delta(&unfiltered));
    }

    /// The crate default principal is deny-by-default: scoped with no grants, it
    /// admits **nothing** until explicitly granted. Firm-wide visibility is only
    /// ever the explicit [`Principal::grant_all`] constructor.
    #[test]
    fn default_principal_admits_nothing() {
        let principal = Principal::default();
        assert!(!principal.is_grant_all());
        let (h, facts) = sample();
        let filter = EntitlementFilter::new(&principal, &h);
        assert!(
            filter.prune(&facts).is_empty(),
            "the default principal must admit nothing (deny-by-default)"
        );
    }

    /// A scoped principal is PRUNED TO ITS SUBTREE BEFORE AGGREGATION — and
    /// critically, the firm total it sees equals the sum of ONLY its desk's facts,
    /// proving there is NO aggregate leakage from the desk it cannot see.
    #[test]
    fn scoped_principal_pruned_before_aggregation_no_leakage() {
        let (h, facts) = sample();

        // Principal entitled to desk 1 only (any book/pair/entity under it).
        let principal = Principal::scoped().grant(Rule::on(DimensionId::Desk, 1));
        let filter = EntitlementFilter::new(&principal, &h);

        // Only facts 1 & 2 (the desk-1 books 10 & 11) are admitted; fact 3 (desk 2)
        // is pruned — and resolved through the Book → Desk parent pointer, since the
        // facts' own desk key is 0.
        let pruned = filter.prune(&facts);
        assert_eq!(pruned.len(), 2);
        assert!(pruned.iter().all(|f| f.position_id != PositionId(3)));

        // The firm aggregate the scoped principal sees == ONLY desk-1 facts.
        let scoped_firm = firm_delta(&filter.entitled_cube(facts.iter().cloned()));
        let want_desk1 = canonicalize(&facts[0].measure.position)
            .unwrap()
            .greeks
            .delta_base
            + canonicalize(&facts[1].measure.position)
                .unwrap()
                .greeks
                .delta_base;
        assert!(
            (scoped_firm - want_desk1).abs() < 1e-9,
            "scoped firm {scoped_firm} must equal desk-1 sum {want_desk1}, not the whole firm"
        );

        // And it must DIFFER from the true firm total (which includes desk 2) — the
        // proof that desk-2's magnitude does not leak into the scoped node.
        let true_firm = {
            let mut c = Cube::with_hierarchy(h.clone());
            for f in &facts {
                c.upsert(f.clone());
            }
            firm_delta(&c)
        };
        assert!(
            (scoped_firm - true_firm).abs() > 1e-9,
            "scoped total must not equal the firm total (no leakage of desk 2)"
        );

        // Drill-down likewise sees only its subtree: grouping by Book yields exactly
        // the two desk-1 books, never the desk-2 book.
        let entitled = filter.entitled_cube(facts.iter().cloned());
        let by_book = entitled.group_by(DimensionId::Book, &DaysPillar);
        let mut books: Vec<u64> = by_book.iter().map(|a: &NodeAggregate| a.group).collect();
        books.sort_unstable();
        assert_eq!(books, vec![10, 11]);
    }

    /// A grant on one axis leaves the OTHER axes wide: "ccy_pair = EURUSD" admits
    /// EURUSD facts across every desk/book/entity, and prunes the USDJPY fact. This
    /// proves dimension orthogonality (§2.1) is honoured by the predicate.
    #[test]
    fn grant_on_one_axis_keeps_other_axes_wide() {
        let (h, facts) = sample();
        let eurusd_value = facts[0].key.group_value(DimensionId::Underlying);
        let principal = Principal::scoped().grant(Rule::on(DimensionId::Underlying, eurusd_value));
        let filter = EntitlementFilter::new(&principal, &h);

        let pruned = filter.prune(&facts);
        // Facts 1 (desk 1) and 3 (desk 2) are EURUSD; fact 2 is USDJPY → pruned.
        assert_eq!(pruned.len(), 2);
        assert!(
            pruned
                .iter()
                .all(|f| f.key.underlying.as_ccy_pair() == Some(eurusd()))
        );
        // Spans both desks — the grant did not constrain the desk axis.
        assert!(pruned.iter().any(|f| f.position_id == PositionId(1)));
        assert!(pruned.iter().any(|f| f.position_id == PositionId(3)));
    }

    /// A conjunctive (multi-axis) rule narrows the intersection: "desk 1 AND
    /// entity 1" admits only desk-1 facts booked in entity 1.
    #[test]
    fn conjunctive_rule_intersects_axes() {
        let (h, facts) = sample();
        let principal =
            Principal::scoped().grant(Rule::on(DimensionId::Desk, 1).and(DimensionId::Entity, 1));
        let filter = EntitlementFilter::new(&principal, &h);
        let pruned = filter.prune(&facts);
        // Both desk-1 facts are in entity 1, so both pass; desk-2 fact is cut.
        assert_eq!(pruned.len(), 2);
        assert!(pruned.iter().all(|f| f.key.entity == EntityId(1)));
    }

    /// DENY WINS over grant: an information barrier cuts a subtree even when it sits
    /// inside an otherwise-granted (here grant-all) scope. The walled book vanishes
    /// before aggregation, so its magnitude cannot leak into the firm total.
    #[test]
    fn deny_wins_over_grant_information_barrier() {
        let (h, facts) = sample();
        // Firm-wide principal, but walled out of book 11 (a Chinese wall).
        let principal = Principal::grant_all().deny(Rule::on(DimensionId::Book, 11));
        let filter = EntitlementFilter::new(&principal, &h);

        let pruned = filter.prune(&facts);
        assert_eq!(pruned.len(), 2, "book 11 is walled off");
        assert!(pruned.iter().all(|f| f.key.book != BookId(11)));

        // Firm total excludes the walled book.
        let walled_firm = firm_delta(&filter.entitled_cube(facts.iter().cloned()));
        let want = canonicalize(&facts[0].measure.position)
            .unwrap()
            .greeks
            .delta_base
            + canonicalize(&facts[2].measure.position)
                .unwrap()
                .greeks
                .delta_base;
        assert!((walled_firm - want).abs() < 1e-9);
    }

    /// A scoped principal with NO grants admits nothing (deny-by-default posture).
    #[test]
    fn scoped_principal_with_no_grants_admits_nothing() {
        let (h, facts) = sample();
        let principal = Principal::scoped();
        let filter = EntitlementFilter::new(&principal, &h);
        assert!(filter.prune(&facts).is_empty());
        assert_eq!(filter.entitled_cube(facts.iter().cloned()).len(), 0);
    }

    /// The single-fact predicate matches the slice prune (consistency of the
    /// `admits` building block the server uses for incremental upserts).
    #[test]
    fn single_fact_admits_matches_prune() {
        let (h, facts) = sample();
        let principal = Principal::scoped().grant(Rule::on(DimensionId::Desk, 2));
        let filter = EntitlementFilter::new(&principal, &h);
        let pruned = filter.prune(&facts);
        for f in &facts {
            assert_eq!(filter.admits(f), pruned.contains(f));
        }
        // Only the desk-2 fact (resolved via book 20 → desk 2).
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].position_id, PositionId(3));
    }
}
