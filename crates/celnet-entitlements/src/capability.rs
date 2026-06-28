//! Action **capabilities**: the "who may *act*" layer, orthogonal to the
//! "who may *see* which risk" predicate ([`crate::Principal`]).
//!
//! # What this is
//!
//! [`crate::Principal`] prunes the risk **read** path (a Chinese-wall predicate
//! over the cube's fact stream). A [`CapabilitySet`] answers the prior, different
//! question a trading desk asks of an authenticated user:
//!
//! > *May this user **price / quote / respond-to-RFQ / respond-to-IOI / stream /
//! > execute (deal) / book** on **FX options** vs **fixed income**?*
//!
//! A [`Capability`] is one [`Action`] on one [`AssetClass`]. A [`CapabilitySet`]
//! is the effective set a caller holds, with the **same deny-by-default,
//! deny-wins** algebra the entitlement filter uses:
//!
//! ```text
//! allowed  ⇔  (grant-all  ∨  some grant covers it)  ∧  no deny covers it
//! ```
//!
//! so the default ([`CapabilitySet::empty`]) admits **nothing** until granted, and
//! an explicit deny overrides any grant (the information-barrier rule, mirrored
//! from [`crate::Principal`]). The desk **scope** a capability is exercised under
//! is applied separately at the server's resource edge (the existing desk-owner
//! filter), so this type stays a pure value with no scope or IO concern.
//!
//! # Determinism & purity
//!
//! Every method is pure and allocation-disciplined; the grant/deny sets are
//! `BTreeSet`s so iteration order is deterministic. No float math, no clock, no
//! logger — like the rest of the crate, this defines what the server enforces and
//! audits; it performs no IO itself.
//!
//! # Provenance
//!
//! The deny-by-default + deny-wins algebra follows `docs/RISK-HIERARCHY.md`
//! §2.6/§4 (mirrored from the read-side predicate); the action/asset-class
//! decomposition follows `docs/plan/PERMISSIONS-ADMINISTRATION-REQUIREMENT.md` §3.
//! Provenance is in doc comments only; no vendor/person/method name appears in any
//! identifier (guardrail #8).

use std::collections::BTreeSet;

/// One **action** a caller may be entitled to perform. Ordered by a stable
/// discriminant so a [`Capability`] is `Ord` (it keys a `BTreeSet`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Action {
    /// See blotters / positions / curves for the asset class (the read floor).
    View,
    /// Price a hypothetical — no market commitment.
    Price,
    /// Publish a tradeable quote on a panel / RFS line.
    QuoteRespond,
    /// Respond to a counterparty **request for quote**.
    RfqRespond,
    /// Respond to / action an **indication of interest**.
    IoiRespond,
    /// Open an RFS price **stream** (subscribe).
    Stream,
    /// **Deal** — click-to-trade / accept a quote (the booking-committing action).
    Execute,
    /// Book a resulting position to a desk book.
    Book,
    /// User / desk / connection / permission **administration**.
    Administer,
}

impl Action {
    /// Every action, in discriminant order — the canonical iteration set for
    /// building bundles and exhaustiveness tests.
    pub const ALL: [Action; 9] = [
        Action::View,
        Action::Price,
        Action::QuoteRespond,
        Action::RfqRespond,
        Action::IoiRespond,
        Action::Stream,
        Action::Execute,
        Action::Book,
        Action::Administer,
    ];

    /// Stable snake_case label for audit/log/wire fields.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Action::View => "view",
            Action::Price => "price",
            Action::QuoteRespond => "quote_respond",
            Action::RfqRespond => "rfq_respond",
            Action::IoiRespond => "ioi_respond",
            Action::Stream => "stream",
            Action::Execute => "execute",
            Action::Book => "book",
            Action::Administer => "administer",
        }
    }

    /// Parse an action from its [`label`](Action::label) — the exact inverse, so a
    /// persisted / wire / audit string round-trips. Returns `None` for any unknown
    /// token (the boundary fails loud rather than silently dropping authority).
    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.label() == label)
    }
}

