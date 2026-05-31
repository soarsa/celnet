//! The entitlement **principal** (`docs/RISK-HIERARCHY.md` §4,
//! `docs/EXPERIENCE-ARCHITECTURE.md` §3).
//!
//! A [`Principal`] is *who is looking*: a set of **grant** rules (the dimension
//! subtrees the principal may read) and **deny** rules (information barriers —
//! subtrees cut for legal need-to-know, §4). The decision for one fact is:
//!
//! ```text
//! admitted  ⇔  (grant-all  ∨  some grant covers it)  ∧  no deny covers it
//! ```
//!
//! **Deny wins** over grant: an information barrier is a hard cut even inside an
//! otherwise-granted subtree, which is exactly the "Chinese wall" semantics §4
//! requires (a desk head granted their whole desk is still walled out of a named
//! sub-book under investigation).
//!
//! # The default principal is grant-all (zero-rework migration)
//!
//! [`Principal::grant_all`] admits **every** fact and is the crate default. This
//! mirrors the GUI's `ScopeContext { principal: "grant-all" }` (today's show-all
//! firm-wide view, `docs/EXPERIENCE-ARCHITECTURE.md` §3). Every server-side
//! aggregation path flows the fact stream through [`crate::EntitlementFilter`]
//! **now**, while the principal is grant-all and the predicate is the identity, so
//! slotting a real scoped principal in later changes *only which facts the
//! predicate admits* — not a single call site. That is the explicit
//! "entitlement-ready, zero-rework" contract from §3.

use celnet_risk_cube::{FactKey, Hierarchy};

use crate::scope::Rule;

/// An entitlement principal: who is looking, and what subtrees they may read
/// (`docs/RISK-HIERARCHY.md` §4).
///
/// Construct the firm-wide default with [`Principal::grant_all`] (the GUI's
/// `"grant-all"` principal), or a scoped principal with [`Principal::scoped`] +
/// [`Principal::grant`] / [`Principal::deny`].
#[derive(Debug, Clone, PartialEq)]
pub struct Principal {
    /// `true` ⇒ admit everything regardless of grants/denies (the grant-all
    /// default). When `false`, a fact must match at least one grant.
    all: bool,
    /// Read grants — a fact is candidate-visible if **any** grant covers it.
    grants: Vec<Rule>,
    /// Information-barrier deny rules — a fact covered by **any** deny is cut,
    /// even if granted (deny wins).
    denies: Vec<Rule>,
}

/// The default principal is **grant-all** — the show-all-now anchor that mirrors
/// the GUI's `ScopeContext.principal = "grant-all"`. This is deliberately **not**
/// the derived all-`false` default: a fresh `Principal` must see everything so the
/// pre-aggregation filter is wired in as the identity today and swapping in a real
/// scoped principal later is zero-rework (module docs).
impl Default for Principal {
    fn default() -> Self {
        Self::grant_all()
    }
}

impl Principal {
    /// The **grant-all** principal: admits every fact, no pruning. The crate
    /// default and the migration anchor (see module docs) — mirrors the GUI
    /// `ScopeContext.principal = "grant-all"`.
    #[must_use]
    pub fn grant_all() -> Self {
        Self {
            all: true,
            grants: Vec::new(),
            denies: Vec::new(),
        }
    }

    /// A **scoped** principal with no grants yet — admits **nothing** until at
    /// least one [`grant`](Self::grant) is added. (Deny-by-default: an unscoped
    /// real principal must be explicitly granted a subtree before it sees risk,
    /// the §4 separation-of-duties posture.)
    #[must_use]
    pub fn scoped() -> Self {
        Self {
            all: false,
            grants: Vec::new(),
            denies: Vec::new(),
        }
    }

    /// Whether this is the grant-all principal (admits everything).
    #[must_use]
    pub fn is_grant_all(&self) -> bool {
        self.all
    }

    /// Add a read **grant** over a dimension subtree. A scoped principal sees a
    /// fact if any of its grants covers it (and no deny cuts it). No-op semantics
    /// on a grant-all principal would be confusing, so this clears the all-flag:
    /// granting a specific subtree narrows a principal to exactly its grants.
    #[must_use]
    pub fn grant(mut self, rule: Rule) -> Self {
        self.all = false;
        self.grants.push(rule);
        self
    }

    /// Add an information-barrier **deny** over a dimension subtree. Deny wins
    /// over grant and over grant-all: a denied subtree is cut from *any*
    /// principal. This is how a Chinese wall is layered onto an otherwise-broad
    /// principal (e.g. grant-all firm view minus one walled desk).
    #[must_use]
    pub fn deny(mut self, rule: Rule) -> Self {
        self.denies.push(rule);
        self
    }

    /// The principal's grant rules (read-only).
    #[must_use]
    pub fn grants(&self) -> &[Rule] {
        &self.grants
    }

    /// The principal's deny rules (read-only).
    #[must_use]
    pub fn denies(&self) -> &[Rule] {
        &self.denies
    }

    /// **The entitlement decision for one fact's org placement** — the predicate
    /// the server applies *before* aggregation (`docs/RISK-HIERARCHY.md` §2.6/§4).
    ///
    /// Pure and deterministic: admitted iff `(grant-all ∨ some grant covers) ∧ no
    /// deny covers`. `hierarchy` resolves the `Book → Desk` / `Location → Entity`
    /// parent chains so a scope covers exactly the facts that roll into the
    /// matching node — the scope and the cube's group-by share one key space.
    #[must_use]
    pub fn admits(&self, hierarchy: &Hierarchy, key: &FactKey) -> bool {
        // Deny wins, evaluated first (information barriers are hard cuts).
        if self.denies.iter().any(|d| d.covers(hierarchy, key)) {
            return false;
        }
        if self.all {
            return true;
        }
        self.grants.iter().any(|g| g.covers(hierarchy, key))
    }
}
