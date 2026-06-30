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

use celnet_entitlements::{Action, AssetClass, Capability};
use celnet_proto::auth_service_server::AuthService;
use celnet_proto::{
    BookDesc, BrokenDate, BuildCurveRequest, CalibratedCurve, CalibratedCurvePoint, CapabilityDesc,
    CreateBookRequest, CreateBookResponse, CreateDeskRequest,
    CreateDeskResponse, CreateEntityRequest, CreateEntityResponse, CreateInstrumentRequest,
    CreateInstrumentResponse, CreateUserRequest, CreateUserResponse, DeleteBookRequest,
    DeleteBookResponse, DeleteDeskRequest, DeleteDeskResponse, DeleteEntityRequest,
    DeleteEntityResponse, DeleteInstrumentRequest, DeleteInstrumentResponse, DeleteUserRequest,
    DeleteUserResponse, DeskDesc, EntityDesc, GetInstrumentRequest, GetInstrumentResponse,
    GetRoleCapabilitiesRequest, GetRoleCapabilitiesResponse, GetUserCapabilitiesRequest,
    GetUserCapabilitiesResponse, ListBooksRequest, ListBooksResponse, ListDesksRequest,
    ListDesksResponse, ListEntitiesRequest, ListEntitiesResponse, ListInstrumentsRequest,
    ListInstrumentsResponse, ListUsersRequest, ListUsersResponse, LoginRequest, LoginResponse,
    LogoutRequest, LogoutResponse, ResetPasswordRequest, ResetPasswordResponse,
    SetRoleCapabilitiesRequest, SetRoleCapabilitiesResponse, SetUserCapabilitiesRequest,
    SetUserCapabilitiesResponse, UpdateBookRequest, UpdateBookResponse, UpdateEntityRequest,
    UpdateEntityResponse, UpdateInstrumentRequest, UpdateInstrumentResponse, UpdateUserRequest,
    UpdateUserResponse, UserDesc, UserRole,
};
use celnet_rates::bootstrap_curve;
use tonic::{Request, Response, Status};

