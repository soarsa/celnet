//! Persisted **operator identity**: the users who may drive the edge and the
//! **desks** that group them.
//!
//! This module owns only the *data + persistence + password cryptography* — the
//! live login/session lifecycle is [`crate::services::sessions`]'s job, and the
//! management API is `AuthService`. The document persists as a small JSON file so
//! accounts survive restarts and auto-load on boot (mirroring the durability
//! discipline of [`super::fix_connections`]).
//!
//! # Password storage
//!
//! Passwords are **never** stored or logged in the clear. Each user carries an
//! Argon2id PHC hash string ([`hash_password`]); verification is constant-time
//! through the `argon2` verifier ([`verify_password`]). Argon2id is the memory-
//! hard, side-channel-resistant default recommended for password storage. Salts
//! are 16 random bytes from the OS CSPRNG (`getrandom`), encoded into the PHC
//! string, so two users with the same password never share a hash.
//!
//! # Roles & desks
//!
//! A [`Role`] is either [`Role::Admin`] (may administer users, desks and
//! connections) or [`Role::Trader`] (a desk member who sees their desk's inbound
//! RFQ traffic). A user belongs to zero, one, or many [`DeskDef`]s by `desk_ids`
//! (or to every desk via `all_desks`); desk membership is what scopes RFQ/monitor
//! visibility (built on top of this store).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use celnet_acceptance::{AcceptanceGraph, default_accept_all_graph};
use celnet_entitlements::{Action, AssetClass, Capability};
use celnet_hedge_routing::HedgeGraph;
use celnet_risk_routing::{RiskRoutingGraph, RoutingNode};
use serde::{Deserialize, Serialize};

use super::hedge_policy::{
    HedgeConfigDef, HedgeMetric, HedgeScopeKind, HedgeThresholdDef, ScopedThreshold,
};

use super::reference_data::{
    self, ExternalScheme, InstrumentDef, ensure_seed_instruments, government_bond_defs,
    validate_instruments,
};

/// Env var naming the identity JSON file. Absent ⇒ [`DEFAULT_CONFIG_PATH`].
pub const CONFIG_ENV: &str = "CELNET_IDENTITY_CONFIG";
/// Default identity path (repo-/cwd-local) when the env is unset.
pub const DEFAULT_CONFIG_PATH: &str = "identity.json";

/// The email of the default administrator seeded on first run.
pub const SEED_ADMIN_EMAIL: &str = "admin@celnet.com";
/// The password of the default administrator seeded on first run. The operator
/// is expected to change it immediately via `AuthService.ResetPassword`.
pub const SEED_ADMIN_PASSWORD: &str = "password";

/// The stable id of the default **"Firm Warehouse"** risk book seeded on a pristine
/// store (`docs/FI-RISK-ROUTING-REQUIREMENTS.md` §3.1/§8.2). Its presence — plus the
/// default single-leaf routing graph that targets it — is what makes an accepted /
/// lifted fill land in an ENABLED risk book (and show a row on the per-book risk
/// dashboard) out of the box, before an operator has defined any finer books.
pub const DEFAULT_WAREHOUSE_BOOK_ID: &str = "warehouse";
/// The display name of the default warehouse book seeded on a pristine store.
pub const DEFAULT_WAREHOUSE_BOOK_NAME: &str = "Firm Warehouse";
/// The default warehouse **DV01 budget** (the "100") seeded for the default warehouse book on
/// a pristine store, so the internalise decision has a cap to measure fills against out of the
/// box (`docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` §4). A deliberately generous
/// firm-warehouse budget: a normally-sized single fill sits comfortably under it (⇒ fully
/// internalised when the desk captured edge), so the deal blotter shows a real internalise
/// decision from first boot. An operator narrows it via `AuthService.UpdateHedgeThreshold`.
pub const DEFAULT_WAREHOUSE_DV01_CAP: f64 = 1_000_000.0;

/// What a user is allowed to do on the edge.
///
/// `Ord`/`PartialOrd` are derived so a [`Role`] can key the persisted
/// [`IdentityStore::role_bundles`] map deterministically (a `BTreeMap` orders its
/// keys, so `identity.json` round-trips byte-identically).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Full administration: user/desk CRUD, password resets, FIX connections.
    Admin,
    /// A desk member: sees the inbound RFQ/monitor traffic for their desk.
    Trader,
}

impl Role {
    /// A stable lowercase wire/display token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Trader => "trader",
        }
    }

    /// Parse the wire/display token (case-insensitive).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "admin" => Some(Role::Admin),
            "trader" => Some(Role::Trader),
            _ => None,
        }
    }

    /// Whether this role carries administrative authority.
    #[must_use]
    pub fn is_admin(self) -> bool {
        matches!(self, Role::Admin)
    }
}

/// The default capability **bundle** of the [`Role::Trader`] role (and any future
/// non-admin role): every action on **both** asset classes **except** the five narrow,
/// explicitly-granted authorities held back from the default —
/// [`Action::Administer`] (super-admin), [`Action::RiskTransfer`] (cross-desk risk
/// move), and the three per-feature management authorities [`Action::RiskManage`],
/// [`Action::ManagePricing`] and [`Action::ManageLiquidity`]
/// (`docs/PERMISSIONS-GRANULAR-REVIEW.md` §5). A plain trader therefore keeps every
/// trading capability but does **not** get the Risk Routing / Risk Portfolios / Risk
/// Dashboard, Pricing-Group / Tiering, or FIX-connection / Aggregated-Book management
/// surfaces until an admin grants the matching capability (per-user overlay or an
/// editable role bundle). This is the slice-1 hardcoded base that the admin-editable
/// [`IdentityStore::role_bundles`] overlay persists and can narrow/widen
/// (`docs/plan/PERMISSIONS-ADMINISTRATION-REQUIREMENT.md` §3.3/§10). A store with no
/// persisted bundle for the role resolves to exactly this set, so an existing
/// `identity.json` (which carries no `role_bundles`) behaves identically to before.
#[must_use]
pub fn default_trader_bundle() -> Vec<Capability> {
    let mut caps = Vec::new();
    for action in Action::ALL {
        if matches!(
            action,
            Action::Administer
                | Action::RiskTransfer
                | Action::RiskManage
                | Action::ManagePricing
                | Action::ManageLiquidity
                | Action::ViewAnalytics
                | Action::Hedge
                | Action::Refdata
                | Action::ManageAcceptance
        ) {
            continue;
        }
        for asset in AssetClass::ALL {
            caps.push(Capability::new(action, asset));
        }
    }
    caps
}

/// One persisted user account.
///
/// Desk membership is **many-to-many**: a user belongs to [`all_desks`] (every desk,
/// present and future) or to the set named in [`desk_ids`] (zero, one, or many). A
/// notification routed to desk `D` reaches every user whose membership is `all_desks`
/// or whose [`desk_ids`] contains `D`; an empty set with `all_desks == false` is a
/// **deskless** user (receives nothing desk-routed). Deserialization migrates the
/// legacy single `desk_id` string into a one-element [`desk_ids`] set (see the manual
/// [`Deserialize`] impl below), so an `identity.json` written before this contract
/// loads without losing membership.
///
/// [`all_desks`]: UserDef::all_desks
/// [`desk_ids`]: UserDef::desk_ids
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserDef {
    /// Stable identifier (the store/API key). Never reused; minted from the email.
    pub id: String,
    /// Login email — unique across the store (case-insensitive), the credential.
    pub email: String,
    /// Human-friendly display name shown in the UI.
    pub display_name: String,
    /// What the user may do.
    pub role: Role,
    /// The desks this user belongs to, by [`DeskDef::id`] (order-preserving, unique).
    /// Empty + `!all_desks` ⇒ deskless. Ignored (kept empty, the canonical form) when
    /// [`all_desks`](Self::all_desks) is set.
    #[serde(default)]
    pub desk_ids: Vec<String>,
    /// When set, the user belongs to **every** desk (present and future); the
    /// [`desk_ids`](Self::desk_ids) set is then ignored and canonically empty. An admin
    /// role is always all-desks regardless of this flag; a trader expresses firm-wide
    /// desk visibility through it.
    #[serde(default)]
    pub all_desks: bool,
    /// The Argon2id PHC hash of the user's password. Never the plaintext.
    pub password_hash: String,
    /// Whether the account is disabled (cannot log in) without being deleted.
    #[serde(default)]
    pub disabled: bool,
    /// Per-user capability **grants** layered on top of the user's role bundle
    /// (admin-editable overlay; `docs/plan/PERMISSIONS-ADMINISTRATION-REQUIREMENT.md`
    /// §3.3). Empty ⇒ a pure role-derived user. Stored as the kernel's stable
    /// snake_case labels so the file stays human-editable and round-trips exactly.
    #[serde(default)]
    pub capability_grants: Vec<PermissionGrant>,
    /// Per-user capability **denials**; deny-wins over both the role bundle and any
    /// grant (the information-barrier rule, mirrored from the read-side predicate).
    #[serde(default)]
    pub capability_denies: Vec<PermissionGrant>,
}

impl UserDef {
    /// Verify a candidate plaintext password against this user's stored hash in
    /// constant time. A disabled account always rejects.
    #[must_use]
    pub fn verify(&self, candidate: &str) -> bool {
        !self.disabled && verify_password(&self.password_hash, candidate)
    }

    /// The typed capability overlay `(grants, denies)`, parsed from the persisted
    /// labels. Returns an error on **any** unknown label so a corrupt overlay is
    /// rejected loudly (at load and at admin write) rather than silently dropping a
    /// capability when a session is built.
    ///
    /// # Errors
    /// A label that is not a known [`Action`]/[`AssetClass`].
    pub fn capability_overlay(&self) -> Result<(Vec<Capability>, Vec<Capability>), String> {
        let grants = self
            .capability_grants
            .iter()
            .map(PermissionGrant::parse)
            .collect::<Result<Vec<_>, _>>()?;
        let denies = self
            .capability_denies
            .iter()
            .map(PermissionGrant::parse)
            .collect::<Result<Vec<_>, _>>()?;
        Ok((grants, denies))
    }
}

/// Deserialize a [`UserDef`], migrating the legacy single-desk membership.
///
/// A `UserDef` written before the many-to-many contract carried a single
/// `optional string desk_id`; a current one carries `desk_ids` + `all_desks`. This
/// manual impl accepts **both**: a present legacy `desk_id` folds into the
/// `desk_ids` set (deduped, order-preserving) so an old `identity.json` loads with
/// its membership intact and nothing is lost. New writes emit only `desk_ids` /
/// `all_desks` (the derived [`Serialize`]), so the legacy key never round-trips back.
impl<'de> Deserialize<'de> for UserDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            id: String,
            email: String,
            display_name: String,
            role: Role,
            /// Legacy single-desk membership (pre-many-to-many); migrated below.
            #[serde(default)]
            desk_id: Option<String>,
            #[serde(default)]
            desk_ids: Vec<String>,
            #[serde(default)]
            all_desks: bool,
            password_hash: String,
            #[serde(default)]
            disabled: bool,
            #[serde(default)]
            capability_grants: Vec<PermissionGrant>,
            #[serde(default)]
            capability_denies: Vec<PermissionGrant>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let mut desk_ids = raw.desk_ids;
        if let Some(legacy) = raw.desk_id
            && !legacy.trim().is_empty()
            && !desk_ids.iter().any(|d| d == &legacy)
        {
            desk_ids.push(legacy);
        }
        Ok(UserDef {
            id: raw.id,
            email: raw.email,
            display_name: raw.display_name,
            role: raw.role,
            desk_ids,
            all_desks: raw.all_desks,
            password_hash: raw.password_hash,
            disabled: raw.disabled,
            capability_grants: raw.capability_grants,
            capability_denies: raw.capability_denies,
        })
    }
}

/// One persisted capability on a user's overlay — an [`Action`] on an
/// [`AssetClass`], stored as the kernel's stable snake_case labels
/// ([`Action::label`] / [`AssetClass::label`]) so `identity.json` is human-editable
/// and round-trips exactly through [`Action::from_label`] / [`AssetClass::from_label`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionGrant {
    /// The action label, e.g. `"execute"`.
    pub action: String,
    /// The asset-class label, e.g. `"fixed_income"`.
    pub asset: String,
}

impl PermissionGrant {
    /// Render a typed [`Capability`] into its persisted label form.
    #[must_use]
    pub fn of(cap: Capability) -> Self {
        Self {
            action: cap.action.label().to_string(),
            asset: cap.asset.label().to_string(),
        }
    }

    /// Parse this persisted entry into the typed kernel [`Capability`].
    ///
    /// # Errors
    /// An `action` or `asset` that is not a known label.
    pub fn parse(&self) -> Result<Capability, String> {
        let action = Action::from_label(&self.action)
            .ok_or_else(|| format!("unknown capability action {:?}", self.action))?;
        let asset = AssetClass::from_label(&self.asset)
            .ok_or_else(|| format!("unknown capability asset {:?}", self.asset))?;
        Ok(Capability::new(action, asset))
    }
}

/// One persisted desk: a named group traders belong to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeskDef {
    /// Stable identifier (the store/API key). Never reused; minted from the name.
    pub id: String,
    /// Human-friendly desk label, e.g. `G10 Options`.
    pub name: String,
    /// The book **names** this desk owns — the attribution book strings the
    /// interner sees on the live booking path. The desk-identity bridge (item B §3)
    /// interns each at boot and sets its `Book → Desk` parent, so a logged-in
    /// trader's session narrows to exactly the facts booked into their desk's books
    /// (`PositionStore::configure_desk`). Empty ⇒ the desk owns no books yet (a
    /// no-op at boot), an additive serde-default field (no `schema_version`).
    #[serde(default)]
    pub books: Vec<String>,
}

/// One persisted **legal entity / account**: a named regulatory-capital unit that
/// maps to the `uint32` `RatesPosition.entity` partition key on the wire. The
/// position wire stays numeric — this registry only names the existing key so the
/// booking form and the Book/blotter views show a name, never a raw number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityDef {
    /// The `uint32` wire partition key (`RatesPosition.entity`) this entry names.
    pub key: u32,
    /// Human-friendly legal-entity / account name, e.g. `ACME Capital` — unique
    /// across the store (case-insensitive).
    pub name: String,
    /// Short display code shown in compact cells, e.g. `ACME` — unique across the
    /// store (case-insensitive).
    pub code: String,
}

/// One persisted **netting book**: a named book that maps to the `uint32`
/// `RatesPosition.book` key on the wire, belonging to exactly one [`EntityDef`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookDef {
    /// The `uint32` wire key (`RatesPosition.book`) this entry names.
    pub key: u32,
    /// Human-friendly book name, e.g. `Rates Trading` — unique across the store
    /// (case-insensitive).
    pub name: String,
    /// The [`EntityDef::key`] of the entity this book belongs to — every book must
    /// resolve to an existing entity (validated at load and at every admin write).
    pub entity_key: u32,
}

/// The **instrument coverage** of an [`AggregatedBookDef`]: which instruments the
/// composite is produced for.
///
/// serde uses the **adjacently-tagged** encoding (a `mode` discriminant plus an
/// `instrument_ids` payload) so `identity.json` stays human-readable and round-trips
/// exactly: `{"mode":"all_members_quote"}` or
/// `{"mode":"explicit","instrument_ids":["ust-10y", …]}`. (The default externally-
/// tagged form — `{"explicit":[…]}` — is terser but less self-describing in a
/// hand-edited operator file, so the tagged form is preferred here.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "instrument_ids", rename_all = "snake_case")]
pub enum Scope {
    /// Aggregate **every** instrument any member quotes — the composite tracks the
    /// union of the members' live streams with no pre-declared instrument list. The
    /// coverage is discovered from the ingest side at runtime (decision E), so an
    /// operator need not enumerate instruments to stand a book up.
    AllMembersQuote,
    /// Aggregate only the explicitly listed instruments, by canonical server
    /// [`InstrumentDef::instrument_id`](reference_data::InstrumentDef::instrument_id)
    /// (decision D — one instrument identity across reference data, GUI, and the
    /// composite). Every id must resolve to an existing instrument in the store's
    /// reference-data registry (validated at load and at every admin write).
    Explicit(Vec<String>),
}

/// The consolidation-engine tuning of an [`AggregatedBookDef`] — the knobs the
/// `celnet-aggregation` engine consumes once a book is wired (P2). Persisted with the
/// book so a composite reproduces byte-for-byte across restarts; each field is an
/// operator-facing control surfaced on the Aggregation admin form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregationParams {
    /// Staleness half-life **τ** in milliseconds: a member quote's contribution
    /// decays as `2^{-Δt/τ}` with age `Δt`, so a lagging feed fades smoothly rather
    /// than dropping in a step (the engine's exponential staleness weight). Must be
    /// `> 0` (a zero half-life is undefined — validated at load and at every write).
    pub staleness_tau_ms: u64,
    /// Hard maximum quote age in milliseconds: a member quote older than this is
    /// **fully excluded** from the composite (the engine's hard staleness cutoff),
    /// independent of the soft `τ` decay above.
    pub max_quote_age_ms: u64,
    /// Whether MAD-based **divergence gating** is enabled: an outlier member whose
    /// mid diverges beyond the median-absolute-deviation band is dropped before the
    /// composite is formed, so one mispriced feed cannot skew the book.
    pub divergence_gating: bool,
    /// Minimum number of surviving member contributors required to publish a
    /// composite — below this the book produces no price (avoids a "composite" that
    /// is really a single feed). Must be `>= 1` (validated at load and at every write).
    pub min_contributors: u32,
    /// How many stacked depth levels the composite exposes: `1` = top-of-book only,
    /// `n` = the best `n` price levels of consolidated depth.
    pub depth_levels: u32,
}

impl Default for AggregationParams {
    /// Sane consolidation defaults for a freshly created book: a 500 ms staleness
    /// half-life, a 2500 ms hard age cutoff, divergence gating on, a single required
    /// contributor, and top-of-book depth. An operator narrows/widens these on the
    /// Aggregation admin form.
    fn default() -> Self {
        Self {
            staleness_tau_ms: 500,
            max_quote_age_ms: 2500,
            divergence_gating: true,
            min_contributors: 1,
            depth_levels: 1,
        }
    }
}

/// One persisted **aggregated book**: an operator-defined, named set of inbound
/// liquidity members whose per-instrument top-of-book is consolidated into one
/// composite price (best bid/offer + size + depth), managed from the Administration
/// surface (ADR-0022).
///
/// Unlike [`BookDef`] (a numeric netting/accounting partition) and [`DeskDef`] (a
/// trader grouping), an aggregated book is **global and not desk-owned** (decision
/// C): any authenticated user may view its composite and — subject to their own FI
/// action capabilities — quote/book off it; only an admin may define or edit one. It
/// therefore carries no `desk_id`/`entity_key` ownership key.
///
/// Members are recorded as **transport-agnostic connection ids** (decision A), not a
/// FIX-specific type: a member is any inbound liquidity connection (FIX RFS today, any
/// other API adapter tomorrow) behind the `celnet-aggregation::VenueFeed` seam.
///
/// Derives `PartialEq` (not `Eq`) to stay uniform with [`IdentityStore`], whose
/// pricing-group pipelines carry `f64` feature magnitudes — an aggregated book is
/// compared structurally, never used as a hash/ordering key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregatedBookDef {
    /// Stable identifier (the store/API key). Never reused; minted from the name via
    /// [`mint_aggregated_book_id`].
    pub id: String,
    /// Human-friendly book label, e.g. `G10 Rates Composite` — unique across the store
    /// (case-insensitive), validated at load and at every admin write.
    pub name: String,
    /// The inbound liquidity members whose quotes feed the composite, by **connection
    /// id** (decision A — transport-agnostic; a FIX RFS connection today, any other API
    /// adapter tomorrow). Order-preserving and unique **within this book** (duplicate
    /// member ids are rejected). These ids reference the **separate** fix/connection
    /// registry (`super::fix_connections`), which is **not** part of [`IdentityStore`],
    /// so cross-store resolution — confirming each id names a live connection — is a
    /// service-layer concern (task T1.3) and is deliberately **not** checked here.
    pub member_connection_ids: Vec<String>,
    /// Which instruments the composite is produced for — every member's instruments
    /// ([`Scope::AllMembersQuote`]) or an explicit reference-data id list
    /// ([`Scope::Explicit`], each id resolving to an [`InstrumentDef`]).
    pub instrument_scope: Scope,
    /// The consolidation-engine tuning (staleness, gating, depth, quorum) applied when
    /// the book is wired to the `celnet-aggregation` engine (P2).
    pub params: AggregationParams,
    /// Whether the book is active. A disabled book is persisted and editable but stands
    /// up no engine and publishes no composite — the operator's on/off switch.
    pub enabled: bool,
}

/// The **editable** fields of an aggregated book (everything but the server-minted
/// `id`) — the single payload the store's create/update CRUD take, so the config layer
/// and the admin RPCs pass one value rather than a long positional argument list. The
/// service layer builds one from an [`AggregatedBookSpec`](celnet_proto::AggregatedBookSpec).
#[derive(Debug, Clone, PartialEq)]
pub struct AggregatedBookEdit {
    /// Human-friendly book label (unique across the store, case-insensitive).
    pub name: String,
    /// The inbound liquidity members feeding the composite, by connection id.
    pub member_connection_ids: Vec<String>,
    /// Which instruments the composite is produced for.
    pub instrument_scope: Scope,
    /// The consolidation-engine tuning.
    pub params: AggregationParams,
    /// Whether the book is active.
    pub enabled: bool,
}

impl AggregatedBookEdit {
    /// A minimal edit — the single payload the store's aggregated-book create/update
    /// CRUD take.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        member_connection_ids: Vec<String>,
        instrument_scope: Scope,
        params: AggregationParams,
        enabled: bool,
    ) -> Self {
        Self {
            name: name.into(),
            member_connection_ids,
            instrument_scope,
            params,
            enabled,
        }
    }
}