/// The **asset class** a capability applies to. Rates live under
/// [`AssetClass::FixedIncome`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssetClass {
    /// FX options (the vanilla/exotic options franchise).
    FxOptions,
    /// Fixed income — rates / OIS / FRA / STIR.
    FixedIncome,
}

impl AssetClass {
    /// Both asset classes, in discriminant order.
    pub const ALL: [AssetClass; 2] = [AssetClass::FxOptions, AssetClass::FixedIncome];

    /// Stable snake_case label for audit/log/wire fields.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            AssetClass::FxOptions => "fx_options",
            AssetClass::FixedIncome => "fixed_income",
        }
    }

    /// Parse an asset class from its [`label`](AssetClass::label) — the exact
    /// inverse, so a persisted / wire / audit string round-trips. Returns `None`
    /// for any unknown token.
    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.label() == label)
    }
}

/// One capability: an [`Action`] on an [`AssetClass`]. The atom a
/// [`CapabilitySet`] grants, denies and is queried for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Capability {
    /// The action permitted.
    pub action: Action,
    /// The asset class it applies to.
    pub asset: AssetClass,
}

impl Capability {
    /// Construct a capability.
    #[must_use]
    pub const fn new(action: Action, asset: AssetClass) -> Self {
        Self { action, asset }
    }
}

/// The effective set of capabilities a caller holds, with deny-by-default,
/// deny-wins semantics (module docs). Built immutably via [`CapabilitySet::empty`]
/// / [`CapabilitySet::grant_all`] + [`CapabilitySet::grant`] / [`CapabilitySet::deny`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilitySet {
    /// Firm-wide "may do anything" — the admin posture. Still subject to `denies`.
    grant_all: bool,
    /// Explicitly granted capabilities (ignored when `grant_all`, except that
    /// `grant_all` already covers them).
    grants: BTreeSet<Capability>,
    /// Explicitly denied capabilities — **deny wins** over any grant or grant-all.
    denies: BTreeSet<Capability>,
}

impl CapabilitySet {
    /// The deny-by-default set: admits **nothing** until a grant is added. This is
    /// also [`Default`].
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The firm-wide administrator set: admits every capability not explicitly
    /// denied. The only way to "see everything" — never an implicit fallback.
    #[must_use]
    pub fn grant_all() -> Self {
        Self {
            grant_all: true,
            grants: BTreeSet::new(),
            denies: BTreeSet::new(),
        }
    }

    /// Add a grant, returning the updated set (immutable builder).
    #[must_use]
    pub fn grant(mut self, cap: Capability) -> Self {
        self.grants.insert(cap);
        self
    }

    /// Add an explicit deny (wins over any grant), returning the updated set.
    #[must_use]
    pub fn deny(mut self, cap: Capability) -> Self {
        self.denies.insert(cap);
        self
    }

    /// Grant every [`Action`] in `actions` on `asset` — the bundle helper roles
    /// build their defaults from.
    #[must_use]
    pub fn grant_actions(mut self, actions: &[Action], asset: AssetClass) -> Self {
        for &action in actions {
            self.grants.insert(Capability::new(action, asset));
        }
        self
    }

    /// **The decision predicate**: whether this set admits `cap`, under
    /// deny-by-default, deny-wins (module docs).
    #[must_use]
    pub fn allows(&self, cap: Capability) -> bool {
        if self.denies.contains(&cap) {
            return false;
        }
        self.grant_all || self.grants.contains(&cap)
    }

    /// Whether this is the firm-wide grant-all administrator set.
    #[must_use]
    pub fn is_grant_all(&self) -> bool {
        self.grant_all
    }

