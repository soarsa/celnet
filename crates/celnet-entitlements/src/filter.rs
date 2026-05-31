//! The server-side **pre-aggregation pruning filter** (`docs/RISK-HIERARCHY.md`
//! §2.6/§4, `docs/EXPERIENCE-ARCHITECTURE.md` §3).
//!
//! # Why pruning must precede aggregation
//!
//! Entitlements are applied **before** any roll-up, never after. If a parent node
//! were aggregated over *all* facts and then the result handed to the principal,
//! the node total would betray the **magnitude** of subtrees the principal cannot
//! see — *aggregate leakage* (§2.6). The only leak-proof point is to admit a fact
//! into the aggregation **iff** the principal may see that leaf, then reduce only
//! the admitted set. This filter is that admission gate.
//!
//! # Composition with the cube's group-by
//!
//! [`celnet_risk_cube::Cube`] ingests facts via `upsert` and reduces them with
//! `group_by` / `firm_aggregate`. The filter sits **upstream** of ingestion:
//!
//! ```text
//! fact stream ─▶ EntitlementFilter::entitled_cube(principal, …) ─▶ Cube ─▶ group_by
//! ```
//!
//! [`EntitlementFilter::entitled_cube`] builds a [`Cube`] holding **only** the
//! admitted facts; every subsequent `group_by` / `firm_aggregate` / non-additive
//! re-derivation over that cube is therefore automatically leak-free at every node
//! and every drill level — the principal's drill-down can only ever descend into
//! subtrees it was admitted to (§4 "drill-down respects entitlements at every
//! level"). For the grant-all principal the admitted set is the full fact set, so
//! the entitled cube is identical to an unfiltered one — the zero-rework identity.
//!
//! [`EntitlementFilter::prune`] is the lower-level building block (slice → admitted
//! subset) for callers that maintain their own fact store and want only the
//! predicate.

use celnet_risk_cube::{Cube, Hierarchy, RiskFact};

use crate::principal::Principal;

/// The pruning filter: a [`Principal`] plus the org [`Hierarchy`] it resolves
/// scopes against. Pure and deterministic — same inputs, same admitted set, in the
/// same order.
///
/// The hierarchy is borrowed (not owned) so the filter resolves scopes against the
/// **same** parent pointers the cube rolls up with — one source of truth, no
/// duplicated org config that could drift.
#[derive(Debug, Clone, Copy)]
pub struct EntitlementFilter<'a> {
    principal: &'a Principal,
    hierarchy: &'a Hierarchy,
}

impl<'a> EntitlementFilter<'a> {
    /// A filter for `principal`, resolving dimension subtrees against `hierarchy`.
    #[must_use]
    pub fn new(principal: &'a Principal, hierarchy: &'a Hierarchy) -> Self {
        Self {
            principal,
            hierarchy,
        }
    }

    /// Whether this filter admits one fact (the per-fact predicate). Delegates to
    /// [`Principal::admits`]; exposed so callers can gate a single fact (e.g. a
    /// pre-trade check, or an incremental cube upsert) without materializing a set.
    #[must_use]
    pub fn admits(&self, fact: &RiskFact) -> bool {
        self.principal.admits(self.hierarchy, &fact.key)
    }

    /// Prune a fact slice to the principal's entitled subset, preserving order.
    /// The building block under [`Self::entitled_cube`]; use it directly when the
    /// caller owns its fact store and only needs the admitted leaves.
    #[must_use]
    pub fn prune(&self, facts: &[RiskFact]) -> Vec<RiskFact> {
        facts.iter().copied().filter(|f| self.admits(f)).collect()
    }

    /// Build a [`Cube`] over **only** the admitted facts, wired to the same
    /// `hierarchy` (cloned in, so the entitled cube rolls up with identical parent
    /// pointers). This is the leak-free aggregation entry point: every `group_by`
    /// / `firm_aggregate` / VaR / curvature run over the returned cube reduces only
    /// facts the principal may see, at every node and drill level (§2.6/§4).
    ///
    /// Facts are inserted with the cube's `upsert` supersede semantics (one live
    /// fact per `position_id`); pass at most one current fact per position, as the
    /// fact store does.
    #[must_use]
    pub fn entitled_cube<I>(&self, facts: I) -> Cube
    where
        I: IntoIterator<Item = RiskFact>,
    {
        let mut cube = Cube::with_hierarchy(self.hierarchy.clone());
        for fact in facts {
            if self.admits(&fact) {
                cube.upsert(fact);
            }
        }
        cube
    }
}