/// How a rates/bond FIX auto-quote **sources its price** for a client in this pricing
/// group — the admin-configurable pricing-source policy. Orthogonal to the feature
/// pipelines (which shape a price *after* it is sourced): this decides whether the price
/// comes from the internal curve, the aggregated-book composite, or a blend of the two,
/// applied on the **initial** quote as well as on every RFS re-price.
///
/// The default (proto3 zero / serde default / an unconfigured session with no group) is
/// [`CompositeFirstCurveFallback`](Self::CompositeFirstCurveFallback). That is the
/// **least-surprising** default: it preserves and generalizes the existing RFS
/// composite-first re-price (a `CurveOnly` default would *regress* a book-fed stream back
/// to the curve), removes the historical quote-#1-curve vs quote-#2-composite
/// discontinuity, and — because no aggregated book covers a plain OIS curve — leaves the
/// OIS arm curve-priced exactly as before. Only instruments a book actually covers (cash
/// bonds) move to the book-driven price.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PricingSourceMode {
    /// Composite where an enabled aggregated book covers the instrument, else the internal
    /// curve. The platform default (see the type doc).
    #[default]
    CompositeFirstCurveFallback,
    /// Always the internal curve — the safe back-out that reproduces the pre-policy
    /// initial-quote behaviour (never consults the composite).
    CurveOnly,
    /// Split by product: a cash **bond** prices off the composite (book-covered), an
    /// **OIS/swap** off the curve.
    ProductSplit,
    /// Curve backbone with the mid skewed toward the composite where a book exists — see
    /// [`PricingGroupDef::book_skew_weight`] for the exact blend.
    CurveAnchoredBookSkew,
}

impl PricingSourceMode {
    /// A stable lowercase wire/display token (mirrors the serde `snake_case` form).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PricingSourceMode::CompositeFirstCurveFallback => "composite_first_curve_fallback",
            PricingSourceMode::CurveOnly => "curve_only",
            PricingSourceMode::ProductSplit => "product_split",
            PricingSourceMode::CurveAnchoredBookSkew => "curve_anchored_book_skew",
        }
    }
}

/// The default [`PricingGroupDef::book_skew_weight`]: pull the outbound mid **halfway**
/// from the internal curve to the composite under
/// [`PricingSourceMode::CurveAnchoredBookSkew`].
pub const DEFAULT_BOOK_SKEW_WEIGHT: f64 = 0.5;

/// serde default for [`PricingGroupDef::book_skew_weight`] (an additive field: an
/// `identity.json` written before the pricing-source policy loads at the default weight).
fn default_book_skew_weight() -> f64 {
    DEFAULT_BOOK_SKEW_WEIGHT
}

/// How this pricing group treats the **favorable** side of a market-data (ESP) last-look
/// — the admin-configurable last-look policy governing what happens when the published
/// price moved in the **dealer's** favor between the streamed snapshot the client lifted
/// (`q`) and the current published price at order arrival (`c`). The adverse side is a
/// plain reject-beyond-tolerance / honor-within-tolerance regardless of this mode.
///
/// The default is [`Sync`](Self::Sync): the dealer keeps the whole favorable move and the
/// client is filled at exactly the price they requested — the least-surprising "you get
/// what you asked for" behaviour, and the proto3 zero value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LastLookMode {
    /// Fill the client at EXACTLY their requested (lifted) price; the dealer captures
    /// 100% of the favorable slippage. The platform default (see the type doc).
    #[default]
    Sync,
    /// Fill at an IMPROVED price: the requested price adjusted toward the current price by
    /// [`PricingGroupDef::async_giveback_pct`]% of the favorable move (that % passed back
    /// to the client; the dealer keeps the rest).
    Async,
}

impl LastLookMode {
    /// A stable lowercase wire/display token (mirrors the serde `snake_case` form).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            LastLookMode::Sync => "sync",
            LastLookMode::Async => "async",
        }
    }
}

/// The default [`PricingGroupDef::last_look_tolerance_bps`]: the market-data last-look
/// adverse-move band, **1 bp of price** (relative, so it scales across bond price points
/// and OIS rates). Comfortably absorbs the sub-second composite drift a fast bond shows
/// between the snapshot the client lifted and order arrival, while still rejecting a
/// genuine market move.
pub const DEFAULT_LAST_LOOK_TOLERANCE_BPS: f64 = 1.0;

/// serde default for [`PricingGroupDef::last_look_tolerance_bps`] (additive field).
fn default_last_look_tolerance_bps() -> f64 {
    DEFAULT_LAST_LOOK_TOLERANCE_BPS
}

/// The default [`PricingGroupDef::async_giveback_pct`]: in Async mode, return **50%** of
/// the favorable slippage to the client (the dealer keeps the other half).
pub const DEFAULT_ASYNC_GIVEBACK_PCT: f64 = 50.0;

/// serde default for [`PricingGroupDef::async_giveback_pct`] (additive field).
fn default_async_giveback_pct() -> f64 {
    DEFAULT_ASYNC_GIVEBACK_PCT
}

/// One persisted **pricing group**: a named, trader-defined grouping that binds a
/// set of connected clients (inbound FIX sessions, GUI/API principals, or a desk as
/// a default tier) to their own outbound **feature pipelines**, so different clients
/// receive different outbound prices off the **same** raw composite
/// (`docs/FI-PRICING-GROUPS-DESIGN.md`; Phase 2a).
///
/// Membership is **many-to-one**: a group serves many members, and each member
/// resolves to **exactly one enabled group** (validated — see
/// [`check_pricing_group`](IdentityStore::check_pricing_group) /
/// [`validate_pricing_groups`](IdentityStore::validate_pricing_groups)), so pricing
/// is deterministic. A [`member_desks`](Self::member_desks) entry is a **fallback**
/// tier consulted only when a caller's own connection/user id matches no group.
///
/// The group carries a **separate `FeaturePipeline` per outbound mode** — ESP
/// (streaming) and RFS/RFQ — plus [`share_pipeline`](Self::share_pipeline): when set,
/// the RFQ mode reuses the ESP pipeline (one pipeline for both), so a trader who wants
/// identical ESP/RFQ pricing configures it once.
///
/// `Eq` is intentionally **not** derived: the embedded [`FeaturePipeline`]s carry
/// `f64` feature/guardrail magnitudes, so the definition is only `PartialEq` (as
/// [`AggregatedBookDef`] and [`IdentityStore`] already are, for the same reason).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PricingGroupDef {
    /// Stable identifier (the store/API key). Never reused; minted from the name via
    /// [`mint_pricing_group_id`].
    pub id: String,
    /// Human-friendly group label — the trader's code name (e.g. `GROUP-A`,
    /// `TIER1-EU`) — unique across the store (case-insensitive), validated at load and
    /// at every admin write.
    pub name: String,
    /// Free-text operator description of what the group is for.
    #[serde(default)]
    pub description: String,
    /// Inbound FIX sessions in this group, by **connection id**
    /// ([`super::fix_connections::FixConnectionDef::id`]). Order-preserving and unique
    /// **within this group**. Like [`AggregatedBookDef::member_connection_ids`], these
    /// reference the **separate** fix/connection registry (not part of
    /// [`IdentityStore`]), so their existence is a service-layer concern and is
    /// deliberately **not** resolved here.
    #[serde(default)]
    pub member_connection_ids: Vec<String>,
    /// GUI/API principals in this group, by [`UserDef::id`]. Order-preserving and
    /// unique within this group; each must resolve to an existing user (validated).
    #[serde(default)]
    pub member_user_ids: Vec<String>,
    /// Desk-level **default** membership, by [`DeskDef::id`] — the fallback tier a
    /// caller resolves to when neither its connection nor its user id names a group.
    /// Order-preserving and unique within this group; each must resolve to an existing
    /// desk (validated).
    #[serde(default)]
    pub member_desks: Vec<String>,
    /// The outbound **ESP / streaming** feature pipeline: the ordered features applied
    /// per subscriber to the raw composite before the aggregated-book stream is
    /// published to a member of this group.
    pub esp_pipeline: celnet_tiering::FeaturePipeline,
    /// The outbound **RFS/RFQ** feature pipeline: the ordered features applied to the
    /// raw composite before a quote is packaged for a member of this group. Ignored
    /// when [`share_pipeline`](Self::share_pipeline) is set (the ESP pipeline is reused).
    pub rfq_pipeline: celnet_tiering::FeaturePipeline,
    /// When `true`, the RFS/RFQ mode reuses [`esp_pipeline`](Self::esp_pipeline) — one
    /// pipeline drives both outbound modes; when `false`, each mode uses its own.
    #[serde(default)]
    pub share_pipeline: bool,
    /// The admin-configurable **pricing-source policy** for this group's outbound
    /// rates/bond FIX auto-quotes (see [`PricingSourceMode`]). Additive serde-default:
    /// an `identity.json` written before the policy loads as
    /// [`PricingSourceMode::CompositeFirstCurveFallback`] (the platform default), so a
    /// pre-policy config behaves identically to before.
    #[serde(default)]
    pub pricing_source_mode: PricingSourceMode,
    /// The blend weight `w ∈ [0, 1]` for [`PricingSourceMode::CurveAnchoredBookSkew`]: the
    /// fraction of the curve→composite mid gap the outbound mid traverses,
    /// `skew_mid = curve_mid + w·(composite_mid − curve_mid)` (a convex blend — `w = 0` is
    /// pure curve, `w = 1` pure composite, and the move is inherently bounded by the gap).
    /// Ignored by every other mode. Additive serde-default ([`DEFAULT_BOOK_SKEW_WEIGHT`]);
    /// shape-validated finite and in `[0, 1]` at load and every admin write.
    #[serde(default = "default_book_skew_weight")]
    pub book_skew_weight: f64,
    /// The **market-data (ESP) last-look mode** for this group's outbound stream — how a
    /// FAVORABLE move between the streamed snapshot the client lifted and the current
    /// published price is shared (see [`LastLookMode`]). Additive serde-default
    /// ([`LastLookMode::Sync`]).
    #[serde(default)]
    pub last_look_mode: LastLookMode,
    /// The market-data last-look **adverse-move tolerance in bps of price** (relative, so
    /// it scales across bond price points and OIS rates): a lift whose price has moved
    /// against the dealer by more than `tolerance_bps · 1e-4 · |price|` is rejected as
    /// superseded (token preserved, so a fresh re-lift at the current price can book).
    /// Additive serde-default ([`DEFAULT_LAST_LOOK_TOLERANCE_BPS`]); shape-validated finite
    /// and `>= 0` at load and every admin write.
    #[serde(default = "default_last_look_tolerance_bps")]
    pub last_look_tolerance_bps: f64,
    /// Only meaningful under [`LastLookMode::Async`]: the percentage `∈ [0, 100]` of the
    /// **favorable** slippage returned to the client (the dealer keeps the rest). Additive
    /// serde-default ([`DEFAULT_ASYNC_GIVEBACK_PCT`]); shape-validated finite and in
    /// `[0, 100]` at load and every admin write.
    #[serde(default = "default_async_giveback_pct")]
    pub async_giveback_pct: f64,
    /// Whether the group is active. A disabled group is persisted and editable but
    /// never participates in resolution — its members fall through as if it did not
    /// exist (so a disabled group's members can be re-homed without a determinism
    /// conflict).
    pub enabled: bool,
}

impl PricingGroupDef {
    /// The effective **ESP / streaming** pipeline for this group (always
    /// [`esp_pipeline`](Self::esp_pipeline)).
    #[must_use]
    pub fn esp_effective_pipeline(&self) -> &celnet_tiering::FeaturePipeline {
        &self.esp_pipeline
    }

    /// The effective **RFS/RFQ** pipeline: [`esp_pipeline`](Self::esp_pipeline) when
    /// [`share_pipeline`](Self::share_pipeline) is set, else
    /// [`rfq_pipeline`](Self::rfq_pipeline).
    #[must_use]
    pub fn rfq_effective_pipeline(&self) -> &celnet_tiering::FeaturePipeline {
        if self.share_pipeline {
            &self.esp_pipeline
        } else {
            &self.rfq_pipeline
        }
    }
}

/// The editable fields of a pricing group (the create/update payload the store's
/// [`IdentityStore::create_pricing_group`] /
/// [`update_pricing_group`](IdentityStore::update_pricing_group) take). The
/// definition's `id` is minted (create) or carried from the request (update), so it is
/// not part of the edit — mirrors [`AggregatedBookEdit`].
#[derive(Debug, Clone, PartialEq)]
pub struct PricingGroupEdit {
    /// The trader's code name (unique across the store, case-insensitive).
    pub name: String,
    /// Free-text operator description.
    pub description: String,
    /// Inbound FIX sessions in this group, by connection id.
    pub member_connection_ids: Vec<String>,
    /// GUI/API principals in this group, by user id.
    pub member_user_ids: Vec<String>,
    /// Desk-level default membership, by desk id.
    pub member_desks: Vec<String>,
    /// The outbound ESP / streaming feature pipeline.
    pub esp_pipeline: celnet_tiering::FeaturePipeline,
    /// The outbound RFS/RFQ feature pipeline (ignored when `share_pipeline` is set).
    pub rfq_pipeline: celnet_tiering::FeaturePipeline,
    /// When `true`, the RFS/RFQ mode reuses `esp_pipeline`.
    pub share_pipeline: bool,
    /// The admin-configurable pricing-source policy (see [`PricingSourceMode`]).
    pub pricing_source_mode: PricingSourceMode,
    /// The curve→composite blend weight for [`PricingSourceMode::CurveAnchoredBookSkew`].
    pub book_skew_weight: f64,
    /// The market-data (ESP) last-look mode (see [`LastLookMode`]).
    pub last_look_mode: LastLookMode,
    /// The market-data last-look adverse-move tolerance, in bps of price.
    pub last_look_tolerance_bps: f64,
    /// The % of favorable slippage returned to the client under [`LastLookMode::Async`].
    pub async_giveback_pct: f64,
    /// Whether the group is active.
    pub enabled: bool,
}

/// Which outbound pricing mode a pricing-group pipeline retune targets — the ESP
/// (streaming) pipeline or the RFS/RFQ pipeline. The store side of the proto `EspOrRfq`
/// selector on
/// [`update_pricing_group_pipeline`](IdentityStore::update_pricing_group_pipeline).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PricingMode {
    /// The ESP / streaming pipeline.
    Esp,
    /// The RFS / RFQ pipeline.
    Rfq,
}

/// A precomputed **caller → pricing group** resolver, built once from the
/// [`IdentityStore::pricing_groups`] registry and cached on the hub/edge (mirroring
/// the aggregation identities cache). It maps each **enabled** group's members —
/// connection ids, user ids, and desks — to the group, so a hot-path lookup is a
/// single hash probe.
///
/// Determinism: [`IdentityStore::validate_pricing_groups`] rejects any member that
/// appears in two enabled groups, so each key resolves to exactly one group. The
/// build is nonetheless first-wins per key (deterministic in registry order) as a
/// belt-and-braces guard. Disabled groups are excluded entirely.
#[derive(Debug, Clone, Default)]
pub struct PricingGroupResolver {
    groups: Vec<PricingGroupDef>,
    by_connection: BTreeMap<String, usize>,
    by_user: BTreeMap<String, usize>,
    by_desk: BTreeMap<String, usize>,
}

impl PricingGroupResolver {
    /// Build the resolver from the registry, indexing only **enabled** groups.
    #[must_use]
    pub fn build(groups: &[PricingGroupDef]) -> Self {
        let mut kept: Vec<PricingGroupDef> = Vec::new();
        let mut by_connection = BTreeMap::new();
        let mut by_user = BTreeMap::new();
        let mut by_desk = BTreeMap::new();
        for g in groups.iter().filter(|g| g.enabled) {
            let idx = kept.len();
            for c in &g.member_connection_ids {
                by_connection.entry(c.clone()).or_insert(idx);
            }
            for u in &g.member_user_ids {
                by_user.entry(u.clone()).or_insert(idx);
            }
            for d in &g.member_desks {
                by_desk.entry(d.clone()).or_insert(idx);
            }
            kept.push(g.clone());
        }
        Self {
            groups: kept,
            by_connection,
            by_user,
            by_desk,
        }
    }

    /// Whether the resolver indexes no enabled group (the hot path can then skip
    /// resolution entirely).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Resolve a **FIX connection** to its pricing group: its own `connection id`
    /// first, then — when given — its `desk` as the fallback tier, then none.
    #[must_use]
    pub fn resolve_for_connection(
        &self,
        connection_id: &str,
        desk: Option<&str>,
    ) -> Option<&PricingGroupDef> {
        if let Some(&i) = self.by_connection.get(connection_id) {
            return self.groups.get(i);
        }
        if let Some(d) = desk
            && let Some(&i) = self.by_desk.get(d)
        {
            return self.groups.get(i);
        }
        None
    }

    /// Resolve an authenticated **user** to its pricing group: its own `user id`
    /// first, then — in order — each of its `desk_ids` as the fallback tier (the first
    /// desk that names a group wins, deterministically), then none.
    #[must_use]
    pub fn resolve_for_user(&self, user_id: &str, desk_ids: &[String]) -> Option<&PricingGroupDef> {
        if let Some(&i) = self.by_user.get(user_id) {
            return self.groups.get(i);
        }
        for d in desk_ids {
            if let Some(&i) = self.by_desk.get(d) {
                return self.groups.get(i);
            }
        }
        None
    }
}

/// Per-**risk-book** pre-trade limits: the persisted, shape-validated caps a book
/// carries (`docs/FI-RISK-ROUTING-REQUIREMENTS.md` §3.1). Each cap is an optional,
/// non-negative magnitude in the book's booking currency (net/gross notional) or in
/// PV-per-basis-point (`max_dv01`); `None` means "no cap on this axis".
///
/// This is deliberately a **small, self-describing persisted config shape**, distinct
/// from `celnet_limits::LimitSpec` — that type is a Greeks/rates-aggregate pre-trade
/// *check engine* (delta/vega/DV01 metrics with amber/red utilization bands, not serde,
/// no plain net/gross-notional axis) and does not fit a hand-editable per-book config
/// record. Full limit **enforcement** is a later phase; here the values are only
/// persisted and shape-validated (finite and `>= 0`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct RiskLimits {
    /// Cap on the book's **net** (signed-then-absolute) base-currency notional. `None`
    /// ⇒ uncapped. Must be finite and `>= 0` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_net_notional: Option<f64>,
    /// Cap on the book's **gross** (sum-of-absolute) base-currency notional. `None` ⇒
    /// uncapped. Must be finite and `>= 0` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_gross_notional: Option<f64>,
    /// Cap on the book's net **DV01** magnitude (PV change per +1bp parallel curve bump).
    /// `None` ⇒ uncapped. Must be finite and `>= 0` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_dv01: Option<f64>,
}

impl RiskLimits {
    /// Shape-validate: every present cap is finite and non-negative.
    ///
    /// # Errors
    /// The first cap that is negative or non-finite (NaN / ∞).
    fn validate(&self) -> Result<(), String> {
        let check = |v: Option<f64>, what: &str| -> Result<(), String> {
            match v {
                Some(x) if !(x.is_finite() && x >= 0.0) => {
                    Err(format!("{what} must be finite and non-negative"))
                }
                _ => Ok(()),
            }
        };
        check(self.max_net_notional, "max_net_notional")?;
        check(self.max_gross_notional, "max_gross_notional")?;
        check(self.max_dv01, "max_dv01")?;
        Ok(())
    }
}

/// One persisted **risk book**: a trader-defined portfolio an accepted fill's risk can
/// land in (`docs/FI-RISK-ROUTING-REQUIREMENTS.md` §3.1). Books form a **tree** via
/// [`parent_id`](Self::parent_id) — a top-level book (`parent_id == None`) tags an owning
/// [`desk_id`](Self::desk_id); a sub-book refines a parent for finer attribution and
/// later roll-up. The firm-wide [`RiskRoutingGraph`] routes a fill to a book by id.
///
/// `Eq` is intentionally **not** derived: [`limits`](Self::limits) carries `f64` caps, so
/// the definition is only `PartialEq` (as [`PricingGroupDef`] and [`IdentityStore`] are,
/// for the same reason).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskBookDef {
    /// Stable identifier (the store/API key). Never reused; minted from the name via
    /// [`mint_risk_book_id`].
    pub id: String,
    /// Human-friendly book label — unique across the store (case-insensitive), validated
    /// at load and at every admin write.
    pub name: String,
    /// The parent book by [`RiskBookDef::id`], or `None` for a top-level book. Must
    /// resolve to an existing book and must not form a cycle (a book cannot be its own
    /// ancestor) — validated. An additive serde-default field (no `schema_version`).
    #[serde(default)]
    pub parent_id: Option<String>,
    /// The owning desk by [`DeskDef::id`]. Top-level books tag a desk; `None` ⇒ unowned.
    /// Must resolve to an existing desk when present — validated.
    #[serde(default)]
    pub desk_id: Option<String>,
    /// Free-text operator description of what the book is for.
    #[serde(default)]
    pub description: String,
    /// Optional per-book pre-trade limits (persisted + shape-validated here; enforcement
    /// is a later phase). `None` ⇒ the book carries no caps yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<RiskLimits>,
    /// Whether the book is active. Only **enabled** books are valid routing targets
    /// ([`check_risk_routing_graph`](IdentityStore::check_risk_routing_graph)); a disabled
    /// book is persisted and editable but never routed to.
    pub enabled: bool,
}

/// The editable fields of a risk book (the create/update payload the store's
/// [`IdentityStore::create_risk_book`] /
/// [`update_risk_book`](IdentityStore::update_risk_book) take). The definition's `id` is
/// minted (create) or carried from the request (update), so it is not part of the edit —
/// mirrors [`PricingGroupEdit`].
#[derive(Debug, Clone, PartialEq)]
pub struct RiskBookEdit {
    /// The book label (unique across the store, case-insensitive).
    pub name: String,
    /// The parent book by id, or `None` for a top-level book.
    pub parent_id: Option<String>,
    /// The owning desk by id, or `None`.
    pub desk_id: Option<String>,
    /// Free-text operator description.
    pub description: String,
    /// Optional per-book pre-trade limits.
    pub limits: Option<RiskLimits>,
    /// Whether the book is active.
    pub enabled: bool,
}

/// serde default for the [`PricingControlDef`] flags — both controls default **on**.
const fn default_true() -> bool {
    true
}

