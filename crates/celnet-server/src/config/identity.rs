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

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl UserDef {
    /// Verify a candidate plaintext password against this user's stored hash in
    /// constant time. A disabled account always rejects.
    #[must_use]
    pub fn verify(&self, candidate: &str) -> bool {
        !self.disabled && verify_password(&self.password_hash, candidate)
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

/// The persisted document: the users and desks of the edge.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityStore {
    /// The user accounts, in creation order.
    #[serde(default)]
    pub users: Vec<UserDef>,
    /// The desks traders can belong to, in creation order.
    #[serde(default)]
    pub desks: Vec<DeskDef>,
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
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
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
}
