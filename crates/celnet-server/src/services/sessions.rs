//! The live **session registry**: the server-enforced authentication state that
//! turns a successful login into a bearer token, and validates that token on
//! every gated RPC.
//!
//! # Why server-enforced
//!
//! The historical [`access`](super::access) boundary authorized a *client-
//! asserted* entitlement principal — honest about being a deploy-layer concern,
//! but it authenticated no one. This registry is the authentication half: a caller
//! proves identity once ([`AuthService.Login`], password-checked against the
//! Argon2id [`IdentityStore`](crate::config::identity::IdentityStore)), receives an
//! unguessable [`Session`] token, and presents it on subsequent requests. The
//! server — not the client — decides who the caller is by looking the token up
//! here.
//!
//! # Token secrecy
//!
//! A token is **256 bits** of OS-CSPRNG entropy, hex-encoded. It is a secret
//! equivalent to the password for the session's lifetime: it is never persisted to
//! disk (sessions live only in process memory), never logged, and only ever
//! compared, never reconstructed. A process restart invalidates every token (the
//! map is empty on boot) — callers simply log in again.
//!
//! # Lifetime & revocation
//!
//! Sessions expire a fixed [`DEFAULT_TTL`](SessionRegistry::DEFAULT_TTL) after
//! issue. Expiry is enforced lazily on [`validate`](SessionRegistry::validate) (an
//! expired token is dropped and rejected) and the whole map can be swept.
//! Administrative changes that alter a user's authority — disable, delete,
//! role/desk change, password reset — call
//! [`revoke_user`](SessionRegistry::revoke_user) so stale sessions cannot outlive
//! the change.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use celnet_entitlements::{Capability, CapabilitySet};

use crate::clock::Clock;
use crate::config::identity::{Role, UserDef, default_trader_bundle};

/// Number of random bytes in a session token (256 bits).
const TOKEN_BYTES: usize = 32;

/// The authenticated identity resolved from a valid session — a cheap, owned
/// snapshot taken at login. Downstream gating (admin checks, desk-scoped RFQ
/// visibility) reads this without touching the identity store.
///
/// Because it is a snapshot, an administrative change to the underlying user is
/// reflected only after the affected sessions are revoked (which the admin path
/// does) and the user logs in again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedUser {
    /// The user's stable id ([`UserDef::id`]).
    pub user_id: String,
    /// The user's login email.
    pub email: String,
    /// The user's display name.
    pub display_name: String,
    /// The user's role at the time the session was issued.
    pub role: Role,
    /// The desks the user belonged to at login (snapshot of [`UserDef::desk_ids`]).
    /// Empty + `!all_desks` ⇒ deskless. The access boundary maps this to a
    /// [`DeskScope`](super::access::DeskScope) for RFQ/notification/risk narrowing.
    pub desk_ids: Vec<String>,
    /// When set, the user belonged to **every** desk at login (snapshot of
    /// [`UserDef::all_desks`]) — mapped to `DeskScope::All`. An admin is always
    /// all-desks (via [`is_admin`](Self::is_admin)) regardless of this flag.
    pub all_desks: bool,
    /// The resolved capability **base** the user's role conferred at login — the
    /// admin-editable per-role bundle snapshot
    /// ([`IdentityStore::role_base`](crate::config::identity::IdentityStore::role_base)),
    /// layered under the per-user overlay in [`capabilities`](Self::capabilities). An
    /// [`Role::Admin`] session ignores this (it resolves to grant-all); a non-admin
    /// session uses it as the base set. Because it is a snapshot, an admin edit to the
    /// bundle takes effect only after the role's sessions are revoked (which the admin
    /// path does) and the user logs in again.
    pub role_caps: Vec<Capability>,
    /// The user's per-user capability **grants** at login (snapshot of
    /// [`UserDef::capability_grants`]), layered on top of the role bundle in
    /// [`capabilities`](Self::capabilities).
    pub cap_grants: Vec<Capability>,
    /// The user's per-user capability **denials** at login (snapshot of
    /// [`UserDef::capability_denies`]); deny-wins over the role bundle and grants.
    pub cap_denies: Vec<Capability>,
}

