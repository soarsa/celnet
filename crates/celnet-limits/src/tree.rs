//! The hierarchical limit tree (`docs/RISK-HIERARCHY.md` §5.2).
//!
//! Limits are set at any **node** of the same organizational hierarchy the risk
//! cube rolls up over, and "cascade board → entity → desk → book → trader". A trade
//! consumes limit at **multiple nodes simultaneously** (the trader, its book, the
//! book's desk, the ccy-pair, and the entity all at once — RH §5), so the tree is
//! addressed by a [`LimitScope`]: a single dimension value at a single level.
//!
//! This module owns the **storage and addressing** of limits; the
//! [`crate::check`] layer evaluates them against a cube node. A scope maps 1:1 onto
//! a `celnet_risk_cube::NodeAggregate::group` value at a given
//! `celnet_risk_cube::DimensionId`, which is how a pre-trade check finds every
//! limit on a position's roll-up path: for each `(dimension, group)` the position
//! belongs to, look up the node's limits and evaluate them against that node's
//! aggregate.

use celnet_risk_cube::{
    BookId, DeskId, DimensionId, EntityId, FactKey, Hierarchy, LocationId, TraderId,
};
use celnet_types::{CcyPair, Underlying};

use crate::limit::LimitSpec;

/// Project a fact's [`Underlying`] onto the [`CcyPair`] the currency-pair limit
/// scope addresses. FX and metal underlyings project to their leg-pair via
/// [`Underlying::as_ccy_pair`] (byte-identical to the pre-generalization
/// `CcyPair` scope, so an FX/metal limit's matched group discriminant is
/// unchanged). A cross-asset arm has no `CcyPair` projection; the currency-pair
/// scope vocabulary is FX/metal-shaped, so such a fact's underlying-axis limit is
/// addressed by its own group value, and the scope's pair is the metal/FX
/// projection where one exists. The limit book is FX/metal today, so this falls
/// back only for a cross-asset fact, deriving a stable self-pair from its
/// numeraire currency so the scope remains a well-formed, deterministic key.
#[must_use]
fn scope_pair_of(underlying: &Underlying) -> CcyPair {
    use celnet_types::Ccy;
    underlying.as_ccy_pair().unwrap_or_else(|| {
        let n = if let Some(e) = underlying.as_equity() {
            e.currency
        } else if let Some(c) = underlying.as_commodity() {
            c.currency
        } else if let Some(p) = underlying.as_digital_asset() {
            Ccy::parse(&p.quote).unwrap_or(Ccy::USD)
        } else {
            Ccy::USD
        };
        CcyPair::new(n, n)
    })
}

/// A single addressable node in the limit hierarchy: one dimension value at one
/// level (`docs/RISK-HIERARCHY.md` §5). The firm apex is [`LimitScope::Firm`].
///
/// A scope is the limit-tree analogue of a cube group: [`LimitScope::group_value`]
/// returns the same `u64` discriminant `celnet_risk_cube` groups by, so a scope's
/// limits are evaluated against exactly the matching `NodeAggregate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LimitScope {
    /// The whole firm (the implicit apex every dimension rolls into).
    Firm,
    /// A single trader.
    Trader(TraderId),
    /// A single book.
    Book(BookId),
    /// A single desk.
    Desk(DeskId),
    /// A single currency pair.
    CcyPair(CcyPair),
    /// A single booking location.
    Location(LocationId),
    /// A single legal entity.
    Entity(EntityId),
}

impl LimitScope {
    /// The cube [`DimensionId`] this scope addresses, or `None` for the firm apex
    /// (which is the whole-cube `firm_aggregate`, not a `group_by` dimension).
    #[must_use]
    pub fn dimension(self) -> Option<DimensionId> {
        match self {
            LimitScope::Firm => None,
            LimitScope::Trader(_) => Some(DimensionId::Trader),
            LimitScope::Book(_) => Some(DimensionId::Book),
            LimitScope::Desk(_) => Some(DimensionId::Desk),
            LimitScope::CcyPair(_) => Some(DimensionId::Underlying),
            LimitScope::Location(_) => Some(DimensionId::Location),
            LimitScope::Entity(_) => Some(DimensionId::Entity),
        }
    }

    /// The `u64` group discriminant this scope matches in the cube (the same packing
    /// as `FactKey::group_value`), or `None` for the firm apex.
    #[must_use]
    pub fn group_value(self) -> Option<u64> {
        match self {
            LimitScope::Firm => None,
            LimitScope::Trader(t) => Some(u64::from(t.raw())),
            LimitScope::Book(b) => Some(u64::from(b.raw())),
            LimitScope::Desk(d) => Some(u64::from(d.raw())),
            LimitScope::Location(l) => Some(u64::from(l.raw())),
            LimitScope::Entity(e) => Some(u64::from(e.raw())),
            LimitScope::CcyPair(p) => {
                // The currency-pair scope addresses the cube's underlying axis; an FX
                // pair packs `as_ccy_pair()` byte-identically to the pre-generalization
                // `CcyPair` group value, so the discriminant a CcyPair limit matches is
                // unchanged.
                let key = FactKey {
                    trader: TraderId(0),
                    book: BookId(0),
                    desk: DeskId(0),
                    underlying: Underlying::Fx(p),
                    location: LocationId(0),
                    entity: EntityId(0),
                };
                Some(key.group_value(DimensionId::Underlying))
            }
        }
    }
}