/// The persisted **firm-wide pricing kill-switch** setting — the operator's last
/// [`outbound_enabled`](Self::outbound_enabled) / [`inbound_enabled`](Self::inbound_enabled)
/// choice, so a server bounce restores the halt rather than silently resuming pricing.
/// Both flags `#[serde(default = "default_true")]`, so an existing `identity.json`
/// written before this contract loads as **both-enabled** (additive, no
/// `schema_version`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PricingControlDef {
    /// Whether the server sends outbound pricing to FIX-connected clients (RFQ
    /// auto-quotes + RFS/ESP streams). `false` = "Stop all pricing".
    #[serde(default = "default_true")]
    pub outbound_enabled: bool,
    /// Whether LP-feed ingestion into the aggregated books runs. `false` stops new
    /// composite prices entering; combined with `outbound_enabled: false` = "Stop all".
    #[serde(default = "default_true")]
    pub inbound_enabled: bool,
}

impl Default for PricingControlDef {
    /// Both controls on — pricing flows in and out by default.
    fn default() -> Self {
        Self {
            outbound_enabled: true,
            inbound_enabled: true,
        }
    }
}

/// The persisted document: the users and desks of the edge.
///
/// `Eq` is intentionally **not** derived: the instrument reference-data registry
/// carries `f64` convention fields (bond coupon/redemption, futures size/vol), so
/// the document is only `PartialEq` (sufficient for the `assert_eq!`-based tests
/// and the persist-before-commit comparison).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IdentityStore {
    /// The user accounts, in creation order.
    #[serde(default)]
    pub users: Vec<UserDef>,
    /// The desks traders can belong to, in creation order.
    #[serde(default)]
    pub desks: Vec<DeskDef>,
    /// The named legal entities / accounts that map `RatesPosition.entity` keys to
    /// display names, in creation order. An additive serde-default field (no
    /// `schema_version`), so an existing `identity.json` (which carries no
    /// `entities`) loads unchanged.
    #[serde(default)]
    pub entities: Vec<EntityDef>,
    /// The named netting books that map `RatesPosition.book` keys to display names,
    /// in creation order. Each belongs to an entity by [`BookDef::entity_key`]. An
    /// additive serde-default field, so an existing `identity.json` loads unchanged.
    #[serde(default)]
    pub books: Vec<BookDef>,
    /// The admin-editable per-**role** capability bundles — the *base* authority a
    /// role confers before the per-user overlay layers on top
    /// (`docs/plan/PERMISSIONS-ADMINISTRATION-REQUIREMENT.md` §3.3/§10). Keyed by
    /// [`Role`]; only **non-admin** roles appear here ([`Role::Admin`] is always
    /// grant-all and is never narrowable, so it is never stored). A role absent from
    /// the map resolves to [`default_trader_bundle`], so an existing `identity.json`
    /// (which carries no `role_bundles`) loads unchanged — an additive serde-default
    /// field (no `schema_version`). Each entry is stored as the kernel's stable
    /// snake_case labels so the file stays human-editable and round-trips exactly.
    #[serde(default)]
    pub role_bundles: BTreeMap<Role, Vec<PermissionGrant>>,
    /// The instrument **reference-data** registry: instrument definitions keyed by
    /// an internal id with external-id cross-refs, persisted in this same document
    /// so it auto-loads on boot (`super::reference_data`). Curve-building and
    /// pricing resolve a ref → definition against it
    /// (`docs/CURVES-AND-INSTRUMENT-REFERENCE-DATA-REVIEW.md` §C). An additive
    /// serde-default field, so an existing `identity.json` (which carries no
    /// `instruments`) loads unchanged.
    #[serde(default)]
    pub instruments: Vec<InstrumentDef>,
    /// The operator-defined **aggregated books**: named sets of inbound liquidity
    /// members whose per-instrument top-of-book is consolidated into one composite
    /// price (ADR-0022). Global (not desk-owned) and admin-managed. An additive
    /// serde-default field (no `schema_version`), so an existing `identity.json` (which
    /// carries no `aggregated_books`) loads unchanged.
    #[serde(default)]
    pub aggregated_books: Vec<AggregatedBookDef>,
    /// The trader-defined **pricing groups**: named groupings that bind connected
    /// clients (FIX sessions / users / desks) to their own outbound feature pipelines,
    /// so different clients receive different outbound prices off the same raw composite
    /// (`docs/FI-PRICING-GROUPS-DESIGN.md`; Phase 2a). An additive serde-default field
    /// (no `schema_version`), so an existing `identity.json` (which carries no
    /// `pricing_groups`) loads unchanged.
    #[serde(default)]
    pub pricing_groups: Vec<PricingGroupDef>,
    /// The trader-defined **risk books**: the tree of portfolios an accepted fill's risk
    /// can land in (`docs/FI-RISK-ROUTING-REQUIREMENTS.md` §3.1). Books nest via
    /// [`RiskBookDef::parent_id`]; the routing graph below targets them by id. An additive
    /// serde-default field (no `schema_version`), so an existing `identity.json` (which
    /// carries no `risk_books`) loads unchanged.
    #[serde(default)]
    pub risk_books: Vec<RiskBookDef>,
    /// The firm-wide **risk-routing decision graph** that maps an accepted fill to a
    /// landing [`RiskBookDef`] (§8.2). `None` until an operator first defines one; when
    /// present, every `Book` leaf must target an existing **enabled** risk book (validated
    /// at load and at every write). An additive serde-default field, so an existing
    /// `identity.json` (which carries no `risk_routing_graph`) loads unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_routing_graph: Option<RiskRoutingGraph>,
    /// The firm-wide **auto-hedge / risk-internalisation policy graph** (Phase B —
    /// `docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md` §5). The sibling of
    /// [`risk_routing_graph`](Self::risk_routing_graph) whose leaves are **exit actions**
    /// instead of book targets. `None` until an operator first defines one; when present,
    /// every `CrossInternal` leaf must name a known instrument and every `RfqOut` leaf a
    /// known LP (validated at load and every write via
    /// [`check_hedge_policy_graph`](IdentityStore::check_hedge_policy_graph)). An additive
    /// serde-default field, so an existing `identity.json` loads unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hedge_policy_graph: Option<HedgeGraph>,
    /// The firm-wide **incoming-quote-acceptance decision graph** (the third
    /// trader-configurable rule engine — `celnet-acceptance`). Gates whether an inbound
    /// counterparty lift is ACCEPTED, REJECTED, or HELD for manual review at the
    /// acceptance point. The sibling of [`risk_routing_graph`](Self::risk_routing_graph)
    /// and [`hedge_policy_graph`](Self::hedge_policy_graph) whose leaves are **acceptance
    /// actions**. Seeded **accept-all** on a pristine store so existing behaviour is
    /// UNCHANGED until a trader writes rules; validated (acyclic, type-consistent
    /// conditions) at load and every write via
    /// [`check_acceptance_graph`](IdentityStore::check_acceptance_graph). An additive
    /// serde-default field, so an existing `identity.json` (which carries no
    /// `acceptance_graph`) loads unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_graph: Option<AcceptanceGraph>,
    /// The configured **warehouse thresholds** (the "100" per desk / book / instrument —
    /// §4). Each binds a scope id to a soft, banded risk budget. An additive serde-default
    /// list, so an existing `identity.json` (which carries no `hedge_thresholds`) loads
    /// unchanged as an empty roster.
    #[serde(default)]
    pub hedge_thresholds: Vec<ScopedThreshold>,
    /// The **auto-hedge engine config** — kill-switch, advisory-only, per-desk toggles,
    /// rate guards (§8.4). An additive serde-default field (advisory-only defaults ON), so
    /// an existing `identity.json` loads with the safe shadow-run defaults.
    #[serde(default)]
    pub hedge_config: HedgeConfigDef,
    /// The persisted **firm-wide pricing kill-switch** setting (the operator's last
    /// outbound/inbound halt choice). Restored into the runtime `PricingControl` at
    /// boot so a server bounce does not silently resume pricing an operator halted. An
    /// additive serde-default field (both controls default on), so an existing
    /// `identity.json` (which carries no `pricing_control`) loads as both-enabled.
    #[serde(default)]
    pub pricing_control: PricingControlDef,
}

