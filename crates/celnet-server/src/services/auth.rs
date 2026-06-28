//! The `AuthService` edge: **server-enforced authentication** plus user / desk
//! administration over the persisted [`IdentityStore`].
//!
//! Unlike the entitlement boundary ([`super::access`]), which authorizes a
//! *client-asserted* principal, this service authenticates: `Login` checks a
//! password against the Argon2id hash and mints an unguessable session token; every
//! other RPC carries that token and the **server** resolves it
//! ([`SessionRegistry::validate`]) to decide who the caller is. Administrative RPCs
//! additionally require the resolved identity to be an admin
//! ([`AuthEdge::require_admin`]) — a trader's token is rejected with
//! `permission_denied`.
//!
//! # Consistency & safety
//!
//! * **Persist-before-commit.** A mutation builds a candidate [`IdentityStore`],
//!   saves it atomically, and only then swaps it into memory — so a failed write
//!   never leaves disk and memory disagreeing.
//! * **No admin lockout.** The last enabled admin cannot be deleted, demoted or
//!   disabled (`failed_precondition`), so the edge can never become unadministrable.
//! * **Sessions follow authority.** Any change to a user's role, desk, disabled
//!   flag or password revokes that user's live sessions, so a stale token can never
//!   outlive the authority it was minted under.
//! * **Passwords never block the runtime.** Argon2 hashing/verification is memory-
//!   hard (deliberately slow); it runs on `spawn_blocking`, off the async worker.

// `tonic::Status` is the contract's typed error; its size is the wire library's
// choice (the same allowance every service module carries).
#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use celnet_proto::auth_service_server::AuthService;
use celnet_proto::{
    CreateDeskRequest, CreateDeskResponse, CreateUserRequest, CreateUserResponse,
    DeleteDeskRequest, DeleteDeskResponse, DeleteUserRequest, DeleteUserResponse, DeskDesc,
    ListDesksRequest, ListDesksResponse, ListUsersRequest, ListUsersResponse, LoginRequest,
    LoginResponse, LogoutRequest, LogoutResponse, ResetPasswordRequest, ResetPasswordResponse,
    UpdateUserRequest, UpdateUserResponse, UserDesc, UserRole,
};
use tonic::{Request, Response, Status};

use crate::clock::Clock;
use crate::config::identity::{
    DeskDef, IdentityStore, Role, UserDef, hash_password, mint_desk_id, mint_user_id,
    verify_password,
};
use crate::readiness::ReadinessGate;
use crate::services::sessions::{AuthenticatedUser, SessionRegistry};

/// The minimum acceptable password length for a created or reset password. A
/// privileged trading-edge account warrants more than the bare NIST floor; the
/// seeded default is rotated to something at least this long.
const MIN_PASSWORD_LEN: usize = 12;

/// Consecutive failed logins for one email before it is temporarily locked.
const MAX_LOGIN_FAILS: u32 = 10;
/// How long an email stays locked after [`MAX_LOGIN_FAILS`] failures (nanos).
const LOGIN_LOCK_NANOS: i64 = 15 * 60 * 1_000_000_000;

/// One email's recent failed-login state.
#[derive(Debug, Clone, Copy, Default)]
struct ThrottleEntry {
    fails: u32,
    locked_until: i64,
}

/// A per-email login throttle that bounds password brute-force. Argon2's cost
/// already rate-limits a single guesser; this caps a determined attacker who
/// parallelizes attempts against a known email. Keyed by lowercased email; purely
/// in-memory (reset on restart), stamped off the shared edge clock.
#[derive(Debug)]
struct LoginThrottle {
    inner: Mutex<HashMap<String, ThrottleEntry>>,
    clock: Clock,
}

impl LoginThrottle {
    fn new(clock: Clock) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            clock,
        }
    }

    /// If the email is currently locked, the remaining lock window in seconds;
    /// otherwise `None` (the attempt may proceed).
    fn locked_for(&self, email_key: &str) -> Option<i64> {
        let now = self.clock.now_nanos();
        let map = self.inner.lock().expect("login throttle mutex poisoned");
        map.get(email_key)
            .filter(|e| e.locked_until > now)
            .map(|e| (e.locked_until - now) / 1_000_000_000 + 1)
    }

    /// Record a failed attempt; locks the email once it crosses the threshold.
    fn record_failure(&self, email_key: &str) {
        let now = self.clock.now_nanos();
        let mut map = self.inner.lock().expect("login throttle mutex poisoned");
        let e = map.entry(email_key.to_string()).or_default();
        e.fails = e.fails.saturating_add(1);
        if e.fails >= MAX_LOGIN_FAILS {
            e.locked_until = now.saturating_add(LOGIN_LOCK_NANOS);
            e.fails = 0;
        }
    }

    /// Clear an email's failure state on a successful login.
    fn record_success(&self, email_key: &str) {
        self.inner
            .lock()
            .expect("login throttle mutex poisoned")
            .remove(email_key);
    }
}