impl AuthenticatedUser {
    /// Snapshot a stored user into a session identity.
    ///
    /// The capability overlay is parsed from the user's persisted labels. The
    /// identity store validates every overlay at load
    /// ([`IdentityStore::load`](crate::config::identity::IdentityStore::load)) and at
    /// every admin write, so the labels are well-formed here; a (load-impossible)
    /// malformed entry is dropped per-list rather than panicking — dropping a
    /// **grant** fails closed (less authority), the safe direction.
    #[must_use]
    pub fn from_user(user: &UserDef) -> Self {
        Self::from_user_with_role_base(user, default_trader_bundle())
    }

    /// Snapshot a stored user into a session identity using an **explicit** resolved
    /// role base (the admin-editable per-role bundle resolved from the live
    /// [`IdentityStore`](crate::config::identity::IdentityStore)). The login path uses
    /// this so a narrowed/widened role bundle is reflected; [`from_user`](Self::from_user)
    /// is the convenience that defaults to the standard
    /// [`default_trader_bundle`](crate::config::identity::default_trader_bundle).
    ///
    /// The per-user overlay is parsed from the user's persisted labels (validated at
    /// load and at every admin write); a (load-impossible) malformed entry is dropped
    /// per-list rather than panicking — dropping a **grant** fails closed (less
    /// authority), the safe direction.
    #[must_use]
    pub fn from_user_with_role_base(user: &UserDef, role_caps: Vec<Capability>) -> Self {
        let cap_grants = user
            .capability_grants
            .iter()
            .filter_map(|g| g.parse().ok())
            .collect();
        let cap_denies = user
            .capability_denies
            .iter()
            .filter_map(|g| g.parse().ok())
            .collect();
        Self {
            user_id: user.id.clone(),
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            role: user.role,
            desk_ids: user.desk_ids.clone(),
            all_desks: user.all_desks,
            role_caps,
            cap_grants,
            cap_denies,
        }
    }

    /// Whether this session carries administrative authority.
    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.role.is_admin()
    }

    /// The effective **action capabilities** this caller holds, derived from their
    /// role (`docs/plan/PERMISSIONS-ADMINISTRATION-REQUIREMENT.md` §3):
    ///
    /// * [`Role::Admin`] ⇒ [`CapabilitySet::grant_all`] (every action, both asset
    ///   classes — including [`Action::Administer`](celnet_entitlements::Action::Administer)),
    ///   never narrowable;
    /// * a non-admin role ⇒ the admin-editable per-role bundle snapshotted at login
    ///   ([`role_caps`](Self::role_caps)), scoped at the resource edge by
    ///   [`super::access::ResolvedCaller::desk_scope`].
    ///
    /// The role bundle is the **base**; the per-user overlay then applies
    /// ([`cap_grants`](Self::cap_grants) widen, [`cap_denies`](Self::cap_denies)
    /// narrow, deny-wins). The returned set is the single source the access boundary
    /// consults — separation of duties (e.g. a trader granted FX but **denied**
    /// `Execute·FixedIncome`) is expressed entirely here.
    #[must_use]
    pub fn capabilities(&self) -> CapabilitySet {
        let mut set = match self.role {
            Role::Admin => CapabilitySet::grant_all(),
            Role::Trader => {
                let mut base = CapabilitySet::empty();
                for &cap in &self.role_caps {
                    base = base.grant(cap);
                }
                base
            }
        };
        for &cap in &self.cap_grants {
            set = set.grant(cap);
        }
        for &cap in &self.cap_denies {
            set = set.deny(cap);
        }
        set
    }
}

/// One live session: the authenticated identity plus its issue/expiry window.
#[derive(Clone)]
struct Session {
    user: AuthenticatedUser,
    expires_nanos: i64,
}

// The bearer token never appears in a debug rendering — defense-in-depth against
// accidental logging of a live session secret.
impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("user", &self.user.user_id)
            .field("expires_nanos", &self.expires_nanos)
            .finish()
    }
}