impl IdentityStore {
    /// Resolve the config path from [`CONFIG_ENV`], falling back to
    /// [`DEFAULT_CONFIG_PATH`].
    #[must_use]
    pub fn config_path() -> PathBuf {
        std::env::var_os(CONFIG_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH))
    }

    /// Load from `path`. A **missing** file is a first run ⇒ empty store (not an
    /// error); a present-but-corrupt file is an `InvalidData` error so a broken
    /// identity file fails loudly rather than silently dropping accounts.
    ///
    /// # Errors
    /// Propagates IO errors other than not-found, and JSON parse failures.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let store: Self = serde_json::from_slice(&bytes)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                // A corrupt capability overlay is rejected at load (fail-fast), so a
                // bad label can never silently drop a capability at session build.
                store
                    .validate_capabilities()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                // A corrupt entity/book registry (duplicate keys/names or a book whose
                // `entity_key` dangles) is rejected at load too, so the name↔key map is
                // always sound before any booking form resolves against it.
                store
                    .validate_registry()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                // A corrupt instrument reference-data registry (duplicate ids/external
                // ids, an unknown convention label, or a missing required field) is
                // rejected at load too, so a ref always resolves to a sound definition.
                validate_instruments(&store.instruments)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                // A corrupt aggregated-book set (duplicate/empty name, duplicate member
                // id within a book, an explicit-scope instrument id that dangles, or an
                // out-of-range param) is rejected at load too, so a composite is only
                // ever stood up from a sound definition.
                store
                    .validate_aggregated_books()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                // A corrupt pricing-group set (duplicate/empty name, a member in two
                // enabled groups, an unknown user/desk member, or an invalid pipeline) is
                // rejected at load too, so a client is only ever priced from a sound,
                // deterministic group.
                store
                    .validate_pricing_groups()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                // A corrupt risk-book tree (duplicate/empty name, an unknown/cyclic parent,
                // an unknown owning desk, a negative limit) or a routing graph targeting an
                // unknown/disabled book is rejected at load too, so a fill is only ever
                // routed against a sound, acyclic book tree.
                store
                    .validate_risk_books()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                Ok(store)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Validate every user's capability overlay **and** every persisted role bundle
    /// parses (unknown labels ⇒ error). Called at [`load`](IdentityStore::load) so a
    /// broken `identity.json` fails loudly rather than degrading authority resolution
    /// at runtime.
    ///
    /// # Errors
    /// The first user overlay or role bundle carrying an unknown action/asset label.
    fn validate_capabilities(&self) -> Result<(), String> {
        for user in &self.users {
            user.capability_overlay()
                .map_err(|e| format!("user {:?}: {e}", user.id))?;
        }
        for (role, bundle) in &self.role_bundles {
            for grant in bundle {
                grant
                    .parse()
                    .map_err(|e| format!("role bundle {:?}: {e}", role.as_str()))?;
            }
        }
        Ok(())
    }

    /// Validate the entity/book registry: within each list keys are unique, names
    /// and (for entities) codes are unique case-insensitively, and every book's
    /// `entity_key` resolves to an existing entity. Called at
    /// [`load`](IdentityStore::load) so a corrupt registry fails loudly rather than
    /// resolving a booking-form selection to a wrong or missing key at runtime.
    ///
    /// # Errors
    /// The first duplicate key/name/code, or a book whose `entity_key` dangles.
    fn validate_registry(&self) -> Result<(), String> {
        let mut entity_keys = std::collections::HashSet::new();
        let mut entity_names = std::collections::HashSet::new();
        let mut entity_codes = std::collections::HashSet::new();
        for e in &self.entities {
            if e.name.trim().is_empty() {
                return Err(format!("entity key {} has an empty name", e.key));
            }
            if e.code.trim().is_empty() {
                return Err(format!("entity key {} has an empty code", e.key));
            }
            if !entity_keys.insert(e.key) {
                return Err(format!("duplicate entity key {}", e.key));
            }
            if !entity_names.insert(e.name.to_ascii_lowercase()) {
                return Err(format!("duplicate entity name {:?}", e.name));
            }
            if !entity_codes.insert(e.code.to_ascii_lowercase()) {
                return Err(format!("duplicate entity code {:?}", e.code));
            }
        }
        let mut book_keys = std::collections::HashSet::new();
        let mut book_names = std::collections::HashSet::new();
        for b in &self.books {
            if b.name.trim().is_empty() {
                return Err(format!("book key {} has an empty name", b.key));
            }
            if !book_keys.insert(b.key) {
                return Err(format!("duplicate book key {}", b.key));
            }
            if !book_names.insert(b.name.to_ascii_lowercase()) {
                return Err(format!("duplicate book name {:?}", b.name));
            }
            if !entity_keys.contains(&b.entity_key) {
                return Err(format!(
                    "book {:?} (key {}) references unknown entity_key {}",
                    b.name, b.key, b.entity_key
                ));
            }
        }
        Ok(())
    }

    /// Validate the aggregated-book set: names are non-empty and unique
    /// case-insensitively across the set, and every book satisfies its own invariants
    /// ([`check_aggregated_book`](Self::check_aggregated_book) — no duplicate member ids,
    /// in-range params, and every explicit-scope instrument id resolving to an
    /// [`InstrumentDef`]). Called at [`load`](IdentityStore::load) so a corrupt set
    /// fails loudly rather than standing up a composite from an unsound definition.
    ///
    /// Member connection ids are **not** resolved here: they reference the separate
    /// fix/connection registry (not part of this document), so cross-store resolution
    /// is a service-layer concern (task T1.3).
    ///
    /// # Errors
    /// The first empty/duplicate name, or the first book failing its invariants.
    fn validate_aggregated_books(&self) -> Result<(), String> {
        let mut names = std::collections::HashSet::new();
        for b in &self.aggregated_books {
            if b.name.trim().is_empty() {
                return Err(format!("aggregated book {:?} has an empty name", b.id));
            }
            if !names.insert(b.name.to_ascii_lowercase()) {
                return Err(format!("duplicate aggregated book name {:?}", b.name));
            }
            self.check_aggregated_book(b)?;
        }
        Ok(())
    }

    /// Validate a single aggregated book's **document-resolvable** invariants, used by
    /// both load-time validation and every admin write (so a bad definition is rejected
    /// identically whichever path creates it):
    ///
    /// * no duplicate `member_connection_ids` within the book;
    /// * `params.min_contributors >= 1` and `params.staleness_tau_ms > 0`;
    /// * every [`Scope::Explicit`] instrument id resolves to an existing
    ///   [`InstrumentDef`] in [`instruments`](Self::instruments).
    ///
    /// Name uniqueness is **not** checked here (it needs the surrounding set — the
    /// callers layer it on); member connection ids are **not** resolved here (they
    /// live in the separate connection registry — a service-layer concern, T1.3).
    ///
    /// # Errors
    /// The first duplicate member id, out-of-range param, or dangling instrument id.
    fn check_aggregated_book(&self, def: &AggregatedBookDef) -> Result<(), String> {
        let mut seen_members = std::collections::HashSet::new();
        for member in &def.member_connection_ids {
            if !seen_members.insert(member) {
                return Err(format!(
                    "aggregated book {:?} lists member {:?} more than once",
                    def.name, member
                ));
            }
        }
        if def.params.min_contributors < 1 {
            return Err(format!(
                "aggregated book {:?} requires min_contributors >= 1",
                def.name
            ));
        }
        if def.params.staleness_tau_ms == 0 {
            return Err(format!(
                "aggregated book {:?} requires staleness_tau_ms > 0",
                def.name
            ));
        }
        if let Scope::Explicit(ids) = &def.instrument_scope {
            for id in ids {
                if self.instrument_by_id(id).is_none() {
                    return Err(format!(
                        "aggregated book {:?} scopes unknown instrument_id {:?}",
                        def.name, id
                    ));
                }
            }
        }
        Ok(())
    }

    /// Seed a small, realistic default registry on a store that has **no** entities,
    /// so a fresh edge can book named positions immediately; report `true` (the
    /// caller should persist). A store that already has any entity is left untouched
    /// and reports `false` (idempotent, mirroring [`ensure_seed_admin`]). The seeded
    /// strings are sample DATA, not product identifiers.
    pub fn ensure_seed_registry(&mut self) -> bool {
        if !self.entities.is_empty() {
            return false;
        }
        self.entities = vec![
            EntityDef {
                key: 1,
                name: "Celnet Global Markets".to_string(),
                code: "CGM".to_string(),
            },
            EntityDef {
                key: 2,
                name: "Celnet Securities".to_string(),
                code: "CSEC".to_string(),
            },
        ];
        self.books = vec![
            BookDef {
                key: 1,
                name: "Rates Trading".to_string(),
                entity_key: 1,
            },
            BookDef {
                key: 2,
                name: "Rates Relative Value".to_string(),
                entity_key: 1,
            },
            BookDef {
                key: 3,
                name: "Government Bonds".to_string(),
                entity_key: 2,
            },
            BookDef {
                key: 4,
                name: "Swaps".to_string(),
                entity_key: 2,
            },
        ];
        true
    }

    /// Borrow an entity by its `uint32` wire key.
    #[must_use]
    pub fn entity_by_key(&self, key: u32) -> Option<&EntityDef> {
        self.entities.iter().find(|e| e.key == key)
    }

    /// Borrow a book by its `uint32` wire key.
    #[must_use]
    pub fn book_by_key(&self, key: u32) -> Option<&BookDef> {
        self.books.iter().find(|b| b.key == key)
    }

    /// The display name of an entity key, or `None` if no entity names it. The
    /// blotter/Book views resolve `RatesPosition.entity` through this so a raw number
    /// is never shown.
    #[must_use]
    pub fn entity_name(&self, key: u32) -> Option<&str> {
        self.entity_by_key(key).map(|e| e.name.as_str())
    }

    /// The display name of a book key, or `None` if no book names it.
    #[must_use]
    pub fn book_name(&self, key: u32) -> Option<&str> {
        self.book_by_key(key).map(|b| b.name.as_str())
    }

    /// Seed a small, realistic default **instrument reference-data** registry on a
    /// store that has no instruments; report `true` (the caller should persist). A
    /// store that already has any instrument is left untouched and reports `false`
    /// (idempotent, mirroring [`ensure_seed_registry`](Self::ensure_seed_registry)).
    pub fn ensure_seed_instruments(&mut self) -> bool {
        ensure_seed_instruments(&mut self.instruments)
    }

    /// **Additively** ensure the curated government-bond reference universe (US
    /// Treasuries + UK gilts + EUR govvies from [`government_bond_defs`]) is present,
    /// adding only the definitions whose `instrument_id` is not already registered and
    /// whose external ids do not collide with an existing entry; report `true` when any
    /// were added (the caller should persist). Unlike [`ensure_seed_instruments`], this
    /// runs on EVERY boot (not only an empty store) so an already-populated registry
    /// gains the government universe without wiping admin-added instruments — the FI
    /// Aggregated Book tiles then resolve real names and the security-list download can
    /// filter by region + sub-asset-type. Idempotent: a second call adds nothing.
    pub fn ensure_seed_government_bonds(&mut self) -> bool {
        let mut have_ids: std::collections::HashSet<String> = self
            .instruments
            .iter()
            .map(|d| d.instrument_id.to_ascii_lowercase())
            .collect();
        let mut have_ext: std::collections::HashSet<(String, String)> = self
            .instruments
            .iter()
            .flat_map(|d| &d.external_ids)
            .map(|e| (e.scheme.to_ascii_lowercase(), e.value.to_ascii_lowercase()))
            .collect();
        let mut added = 0usize;
        for def in government_bond_defs() {
            if have_ids.contains(&def.instrument_id.to_ascii_lowercase()) {
                continue;
            }
            if def.external_ids.iter().any(|e| {
                have_ext.contains(&(e.scheme.to_ascii_lowercase(), e.value.to_ascii_lowercase()))
            }) {
                continue;
            }
            have_ids.insert(def.instrument_id.to_ascii_lowercase());
            for e in &def.external_ids {
                have_ext.insert((e.scheme.to_ascii_lowercase(), e.value.to_ascii_lowercase()));
            }
            self.instruments.push(def);
            added += 1;
        }
        added > 0
    }

    /// Resolve an instrument definition by its internal `instrument_id` — the
    /// primary reference-data lookup curve-building/pricing will consume.
    #[must_use]
    pub fn instrument_by_id(&self, instrument_id: &str) -> Option<&InstrumentDef> {
        self.instruments
            .iter()
            .find(|i| i.instrument_id == instrument_id)
    }

    /// Resolve an instrument definition by an external `(scheme, value)` cross-ref
    /// (case-insensitive on both), e.g. `(Isin, "US91282CKM23")`.
    #[must_use]
    pub fn instrument_by_external_id(
        &self,
        scheme: ExternalScheme,
        value: &str,
    ) -> Option<&InstrumentDef> {
        let want = value.trim();
        self.instruments.iter().find(|i| {
            i.external_ids.iter().any(|x| {
                reference_data::ExternalScheme::from_label(&x.scheme) == Some(scheme)
                    && x.value.eq_ignore_ascii_case(want)
            })
        })
    }

    /// The lowest `uint32` key not already used by an entity (for auto-assignment
    /// when an admin creates an entity without pinning a specific key). Starts at 1
    /// (0 is the protobuf default/"unset" sentinel, never a registry key).
    #[must_use]
    pub fn next_entity_key(&self) -> u32 {
        (1..)
            .find(|k| self.entity_by_key(*k).is_none())
            .unwrap_or(1)
    }

    /// The lowest `uint32` key not already used by a book (for auto-assignment).
    #[must_use]
    pub fn next_book_key(&self) -> u32 {
        (1..).find(|k| self.book_by_key(*k).is_none()).unwrap_or(1)
    }

    /// The resolved capability **base** a role confers, before the per-user overlay.
    ///
    /// * [`Role::Admin`] ⇒ an empty set here — administration is grant-all and is
    ///   resolved by the session's grant-all path, never narrowable, so the Admin
    ///   role never has (and can never be given) a stored bundle.
    /// * A non-admin role ⇒ its persisted [`role_bundles`](Self::role_bundles) entry,
    ///   or [`default_trader_bundle`] if none is stored.
    ///
    /// Labels are validated at load and at every admin write, so a (load-impossible)
    /// malformed entry is dropped per-list rather than panicking — dropping a base
    /// capability fails closed (less authority), the safe direction.
    #[must_use]
    pub fn role_base(&self, role: Role) -> Vec<Capability> {
        if role.is_admin() {
            return Vec::new();
        }
        match self.role_bundles.get(&role) {
            Some(bundle) => bundle.iter().filter_map(|g| g.parse().ok()).collect(),
            None => default_trader_bundle(),
        }
    }

    /// Persist **atomically**: pretty-print to a sibling `*.tmp` file then rename
    /// over `path`, so a crash mid-write can never leave a half-written identity
    /// file (the same durability discipline as [`super::fix_connections`]).
    ///
    /// # Errors
    /// Propagates IO/serialization failures.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = tmp_sibling(path);
        std::fs::write(&tmp, &json)?;
        // The file holds Argon2id password hashes — restrict it to the owning
        // process so it is never world-readable for offline cracking (Unix).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Ensure a default administrator exists. On a store with **no** enabled
    /// admin, seed [`SEED_ADMIN_EMAIL`] / [`SEED_ADMIN_PASSWORD`] as a
    /// [`Role::Admin`] user and report `true` (the caller should persist). A store
    /// that already has any enabled admin is left untouched and reports `false`.
    ///
    /// # Errors
    /// Propagates a password-hashing failure (an OS CSPRNG failure).
    pub fn ensure_seed_admin(&mut self) -> Result<bool, String> {
        if self.users.iter().any(|u| u.role.is_admin() && !u.disabled) {
            return Ok(false);
        }
        let hash = hash_password(SEED_ADMIN_PASSWORD)?;
        self.users.push(UserDef {
            id: mint_user_id(SEED_ADMIN_EMAIL, &self.users),
            email: SEED_ADMIN_EMAIL.to_string(),
            display_name: "Administrator".to_string(),
            role: Role::Admin,
            desk_ids: Vec::new(),
            all_desks: false,
            password_hash: hash,
            disabled: false,
            capability_grants: Vec::new(),
            capability_denies: Vec::new(),
        });
        Ok(true)
    }

    /// Borrow a user by id.
    #[must_use]
    pub fn user(&self, id: &str) -> Option<&UserDef> {
        self.users.iter().find(|u| u.id == id)
    }

    /// Borrow a user by login email (case-insensitive) — the login lookup.
    #[must_use]
    pub fn user_by_email(&self, email: &str) -> Option<&UserDef> {
        let needle = email.trim().to_ascii_lowercase();
        self.users
            .iter()
            .find(|u| u.email.to_ascii_lowercase() == needle)
    }

    /// Borrow a desk by id.
    #[must_use]
    pub fn desk(&self, id: &str) -> Option<&DeskDef> {
        self.desks.iter().find(|d| d.id == id)
    }

    /// Borrow an aggregated book by id.
    #[must_use]
    pub fn aggregated_book(&self, id: &str) -> Option<&AggregatedBookDef> {
        self.aggregated_books.iter().find(|b| b.id == id)
    }

    /// Create an aggregated book from operator input: trims and uniqueness-checks the
    /// name, mints a stable id, validates the definition's document-resolvable
    /// invariants ([`check_aggregated_book`](Self::check_aggregated_book)), appends it,
    /// and returns the created definition. Mirrors the inline create-validation the
    /// entity/book admin RPCs run, so the config layer rejects a bad book identically
    /// whether it arrives at load or over the wire (T1.3 calls this).
    ///
    /// Note member connection ids are **not** resolved here (they live in the separate
    /// connection registry — a service-layer concern).
    ///
    /// # Errors
    /// An empty or (case-insensitively) duplicate name, or a definition failing its
    /// invariants (duplicate member id, out-of-range param, dangling instrument id).
    pub fn create_aggregated_book(
        &mut self,
        edit: AggregatedBookEdit,
    ) -> Result<AggregatedBookDef, String> {
        let name = edit.name.trim().to_string();
        if name.is_empty() {
            return Err("aggregated book name is required".to_string());
        }
        if self
            .aggregated_books
            .iter()
            .any(|b| b.name.eq_ignore_ascii_case(&name))
        {
            return Err(format!("an aggregated book named {name:?} already exists"));
        }
        let def = AggregatedBookDef {
            id: mint_aggregated_book_id(&name, &self.aggregated_books),
            name,
            member_connection_ids: edit.member_connection_ids,
            instrument_scope: edit.instrument_scope,
            params: edit.params,
            enabled: edit.enabled,
        };
        self.check_aggregated_book(&def)?;
        self.aggregated_books.push(def.clone());
        Ok(def)
    }

    /// Update an existing aggregated book in place (id preserved): trims and
    /// uniqueness-checks the name against **every other** book (the edited one may keep
    /// its own name), validates the new definition's invariants, replaces the slot, and
    /// returns the updated definition.
    ///
    /// # Errors
    /// No book with `id`; an empty or duplicate name; or a definition failing its
    /// invariants.
    pub fn update_aggregated_book(
        &mut self,
        id: &str,
        edit: AggregatedBookEdit,
    ) -> Result<AggregatedBookDef, String> {
        if self.aggregated_book(id).is_none() {
            return Err(format!("no aggregated book with id {id:?}"));
        }
        let name = edit.name.trim().to_string();
        if name.is_empty() {
            return Err("aggregated book name is required".to_string());
        }
        if self
            .aggregated_books
            .iter()
            .any(|b| b.id != id && b.name.eq_ignore_ascii_case(&name))
        {
            return Err(format!("an aggregated book named {name:?} already exists"));
        }
        let def = AggregatedBookDef {
            id: id.to_string(),
            name,
            member_connection_ids: edit.member_connection_ids,
            instrument_scope: edit.instrument_scope,
            params: edit.params,
            enabled: edit.enabled,
        };
        self.check_aggregated_book(&def)?;
        if let Some(slot) = self.aggregated_books.iter_mut().find(|b| b.id == id) {
            *slot = def.clone();
        }
        Ok(def)
    }

    /// Delete an aggregated book by id, reporting whether one was removed (a missing id
    /// is a no-op that reports `false`, mirroring the entity/book delete RPCs). An
    /// aggregated book is global and owns no downstream rows, so — unlike an entity —
    /// there is no referential-integrity guard to run first.
    ///
    /// # Errors
    /// Reserved for signature consistency with the other CRUD helpers; deletion has no
    /// document-resolvable failure mode, so this is always `Ok`.
    pub fn delete_aggregated_book(&mut self, id: &str) -> Result<bool, String> {
        let before = self.aggregated_books.len();
        self.aggregated_books.retain(|b| b.id != id);
        Ok(self.aggregated_books.len() != before)
    }

    /// Borrow a pricing group by id.
    #[must_use]
    pub fn pricing_group(&self, id: &str) -> Option<&PricingGroupDef> {
        self.pricing_groups.iter().find(|g| g.id == id)
    }

    /// Create a pricing group from operator input: trims the name, mints a stable id,
    /// appends it, and re-validates the **whole** set
    /// ([`validate_pricing_groups`](Self::validate_pricing_groups)) — so name
    /// uniqueness, per-group invariants, and the cross-group determinism rule (no member
    /// in two enabled groups) are enforced identically whether a group arrives at load
    /// or over the wire. On any failure the tentative group is rolled back and `self` is
    /// left unchanged.
    ///
    /// Member connection ids are **not** resolved here (they live in the separate
    /// connection registry — a service-layer concern, exactly as for aggregated books).
    ///
    /// # Errors
    /// An empty/duplicate name, an unknown user/desk member, a duplicate member, an
    /// invalid feature pipeline, or a member already in another enabled group.
    pub fn create_pricing_group(
        &mut self,
        edit: PricingGroupEdit,
    ) -> Result<PricingGroupDef, String> {
        let name = edit.name.trim().to_string();
        if name.is_empty() {
            return Err("pricing group name is required".to_string());
        }
        let def = PricingGroupDef {
            id: mint_pricing_group_id(&name, &self.pricing_groups),
            name,
            description: edit.description,
            member_connection_ids: edit.member_connection_ids,
            member_user_ids: edit.member_user_ids,
            member_desks: edit.member_desks,
            esp_pipeline: edit.esp_pipeline,
            rfq_pipeline: edit.rfq_pipeline,
            share_pipeline: edit.share_pipeline,
            pricing_source_mode: edit.pricing_source_mode,
            book_skew_weight: edit.book_skew_weight,
            last_look_mode: edit.last_look_mode,
            last_look_tolerance_bps: edit.last_look_tolerance_bps,
            async_giveback_pct: edit.async_giveback_pct,
            enabled: edit.enabled,
        };
        self.pricing_groups.push(def.clone());
        if let Err(e) = self.validate_pricing_groups() {
            self.pricing_groups.pop();
            return Err(e);
        }
        Ok(def)
    }

    /// Replace an existing pricing group in place (id preserved): rebuilds the
    /// definition from `edit`, swaps the slot, and re-validates the **whole** set. On
    /// failure the prior definition is restored and `self` is left unchanged.
    ///
    /// # Errors
    /// No group with `id`; an empty/duplicate name; an unknown user/desk member; a
    /// duplicate member; an invalid pipeline; or a member already in another enabled
    /// group.
    pub fn update_pricing_group(
        &mut self,
        id: &str,
        edit: PricingGroupEdit,
    ) -> Result<PricingGroupDef, String> {
        let Some(pos) = self.pricing_groups.iter().position(|g| g.id == id) else {
            return Err(format!("no pricing group with id {id:?}"));
        };
        let name = edit.name.trim().to_string();
        if name.is_empty() {
            return Err("pricing group name is required".to_string());
        }
        let def = PricingGroupDef {
            id: id.to_string(),
            name,
            description: edit.description,
            member_connection_ids: edit.member_connection_ids,
            member_user_ids: edit.member_user_ids,
            member_desks: edit.member_desks,
            esp_pipeline: edit.esp_pipeline,
            rfq_pipeline: edit.rfq_pipeline,
            share_pipeline: edit.share_pipeline,
            pricing_source_mode: edit.pricing_source_mode,
            book_skew_weight: edit.book_skew_weight,
            last_look_mode: edit.last_look_mode,
            last_look_tolerance_bps: edit.last_look_tolerance_bps,
            async_giveback_pct: edit.async_giveback_pct,
            enabled: edit.enabled,
        };
        let prev = std::mem::replace(&mut self.pricing_groups[pos], def.clone());
        if let Err(e) = self.validate_pricing_groups() {
            self.pricing_groups[pos] = prev;
            return Err(e);
        }
        Ok(def)
    }

    /// Retune **only** one outbound pipeline of a pricing group (the ESP or the RFS/RFQ
    /// [`FeaturePipeline`](celnet_tiering::FeaturePipeline)) plus its
    /// [`share_pipeline`](PricingGroupDef::share_pipeline) flag, leaving the group's
    /// **structure** — id, name, description, all three membership lists, enabled flag,
    /// and the OTHER mode's pipeline — byte-identical. Re-validates the whole set (a bad
    /// pipeline fails the group's invariants); on failure the prior definition is
    /// restored. This is the store side of the trader-facing pipeline RPC: the admin
    /// owns what the group *is* and who is in it; the trader owns the outbound pricing.
    ///
    /// # Errors
    /// No group with `id`, or the retuned pipeline failing its invariants (inconsistent
    /// guardrails, invalid embedded tiering config, non-finite feature magnitude).
    pub fn update_pricing_group_pipeline(
        &mut self,
        id: &str,
        mode: PricingMode,
        pipeline: celnet_tiering::FeaturePipeline,
        share_pipeline: bool,
    ) -> Result<PricingGroupDef, String> {
        let Some(pos) = self.pricing_groups.iter().position(|g| g.id == id) else {
            return Err(format!("no pricing group with id {id:?}"));
        };
        let mut def = self.pricing_groups[pos].clone();
        match mode {
            PricingMode::Esp => def.esp_pipeline = pipeline,
            PricingMode::Rfq => def.rfq_pipeline = pipeline,
        }
        def.share_pipeline = share_pipeline;
        let prev = std::mem::replace(&mut self.pricing_groups[pos], def.clone());
        if let Err(e) = self.validate_pricing_groups() {
            self.pricing_groups[pos] = prev;
            return Err(e);
        }
        Ok(def)
    }

    /// Delete a pricing group by id, reporting whether one was removed (a missing id is a
    /// no-op that reports `false`, mirroring [`delete_aggregated_book`](Self::delete_aggregated_book)).
    /// Removing a group can never create a determinism conflict, so no re-validation is
    /// needed.
    ///
    /// # Errors
    /// Reserved for signature consistency with the other CRUD helpers; deletion has no
    /// document-resolvable failure mode, so this is always `Ok`.
    pub fn delete_pricing_group(&mut self, id: &str) -> Result<bool, String> {
        let before = self.pricing_groups.len();
        self.pricing_groups.retain(|g| g.id != id);
        Ok(self.pricing_groups.len() != before)
    }

    /// Build the cached **caller → pricing-group** resolver from the registry — the
    /// deterministic map the ESP / RFQ pricing paths resolve a subscriber/caller
    /// against (cached on the hub, rebuilt on every reconcile). Only **enabled** groups
    /// are indexed; determinism is guaranteed by
    /// [`validate_pricing_groups`](Self::validate_pricing_groups).
    #[must_use]
    pub fn pricing_group_resolver(&self) -> PricingGroupResolver {
        PricingGroupResolver::build(&self.pricing_groups)
    }

    /// Validate the pricing-group set: names are non-empty and unique
    /// case-insensitively, every group satisfies its own invariants
    /// ([`check_pricing_group`](Self::check_pricing_group)), and — the determinism
    /// guarantee — **no member (connection id, user id, or desk) belongs to two enabled
    /// groups**, so caller → group resolution has exactly one answer. Called at
    /// [`load`](Self::load) and by every admin write.
    ///
    /// Member **connection ids** are not resolved against a registry here (they live in
    /// the separate fix/connection registry — a service-layer concern, exactly as for
    /// [`AggregatedBookDef::member_connection_ids`]); user ids and desks, which live in
    /// this document, **are** resolved by [`check_pricing_group`](Self::check_pricing_group).
    ///
    /// # Errors
    /// The first empty/duplicate name, a group failing its invariants, or a member in
    /// two enabled groups.
    fn validate_pricing_groups(&self) -> Result<(), String> {
        let mut names = std::collections::HashSet::new();
        // A member may belong to at most ONE enabled group ⇒ deterministic resolution.
        // Track which enabled group owns each connection / user / desk key.
        let mut conn_owner: std::collections::HashMap<&str, &str> =
            std::collections::HashMap::new();
        let mut user_owner: std::collections::HashMap<&str, &str> =
            std::collections::HashMap::new();
        let mut desk_owner: std::collections::HashMap<&str, &str> =
            std::collections::HashMap::new();
        for g in &self.pricing_groups {
            if g.name.trim().is_empty() {
                return Err(format!("pricing group {:?} has an empty name", g.id));
            }
            if !names.insert(g.name.to_ascii_lowercase()) {
                return Err(format!("duplicate pricing group name {:?}", g.name));
            }
            self.check_pricing_group(g)?;
            // Determinism is a property of the ENABLED groups only — a disabled group
            // never resolves, so its members are free to live elsewhere.
            if !g.enabled {
                continue;
            }
            // Within-group duplicates are already rejected by `check_pricing_group`, so
            // any collision here is a genuine cross-group conflict.
            for c in &g.member_connection_ids {
                if let Some(prev) = conn_owner.insert(c.as_str(), g.id.as_str()) {
                    return Err(format!(
                        "connection {c:?} is a member of two enabled pricing groups ({prev:?} and {:?})",
                        g.id
                    ));
                }
            }
            for u in &g.member_user_ids {
                if let Some(prev) = user_owner.insert(u.as_str(), g.id.as_str()) {
                    return Err(format!(
                        "user {u:?} is a member of two enabled pricing groups ({prev:?} and {:?})",
                        g.id
                    ));
                }
            }
            for d in &g.member_desks {
                if let Some(prev) = desk_owner.insert(d.as_str(), g.id.as_str()) {
                    return Err(format!(
                        "desk {d:?} is a member of two enabled pricing groups ({prev:?} and {:?})",
                        g.id
                    ));
                }
            }
        }
        Ok(())
    }

    /// Validate a single pricing group's **document-resolvable** invariants (used by
    /// both load-time validation and every admin write):
    ///
    /// * no duplicate member within any one dimension (connection / user / desk);
    /// * every user member resolves to an existing [`UserDef`] and every desk member to
    ///   an existing [`DeskDef`];
    /// * both feature pipelines (ESP and RFQ) are valid
    ///   ([`validate_feature_pipeline`] — consistent guardrails + a valid embedded
    ///   [`TieringConfig`](celnet_tiering::TieringConfig) per TIERING feature).
    ///
    /// Name uniqueness and the cross-group at-most-one-enabled-group rule are **not**
    /// checked here (they need the surrounding set — the caller layers them on). Member
    /// connection ids are **not** resolved (they live in the separate connection
    /// registry — a service-layer concern, exactly as for aggregated books).
    ///
    /// # Errors
    /// The first duplicate member, unknown user/desk member, or invalid pipeline.
    fn check_pricing_group(&self, def: &PricingGroupDef) -> Result<(), String> {
        let mut seen_conn = std::collections::HashSet::new();
        for c in &def.member_connection_ids {
            if !seen_conn.insert(c) {
                return Err(format!(
                    "pricing group {:?} lists connection member {:?} more than once",
                    def.name, c
                ));
            }
        }
        let mut seen_user = std::collections::HashSet::new();
        for u in &def.member_user_ids {
            if !seen_user.insert(u) {
                return Err(format!(
                    "pricing group {:?} lists user member {:?} more than once",
                    def.name, u
                ));
            }
            if self.user(u).is_none() {
                return Err(format!(
                    "pricing group {:?} lists unknown user member {:?}",
                    def.name, u
                ));
            }
        }
        let mut seen_desk = std::collections::HashSet::new();
        for d in &def.member_desks {
            if !seen_desk.insert(d) {
                return Err(format!(
                    "pricing group {:?} lists desk member {:?} more than once",
                    def.name, d
                ));
            }
            if self.desk(d).is_none() {
                return Err(format!(
                    "pricing group {:?} lists unknown desk member {:?}",
                    def.name, d
                ));
            }
        }
        // The curve→composite blend weight must be finite and in `[0, 1]` (a convex blend
        // weight) — rejected loudly at the admin write / at load rather than silently
        // clamped at pricing time, so a bad config never reaches the hot path.
        if !(def.book_skew_weight.is_finite() && (0.0..=1.0).contains(&def.book_skew_weight)) {
            return Err(format!(
                "pricing group {:?} book_skew_weight must be finite and in [0, 1]",
                def.name
            ));
        }
        // The market-data last-look tolerance is a non-negative bps-of-price band; a bad
        // value fails loudly at the admin write / at load rather than at pricing time.
        if !(def.last_look_tolerance_bps.is_finite() && def.last_look_tolerance_bps >= 0.0) {
            return Err(format!(
                "pricing group {:?} last_look_tolerance_bps must be finite and >= 0",
                def.name
            ));
        }
        // The async giveback is a percentage in [0, 100].
        if !(def.async_giveback_pct.is_finite() && (0.0..=100.0).contains(&def.async_giveback_pct))
        {
            return Err(format!(
                "pricing group {:?} async_giveback_pct must be finite and in [0, 100]",
                def.name
            ));
        }
        validate_feature_pipeline(&def.name, "ESP", &def.esp_pipeline)?;
        // The RFQ pipeline is IGNORED when `share_pipeline` is set (`rfq_effective_pipeline`
        // returns the ESP pipeline), so only validate it when it is actually used — otherwise
        // a group that shares its pipeline is wrongly rejected for an unused/default RFQ block.
        if !def.share_pipeline {
            validate_feature_pipeline(&def.name, "RFQ", &def.rfq_pipeline)?;
        }
        Ok(())
    }

    // --- risk books + routing graph (FI risk routing, Phase 2) ----------------

    /// Borrow a risk book by id.
    #[must_use]
    pub fn risk_book(&self, id: &str) -> Option<&RiskBookDef> {
        self.risk_books.iter().find(|b| b.id == id)
    }

    /// The firm-wide risk-routing decision graph, if one has been defined.
    #[must_use]
    pub fn risk_routing_graph(&self) -> Option<&RiskRoutingGraph> {
        self.risk_routing_graph.as_ref()
    }

    /// Create a risk book from operator input: trims the name, mints a stable id, appends
    /// it, and re-validates the **whole** risk config
    /// ([`validate_risk_books`](Self::validate_risk_books)) — so name uniqueness, the
    /// parent tree (existing + acyclic), the owning desk, and the limit shape are enforced
    /// identically whether a book arrives at load or over the wire. On any failure the
    /// tentative book is rolled back and `self` is left unchanged.
    ///
    /// # Errors
    /// An empty/duplicate name, an unknown/cyclic parent, an unknown desk, or a negative
    /// limit value.
    pub fn create_risk_book(&mut self, edit: RiskBookEdit) -> Result<RiskBookDef, String> {
        let name = edit.name.trim().to_string();
        if name.is_empty() {
            return Err("risk book name is required".to_string());
        }
        let def = RiskBookDef {
            id: mint_risk_book_id(&name, &self.risk_books),
            name,
            parent_id: edit.parent_id,
            desk_id: edit.desk_id,
            description: edit.description,
            limits: edit.limits,
            enabled: edit.enabled,
        };
        self.risk_books.push(def.clone());
        if let Err(e) = self.validate_risk_books() {
            self.risk_books.pop();
            return Err(e);
        }
        Ok(def)
    }

    /// Replace an existing risk book in place (id preserved): rebuilds the definition from
    /// `edit`, swaps the slot, and re-validates the **whole** risk config (so a rename, a
    /// re-parent that would cycle, or disabling a book the routing graph still targets is
    /// rejected). On failure the prior definition is restored and `self` is left unchanged.
    ///
    /// # Errors
    /// No book with `id`; an empty/duplicate name; an unknown/cyclic parent; an unknown
    /// desk; a negative limit; or a change that would leave the routing graph targeting an
    /// unknown/disabled book.
    pub fn update_risk_book(
        &mut self,
        id: &str,
        edit: RiskBookEdit,
    ) -> Result<RiskBookDef, String> {
        let Some(pos) = self.risk_books.iter().position(|b| b.id == id) else {
            return Err(format!("no risk book with id {id:?}"));
        };
        let name = edit.name.trim().to_string();
        if name.is_empty() {
            return Err("risk book name is required".to_string());
        }
        let def = RiskBookDef {
            id: id.to_string(),
            name,
            parent_id: edit.parent_id,
            desk_id: edit.desk_id,
            description: edit.description,
            limits: edit.limits,
            enabled: edit.enabled,
        };
        let prev = std::mem::replace(&mut self.risk_books[pos], def.clone());
        if let Err(e) = self.validate_risk_books() {
            self.risk_books[pos] = prev;
            return Err(e);
        }
        Ok(def)
    }

    /// Delete a risk book by id, reporting whether one was removed (a missing id is a
    /// no-op that reports `false`). **No orphaning:** a book that is the parent of another
    /// book, or that an existing routing-graph `Book` leaf targets, is refused so the tree
    /// and graph stay referentially sound.
    ///
    /// # Errors
    /// The book is a parent of another book, or a routing-graph leaf targets it.
    pub fn delete_risk_book(&mut self, id: &str) -> Result<bool, String> {
        if self.risk_book(id).is_none() {
            return Ok(false);
        }
        if let Some(child) = self
            .risk_books
            .iter()
            .find(|b| b.parent_id.as_deref() == Some(id))
        {
            return Err(format!(
                "risk book {id:?} cannot be deleted: it is the parent of {:?}",
                child.id
            ));
        }
        if let Some(graph) = &self.risk_routing_graph
            && graph
                .nodes
                .values()
                .any(|n| matches!(n, RoutingNode::Book { risk_book_id } if risk_book_id == id))
        {
            return Err(format!(
                "risk book {id:?} cannot be deleted: it is a target of the risk-routing graph"
            ));
        }
        let before = self.risk_books.len();
        self.risk_books.retain(|b| b.id != id);
        Ok(self.risk_books.len() != before)
    }

    /// Install (or replace) the firm-wide risk-routing graph after validating it against
    /// the current risk-book registry ([`check_risk_routing_graph`](Self::check_risk_routing_graph)).
    /// On any defect the store is left unchanged.
    ///
    /// # Errors
    /// The graph is malformed (dangling entry/edge, a cycle, a type-inconsistent
    /// condition) or targets a risk book that does not exist or is disabled.
    pub fn set_risk_routing_graph(&mut self, graph: RiskRoutingGraph) -> Result<(), String> {
        self.check_risk_routing_graph(&graph)?;
        self.risk_routing_graph = Some(graph);
        Ok(())
    }

    /// Install (or replace) the firm-wide risk-routing graph, **auto-provisioning** an
    /// ENABLED [`RiskBookDef`] for every terminal [`RoutingNode::Book`] leaf whose target
    /// id has no existing book — so saving a graph that references a not-yet-created
    /// portfolio id *creates that portfolio* (enabled) and a routed fill therefore always
    /// has an enabled home to show on the per-book risk dashboard
    /// (`docs/FI-RISK-ROUTING-REQUIREMENTS.md` §8.2). Idempotent and **non-destructive**:
    /// a leaf that names an already-defined book (enabled OR disabled) never mutates it, so
    /// re-saving a graph provisions nothing new and a deliberately-disabled book stays
    /// disabled (and its graph then fails validation, loudly, rather than being silently
    /// re-enabled). After provisioning it re-validates the whole risk config; on any defect
    /// the store is left unchanged (the caller commits only on `Ok`).
    ///
    /// A provisioned book is named for its id (ids are unique, so the name is unique) and
    /// carries no parent / desk / limits.
    ///
    /// # Errors
    /// The graph is malformed (dangling entry/edge, a cycle, a type-inconsistent condition),
    /// a leaf targets a book that already exists but is **disabled**, or auto-provisioning
    /// would violate a book invariant (e.g. a provisioned name collides with an existing
    /// book's name).
    pub fn install_risk_routing_graph(&mut self, graph: RiskRoutingGraph) -> Result<(), String> {
        // Snapshot so a validation failure leaves the store byte-for-byte unchanged
        // (mirrors [`create_risk_book`](Self::create_risk_book)'s roll-back discipline).
        let prev_books = self.risk_books.clone();
        let prev_graph = self.risk_routing_graph.clone();
        for node in graph.nodes.values() {
            if let RoutingNode::Book { risk_book_id } = node
                && self.risk_book(risk_book_id).is_none()
            {
                self.risk_books.push(RiskBookDef {
                    id: risk_book_id.clone(),
                    name: risk_book_id.clone(),
                    parent_id: None,
                    desk_id: None,
                    description: "Auto-provisioned from the risk-routing graph.".to_owned(),
                    limits: None,
                    enabled: true,
                });
            }
        }
        self.risk_routing_graph = Some(graph);
        // Validates BOTH the (now-extended) book registry and the graph against it — so a
        // graph naming a pre-existing DISABLED book is rejected here (never routed to).
        if let Err(e) = self.validate_risk_books() {
            self.risk_books = prev_books;
            self.risk_routing_graph = prev_graph;
            return Err(e);
        }
        Ok(())
    }

    /// Additively ensure a default ENABLED **"Firm Warehouse"** risk book + a default
    /// single-leaf routing graph that targets it exist on a **pristine** store, so an
    /// accepted / lifted fill routes into an enabled book and the per-book risk dashboard
    /// shows a row from first boot (§3.1/§8.2). Seeds ONLY when the store carries neither a
    /// risk book nor a routing graph — so a restart (or an operator who has begun defining
    /// their own books/graph) is never clobbered. Returns whether anything was seeded (the
    /// caller then persists), mirroring [`ensure_seed_registry`](Self::ensure_seed_registry).
    pub fn ensure_seed_risk_routing(&mut self) -> bool {
        // A pristine store only: never seed over any operator-defined book or graph.
        if !self.risk_books.is_empty() || self.risk_routing_graph.is_some() {
            return false;
        }
        self.risk_books.push(RiskBookDef {
            id: DEFAULT_WAREHOUSE_BOOK_ID.to_owned(),
            name: DEFAULT_WAREHOUSE_BOOK_NAME.to_owned(),
            parent_id: None,
            desk_id: None,
            description: "Default landing book for routed fills — every accepted / lifted \
                          position's risk lands here until finer risk books and a routing \
                          graph are configured."
                .to_owned(),
            limits: None,
            enabled: true,
        });
        self.risk_routing_graph = Some(default_risk_routing_graph(DEFAULT_WAREHOUSE_BOOK_ID));
        true
    }

    /// Additively ensure a default **auto-hedge / internalisation policy** (a warehouse-vs-
    /// hedge exit graph + a DV01 warehouse threshold on the default warehouse book) exists on
    /// a pristine hedge config, so a booked FI fill carries a real internalise decision on the
    /// deal blotter from first boot (§4/§6). Seeds ONLY when no hedge-policy graph AND no
    /// warehouse thresholds are configured yet AND the default warehouse book exists to bind
    /// the threshold to — so an operator who has begun configuring the hedge policy is never
    /// clobbered. The seeded graph warehouses green-band risk and sheds the over-cap overflow
    /// to an *advisory* external back-to-back (advisory-only is the config default), so nothing
    /// trades externally. Returns whether anything was seeded (the caller then persists).
    pub fn ensure_seed_hedge_policy(&mut self) -> bool {
        if self.hedge_policy_graph.is_some() || !self.hedge_thresholds.is_empty() {
            return false;
        }
        if self.risk_book(DEFAULT_WAREHOUSE_BOOK_ID).is_none() {
            return false;
        }
        self.hedge_policy_graph = Some(default_hedge_policy_graph());
        self.hedge_thresholds.push(ScopedThreshold {
            scope_id: DEFAULT_WAREHOUSE_BOOK_ID.to_owned(),
            def: default_warehouse_threshold_def(),
        });
        true
    }

    /// Additively seed the default **accept-all** acceptance graph on a pristine store, so
    /// `acceptance_graph()` returns a real (identity) policy from first boot — every inbound
    /// lift resolves to `Accept` and books exactly as before, leaving existing behaviour
    /// UNCHANGED until a trader writes rules. Seeds ONLY when no acceptance graph is
    /// configured yet (idempotent; an operator who has begun configuring it is never
    /// clobbered). Returns whether anything was seeded (the caller then persists).
    pub fn ensure_seed_acceptance(&mut self) -> bool {
        if self.acceptance_graph.is_some() {
            return false;
        }
        self.acceptance_graph = Some(default_accept_all_graph());
        true
    }

    // --- incoming-quote acceptance ------------------------------------------

    /// The firm-wide acceptance decision graph, if defined.
    #[must_use]
    pub fn acceptance_graph(&self) -> Option<&AcceptanceGraph> {
        self.acceptance_graph.as_ref()
    }

    /// Install (or replace) the firm-wide acceptance graph after validating it
    /// ([`check_acceptance_graph`](Self::check_acceptance_graph)). On any defect the store
    /// is left unchanged.
    ///
    /// # Errors
    /// The graph is malformed (dangling entry/edge, a cycle, a type-inconsistent condition).
    pub fn set_acceptance_graph(&mut self, graph: AcceptanceGraph) -> Result<(), String> {
        self.check_acceptance_graph(&graph)?;
        self.acceptance_graph = Some(graph);
        Ok(())
    }

    /// Validate an acceptance graph via the pure engine's
    /// [`AcceptanceGraph::validate`](celnet_acceptance::AcceptanceGraph::validate), mapping
    /// its collected defects into one human-readable message. An acceptance decision leaf
    /// carries no external registry target, so — unlike the hedge graph — no instrument / LP
    /// registry is consulted.
    ///
    /// # Errors
    /// A malformed graph (dangling entry/edge, cycle, type-inconsistent condition).
    pub fn check_acceptance_graph(&self, graph: &AcceptanceGraph) -> Result<(), String> {
        graph.validate().map_err(|errs| {
            let joined = errs
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ");
            format!("acceptance graph is invalid: {joined}")
        })
    }

    // --- auto-hedge policy / thresholds / config (Phase B) -------------------

    /// The firm-wide auto-hedge policy graph, if defined.
    #[must_use]
    pub fn hedge_policy_graph(&self) -> Option<&HedgeGraph> {
        self.hedge_policy_graph.as_ref()
    }

    /// Install (or replace) the firm-wide hedge policy graph after validating it against
    /// the aggregation instrument registry + FIX LP registry the store holds
    /// ([`check_hedge_policy_graph`](Self::check_hedge_policy_graph)). On any defect the
    /// store is left unchanged.
    ///
    /// # Errors
    /// The graph is malformed (dangling entry/edge, a cycle, a type-inconsistent
    /// condition) or an action leaf names an unknown instrument / LP.
    pub fn set_hedge_policy_graph(&mut self, graph: HedgeGraph) -> Result<(), String> {
        self.check_hedge_policy_graph(&graph)?;
        self.hedge_policy_graph = Some(graph);
        Ok(())
    }

    /// The set of instrument ids a `CrossInternal` leaf may target — the canonical
    /// instrument-reference-data registry (§6.1: the Agg Book crosses named instruments).
    #[must_use]
    fn known_hedge_instruments(&self) -> BTreeSet<String> {
        self.instruments
            .iter()
            .map(|i| i.instrument_id.clone())
            .collect()
    }

    /// The set of LP ids an `RfqOut` leaf may target — the FIX connection ids the store
    /// holds via the aggregated-book members (the external-liquidity venues wired in). Public
    /// so the boot / reconcile path can prime the rates store's hedge-policy snapshot with the
    /// live known-LP set the engine resolves an external action's target panel against.
    #[must_use]
    pub fn known_hedge_lps(&self) -> BTreeSet<String> {
        self.aggregated_books
            .iter()
            .flat_map(|b| b.member_connection_ids.iter().cloned())
            .collect()
    }

    /// Validate a hedge policy graph against the current registries: build the known
    /// instrument + LP sets and delegate to the pure engine's
    /// [`HedgeGraph::validate`](celnet_hedge_routing::HedgeGraph::validate), mapping its
    /// collected defects into one human-readable message.
    ///
    /// # Errors
    /// A malformed graph, or an action leaf naming an unknown instrument / LP.
    pub fn check_hedge_policy_graph(&self, graph: &HedgeGraph) -> Result<(), String> {
        let instruments = self.known_hedge_instruments();
        let lps = self.known_hedge_lps();
        graph.validate(&instruments, &lps).map_err(|errs| {
            let joined = errs
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ");
            format!("hedge policy graph is invalid: {joined}")
        })
    }

    /// The configured warehouse thresholds (§4), in insertion order.
    #[must_use]
    pub fn hedge_thresholds(&self) -> &[ScopedThreshold] {
        &self.hedge_thresholds
    }

    /// Upsert one warehouse threshold by scope (kind + id). A non-positive `cap` **removes**
    /// the matching threshold (the operator's delete gesture); otherwise it inserts or
    /// replaces the same-scope entry. Returns the full roster after the change.
    pub fn upsert_hedge_threshold(&mut self, entry: ScopedThreshold) -> &[ScopedThreshold] {
        self.hedge_thresholds.retain(|t| !t.same_scope(&entry));
        if entry.def.cap > 0.0 {
            self.hedge_thresholds.push(entry);
        }
        &self.hedge_thresholds
    }

    /// The auto-hedge engine config (§8.4).
    #[must_use]
    pub fn hedge_config(&self) -> &HedgeConfigDef {
        &self.hedge_config
    }

    /// Validate an engine config's **hedging LP panels** against the live known-LP
    /// registry: every `include` / `exclude` id a panel names must be a known LP, and a
    /// panel must not resolve to an empty effective set (all excluded). Delegates to the
    /// pure [`HedgeLpPanel::effective_lps`](celnet_hedge_routing::HedgeLpPanel::effective_lps)
    /// resolver (one source of truth), collecting every defect into one message.
    ///
    /// # Errors
    /// Any panel naming an unknown LP, or resolving to no LPs.
    pub fn check_hedge_lp_panels(&self, config: &HedgeConfigDef) -> Result<(), String> {
        let lps = self.known_hedge_lps();
        let mut msgs = Vec::new();
        for scoped in &config.lp_panels {
            if let Err(errs) = scoped.panel.effective_lps(&lps) {
                let joined = errs
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ");
                msgs.push(format!(
                    "LP panel for {} {:?}: {joined}",
                    scoped.scope_kind.label(),
                    scoped.scope_id
                ));
            }
        }
        if msgs.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "hedge LP panel config is invalid: {}",
                msgs.join("; ")
            ))
        }
    }

    /// Replace the auto-hedge engine config after validating its LP panels against the
    /// live known-LP registry ([`check_hedge_lp_panels`](Self::check_hedge_lp_panels)).
    /// On any defect the store is left unchanged.
    ///
    /// # Errors
    /// A panel naming an unknown LP, or resolving to an empty effective set.
    pub fn set_hedge_config(&mut self, config: HedgeConfigDef) -> Result<(), String> {
        self.check_hedge_lp_panels(&config)?;
        self.hedge_config = config;
        Ok(())
    }

    /// The persisted firm-wide pricing kill-switch setting.
    #[must_use]
    pub fn pricing_control(&self) -> PricingControlDef {
        self.pricing_control
    }

    /// Replace the persisted firm-wide pricing kill-switch setting (persisted through
    /// the usual atomic `save`; the runtime `PricingControl` is updated separately by
    /// the RPC handler).
    pub fn set_pricing_control(&mut self, def: PricingControlDef) {
        self.pricing_control = def;
    }

    /// The risk books strictly **above** `id` in the parent tree, immediate parent first
    /// then upward. Pure and cycle-safe (bounded by the registry, even on an as-yet
    /// unvalidated store). An unknown or top-level `id` yields an empty vec.
    #[must_use]
    pub fn risk_book_ancestors(&self, id: &str) -> Vec<&RiskBookDef> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        seen.insert(id.to_string());
        let mut cur = self.risk_book(id).and_then(|b| b.parent_id.clone());
        while let Some(pid) = cur {
            if !seen.insert(pid.clone()) {
                break; // cycle guard — a validated store is acyclic
            }
            match self.risk_book(&pid) {
                Some(b) => {
                    out.push(b);
                    cur = b.parent_id.clone();
                }
                None => break,
            }
        }
        out
    }

    /// The risk books strictly **below** `id` in the parent tree — every book whose
    /// ancestor chain contains `id`, in registry order. Pure and cycle-safe.
    #[must_use]
    pub fn risk_book_descendants(&self, id: &str) -> Vec<&RiskBookDef> {
        self.risk_books
            .iter()
            .filter(|b| b.id != id && self.risk_book_ancestors(&b.id).iter().any(|a| a.id == id))
            .collect()
    }

    /// Validate the whole risk config — the book registry **and** (when set) the routing
    /// graph — so a persisted store round-trips consistent. Called at [`load`](Self::load)
    /// and by every risk-book / graph write.
    ///
    /// Registry: every book passes [`check_risk_book`](Self::check_risk_book) (non-empty +
    /// unique name/id, existing + acyclic parent, existing owning desk, non-negative
    /// limits). Graph: [`check_risk_routing_graph`](Self::check_risk_routing_graph).
    ///
    /// # Errors
    /// The first book failing its invariants, or an invalid routing graph.
    fn validate_risk_books(&self) -> Result<(), String> {
        for b in &self.risk_books {
            self.check_risk_book(b)?;
        }
        if let Some(graph) = &self.risk_routing_graph {
            self.check_risk_routing_graph(graph)?;
        }
        if let Some(graph) = &self.hedge_policy_graph {
            self.check_hedge_policy_graph(graph)?;
        }
        if let Some(graph) = &self.acceptance_graph {
            self.check_acceptance_graph(graph)?;
        }
        Ok(())
    }

    /// Validate a single risk book's document-resolvable invariants (used by the whole-set
    /// pass and, transitively, every admin write). The book is assumed already present in
    /// `self.risk_books` (create/update push/replace before validating), so duplicate
    /// detection counts occurrences in the set.
    ///
    /// Rejects: an empty name; a duplicate id; a case-insensitively duplicate name; a
    /// `parent_id` that is the book itself, references a non-existent book, or forms a
    /// cycle (the book is its own ancestor); a `desk_id` referencing an unknown
    /// [`DeskDef`]; or a [`RiskLimits`] with a negative/non-finite cap.
    ///
    /// # Errors
    /// The first invariant the book violates.
    fn check_risk_book(&self, def: &RiskBookDef) -> Result<(), String> {
        if def.name.trim().is_empty() {
            return Err(format!("risk book {:?} has an empty name", def.id));
        }
        if self.risk_books.iter().filter(|b| b.id == def.id).count() > 1 {
            return Err(format!("duplicate risk book id {:?}", def.id));
        }
        if self
            .risk_books
            .iter()
            .filter(|b| b.name.eq_ignore_ascii_case(&def.name))
            .count()
            > 1
        {
            return Err(format!("duplicate risk book name {:?}", def.name));
        }
        if let Some(parent) = &def.parent_id {
            if parent == &def.id {
                return Err(format!("risk book {:?} cannot be its own parent", def.id));
            }
            if self.risk_book(parent).is_none() {
                return Err(format!(
                    "risk book {:?} references unknown parent {parent:?}",
                    def.id
                ));
            }
            // Walk the ancestor chain from the parent up; returning to `def.id` (or any
            // repeat) is a cycle. Bounded by the registry size.
            let mut seen = BTreeSet::new();
            seen.insert(def.id.clone());
            let mut cur = Some(parent.clone());
            while let Some(pid) = cur {
                if !seen.insert(pid.clone()) {
                    return Err(format!(
                        "risk book {:?} has a cyclic parent chain (through {pid:?})",
                        def.id
                    ));
                }
                cur = self.risk_book(&pid).and_then(|b| b.parent_id.clone());
            }
        }
        if let Some(desk) = &def.desk_id
            && self.desk(desk).is_none()
        {
            return Err(format!(
                "risk book {:?} references unknown desk {desk:?}",
                def.id
            ));
        }
        if let Some(limits) = &def.limits {
            limits
                .validate()
                .map_err(|e| format!("risk book {:?} {e}", def.id))?;
        }
        Ok(())
    }

    /// Validate a routing graph against the current registry: build the set of **enabled**
    /// risk-book ids and delegate to the pure engine's
    /// [`RiskRoutingGraph::validate`](celnet_risk_routing::RiskRoutingGraph::validate),
    /// mapping its collected [`RouteError`](celnet_risk_routing::RouteError)s into one
    /// human-readable message.
    ///
    /// **Routing-target policy:** a `Book` leaf may target **any existing enabled book**,
    /// parent or leaf — a parent aggregates its children's risk, so routing a fill to a
    /// parent (a coarser bucket) is legitimate; only *disabled* / non-existent ids are
    /// rejected. The engine's `validate` additionally guarantees every path terminates at
    /// a `Book`, so no separate default/reachability check is needed.
    ///
    /// # Errors
    /// A malformed graph, or a leaf targeting an unknown/disabled book.
    fn check_risk_routing_graph(&self, graph: &RiskRoutingGraph) -> Result<(), String> {
        let known: BTreeSet<String> = self
            .risk_books
            .iter()
            .filter(|b| b.enabled)
            .map(|b| b.id.clone())
            .collect();
        graph.validate(&known).map_err(|errs| {
            let joined = errs
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ");
            format!("risk routing graph is invalid: {joined}")
        })
    }
}

