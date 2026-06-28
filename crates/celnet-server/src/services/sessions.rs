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

use celnet_entitlements::{Action, AssetClass, CapabilitySet};

use crate::clock::Clock;
use crate::config::identity::{Role, UserDef};

/// The action bundle a desk `Trader` holds by default on **each** asset class:
/// every action except [`Action::Administer`]. This is the slice-1 *role-derived*
/// default; admin-editable per-user grants/denials layer on top in a later slice
/// (`docs/plan/PERMISSIONS-ADMINISTRATION-REQUIREMENT.md` §3.3/§10).
const TRADER_ACTIONS: [Action; 8] = [
    Action::View,
    Action::Price,
    Action::QuoteRespond,
    Action::RfqRespond,
    Action::IoiRespond,
    Action::Stream,
    Action::Execute,
    Action::Book,
];

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
    /// The desk the user belonged to at login, if any.
    pub desk_id: Option<String>,
}

impl AuthenticatedUser {
    /// Snapshot a stored user into a session identity.
    #[must_use]
    pub fn from_user(user: &UserDef) -> Self {
        Self {
            user_id: user.id.clone(),
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            role: user.role,
            desk_id: user.desk_id.clone(),
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
    ///   classes — including [`Action::Administer`]);
    /// * [`Role::Trader`] ⇒ every non-admin action ([`TRADER_ACTIONS`]) on **both**
    ///   FX options and fixed income, scoped at the resource edge by
    ///   [`super::access::ResolvedCaller::desk_scope`].
    ///
    /// This is the role-derived resolution; admin-editable per-user grants/denials
    /// (deny-wins) layer on in a later slice without changing this signature — the
    /// returned set is already the single source the access boundary consults.
    #[must_use]
    pub fn capabilities(&self) -> CapabilitySet {
        match self.role {
            Role::Admin => CapabilitySet::grant_all(),
            Role::Trader => CapabilitySet::empty()
                .grant_actions(&TRADER_ACTIONS, AssetClass::FxOptions)
                .grant_actions(&TRADER_ACTIONS, AssetClass::FixedIncome),
        }
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

    /// Resolve a bearer token to its authenticated identity, or `None` if the
    /// token is unknown or expired. An expired token is dropped here (lazy
    /// expiry) so a stale credential cannot be replayed.
    #[must_use]
    pub fn validate(&self, token: &str) -> Option<AuthenticatedUser> {
        let now = self.clock.now_nanos();
        let mut map = self.inner.lock().expect("session registry mutex poisoned");
        match map.get(token) {
            Some(s) if s.expires_nanos > now => Some(s.user.clone()),
            Some(_) => {
                map.remove(token);
                None
            }
            None => None,
        }
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
            desk_id: Some("g10".into()),
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
