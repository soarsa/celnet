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

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use celnet_entitlements::{Action, AssetClass, Capability};
use serde::{Deserialize, Serialize};

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
/// non-admin role): every action except [`Action::Administer`] on **both** asset
/// classes. This is the slice-1 hardcoded base that the admin-editable
/// [`IdentityStore::role_bundles`] overlay now persists and can narrow/widen
/// (`docs/plan/PERMISSIONS-ADMINISTRATION-REQUIREMENT.md` §3.3/§10). A store with no
/// persisted bundle for the role resolves to exactly this set, so an existing
/// `identity.json` (which carries no `role_bundles`) behaves identically to before.
#[must_use]
pub fn default_trader_bundle() -> Vec<Capability> {
    let mut caps = Vec::new();
    for action in Action::ALL {
        if matches!(action, Action::Administer) {
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
/// `Eq` is intentionally **not** derived: the optional [`tiering`](Self::tiering)
/// block carries `f64` spread/guardrail magnitudes, so the definition is only
/// `PartialEq` (as [`IdentityStore`] itself already is, for the same reason).
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
    /// The optional outbound-**tiering** configuration (`celnet-tiering`): the enabled
    /// strategies + params + guardrails + unit + stale policy applied to this book's
    /// composite *before* publish (`docs/FI-TIERING-RESEARCH.md`; Phase 2a). `None`
    /// (the additive serde-default) ⇒ tiering disabled: the raw composite is published
    /// unchanged — zero behaviour change for an existing book, which carries no
    /// `tiering` key and loads exactly as before. Validated at load and every admin
    /// write (see [`check_aggregated_book`](IdentityStore::check_aggregated_book)).
    #[serde(default)]
    pub tiering: Option<celnet_tiering::TieringConfig>,
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
    /// The optional outbound-tiering configuration (`None` ⇒ tiering disabled).
    pub tiering: Option<celnet_tiering::TieringConfig>,
}

impl AggregatedBookEdit {
    /// A minimal edit with tiering disabled — the common form; layer a config on with
    /// [`Self::with_tiering`].
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
            tiering: None,
        }
    }

    /// Set the outbound-tiering configuration.
    #[must_use]
    pub fn with_tiering(mut self, tiering: Option<celnet_tiering::TieringConfig>) -> Self {
        self.tiering = tiering;
        self
    }
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
        if let Some(tiering) = &def.tiering {
            validate_tiering_config(&def.name, tiering)?;
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
            tiering: edit.tiering,
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
            tiering: edit.tiering,
        };
        self.check_aggregated_book(&def)?;
        if let Some(slot) = self.aggregated_books.iter_mut().find(|b| b.id == id) {
            *slot = def.clone();
        }
        Ok(def)
    }

    /// Replace **only** an aggregated book's outbound-[`tiering`](AggregatedBookDef::tiering)
    /// block (the trader-configurable spread), leaving its **structure** — id, name,
    /// members, instrument scope, consolidation params, enabled flag — byte-identical.
    /// Validates the resulting definition's invariants (via
    /// [`check_aggregated_book`](Self::check_aggregated_book), which runs
    /// [`validate_tiering_config`] on a `Some` config), replaces the slot, and returns the
    /// updated definition. This is the store side of the trader-facing tiering RPC: the
    /// admin owns what the book *is* (structure), the trader owns the outbound spread.
    ///
    /// # Errors
    /// No book with `id`, or a tiering config failing its invariants (inconsistent
    /// guardrails, empty strategy list, non-finite / negative magnitude).
    pub fn update_aggregated_book_tiering(
        &mut self,
        id: &str,
        tiering: Option<celnet_tiering::TieringConfig>,
    ) -> Result<AggregatedBookDef, String> {
        let Some(existing) = self.aggregated_book(id).cloned() else {
            return Err(format!("no aggregated book with id {id:?}"));
        };
        // Structure preserved verbatim; only the tiering block is swapped.
        let def = AggregatedBookDef {
            tiering,
            ..existing
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
        validate_feature_pipeline(&def.name, "ESP", &def.esp_pipeline)?;
        validate_feature_pipeline(&def.name, "RFQ", &def.rfq_pipeline)?;
        Ok(())
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

/// Validate a book's outbound-[`tiering`](AggregatedBookDef::tiering) configuration
/// at load and at every admin write (mirroring the `reference_data.rs::validate_*`
/// discipline — reject non-finite magnitudes, negative-where-magnitude, and
/// internally inconsistent guardrail bounds), so an unsound config is rejected
/// loudly here rather than silently suppressing every quote at runtime.
///
/// * The guardrail bounds must pass the engine's own invariant (`0 ≤ h_min ≤ h_max`,
///   `s_max ≥ 0`, `spread_floor > 0`, `h_max ≥ spread_floor/2`).
/// * At least one strategy must be enabled (an empty strategy list is a mistake —
///   omit the whole `tiering` block to disable tiering instead).
/// * Every strategy magnitude is finite, and the half-spread / skew-cap / gain
///   magnitudes are non-negative.
///
/// # Errors
/// The first inconsistent guardrail, empty strategy list, or non-finite / negative
/// magnitude, as a human-readable message.
fn validate_tiering_config(book: &str, cfg: &celnet_tiering::TieringConfig) -> Result<(), String> {
    tiering_config_reason(cfg).map_err(|m| format!("aggregated book {book:?} tiering {m}"))
}

/// The bare (entity-unprefixed) reason a [`celnet_tiering::TieringConfig`] is invalid,
/// or `Ok` — the single rule set behind both the aggregated-book tiering validator
/// ([`validate_tiering_config`]) and the pricing-group pipeline validator
/// ([`validate_feature_pipeline`], for each embedded TIERING feature). Each caller
/// prefixes the returned reason with its own entity context, so both messages are
/// produced from one implementation with no drift (and the aggregated-book message is
/// byte-identical to before the extraction).
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
/// Reusing the tiering validation verbatim keeps a group's TIERING feature held to
/// exactly the same standard as a book's tiering block.
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
            tiering: None,
        }];
        assert_eq!(mint_aggregated_book_id("Alpha", &existing), "alpha-2");
    }
}