/// Argon2id-hash a plaintext password into a self-describing PHC string (salt and
/// parameters embedded), using a fresh 16-byte OS-CSPRNG salt.
///
/// # Errors
/// Returns a message if the OS CSPRNG or the hasher fails.
pub fn hash_password(plain: &str) -> Result<String, String> {
    let mut salt_bytes = [0u8; 16];
    getrandom::getrandom(&mut salt_bytes).map_err(|e| format!("csprng: {e}"))?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| format!("salt: {e}"))?;
    Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("hash: {e}"))
}

/// Verify a plaintext password against a stored Argon2 PHC hash in constant time.
/// A malformed stored hash verifies as `false` (never panics).
#[must_use]
pub fn verify_password(stored_hash: &str, candidate: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(candidate.as_bytes(), &parsed)
        .is_ok()
}

/// Mint a stable, unique, URL-safe id for a new user from its email local-part,
/// disambiguating against the existing set with a numeric suffix.
#[must_use]
pub fn mint_user_id(email: &str, existing: &[UserDef]) -> String {
    let base = slugify(email.split('@').next().unwrap_or(email));
    let base = if base.is_empty() {
        "user".to_string()
    } else {
        base
    };
    unique_id(&base, |cand| existing.iter().any(|u| u.id == cand))
}