/// A freshly issued session: the bearer token plus its absolute expiry, returned
/// to [`AuthService.Login`] so the response can carry the deadline.
#[derive(Clone, PartialEq, Eq)]
pub struct IssuedSession {
    /// The opaque bearer token to present on subsequent RPCs.
    pub token: String,
    /// Absolute expiry (epoch nanos); the session is rejected past it.
    pub expires_nanos: i64,
}

// The raw bearer token is a secret; redact it from any debug rendering so it
// cannot leak into a log line, span, or panic message.
impl fmt::Debug for IssuedSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IssuedSession")
            .field("token", &"[REDACTED]")
            .field("expires_nanos", &self.expires_nanos)
            .finish()
    }
}

/// The process-local registry of live sessions, keyed by bearer token.
///
/// Cheap concern: a single [`Mutex`] guards the map. Session validation is a
/// control-plane (async edge) operation, never the pinned pricing core, so the
/// lock never touches a latency budget.
#[derive(Debug)]
pub struct SessionRegistry {
    inner: Mutex<HashMap<String, Session>>,
    clock: Clock,
    ttl_nanos: i64,
}

impl SessionRegistry {
    /// The default session lifetime: 12 hours (a trading day), in nanoseconds.
    pub const DEFAULT_TTL: i64 = 12 * 60 * 60 * 1_000_000_000;

    /// A registry with the [`DEFAULT_TTL`](SessionRegistry::DEFAULT_TTL) lifetime.
    #[must_use]
    pub fn new(clock: Clock) -> Self {
        Self::with_ttl(clock, Self::DEFAULT_TTL)
    }

    /// A registry with an explicit session lifetime (tests drive short TTLs).
    #[must_use]
    pub fn with_ttl(clock: Clock, ttl_nanos: i64) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            clock,
            ttl_nanos,
        }
    }

    /// Issue a fresh session for an authenticated user, returning the bearer
    /// token and its expiry. The token is 256 bits of OS-CSPRNG entropy;
    /// collisions are cryptographically impossible, so an insert never overwrites
    /// a live session.
    ///
    /// # Errors
    /// Returns a message only if the OS CSPRNG fails.
    pub fn issue(&self, user: AuthenticatedUser) -> Result<IssuedSession, String> {
        let token = mint_token()?;
        let expires_nanos = self.clock.now_nanos().saturating_add(self.ttl_nanos);
        self.inner
            .lock()
            .expect("session registry mutex poisoned")
            .insert(
                token.clone(),
                Session {
                    user,
                    expires_nanos,
                },
            );
        Ok(IssuedSession {
            token,
            expires_nanos,
        })
    }

    /// Resolve a bearer token to its authenticated identity and expiration (epoch nanos),
    /// or `None` if the token is unknown or expired. An expired token is dropped here
    /// (lazy expiry) so a stale credential cannot be replayed.
    #[must_use]
    pub fn get_session(&self, token: &str) -> Option<(AuthenticatedUser, i64)> {
        let now = self.clock.now_nanos();
        let mut map = self.inner.lock().expect("session registry mutex poisoned");
        if let Some(s) = map.get(token) {
            if s.expires_nanos > now {
                return Some((s.user.clone(), s.expires_nanos));
            } else {
                map.remove(token);
                return None;
            }
        }

        // If not present in process session map, check if token is a DeskModal JWT (RFC 7519)
        if token.starts_with("ey") || token.contains('.') {
            let secret = crate::services::jwt::get_jwt_secret();
            let now_secs = now / 1_000_000_000;
            if let Ok(claims) = crate::services::jwt::verify_deskmodal_jwt(token, secret, now_secs) {
                let user = crate::services::jwt::claims_to_authenticated_user(&claims);
                let expires_nanos = claims.exp.saturating_mul(1_000_000_000);
                tracing::info!(
                    class = celnet_observability::LogClass::Security.label(),
                    scope = "celnet.auth",
                    user_id = %user.user_id,
                    email = %user.email,
                    role = ?user.role,
                    auth_provider = "deskmodal_sso",
                    "DeskModal JWT pass-through authentication succeeded"
                );
                map.insert(
                    token.to_string(),
                    Session {
                        user: user.clone(),
                        expires_nanos,
                    },
                );
                return Some((user, expires_nanos));
            }
        }

        None
    }

    /// Resolve a bearer token to its authenticated identity, or `None` if the
    /// token is unknown or expired.
    #[must_use]
    pub fn validate(&self, token: &str) -> Option<AuthenticatedUser> {
        self.get_session(token).map(|(u, _)| u)
    }

    /// Invalidate a single session (logout). Returns whether a session was
    /// present.
    pub fn logout(&self, token: &str) -> bool {
        self.inner
            .lock()
            .expect("session registry mutex poisoned")
            .remove(token)
            .is_some()
    }

    /// Invalidate **every** session belonging to a user — called whenever an
    /// administrative change alters that user's authority (disable, delete,
    /// role/desk change, password reset) so no stale session survives it. Returns
    /// the number of sessions revoked.
    pub fn revoke_user(&self, user_id: &str) -> usize {
        let mut map = self.inner.lock().expect("session registry mutex poisoned");
        let before = map.len();
        map.retain(|_, s| s.user.user_id != user_id);
        before - map.len()
    }

    /// Drop all expired sessions; returns the number removed. Validation already
    /// expires lazily, so this is a housekeeping convenience.
    pub fn sweep_expired(&self) -> usize {
        let now = self.clock.now_nanos();
        let mut map = self.inner.lock().expect("session registry mutex poisoned");
        let before = map.len();
        map.retain(|_, s| s.expires_nanos > now);
        before - map.len()
    }

    /// The number of live (still-mapped) sessions — for tests and metrics.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.inner
            .lock()
            .expect("session registry mutex poisoned")
            .len()
    }
}