/// The `AuthService` edge over the shared identity store + session registry.
///
/// Holds the live [`IdentityStore`] behind a [`Mutex`] (administrative mutation is
/// rare and control-plane), the file path it persists to, the [`SessionRegistry`]
/// that mints/validates bearer tokens, and the [`ReadinessGate`] so auth ops are
/// refused while the edge is starting or draining (like every other service).
pub struct AuthEdge {
    identity: Arc<Mutex<IdentityStore>>,
    path: PathBuf,
    sessions: Arc<SessionRegistry>,
    gate: Arc<ReadinessGate>,
    throttle: LoginThrottle,
}

impl AuthEdge {
    /// Construct the auth edge over the shared identity store, its persist path,
    /// the session registry, the readiness gate, and the edge clock (used to time
    /// the login brute-force throttle).
    #[must_use]
    pub fn new(
        identity: Arc<Mutex<IdentityStore>>,
        path: PathBuf,
        sessions: Arc<SessionRegistry>,
        gate: Arc<ReadinessGate>,
        clock: Clock,
    ) -> Self {
        Self {
            identity,
            path,
            sessions,
            gate,
            throttle: LoginThrottle::new(clock),
        }
    }

    /// Refuse work unless the edge is ready (mirrors every other service).
    fn require_ready(&self) -> Result<(), Status> {
        if self.gate.is_ready() {
            Ok(())
        } else {
            Err(Status::unavailable(
                "edge not ready (starting or draining); steer to the active instance",
            ))
        }
    }

    /// Resolve a bearer token to its authenticated identity, or
    /// `unauthenticated`.
    fn authenticate(&self, token: &str) -> Result<AuthenticatedUser, Status> {
        self.sessions
            .validate(token)
            .ok_or_else(|| Status::unauthenticated("invalid or expired session token"))
    }

    /// Resolve a bearer token and require it to carry administrative authority.
    fn require_admin(&self, token: &str) -> Result<AuthenticatedUser, Status> {
        let who = self.authenticate(token)?;
        if who.is_admin() {
            Ok(who)
        } else {
            Err(Status::permission_denied(
                "administrative privilege required",
            ))
        }
    }

    /// Lock the identity store. The mutex only guards rare control-plane mutation,
    /// never a latency budget.
    fn lock(&self) -> std::sync::MutexGuard<'_, IdentityStore> {
        self.identity.lock().expect("identity store mutex poisoned")
    }

    /// Persist a candidate store atomically, then commit it into `guard`. On a
    /// write failure nothing is committed, so disk and memory never diverge.
    fn persist_and_commit(
        &self,
        guard: &mut IdentityStore,
        next: IdentityStore,
    ) -> Result<(), Status> {
        next.save(&self.path)
            .map_err(|e| Status::internal(format!("persist identity store: {e}")))?;
        *guard = next;
        Ok(())
    }
}

#[tonic::async_trait]
impl AuthService for AuthEdge {
    async fn login(
        &self,
        request: Request<LoginRequest>,
    ) -> Result<Response<LoginResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();

        // Brute-force throttle: refuse early while this email is locked out, so a
        // parallelized guesser cannot keep spending Argon2 verifications.
        let email_key = req.email.trim().to_ascii_lowercase();
        if let Some(retry_secs) = self.throttle.locked_for(&email_key) {
            return Err(Status::resource_exhausted(format!(
                "too many failed login attempts; retry in {retry_secs}s"
            )));
        }

        // Look the candidate user up and copy out only what verification needs,
        // dropping the lock before the (slow) Argon2 check.
        let candidate = {
            let store = self.lock();
            store.user_by_email(&req.email).cloned()
        };

