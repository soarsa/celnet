//! The dimension-subtree scope primitive (`docs/RISK-HIERARCHY.md` §4,
//! `docs/EXPERIENCE-ARCHITECTURE.md` §3).
//!
//! A [`Scope`] names a **subtree** of one cube dimension by pinning that
//! dimension to a single value — e.g. `Desk = 99` is "the EM-vol desk subtree".
//! A [`RiskFact`](celnet_risk_cube::RiskFact) is *covered* by a scope when its
//! org placement, resolved through the [`Hierarchy`], matches the pinned value on
//! that axis. Because Celnet's dimensions are **orthogonal** (a position is
//! simultaneously a desk fact, a pair fact, an entity fact — §2.1), a scope on one
//! axis says nothing about the others: `Desk = 99` admits *any* book/pair/entity
//! under desk 99.
//!
//! A [`Rule`] is a **conjunction** of scopes across axes, so the common
//! entitlement sentence "read risk for desk = EM-vol, any book, booked in London"
//! is one rule with two scopes (`Desk = 99` ∧ `Entity = london`); the unconstrained
//! axes (book, pair) stay wide. A rule with no scopes covers **everything** (the
//! firm root), which is how the grant-all principal is expressed.

use celnet_risk_cube::{DimensionId, FactKey, Hierarchy};

/// A pinned value on one cube dimension — names a subtree of that dimension
/// (`docs/RISK-HIERARCHY.md` §4). The `value` is the dimension's group
/// discriminant, identical to the one the cube's roll-up groups by
/// ([`FactKey::group_value`]), so a scope and a group-by speak the **same key
/// space** — there is no separate scope-key encoding to drift out of sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scope {
    /// The dimension this scope pins.
    pub dimension: DimensionId,
    /// The group value on that dimension (see [`FactKey::group_value`]; for
    /// `Desk`/`Entity` it is the hierarchy-resolved ancestor's handle).
    pub value: u64,
}

impl Scope {
    /// A scope pinning `dimension` to `value`.
    #[must_use]
    pub const fn new(dimension: DimensionId, value: u64) -> Self {
        Self { dimension, value }
    }

    /// Whether a fact's org placement falls under this scope, resolving the
    /// `Book → Desk` / `Location → Entity` parent chains via `hierarchy` exactly
    /// as the cube's roll-up does — so "covered by `Desk = 99`" means precisely
    /// "rolls up into the desk-99 node".
    #[must_use]
    pub fn covers(&self, hierarchy: &Hierarchy, key: &FactKey) -> bool {
        resolved_group_value(hierarchy, key, self.dimension) == self.value
    }
}

/// Resolve a fact's group value along `dim`, applying the same parent-pointer
/// resolution the cube uses (`Desk` ← book's parent, `Entity` ← location's
/// parent). This is the single source of the scope↔roll-up key agreement: a scope
/// covers exactly the facts that roll into the matching node.
pub(crate) fn resolved_group_value(hierarchy: &Hierarchy, key: &FactKey, dim: DimensionId) -> u64 {
    match dim {
        DimensionId::Desk => hierarchy
            .desk_of(key.book)
            .map_or_else(|| key.group_value(dim), |d| u64::from(d.raw())),
        DimensionId::Entity => hierarchy
            .entity_of(key.location)
            .map_or_else(|| key.group_value(dim), |e| u64::from(e.raw())),
        other => key.group_value(other),
    }
}

/// A conjunction of [`Scope`]s — covers a fact iff **every** scope covers it
/// (`docs/RISK-HIERARCHY.md` §4). An **empty** rule covers everything (the firm
/// root): it is the rule a grant-all principal would carry, though that principal
/// short-circuits before evaluating rules at all (see [`crate::Principal`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rule {
    scopes: Vec<Scope>,
}

impl Rule {
    /// A rule covering everything (no scope constraints — the firm subtree).
    #[must_use]
    pub fn firm() -> Self {
        Self::default()
    }

    /// A single-axis rule: "this dimension pinned to this value, every other axis
    /// wide" — e.g. `Rule::on(DimensionId::Desk, 99)` = "desk 99, any book/pair".
    #[must_use]
    pub fn on(dimension: DimensionId, value: u64) -> Self {
        Self {
            scopes: vec![Scope::new(dimension, value)],
        }
    }

    /// Add a conjunctive scope, narrowing the rule to also require this axis —
    /// e.g. `Rule::on(Desk, 99).and(Entity, london)`.
    #[must_use]
    pub fn and(mut self, dimension: DimensionId, value: u64) -> Self {
        self.scopes.push(Scope::new(dimension, value));
        self
    }

    /// The scopes composing this rule (read-only).
    #[must_use]
    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    /// Whether this rule covers a fact: **all** of its scopes must cover it (an
    /// empty rule trivially covers everything).
    #[must_use]
    pub fn covers(&self, hierarchy: &Hierarchy, key: &FactKey) -> bool {
        self.scopes.iter().all(|s| s.covers(hierarchy, key))
    }
}