/// Mint a stable, unique, URL-safe id for a new desk from its name.
#[must_use]
pub fn mint_desk_id(name: &str, existing: &[DeskDef]) -> String {
    let base = slugify(name);
    let base = if base.is_empty() {
        "desk".to_string()
    } else {
        base
    };
    unique_id(&base, |cand| existing.iter().any(|d| d.id == cand))
}

/// The bare (entity-unprefixed) reason a [`celnet_tiering::TieringConfig`] is invalid,
/// or `Ok` — the rule set behind the pricing-group pipeline validator
/// ([`validate_feature_pipeline`], for each embedded TIERING feature). The caller
/// prefixes the returned reason with its own entity context.
fn tiering_config_reason(cfg: &celnet_tiering::TieringConfig) -> Result<(), String> {
    use celnet_tiering::StrategySpec;
    let finite = |v: f64, what: &str| -> Result<(), String> {
        if v.is_finite() {
            Ok(())
        } else {
            Err(format!("{what} must be finite"))
        }
    };
    let finite_nonneg = |v: f64, what: &str| -> Result<(), String> {
        if v.is_finite() && v >= 0.0 {
            Ok(())
        } else {
            Err(format!("{what} must be finite and non-negative"))
        }
    };
    cfg.guardrails
        .validate()
        .map_err(|e| format!("guardrails are inconsistent ({:?})", e.reason))?;
    if cfg.strategies.is_empty() {
        return Err("enables no strategy (omit the tiering block to disable tiering)".to_string());
    }
    for s in &cfg.strategies {
        match *s {
            StrategySpec::FlatMarkup { half_spread } => {
                finite_nonneg(half_spread, "flat-markup half_spread")?;
            }
            StrategySpec::InventorySkew {
                half_spread,
                kappa,
                s_max,
            } => {
                finite_nonneg(half_spread, "inventory-skew half_spread")?;
                finite(kappa, "inventory-skew kappa")?;
                finite_nonneg(s_max, "inventory-skew s_max")?;
            }
            StrategySpec::ScaledSmoothedSpread {
                smoothing_weight,
                expected_spread,
                max_divergence,
                core_spread,
                max_output_spread,
                spread_scale_factor,
            } => {
                // Smoothing Weight w ∈ (0, 1].
                if !(smoothing_weight.is_finite()
                    && smoothing_weight > 0.0
                    && smoothing_weight <= 1.0)
                {
                    return Err("scaled-smoothed smoothing_weight must be in (0, 1]".to_string());
                }
                // Expected Spread e > 0 (it is the divergence denominator).
                if !(expected_spread.is_finite() && expected_spread > 0.0) {
                    return Err(
                        "scaled-smoothed expected_spread must be finite and > 0".to_string()
                    );
                }
                finite_nonneg(max_divergence, "scaled-smoothed max_divergence")?;
                finite_nonneg(core_spread, "scaled-smoothed core_spread")?;
                finite_nonneg(max_output_spread, "scaled-smoothed max_output_spread")?;
                finite_nonneg(spread_scale_factor, "scaled-smoothed spread_scale_factor")?;
                // Max Output Spread m must be able to contain the Core spread c.
                if max_output_spread < core_spread {
                    return Err(
                        "scaled-smoothed max_output_spread must be >= core_spread".to_string()
                    );
                }
            }
        }
    }
    Ok(())
}

/// Validate a pricing-group outbound [`FeaturePipeline`](celnet_tiering::FeaturePipeline)
/// (`mode` = `"ESP"` or `"RFQ"`) at load and at every admin write: the closing
/// guardrails must be internally consistent, every embedded **TIERING** feature's
/// [`TieringConfig`](celnet_tiering::TieringConfig) must pass the shared tiering rule
/// set ([`tiering_config_reason`]), and every other feature's magnitudes must be finite
/// (a NaN offset would silently corrupt every outbound price the pipeline produces).
///
/// # Errors
/// The first inconsistent guardrail, invalid embedded tiering config, or non-finite
/// feature magnitude, as a human-readable message.
fn validate_feature_pipeline(
    group: &str,
    mode: &str,
    pipeline: &celnet_tiering::FeaturePipeline,
) -> Result<(), String> {
    use celnet_tiering::PricingFeature;
    let ctx = |m: String| format!("pricing group {group:?} {mode} pipeline {m}");
    let finite = |v: f64, what: &str| -> Result<(), String> {
        if v.is_finite() {
            Ok(())
        } else {
            Err(format!("{what} must be finite"))
        }
    };
    let finite_nonneg = |v: f64, what: &str| -> Result<(), String> {
        if v.is_finite() && v >= 0.0 {
            Ok(())
        } else {
            Err(format!("{what} must be finite and non-negative"))
        }
    };
    pipeline
        .guardrails
        .validate()
        .map_err(|e| ctx(format!("guardrails are inconsistent ({:?})", e.reason)))?;
    for feature in &pipeline.features {
        match feature {
            PricingFeature::MidShift {
                shift, reference, ..
            } => {
                finite(*shift, "mid-shift shift").map_err(ctx)?;
                if let Some(r) = reference {
                    finite(*r, "mid-shift reference").map_err(ctx)?;
                }
            }
            PricingFeature::Tiering { config } => {
                tiering_config_reason(config).map_err(|m| ctx(format!("tiering {m}")))?;
            }
            PricingFeature::Axe { magnitude, .. } => {
                finite_nonneg(*magnitude, "axe magnitude").map_err(ctx)?;
            }
            PricingFeature::Position { kappa, s_max, .. } => {
                finite(*kappa, "position kappa").map_err(ctx)?;
                finite_nonneg(*s_max, "position s_max").map_err(ctx)?;
            }
            PricingFeature::PanicSkew { skew, .. } => {
                finite(*skew, "panic-skew skew").map_err(ctx)?;
            }
        }
    }
    Ok(())
}

/// Mint a stable, unique, URL-safe id for a new pricing group from its name,
/// disambiguating against the existing set with a numeric suffix (mirrors
/// [`mint_aggregated_book_id`]).
#[must_use]
pub fn mint_pricing_group_id(name: &str, existing: &[PricingGroupDef]) -> String {
    let base = slugify(name);
    let base = if base.is_empty() {
        "pricing-group".to_string()
    } else {
        base
    };
    unique_id(&base, |cand| existing.iter().any(|g| g.id == cand))
}

/// Build the default single-leaf risk-routing graph whose only terminal routes **every**
/// fill to `book_id` — the out-of-the-box graph seeded beside the default warehouse book
/// ([`IdentityStore::ensure_seed_risk_routing`]) so a lifted fill lands in an enabled book
/// from first boot. A one-node graph (`entry → Book{book_id}`) is trivially acyclic and
/// terminates, so it validates against any registry that carries `book_id` enabled.
#[must_use]
pub fn default_risk_routing_graph(book_id: &str) -> RiskRoutingGraph {
    let mut nodes = std::collections::BTreeMap::new();
    nodes.insert(
        0u32,
        RoutingNode::Book {
            risk_book_id: book_id.to_owned(),
        },
    );
    RiskRoutingGraph { entry: 0, nodes }
}

/// Build the default **auto-hedge / internalisation** exit graph seeded on a pristine store
/// ([`IdentityStore::ensure_seed_hedge_policy`]): warehouse (hold) while the book's risk sits
/// in the green band (`breached == false`), and shed the over-cap overflow to an *advisory*
/// external back-to-back (`SUBMIT_MARKET_ORDER`) once the red band fires. Names no instrument
/// / LP (so it validates against any registry), is acyclic, and every path terminates.
#[must_use]
pub fn default_hedge_policy_graph() -> HedgeGraph {
    use celnet_hedge_routing::{
        ExecStyle, ExitAction, HedgeField, HedgeNode, HedgeSize, RouteOp, RouteValue,
    };
    let mut nodes = std::collections::BTreeMap::new();
    nodes.insert(
        0u32,
        HedgeNode::Condition {
            field: HedgeField::Breached,
            op: RouteOp::Eq,
            value: RouteValue::Text("false".to_owned()),
            on_true: 1,
            on_false: 2,
        },
    );
    nodes.insert(
        1u32,
        HedgeNode::Action {
            exit: ExitAction::Warehouse,
        },
    );
    nodes.insert(
        2u32,
        HedgeNode::Action {
            exit: ExitAction::SubmitMarketOrder {
                size: HedgeSize::Overflow,
                style: ExecStyle::Immediate,
            },
        },
    );
    HedgeGraph { entry: 0, nodes }
}

/// The default firm **warehouse threshold** — a DV01 cap at [`DEFAULT_WAREHOUSE_DV01_CAP`]
/// with amber/red bands at 80% / 90% utilisation and an 80% target shed fraction. Single-
/// sourced so both the pristine-store seed ([`IdentityStore::ensure_seed_hedge_policy`], which
/// binds it to the default warehouse book) AND the booking-path runtime fallback
/// (`RatesPositionStore::stamp_internalise`, when a fill routes into a book with no configured
/// threshold) measure against the SAME warehouse budget — so every booked fill carries an
/// internalise decision + RAG band, never an empty `—`, regardless of which risk book it lands
/// in. `max_clip` is `f64::INFINITY` (a single fill is never held back), which the persisted
/// seed serialises through the `nonfinite_f64` module; the runtime fallback is never persisted.
#[must_use]
pub fn default_warehouse_threshold_def() -> HedgeThresholdDef {
    HedgeThresholdDef {
        scope_kind: HedgeScopeKind::Book,
        metric: HedgeMetric::Dv01,
        cap: DEFAULT_WAREHOUSE_DV01_CAP,
        amber: 0.8,
        red: 0.9,
        target_fraction: 0.8,
        min_clip: 0.0,
        max_clip: f64::INFINITY,
        ramped: false,
        ramp_k: 1.0,
    }
}

/// Mint a stable, unique, URL-safe id for a new risk book from its name, disambiguating
/// against the existing set with a numeric suffix (mirrors [`mint_aggregated_book_id`]).
#[must_use]
pub fn mint_risk_book_id(name: &str, existing: &[RiskBookDef]) -> String {
    let base = slugify(name);
    let base = if base.is_empty() {
        "risk-book".to_string()
    } else {
        base
    };
    unique_id(&base, |cand| existing.iter().any(|b| b.id == cand))
}

/// Mint a stable, unique, URL-safe id for a new aggregated book from its name,
/// disambiguating against the existing set with a numeric suffix (mirrors
/// [`mint_desk_id`]).
#[must_use]
pub fn mint_aggregated_book_id(name: &str, existing: &[AggregatedBookDef]) -> String {
    let base = slugify(name);
    let base = if base.is_empty() {
        "aggregated-book".to_string()
    } else {
        base
    };
    unique_id(&base, |cand| existing.iter().any(|b| b.id == cand))
}

/// Lowercase, replace any run of non-alphanumeric chars with a single `-`, and
/// trim leading/trailing `-`.
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = false;
    for ch in s.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

/// Return `base`, or `base-2`, `base-3`, … — the first that `taken` rejects.
fn unique_id(base: &str, mut taken: impl FnMut(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|cand| !taken(cand))
        .unwrap_or_else(|| base.to_string())
}