    /// Iterate the explicitly-granted capabilities (deterministic order). Used by
    /// the client-capability projection that tells a GUI which affordances to
    /// enable; never the authorization decision itself (that is [`Self::allows`]).
    pub fn granted(&self) -> impl Iterator<Item = Capability> + '_ {
        self.grants.iter().copied()
    }

    /// Iterate the explicitly-denied capabilities (deterministic order).
    pub fn denied(&self) -> impl Iterator<Item = Capability> + '_ {
        self.denies.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FX_EXEC: Capability = Capability::new(Action::Execute, AssetClass::FxOptions);
    const FI_EXEC: Capability = Capability::new(Action::Execute, AssetClass::FixedIncome);
    const FX_PRICE: Capability = Capability::new(Action::Price, AssetClass::FxOptions);

    /// The default/empty set admits nothing (deny-by-default).
    #[test]
    fn empty_admits_nothing() {
        let set = CapabilitySet::empty();
        assert!(!set.is_grant_all());
        for action in Action::ALL {
            for asset in AssetClass::ALL {
                assert!(!set.allows(Capability::new(action, asset)));
            }
        }
        assert_eq!(CapabilitySet::default(), CapabilitySet::empty());
    }

    /// Grant-all admits every capability — and is the only way to.
    #[test]
    fn grant_all_admits_everything() {
        let set = CapabilitySet::grant_all();
        assert!(set.is_grant_all());
        for action in Action::ALL {
            for asset in AssetClass::ALL {
                assert!(set.allows(Capability::new(action, asset)));
            }
        }
    }

    /// A targeted grant admits exactly that capability and nothing else — the
    /// action AND the asset class must both match (no cross-asset leakage).
    #[test]
    fn targeted_grant_is_exact() {
        let set = CapabilitySet::empty().grant(FX_EXEC);
        assert!(set.allows(FX_EXEC));
        assert!(
            !set.allows(FI_EXEC),
            "execute on FX must not grant execute on FI"
        );
        assert!(!set.allows(FX_PRICE), "execute must not grant price");
    }

    /// Separation of duties: price-but-not-execute is representable.
    #[test]
    fn price_without_execute() {
        let set = CapabilitySet::empty().grant(FX_PRICE);
        assert!(set.allows(FX_PRICE));
        assert!(!set.allows(FX_EXEC));
    }

    /// Deny wins over grant-all (an admin walled out of one action/asset).
    #[test]
    fn deny_wins_over_grant_all() {
        let set = CapabilitySet::grant_all().deny(FX_EXEC);
        assert!(!set.allows(FX_EXEC), "explicit deny overrides grant-all");
        assert!(set.allows(FI_EXEC), "other capabilities still admitted");
    }

    /// Deny wins over an explicit grant too.
    #[test]
    fn deny_wins_over_grant() {
        let set = CapabilitySet::empty().grant(FX_EXEC).deny(FX_EXEC);
        assert!(!set.allows(FX_EXEC));
    }

    /// The bundle helper grants a list of actions on one asset class only.
    #[test]
    fn grant_actions_bundles_one_asset() {
        let set = CapabilitySet::empty().grant_actions(
            &[Action::View, Action::Price, Action::Execute],
            AssetClass::FixedIncome,
        );
        assert!(set.allows(Capability::new(Action::View, AssetClass::FixedIncome)));
        assert!(set.allows(FI_EXEC));
        assert!(!set.allows(FX_EXEC), "bundle was FI-only");
        assert!(!set.allows(Capability::new(Action::Book, AssetClass::FixedIncome)));
    }

    /// Every action/asset label round-trips through `from_label`, and an unknown
    /// token is rejected (so a persisted/wire string can never silently widen or
    /// drop authority).
    #[test]
    fn labels_round_trip_through_from_label() {
        for action in Action::ALL {
            assert_eq!(Action::from_label(action.label()), Some(action));
        }
        for asset in AssetClass::ALL {
            assert_eq!(AssetClass::from_label(asset.label()), Some(asset));
        }
        assert_eq!(Action::from_label("teleport"), None);
        assert_eq!(Action::from_label("Book"), None, "labels are case-exact");
        assert_eq!(AssetClass::from_label("equities"), None);
    }

    /// Action/asset labels are stable and distinct (audit fields key on them).
    #[test]
    fn labels_are_distinct() {
        let mut labels: Vec<&str> = Action::ALL.iter().map(|a| a.label()).collect();
        labels.extend(AssetClass::ALL.iter().map(|a| a.label()));
        let mut sorted = labels.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), labels.len(), "all labels must be distinct");
    }
}
