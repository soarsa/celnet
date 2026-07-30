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
    /// **Transfer** existing risk between books / desks / traders — the manual move
    /// of already-open risk (the complement to routing). A cross-desk booking-class
    /// write **distinct** from [`Action::Book`]: a desk can be granted booking
    /// (`Book`) without being granted the authority to move risk across the desk
    /// boundary (`RiskTransfer`). Gates the initiate/accept transfer RPCs
    /// (`docs/RISK-TRANSFER-REQUIREMENTS.md` §7). Not in the default trader bundle —
    /// a narrow, explicitly-granted authority (like [`Action::Administer`]).
    RiskTransfer,
    /// Run the **client-side counterparty sandbox** — generate mock RFQ/IOI/order
    /// items and sample quotes purely in the GUI. This action gates a UI affordance
    /// only; it has **no** server RPC and never injects into the live priced desk
    /// flow (the simulator is a pure client sandbox). Part of the default trader
    /// bundle (on by default, admin-disable-able).
    Simulate,
    /// User / desk / connection / permission **administration**.
    Administer,
    /// Manage the **risk-control** surfaces: the risk-portfolio tree
    /// (`RiskBookDef` CRUD), the risk-routing decision graph (get/update), and the
    /// firm-wide routed-risk roll-up (the risk dashboard). A firm risk-control
    /// function **distinct** from publishing a quote ([`Action::QuoteRespond`], which
    /// this replaces as the FI risk-manager stand-in) and from super-admin
    /// ([`Action::Administer`]): a desk/risk lead can be granted it **without** full
    /// administration. Not in the default trader bundle — a narrow, explicitly-granted
    /// authority (`docs/PERMISSIONS-GRANULAR-REVIEW.md` §3.1).
    RiskManage,
    /// Manage **FI client-pricing** structure: pricing-group structure/membership
    /// CRUD (which inbound FIX sessions / users / desks a group prices) — hence the
    /// session-pivoted **tiering assignment**, expressed as a group's membership edit.
    /// The per-group **pipeline retune** stays a quoting-trader knob on
    /// [`Action::QuoteRespond`], NOT this. A client-pricing-desk control separable
    /// from identity administration (`docs/PERMISSIONS-GRANULAR-REVIEW.md` §3.1).
    ManagePricing,
    /// Manage **inbound liquidity / venue ops**: FIX connection administration
    /// (create/update/delete/enable) and aggregated-book configuration. Distinct from
    /// identity administration ([`Action::Administer`]) — a venue-ops seat. Exercised
    /// on the asset the venue serves, so an FX-liquidity and an FI-liquidity seat are
    /// separately expressible (`docs/PERMISSIONS-GRANULAR-REVIEW.md` §3.1).
    ManageLiquidity,
    /// View the **cross-product Analytics** surface — the per-client flow / P&L-
    /// attribution ($/mm, spread economics, quote-fishing) rollups over what the desk
    /// priced and traded (`docs/ANALYTICS-REQUIREMENTS.md` §11). A management-sensitive
    /// *read* distinct from the per-trade [`Action::View`] floor: it exposes per-client
    /// margin, adverse-selection and fishing signals a line trader should not see by
    /// default. Because Analytics is its own top-level surface spanning **both** asset
    /// classes, this action is exercised per [`AssetClass`] (a caller holding it on
    /// *any* asset may open the tab and see that asset's slice). Not in the default
    /// trader bundle — a narrow, explicitly-granted authority like the `Manage*` seats.
    ViewAnalytics,
}