use crate::clock::Clock;
use crate::config::identity::{
    BookDef, DeskDef, EntityDef, IdentityStore, PermissionGrant, Role, UserDef, hash_password,
    mint_desk_id, mint_user_id, verify_password,
};
use crate::config::curve_calibration::{CurveCalibrationError, calibration_set};
use crate::config::reference_data::{InstrumentDef, mint_instrument_id, validate_instruments};
use crate::readiness::ReadinessGate;
use crate::services::instrument_wire::{instrument_from_wire, instrument_to_wire};
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

        // Snapshot the user's resolved role base (the admin-editable per-role bundle)
        // into the session, so a narrowed/widened bundle is reflected from this login.
        let role_base = self.lock().role_base(user.role);
        let issued = self
            .sessions
            .issue(AuthenticatedUser::from_user_with_role_base(
                &user,
                role_base.clone(),
            ))
            .map_err(|e| Status::internal(format!("issue session: {e}")))?;
        Ok(Response::new(LoginResponse {
            session_token: issued.token,
            user: Some(user_to_wire(&user)),
            expires_nanos: issued.expires_nanos,
            // The caller's own resolved set, so a client can gate its own
            // affordances without an admin-only capabilities round-trip.
            capabilities: effective_caps(&user, &role_base),
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

    async fn get_user_capabilities(
        &self,
        request: Request<GetUserCapabilitiesRequest>,
    ) -> Result<Response<GetUserCapabilitiesResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let (user, role_base) = {
            let guard = self.lock();
            let user = guard
                .user(&req.id)
                .cloned()
                .ok_or_else(|| Status::not_found(format!("no user with id `{}`", req.id)))?;
            let role_base = guard.role_base(user.role);
            (user, role_base)
        };
        Ok(Response::new(GetUserCapabilitiesResponse {
            grants: overlay_to_wire(&user.capability_grants),
            denies: overlay_to_wire(&user.capability_denies),
            effective: effective_caps(&user, &role_base),
            correlation_id: req.correlation_id,
        }))
    }

    async fn set_user_capabilities(
        &self,
        request: Request<SetUserCapabilitiesRequest>,
    ) -> Result<Response<SetUserCapabilitiesResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        // Parse + validate the whole overlay up front: an unknown action/asset
        // label is rejected (`invalid_argument`), never silently dropped — the
        // overlay that lands is exactly the one the admin sent or no change at all.
        let grants = caps_from_wire(&req.grants)?;
        let denies = caps_from_wire(&req.denies)?;

        let mut guard = self.lock();
        let Some(old) = guard.user(&req.id).cloned() else {
            return Err(Status::not_found(format!("no user with id `{}`", req.id)));
        };
        let updated = UserDef {
            id: old.id.clone(),
            email: old.email.clone(),
            display_name: old.display_name.clone(),
            role: old.role,
            desk_id: old.desk_id.clone(),
            password_hash: old.password_hash.clone(),
            disabled: old.disabled,
            // The overlay is replaced wholesale (it is the full new set, not a
            // delta); identity fields are carried through untouched.
            capability_grants: grants.iter().copied().map(PermissionGrant::of).collect(),
            capability_denies: denies.iter().copied().map(PermissionGrant::of).collect(),
        };

        // The per-user RPC never edits the role bundle, so the user's role base is
        // unchanged; resolve it for the read-back before committing.
        let role_base = guard.role_base(updated.role);
        let mut next = guard.clone();
        if let Some(slot) = next.users.iter_mut().find(|u| u.id == updated.id) {
            *slot = updated.clone();
        }
        self.persist_and_commit(&mut guard, next)?;
        drop(guard);

        // The overlay changed the user's authority — revoke their live sessions so
        // a token minted under the old capabilities cannot outlive them (the same
        // rule role/desk/disabled/password changes follow).
        self.sessions.revoke_user(&updated.id);
        Ok(Response::new(SetUserCapabilitiesResponse {
            grants: overlay_to_wire(&updated.capability_grants),
            denies: overlay_to_wire(&updated.capability_denies),
            effective: effective_caps(&updated, &role_base),
            correlation_id: req.correlation_id,
        }))
    }

    async fn get_role_capabilities(
        &self,
        request: Request<GetRoleCapabilitiesRequest>,
    ) -> Result<Response<GetRoleCapabilitiesResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let role = role_from_wire(req.role)?;
        // Admin is grant-all and immutable — report the full surface, never a stored
        // bundle (the Admin role never has one).
        let caps = if role.is_admin() {
            grant_all_wire()
        } else {
            self.lock()
                .role_base(role)
                .iter()
                .copied()
                .map(cap_to_wire)
                .collect()
        };
        Ok(Response::new(GetRoleCapabilitiesResponse {
            capabilities: caps,
            correlation_id: req.correlation_id,
        }))
    }

    async fn set_role_capabilities(
        &self,
        request: Request<SetRoleCapabilitiesRequest>,
    ) -> Result<Response<SetRoleCapabilitiesResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let role = role_from_wire(req.role)?;
        // The Admin role is grant-all and can never be narrowed — narrowing it would
        // risk an unadministrable edge, so the bundle is immutable.
        if role.is_admin() {
            return Err(Status::failed_precondition(
                "the Admin role is grant-all and cannot be narrowed",
            ));
        }
        // Parse + validate the whole bundle up front: an unknown action/asset label is
        // rejected (`invalid_argument`), never silently dropped — the bundle that lands
        // is exactly the one the admin sent or no change at all.
        let caps = caps_from_wire(&req.capabilities)?;

        let mut guard = self.lock();
        let mut next = guard.clone();
        next.role_bundles.insert(
            role,
            caps.iter().copied().map(PermissionGrant::of).collect(),
        );
        // Identify every holder of this role BEFORE committing, so the change can be
        // forced to take effect by revoking their live sessions (next login re-derives
        // the new base).
        let holders: Vec<String> = guard
            .users
            .iter()
            .filter(|u| u.role == role)
            .map(|u| u.id.clone())
            .collect();
        self.persist_and_commit(&mut guard, next)?;
        drop(guard);

        for id in holders {
            self.sessions.revoke_user(&id);
        }
        Ok(Response::new(SetRoleCapabilitiesResponse {
            capabilities: caps.iter().copied().map(cap_to_wire).collect(),
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
            // A freshly-created desk owns no books yet; book→desk membership is
            // declared separately (config/`configure_desk` at boot).
            books: Vec::new(),
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

    async fn list_entities(
        &self,
        request: Request<ListEntitiesRequest>,
    ) -> Result<Response<ListEntitiesResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        // Read-only roster: any authenticated caller may read it (the booking form
        // populates its entity dropdown from this), so it is NOT admin-gated.
        self.authenticate(&req.session_token)?;
        let entities = self.lock().entities.iter().map(entity_to_wire).collect();
        Ok(Response::new(ListEntitiesResponse {
            entities,
            correlation_id: req.correlation_id,
        }))
    }

    async fn create_entity(
        &self,
        request: Request<CreateEntityRequest>,
    ) -> Result<Response<CreateEntityResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let name = req.name.trim().to_string();
        let code = req.code.trim().to_string();
        if name.is_empty() {
            return Err(Status::invalid_argument("entity name is required"));
        }
        if code.is_empty() {
            return Err(Status::invalid_argument("entity code is required"));
        }

        let mut guard = self.lock();
        if guard
            .entities
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case(&name))
        {
            return Err(Status::already_exists(format!(
                "an entity named `{name}` already exists"
            )));
        }
        if guard
            .entities
            .iter()
            .any(|e| e.code.eq_ignore_ascii_case(&code))
        {
            return Err(Status::already_exists(format!(
                "an entity with code `{code}` already exists"
            )));
        }
        let key = if req.key == 0 {
            guard.next_entity_key()
        } else {
            if guard.entity_by_key(req.key).is_some() {
                return Err(Status::already_exists(format!(
                    "entity key {} is already in use",
                    req.key
                )));
            }
            req.key
        };
        let entity = EntityDef { key, name, code };
        let mut next = guard.clone();
        next.entities.push(entity.clone());
        self.persist_and_commit(&mut guard, next)?;
        Ok(Response::new(CreateEntityResponse {
            entity: Some(entity_to_wire(&entity)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn update_entity(
        &self,
        request: Request<UpdateEntityRequest>,
    ) -> Result<Response<UpdateEntityResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let name = req.name.trim().to_string();
        let code = req.code.trim().to_string();
        if name.is_empty() {
            return Err(Status::invalid_argument("entity name is required"));
        }
        if code.is_empty() {
            return Err(Status::invalid_argument("entity code is required"));
        }

        let mut guard = self.lock();
        if guard.entity_by_key(req.key).is_none() {
            return Err(Status::not_found(format!("no entity with key {}", req.key)));
        }
        // Uniqueness is checked against every OTHER entity (the edited one may keep
        // its own name/code).
        if guard
            .entities
            .iter()
            .any(|e| e.key != req.key && e.name.eq_ignore_ascii_case(&name))
        {
            return Err(Status::already_exists(format!(
                "an entity named `{name}` already exists"
            )));
        }
        if guard
            .entities
            .iter()
            .any(|e| e.key != req.key && e.code.eq_ignore_ascii_case(&code))
        {
            return Err(Status::already_exists(format!(
                "an entity with code `{code}` already exists"
            )));
        }
        let entity = EntityDef {
            key: req.key,
            name,
            code,
        };
        let mut next = guard.clone();
        if let Some(slot) = next.entities.iter_mut().find(|e| e.key == req.key) {
            *slot = entity.clone();
        }
        self.persist_and_commit(&mut guard, next)?;
        Ok(Response::new(UpdateEntityResponse {
            entity: Some(entity_to_wire(&entity)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn delete_entity(
        &self,
        request: Request<DeleteEntityRequest>,
    ) -> Result<Response<DeleteEntityResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let mut guard = self.lock();
        if guard.entity_by_key(req.key).is_none() {
            return Ok(Response::new(DeleteEntityResponse {
                removed: false,
                correlation_id: req.correlation_id,
            }));
        }
        // Referential integrity: a book maps to its entity by `entity_key`; deleting
        // an entity that still has books would orphan them, so it is rejected.
        if guard.books.iter().any(|b| b.entity_key == req.key) {
            return Err(Status::failed_precondition(
                "cannot delete an entity while books still reference it",
            ));
        }
        let mut next = guard.clone();
        next.entities.retain(|e| e.key != req.key);
        self.persist_and_commit(&mut guard, next)?;
        Ok(Response::new(DeleteEntityResponse {
            removed: true,
            correlation_id: req.correlation_id,
        }))
    }

    async fn list_books(
        &self,
        request: Request<ListBooksRequest>,
    ) -> Result<Response<ListBooksResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        // Read-only roster: any authenticated caller may read it (the booking form
        // populates its book dropdown from this), so it is NOT admin-gated.
        self.authenticate(&req.session_token)?;
        let books = self.lock().books.iter().map(book_to_wire).collect();
        Ok(Response::new(ListBooksResponse {
            books,
            correlation_id: req.correlation_id,
        }))
    }

    async fn create_book(
        &self,
        request: Request<CreateBookRequest>,
    ) -> Result<Response<CreateBookResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let name = req.name.trim().to_string();
        if name.is_empty() {
            return Err(Status::invalid_argument("book name is required"));
        }

        let mut guard = self.lock();
        if guard.entity_by_key(req.entity_key).is_none() {
            return Err(Status::failed_precondition(format!(
                "no entity with key {}",
                req.entity_key
            )));
        }
        if guard
            .books
            .iter()
            .any(|b| b.name.eq_ignore_ascii_case(&name))
        {
            return Err(Status::already_exists(format!(
                "a book named `{name}` already exists"
            )));
        }
        let key = if req.key == 0 {
            guard.next_book_key()
        } else {
            if guard.book_by_key(req.key).is_some() {
                return Err(Status::already_exists(format!(
                    "book key {} is already in use",
                    req.key
                )));
            }
            req.key
        };
        let book = BookDef {
            key,
            name,
            entity_key: req.entity_key,
        };
        let mut next = guard.clone();
        next.books.push(book.clone());
        self.persist_and_commit(&mut guard, next)?;
        Ok(Response::new(CreateBookResponse {
            book: Some(book_to_wire(&book)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn update_book(
        &self,
        request: Request<UpdateBookRequest>,
    ) -> Result<Response<UpdateBookResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let name = req.name.trim().to_string();
        if name.is_empty() {
            return Err(Status::invalid_argument("book name is required"));
        }

        let mut guard = self.lock();
        if guard.book_by_key(req.key).is_none() {
            return Err(Status::not_found(format!("no book with key {}", req.key)));
        }
        if guard.entity_by_key(req.entity_key).is_none() {
            return Err(Status::failed_precondition(format!(
                "no entity with key {}",
                req.entity_key
            )));
        }
        if guard
            .books
            .iter()
            .any(|b| b.key != req.key && b.name.eq_ignore_ascii_case(&name))
        {
            return Err(Status::already_exists(format!(
                "a book named `{name}` already exists"
            )));
        }
        let book = BookDef {
            key: req.key,
            name,
            entity_key: req.entity_key,
        };
        let mut next = guard.clone();
        if let Some(slot) = next.books.iter_mut().find(|b| b.key == req.key) {
            *slot = book.clone();
        }
        self.persist_and_commit(&mut guard, next)?;
        Ok(Response::new(UpdateBookResponse {
            book: Some(book_to_wire(&book)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn delete_book(
        &self,
        request: Request<DeleteBookRequest>,
    ) -> Result<Response<DeleteBookResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let mut guard = self.lock();
        if guard.book_by_key(req.key).is_none() {
            return Ok(Response::new(DeleteBookResponse {
                removed: false,
                correlation_id: req.correlation_id,
            }));
        }
        let mut next = guard.clone();
        next.books.retain(|b| b.key != req.key);
        self.persist_and_commit(&mut guard, next)?;
        Ok(Response::new(DeleteBookResponse {
            removed: true,
            correlation_id: req.correlation_id,
        }))
    }

    // --- instrument reference data ---------------------------------------------

    async fn list_instruments(
        &self,
        request: Request<ListInstrumentsRequest>,
    ) -> Result<Response<ListInstrumentsResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        // Read-only roster: any authenticated caller may read it (curve-building and
        // pricing resolve against it), so it is NOT admin-gated.
        self.authenticate(&req.session_token)?;
        let instruments = self
            .lock()
            .instruments
            .iter()
            .map(instrument_to_wire)
            .collect();
        Ok(Response::new(ListInstrumentsResponse {
            instruments,
            correlation_id: req.correlation_id,
        }))
    }

    async fn get_instrument(
        &self,
        request: Request<GetInstrumentRequest>,
    ) -> Result<Response<GetInstrumentResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.authenticate(&req.session_token)?;
        let instrument = self
            .lock()
            .instrument_by_id(req.instrument_id.trim())
            .map(instrument_to_wire);
        Ok(Response::new(GetInstrumentResponse {
            instrument,
            correlation_id: req.correlation_id,
        }))
    }

    async fn build_curve(
        &self,
        request: Request<BuildCurveRequest>,
    ) -> Result<Response<CalibratedCurve>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        // Read-only against the reference-data registry, so — like `list_instruments`
        // / `get_instrument` — any authenticated caller may build, but unauthenticated
        // callers are rejected.
        self.authenticate(&req.session_token)?;

        if req.pillars.is_empty() {
            return Err(Status::invalid_argument(
                "build_curve requires at least one pillar",
            ));
        }
        // The curve reference (spot-anchor / value) date the pillar schedules roll from.
        let reference = req
            .reference_date
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing `reference_date`"))?;
        let value_date = decode_broken_date(reference).ok_or_else(|| {
            Status::invalid_argument("`reference_date` is not a real calendar date")
        })?;

        // Resolve every pillar id against the reference-data registry under a single
        // store read, then release the lock before the (root-finding) bootstrap so the
        // numeric solve never serializes other identity operations.
        let resolved: Vec<(String, InstrumentDef, f64)> = {
            let guard = self.lock();
            let mut rows = Vec::with_capacity(req.pillars.len());
            for pillar in &req.pillars {
                let id = pillar.instrument_id.trim();
                let def = guard
                    .instrument_by_id(id)
                    .ok_or_else(|| Status::not_found(format!("unknown instrument id `{id}`")))?;
                rows.push((id.to_string(), def.clone(), pillar.quote));
            }
            rows
        };

        // Map each resolved definition + quote onto an engine calibration instrument.
        // `calibration_set` preserves input order, so `instruments[i]` pairs with
        // `resolved[i]`. Every `CurveCalibrationError` is a static input-shape failure
        // (uncalibratable family, unknown convention label, currency mismatch, …).
        let defs: Vec<(&InstrumentDef, f64)> =
            resolved.iter().map(|(_, def, q)| (def, *q)).collect();
        let instruments =
            calibration_set(&defs, value_date).map_err(curve_calibration_status)?;

        // Bootstrap the self-discounting curve. A failure here is a numeric fault on
        // otherwise-valid input (mirrors the `price_rates` edge mapping it to internal).
        let curve = bootstrap_curve(&instruments)
            .map_err(|e| Status::internal(format!("curve bootstrap failed: {e}")))?;

        // Sample each input instrument at its own pillar maturity, ordered short → long
        // by maturity (the documented, deterministic point order).
        let mut points: Vec<CalibratedCurvePoint> = resolved
            .iter()
            .zip(&instruments)
            .map(|((id, _, _), inst)| {
                let t = inst.maturity();
                CalibratedCurvePoint {
                    instrument_id: id.clone(),
                    time_years: t.0,
                    discount_factor: curve.discount_factor(t).0,
                    zero_rate: curve.zero_rate(t).0,
                }
            })
            .collect();
        points.sort_by(|a, b| a.time_years.total_cmp(&b.time_years));

        Ok(Response::new(CalibratedCurve {
            request_id: req.request_id,
            currency: req.currency,
            reference_date: req.reference_date,
            points,
        }))
    }

    async fn create_instrument(
        &self,
        request: Request<CreateInstrumentRequest>,
    ) -> Result<Response<CreateInstrumentResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let wire = req
            .instrument
            .ok_or_else(|| Status::invalid_argument("instrument is required"))?;
        let mut def = instrument_from_wire(&wire)?;

        let mut guard = self.lock();
        // Mint the id from the name when blank; a provided id must be free.
        if def.instrument_id.is_empty() {
            def.instrument_id = mint_instrument_id(&def.name, &guard.instruments);
        } else if guard.instrument_by_id(&def.instrument_id).is_some() {
            return Err(Status::already_exists(format!(
                "an instrument with id `{}` already exists",
                def.instrument_id
            )));
        }

        let mut next = guard.clone();
        next.instruments.push(def.clone());
        // Validate the whole candidate registry (id/external-id uniqueness + labels +
        // required fields) before committing, so a bad write never persists.
        validate_instruments(&next.instruments).map_err(Status::invalid_argument)?;
        self.persist_and_commit(&mut guard, next)?;
        Ok(Response::new(CreateInstrumentResponse {
            instrument: Some(instrument_to_wire(&def)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn update_instrument(
        &self,
        request: Request<UpdateInstrumentRequest>,
    ) -> Result<Response<UpdateInstrumentResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let wire = req
            .instrument
            .ok_or_else(|| Status::invalid_argument("instrument is required"))?;
        let def = instrument_from_wire(&wire)?;
        if def.instrument_id.is_empty() {
            return Err(Status::invalid_argument(
                "instrument_id is required to update an instrument",
            ));
        }

        let mut guard = self.lock();
        if guard.instrument_by_id(&def.instrument_id).is_none() {
            return Err(Status::not_found(format!(
                "no instrument with id `{}`",
                def.instrument_id
            )));
        }
        let mut next = guard.clone();
        if let Some(slot) = next
            .instruments
            .iter_mut()
            .find(|i| i.instrument_id == def.instrument_id)
        {
            *slot = def.clone();
        }
        validate_instruments(&next.instruments).map_err(Status::invalid_argument)?;
        self.persist_and_commit(&mut guard, next)?;
        Ok(Response::new(UpdateInstrumentResponse {
            instrument: Some(instrument_to_wire(&def)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn delete_instrument(
        &self,
        request: Request<DeleteInstrumentRequest>,
    ) -> Result<Response<DeleteInstrumentResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let id = req.instrument_id.trim();
        let mut guard = self.lock();
        if guard.instrument_by_id(id).is_none() {
            return Ok(Response::new(DeleteInstrumentResponse {
                removed: false,
                correlation_id: req.correlation_id,
            }));
        }
        let mut next = guard.clone();
        next.instruments.retain(|i| i.instrument_id != id);
        self.persist_and_commit(&mut guard, next)?;
        Ok(Response::new(DeleteInstrumentResponse {
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

/// Map a domain [`Capability`] onto its wire [`CapabilityDesc`] (canonical labels).
fn cap_to_wire(cap: Capability) -> CapabilityDesc {
    CapabilityDesc {
        action: cap.action.label().to_string(),
        asset: cap.asset.label().to_string(),
    }
}

/// Map a stored overlay (`Vec<PermissionGrant>`) onto its wire form. The stored
/// labels are validated at load, so this is total and lossless.
fn overlay_to_wire(overlay: &[PermissionGrant]) -> Vec<CapabilityDesc> {
    overlay
        .iter()
        .map(|g| CapabilityDesc {
            action: g.action.clone(),
            asset: g.asset.clone(),
        })
        .collect()
}

/// Resolve a wire [`CapabilityDesc`] to a domain [`Capability`], rejecting an
/// unrecognised action/asset label with `invalid_argument`.
fn cap_from_wire(d: &CapabilityDesc) -> Result<Capability, Status> {
    let action = Action::from_label(&d.action)
        .ok_or_else(|| Status::invalid_argument(format!("unknown action label `{}`", d.action)))?;
    let asset = AssetClass::from_label(&d.asset)
        .ok_or_else(|| Status::invalid_argument(format!("unknown asset label `{}`", d.asset)))?;
    Ok(Capability::new(action, asset))
}

/// Resolve a wire overlay, failing on the first unknown label.
fn caps_from_wire(descs: &[CapabilityDesc]) -> Result<Vec<Capability>, Status> {
    descs.iter().map(cap_from_wire).collect()
}

/// The fully-resolved effective set for a user: role bundle ∪ grants ∖ denies,
/// enumerated over every action × asset. Built through the *same* resolution the
/// access boundary uses ([`AuthenticatedUser::capabilities`]), so the read-back can
/// never diverge from the live decision.
fn effective_caps(user: &UserDef, role_base: &[Capability]) -> Vec<CapabilityDesc> {
    let set = AuthenticatedUser::from_user_with_role_base(user, role_base.to_vec()).capabilities();
    let mut out = Vec::new();
    for action in Action::ALL {
        for asset in AssetClass::ALL {
            let cap = Capability::new(action, asset);
            if set.allows(cap) {
                out.push(cap_to_wire(cap));
            }
        }
    }
    out
}

/// The full action × asset surface as wire capabilities — the grant-all set the
/// immutable [`Role::Admin`] role confers. Used to report the Admin role bundle
/// (which is never stored and never narrowable).
fn grant_all_wire() -> Vec<CapabilityDesc> {
    let mut out = Vec::new();
    for action in Action::ALL {
        for asset in AssetClass::ALL {
            out.push(cap_to_wire(Capability::new(action, asset)));
        }
    }
    out
}

/// Map a stored [`DeskDef`] onto its wire [`DeskDesc`].
fn desk_to_wire(d: &DeskDef) -> DeskDesc {
    DeskDesc {
        id: d.id.clone(),
        name: d.name.clone(),
    }
}

/// Map a stored [`EntityDef`] onto its wire [`EntityDesc`].
fn entity_to_wire(e: &EntityDef) -> EntityDesc {
    EntityDesc {
        key: e.key,
        name: e.name.clone(),
        code: e.code.clone(),
    }
}

/// Map a stored [`BookDef`] onto its wire [`BookDesc`].
fn book_to_wire(b: &BookDef) -> BookDesc {
    BookDesc {
        key: b.key,
        name: b.name.clone(),
        entity_key: b.entity_key,
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
/// Decode a wire [`BrokenDate`] to a real [`time::Date`], or `None` if the triple
/// is not a real civil date (the caller maps `None` to `invalid_argument`).
fn decode_broken_date(d: &BrokenDate) -> Option<time::Date> {
    let month = u8::try_from(d.month)
        .ok()
        .and_then(|m| time::Month::try_from(m).ok())?;
    let day = u8::try_from(d.day).ok()?;
    time::Date::from_calendar_date(d.year, month, day).ok()
}

/// Map a [`CurveCalibrationError`] to a `tonic::Status`. Every variant is a static
/// (input-shape) failure — an uncalibratable family, an unresolvable tenor/label, a
/// currency mismatch, or a rejected schedule coordinate — so all map to
/// `invalid_argument`. Numeric (bootstrap) faults are handled separately as
/// `internal`.
fn curve_calibration_status(e: CurveCalibrationError) -> Status {
    Status::invalid_argument(e.to_string())
}

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

    // --- build_curve (Curves Part B inc.2): reference-data → bootstrapped curve ------

    use crate::config::curve_calibration::calibration_instrument;
    use crate::config::reference_data::{
        BondDef, CivilDate, DepositDef, FraDef, InstrumentFamily, OisDef, StirFutureDef,
        VanillaIrsDef,
    };
    use celnet_proto::InstrumentQuote;
    use celnet_rates::{
        CalibrationInstrument, Curve, deposit_par_rate, fixed_annuity, float_leg_value,
        fra_par_rate, implied_forward_rate, ois_par_rate,
    };
    use celnet_types::{Df, Time};

    /// The US business day used as the curve trade/value date (Monday 16 Jun 2025).
    fn curve_value_date() -> time::Date {
        time::Date::from_calendar_date(2025, time::Month::June, 16).expect("valid date")
    }

    /// The same value date as a wire `BrokenDate`.
    fn curve_reference() -> BrokenDate {
        BrokenDate {
            year: 2025,
            month: 6,
            day: 16,
        }
    }

    /// A USD instrument-definition header wrapping a family block.
    fn cv_usd(id: &str, family: InstrumentFamily) -> InstrumentDef {
        InstrumentDef {
            instrument_id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            currency: "USD".to_string(),
            external_ids: Vec::new(),
            definition: family,
        }
    }

    fn cv_dep(id: &str, tenor: &str) -> InstrumentDef {
        cv_usd(
            id,
            InstrumentFamily::Deposit(DepositDef {
                index: "USD-SOFR".to_string(),
                tenor: tenor.to_string(),
                day_count: "act_360".to_string(),
                business_day_convention: "modified_following".to_string(),
                calendars: vec!["united_states".to_string()],
                spot_lag_days: 2,
            }),
        )
    }

    fn cv_fra(id: &str, start: &str, end: &str) -> InstrumentDef {
        cv_usd(
            id,
            InstrumentFamily::Fra(FraDef {
                float_index: "USD-SOFR".to_string(),
                start_tenor: start.to_string(),
                end_tenor: end.to_string(),
                accrual_day_count: "act_360".to_string(),
                business_day_convention: "modified_following".to_string(),
                calendars: vec!["united_states".to_string()],
                spot_lag_days: 2,
            }),
        )
    }

    fn cv_stir(id: &str, ref_start: &str, ref_end: &str) -> InstrumentDef {
        cv_usd(
            id,
            InstrumentFamily::StirFuture(StirFutureDef {
                contract_code: "SR3".to_string(),
                reference_start: ref_start.to_string(),
                reference_end: ref_end.to_string(),
                day_count: "act_360".to_string(),
                calendars: vec!["united_states".to_string()],
                convexity_vol: 0.005,
                contract_size: 1_000_000.0,
            }),
        )
    }

    fn cv_ois(id: &str, tenor: &str) -> InstrumentDef {
        cv_usd(
            id,
            InstrumentFamily::Ois(OisDef {
                tenor: tenor.to_string(),
                index: "USD-SOFR".to_string(),
                fixed_frequency: "annual".to_string(),
                fixed_day_count: "act_360".to_string(),
                float_day_count: "act_360".to_string(),
                business_day_convention: "modified_following".to_string(),
                calendars: vec!["united_states".to_string()],
                spot_lag_days: 2,
            }),
        )
    }

    fn cv_irs(id: &str, tenor: &str) -> InstrumentDef {
        cv_usd(
            id,
            InstrumentFamily::VanillaIrs(VanillaIrsDef {
                tenor: tenor.to_string(),
                fixed_frequency: "annual".to_string(),
                fixed_day_count: "thirty_360_bond_basis".to_string(),
                float_index: "USD-SOFR".to_string(),
                float_frequency: "quarterly".to_string(),
                float_day_count: "act_360".to_string(),
                business_day_convention: "modified_following".to_string(),
                calendars: vec!["united_states".to_string()],
                roll_convention: "none".to_string(),
                spot_lag_days: 2,
            }),
        )
    }

    /// Build an edge over a temp identity file seeded with the default admin plus the
    /// supplied instrument reference-data definitions.
    fn curve_edge(tag: &str, instruments: Vec<InstrumentDef>) -> (AuthEdge, PathBuf) {
        let dir = std::env::temp_dir().join("celnet-auth-edge");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("identity-curve-{}-{}.json", std::process::id(), tag));
        let _ = std::fs::remove_file(&path);
        let mut store = IdentityStore::default();
        store.ensure_seed_admin().unwrap();
        store.instruments = instruments;
        store.save(&path).unwrap();
        let identity = Arc::new(Mutex::new(store));
        let clock = Clock::system();
        let sessions = Arc::new(SessionRegistry::new(clock.clone()));
        let gate = Arc::new(ReadinessGate::new());
        gate.mark_ready();
        (
            AuthEdge::new(identity, path.clone(), sessions, gate, clock),
            path,
        )
    }

    /// Reprice a calibration instrument on `curve`, returning its model par rate
    /// (or futures rate) — the quantity the bootstrap drove to the input quote.
    fn model_quote(curve: &Curve, inst: &CalibrationInstrument) -> f64 {
        match inst {
            CalibrationInstrument::Deposit(d) => deposit_par_rate(curve, d).0,
            CalibrationInstrument::Fra(f) => fra_par_rate(curve, f).0,
            CalibrationInstrument::StirFuture(q) => {
                // The futures quote is the curve's simple forward over the fixing window
                // plus the (quote-implied) convexity bias the bootstrap debiased out.
                let t1 = q.future.fixing_start();
                let t2 = q.future.fixing_end();
                let fwd = (curve.discount_factor(t1).0 / curve.discount_factor(t2).0 - 1.0)
                    / (t2.0 - t1.0);
                let convexity = q.futures_rate.0 - implied_forward_rate(q).0;
                fwd + convexity
            }
            CalibrationInstrument::Ois(q) => ois_par_rate(curve, &q.schedule).0,
            CalibrationInstrument::VanillaIrs(q) => {
                float_leg_value(curve, &q.float_leg) / fixed_annuity(curve, &q.fixed_leg)
            }
        }
    }

    /// End-to-end oracle gate: a USD curve built from a registry-referenced ladder
    /// (deposits + FRA + STIR future + OIS + vanilla IRS) reprices every calibrating
    /// instrument back to its quote. The curve is reconstructed *purely* from the wire
    /// `(time_years, discount_factor)` points and each instrument repriced via the
    /// engine's par-rate identities — never by re-running `build_curve` as its own check.
    #[tokio::test]
    async fn build_curve_reprices_every_pillar_to_par() {
        // Maturities are strictly increasing: 1M, 3M, 6M(FRA), 9M(STIR), 1Y, 2Y, 5Y, 10Y.
        let set: Vec<(InstrumentDef, f64)> = vec![
            (cv_dep("usd-depo-1m", "1M"), 0.0433),
            (cv_dep("usd-depo-3m", "3M"), 0.0431),
            (cv_fra("usd-fra-3x6", "3M", "6M"), 0.0429),
            (cv_stir("usd-stir-6x9", "6M", "9M"), 0.0427),
            (cv_ois("usd-ois-1y", "1Y"), 0.0425),
            (cv_ois("usd-ois-2y", "2Y"), 0.0418),
            (cv_irs("usd-irs-5y", "5Y"), 0.0410),
            (cv_irs("usd-irs-10y", "10Y"), 0.0415),
        ];

        let instruments: Vec<InstrumentDef> = set.iter().map(|(d, _)| d.clone()).collect();
        let (edge, path) = curve_edge("reprice", instruments);
        let token = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;

        // Submit the pillars out of maturity order to prove the server orders them.
        let pillars: Vec<InstrumentQuote> = set
            .iter()
            .rev()
            .map(|(d, q)| InstrumentQuote {
                instrument_id: d.instrument_id.clone(),
                quote: *q,
            })
            .collect();

        let resp = edge
            .build_curve(Request::new(BuildCurveRequest {
                request_id: "curve-req-1".to_string(),
                currency: "USD".to_string(),
                reference_date: Some(curve_reference()),
                pillars,
                session_token: token,
            }))
            .await
            .expect("build_curve should succeed")
            .into_inner();

        // Header echoed; one point per input; points strictly increasing in maturity.
        assert_eq!(resp.request_id, "curve-req-1");
        assert_eq!(resp.currency, "USD");
        assert_eq!(resp.reference_date, Some(curve_reference()));
        assert_eq!(resp.points.len(), set.len());
        for w in resp.points.windows(2) {
            assert!(
                w[0].time_years < w[1].time_years,
                "points not strictly increasing by maturity"
            );
        }

        // Reconstruct the curve from the wire points, prepended with the definitional
        // origin (DF = 1 at t = 0 — a curve invariant, not engine output).
        let mut td: Vec<(Time, Df)> = vec![(Time(0.0), Df(1.0))];
        td.extend(
            resp.points
                .iter()
                .map(|p| (Time(p.time_years), Df(p.discount_factor))),
        );
        let curve = Curve::from_log_linear_dfs(&td).expect("curve reconstructs from wire points");

        // The wire points are self-consistent log-linear pillars (DF gate, 1e-10).
        for p in &resp.points {
            let df = curve.discount_factor(Time(p.time_years)).0;
            assert!(
                (df - p.discount_factor).abs() < 1e-10,
                "{}: DF {df} vs wire {} (resid {})",
                p.instrument_id,
                p.discount_factor,
                (df - p.discount_factor).abs()
            );
        }

        // Reprice-to-par gate (rate, 1e-8): every input instrument reprices to its quote.
        let value_date = curve_value_date();
        let mut worst = 0.0_f64;
        for (def, quote) in &set {
            let inst = calibration_instrument(def, *quote, value_date)
                .expect("definition resolves to a calibration instrument");
            let model = model_quote(&curve, &inst);
            let resid = (model - quote).abs();
            worst = worst.max(resid);
            assert!(
                resid < 1e-8,
                "{}: repriced {model} vs quote {quote} (resid {resid})",
                def.instrument_id
            );
        }
        assert!(worst < 1e-8, "worst reprice-to-par residual {worst}");

        let _ = std::fs::remove_file(&path);
    }

    /// An unknown pillar instrument id is rejected with `NOT_FOUND`.
    #[tokio::test]
    async fn build_curve_rejects_unknown_instrument_id() {
        let (edge, path) = curve_edge("notfound", vec![cv_ois("usd-ois-1y", "1Y")]);
        let token = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;
        let err = edge
            .build_curve(Request::new(BuildCurveRequest {
                request_id: "x".to_string(),
                currency: "USD".to_string(),
                reference_date: Some(curve_reference()),
                pillars: vec![InstrumentQuote {
                    instrument_id: "ghost".to_string(),
                    quote: 0.04,
                }],
                session_token: token,
            }))
            .await
            .expect_err("unknown id must be rejected");
        assert_eq!(err.code(), tonic::Code::NotFound);
        let _ = std::fs::remove_file(&path);
    }

    /// A non-calibratable family (a cash bond) is rejected with `INVALID_ARGUMENT`.
    #[tokio::test]
    async fn build_curve_rejects_non_calibratable_bond() {
        let bond = cv_usd(
            "usd-bond-5y",
            InstrumentFamily::Bond(BondDef {
                issuer: "US Treasury".to_string(),
                coupon_rate: 0.04,
                coupon_type: "fixed".to_string(),
                coupon_frequency: "semi_annual".to_string(),
                day_count: "act_365_fixed".to_string(),
                issue_date: None,
                dated_date: None,
                first_coupon_date: None,
                maturity_date: CivilDate {
                    year: 2030,
                    month: 6,
                    day: 16,
                },
                redemption: 100.0,
                calendars: vec!["united_states".to_string()],
            }),
        );
        let (edge, path) = curve_edge("bond", vec![bond]);
        let token = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;
        let err = edge
            .build_curve(Request::new(BuildCurveRequest {
                request_id: "b".to_string(),
                currency: "USD".to_string(),
                reference_date: Some(curve_reference()),
                pillars: vec![InstrumentQuote {
                    instrument_id: "usd-bond-5y".to_string(),
                    quote: 0.04,
                }],
                session_token: token,
            }))
            .await
            .expect_err("a bond is not a curve pillar");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        let _ = std::fs::remove_file(&path);
    }

    /// An empty pillar set is rejected with `INVALID_ARGUMENT`.
    #[tokio::test]
    async fn build_curve_rejects_empty_pillars() {
        let (edge, path) = curve_edge("empty", vec![cv_ois("usd-ois-1y", "1Y")]);
        let token = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;
        let err = edge
            .build_curve(Request::new(BuildCurveRequest {
                request_id: "e".to_string(),
                currency: "USD".to_string(),
                reference_date: Some(curve_reference()),
                pillars: Vec::new(),
                session_token: token,
            }))
            .await
            .expect_err("empty pillar set must be rejected");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        let _ = std::fs::remove_file(&path);
    }

    /// An absent/invalid session token is rejected with `UNAUTHENTICATED` before any
    /// registry read — mirroring the `list_instruments` / `get_instrument` read RPCs.
    #[tokio::test]
    async fn build_curve_rejects_unauthenticated() {
        let (edge, path) = curve_edge("unauth", vec![cv_ois("usd-ois-1y", "1Y")]);
        let err = edge
            .build_curve(Request::new(BuildCurveRequest {
                request_id: "u".to_string(),
                currency: "USD".to_string(),
                reference_date: Some(curve_reference()),
                pillars: vec![InstrumentQuote {
                    instrument_id: "usd-ois-1y".to_string(),
                    quote: 0.04,
                }],
                session_token: "not-a-token".to_string(),
            }))
            .await
            .expect_err("an invalid token must be rejected");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
        let _ = std::fs::remove_file(&path);
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

    /// Create a trader and return (its id, a live trader token).
    async fn make_trader(edge: &AuthEdge, admin_token: &str, email: &str) -> (String, String) {
        let created = edge
            .create_user(Request::new(CreateUserRequest {
                session_token: admin_token.to_string(),
                email: email.into(),
                display_name: "Trader".into(),
                role: UserRole::Trader as i32,
                desk_id: None,
                password: "trader-pw-123".into(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner();
        let id = created.user.unwrap().id;
        let token = login(edge, email, "trader-pw-123")
            .await
            .unwrap()
            .session_token;
        (id, token)
    }

    fn cap(action: &str, asset: &str) -> CapabilityDesc {
        CapabilityDesc {
            action: action.into(),
            asset: asset.into(),
        }
    }

    fn has_cap(caps: &[CapabilityDesc], action: &str, asset: &str) -> bool {
        caps.iter().any(|c| c.action == action && c.asset == asset)
    }

    #[tokio::test]
    async fn set_user_capabilities_widens_narrows_persists_and_revokes() {
        let (edge, path, sessions) = edge("set-caps");
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        let (trader_id, trader_token) =
            make_trader(&edge, &admin.session_token, "ovl@celnet.com").await;

        // Baseline: a trader holds Execute·FixedIncome (role bundle) and NOT
        // Administer·FxOptions.
        let base = edge
            .get_user_capabilities(Request::new(GetUserCapabilitiesRequest {
                session_token: admin.session_token.clone(),
                id: trader_id.clone(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(base.grants.is_empty() && base.denies.is_empty());
        assert!(has_cap(&base.effective, "execute", "fixed_income"));
        assert!(!has_cap(&base.effective, "administer", "fx_options"));

        // Admin overlays: grant Administer·FxOptions (widen), deny Execute·FixedIncome
        // (narrow). The trader has a live session — it must be revoked by the change.
        assert!(sessions.validate(&trader_token).is_some());
        let set = edge
            .set_user_capabilities(Request::new(SetUserCapabilitiesRequest {
                session_token: admin.session_token.clone(),
                id: trader_id.clone(),
                grants: vec![cap("administer", "fx_options")],
                denies: vec![cap("execute", "fixed_income")],
                correlation_id: Some(7),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(set.correlation_id, Some(7));
        // Read-back: effective now includes the grant and excludes the deny.
        assert!(has_cap(&set.effective, "administer", "fx_options"));
        assert!(!has_cap(&set.effective, "execute", "fixed_income"));
        assert!(has_cap(&set.grants, "administer", "fx_options"));
        assert!(has_cap(&set.denies, "execute", "fixed_income"));
        // The trader's stale session is gone (authority changed).
        assert!(sessions.validate(&trader_token).is_none());

        // Persisted: a fresh get returns the same overlay, and it survives reload
        // from disk (the overlay round-tripped through identity.json).
        let again = edge
            .get_user_capabilities(Request::new(GetUserCapabilitiesRequest {
                session_token: admin.session_token.clone(),
                id: trader_id.clone(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(has_cap(&again.grants, "administer", "fx_options"));
        assert!(has_cap(&again.denies, "execute", "fixed_income"));
        let reloaded = IdentityStore::load(&path).unwrap();
        let stored = reloaded.user(&trader_id).unwrap();
        assert_eq!(stored.capability_grants.len(), 1);
        assert_eq!(stored.capability_denies.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn login_returns_caller_effective_capabilities() {
        let (edge, path, _s) = edge("login-caps");
        // The seed admin resolves to grant-all: every action × asset (10 × 2 = 20).
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        assert_eq!(
            admin.capabilities.len(),
            Action::ALL.len() * AssetClass::ALL.len()
        );

        // A fresh trader holds the role bundle: all actions but `administer` (9 × 2).
        let (trader_id, _t) = make_trader(&edge, &admin.session_token, "lc@celnet.com").await;
        let trader = login(&edge, "lc@celnet.com", "trader-pw-123")
            .await
            .unwrap();
        assert_eq!(
            trader.capabilities.len(),
            (Action::ALL.len() - 1) * AssetClass::ALL.len()
        );
        assert!(has_cap(&trader.capabilities, "execute", "fixed_income"));
        assert!(!has_cap(&trader.capabilities, "administer", "fx_options"));

        // After an admin denies one capability, the trader's NEXT login (their prior
        // session was revoked by the change) re-derives the narrowed set.
        edge.set_user_capabilities(Request::new(SetUserCapabilitiesRequest {
            session_token: admin.session_token,
            id: trader_id,
            grants: vec![],
            denies: vec![cap("execute", "fixed_income")],
            correlation_id: None,
        }))
        .await
        .unwrap();
        let relogged = login(&edge, "lc@celnet.com", "trader-pw-123")
            .await
            .unwrap();
        assert!(!has_cap(&relogged.capabilities, "execute", "fixed_income"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn set_user_capabilities_rejects_unknown_label() {
        let (edge, path, _s) = edge("set-caps-bad");
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        let (trader_id, _t) = make_trader(&edge, &admin.session_token, "bad@celnet.com").await;
        let err = edge
            .set_user_capabilities(Request::new(SetUserCapabilitiesRequest {
                session_token: admin.session_token.clone(),
                id: trader_id,
                grants: vec![cap("teleport", "fx_options")],
                denies: vec![],
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn capability_rpcs_require_admin_and_known_user() {
        let (edge, path, _s) = edge("caps-authz");
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        let (_id, trader_token) =
            make_trader(&edge, &admin.session_token, "authz@celnet.com").await;

        // A trader's token cannot read or write any user's overlay.
        let get_denied = edge
            .get_user_capabilities(Request::new(GetUserCapabilitiesRequest {
                session_token: trader_token.clone(),
                id: "anyone".into(),
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(get_denied.code(), tonic::Code::PermissionDenied);
        let set_denied = edge
            .set_user_capabilities(Request::new(SetUserCapabilitiesRequest {
                session_token: trader_token,
                id: "anyone".into(),
                grants: vec![],
                denies: vec![],
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(set_denied.code(), tonic::Code::PermissionDenied);

        // An admin targeting a missing user gets not_found, not a silent no-op.
        let missing = edge
            .get_user_capabilities(Request::new(GetUserCapabilitiesRequest {
                session_token: admin.session_token,
                id: "ghost".into(),
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(missing.code(), tonic::Code::NotFound);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn set_role_capabilities_narrows_trader_base_persists_and_revokes() {
        let (edge, path, sessions) = edge("set-role-caps");
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        let (_trader_id, trader_token) =
            make_trader(&edge, &admin.session_token, "rb@celnet.com").await;

        // Baseline: the default trader role bundle includes book·fixed_income.
        let base = edge
            .get_role_capabilities(Request::new(GetRoleCapabilitiesRequest {
                session_token: admin.session_token.clone(),
                role: UserRole::Trader as i32,
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(has_cap(&base.capabilities, "book", "fixed_income"));
        assert!(has_cap(&base.capabilities, "execute", "fx_options"));

        // Narrow the bundle: drop book·fixed_income. The trader has a live session —
        // setting the role bundle must revoke it (next login re-derives the new base).
        assert!(sessions.validate(&trader_token).is_some());
        let narrowed: Vec<CapabilityDesc> = base
            .capabilities
            .iter()
            .filter(|c| !(c.action == "book" && c.asset == "fixed_income"))
            .cloned()
            .collect();
        let set = edge
            .set_role_capabilities(Request::new(SetRoleCapabilitiesRequest {
                session_token: admin.session_token.clone(),
                role: UserRole::Trader as i32,
                capabilities: narrowed,
                correlation_id: Some(9),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(set.correlation_id, Some(9));
        assert!(!has_cap(&set.capabilities, "book", "fixed_income"));
        assert!(sessions.validate(&trader_token).is_none());

        // A trader re-logging in now lacks book·fixed_income in their effective set.
        let relogged = login(&edge, "rb@celnet.com", "trader-pw-123")
            .await
            .unwrap();
        assert!(!has_cap(&relogged.capabilities, "book", "fixed_income"));
        assert!(has_cap(&relogged.capabilities, "execute", "fx_options"));

        // Persisted: the narrowed bundle survives a reload from disk.
        let reloaded = IdentityStore::load(&path).unwrap();
        let stored = reloaded.role_base(Role::Trader);
        assert!(!stored.contains(&Capability::new(Action::Book, AssetClass::FixedIncome)));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn admin_role_bundle_is_grant_all_and_set_is_rejected() {
        let (edge, path, _s) = edge("admin-role-caps");
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();

        // Get for Admin reports the full grant-all surface.
        let got = edge
            .get_role_capabilities(Request::new(GetRoleCapabilitiesRequest {
                session_token: admin.session_token.clone(),
                role: UserRole::Admin as i32,
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            got.capabilities.len(),
            Action::ALL.len() * AssetClass::ALL.len()
        );

        // Set for Admin is rejected — the role is immutable, never narrowable.
        let err = edge
            .set_role_capabilities(Request::new(SetRoleCapabilitiesRequest {
                session_token: admin.session_token.clone(),
                role: UserRole::Admin as i32,
                capabilities: vec![cap("view", "fx_options")],
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn role_capability_rpcs_require_admin_and_reject_unknown_label() {
        let (edge, path, _s) = edge("role-caps-authz");
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        let (_id, trader_token) =
            make_trader(&edge, &admin.session_token, "rauthz@celnet.com").await;

        // A trader's token cannot read or write a role bundle.
        let get_denied = edge
            .get_role_capabilities(Request::new(GetRoleCapabilitiesRequest {
                session_token: trader_token.clone(),
                role: UserRole::Trader as i32,
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(get_denied.code(), tonic::Code::PermissionDenied);
        let set_denied = edge
            .set_role_capabilities(Request::new(SetRoleCapabilitiesRequest {
                session_token: trader_token,
                role: UserRole::Trader as i32,
                capabilities: vec![],
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(set_denied.code(), tonic::Code::PermissionDenied);

        // An admin sending an unknown label is rejected (invalid_argument).
        let bad = edge
            .set_role_capabilities(Request::new(SetRoleCapabilitiesRequest {
                session_token: admin.session_token,
                role: UserRole::Trader as i32,
                capabilities: vec![cap("teleport", "fx_options")],
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(bad.code(), tonic::Code::InvalidArgument);
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

    #[tokio::test]
    async fn entity_book_registry_crud_and_trader_can_list() {
        let (edge, path, _s) = edge("registry");
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        let tok = admin.session_token.clone();
        let (_tid, trader_token) = make_trader(&edge, &tok, "reg@celnet.com").await;

        // Admin creates an entity (auto-assigned key) and a book under it.
        let ent = edge
            .create_entity(Request::new(CreateEntityRequest {
                session_token: tok.clone(),
                name: "ACME Capital".into(),
                code: "ACME".into(),
                key: 0,
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .entity
            .unwrap();
        assert!(ent.key >= 1);
        let book = edge
            .create_book(Request::new(CreateBookRequest {
                session_token: tok.clone(),
                name: "Rates Trading".into(),
                entity_key: ent.key,
                key: 0,
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .book
            .unwrap();
        assert_eq!(book.entity_key, ent.key);

        // A non-admin trader CAN list entities and books (the booking form needs them).
        let ents = edge
            .list_entities(Request::new(ListEntitiesRequest {
                session_token: trader_token.clone(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .entities;
        assert!(ents.iter().any(|e| e.name == "ACME Capital"));
        let books = edge
            .list_books(Request::new(ListBooksRequest {
                session_token: trader_token.clone(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .books;
        assert!(books.iter().any(|b| b.name == "Rates Trading"));

        // A trader CANNOT create an entity (admin only).
        let denied = edge
            .create_entity(Request::new(CreateEntityRequest {
                session_token: trader_token.clone(),
                name: "Nope".into(),
                code: "NO".into(),
                key: 0,
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(denied.code(), tonic::Code::PermissionDenied);

        // Deleting the entity while a book references it is rejected.
        let pre = edge
            .delete_entity(Request::new(DeleteEntityRequest {
                session_token: tok.clone(),
                key: ent.key,
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(pre.code(), tonic::Code::FailedPrecondition);

        // Delete the book, then the entity succeeds; survives reload from disk.
        edge.delete_book(Request::new(DeleteBookRequest {
            session_token: tok.clone(),
            key: book.key,
            correlation_id: None,
        }))
        .await
        .unwrap();
        let removed = edge
            .delete_entity(Request::new(DeleteEntityRequest {
                session_token: tok.clone(),
                key: ent.key,
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .removed;
        assert!(removed);
        let reloaded = IdentityStore::load(&path).unwrap();
        assert!(reloaded.entity_by_key(ent.key).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn instrument_registry_crud_capability_model_and_persistence() {
        use celnet_proto::{InstrumentDefDesc, OisDef, instrument_def_desc::Definition};

        let (edge, path, _s) = edge("instruments");
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        let tok = admin.session_token.clone();
        let (_tid, trader_token) = make_trader(&edge, &tok, "inst@celnet.com").await;

        let ois_desc = |id: &str, ticker: &str| InstrumentDefDesc {
            instrument_id: id.to_string(),
            name: format!("Test OIS {id}"),
            description: "test".into(),
            currency: "USD".into(),
            external_ids: vec![celnet_proto::ExternalId {
                scheme: "ticker".into(),
                value: ticker.into(),
            }],
            definition: Some(Definition::Ois(OisDef {
                tenor: "3Y".into(),
                index: "sofr".into(),
                fixed_frequency: "annual".into(),
                fixed_day_count: "act_360".into(),
                float_day_count: "act_360".into(),
                business_day_convention: "modified_following".into(),
                calendars: vec!["united_states".into()],
                spot_lag_days: 2,
            })),
        };

        // Admin creates an instrument (explicit id).
        let created = edge
            .create_instrument(Request::new(CreateInstrumentRequest {
                session_token: tok.clone(),
                instrument: Some(ois_desc("test-ois-3y", "TEST-OIS-3Y")),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .instrument
            .unwrap();
        assert_eq!(created.instrument_id, "test-ois-3y");

        // A trader CAN list (curve-building needs it) and includes the seeded set.
        let listed = edge
            .list_instruments(Request::new(ListInstrumentsRequest {
                session_token: trader_token.clone(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .instruments;
        assert!(listed.iter().any(|i| i.instrument_id == "test-ois-3y"));

        // A trader CANNOT create (admin only).
        let denied = edge
            .create_instrument(Request::new(CreateInstrumentRequest {
                session_token: trader_token.clone(),
                instrument: Some(ois_desc("nope", "NOPE")),
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(denied.code(), tonic::Code::PermissionDenied);

        // A duplicate external id is rejected (registry-wide uniqueness).
        let dup = edge
            .create_instrument(Request::new(CreateInstrumentRequest {
                session_token: tok.clone(),
                instrument: Some(ois_desc("test-ois-other", "TEST-OIS-3Y")),
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(dup.code(), tonic::Code::InvalidArgument);

        // Get resolves by id (trader-readable).
        let got = edge
            .get_instrument(Request::new(GetInstrumentRequest {
                session_token: trader_token.clone(),
                instrument_id: "test-ois-3y".into(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .instrument
            .unwrap();
        assert_eq!(got.name, "Test OIS test-ois-3y");

        // Delete (admin), and confirm it survives a reload from disk.
        let removed = edge
            .delete_instrument(Request::new(DeleteInstrumentRequest {
                session_token: tok.clone(),
                instrument_id: "test-ois-3y".into(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .removed;
        assert!(removed);
        let reloaded = IdentityStore::load(&path).unwrap();
        assert!(reloaded.instrument_by_id("test-ois-3y").is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn create_entity_rejects_duplicate_name_and_unauthenticated_list() {
        let (edge, path, _s) = edge("registry-dup");
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        let tok = admin.session_token.clone();
        edge.create_entity(Request::new(CreateEntityRequest {
            session_token: tok.clone(),
            name: "ACME Capital".into(),
            code: "ACME".into(),
            key: 0,
            correlation_id: None,
        }))
        .await
        .unwrap();
        // Duplicate name (case-insensitive) is rejected.
        let dup = edge
            .create_entity(Request::new(CreateEntityRequest {
                session_token: tok.clone(),
                name: "acme capital".into(),
                code: "ACME2".into(),
                key: 0,
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(dup.code(), tonic::Code::AlreadyExists);
        // An unauthenticated list is rejected.
        let anon = edge
            .list_entities(Request::new(ListEntitiesRequest {
                session_token: "not-a-token".into(),
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(anon.code(), tonic::Code::Unauthenticated);
        let _ = std::fs::remove_file(&path);
    }
}