/// `path` with `.tmp` appended to its file name (a sibling temp for atomic save).
fn tmp_sibling(path: &Path) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_token_round_trips() {
        assert_eq!(Role::parse("ADMIN"), Some(Role::Admin));
        assert_eq!(Role::parse("trader"), Some(Role::Trader));
        assert_eq!(Role::parse("root"), None);
        assert_eq!(Role::Admin.as_str(), "admin");
        assert!(Role::Admin.is_admin());
        assert!(!Role::Trader.is_admin());
    }

    #[test]
    fn hash_is_salted_and_verifies() {
        let h1 = hash_password("hunter2").unwrap();
        let h2 = hash_password("hunter2").unwrap();
        // Distinct salts ⇒ distinct hashes for the same password.
        assert_ne!(
            h1, h2,
            "salting must make identical passwords hash differently"
        );
        assert!(verify_password(&h1, "hunter2"));
        assert!(verify_password(&h2, "hunter2"));
        assert!(!verify_password(&h1, "wrong"));
        // A malformed stored hash never panics and never verifies.
        assert!(!verify_password("not-a-phc-string", "hunter2"));
    }

    #[test]
    fn seed_admin_is_idempotent() {
        let mut store = IdentityStore::default();
        assert!(store.ensure_seed_admin().unwrap(), "first call seeds");
        assert_eq!(store.users.len(), 1);
        let admin = &store.users[0];
        assert_eq!(admin.email, SEED_ADMIN_EMAIL);
        assert!(admin.role.is_admin());
        assert!(
            admin.verify(SEED_ADMIN_PASSWORD),
            "seed admin can authenticate"
        );
        assert!(!store.password_hash_is_plaintext());
        // A second call is a no-op (an admin already exists).
        assert!(
            !store.ensure_seed_admin().unwrap(),
            "second call is idempotent"
        );
        assert_eq!(store.users.len(), 1);
    }

    #[test]
    fn disabled_user_never_verifies() {
        let mut admin = UserDef {
            id: "a".into(),
            email: "a@celnet.com".into(),
            display_name: "A".into(),
            role: Role::Trader,
            desk_ids: Vec::new(),
            all_desks: false,
            password_hash: hash_password("pw").unwrap(),
            disabled: false,
            capability_grants: Vec::new(),
            capability_denies: Vec::new(),
        };
        assert!(admin.verify("pw"));
        admin.disabled = true;
        assert!(
            !admin.verify("pw"),
            "a disabled account must reject even a correct password"
        );
    }

    #[test]
    fn email_lookup_is_case_insensitive() {
        let mut store = IdentityStore::default();
        store.ensure_seed_admin().unwrap();
        assert!(store.user_by_email("ADMIN@CELNET.COM").is_some());
        assert!(store.user_by_email("nobody@celnet.com").is_none());
    }

    #[test]
    fn ids_are_minted_unique() {
        let users = vec![UserDef {
            id: "jane".into(),
            email: "jane@celnet.com".into(),
            display_name: "Jane".into(),
            role: Role::Trader,
            desk_ids: Vec::new(),
            all_desks: false,
            password_hash: "x".into(),
            disabled: false,
            capability_grants: Vec::new(),
            capability_denies: Vec::new(),
        }];
        // Same local-part ⇒ disambiguated.
        assert_eq!(mint_user_id("jane@other.com", &users), "jane-2");
        assert_eq!(mint_user_id("new.trader@celnet.com", &users), "new-trader");

        let desks = vec![DeskDef {
            id: "g10-options".into(),
            name: "G10 Options".into(),
            books: Vec::new(),
        }];
        assert_eq!(mint_desk_id("G10 Options", &desks), "g10-options-2");
        assert_eq!(mint_desk_id("EM Vol", &desks), "em-vol");
    }

    #[test]
    fn json_round_trips_through_store() {
        let mut store = IdentityStore::default();
        store.ensure_seed_admin().unwrap();
        store.desks.push(DeskDef {
            id: "g10".into(),
            name: "G10".into(),
            books: Vec::new(),
        });
        let bytes = serde_json::to_vec(&store).unwrap();
        let back: IdentityStore = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(store, back);
    }

    // --- pricing groups (Phase 2a) -----------------------------------------

    /// A valid, empty (no-feature) ESP/RFQ pipeline with consistent guardrails — the
    /// resolver/validation tests exercise membership and determinism, not the feature
    /// arithmetic (that is oracle-tested at the hub in `services::aggregation`).
    fn ok_pipeline() -> celnet_tiering::FeaturePipeline {
        celnet_tiering::FeaturePipeline::new(
            Vec::new(),
            celnet_tiering::Guardrails::new(0.0, 1.0, 1.0, 1e-6),
        )
    }

    fn group(
        id: &str,
        name: &str,
        conns: &[&str],
        users: &[&str],
        desks: &[&str],
        enabled: bool,
    ) -> PricingGroupDef {
        PricingGroupDef {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            member_connection_ids: conns.iter().map(|s| (*s).to_string()).collect(),
            member_user_ids: users.iter().map(|s| (*s).to_string()).collect(),
            member_desks: desks.iter().map(|s| (*s).to_string()).collect(),
            esp_pipeline: ok_pipeline(),
            rfq_pipeline: ok_pipeline(),
            share_pipeline: false,
            pricing_source_mode: PricingSourceMode::default(),
            book_skew_weight: DEFAULT_BOOK_SKEW_WEIGHT,
            last_look_mode: LastLookMode::default(),
            last_look_tolerance_bps: DEFAULT_LAST_LOOK_TOLERANCE_BPS,
            async_giveback_pct: DEFAULT_ASYNC_GIVEBACK_PCT,
            enabled,
        }
    }

    fn trader(id: &str, desks: &[&str]) -> UserDef {
        UserDef {
            id: id.into(),
            email: format!("{id}@celnet.com"),
            display_name: id.into(),
            role: Role::Trader,
            desk_ids: desks.iter().map(|s| (*s).to_string()).collect(),
            all_desks: false,
            password_hash: "x".into(),
            disabled: false,
            capability_grants: Vec::new(),
            capability_denies: Vec::new(),
        }
    }

    #[test]
    fn resolver_resolves_connection_user_and_desk_fallback() {
        let mut store = IdentityStore::default();
        store.desks.push(DeskDef {
            id: "emea".into(),
            name: "EMEA".into(),
            books: Vec::new(),
        });
        store.users.push(trader("alice", &["emea"]));
        // One enabled group binds connection `conn-1`, user `alice`, and desk `emea`.
        store.pricing_groups.push(group(
            "ga",
            "GROUP-A",
            &["conn-1"],
            &["alice"],
            &["emea"],
            true,
        ));
        // A disabled group must never resolve (its members fall through).
        store
            .pricing_groups
            .push(group("gb", "GROUP-B", &["conn-2"], &[], &[], false));
        store.validate_pricing_groups().expect("valid");

        let r = store.pricing_group_resolver();
        // Direct connection-id hit.
        assert_eq!(
            r.resolve_for_connection("conn-1", None)
                .map(|g| g.id.as_str()),
            Some("ga")
        );
        // Direct user-id hit.
        assert_eq!(
            r.resolve_for_user("alice", &[]).map(|g| g.id.as_str()),
            Some("ga")
        );
        // Connection desk fallback: an unknown connection whose desk is `emea`.
        assert_eq!(
            r.resolve_for_connection("conn-x", Some("emea"))
                .map(|g| g.id.as_str()),
            Some("ga")
        );
        // User desk fallback: an unknown user whose desk membership includes `emea`.
        assert_eq!(
            r.resolve_for_user("bob", &["emea".to_string()])
                .map(|g| g.id.as_str()),
            Some("ga")
        );
        // No connection, no matching desk ⇒ no group.
        assert!(r.resolve_for_connection("conn-x", Some("apac")).is_none());
        assert!(r.resolve_for_user("bob", &[]).is_none());
        // A disabled group's member does not resolve.
        assert!(r.resolve_for_connection("conn-2", None).is_none());
    }

    #[test]
    fn validation_rejects_member_in_two_enabled_groups() {
        let mut store = IdentityStore::default();
        store
            .pricing_groups
            .push(group("ga", "GROUP-A", &["conn-1"], &[], &[], true));
        store
            .pricing_groups
            .push(group("gb", "GROUP-B", &["conn-1"], &[], &[], true));
        let err = store.validate_pricing_groups().unwrap_err();
        assert!(
            err.contains("two enabled pricing groups"),
            "unexpected error: {err}"
        );
        // Disabling one group removes the determinism conflict.
        store.pricing_groups[1].enabled = false;
        store
            .validate_pricing_groups()
            .expect("no longer conflicting");
    }

    #[test]
    fn validation_rejects_unknown_member_and_bad_pipeline() {
        // An unknown user member is rejected (users live in this document).
        let mut store = IdentityStore::default();
        store
            .pricing_groups
            .push(group("ga", "GROUP-A", &[], &["ghost"], &[], true));
        assert!(
            store
                .validate_pricing_groups()
                .unwrap_err()
                .contains("unknown user member")
        );

        // An internally inconsistent pipeline guardrail (h_min > h_max) is rejected via
        // the reused tiering guardrail validation.
        let mut store2 = IdentityStore::default();
        let mut g = group("gb", "GROUP-B", &[], &[], &[], true);
        g.esp_pipeline = celnet_tiering::FeaturePipeline::new(
            Vec::new(),
            celnet_tiering::Guardrails::new(1.0, 0.0, 1.0, 1e-6),
        );
        store2.pricing_groups.push(g);
        assert!(
            store2
                .validate_pricing_groups()
                .unwrap_err()
                .contains("guardrails are inconsistent")
        );
    }

    #[test]
    fn shared_pipeline_ignores_the_unused_rfq_pipeline_guardrails() {
        // Regression (GOLD_TIERING): with share_pipeline set the RFQ pipeline is unused
        // (rfq_effective returns ESP), so a default/inconsistent RFQ block must NOT reject
        // the group — previously it did ("RFQ pipeline guardrails are inconsistent").
        let mut store = IdentityStore::default();
        let mut g = group("gs", "GOLD-TIERING", &[], &[], &[], true);
        g.esp_pipeline = celnet_tiering::FeaturePipeline::new(
            Vec::new(),
            celnet_tiering::Guardrails::new(0.0, 1.0, 0.5, 0.01), // valid ESP guardrails
        );
        g.rfq_pipeline = celnet_tiering::FeaturePipeline::new(
            Vec::new(),
            celnet_tiering::Guardrails::new(0.0, 0.0, 0.0, 0.0), // inconsistent, but IGNORED
        );
        g.share_pipeline = true;
        store.pricing_groups.push(g);
        store
            .validate_pricing_groups()
            .expect("a shared pipeline must not validate the unused RFQ block");

        // With sharing OFF the same RFQ block IS used, and IS rejected.
        store.pricing_groups[0].share_pipeline = false;
        assert!(
            store
                .validate_pricing_groups()
                .unwrap_err()
                .contains("guardrails are inconsistent")
        );
    }

    #[test]
    fn pricing_group_json_round_trips() {
        let mut store = IdentityStore::default();
        store.desks.push(DeskDef {
            id: "emea".into(),
            name: "EMEA".into(),
            books: Vec::new(),
        });
        store.users.push(trader("alice", &["emea"]));
        store.pricing_groups.push(group(
            "ga",
            "GROUP-A",
            &["conn-1"],
            &["alice"],
            &["emea"],
            true,
        ));
        store.validate_pricing_groups().expect("valid");
        let bytes = serde_json::to_vec(&store).unwrap();
        let back: IdentityStore = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(store, back);
        // The resolver rebuilds identically from the reloaded store.
        assert_eq!(
            back.pricing_group_resolver()
                .resolve_for_user("alice", &[])
                .map(|g| g.id.as_str()),
            Some("ga")
        );
    }

    /// Non-default market-data last-look settings survive a save → reload (restart
    /// persistence): the enum as its `snake_case` token, the tolerance and giveback as
    /// numbers, all reload byte-for-byte and the reloaded store re-validates.
    #[test]
    fn pricing_group_last_look_config_persists_across_reload() {
        let mut store = IdentityStore::default();
        let mut g = group("ll", "GROUP-LL", &["conn-ll"], &[], &[], true);
        g.last_look_mode = LastLookMode::Async;
        g.last_look_tolerance_bps = 2.5;
        g.async_giveback_pct = 40.0;
        store.pricing_groups.push(g);
        store
            .validate_pricing_groups()
            .expect("valid last-look config");

        let json = serde_json::to_string(&store).unwrap();
        // The enum persists as its lowercase wire token, not an int.
        assert!(json.contains("\"last_look_mode\":\"async\""));
        let back: IdentityStore = serde_json::from_str(&json).unwrap();
        assert_eq!(store, back, "the whole store reloads identically");
        let g = &back.pricing_groups[0];
        assert_eq!(g.last_look_mode, LastLookMode::Async);
        assert!((g.last_look_tolerance_bps - 2.5).abs() < 1e-12);
        assert!((g.async_giveback_pct - 40.0).abs() < 1e-12);
    }

    /// A pre-policy `identity.json` (no `pricing_source_mode` / `book_skew_weight` keys)
    /// loads at the platform defaults — composite-first + a 0.5 skew weight — so an
    /// existing config behaves identically to before (additive serde-default contract).
    #[test]
    fn pricing_group_pre_policy_json_loads_at_defaults() {
        let legacy = r#"{
            "pricing_groups": [{
                "id": "ga", "name": "GROUP-A", "member_connection_ids": ["conn-1"],
                "esp_pipeline": {"features": [], "guardrails":
                    {"h_min": 0.0, "h_max": 5.0, "s_max": 2.0, "spread_floor": 0.01}},
                "rfq_pipeline": {"features": [], "guardrails":
                    {"h_min": 0.0, "h_max": 5.0, "s_max": 2.0, "spread_floor": 0.01}},
                "share_pipeline": true, "enabled": true
            }]
        }"#;
        let store: IdentityStore = serde_json::from_str(legacy).unwrap();
        let g = &store.pricing_groups[0];
        assert_eq!(
            g.pricing_source_mode,
            PricingSourceMode::CompositeFirstCurveFallback
        );
        assert!((g.book_skew_weight - DEFAULT_BOOK_SKEW_WEIGHT).abs() < 1e-12);
        // The market-data last-look fields also default (Sync / 1bp / 50%).
        assert_eq!(g.last_look_mode, LastLookMode::Sync);
        assert!((g.last_look_tolerance_bps - DEFAULT_LAST_LOOK_TOLERANCE_BPS).abs() < 1e-12);
        assert!((g.async_giveback_pct - DEFAULT_ASYNC_GIVEBACK_PCT).abs() < 1e-12);
        // And it round-trips through the enum's snake_case token on re-serialize.
        let re = serde_json::to_string(&store.pricing_groups[0]).unwrap();
        assert!(re.contains("\"composite_first_curve_fallback\""));
        assert!(re.contains("\"sync\""));
    }

    /// The market-data last-look shape guards: a negative tolerance or an out-of-range
    /// giveback percentage is rejected at the admin write / at load, so a bad last-look
    /// config never reaches the hot path.
    #[test]
    fn pricing_group_rejects_bad_last_look_config() {
        let mut neg_tol = IdentityStore::default();
        let mut g = group("ga", "GROUP-A", &["conn-1"], &[], &[], true);
        g.last_look_tolerance_bps = -1.0;
        neg_tol.pricing_groups.push(g);
        assert!(
            neg_tol
                .validate_pricing_groups()
                .unwrap_err()
                .contains("last_look_tolerance_bps")
        );

        let mut bad_pct = IdentityStore::default();
        let mut g = group("gb", "GROUP-B", &["conn-2"], &[], &[], true);
        g.async_giveback_pct = 150.0;
        bad_pct.pricing_groups.push(g);
        assert!(
            bad_pct
                .validate_pricing_groups()
                .unwrap_err()
                .contains("async_giveback_pct")
        );
    }

    /// A `book_skew_weight` outside `[0, 1]` (or non-finite) is rejected at the admin write
    /// and at load — a bad blend weight fails loudly, never silently clamped on the hot path.
    #[test]
    fn pricing_group_rejects_out_of_range_skew_weight() {
        let mut store = IdentityStore::default();
        let mut g = group("ga", "GROUP-A", &["conn-1"], &[], &[], true);
        g.book_skew_weight = 1.5;
        store.pricing_groups.push(g);
        assert!(
            store
                .validate_pricing_groups()
                .unwrap_err()
                .contains("book_skew_weight")
        );
    }

    // --- risk books + routing graph (FI risk routing, Phase 2) -------------

    fn rb_edit(name: &str, parent: Option<&str>, desk: Option<&str>) -> RiskBookEdit {
        RiskBookEdit {
            name: name.into(),
            parent_id: parent.map(str::to_string),
            desk_id: desk.map(str::to_string),
            description: String::new(),
            limits: None,
            enabled: true,
        }
    }

    /// A minimal valid decision graph both of whose `Book` leaves target one book id.
    fn cond_graph(target_book_id: &str) -> RiskRoutingGraph {
        use celnet_risk_routing::{RouteField, RouteOp, RouteValue};
        let mut nodes = BTreeMap::new();
        nodes.insert(
            0u32,
            RoutingNode::Condition {
                field: RouteField::Ccy,
                op: RouteOp::Eq,
                value: RouteValue::Text("EUR".into()),
                on_true: 1,
                on_false: 2,
            },
        );
        nodes.insert(
            1u32,
            RoutingNode::Book {
                risk_book_id: target_book_id.into(),
            },
        );
        nodes.insert(
            2u32,
            RoutingNode::Book {
                risk_book_id: target_book_id.into(),
            },
        );
        RiskRoutingGraph { entry: 0, nodes }
    }

    #[test]
    fn risk_book_crud_round_trip() {
        let mut store = IdentityStore::default();
        store.desks.push(DeskDef {
            id: "emea".into(),
            name: "EMEA".into(),
            books: Vec::new(),
        });
        let created = store
            .create_risk_book(rb_edit("Global Macro", None, Some("emea")))
            .expect("created");
        assert_eq!(created.id, "global-macro");
        assert!(store.risk_book("global-macro").is_some());

        // Persist + reload (serde round-trip) leaves the store equal and the book present.
        let bytes = serde_json::to_vec(&store).unwrap();
        let back: IdentityStore = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(store, back);
        assert!(back.risk_book("global-macro").is_some());

        // Update preserves the id and applies the new name.
        let upd = store
            .update_risk_book("global-macro", rb_edit("Global Rates", None, Some("emea")))
            .expect("updated");
        assert_eq!(upd.id, "global-macro");
        assert_eq!(
            store.risk_book("global-macro").unwrap().name,
            "Global Rates"
        );
    }

    #[test]
    fn risk_book_name_and_id_collision_rejected() {
        let mut store = IdentityStore::default();
        store
            .create_risk_book(rb_edit("Alpha", None, None))
            .unwrap();
        // Same name (case-insensitive) is rejected; the tentative book is rolled back.
        let err = store
            .create_risk_book(rb_edit("alpha", None, None))
            .unwrap_err();
        assert!(err.contains("duplicate risk book name"), "got: {err}");
        assert_eq!(store.risk_books.len(), 1);

        // A duplicate id (injected directly) is caught by whole-set validation.
        store.risk_books.push(RiskBookDef {
            id: "alpha".into(),
            name: "Other".into(),
            parent_id: None,
            desk_id: None,
            description: String::new(),
            limits: None,
            enabled: true,
        });
        assert!(
            store
                .validate_risk_books()
                .unwrap_err()
                .contains("duplicate risk book id")
        );
    }

    #[test]
    fn risk_book_cyclic_parent_rejected() {
        let mut store = IdentityStore::default();
        store.create_risk_book(rb_edit("A", None, None)).unwrap(); // id "a"
        store
            .create_risk_book(rb_edit("B", Some("a"), None))
            .unwrap(); // id "b", parent a
        // Re-parent A onto B ⇒ a→b→a cycle.
        let err = store
            .update_risk_book("a", rb_edit("A", Some("b"), None))
            .unwrap_err();
        assert!(err.contains("cyclic parent chain"), "got: {err}");
        // A self-parent is rejected with a dedicated message.
        let err2 = store
            .update_risk_book("a", rb_edit("A", Some("a"), None))
            .unwrap_err();
        assert!(err2.contains("its own parent"), "got: {err2}");
    }

    #[test]
    fn risk_book_unknown_parent_and_desk_rejected() {
        let mut store = IdentityStore::default();
        assert!(
            store
                .create_risk_book(rb_edit("A", Some("ghost"), None))
                .unwrap_err()
                .contains("unknown parent")
        );
        assert!(
            store
                .create_risk_book(rb_edit("A", None, Some("no-desk")))
                .unwrap_err()
                .contains("unknown desk")
        );
        assert!(store.risk_books.is_empty(), "rejected books roll back");
    }

    #[test]
    fn risk_book_negative_and_nonfinite_limit_rejected() {
        let mut store = IdentityStore::default();
        let mut edit = rb_edit("A", None, None);
        edit.limits = Some(RiskLimits {
            max_net_notional: Some(-1.0),
            ..Default::default()
        });
        assert!(
            store
                .create_risk_book(edit)
                .unwrap_err()
                .contains("max_net_notional must be finite and non-negative")
        );
        let mut edit2 = rb_edit("A", None, None);
        edit2.limits = Some(RiskLimits {
            max_dv01: Some(f64::NAN),
            ..Default::default()
        });
        assert!(
            store
                .create_risk_book(edit2)
                .unwrap_err()
                .contains("max_dv01")
        );

        // A fully-populated, valid limit set is accepted.
        let mut edit3 = rb_edit("A", None, None);
        edit3.limits = Some(RiskLimits {
            max_net_notional: Some(1e9),
            max_gross_notional: Some(2e9),
            max_dv01: Some(5e5),
        });
        assert!(store.create_risk_book(edit3).is_ok());
    }

    #[test]
    fn risk_routing_graph_accepts_known_and_rejects_unknown_book() {
        let mut store = IdentityStore::default();
        let a = store
            .create_risk_book(rb_edit("Alpha", None, None))
            .unwrap();
        // A graph targeting the enabled book is accepted and stored.
        store
            .set_risk_routing_graph(cond_graph(&a.id))
            .expect("valid graph");
        assert!(store.risk_routing_graph().is_some());

        // A graph targeting an unknown book surfaces RouteError::UnknownBook.
        let err = store
            .set_risk_routing_graph(cond_graph("ghost"))
            .unwrap_err();
        assert!(
            err.contains("unknown risk book") && err.contains("ghost"),
            "got: {err}"
        );

        // Disabling the only routed-to book makes the stored graph invalid at the whole-
        // config re-validation an update runs ⇒ the update is refused and rolled back.
        let err2 = store
            .update_risk_book(
                &a.id,
                RiskBookEdit {
                    enabled: false,
                    ..rb_edit("Alpha", None, None)
                },
            )
            .unwrap_err();
        assert!(
            err2.contains("risk routing graph is invalid"),
            "got: {err2}"
        );
        assert!(
            store.risk_book(&a.id).unwrap().enabled,
            "update rolled back"
        );
    }

    #[test]
    fn seed_risk_routing_is_pristine_only_and_idempotent() {
        // A pristine store seeds the default enabled warehouse book + a default graph that
        // routes every fill to it.
        let mut store = IdentityStore::default();
        assert!(store.ensure_seed_risk_routing(), "pristine store seeds");
        let seeded = store
            .risk_book(DEFAULT_WAREHOUSE_BOOK_ID)
            .expect("default warehouse book seeded");
        assert!(seeded.enabled, "the default book is enabled (routable)");
        assert_eq!(seeded.name, DEFAULT_WAREHOUSE_BOOK_NAME);
        let graph = store.risk_routing_graph().expect("default graph seeded");
        assert!(
            graph.nodes.values().any(|n| matches!(
                n,
                RoutingNode::Book { risk_book_id } if risk_book_id == DEFAULT_WAREHOUSE_BOOK_ID
            )),
            "the default graph routes to the default warehouse book",
        );
        // The seed passes the whole-config validation (enabled target, acyclic, terminates).
        store.validate_risk_books().expect("seed is valid");

        // Idempotent: a second call seeds nothing.
        assert!(!store.ensure_seed_risk_routing(), "already-seeded ⇒ no-op");
        assert_eq!(store.risk_books.len(), 1, "no duplicate default book");

        // Never seeds over an operator who already defined a book (no graph yet).
        let mut with_book = IdentityStore::default();
        with_book
            .create_risk_book(rb_edit("Alpha", None, None))
            .unwrap();
        assert!(
            !with_book.ensure_seed_risk_routing(),
            "a store with an operator book is not pristine ⇒ no seed",
        );
        assert!(with_book.risk_book(DEFAULT_WAREHOUSE_BOOK_ID).is_none());
        assert!(with_book.risk_routing_graph().is_none());
    }

    #[test]
    fn install_risk_routing_graph_auto_provisions_new_books() {
        // Saving a graph that references a not-yet-created portfolio id creates that
        // portfolio ENABLED, so a routed fill always has an enabled home.
        let mut store = IdentityStore::default();
        assert!(store.risk_book("fresh-book").is_none());
        store
            .install_risk_routing_graph(cond_graph("fresh-book"))
            .expect("graph installs, provisioning the new book");
        let provisioned = store
            .risk_book("fresh-book")
            .expect("the referenced book was auto-provisioned");
        assert!(provisioned.enabled, "auto-provisioned books are enabled");
        assert!(store.risk_routing_graph().is_some());
        // Whole-config validation now passes (the graph targets an enabled, known book).
        store
            .validate_risk_books()
            .expect("valid after provisioning");
    }

    #[test]
    fn install_risk_routing_graph_never_mutates_existing_books() {
        let mut store = IdentityStore::default();
        // An existing ENABLED book keeps its identity untouched (no duplicate provision).
        let alpha = store
            .create_risk_book(rb_edit("Alpha", None, None))
            .unwrap();
        store
            .install_risk_routing_graph(cond_graph(&alpha.id))
            .expect("installs against the existing enabled book");
        assert_eq!(store.risk_books.len(), 1, "no book auto-provisioned");
        assert_eq!(
            store.risk_book(&alpha.id).unwrap().name,
            "Alpha",
            "the existing book is not mutated",
        );

        // A graph naming a pre-existing DISABLED book is rejected (never silently
        // re-enabled), and the store is left unchanged (the caller commits only on Ok).
        let mut store2 = IdentityStore::default();
        let beta = store2
            .create_risk_book(RiskBookEdit {
                enabled: false,
                ..rb_edit("Beta", None, None)
            })
            .unwrap();
        let err = store2
            .install_risk_routing_graph(cond_graph(&beta.id))
            .unwrap_err();
        assert!(err.contains("risk routing graph is invalid"), "got: {err}");
        assert!(
            !store2.risk_book(&beta.id).unwrap().enabled,
            "the disabled book stays disabled",
        );
    }

    #[test]
    fn hedge_policy_graph_validates_against_instrument_and_lp_registries() {
        use celnet_hedge_routing::{ExitAction, HedgeGraph, HedgeNode, HedgeSize};
        use std::collections::BTreeMap;

        let mut store = IdentityStore::default();

        // A Warehouse-only policy names no external target ⇒ always valid, no registries needed.
        let warehouse = {
            let mut nodes = BTreeMap::new();
            nodes.insert(
                0,
                HedgeNode::Action {
                    exit: ExitAction::Warehouse,
                },
            );
            HedgeGraph { entry: 0, nodes }
        };
        store
            .set_hedge_policy_graph(warehouse)
            .expect("a warehouse-only policy is always valid");
        assert!(store.hedge_policy_graph().is_some());

        // A CROSS_INTERNAL leaf naming an unknown instrument is rejected (the known set is
        // derived from the instrument registry, which is empty here).
        let cross_ghost = {
            let mut nodes = BTreeMap::new();
            nodes.insert(
                0,
                HedgeNode::Action {
                    exit: ExitAction::CrossInternal {
                        instrument: "GHOST".into(),
                        max_size: HedgeSize::Overflow,
                    },
                },
            );
            HedgeGraph { entry: 0, nodes }
        };
        let err = store.set_hedge_policy_graph(cross_ghost).unwrap_err();
        assert!(
            err.contains("hedge policy graph is invalid") && err.contains("GHOST"),
            "got: {err}"
        );

        // Seeding the instrument registry makes a CROSS_INTERNAL against a seeded id valid.
        store.ensure_seed_instruments();
        let known = store
            .instruments
            .first()
            .expect("seed instruments")
            .instrument_id
            .clone();
        let cross_known = {
            let mut nodes = BTreeMap::new();
            nodes.insert(
                0,
                HedgeNode::Action {
                    exit: ExitAction::CrossInternal {
                        instrument: known,
                        max_size: HedgeSize::Overflow,
                    },
                },
            );
            HedgeGraph { entry: 0, nodes }
        };
        store
            .set_hedge_policy_graph(cross_known)
            .expect("a cross against a seeded instrument is valid");
    }

    #[test]
    fn set_hedge_config_validates_lp_panels_against_known_lps() {
        use crate::config::hedge_policy::{HedgeConfigDef, HedgeScopeKind, ScopedLpPanel};
        use celnet_hedge_routing::HedgeLpPanel;

        let mut store = IdentityStore::default();
        // The default config (no panels) is always valid.
        store
            .set_hedge_config(HedgeConfigDef::default())
            .expect("an empty-panel config is always valid");

        // A panel naming an LP the (here empty) registry does not know is rejected at the
        // write, collecting the unknown id into the message.
        let bad = HedgeConfigDef {
            lp_panels: vec![ScopedLpPanel {
                scope_kind: HedgeScopeKind::Book,
                scope_id: "RATES-EUR".into(),
                panel: HedgeLpPanel {
                    include: vec!["LP-1".into()],
                    exclude: vec![],
                },
            }],
            ..HedgeConfigDef::default()
        };
        let err = store.set_hedge_config(bad).unwrap_err();
        assert!(
            err.contains("hedge LP panel config is invalid") && err.contains("LP-1"),
            "got: {err}"
        );
        // The store is left unchanged on rejection.
        assert!(store.hedge_config().lp_panels.is_empty());
    }

    #[test]
    fn hedge_threshold_upsert_replaces_by_scope_and_deletes_on_zero_cap() {
        use crate::config::hedge_policy::{
            HedgeMetric, HedgeScopeKind, HedgeThresholdDef, ScopedThreshold,
        };

        let mut store = IdentityStore::default();
        let def = |cap: f64| HedgeThresholdDef {
            scope_kind: HedgeScopeKind::Book,
            metric: HedgeMetric::Dv01,
            cap,
            amber: 0.8,
            red: 0.9,
            target_fraction: 0.8,
            min_clip: 0.0,
            max_clip: f64::INFINITY,
            ramped: false,
            ramp_k: 1.0,
        };
        store.upsert_hedge_threshold(ScopedThreshold {
            scope_id: "RATES-EUR".into(),
            def: def(100_000.0),
        });
        assert_eq!(store.hedge_thresholds().len(), 1);
        // Same scope replaces (not appends).
        store.upsert_hedge_threshold(ScopedThreshold {
            scope_id: "RATES-EUR".into(),
            def: def(200_000.0),
        });
        assert_eq!(store.hedge_thresholds().len(), 1);
        assert_eq!(store.hedge_thresholds()[0].def.cap, 200_000.0);
        // A different scope appends.
        store.upsert_hedge_threshold(ScopedThreshold {
            scope_id: "RATES-USD".into(),
            def: def(50_000.0),
        });
        assert_eq!(store.hedge_thresholds().len(), 2);
        // A non-positive cap deletes the matching scope.
        store.upsert_hedge_threshold(ScopedThreshold {
            scope_id: "RATES-EUR".into(),
            def: def(0.0),
        });
        assert_eq!(store.hedge_thresholds().len(), 1);
        assert_eq!(store.hedge_thresholds()[0].scope_id, "RATES-USD");
    }

    #[test]
    fn risk_book_delete_guard_refuses_orphaning() {
        let mut store = IdentityStore::default();
        let parent = store
            .create_risk_book(rb_edit("Parent", None, None))
            .unwrap();
        store
            .create_risk_book(rb_edit("Child", Some(&parent.id), None))
            .unwrap();
        // A parent-of-children cannot be deleted (no orphaning).
        assert!(
            store
                .delete_risk_book(&parent.id)
                .unwrap_err()
                .contains("parent of")
        );

        // A graph-referenced book cannot be deleted.
        let leaf = store.create_risk_book(rb_edit("Leaf", None, None)).unwrap();
        store.set_risk_routing_graph(cond_graph(&leaf.id)).unwrap();
        assert!(
            store
                .delete_risk_book(&leaf.id)
                .unwrap_err()
                .contains("routing graph")
        );

        // A free leaf (no children, not routed to) deletes cleanly; a missing id no-ops.
        assert!(store.delete_risk_book("child").unwrap());
        assert!(!store.delete_risk_book("nope").unwrap());
    }

    #[test]
    fn risk_book_ancestors_and_descendants_three_levels() {
        let mut store = IdentityStore::default();
        let a = store.create_risk_book(rb_edit("A", None, None)).unwrap();
        let b = store
            .create_risk_book(rb_edit("B", Some(&a.id), None))
            .unwrap();
        let c = store
            .create_risk_book(rb_edit("C", Some(&b.id), None))
            .unwrap();

        // Ancestors of C: immediate parent B first, then A.
        let anc: Vec<&str> = store
            .risk_book_ancestors(&c.id)
            .iter()
            .map(|x| x.id.as_str())
            .collect();
        assert_eq!(anc, vec!["b", "a"]);
        // A top-level book has no ancestors.
        assert!(store.risk_book_ancestors(&a.id).is_empty());

        // Descendants of A: B and C, in registry order.
        let desc: Vec<&str> = store
            .risk_book_descendants(&a.id)
            .iter()
            .map(|x| x.id.as_str())
            .collect();
        assert_eq!(desc, vec!["b", "c"]);
        // A leaf has no descendants.
        assert!(store.risk_book_descendants(&c.id).is_empty());
    }

    #[test]
    fn save_is_atomic_and_reloads_equal() {
        let dir = std::env::temp_dir().join("celnet-identity-save");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("identity-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut store = IdentityStore::default();
        store.ensure_seed_admin().unwrap();
        store.save(&path).unwrap();
        assert!(!tmp_sibling(&path).exists(), "atomic temp left behind");

        let reloaded = IdentityStore::load(&path).unwrap();
        assert_eq!(store, reloaded);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pricing_control_round_trips_and_defaults_both_enabled() {
        // A store with no `pricing_control` key loads as both-enabled (additive default).
        let bare: IdentityStore = serde_json::from_str("{}").unwrap();
        assert_eq!(bare.pricing_control(), PricingControlDef::default());
        assert!(bare.pricing_control().outbound_enabled);
        assert!(bare.pricing_control().inbound_enabled);

        // A halt setting round-trips through JSON exactly.
        let mut store = IdentityStore::default();
        store.set_pricing_control(PricingControlDef {
            outbound_enabled: false,
            inbound_enabled: true,
        });
        let json = serde_json::to_string(&store).unwrap();
        let reloaded: IdentityStore = serde_json::from_str(&json).unwrap();
        assert_eq!(
            reloaded.pricing_control(),
            PricingControlDef {
                outbound_enabled: false,
                inbound_enabled: true,
            }
        );
    }

    #[test]
    fn missing_file_is_an_empty_store() {
        let path = std::env::temp_dir().join("celnet-identity-missing/none.json");
        let _ = std::fs::remove_file(&path);
        let store = IdentityStore::load(&path).unwrap();
        assert!(store.users.is_empty() && store.desks.is_empty());
    }

    impl IdentityStore {
        /// Test guard: no stored hash is the literal seed plaintext.
        fn password_hash_is_plaintext(&self) -> bool {
            self.users
                .iter()
                .any(|u| u.password_hash == SEED_ADMIN_PASSWORD)
        }
    }

    /// A persisted overlay round-trips: typed `Capability` → label form → typed,
    /// and `capability_overlay` returns the grants and denies it was given.
    #[test]
    fn capability_overlay_round_trips() {
        let fi_book = Capability::new(Action::Book, AssetClass::FixedIncome);
        let fx_exec = Capability::new(Action::Execute, AssetClass::FxOptions);
        assert_eq!(PermissionGrant::of(fi_book).parse().unwrap(), fi_book);

        let user = UserDef {
            id: "u".into(),
            email: "u@celnet.com".into(),
            display_name: "U".into(),
            role: Role::Trader,
            desk_ids: Vec::new(),
            all_desks: false,
            password_hash: "x".into(),
            disabled: false,
            capability_grants: vec![PermissionGrant::of(fi_book)],
            capability_denies: vec![PermissionGrant::of(fx_exec)],
        };
        let (grants, denies) = user.capability_overlay().expect("known labels parse");
        assert_eq!(grants, vec![fi_book]);
        assert_eq!(denies, vec![fx_exec]);
    }

    /// A role with no persisted bundle resolves to the default trader bundle (every
    /// action but `administer` on both assets); Admin resolves to the empty base
    /// (its grant-all is the session's concern, never a stored bundle).
    #[test]
    fn role_base_defaults_then_persists() {
        let mut store = IdentityStore::default();
        assert_eq!(store.role_base(Role::Trader), default_trader_bundle());
        assert!(store.role_base(Role::Admin).is_empty());

        // Narrow the trader bundle to a single capability and round-trip it.
        let only = Capability::new(Action::View, AssetClass::FxOptions);
        store
            .role_bundles
            .insert(Role::Trader, vec![PermissionGrant::of(only)]);
        assert_eq!(store.role_base(Role::Trader), vec![only]);

        let bytes = serde_json::to_vec(&store).unwrap();
        let back: IdentityStore = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(store, back);
        assert_eq!(back.role_base(Role::Trader), vec![only]);
    }

    /// Migration: an identity record carrying the legacy single `desk_id` string
    /// loads as a one-element `desk_ids` set (membership preserved, nothing lost),
    /// and a blank legacy `desk_id` loads as the empty (deskless) set.
    #[test]
    fn legacy_single_desk_id_migrates_to_set() {
        let json = r#"{
            "users": [
                { "id": "jane", "email": "jane@celnet.com", "display_name": "Jane",
                  "role": "trader", "desk_id": "g10", "password_hash": "x" },
                { "id": "joe", "email": "joe@celnet.com", "display_name": "Joe",
                  "role": "trader", "desk_id": "  ", "password_hash": "y" }
            ],
            "desks": []
        }"#;
        let store: IdentityStore = serde_json::from_str(json).unwrap();
        let jane = store.user("jane").unwrap();
        assert_eq!(jane.desk_ids, vec!["g10".to_owned()]);
        assert!(!jane.all_desks);
        // A blank legacy id ⇒ deskless, not a `[""]` set.
        let joe = store.user("joe").unwrap();
        assert!(joe.desk_ids.is_empty() && !joe.all_desks);
    }

    /// A current record carrying `desk_ids` + `all_desks` loads verbatim, and a new
    /// write emits only the new keys (the legacy `desk_id` never round-trips back).
    #[test]
    fn new_membership_round_trips_without_legacy_key() {
        let json = r#"{
            "users": [
                { "id": "u", "email": "u@celnet.com", "display_name": "U",
                  "role": "trader", "desk_ids": ["a", "b"], "password_hash": "x" },
                { "id": "v", "email": "v@celnet.com", "display_name": "V",
                  "role": "trader", "all_desks": true, "password_hash": "y" }
            ],
            "desks": []
        }"#;
        let store: IdentityStore = serde_json::from_str(json).unwrap();
        assert_eq!(
            store.user("u").unwrap().desk_ids,
            vec!["a".to_owned(), "b".to_owned()]
        );
        assert!(store.user("v").unwrap().all_desks);
        // Serialize → the legacy key is absent; reload is equal (stable round-trip).
        let bytes = serde_json::to_vec(&store).unwrap();
        assert!(
            !String::from_utf8(bytes.clone())
                .unwrap()
                .contains("\"desk_id\"")
        );
        let back: IdentityStore = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(store, back);
    }

    /// An identity file carrying an unknown role-bundle label is rejected at load
    /// (`InvalidData`), exactly like a bad per-user overlay.
    #[test]
    fn load_rejects_unknown_role_bundle_label() {
        let json = r#"{
            "users": [],
            "desks": [],
            "role_bundles": { "trader": [{"action": "teleport", "asset": "fx_options"}] }
        }"#;
        let path = std::env::temp_dir().join("celnet-identity-badrole.json");
        std::fs::write(&path, json).unwrap();
        let err = IdentityStore::load(&path).expect_err("unknown label must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        let _ = std::fs::remove_file(&path);
    }

    /// An identity file with no `role_bundles` key loads (serde-default empty map) and
    /// the trader resolves to the default bundle — existing files behave unchanged.
    #[test]
    fn load_without_role_bundles_uses_default() {
        let json = r#"{ "users": [], "desks": [] }"#;
        let path = std::env::temp_dir().join("celnet-identity-norole.json");
        std::fs::write(&path, json).unwrap();
        let store = IdentityStore::load(&path).unwrap();
        assert!(store.role_bundles.is_empty());
        assert_eq!(store.role_base(Role::Trader), default_trader_bundle());
        let _ = std::fs::remove_file(&path);
    }

    /// The seeded registry is non-empty, valid, idempotent, and round-trips through
    /// `identity.json` (entities + books survive a save/reload byte-for-byte).
    #[test]
    fn seed_registry_is_idempotent_and_round_trips() {
        let mut store = IdentityStore::default();
        assert!(store.ensure_seed_registry(), "first call seeds");
        assert_eq!(store.entities.len(), 2);
        assert_eq!(store.books.len(), 4);
        store.validate_registry().expect("seeded registry is valid");
        // Resolution helpers map keys → names.
        assert_eq!(store.entity_name(1), Some("Celnet Global Markets"));
        assert_eq!(store.book_name(1), Some("Rates Trading"));
        assert_eq!(store.entity_name(99), None);
        // A second call is a no-op (entities already present).
        assert!(!store.ensure_seed_registry(), "second call is idempotent");

        let path =
            std::env::temp_dir().join(format!("celnet-identity-reg-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        store.save(&path).unwrap();
        let reloaded = IdentityStore::load(&path).unwrap();
        assert_eq!(store, reloaded);
        let _ = std::fs::remove_file(&path);
    }

    /// `next_entity_key`/`next_book_key` return the lowest free key starting at 1.
    #[test]
    fn next_keys_fill_lowest_gap() {
        let mut store = IdentityStore::default();
        assert_eq!(store.next_entity_key(), 1);
        assert_eq!(store.next_book_key(), 1);
        store.ensure_seed_registry();
        // Seeded entities use 1,2 and books use 1..=4.
        assert_eq!(store.next_entity_key(), 3);
        assert_eq!(store.next_book_key(), 5);
    }

    /// A book whose `entity_key` does not resolve is rejected at load (`InvalidData`).
    #[test]
    fn load_rejects_dangling_book_entity_key() {
        let json = r#"{
            "users": [], "desks": [],
            "entities": [{"key": 1, "name": "ACME Capital", "code": "ACME"}],
            "books": [{"key": 1, "name": "Rates Trading", "entity_key": 7}]
        }"#;
        let path = std::env::temp_dir().join("celnet-identity-dangling.json");
        std::fs::write(&path, json).unwrap();
        let err = IdentityStore::load(&path).expect_err("dangling entity_key must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        let _ = std::fs::remove_file(&path);
    }

    /// A registry with duplicate entity keys is rejected at load.
    #[test]
    fn load_rejects_duplicate_entity_key() {
        let json = r#"{
            "users": [], "desks": [],
            "entities": [
                {"key": 1, "name": "A", "code": "A"},
                {"key": 1, "name": "B", "code": "B"}
            ],
            "books": []
        }"#;
        let path = std::env::temp_dir().join("celnet-identity-dupkey.json");
        std::fs::write(&path, json).unwrap();
        let err = IdentityStore::load(&path).expect_err("duplicate key must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        let _ = std::fs::remove_file(&path);
    }

    /// An identity file with no `entities`/`books` keys loads (serde-default empty)
    /// — existing files behave unchanged.
    #[test]
    fn load_without_registry_is_empty() {
        let json = r#"{ "users": [], "desks": [] }"#;
        let path = std::env::temp_dir().join("celnet-identity-noreg.json");
        std::fs::write(&path, json).unwrap();
        let store = IdentityStore::load(&path).unwrap();
        assert!(store.entities.is_empty() && store.books.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    /// An identity file carrying an unknown capability label is rejected at load
    /// (`InvalidData`) — a corrupt overlay never silently degrades authority.
    #[test]
    fn load_rejects_unknown_capability_label() {
        let json = r#"{
            "users": [{
                "id": "u", "email": "u@celnet.com", "display_name": "U",
                "role": "trader", "password_hash": "x", "disabled": false,
                "capability_grants": [{"action": "teleport", "asset": "fixed_income"}]
            }],
            "desks": []
        }"#;
        let path = std::env::temp_dir().join("celnet-identity-badcap.json");
        std::fs::write(&path, json).unwrap();
        let err = IdentityStore::load(&path).expect_err("unknown label must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        let _ = std::fs::remove_file(&path);
    }

    /// Create an aggregated book, read it back by id, then delete it. The default
    /// `all_members_quote` scope needs no reference data.
    #[test]
    fn aggregated_book_create_get_and_delete() {
        let mut store = IdentityStore::default();
        let def = store
            .create_aggregated_book(AggregatedBookEdit::new(
                "G10 Rates Composite",
                vec!["fix-lp-a".into(), "fix-lp-b".into()],
                Scope::AllMembersQuote,
                AggregationParams::default(),
                true,
            ))
            .expect("valid book creates");
        assert_eq!(def.id, "g10-rates-composite");
        assert_eq!(store.aggregated_book(&def.id), Some(&def));
        assert!(def.enabled);
        assert_eq!(def.params, AggregationParams::default());
        // Delete reports removal; a second delete is a no-op.
        assert!(store.delete_aggregated_book(&def.id).unwrap());
        assert!(store.aggregated_book(&def.id).is_none());
        assert!(!store.delete_aggregated_book(&def.id).unwrap());
    }

    /// Renaming keeps the id stable and rejects a name that collides with another book.
    #[test]
    fn aggregated_book_rename_keeps_id() {
        let mut store = IdentityStore::default();
        let a = store
            .create_aggregated_book(AggregatedBookEdit::new(
                "Alpha",
                vec![],
                Scope::AllMembersQuote,
                AggregationParams::default(),
                true,
            ))
            .unwrap();
        let updated = store
            .update_aggregated_book(
                &a.id,
                AggregatedBookEdit::new(
                    "Alpha Prime",
                    vec!["lp-1".into()],
                    Scope::AllMembersQuote,
                    AggregationParams::default(),
                    false,
                ),
            )
            .expect("rename succeeds");
        assert_eq!(updated.id, a.id, "id is preserved across a rename");
        assert_eq!(store.aggregated_book(&a.id).unwrap().name, "Alpha Prime");
        assert!(!store.aggregated_book(&a.id).unwrap().enabled);
    }

    /// A name that duplicates an existing book (case-insensitively) is rejected on both
    /// create and update.
    #[test]
    fn aggregated_book_duplicate_name_rejected() {
        let mut store = IdentityStore::default();
        store
            .create_aggregated_book(AggregatedBookEdit::new(
                "Composite One",
                vec![],
                Scope::AllMembersQuote,
                AggregationParams::default(),
                true,
            ))
            .unwrap();
        let err = store
            .create_aggregated_book(AggregatedBookEdit::new(
                "  composite one  ",
                vec![],
                Scope::AllMembersQuote,
                AggregationParams::default(),
                true,
            ))
            .expect_err("case-insensitive duplicate name must be rejected");
        assert!(err.contains("already exists"));
        // The edited book may keep its own name, but not take another's.
        let two = store
            .create_aggregated_book(AggregatedBookEdit::new(
                "Composite Two",
                vec![],
                Scope::AllMembersQuote,
                AggregationParams::default(),
                true,
            ))
            .unwrap();
        let err = store
            .update_aggregated_book(
                &two.id,
                AggregatedBookEdit::new(
                    "COMPOSITE ONE",
                    vec![],
                    Scope::AllMembersQuote,
                    AggregationParams::default(),
                    true,
                ),
            )
            .expect_err("rename onto another book's name must be rejected");
        assert!(err.contains("already exists"));
    }

    /// An explicit scope with an instrument id that does not resolve to an
    /// [`InstrumentDef`] is rejected; a valid seeded id is accepted.
    #[test]
    fn aggregated_book_explicit_unknown_instrument_rejected() {
        let mut store = IdentityStore::default();
        store.ensure_seed_instruments();
        let known = store.instruments[0].instrument_id.clone();

        let err = store
            .create_aggregated_book(AggregatedBookEdit::new(
                "Explicit Bad",
                vec![],
                Scope::Explicit(vec![known.clone(), "no-such-instrument".into()]),
                AggregationParams::default(),
                true,
            ))
            .expect_err("an unknown explicit instrument id must be rejected");
        assert!(err.contains("unknown instrument_id"));

        // A scope of only known ids is accepted.
        store
            .create_aggregated_book(AggregatedBookEdit::new(
                "Explicit Good",
                vec![],
                Scope::Explicit(vec![known]),
                AggregationParams::default(),
                true,
            ))
            .expect("a resolvable explicit scope is accepted");
    }

    /// A book listing the same member connection id twice is rejected.
    #[test]
    fn aggregated_book_duplicate_member_rejected() {
        let mut store = IdentityStore::default();
        let err = store
            .create_aggregated_book(AggregatedBookEdit::new(
                "Dup Members",
                vec!["lp-x".into(), "lp-x".into()],
                Scope::AllMembersQuote,
                AggregationParams::default(),
                true,
            ))
            .expect_err("a duplicate member id must be rejected");
        assert!(err.contains("more than once"));
    }

    /// Out-of-range params are rejected: `min_contributors` must be `>= 1` and
    /// `staleness_tau_ms` must be `> 0`.
    #[test]
    fn aggregated_book_out_of_range_params_rejected() {
        let mut store = IdentityStore::default();
        let zero_contrib = AggregationParams {
            min_contributors: 0,
            ..AggregationParams::default()
        };
        let err = store
            .create_aggregated_book(AggregatedBookEdit::new(
                "Zero Quorum",
                vec![],
                Scope::AllMembersQuote,
                zero_contrib,
                true,
            ))
            .expect_err("min_contributors = 0 must be rejected");
        assert!(err.contains("min_contributors >= 1"));

        let zero_tau = AggregationParams {
            staleness_tau_ms: 0,
            ..AggregationParams::default()
        };
        let err = store
            .create_aggregated_book(AggregatedBookEdit::new(
                "Zero Tau",
                vec![],
                Scope::AllMembersQuote,
                zero_tau,
                true,
            ))
            .expect_err("staleness_tau_ms = 0 must be rejected");
        assert!(err.contains("staleness_tau_ms > 0"));
    }

    /// A full `identity.json` carrying an aggregated book (members + explicit scope +
    /// tuned params) survives a save/reload byte-for-byte (the load path re-validates).
    #[test]
    fn aggregated_book_json_round_trips() {
        let mut store = IdentityStore::default();
        store.ensure_seed_admin().unwrap();
        store.ensure_seed_instruments();
        let known = store.instruments[0].instrument_id.clone();
        store
            .create_aggregated_book(AggregatedBookEdit::new(
                "Round Trip Composite",
                vec!["fix-lp-a".into(), "api-lp-b".into()],
                Scope::Explicit(vec![known]),
                AggregationParams {
                    staleness_tau_ms: 750,
                    max_quote_age_ms: 3000,
                    divergence_gating: false,
                    min_contributors: 2,
                    depth_levels: 5,
                },
                true,
            ))
            .unwrap();

        let path = std::env::temp_dir().join(format!(
            "celnet-identity-aggbook-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        store.save(&path).unwrap();
        let reloaded = IdentityStore::load(&path).unwrap();
        assert_eq!(store, reloaded);
        let _ = std::fs::remove_file(&path);
    }

    /// An `identity.json` with no `aggregated_books` key loads (serde-default empty vec)
    /// — existing files behave unchanged.
    #[test]
    fn load_without_aggregated_books_is_empty() {
        let json = r#"{ "users": [], "desks": [] }"#;
        let path = std::env::temp_dir().join("celnet-identity-noaggbook.json");
        std::fs::write(&path, json).unwrap();
        let store = IdentityStore::load(&path).unwrap();
        assert!(store.aggregated_books.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    /// An identity file carrying an aggregated book with a dangling explicit instrument
    /// id is rejected at load (`InvalidData`), exactly like a bad registry.
    #[test]
    fn load_rejects_dangling_aggregated_book_instrument() {
        let json = r#"{
            "users": [], "desks": [],
            "aggregated_books": [{
                "id": "c1", "name": "C1", "member_connection_ids": [],
                "instrument_scope": {"mode": "explicit", "instrument_ids": ["ghost"]},
                "params": {"staleness_tau_ms": 500, "max_quote_age_ms": 2500,
                           "divergence_gating": true, "min_contributors": 1, "depth_levels": 1},
                "enabled": true
            }]
        }"#;
        let path = std::env::temp_dir().join("celnet-identity-aggghost.json");
        std::fs::write(&path, json).unwrap();
        let err = IdentityStore::load(&path).expect_err("dangling instrument must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        let _ = std::fs::remove_file(&path);
    }

    /// The adjacently-tagged `Scope` encoding round-trips human-readably: the unit
    /// variant is `{"mode":"all_members_quote"}` and the payload variant carries its
    /// `instrument_ids` list.
    #[test]
    fn scope_tagged_encoding_round_trips() {
        let all = serde_json::to_string(&Scope::AllMembersQuote).unwrap();
        assert_eq!(all, r#"{"mode":"all_members_quote"}"#);
        let explicit = Scope::Explicit(vec!["a".into(), "b".into()]);
        let json = serde_json::to_string(&explicit).unwrap();
        assert_eq!(json, r#"{"mode":"explicit","instrument_ids":["a","b"]}"#);
        assert_eq!(serde_json::from_str::<Scope>(&json).unwrap(), explicit);

        // `mint_aggregated_book_id` disambiguates a colliding slug.
        let existing = vec![AggregatedBookDef {
            id: "alpha".into(),
            name: "Alpha".into(),
            member_connection_ids: vec![],
            instrument_scope: Scope::AllMembersQuote,
            params: AggregationParams::default(),
            enabled: true,
        }];
        assert_eq!(mint_aggregated_book_id("Alpha", &existing), "alpha-2");
    }
}