/// Mint a 256-bit OS-CSPRNG bearer token, lowercase-hex encoded (64 chars).
fn mint_token() -> Result<String, String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("csprng: {e}"))?;
    let mut out = String::with_capacity(TOKEN_BYTES * 2);
    for b in bytes {
        out.push(char::from_digit(u32::from(b >> 4), 16).unwrap());
        out.push(char::from_digit(u32::from(b & 0x0f), 16).unwrap());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> AuthenticatedUser {
        AuthenticatedUser {
            user_id: "alice".into(),
            email: "alice@celnet.com".into(),
            display_name: "Alice".into(),
            role: Role::Trader,
            desk_ids: vec!["g10".into()],
            all_desks: false,
            role_caps: default_trader_bundle(),
            cap_grants: Vec::new(),
            cap_denies: Vec::new(),
        }
    }

    #[test]
    fn issued_token_validates_to_the_user() {
        let reg = SessionRegistry::new(Clock::manual(0));
        let token = reg.issue(alice()).unwrap().token;
        // 256-bit hex.
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        let who = reg.validate(&token).expect("fresh token validates");
        assert_eq!(who, alice());
        assert_eq!(reg.active_count(), 1);
    }

    #[test]
    fn distinct_tokens_per_issue() {
        let reg = SessionRegistry::new(Clock::manual(0));
        let t1 = reg.issue(alice()).unwrap().token;
        let t2 = reg.issue(alice()).unwrap().token;
        assert_ne!(t1, t2, "each session must get a unique token");
        assert_eq!(reg.active_count(), 2);
    }

    #[test]
    fn unknown_token_is_rejected() {
        let reg = SessionRegistry::new(Clock::manual(0));
        assert!(reg.validate("deadbeef").is_none());
    }

    #[test]
    fn expired_token_is_rejected_and_dropped() {
        let clock = Clock::manual(0);
        let reg = SessionRegistry::with_ttl(clock.clone(), 1_000);
        let token = reg.issue(alice()).unwrap().token;
        assert!(reg.validate(&token).is_some());
        clock.advance(1_001);
        assert!(reg.validate(&token).is_none(), "past TTL ⇒ rejected");
        assert_eq!(
            reg.active_count(),
            0,
            "expired token is dropped on validate"
        );
    }

    #[test]
    fn logout_invalidates_the_session() {
        let reg = SessionRegistry::new(Clock::manual(0));
        let token = reg.issue(alice()).unwrap().token;
        assert!(reg.logout(&token));
        assert!(!reg.logout(&token), "second logout is a no-op");
        assert!(reg.validate(&token).is_none());
    }

    #[test]
    fn revoke_user_kills_all_their_sessions_only() {
        let reg = SessionRegistry::new(Clock::manual(0));
        let a1 = reg.issue(alice()).unwrap().token;
        let a2 = reg.issue(alice()).unwrap().token;
        let bob = AuthenticatedUser {
            user_id: "bob".into(),
            ..alice()
        };
        let b1 = reg.issue(bob).unwrap().token;
        assert_eq!(reg.revoke_user("alice"), 2);
        assert!(reg.validate(&a1).is_none());
        assert!(reg.validate(&a2).is_none());
        assert!(
            reg.validate(&b1).is_some(),
            "bob's session survives alice's revocation"
        );
    }

    #[test]
    fn admin_holds_grant_all_caps_trader_holds_no_admin_cap() {
        use celnet_entitlements::{Action, AssetClass, Capability};
        let admin = AuthenticatedUser {
            role: Role::Admin,
            ..alice()
        };
        let admin_caps = admin.capabilities();
        assert!(admin_caps.is_grant_all());
        assert!(admin_caps.allows(Capability::new(Action::Administer, AssetClass::FxOptions)));

        // A trader can deal on both asset classes but cannot administer.
        let trader_caps = alice().capabilities(); // alice is a Trader
        assert!(trader_caps.allows(Capability::new(Action::Execute, AssetClass::FxOptions)));
        assert!(trader_caps.allows(Capability::new(Action::RfqRespond, AssetClass::FixedIncome)));
        assert!(!trader_caps.allows(Capability::new(Action::Administer, AssetClass::FxOptions)));
        assert!(!trader_caps.allows(Capability::new(Action::Administer, AssetClass::FixedIncome)));
    }

    /// The per-user overlay layers on the role bundle: a grant widens authority
    /// beyond the role, and a deny narrows it (deny-wins) — separation of duties.
    #[test]
    fn per_user_overlay_widens_and_narrows_role_bundle() {
        use celnet_entitlements::{Action, AssetClass, Capability};
        let fi_admin = Capability::new(Action::Administer, AssetClass::FixedIncome);
        let fi_exec = Capability::new(Action::Execute, AssetClass::FixedIncome);

        // A trader granted Administer·FixedIncome (not in the role bundle) and denied
        // Execute·FixedIncome (which the role bundle grants).
        let user = AuthenticatedUser {
            cap_grants: vec![fi_admin],
            cap_denies: vec![fi_exec],
            ..alice()
        };
        let caps = user.capabilities();
        assert!(
            caps.allows(fi_admin),
            "explicit grant widens beyond the role"
        );
        assert!(
            !caps.allows(fi_exec),
            "explicit deny wins over the role grant"
        );
        // Untouched capabilities still follow the role bundle.
        assert!(caps.allows(Capability::new(Action::Execute, AssetClass::FxOptions)));
    }

    /// `from_user` snapshots the persisted overlay into the session identity.
    #[test]
    fn from_user_snapshots_capability_overlay() {
        use crate::config::identity::PermissionGrant;
        use celnet_entitlements::{Action, AssetClass, Capability};
        let fi_admin = Capability::new(Action::Administer, AssetClass::FixedIncome);
        let user = UserDef {
            id: "u".into(),
            email: "u@celnet.com".into(),
            display_name: "U".into(),
            role: Role::Trader,
            desk_ids: Vec::new(),
            all_desks: false,
            password_hash: "x".into(),
            disabled: false,
            capability_grants: vec![PermissionGrant::of(fi_admin)],
            capability_denies: Vec::new(),
        };
        let who = AuthenticatedUser::from_user(&user);
        assert_eq!(who.cap_grants, vec![fi_admin]);
        assert!(who.capabilities().allows(fi_admin));
    }

    #[test]
    fn sweep_drops_only_expired() {
        let clock = Clock::manual(0);
        let reg = SessionRegistry::with_ttl(clock.clone(), 1_000);
        let _old = reg.issue(alice()).unwrap().token;
        clock.advance(2_000);
        let fresh = reg.issue(alice()).unwrap().token;
        assert_eq!(reg.sweep_expired(), 1, "only the expired session is swept");
        assert!(reg.validate(&fresh).is_some());
    }
}
