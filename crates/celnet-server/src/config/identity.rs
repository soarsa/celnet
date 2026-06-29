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
//! RFQ traffic). A user optionally belongs to one [`DeskDef`] by `desk_id`; desk
//! membership is what scopes RFQ/monitor visibility (built on top of this store).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use celnet_entitlements::{Action, AssetClass, Capability};
use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDef {
    /// Stable identifier (the store/API key). Never reused; minted from the email.
    pub id: String,
    /// Login email — unique across the store (case-insensitive), the credential.
    pub email: String,
    /// Human-friendly display name shown in the UI.
    pub display_name: String,
    /// What the user may do.
    pub role: Role,
    /// The desk this user belongs to, by [`DeskDef::id`]; `None` ⇒ unassigned.
    #[serde(default)]
    pub desk_id: Option<String>,
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

/// The persisted document: the users and desks of the edge.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
            desk_id: None,
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
            desk_id: None,
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
            desk_id: None,
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
            desk_id: None,
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
}