/// The set of limit scopes a single position is simultaneously constrained at — its
/// roll-up path through the limit tree (`docs/RISK-HIERARCHY.md` §5). A pre-trade
/// check evaluates every scope on this path.
///
/// Built from a position's [`FactKey`] resolved through the cube [`Hierarchy`] (so
/// the genuine `book→desk` / `location→entity` parent pointers are honoured — a
/// position's desk/entity is its book's/location's configured parent, not a raw key
/// guess), with the firm apex always last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopePath {
    scopes: Vec<LimitScope>,
}

impl ScopePath {
    /// Resolve the scope path for a position placed at `key`, using `hierarchy` to
    /// follow the `book→desk` and `location→entity` parent pointers (falling back
    /// to the key's own desk/entity when unconfigured, matching the cube's
    /// `group_value` resolution). The path is trader → book → desk → ccy-pair →
    /// location → entity → firm.
    #[must_use]
    pub fn resolve(key: &FactKey, hierarchy: &Hierarchy) -> Self {
        let desk = hierarchy.desk_of(key.book).unwrap_or(key.desk);
        let entity = hierarchy.entity_of(key.location).unwrap_or(key.entity);
        Self {
            scopes: vec![
                LimitScope::Trader(key.trader),
                LimitScope::Book(key.book),
                LimitScope::Desk(desk),
                LimitScope::CcyPair(scope_pair_of(&key.underlying)),
                LimitScope::Location(key.location),
                LimitScope::Entity(entity),
                LimitScope::Firm,
            ],
        }
    }

    /// The scopes on this path, finest (trader) to apex (firm).
    pub fn scopes(&self) -> impl Iterator<Item = LimitScope> + '_ {
        self.scopes.iter().copied()
    }
}

/// The firm's limit tree: the limits configured at every scope
/// (`docs/RISK-HIERARCHY.md` §5.2).
///
/// Multiple limits may sit at one scope (e.g. a desk has a delta limit, a vega
/// limit, and a VaR limit), so each scope maps to a `Vec<LimitSpec>`. Backed by a
/// small association vector keyed by scope — the configured limit set is bounded
/// (a firm has O(books × metric-types) limits), and the structure stays `Clone` and
/// allocation-light. Pure: the tree holds no exposures, only the constraints.
#[derive(Debug, Clone, Default)]
pub struct LimitTree {
    nodes: Vec<(LimitScope, Vec<LimitSpec>)>,
}

impl LimitTree {
    /// An empty limit tree.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a limit at a scope. Multiple limits may coexist at one scope; a limit on
    /// the **same metric** at the same scope is **replaced** (one current limit per
    /// `(scope, metric)`, so re-setting a desk's delta cap supersedes the old one
    /// rather than stacking two delta limits).
    pub fn set(&mut self, scope: LimitScope, limit: LimitSpec) {
        let slot = match self.nodes.iter_mut().find(|(s, _)| *s == scope) {
            Some((_, v)) => v,
            None => {
                self.nodes.push((scope, Vec::new()));
                &mut self.nodes.last_mut().expect("just pushed").1
            }
        };
        if let Some(existing) = slot.iter_mut().find(|l| l.metric == limit.metric) {
            *existing = limit;
        } else {
            slot.push(limit);
        }
    }

    /// The limits configured at a scope (empty if none).
    #[must_use]
    pub fn at(&self, scope: LimitScope) -> &[LimitSpec] {
        self.nodes
            .iter()
            .find(|(s, _)| *s == scope)
            .map_or(&[], |(_, v)| v.as_slice())
    }

    /// Whether any limit is configured at a scope.
    #[must_use]
    pub fn has(&self, scope: LimitScope) -> bool {
        !self.at(scope).is_empty()
    }

    /// The number of scopes with at least one limit configured.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the tree has no limits configured anywhere.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Every configured `(scope, limit)` pair, in insertion order — for a firm-wide
    /// utilization sweep (post-trade monitoring) or to mirror the tree to the GUI.
    pub fn iter(&self) -> impl Iterator<Item = (LimitScope, &LimitSpec)> + '_ {
        self.nodes
            .iter()
            .flat_map(|(s, v)| v.iter().map(move |l| (*s, l)))
    }
}