        // Verify off the async worker. To blunt user-enumeration timing, an absent
        // (or disabled) account still runs a verification against a fixed dummy
        // hash, so the response time does not reveal whether the email exists.
        let (authed_user, ok) = match candidate {
            Some(u) if !u.disabled => {
                let ok = verify_async(u.password_hash.clone(), req.password.clone()).await;
                (Some(u), ok)
            }
            _ => {
                let _ = verify_async(dummy_hash().to_string(), req.password.clone()).await;
                (None, false)
            }
        };

        let Some(user) = authed_user.filter(|_| ok) else {
            self.throttle.record_failure(&email_key);
            return Err(Status::unauthenticated("invalid email or password"));
        };
        self.throttle.record_success(&email_key);

        let issued = self
            .sessions
            .issue(AuthenticatedUser::from_user(&user))
            .map_err(|e| Status::internal(format!("issue session: {e}")))?;
        Ok(Response::new(LoginResponse {
            session_token: issued.token,
            user: Some(user_to_wire(&user)),
            expires_nanos: issued.expires_nanos,
            correlation_id: req.correlation_id,
        }))
    }

    async fn logout(
        &self,
        request: Request<LogoutRequest>,
    ) -> Result<Response<LogoutResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let ended = self.sessions.logout(&req.session_token);
        Ok(Response::new(LogoutResponse {
            ended,
            correlation_id: req.correlation_id,
        }))
    }

    async fn list_users(
        &self,
        request: Request<ListUsersRequest>,
    ) -> Result<Response<ListUsersResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;
        let users = self.lock().users.iter().map(user_to_wire).collect();
        Ok(Response::new(ListUsersResponse {
            users,
            correlation_id: req.correlation_id,
        }))
    }

    async fn create_user(
        &self,
        request: Request<CreateUserRequest>,
    ) -> Result<Response<CreateUserResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let email = req.email.trim().to_string();
        if !valid_email(&email) {
            return Err(Status::invalid_argument("a valid email is required"));
        }
        let display_name = req.display_name.trim().to_string();
        if display_name.is_empty() {
            return Err(Status::invalid_argument("display name is required"));
        }
        check_password_strength(&req.password)?;
        let role = role_from_wire(req.role)?;
        let desk_id = normalize_desk(req.desk_id);

        // Hash off the async worker before taking the lock.
        let password_hash = hash_async(req.password.clone())
            .await
            .map_err(|e| Status::internal(format!("hash password: {e}")))?;

        let mut guard = self.lock();
        if guard.user_by_email(&email).is_some() {
            return Err(Status::already_exists(format!(
                "a user with email `{email}` already exists"
            )));
        }
        if let Some(desk) = &desk_id
            && guard.desk(desk).is_none()
        {
            return Err(Status::failed_precondition(format!(
                "no desk with id `{desk}`"
            )));
        }
        let new_user = UserDef {
            id: mint_user_id(&email, &guard.users),
            email,
            display_name,
            role,
            desk_id,
            password_hash,
            disabled: false,
            // A new account starts with no per-user overlay — pure role-derived
            // capabilities until an admin grants/denies specific ones.
            capability_grants: Vec::new(),
            capability_denies: Vec::new(),
        };
        let mut next = guard.clone();
        next.users.push(new_user.clone());
        self.persist_and_commit(&mut guard, next)?;
        Ok(Response::new(CreateUserResponse {
            user: Some(user_to_wire(&new_user)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn update_user(
        &self,
        request: Request<UpdateUserRequest>,
    ) -> Result<Response<UpdateUserResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let display_name = req.display_name.trim().to_string();
        if display_name.is_empty() {
            return Err(Status::invalid_argument("display name is required"));
        }
        let role = role_from_wire(req.role)?;
        let desk_id = normalize_desk(req.desk_id);

        let mut guard = self.lock();
        let Some(old) = guard.user(&req.id).cloned() else {
            return Err(Status::not_found(format!("no user with id `{}`", req.id)));
        };
        if let Some(desk) = &desk_id
            && guard.desk(desk).is_none()
        {
            return Err(Status::failed_precondition(format!(
                "no desk with id `{desk}`"
            )));
        }
        let becomes_enabled_admin = role.is_admin() && !req.disabled;
        if !becomes_enabled_admin && was_sole_enabled_admin(&guard, &old) {
            return Err(Status::failed_precondition(
                "cannot demote or disable the last remaining admin",
            ));
        }

        let updated = UserDef {
            id: old.id.clone(),
            email: old.email.clone(),
            display_name,
            role,
            desk_id,
            password_hash: old.password_hash.clone(),
            disabled: req.disabled,
            // Preserve the per-user capability overlay — this RPC edits identity
            // (role/desk/disabled), never the overlay, so it must not silently wipe
            // it. Overlay editing is its own admin RPC (slice 3b).
            capability_grants: old.capability_grants.clone(),
            capability_denies: old.capability_denies.clone(),
        };
        let authority_changed = updated.role != old.role
            || updated.desk_id != old.desk_id
            || updated.disabled != old.disabled;

        let mut next = guard.clone();
        if let Some(slot) = next.users.iter_mut().find(|u| u.id == updated.id) {
            *slot = updated.clone();
        }
        self.persist_and_commit(&mut guard, next)?;
        drop(guard);

        // A change to authority must not be outlived by an existing session.
        if authority_changed {
            self.sessions.revoke_user(&updated.id);
        }
        Ok(Response::new(UpdateUserResponse {
            user: Some(user_to_wire(&updated)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn delete_user(
        &self,
        request: Request<DeleteUserRequest>,
    ) -> Result<Response<DeleteUserResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let mut guard = self.lock();
        let Some(target) = guard.user(&req.id).cloned() else {
            return Ok(Response::new(DeleteUserResponse {
                removed: false,
                correlation_id: req.correlation_id,
            }));
        };
        if was_sole_enabled_admin(&guard, &target) {
            return Err(Status::failed_precondition(
                "cannot delete the last remaining admin",
            ));
        }
        let mut next = guard.clone();
        next.users.retain(|u| u.id != req.id);
        self.persist_and_commit(&mut guard, next)?;
        drop(guard);

        self.sessions.revoke_user(&req.id);
        Ok(Response::new(DeleteUserResponse {
            removed: true,
            correlation_id: req.correlation_id,
        }))
    }

    async fn reset_password(
        &self,
        request: Request<ResetPasswordRequest>,
    ) -> Result<Response<ResetPasswordResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        check_password_strength(&req.new_password)?;
        // Confirm the target exists before paying for a hash.
        if self.lock().user(&req.id).is_none() {
            return Err(Status::not_found(format!("no user with id `{}`", req.id)));
        }
        let password_hash = hash_async(req.new_password.clone())
            .await
            .map_err(|e| Status::internal(format!("hash password: {e}")))?;

        let mut guard = self.lock();
        if guard.user(&req.id).is_none() {
            return Err(Status::not_found(format!("no user with id `{}`", req.id)));
        }
        let mut next = guard.clone();
        if let Some(slot) = next.users.iter_mut().find(|u| u.id == req.id) {
            slot.password_hash = password_hash;
        }
        self.persist_and_commit(&mut guard, next)?;
        drop(guard);

        // Force re-login with the new credential everywhere.
        self.sessions.revoke_user(&req.id);
        Ok(Response::new(ResetPasswordResponse {
            correlation_id: req.correlation_id,
        }))
    }

    async fn list_desks(
        &self,
        request: Request<ListDesksRequest>,
    ) -> Result<Response<ListDesksResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;
        let desks = self.lock().desks.iter().map(desk_to_wire).collect();
        Ok(Response::new(ListDesksResponse {
            desks,
            correlation_id: req.correlation_id,
        }))
    }

    async fn create_desk(
        &self,
        request: Request<CreateDeskRequest>,
    ) -> Result<Response<CreateDeskResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let name = req.name.trim().to_string();
        if name.is_empty() {
            return Err(Status::invalid_argument("desk name is required"));
        }
        let mut guard = self.lock();
        if guard
            .desks
            .iter()
            .any(|d| d.name.eq_ignore_ascii_case(&name))
        {
            return Err(Status::already_exists(format!(
                "a desk named `{name}` already exists"
            )));
        }
        let desk = DeskDef {
            id: mint_desk_id(&name, &guard.desks),
            name,
        };
        let mut next = guard.clone();
        next.desks.push(desk.clone());
        self.persist_and_commit(&mut guard, next)?;
        Ok(Response::new(CreateDeskResponse {
            desk: Some(desk_to_wire(&desk)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn delete_desk(
        &self,
        request: Request<DeleteDeskRequest>,
    ) -> Result<Response<DeleteDeskResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let mut guard = self.lock();
        if guard.desk(&req.id).is_none() {
            return Ok(Response::new(DeleteDeskResponse {
                removed: false,
                correlation_id: req.correlation_id,
            }));
        }
        // Members of the deleted desk become unassigned; their snapshots change, so
        // their sessions are revoked after commit.
        let affected: Vec<String> = guard
            .users
            .iter()
            .filter(|u| u.desk_id.as_deref() == Some(req.id.as_str()))
            .map(|u| u.id.clone())
            .collect();
        let mut next = guard.clone();
        next.desks.retain(|d| d.id != req.id);
        for u in &mut next.users {
            if u.desk_id.as_deref() == Some(req.id.as_str()) {
                u.desk_id = None;
            }
        }
        self.persist_and_commit(&mut guard, next)?;
        drop(guard);

        for id in affected {
            self.sessions.revoke_user(&id);
        }
        Ok(Response::new(DeleteDeskResponse {
            removed: true,
            correlation_id: req.correlation_id,
        }))
    }
}

// --- wire ⇄ domain mapping -------------------------------------------------

/// Map a stored [`UserDef`] onto its wire [`UserDesc`] (never the password hash).
fn user_to_wire(u: &UserDef) -> UserDesc {
    UserDesc {
        id: u.id.clone(),
        email: u.email.clone(),
        display_name: u.display_name.clone(),
        role: role_to_wire(u.role),
        desk_id: u.desk_id.clone(),
        disabled: u.disabled,
    }
}

/// Map a stored [`DeskDef`] onto its wire [`DeskDesc`].
fn desk_to_wire(d: &DeskDef) -> DeskDesc {
    DeskDesc {
        id: d.id.clone(),
        name: d.name.clone(),
    }
}

/// The wire enum value for a domain [`Role`].
fn role_to_wire(role: Role) -> i32 {
    match role {
        Role::Admin => UserRole::Admin as i32,
        Role::Trader => UserRole::Trader as i32,
    }
}

/// Resolve a wire enum value to a domain [`Role`].
///
/// # Errors
/// `invalid_argument` for an unrecognised enum value.
fn role_from_wire(role: i32) -> Result<Role, Status> {
    match UserRole::try_from(role) {
        Ok(UserRole::Admin) => Ok(Role::Admin),
        Ok(UserRole::Trader) => Ok(Role::Trader),
        Err(_) => Err(Status::invalid_argument(format!(
            "unrecognised user role `{role}`"
        ))),
    }
}

/// Reject a password that is empty or shorter than [`MIN_PASSWORD_LEN`]. Length is
/// the single highest-signal strength control (NIST SP 800-63B); composition rules
/// are deliberately not imposed.
fn check_password_strength(password: &str) -> Result<(), Status> {
    if password.len() < MIN_PASSWORD_LEN {
        return Err(Status::invalid_argument(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    Ok(())
}

/// Normalize an optional wire desk id: a present-but-blank value means unassigned.
fn normalize_desk(desk_id: Option<String>) -> Option<String> {
    desk_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Whether `target` is the only enabled admin in the store (so removing or
/// demoting it would orphan the edge).
fn was_sole_enabled_admin(store: &IdentityStore, target: &UserDef) -> bool {
    if !target.role.is_admin() || target.disabled {
        return false;
    }
    !store
        .users
        .iter()
        .any(|u| u.id != target.id && u.role.is_admin() && !u.disabled)
}

/// A minimal email shape check: a single `@` with a non-empty local-part and a
/// dotted domain. Deliberately conservative — the email is an identifier, not a
/// deliverability guarantee.
fn valid_email(s: &str) -> bool {
    let mut parts = s.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

/// A fixed valid Argon2id PHC hash used only to equalize login timing for absent
/// or disabled accounts (user-enumeration resistance). Computed once; if the
/// CSPRNG ever fails, falls back to an empty string (verification still returns
/// `false`, which is the only property the login path depends on).
fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| hash_password("celnet-nonexistent-account").unwrap_or_default())
}

/// Run an Argon2 verification off the async worker (the hash is memory-hard).
async fn verify_async(stored_hash: String, candidate: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(&stored_hash, &candidate))
        .await
        .unwrap_or(false)
}

/// Run an Argon2 hash off the async worker.
async fn hash_async(plain: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || hash_password(&plain))
        .await
        .map_err(|e| format!("join: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Clock;

    fn role_round_trips() {
        assert_eq!(
            role_from_wire(role_to_wire(Role::Admin)).unwrap(),
            Role::Admin
        );
        assert_eq!(
            role_from_wire(role_to_wire(Role::Trader)).unwrap(),
            Role::Trader
        );
    }

    #[test]
    fn pure_helpers() {
        role_round_trips();
        assert!(valid_email("a@celnet.com"));
        assert!(!valid_email("nope"));
        assert!(!valid_email("a@b"));
        assert!(!valid_email("a@@b.com"));
        assert_eq!(normalize_desk(Some("  ".into())), None);
        assert_eq!(normalize_desk(Some(" g10 ".into())), Some("g10".into()));
        assert_eq!(normalize_desk(None), None);
        assert!(role_from_wire(99).is_err());
        // Password strength: empty and short rejected; >= MIN accepted.
        assert!(check_password_strength("").is_err());
        assert!(check_password_strength("short").is_err());
        assert!(check_password_strength("a-strong-enough-secret").is_ok());
    }

    #[test]
    fn login_throttle_locks_then_recovers() {
        let clock = Clock::manual(0);
        let throttle = LoginThrottle::new(clock.clone());
        // Below the threshold: never locked.
        for _ in 0..(MAX_LOGIN_FAILS - 1) {
            throttle.record_failure("a@celnet.com");
            assert!(throttle.locked_for("a@celnet.com").is_none());
        }
        // The threshold failure locks the email.
        throttle.record_failure("a@celnet.com");
        assert!(throttle.locked_for("a@celnet.com").is_some());
        // A different email is unaffected.
        assert!(throttle.locked_for("b@celnet.com").is_none());
        // The lock expires after its window.
        clock.advance(LOGIN_LOCK_NANOS + 1);
        assert!(throttle.locked_for("a@celnet.com").is_none());
        // A success clears accumulated failures.
        throttle.record_failure("a@celnet.com");
        throttle.record_success("a@celnet.com");
        assert!(throttle.locked_for("a@celnet.com").is_none());
    }

    #[test]
    fn sole_admin_guard() {
        let mut store = IdentityStore::default();
        store.ensure_seed_admin().unwrap();
        let admin = store.users[0].clone();
        assert!(
            was_sole_enabled_admin(&store, &admin),
            "the only admin is sole"
        );
        // Add a second admin ⇒ no longer sole.
        store.users.push(UserDef {
            id: "a2".into(),
            email: "a2@celnet.com".into(),
            display_name: "A2".into(),
            role: Role::Admin,
            desk_id: None,
            password_hash: "x".into(),
            disabled: false,
            capability_grants: Vec::new(),
            capability_denies: Vec::new(),
        });
        assert!(!was_sole_enabled_admin(&store, &admin));
        // A trader is never "the sole admin".
        let trader = UserDef {
            id: "t".into(),
            email: "t@celnet.com".into(),
            display_name: "T".into(),
            role: Role::Trader,
            desk_id: None,
            password_hash: "x".into(),
            disabled: false,
            capability_grants: Vec::new(),
            capability_denies: Vec::new(),
        };
        assert!(!was_sole_enabled_admin(&store, &trader));
    }

    /// Build an edge over a temp identity file seeded with the default admin.
    fn edge(tag: &str) -> (AuthEdge, PathBuf, Arc<SessionRegistry>) {
        let dir = std::env::temp_dir().join("celnet-auth-edge");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("identity-{}-{}.json", std::process::id(), tag));
        let _ = std::fs::remove_file(&path);
        let mut store = IdentityStore::default();
        store.ensure_seed_admin().unwrap();
        store.save(&path).unwrap();
        let identity = Arc::new(Mutex::new(store));
        let clock = Clock::system();
        let sessions = Arc::new(SessionRegistry::new(clock.clone()));
        let gate = Arc::new(ReadinessGate::new());
        gate.mark_ready();
        (
            AuthEdge::new(identity, path.clone(), sessions.clone(), gate, clock),
            path,
            sessions,
        )
    }

    async fn login(edge: &AuthEdge, email: &str, password: &str) -> Result<LoginResponse, Status> {
        edge.login(Request::new(LoginRequest {
            email: email.into(),
            password: password.into(),
            correlation_id: Some(1),
        }))
        .await
        .map(Response::into_inner)
    }

    #[tokio::test]
    async fn seed_admin_logs_in_and_token_validates() {
        let (edge, path, _s) = edge("login-ok");
        let resp = login(&edge, "admin@celnet.com", "password").await.unwrap();
        assert!(!resp.session_token.is_empty());
        let user = resp.user.unwrap();
        assert_eq!(user.email, "admin@celnet.com");
        assert_eq!(user.role, UserRole::Admin as i32);
        // The token authorizes an admin RPC.
        let list = edge
            .list_users(Request::new(ListUsersRequest {
                session_token: resp.session_token,
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(list.users.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn wrong_password_and_unknown_email_are_rejected() {
        let (edge, path, _s) = edge("login-bad");
        assert_eq!(
            login(&edge, "admin@celnet.com", "nope")
                .await
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated
        );
        assert_eq!(
            login(&edge, "ghost@celnet.com", "password")
                .await
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn trader_token_cannot_administer() {
        let (edge, path, _s) = edge("trader-gate");
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        // Admin creates a trader.
        let created = edge
            .create_user(Request::new(CreateUserRequest {
                session_token: admin.session_token.clone(),
                email: "jane@celnet.com".into(),
                display_name: "Jane".into(),
                role: UserRole::Trader as i32,
                desk_id: None,
                password: "trader-pw-123".into(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(created.user.unwrap().email, "jane@celnet.com");

        let jane = login(&edge, "jane@celnet.com", "trader-pw-123")
            .await
            .unwrap();
        // The trader's token is rejected by an admin RPC.
        let denied = edge
            .list_users(Request::new(ListUsersRequest {
                session_token: jane.session_token,
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(denied.code(), tonic::Code::PermissionDenied);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn reset_password_revokes_and_rotates() {
        let (edge, path, sessions) = edge("reset");
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        let admin_id = admin.user.unwrap().id;
        edge.reset_password(Request::new(ResetPasswordRequest {
            session_token: admin.session_token.clone(),
            id: admin_id.clone(),
            new_password: "fresh-secret".into(),
            correlation_id: None,
        }))
        .await
        .unwrap();
        // The old session is revoked; the old password no longer works.
        assert!(sessions.validate(&admin.session_token).is_none());
        assert_eq!(
            login(&edge, "admin@celnet.com", "password")
                .await
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated
        );
        // The new password works.
        assert!(
            login(&edge, "admin@celnet.com", "fresh-secret")
                .await
                .is_ok()
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn last_admin_cannot_be_demoted_or_deleted() {
        let (edge, path, _s) = edge("last-admin");
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        let admin_id = admin.user.unwrap().id;
        // Demote the only admin ⇒ rejected.
        let demote = edge
            .update_user(Request::new(UpdateUserRequest {
                session_token: admin.session_token.clone(),
                id: admin_id.clone(),
                display_name: "Administrator".into(),
                role: UserRole::Trader as i32,
                desk_id: None,
                disabled: false,
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(demote.code(), tonic::Code::FailedPrecondition);
        // Delete the only admin ⇒ rejected.
        let del = edge
            .delete_user(Request::new(DeleteUserRequest {
                session_token: admin.session_token.clone(),
                id: admin_id,
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(del.code(), tonic::Code::FailedPrecondition);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn desk_create_assign_and_delete_unassigns() {
        let (edge, path, _s) = edge("desk");
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        let tok = admin.session_token.clone();
        let desk = edge
            .create_desk(Request::new(CreateDeskRequest {
                session_token: tok.clone(),
                name: "G10 Options".into(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .desk
            .unwrap();
        // Create a trader on that desk.
        let jane = edge
            .create_user(Request::new(CreateUserRequest {
                session_token: tok.clone(),
                email: "jane@celnet.com".into(),
                display_name: "Jane".into(),
                role: UserRole::Trader as i32,
                desk_id: Some(desk.id.clone()),
                password: "trader-pw-123".into(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .user
            .unwrap();
        assert_eq!(jane.desk_id.as_deref(), Some(desk.id.as_str()));
        // Delete the desk ⇒ the trader is unassigned.
        edge.delete_desk(Request::new(DeleteDeskRequest {
            session_token: tok.clone(),
            id: desk.id.clone(),
            correlation_id: None,
        }))
        .await
        .unwrap();
        let users = edge
            .list_users(Request::new(ListUsersRequest {
                session_token: tok,
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .users;
        let jane_after = users.into_iter().find(|u| u.id == jane.id).unwrap();
        assert_eq!(
            jane_after.desk_id, None,
            "deleting a desk unassigns its members"
        );
        let _ = std::fs::remove_file(&path);
    }
}