impl Action {
    /// Every action, in discriminant order — the canonical iteration set for
    /// building bundles and exhaustiveness tests.
    pub const ALL: [Action; 15] = [
        Action::View,
        Action::Price,
        Action::QuoteRespond,
        Action::RfqRespond,
        Action::IoiRespond,
        Action::Stream,
        Action::Execute,
        Action::Book,
        Action::RiskTransfer,
        Action::Simulate,
        Action::Administer,
        Action::RiskManage,
        Action::ManagePricing,
        Action::ManageLiquidity,
        Action::ViewAnalytics,
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
            Action::RiskTransfer => "risk_transfer",
            Action::Simulate => "simulate",
            Action::Administer => "administer",
            Action::RiskManage => "risk_manage",
            Action::ManagePricing => "manage_pricing",
            Action::ManageLiquidity => "manage_liquidity",
            Action::ViewAnalytics => "view_analytics",
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

    /// Separation of duties: `risk_transfer` is a NARROW authority distinct from
    /// `book` — granting booking never implies the authority to transfer risk across
    /// the desk boundary (and vice-versa), on either asset class.
    #[test]
    fn risk_transfer_is_distinct_from_book() {
        let fi_book = Capability::new(Action::Book, AssetClass::FixedIncome);
        let fi_transfer = Capability::new(Action::RiskTransfer, AssetClass::FixedIncome);
        let fx_transfer = Capability::new(Action::RiskTransfer, AssetClass::FxOptions);

        let booker = CapabilitySet::empty().grant(fi_book);
        assert!(booker.allows(fi_book));
        assert!(
            !booker.allows(fi_transfer),
            "book must not imply cross-desk transfer"
        );

        let transferrer = CapabilitySet::empty().grant(fi_transfer);
        assert!(transferrer.allows(fi_transfer));
        assert!(!transferrer.allows(fi_book), "transfer must not imply book");
        assert!(
            !transferrer.allows(fx_transfer),
            "FI transfer must not grant FX transfer"
        );
        assert_eq!(
            Action::from_label("risk_transfer"),
            Some(Action::RiskTransfer)
        );
    }

    /// The three management authorities (`risk_manage`, `manage_pricing`,
    /// `manage_liquidity`) are each a NARROW, distinct capability: granting one never
    /// implies another, none is implied by any trading action, and each is asset-scoped
    /// (an FI grant never leaks to FX). Deny-by-default keeps an ungranted user closed.
    #[test]
    fn manage_capabilities_are_distinct_and_narrow() {
        let risk = Capability::new(Action::RiskManage, AssetClass::FixedIncome);
        let pricing = Capability::new(Action::ManagePricing, AssetClass::FixedIncome);
        let liquidity = Capability::new(Action::ManageLiquidity, AssetClass::FixedIncome);

        // A risk-manager holds ONLY risk-manage — not pricing, not liquidity, not
        // quote-respond (the overloaded capability this replaces).
        let rm = CapabilitySet::empty().grant(risk);
        assert!(rm.allows(risk));
        assert!(
            !rm.allows(pricing),
            "risk_manage must not imply manage_pricing"
        );
        assert!(
            !rm.allows(liquidity),
            "risk_manage must not imply manage_liquidity"
        );
        assert!(
            !rm.allows(Capability::new(
                Action::QuoteRespond,
                AssetClass::FixedIncome
            )),
            "risk_manage is distinct from quote_respond"
        );
        assert!(
            !rm.allows(Capability::new(Action::RiskManage, AssetClass::FxOptions)),
            "FI risk_manage must not grant FX risk_manage"
        );

        // A pricing-desk seat holds ONLY manage_pricing.
        let pd = CapabilitySet::empty().grant(pricing);
        assert!(pd.allows(pricing));
        assert!(
            !pd.allows(risk),
            "manage_pricing must not imply risk_manage"
        );
        assert!(
            !pd.allows(liquidity),
            "manage_pricing must not imply manage_liquidity"
        );

        // A venue-ops seat holds ONLY manage_liquidity.
        let vo = CapabilitySet::empty().grant(liquidity);
        assert!(vo.allows(liquidity));
        assert!(
            !vo.allows(risk),
            "manage_liquidity must not imply risk_manage"
        );
        assert!(
            !vo.allows(pricing),
            "manage_liquidity must not imply manage_pricing"
        );

        // None is in the default (empty) set, and each label round-trips.
        for action in [
            Action::RiskManage,
            Action::ManagePricing,
            Action::ManageLiquidity,
        ] {
            assert_eq!(Action::from_label(action.label()), Some(action));
            for asset in AssetClass::ALL {
                assert!(!CapabilitySet::empty().allows(Capability::new(action, asset)));
            }
        }
    }

    /// Deny wins over grant-all for a management capability too — an admin can be
    /// walled out of exactly one manage authority.
    #[test]
    fn deny_wins_over_grant_all_for_manage_caps() {
        let rm_fi = Capability::new(Action::RiskManage, AssetClass::FixedIncome);
        let set = CapabilitySet::grant_all().deny(rm_fi);
        assert!(!set.allows(rm_fi), "explicit deny overrides grant-all");
        assert!(
            set.allows(Capability::new(
                Action::ManagePricing,
                AssetClass::FixedIncome
            )),
            "other manage caps still admitted under grant-all"
        );
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
