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
    AcceptRiskTransferRequest, AcceptRiskTransferResponse, CancelRiskTransferRequest,
    CancelRiskTransferResponse, InitiateRiskTransferRequest, InitiateRiskTransferResponse,
    ListClientFlowMetricsRequest, ListClientFlowMetricsResponse, ListRiskTransfersRequest,
    ListRiskTransfersResponse, RejectRiskTransferRequest,
    RejectRiskTransferResponse,
};
use celnet_proto::{
    AggregatedBookDesc, AggregatedBookSpec, AggregationParamsDesc, AggregationScopeMode, AxeSide,
    BookDesc, BrokenDate, BuildCurveRequest, CalibratedCurve, CalibratedCurvePoint, CapabilityDesc,
    CreateAggregatedBookRequest, CreateAggregatedBookResponse, CreateBookRequest,
    CreateBookResponse, CreateDeskRequest, CreateDeskResponse, CreateEntityRequest,
    CreateEntityResponse, CreateInstrumentRequest, CreateInstrumentResponse,
    CreatePricingGroupRequest, CreatePricingGroupResponse, CreateUserRequest, CreateUserResponse,
    DeleteAggregatedBookRequest, DeleteAggregatedBookResponse, DeleteBookRequest,
    DeleteBookResponse, DeleteDeskRequest, DeleteDeskResponse, DeleteEntityRequest,
    DeleteEntityResponse, DeleteInstrumentRequest, DeleteInstrumentResponse,
    DeletePricingGroupRequest, DeletePricingGroupResponse, DeleteUserRequest, DeleteUserResponse,
    DeskDesc, EntityDesc, EspOrRfq, FeatureKind, FeaturePipelineDesc, FeatureSpecDesc,
    GetInstrumentRequest, GetInstrumentResponse, GetRoleCapabilitiesRequest,
    GetRoleCapabilitiesResponse, GetUserCapabilitiesRequest, GetUserCapabilitiesResponse,
    ListAggregatedBooksRequest, ListAggregatedBooksResponse, ListBooksRequest, ListBooksResponse,
    ListDesksRequest, ListDesksResponse, ListEntitiesRequest, ListEntitiesResponse,
    ListInstrumentsRequest, ListInstrumentsResponse, ListPricingGroupsRequest,
    ListPricingGroupsResponse, ListUsersRequest, ListUsersResponse, LoginRequest, LoginResponse,
    LogoutRequest, LogoutResponse, PricingGroupDesc, PricingGroupSpec, ResetPasswordRequest,
    ResetPasswordResponse, SetRoleCapabilitiesRequest, SetRoleCapabilitiesResponse,
    SetUserCapabilitiesRequest, SetUserCapabilitiesResponse, TieringConfigDesc,
    TieringGuardrailsDesc, TieringSpreadUnit, TieringStalePolicy, TieringStrategyDesc,
    TieringStrategyKind, UpdateAggregatedBookRequest, UpdateAggregatedBookResponse,
    UpdateBookRequest, UpdateBookResponse, UpdateDeskRequest, UpdateDeskResponse,
    UpdateEntityRequest, UpdateEntityResponse, UpdateInstrumentRequest, UpdateInstrumentResponse,
    UpdatePricingGroupPipelineRequest, UpdatePricingGroupPipelineResponse,
    UpdatePricingGroupRequest, UpdatePricingGroupResponse, UpdateUserRequest, UpdateUserResponse,
    UserDesc, UserRole,
};
use celnet_proto::{
    CreateRiskBookRequest, CreateRiskBookResponse, DeleteRiskBookRequest, DeleteRiskBookResponse,
    GetRiskRoutingGraphRequest, GetRiskRoutingGraphResponse, LimitUtilizationDesc,
    ListRiskBookRiskRequest, ListRiskBookRiskResponse, ListRiskBooksRequest, ListRiskBooksResponse,
    RagBand, RiskBookDesc, RiskBookRiskDesc, RiskBookSpec, RiskLimitsDesc, RiskRoutingGraphDesc,
    RouteConditionDesc, RouteFieldEnum, RouteOpEnum, RouteRange, RouteValueDesc, RoutingNodeDesc,
    StringList, UpdateRiskBookRequest, UpdateRiskBookResponse, UpdateRiskRoutingGraphRequest,
    UpdateRiskRoutingGraphResponse, route_value_desc, routing_node_desc,
};
use celnet_rates::{CalibrationInstrument, bootstrap_curve};
use tonic::{Request, Response, Status};

use crate::clock::Clock;
use crate::config::curve_calibration::{
    CurveCalibrationError, calibration_set, date_pillar_instrument,
};
use crate::config::identity::{
    AggregatedBookDef, AggregatedBookEdit, AggregationParams, BookDef, DeskDef, EntityDef,
    IdentityStore, PermissionGrant, PricingGroupDef, PricingGroupEdit, PricingMode, RiskBookDef,
    RiskBookEdit, RiskLimits, Role, Scope, UserDef, hash_password, mint_desk_id, mint_user_id,
    verify_password,
};
use crate::config::reference_data::{InstrumentDef, mint_instrument_id, validate_instruments};
use crate::readiness::ReadinessGate;
use crate::services::instrument_wire::{instrument_from_wire, instrument_to_wire};
use crate::services::risk::book_risk::{
    LimitUtilization, RagBand as DomainRagBand, RiskBookRisk, aggregate_risk_book,
};
use crate::services::risk::store::PositionStore;
use crate::services::sessions::{AuthenticatedUser, SessionRegistry};
use celnet_risk_routing::{NodeId, RiskRoutingGraph, RouteField, RouteOp, RouteValue, RoutingNode};

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
    /// The edge-wide aggregated-book engine hub, re-reconciled after every admin
    /// book create/update/delete so an engine stands up / tears down immediately
    /// (D3). `None` in an isolated auth test (book CRUD then persists only).
    aggregation_hub: Option<Arc<crate::services::aggregation::AggregationHub>>,
    /// The shared live position store whose risk router this edge re-primes after every
    /// risk-book/graph write (phase 4 reconcile, `docs/FI-RISK-ROUTING-REQUIREMENTS.md`
    /// §7). `None` in an isolated auth test (risk CRUD then persists only, exactly as
    /// `aggregation_hub` is `None` there).
    position_store: Option<Arc<PositionStore>>,
    /// The shared **linear-rates** position store whose risk router this edge re-primes
    /// beside the FX store on every risk-book/graph write, so a routed rates fill buckets
    /// into its risk book (and `ListRiskBookRisk` sums it). `None` in an isolated auth test.
    rates_store: Option<Arc<crate::services::rates_book::RatesPositionStore>>,
    /// The shared risk-transfer service backing the transfer RPCs (initiate / accept /
    /// reject / cancel / list). `None` in an isolated auth test (the transfer RPCs then
    /// report `unavailable`, exactly as the other injected collaborators do).
    transfer_service: Option<Arc<crate::services::risk_transfer::RiskTransferService>>,
    /// The cross-asset **client-flow analytics** sources folded by the
    /// `ListClientFlowMetrics` RPC — one per contributing edge (the FXO quote edge,
    /// the FI desk edge). Each maps its OWN already-captured history into neutral
    /// `FlowRecord`s on-query, off the hot path (Analytics phase 2,
    /// `docs/ANALYTICS-REQUIREMENTS.md` §11). Empty in an isolated auth test (the
    /// RPC then returns an empty roster, exactly as the other collaborators no-op).
    analytics_sources: Vec<Arc<dyn crate::services::analytics::ClientFlowSource>>,
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
            aggregation_hub: None,
            position_store: None,
            rates_store: None,
            transfer_service: None,
            analytics_sources: Vec::new(),
        }
    }

    /// Register a client-flow analytics source (the FXO quote edge, the FI desk edge).
    /// Each contributes its own already-captured history to the `ListClientFlowMetrics`
    /// fold. Chainable; order is irrelevant (the fold is a set union then a group-by).
    #[must_use]
    pub fn with_client_flow_source(
        mut self,
        source: Arc<dyn crate::services::analytics::ClientFlowSource>,
    ) -> Self {
        self.analytics_sources.push(source);
        self
    }

    /// Inject the shared risk-transfer service so the transfer RPCs are live (the boot
    /// path shares the SAME registry / inbox broker / apply engine the notification
    /// stream and the position stores are wired into).
    #[must_use]
    pub fn with_transfer_service(
        mut self,
        service: Arc<crate::services::risk_transfer::RiskTransferService>,
    ) -> Self {
        self.transfer_service = Some(service);
        self
    }

    /// The wired transfer service, or a uniform `unavailable` status when none is
    /// injected (an isolated auth test / a misconfigured boot).
    fn transfer_service(
        &self,
    ) -> Result<&Arc<crate::services::risk_transfer::RiskTransferService>, Status> {
        self.transfer_service
            .as_ref()
            .ok_or_else(|| Status::unavailable("risk transfer service is not wired on this edge"))
    }

    /// Inject the shared position store so a risk-book/graph write re-primes its router
    /// (the boot path shares the SAME store the stream / RFQ / risk edges book into).
    #[must_use]
    pub fn with_position_store(mut self, store: Arc<PositionStore>) -> Self {
        self.position_store = Some(store);
        self
    }

    /// Inject the shared **linear-rates** position store so a risk-book/graph write also
    /// re-primes its router (the boot path shares the SAME rates store the RiskService rates
    /// Book/List and the RFQ desk book into), and so `ListRiskBookRisk` sums the rates
    /// positions routed into each book.
    #[must_use]
    pub fn with_rates_store(
        mut self,
        store: Arc<crate::services::rates_book::RatesPositionStore>,
    ) -> Self {
        self.rates_store = Some(store);
        self
    }

    /// Push the committed routing graph into the shared position store's router (a no-op
    /// when no store is wired). Called after every risk-book/graph write so defining /
    /// editing / clearing the firm-wide graph — or disabling a target book — takes effect
    /// on subsequent fills immediately. The SAME operation is run once at boot from the
    /// persisted store (`celnet-server/src/lib.rs`, beside the aggregation reconcile).
    fn reconcile_risk_routing(&self, store: &IdentityStore) {
        if let Some(position_store) = &self.position_store {
            position_store.set_routing(store.risk_routing_graph().cloned());
            // Keep the store's per-book limit view current beside the graph, so a book
            // create/update/delete refreshes the caps the per-book booking gate enforces.
            position_store.set_risk_books(
                store
                    .risk_books
                    .iter()
                    .map(crate::services::risk::store::RiskBookLimitDef::from)
                    .collect(),
            );
            // Keep the store's full risk-book tree current beside the limit view, so the
            // live per-book risk stream re-publishes its roster on a book create/update/
            // delete/enable (`set_risk_book_tree` advances the store's risk version).
            position_store.set_risk_book_tree(store.risk_books.clone());
        }
        // Re-prime the shared LINEAR-RATES store's router + per-book limit view beside the FX
        // store, so a routed rates fill buckets into (and is capped by) the same books. The
        // rates store carries no risk-book *tree* (the streamed roster is driven off the FX
        // store's tree; the rates store only needs the routing graph + caps).
        if let Some(rates_store) = &self.rates_store {
            rates_store.set_routing(store.risk_routing_graph().cloned());
            rates_store.set_risk_books(
                store
                    .risk_books
                    .iter()
                    .map(crate::services::risk::store::RiskBookLimitDef::from)
                    .collect(),
            );
        }
    }

    /// Inject the edge-wide aggregated-book engine hub so book CRUD re-reconciles the
    /// running engines (the boot path shares the SAME hub the stream + LP ingest
    /// services use).
    #[must_use]
    pub fn with_aggregation_hub(
        mut self,
        hub: Arc<crate::services::aggregation::AggregationHub>,
    ) -> Self {
        self.aggregation_hub = Some(hub);
        self
    }

    /// Re-reconcile the aggregated-book engines from the committed store (a no-op
    /// when no hub is wired).
    fn reconcile_aggregation(&self, store: &IdentityStore) {
        if let Some(hub) = &self.aggregation_hub {
            hub.reconcile(store);
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

    /// Resolve a bearer token and require it to hold `cap` under the deny-by-default,
    /// deny-wins capability algebra ([`AuthenticatedUser::capabilities`]). Unlike
    /// [`require_admin`](Self::require_admin), an ordinary trader who holds the specific
    /// action×asset passes — the bar is a granted capability, not the admin role — while
    /// a trader explicitly **denied** it (separation of duties) is refused. An unknown /
    /// expired token is `unauthenticated`; a valid token lacking the capability is
    /// `permission_denied`.
    fn require_capability(
        &self,
        token: &str,
        cap: Capability,
    ) -> Result<AuthenticatedUser, Status> {
        let who = self.authenticate(token)?;
        if who.capabilities().allows(cap) {
            Ok(who)
        } else {
            Err(Status::permission_denied(format!(
                "capability {}·{} required",
                cap.action.label(),
                cap.asset.label()
            )))
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
            // Security-relevant: an email under active brute-force lockout. The
            // external contract is unchanged (`resource_exhausted`); the precise
            // internal reason is recorded for investigation.
            tracing::warn!(
                class = celnet_observability::LogClass::Security.label(),
                email = %email_key,
                reason = "rate_limited",
                retry_secs,
                "login failed"
            );
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
        //
        // `fail_reason` records the PRECISE internal cause for the security log
        // while the external error below stays a single opaque
        // `unauthenticated` (anti-enumeration): the client cannot tell an unknown
        // email from a bad password, but an operator can.
        let (authed_user, ok, fail_reason): (Option<UserDef>, bool, &'static str) = match candidate
        {
            Some(u) if u.disabled => {
                let _ = verify_async(dummy_hash().to_string(), req.password.clone()).await;
                (None, false, "disabled")
            }
            Some(u) => {
                let ok = verify_async(u.password_hash.clone(), req.password.clone()).await;
                let reason = if ok { "ok" } else { "bad_password" };
                (Some(u), ok, reason)
            }
            None => {
                let _ = verify_async(dummy_hash().to_string(), req.password.clone()).await;
                (None, false, "unknown_user")
            }
        };

        let Some(user) = authed_user.filter(|_| ok) else {
            self.throttle.record_failure(&email_key);
            tracing::warn!(
                class = celnet_observability::LogClass::Security.label(),
                email = %email_key,
                reason = fail_reason,
                "login failed"
            );
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
        // Successful authentication is a security-relevant event. The session
        // TOKEN is never logged (only its expiry); the resolved role is recorded
        // so an operator can see which authority was minted.
        tracing::info!(
            class = celnet_observability::LogClass::Security.label(),
            email = %user.email,
            role = role_label(user.role),
            session_expires_nanos = issued.expires_nanos,
            "login succeeded"
        );
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
        // Resolve the actor before ending the session so the security log can
        // attribute the logout by email; the token value itself is never logged.
        let actor_email = self.sessions.validate(&req.session_token).map(|u| u.email);
        let ended = self.sessions.logout(&req.session_token);
        tracing::info!(
            class = celnet_observability::LogClass::Security.label(),
            email = actor_email.as_deref(),
            ended,
            "logout"
        );
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
        let actor = self.require_admin(&req.session_token)?;

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
        let (all_desks, desk_ids) = normalize_membership(req.desk_ids, req.all_desks);

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
        // Every named desk must exist (an `all_desks` user carries no explicit set).
        for desk in &desk_ids {
            if guard.desk(desk).is_none() {
                return Err(Status::failed_precondition(format!(
                    "no desk with id `{desk}`"
                )));
            }
        }
        let new_user = UserDef {
            id: mint_user_id(&email, &guard.users),
            email,
            display_name,
            role,
            desk_ids,
            all_desks,
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
        tracing::info!(
            class = celnet_observability::LogClass::Security.label(),
            actor = %actor.email,
            target_email = %new_user.email,
            role = role_label(new_user.role),
            "admin created user"
        );
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
        let actor = self.require_admin(&req.session_token)?;

        let display_name = req.display_name.trim().to_string();
        if display_name.is_empty() {
            return Err(Status::invalid_argument("display name is required"));
        }
        let role = role_from_wire(req.role)?;
        let (all_desks, desk_ids) = normalize_membership(req.desk_ids, req.all_desks);

        let mut guard = self.lock();
        let Some(old) = guard.user(&req.id).cloned() else {
            return Err(Status::not_found(format!("no user with id `{}`", req.id)));
        };
        // Every named desk must exist (an `all_desks` user carries no explicit set).
        for desk in &desk_ids {
            if guard.desk(desk).is_none() {
                return Err(Status::failed_precondition(format!(
                    "no desk with id `{desk}`"
                )));
            }
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
            desk_ids,
            all_desks,
            password_hash: old.password_hash.clone(),
            disabled: req.disabled,
            // Preserve the per-user capability overlay — this RPC edits identity
            // (role/desk/disabled), never the overlay, so it must not silently wipe
            // it. Overlay editing is its own admin RPC (slice 3b).
            capability_grants: old.capability_grants.clone(),
            capability_denies: old.capability_denies.clone(),
        };
        let authority_changed = updated.role != old.role
            || updated.desk_ids != old.desk_ids
            || updated.all_desks != old.all_desks
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
        tracing::info!(
            class = celnet_observability::LogClass::Security.label(),
            actor = %actor.email,
            target_email = %updated.email,
            role = role_label(updated.role),
            disabled = updated.disabled,
            authority_changed,
            "admin updated user"
        );
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
        let actor = self.require_admin(&req.session_token)?;

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
        tracing::info!(
            class = celnet_observability::LogClass::Security.label(),
            actor = %actor.email,
            target_email = %target.email,
            target_user_id = %req.id,
            "admin deleted user"
        );
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
        let actor = self.require_admin(&req.session_token)?;

        check_password_strength(&req.new_password)?;
        // Confirm the target exists before paying for a hash, capturing its email
        // for the security log (the new password is NEVER logged).
        let target_email = {
            let store = self.lock();
            match store.user(&req.id) {
                Some(u) => u.email.clone(),
                None => return Err(Status::not_found(format!("no user with id `{}`", req.id))),
            }
        };
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
        tracing::info!(
            class = celnet_observability::LogClass::Security.label(),
            actor = %actor.email,
            target_email = %target_email,
            "admin reset user password"
        );
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
            desk_ids: old.desk_ids.clone(),
            all_desks: old.all_desks,
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

    async fn update_desk(
        &self,
        request: Request<UpdateDeskRequest>,
    ) -> Result<Response<UpdateDeskResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_admin(&req.session_token)?;

        let name = req.name.trim().to_string();
        if name.is_empty() {
            return Err(Status::invalid_argument("desk name is required"));
        }
        let mut guard = self.lock();
        if guard.desk(&req.id).is_none() {
            return Err(Status::not_found(format!("no desk with id `{}`", req.id)));
        }
        // Uniqueness is checked against every OTHER desk (the edited one may keep its
        // own name). The new name is only a display label — a desk's stable `id`
        // never changes, so this rename does NOT touch routing: RFQ/deal notification
        // delivery and connection ownership both key on `DeskDef::id`, and every
        // user's desk membership (`desk_ids`) is untouched. No session is revoked (no
        // authority or routing change), so live sessions stay valid.
        if guard
            .desks
            .iter()
            .any(|d| d.id != req.id && d.name.eq_ignore_ascii_case(&name))
        {
            return Err(Status::already_exists(format!(
                "a desk named `{name}` already exists"
            )));
        }
        let mut next = guard.clone();
        let desk = {
            let slot = next
                .desks
                .iter_mut()
                .find(|d| d.id == req.id)
                .expect("desk existence checked above");
            slot.name = name;
            slot.clone()
        };
        self.persist_and_commit(&mut guard, next)?;
        Ok(Response::new(UpdateDeskResponse {
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
        // The deleted desk is dropped from every member's set (a multi-desk member
        // keeps its other desks); their snapshots change, so their sessions are
        // revoked after commit. An `all_desks` user carries no explicit set and is
        // unaffected.
        let affected: Vec<String> = guard
            .users
            .iter()
            .filter(|u| u.desk_ids.iter().any(|d| d == &req.id))
            .map(|u| u.id.clone())
            .collect();
        let mut next = guard.clone();
        next.desks.retain(|d| d.id != req.id);
        for u in &mut next.users {
            u.desk_ids.retain(|d| d != &req.id);
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

    // --- FI aggregated books (ADR-0022) ----------------------------------------

    async fn list_aggregated_books(
        &self,
        request: Request<ListAggregatedBooksRequest>,
    ) -> Result<Response<ListAggregatedBooksResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        // Read-only roster: a book is global (ADR-0022 decision C), so any
        // authenticated caller may read its composite — NOT admin-gated.
        self.authenticate(&req.session_token)?;
        let books = self
            .lock()
            .aggregated_books
            .iter()
            .map(aggregated_book_to_wire)
            .collect();
        Ok(Response::new(ListAggregatedBooksResponse {
            books,
            correlation_id: req.correlation_id,
        }))
    }

    async fn create_aggregated_book(
        &self,
        request: Request<CreateAggregatedBookRequest>,
    ) -> Result<Response<CreateAggregatedBookResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        // Aggregated-book config is inbound-liquidity / venue ops — gated on
        // `manage_liquidity·fixed_income`, NOT super-admin (`docs/PERMISSIONS-GRANULAR-REVIEW.md` §4).
        self.require_capability(
            &req.session_token,
            Capability::new(Action::ManageLiquidity, AssetClass::FixedIncome),
        )?;

        let spec = req
            .spec
            .ok_or_else(|| Status::invalid_argument("aggregated book spec is required"))?;
        let edit = spec_parts(spec);

        // The store validates the definition's document-resolvable invariants (unique
        // name, no duplicate members, in-range params, explicit-scope instrument ids
        // resolving) inside `create_aggregated_book`, so disk and the wire reject a bad
        // book identically. Build on a clone and commit only after the atomic persist.
        let mut guard = self.lock();
        let mut next = guard.clone();
        let def = next
            .create_aggregated_book(edit)
            .map_err(aggregated_book_status)?;
        self.persist_and_commit(&mut guard, next)?;
        self.reconcile_aggregation(&guard);
        Ok(Response::new(CreateAggregatedBookResponse {
            book: Some(aggregated_book_to_wire(&def)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn update_aggregated_book(
        &self,
        request: Request<UpdateAggregatedBookRequest>,
    ) -> Result<Response<UpdateAggregatedBookResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_capability(
            &req.session_token,
            Capability::new(Action::ManageLiquidity, AssetClass::FixedIncome),
        )?;

        let spec = req
            .spec
            .ok_or_else(|| Status::invalid_argument("aggregated book spec is required"))?;
        let edit = spec_parts(spec);

        let mut guard = self.lock();
        if guard.aggregated_book(&req.id).is_none() {
            return Err(Status::not_found(format!(
                "no aggregated book with id `{}`",
                req.id
            )));
        }
        let mut next = guard.clone();
        let def = next
            .update_aggregated_book(&req.id, edit)
            .map_err(aggregated_book_status)?;
        self.persist_and_commit(&mut guard, next)?;
        self.reconcile_aggregation(&guard);
        Ok(Response::new(UpdateAggregatedBookResponse {
            book: Some(aggregated_book_to_wire(&def)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn delete_aggregated_book(
        &self,
        request: Request<DeleteAggregatedBookRequest>,
    ) -> Result<Response<DeleteAggregatedBookResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_capability(
            &req.session_token,
            Capability::new(Action::ManageLiquidity, AssetClass::FixedIncome),
        )?;

        let mut guard = self.lock();
        let mut next = guard.clone();
        let removed = next
            .delete_aggregated_book(&req.id)
            .map_err(aggregated_book_status)?;
        if removed {
            self.persist_and_commit(&mut guard, next)?;
            self.reconcile_aggregation(&guard);
        }
        Ok(Response::new(DeleteAggregatedBookResponse {
            removed,
            correlation_id: req.correlation_id,
        }))
    }

    // --- pricing groups (FI client-tiering) ------------------------------------

    async fn list_pricing_groups(
        &self,
        request: Request<ListPricingGroupsRequest>,
    ) -> Result<Response<ListPricingGroupsResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        // Read-only roster: any authenticated caller may load the groups (the GUI needs
        // them to render the pricing-group admin surface); assignment is admin-owned but
        // the roster itself is NOT admin-gated.
        self.authenticate(&req.session_token)?;
        let groups = self
            .lock()
            .pricing_groups
            .iter()
            .map(pricing_group_to_wire)
            .collect();
        Ok(Response::new(ListPricingGroupsResponse {
            groups,
            correlation_id: req.correlation_id,
        }))
    }

    async fn create_pricing_group(
        &self,
        request: Request<CreatePricingGroupRequest>,
    ) -> Result<Response<CreatePricingGroupResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        // Group STRUCTURE and MEMBERSHIP (id/name/members/enabled + pipelines) — and
        // therefore session-pivoted tiering ASSIGNMENT, which is a member_connection_ids
        // edit — is gated on `manage_pricing·fixed_income` (the FI client-pricing-desk
        // authority), NOT super-admin (`docs/PERMISSIONS-GRANULAR-REVIEW.md` §3.1/§4). The
        // per-mode pipeline RETUNE stays a quoting-trader knob on `quote_respond·FI`
        // (see `update_pricing_group_pipeline`).
        self.require_capability(
            &req.session_token,
            Capability::new(Action::ManagePricing, AssetClass::FixedIncome),
        )?;

        let spec = req
            .spec
            .ok_or_else(|| Status::invalid_argument("pricing group spec is required"))?;
        let edit = pricing_group_spec_parts(spec);

        // The store validates the group's document-resolvable invariants (unique name,
        // known user/desk members, valid pipelines) AND the cross-group determinism rule
        // inside `create_pricing_group`, so disk and the wire reject a bad group
        // identically. Build on a clone and commit only after the atomic persist.
        let mut guard = self.lock();
        let mut next = guard.clone();
        let def = next
            .create_pricing_group(edit)
            .map_err(pricing_group_status)?;
        self.persist_and_commit(&mut guard, next)?;
        // Rebuild the caller→group resolver so the ESP/RFQ pricing paths re-resolve.
        self.reconcile_aggregation(&guard);
        Ok(Response::new(CreatePricingGroupResponse {
            group: Some(pricing_group_to_wire(&def)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn update_pricing_group(
        &self,
        request: Request<UpdatePricingGroupRequest>,
    ) -> Result<Response<UpdatePricingGroupResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_capability(
            &req.session_token,
            Capability::new(Action::ManagePricing, AssetClass::FixedIncome),
        )?;

        let spec = req
            .spec
            .ok_or_else(|| Status::invalid_argument("pricing group spec is required"))?;
        let edit = pricing_group_spec_parts(spec);

        let mut guard = self.lock();
        if guard.pricing_group(&req.id).is_none() {
            return Err(Status::not_found(format!(
                "no pricing group with id `{}`",
                req.id
            )));
        }
        let mut next = guard.clone();
        let def = next
            .update_pricing_group(&req.id, edit)
            .map_err(pricing_group_status)?;
        self.persist_and_commit(&mut guard, next)?;
        self.reconcile_aggregation(&guard);
        Ok(Response::new(UpdatePricingGroupResponse {
            group: Some(pricing_group_to_wire(&def)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn delete_pricing_group(
        &self,
        request: Request<DeletePricingGroupRequest>,
    ) -> Result<Response<DeletePricingGroupResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_capability(
            &req.session_token,
            Capability::new(Action::ManagePricing, AssetClass::FixedIncome),
        )?;

        let mut guard = self.lock();
        let mut next = guard.clone();
        let removed = next
            .delete_pricing_group(&req.id)
            .map_err(pricing_group_status)?;
        if removed {
            self.persist_and_commit(&mut guard, next)?;
            self.reconcile_aggregation(&guard);
        }
        Ok(Response::new(DeletePricingGroupResponse {
            removed,
            correlation_id: req.correlation_id,
        }))
    }

    async fn update_pricing_group_pipeline(
        &self,
        request: Request<UpdatePricingGroupPipelineRequest>,
    ) -> Result<Response<UpdatePricingGroupPipelineResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        // Trader-configurable outbound pricing: gated on the fixed-income quote-respond
        // capability (publishing a tradeable FI quote), NOT the admin role. The group's
        // STRUCTURE and MEMBERSHIP stay admin-owned: this RPC touches ONLY the selected
        // mode's pipeline + the share flag (see `update_pricing_group_pipeline`).
        self.require_capability(
            &req.session_token,
            Capability::new(Action::QuoteRespond, AssetClass::FixedIncome),
        )?;

        let mode = pricing_mode_from_wire(req.mode);
        // An absent / malformed pipeline is rejected below by the store's
        // `validate_feature_pipeline` (loud failure at the write), never silently dropped.
        let pipeline = feature_pipeline_from_wire(req.pipeline);

        let mut guard = self.lock();
        if guard.pricing_group(&req.group_id).is_none() {
            return Err(Status::not_found(format!(
                "no pricing group with id `{}`",
                req.group_id
            )));
        }
        let mut next = guard.clone();
        let def = next
            .update_pricing_group_pipeline(&req.group_id, mode, pipeline, req.share_pipeline)
            .map_err(pricing_group_status)?;
        self.persist_and_commit(&mut guard, next)?;
        // Re-reconcile so the running ESP/RFQ pricing paths pick up the retuned pipeline.
        self.reconcile_aggregation(&guard);
        Ok(Response::new(UpdatePricingGroupPipelineResponse {
            group: Some(pricing_group_to_wire(&def)),
            correlation_id: req.correlation_id,
        }))
    }

    // --- risk routing & risk books ---------------------------------------------
    //
    // All six RPCs gate on the dedicated `risk_manage·fixed_income` capability: the
    // risk-portfolio tree (`RiskBookDef` CRUD), the risk-routing decision graph
    // (get/update) and the firm-wide routed-risk roll-up are a single FI risk-control
    // authority, granted to a desk/risk lead **without** full administration and held
    // back from the default trader bundle (`docs/PERMISSIONS-GRANULAR-REVIEW.md` §3.1/§4).
    // This replaces the earlier coarse gates — `require_admin` on the CRUD and the
    // overloaded `quote_respond·fixed_income` on the reads/routing. Every mutation
    // follows the pricing-group pattern — require_ready → require_capability → lock →
    // clone → apply the phase-2 `IdentityStore` CRUD → `persist_and_commit` — and then,
    // additionally, pushes the live graph into the shared position store's router
    // (`reconcile_risk_routing`) so a defined/edited/cleared graph (or a disabled target
    // book) takes effect on subsequent fills immediately.

    async fn list_risk_books(
        &self,
        request: Request<ListRiskBooksRequest>,
    ) -> Result<Response<ListRiskBooksResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        // Reading the risk-book roster + defining routing is an FI risk-management task —
        // gated on the dedicated `risk_manage·fixed_income` capability (a risk lead holds
        // it), NOT admin-only and NOT the overloaded quoting capability.
        self.require_capability(
            &req.session_token,
            Capability::new(Action::RiskManage, AssetClass::FixedIncome),
        )?;
        let books = self
            .lock()
            .risk_books
            .iter()
            .map(risk_book_to_wire)
            .collect();
        Ok(Response::new(ListRiskBooksResponse {
            books,
            correlation_id: req.correlation_id,
        }))
    }

    async fn create_risk_book(
        &self,
        request: Request<CreateRiskBookRequest>,
    ) -> Result<Response<CreateRiskBookResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_capability(
            &req.session_token,
            Capability::new(Action::RiskManage, AssetClass::FixedIncome),
        )?;

        let spec = req
            .spec
            .ok_or_else(|| Status::invalid_argument("risk book spec is required"))?;
        let edit = risk_book_spec_parts(spec);

        let mut guard = self.lock();
        let mut next = guard.clone();
        let def = next.create_risk_book(edit).map_err(risk_book_status)?;
        self.persist_and_commit(&mut guard, next)?;
        // A new book cannot yet be a routing target, but keep the router primed off the
        // committed store for a single, uniform post-write reconcile path.
        self.reconcile_risk_routing(&guard);
        Ok(Response::new(CreateRiskBookResponse {
            book: Some(risk_book_to_wire(&def)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn update_risk_book(
        &self,
        request: Request<UpdateRiskBookRequest>,
    ) -> Result<Response<UpdateRiskBookResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_capability(
            &req.session_token,
            Capability::new(Action::RiskManage, AssetClass::FixedIncome),
        )?;

        let spec = req
            .spec
            .ok_or_else(|| Status::invalid_argument("risk book spec is required"))?;
        let edit = risk_book_spec_parts(spec);

        let mut guard = self.lock();
        if guard.risk_book(&req.id).is_none() {
            return Err(Status::not_found(format!(
                "no risk book with id `{}`",
                req.id
            )));
        }
        let mut next = guard.clone();
        let def = next
            .update_risk_book(&req.id, edit)
            .map_err(risk_book_status)?;
        self.persist_and_commit(&mut guard, next)?;
        // Disabling a book (or otherwise editing it) can change graph validity; re-prime
        // the router off the committed store.
        self.reconcile_risk_routing(&guard);
        Ok(Response::new(UpdateRiskBookResponse {
            book: Some(risk_book_to_wire(&def)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn delete_risk_book(
        &self,
        request: Request<DeleteRiskBookRequest>,
    ) -> Result<Response<DeleteRiskBookResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_capability(
            &req.session_token,
            Capability::new(Action::RiskManage, AssetClass::FixedIncome),
        )?;

        let mut guard = self.lock();
        let mut next = guard.clone();
        let removed = next.delete_risk_book(&req.id).map_err(risk_book_status)?;
        if removed {
            self.persist_and_commit(&mut guard, next)?;
            self.reconcile_risk_routing(&guard);
        }
        Ok(Response::new(DeleteRiskBookResponse {
            removed,
            correlation_id: req.correlation_id,
        }))
    }

    async fn get_risk_routing_graph(
        &self,
        request: Request<GetRiskRoutingGraphRequest>,
    ) -> Result<Response<GetRiskRoutingGraphResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_capability(
            &req.session_token,
            Capability::new(Action::RiskManage, AssetClass::FixedIncome),
        )?;
        let graph = self.lock().risk_routing_graph().map(routing_graph_to_wire);
        Ok(Response::new(GetRiskRoutingGraphResponse {
            graph,
            correlation_id: req.correlation_id,
        }))
    }

    async fn update_risk_routing_graph(
        &self,
        request: Request<UpdateRiskRoutingGraphRequest>,
    ) -> Result<Response<UpdateRiskRoutingGraphResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_capability(
            &req.session_token,
            Capability::new(Action::RiskManage, AssetClass::FixedIncome),
        )?;

        let wire = req
            .graph
            .ok_or_else(|| Status::invalid_argument("risk routing graph is required"))?;
        let graph = routing_graph_from_wire(wire)?;

        let mut guard = self.lock();
        let mut next = guard.clone();
        // The store re-validates the graph against the live risk-book registry (acyclic,
        // every path terminates at a known ENABLED book, type-consistent conditions), so a
        // malformed graph fails loudly at the write and never reaches the router.
        next.set_risk_routing_graph(graph.clone())
            .map_err(risk_routing_status)?;
        self.persist_and_commit(&mut guard, next)?;
        self.reconcile_risk_routing(&guard);
        Ok(Response::new(UpdateRiskRoutingGraphResponse {
            graph: Some(routing_graph_to_wire(&graph)),
            correlation_id: req.correlation_id,
        }))
    }

    // Read-only per-book risk aggregation, rolled up the book tree (phase 5 — §5, §8.5).
    // Gated on `risk_manage·fixed_income` like the routing/CRUD RPCs: seeing the firm-wide
    // routed-risk roll-up is a risk-control authority, not something every FI quoter holds
    // (`docs/PERMISSIONS-GRANULAR-REVIEW.md` §4). Reads only: the identity registry for the
    // book tree and the shared position store for the routed facts; no mutation, no persist.
    async fn list_risk_book_risk(
        &self,
        request: Request<ListRiskBookRiskRequest>,
    ) -> Result<Response<ListRiskBookRiskResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        self.require_capability(
            &req.session_token,
            Capability::new(Action::RiskManage, AssetClass::FixedIncome),
        )?;

        // Aggregate every ENABLED risk book, rolled up its subtree. When no position store
        // is wired (only some non-serving constructions), the books still list with their
        // zero-position roll-up (`aggregate_risk_book` over an empty store) — never omitted
        // and never a fabricated non-zero.
        let guard = self.lock();
        let rates = self.rates_store.as_deref();
        let books = match &self.position_store {
            Some(store) => guard
                .risk_books
                .iter()
                .filter(|b| b.enabled)
                .map(|b| risk_book_risk_to_wire(&aggregate_risk_book(store, rates, &guard, b)))
                .collect(),
            None => {
                let empty = PositionStore::new();
                guard
                    .risk_books
                    .iter()
                    .filter(|b| b.enabled)
                    .map(|b| risk_book_risk_to_wire(&aggregate_risk_book(&empty, rates, &guard, b)))
                    .collect()
            }
        };
        Ok(Response::new(ListRiskBookRiskResponse {
            books,
            correlation_id: req.correlation_id,
        }))
    }

    // --- risk transfer ---------------------------------------------------------
    //
    // The MANUAL move of EXISTING risk between books / desks / traders (the complement
    // to routing). Every RPC gates on the narrow `risk_transfer` capability × the
    // position's asset class (resolved from where the positions live), NOT admin. A
    // desk-to-desk / trader-to-trader accept additionally enforces approver ≠ initiator
    // (four-eyes) inside the service. `docs/RISK-TRANSFER-REQUIREMENTS.md` §7.

    async fn initiate_risk_transfer(
        &self,
        request: Request<InitiateRiskTransferRequest>,
    ) -> Result<Response<InitiateRiskTransferResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let correlation_id = req.correlation_id;
        let service = Arc::clone(self.transfer_service()?);
        let source = req
            .source
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("a source leg is required"))?;
        let asset = service.classify_asset_class(&source.risk_book_id, &source.position_ids)?;
        let user = self.require_capability(
            &req.session_token,
            Capability::new(Action::RiskTransfer, asset),
        )?;
        let transfer = service.initiate(&user.email, req)?;
        Ok(Response::new(InitiateRiskTransferResponse {
            transfer: Some(transfer),
            correlation_id,
        }))
    }

    async fn accept_risk_transfer(
        &self,
        request: Request<AcceptRiskTransferRequest>,
    ) -> Result<Response<AcceptRiskTransferResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let service = Arc::clone(self.transfer_service()?);
        let asset = service.asset_class_of(&req.transfer_id)?;
        let user = self.require_capability(
            &req.session_token,
            Capability::new(Action::RiskTransfer, asset),
        )?;
        let transfer = service.accept(&user.email, &req.transfer_id)?;
        Ok(Response::new(AcceptRiskTransferResponse {
            transfer: Some(transfer),
            correlation_id: req.correlation_id,
        }))
    }

    async fn reject_risk_transfer(
        &self,
        request: Request<RejectRiskTransferRequest>,
    ) -> Result<Response<RejectRiskTransferResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let service = Arc::clone(self.transfer_service()?);
        let asset = service.asset_class_of(&req.transfer_id)?;
        let user = self.require_capability(
            &req.session_token,
            Capability::new(Action::RiskTransfer, asset),
        )?;
        let transfer = service.reject(&user.email, &req.transfer_id, &req.reason)?;
        Ok(Response::new(RejectRiskTransferResponse {
            transfer: Some(transfer),
            correlation_id: req.correlation_id,
        }))
    }

    async fn cancel_risk_transfer(
        &self,
        request: Request<CancelRiskTransferRequest>,
    ) -> Result<Response<CancelRiskTransferResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let service = Arc::clone(self.transfer_service()?);
        let asset = service.asset_class_of(&req.transfer_id)?;
        let user = self.require_capability(
            &req.session_token,
            Capability::new(Action::RiskTransfer, asset),
        )?;
        let transfer = service.cancel(&user.email, &req.transfer_id)?;
        Ok(Response::new(CancelRiskTransferResponse {
            transfer: Some(transfer),
            correlation_id: req.correlation_id,
        }))
    }

    async fn list_risk_transfers(
        &self,
        request: Request<ListRiskTransfersRequest>,
    ) -> Result<Response<ListRiskTransfersResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let service = Arc::clone(self.transfer_service()?);
        // The audit blotter is readable by any authenticated caller (the write path is
        // capability-gated; the read is the immutable trail).
        self.authenticate(&req.session_token)?;
        let transfers = service.list(&req);
        Ok(Response::new(ListRiskTransfersResponse {
            transfers,
            correlation_id: req.correlation_id,
        }))
    }

    // --- client-flow analytics -------------------------------------------------
    //
    // The cross-product per-client flow / P&L-attribution rollup ($/mm, spread
    // economics, quote-fishing) folded from every registered source (the FXO quote
    // edge + the FI desk edge). A management-sensitive READ gated on the dedicated
    // `view_analytics` capability × the caller's assets — spanning FI+FXO, NOT
    // admin-only. Pure fold over already-captured data, off the hot path
    // (`docs/ANALYTICS-REQUIREMENTS.md` §11).

    async fn list_client_flow_metrics(
        &self,
        request: Request<ListClientFlowMetricsRequest>,
    ) -> Result<Response<ListClientFlowMetricsResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        // Cross-product gate: `view_analytics` on EITHER asset admits the caller (a
        // manager holding it on any asset may open the Analytics surface). Deny-wins
        // still applies via the capability algebra.
        let who = self.authenticate(&req.session_token)?;
        let caps = who.capabilities();
        let allowed = caps.allows(Capability::new(Action::ViewAnalytics, AssetClass::FxOptions))
            || caps.allows(Capability::new(
                Action::ViewAnalytics,
                AssetClass::FixedIncome,
            ));
        if !allowed {
            return Err(Status::permission_denied(
                "capability view_analytics·{fx_options|fixed_income} required",
            ));
        }

        let records = crate::services::analytics::collect(
            &self.analytics_sources,
            req.from_nanos,
            req.to_nanos,
        )
        .await;
        let metrics = crate::services::analytics::fold(&records, req.group_by)
            .values()
            .map(crate::services::analytics::metrics_to_wire)
            .collect();
        Ok(Response::new(ListClientFlowMetricsResponse {
            metrics,
            group_by: req.group_by,
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

        if req.pillars.is_empty() && req.date_pillars.is_empty() {
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
        // `calibration_set` preserves input order, so the resolved instrument pillars line
        // up with `resolved`. Every `CurveCalibrationError` is a static input-shape failure
        // (uncalibratable family, unknown convention label, currency mismatch, …).
        let defs: Vec<(&InstrumentDef, f64)> =
            resolved.iter().map(|(_, def, q)| (def, *q)).collect();
        let instrument_calibs =
            calibration_set(&defs, value_date).map_err(curve_calibration_status)?;

        // Assemble the unified calibration ladder — registry-instrument pillars then
        // standalone date-anchored pillars — keeping each pillar's display metadata
        // (`(instrument_id, label)`) index-aligned with the engine instrument. Date
        // pillars carry an empty instrument id (the client resolves names only by id) and
        // a `Date YYYY-MM-DD` label; the bootstrap orders the ladder by maturity itself.
        let pillar_count = resolved.len() + req.date_pillars.len();
        let mut instruments: Vec<CalibrationInstrument> = Vec::with_capacity(pillar_count);
        let mut meta: Vec<(String, String)> = Vec::with_capacity(pillar_count);
        for ((id, _, _), inst) in resolved.iter().zip(instrument_calibs) {
            meta.push((id.clone(), String::new()));
            instruments.push(inst);
        }
        for dp in &req.date_pillars {
            let md = dp.maturity_date.as_ref().ok_or_else(|| {
                Status::invalid_argument("date pillar is missing `maturity_date`")
            })?;
            let maturity = decode_broken_date(md).ok_or_else(|| {
                Status::invalid_argument("date pillar `maturity_date` is not a real calendar date")
            })?;
            let inst = date_pillar_instrument(value_date, maturity, dp.quote)
                .map_err(curve_calibration_status)?;
            let label = format!("Date {:04}-{:02}-{:02}", md.year, md.month, md.day);
            meta.push((String::new(), label));
            instruments.push(inst);
        }

        // Bootstrap the self-discounting curve. A failure here is a numeric fault on
        // otherwise-valid input (mirrors the `price_rates` edge mapping it to internal).
        let curve = bootstrap_curve(&instruments)
            .map_err(|e| Status::internal(format!("curve bootstrap failed: {e}")))?;

        // Sample each input instrument at its own pillar maturity, ordered short → long
        // by maturity (the documented, deterministic point order).
        let mut points: Vec<CalibratedCurvePoint> = instruments
            .iter()
            .zip(&meta)
            .map(|(inst, (id, label))| {
                let t = inst.maturity();
                CalibratedCurvePoint {
                    instrument_id: id.clone(),
                    time_years: t.0,
                    discount_factor: curve.discount_factor(t).0,
                    zero_rate: curve.zero_rate(t).0,
                    label: label.clone(),
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
        desk_ids: u.desk_ids.clone(),
        disabled: u.disabled,
        all_desks: u.all_desks,
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

/// Map a stored [`AggregationParams`] onto its wire [`AggregationParamsDesc`]
/// (field-for-field; the store and the wire carry the same shape).
fn params_to_wire(p: &AggregationParams) -> AggregationParamsDesc {
    AggregationParamsDesc {
        staleness_tau_ms: p.staleness_tau_ms,
        max_quote_age_ms: p.max_quote_age_ms,
        divergence_gating: p.divergence_gating,
        min_contributors: p.min_contributors,
        depth_levels: p.depth_levels,
    }
}

/// Split a stored [`Scope`] into its wire `(scope_mode, instrument_ids)` pair. The
/// store's adjacently-tagged enum becomes a flat enum + repeated-string on the wire;
/// `AllMembersQuote` carries no ids.
fn scope_to_wire(scope: &Scope) -> (i32, Vec<String>) {
    match scope {
        Scope::AllMembersQuote => (AggregationScopeMode::AllMembersQuote as i32, Vec::new()),
        Scope::Explicit(ids) => (AggregationScopeMode::Explicit as i32, ids.clone()),
    }
}

/// Map a stored [`AggregatedBookDef`] onto its wire [`AggregatedBookDesc`].
fn aggregated_book_to_wire(def: &AggregatedBookDef) -> AggregatedBookDesc {
    let (scope_mode, instrument_ids) = scope_to_wire(&def.instrument_scope);
    AggregatedBookDesc {
        id: def.id.clone(),
        name: def.name.clone(),
        member_connection_ids: def.member_connection_ids.clone(),
        scope_mode,
        instrument_ids,
        params: Some(params_to_wire(&def.params)),
        enabled: def.enabled,
    }
}

/// Map a stored [`celnet_tiering::TieringConfig`] onto its wire [`TieringConfigDesc`]
/// (Phase 2a). Every strategy variant becomes a `kind`-tagged [`TieringStrategyDesc`];
/// magnitudes a variant does not carry are left at proto3 zero and ignored on decode.
fn tiering_to_wire(cfg: &celnet_tiering::TieringConfig) -> TieringConfigDesc {
    use celnet_tiering::{SpreadUnit, StalePolicy, StrategySpec};
    let unit = match cfg.unit {
        SpreadUnit::PriceBps => TieringSpreadUnit::PriceBps,
        SpreadUnit::YieldBps => TieringSpreadUnit::YieldBps,
        SpreadUnit::PricePoints => TieringSpreadUnit::PricePoints,
        SpreadUnit::Percent => TieringSpreadUnit::Percent,
    } as i32;
    let stale_policy = match cfg.stale_policy {
        StalePolicy::Suppress => TieringStalePolicy::Suppress,
        StalePolicy::WidenToMax => TieringStalePolicy::WidenToMax,
    } as i32;
    let strategies = cfg
        .strategies
        .iter()
        .map(|s| match *s {
            StrategySpec::FlatMarkup { half_spread } => TieringStrategyDesc {
                kind: TieringStrategyKind::FlatMarkup as i32,
                half_spread,
                kappa: 0.0,
                s_max: 0.0,
                ..Default::default()
            },
            StrategySpec::InventorySkew {
                half_spread,
                kappa,
                s_max,
            } => TieringStrategyDesc {
                kind: TieringStrategyKind::InventorySkew as i32,
                half_spread,
                kappa,
                s_max,
                ..Default::default()
            },
            StrategySpec::ScaledSmoothedSpread {
                smoothing_weight,
                expected_spread,
                max_divergence,
                core_spread,
                max_output_spread,
                spread_scale_factor,
            } => TieringStrategyDesc {
                kind: TieringStrategyKind::ScaledSmoothedSpread as i32,
                smoothing_weight,
                expected_spread,
                max_divergence,
                core_spread,
                max_output_spread,
                spread_scale_factor,
                ..Default::default()
            },
        })
        .collect();
    TieringConfigDesc {
        unit,
        strategies,
        guardrails: Some(TieringGuardrailsDesc {
            h_min: cfg.guardrails.h_min,
            h_max: cfg.guardrails.h_max,
            s_max: cfg.guardrails.s_max,
            spread_floor: cfg.guardrails.spread_floor,
        }),
        stale_policy,
    }
}

/// Reconstruct a stored [`celnet_tiering::TieringConfig`] from its wire form. `None`
/// (an absent `tiering` message) ⇒ tiering disabled. An unknown enum value defaults to
/// the proto3 zero variant; an absent `guardrails` message defaults to all-zero bounds,
/// which the store's `validate_tiering_config` then rejects (a bad config fails loudly
/// at the admin write rather than silently suppressing at runtime).
fn tiering_from_wire(tiering: Option<TieringConfigDesc>) -> Option<celnet_tiering::TieringConfig> {
    use celnet_tiering::{Guardrails, SpreadUnit, StalePolicy, StrategySpec, TieringConfig};
    let d = tiering?;
    let unit = match TieringSpreadUnit::try_from(d.unit).unwrap_or_default() {
        TieringSpreadUnit::PriceBps => SpreadUnit::PriceBps,
        TieringSpreadUnit::YieldBps => SpreadUnit::YieldBps,
        TieringSpreadUnit::PricePoints => SpreadUnit::PricePoints,
        TieringSpreadUnit::Percent => SpreadUnit::Percent,
    };
    let stale_policy = match TieringStalePolicy::try_from(d.stale_policy).unwrap_or_default() {
        TieringStalePolicy::Suppress => StalePolicy::Suppress,
        TieringStalePolicy::WidenToMax => StalePolicy::WidenToMax,
    };
    let strategies = d
        .strategies
        .into_iter()
        .map(
            |s| match TieringStrategyKind::try_from(s.kind).unwrap_or_default() {
                TieringStrategyKind::FlatMarkup => StrategySpec::FlatMarkup {
                    half_spread: s.half_spread,
                },
                TieringStrategyKind::InventorySkew => StrategySpec::InventorySkew {
                    half_spread: s.half_spread,
                    kappa: s.kappa,
                    s_max: s.s_max,
                },
                TieringStrategyKind::ScaledSmoothedSpread => StrategySpec::ScaledSmoothedSpread {
                    smoothing_weight: s.smoothing_weight,
                    expected_spread: s.expected_spread,
                    max_divergence: s.max_divergence,
                    core_spread: s.core_spread,
                    max_output_spread: s.max_output_spread,
                    spread_scale_factor: s.spread_scale_factor,
                },
            },
        )
        .collect();
    let g = d.guardrails.unwrap_or(TieringGuardrailsDesc {
        h_min: 0.0,
        h_max: 0.0,
        s_max: 0.0,
        spread_floor: 0.0,
    });
    Some(TieringConfig {
        unit,
        strategies,
        guardrails: Guardrails::new(g.h_min, g.h_max, g.s_max, g.spread_floor),
        stale_policy,
    })
}

/// Reconstruct the store [`Scope`] from a wire `(scope_mode, instrument_ids)` pair.
/// An `AllMembersQuote` book drops any stray ids (they are meaningless in that mode);
/// an unknown enum value defaults to `AllMembersQuote` (the proto3 zero default).
fn scope_from_wire(scope_mode: i32, instrument_ids: Vec<String>) -> Scope {
    match AggregationScopeMode::try_from(scope_mode).unwrap_or_default() {
        AggregationScopeMode::Explicit => Scope::Explicit(instrument_ids),
        AggregationScopeMode::AllMembersQuote => Scope::AllMembersQuote,
    }
}

/// Reconstruct the store [`AggregationParams`] from its wire form, defaulting an
/// absent params message to the store's sane defaults (so a minimal spec stands a book
/// up rather than failing the required-field parse; the store still validates ranges).
fn params_from_wire(params: Option<AggregationParamsDesc>) -> AggregationParams {
    params.map_or_else(AggregationParams::default, |p| AggregationParams {
        staleness_tau_ms: p.staleness_tau_ms,
        max_quote_age_ms: p.max_quote_age_ms,
        divergence_gating: p.divergence_gating,
        min_contributors: p.min_contributors,
        depth_levels: p.depth_levels,
    })
}

/// Map an [`AggregatedBookSpec`] onto the [`AggregatedBookEdit`] the store's
/// `create_aggregated_book` / `update_aggregated_book` take. The spec's own `id` is
/// intentionally dropped — create mints a fresh id and update keeps the request `id`.
fn spec_parts(spec: AggregatedBookSpec) -> AggregatedBookEdit {
    let scope = scope_from_wire(spec.scope_mode, spec.instrument_ids);
    let params = params_from_wire(spec.params);
    AggregatedBookEdit::new(
        spec.name,
        spec.member_connection_ids,
        scope,
        params,
        spec.enabled,
    )
}

/// Map the store's aggregated-book validation error string onto a gRPC [`Status`]: a
/// name collision is `already_exists`, a missing id on update is `not_found`, and every
/// other document-resolvable failure (duplicate member, out-of-range param, dangling
/// instrument id) is `invalid_argument`. The store's message is surfaced verbatim.
fn aggregated_book_status(msg: String) -> Status {
    if msg.contains("already exists") {
        Status::already_exists(msg)
    } else if msg.starts_with("no aggregated book with id") {
        Status::not_found(msg)
    } else {
        Status::invalid_argument(msg)
    }
}

// --- pricing-group wire conversions ----------------------------------------------

/// The wire enum value for a domain [`celnet_tiering::SpreadUnit`] (shared by the
/// pricing-feature converters). Mirrors the mapping inside [`tiering_to_wire`].
fn spread_unit_to_wire(u: celnet_tiering::SpreadUnit) -> i32 {
    use celnet_tiering::SpreadUnit;
    (match u {
        SpreadUnit::PriceBps => TieringSpreadUnit::PriceBps,
        SpreadUnit::YieldBps => TieringSpreadUnit::YieldBps,
        SpreadUnit::PricePoints => TieringSpreadUnit::PricePoints,
        SpreadUnit::Percent => TieringSpreadUnit::Percent,
    }) as i32
}

/// Resolve a wire [`TieringSpreadUnit`] value to a domain [`celnet_tiering::SpreadUnit`]
/// (an unknown value defaults to the proto3 zero variant, `PriceBps`).
fn spread_unit_from_wire(u: i32) -> celnet_tiering::SpreadUnit {
    use celnet_tiering::SpreadUnit;
    match TieringSpreadUnit::try_from(u).unwrap_or_default() {
        TieringSpreadUnit::PriceBps => SpreadUnit::PriceBps,
        TieringSpreadUnit::YieldBps => SpreadUnit::YieldBps,
        TieringSpreadUnit::PricePoints => SpreadUnit::PricePoints,
        TieringSpreadUnit::Percent => SpreadUnit::Percent,
    }
}

/// The wire enum value for a domain [`celnet_tiering::AxeSide`].
fn axe_side_to_wire(s: celnet_tiering::AxeSide) -> i32 {
    (match s {
        celnet_tiering::AxeSide::Buy => AxeSide::Buy,
        celnet_tiering::AxeSide::Sell => AxeSide::Sell,
    }) as i32
}

/// Resolve a wire [`AxeSide`] value to a domain [`celnet_tiering::AxeSide`] (an unknown
/// value defaults to the proto3 zero variant, `Buy`).
fn axe_side_from_wire(s: i32) -> celnet_tiering::AxeSide {
    match AxeSide::try_from(s).unwrap_or_default() {
        AxeSide::Buy => celnet_tiering::AxeSide::Buy,
        AxeSide::Sell => celnet_tiering::AxeSide::Sell,
    }
}

/// Resolve a wire [`EspOrRfq`] selector to a domain [`PricingMode`] (an unknown value
/// defaults to the proto3 zero variant, ESP).
fn pricing_mode_from_wire(m: i32) -> PricingMode {
    match EspOrRfq::try_from(m).unwrap_or_default() {
        EspOrRfq::Esp => PricingMode::Esp,
        EspOrRfq::Rfq => PricingMode::Rfq,
    }
}

/// Map a stored [`celnet_tiering::PricingFeature`] onto its flat, `kind`-discriminated
/// wire [`FeatureSpecDesc`]. Fields a variant does not carry are left at proto3 zero and
/// ignored on decode (identical convention to [`tiering_to_wire`]'s strategies).
fn feature_spec_to_wire(f: &celnet_tiering::PricingFeature) -> FeatureSpecDesc {
    use celnet_tiering::PricingFeature;
    match f {
        PricingFeature::MidShift {
            shift,
            unit,
            reference,
        } => FeatureSpecDesc {
            kind: FeatureKind::MidShift as i32,
            unit: spread_unit_to_wire(*unit),
            shift: *shift,
            reference: *reference,
            ..Default::default()
        },
        PricingFeature::Tiering { config } => FeatureSpecDesc {
            kind: FeatureKind::Tiering as i32,
            tiering: Some(tiering_to_wire(config)),
            ..Default::default()
        },
        PricingFeature::Axe {
            side,
            magnitude,
            unit,
        } => FeatureSpecDesc {
            kind: FeatureKind::Axe as i32,
            unit: spread_unit_to_wire(*unit),
            axe_side: axe_side_to_wire(*side),
            magnitude: *magnitude,
            ..Default::default()
        },
        PricingFeature::Position { kappa, s_max, unit } => FeatureSpecDesc {
            kind: FeatureKind::Position as i32,
            unit: spread_unit_to_wire(*unit),
            kappa: *kappa,
            s_max: *s_max,
            ..Default::default()
        },
        PricingFeature::PanicSkew {
            skew,
            unit,
            triggered,
        } => FeatureSpecDesc {
            kind: FeatureKind::PanicSkew as i32,
            unit: spread_unit_to_wire(*unit),
            skew: *skew,
            triggered: *triggered,
            ..Default::default()
        },
    }
}

/// Reconstruct a stored [`celnet_tiering::PricingFeature`] from its wire form. A
/// TIERING feature whose `tiering` message is absent decodes to an empty config, which
/// the store's `validate_feature_pipeline` then rejects (a bad feature fails loudly at
/// the admin write rather than silently no-op'ing at runtime).
fn feature_spec_from_wire(s: FeatureSpecDesc) -> celnet_tiering::PricingFeature {
    use celnet_tiering::PricingFeature;
    let unit = spread_unit_from_wire(s.unit);
    match FeatureKind::try_from(s.kind).unwrap_or_default() {
        FeatureKind::MidShift => PricingFeature::MidShift {
            shift: s.shift,
            unit,
            reference: s.reference,
        },
        FeatureKind::Tiering => PricingFeature::Tiering {
            // `tiering_from_wire(Some(_))` is always `Some`; an absent message yields the
            // empty (all-zero) config the pipeline validation then rejects.
            config: tiering_from_wire(Some(s.tiering.unwrap_or_default()))
                .expect("tiering_from_wire(Some(_)) always yields Some"),
        },
        FeatureKind::Axe => PricingFeature::Axe {
            side: axe_side_from_wire(s.axe_side),
            magnitude: s.magnitude,
            unit,
        },
        FeatureKind::Position => PricingFeature::Position {
            kappa: s.kappa,
            s_max: s.s_max,
            unit,
        },
        FeatureKind::PanicSkew => PricingFeature::PanicSkew {
            skew: s.skew,
            unit,
            triggered: s.triggered,
        },
    }
}

/// Map a stored [`celnet_tiering::FeaturePipeline`] onto its wire [`FeaturePipelineDesc`]
/// (the ordered features + the closing guardrails, reusing [`TieringGuardrailsDesc`]).
fn feature_pipeline_to_wire(p: &celnet_tiering::FeaturePipeline) -> FeaturePipelineDesc {
    FeaturePipelineDesc {
        features: p.features.iter().map(feature_spec_to_wire).collect(),
        guardrails: Some(TieringGuardrailsDesc {
            h_min: p.guardrails.h_min,
            h_max: p.guardrails.h_max,
            s_max: p.guardrails.s_max,
            spread_floor: p.guardrails.spread_floor,
        }),
    }
}

/// Reconstruct a stored [`celnet_tiering::FeaturePipeline`] from its wire form. An absent
/// pipeline (or absent `guardrails`) defaults to all-zero guardrails, which the store's
/// `validate_feature_pipeline` then rejects.
fn feature_pipeline_from_wire(p: Option<FeaturePipelineDesc>) -> celnet_tiering::FeaturePipeline {
    use celnet_tiering::{FeaturePipeline, Guardrails};
    let d = p.unwrap_or_default();
    let features = d.features.into_iter().map(feature_spec_from_wire).collect();
    let g = d.guardrails.unwrap_or_default();
    FeaturePipeline::new(
        features,
        Guardrails::new(g.h_min, g.h_max, g.s_max, g.spread_floor),
    )
}

/// Map a stored [`PricingGroupDef`] onto its wire [`PricingGroupDesc`] (field for field).
fn pricing_group_to_wire(def: &PricingGroupDef) -> PricingGroupDesc {
    PricingGroupDesc {
        id: def.id.clone(),
        name: def.name.clone(),
        description: def.description.clone(),
        member_connection_ids: def.member_connection_ids.clone(),
        member_user_ids: def.member_user_ids.clone(),
        member_desks: def.member_desks.clone(),
        esp_pipeline: Some(feature_pipeline_to_wire(&def.esp_pipeline)),
        rfq_pipeline: Some(feature_pipeline_to_wire(&def.rfq_pipeline)),
        share_pipeline: def.share_pipeline,
        enabled: def.enabled,
    }
}

/// Map a [`PricingGroupSpec`] onto the [`PricingGroupEdit`] the store's
/// `create_pricing_group` / `update_pricing_group` take. The spec's own `id` is
/// intentionally dropped — create mints a fresh id and update keeps the request `id`.
fn pricing_group_spec_parts(spec: PricingGroupSpec) -> PricingGroupEdit {
    PricingGroupEdit {
        name: spec.name,
        description: spec.description,
        member_connection_ids: spec.member_connection_ids,
        member_user_ids: spec.member_user_ids,
        member_desks: spec.member_desks,
        esp_pipeline: feature_pipeline_from_wire(spec.esp_pipeline),
        rfq_pipeline: feature_pipeline_from_wire(spec.rfq_pipeline),
        share_pipeline: spec.share_pipeline,
        enabled: spec.enabled,
    }
}

/// Map the store's pricing-group validation error string onto a gRPC [`Status`]: a name
/// collision is `already_exists`, a missing id on update is `not_found`, and every other
/// document-resolvable failure (duplicate/unknown member, invalid pipeline, cross-group
/// determinism conflict) is `invalid_argument`. The store's message is surfaced verbatim.
fn pricing_group_status(msg: String) -> Status {
    if msg.contains("duplicate pricing group name") {
        Status::already_exists(msg)
    } else if msg.starts_with("no pricing group with id") {
        Status::not_found(msg)
    } else {
        Status::invalid_argument(msg)
    }
}

// --- risk routing & risk books: wire converters + error mapping --------------

/// Map a stored [`RiskLimits`] onto its wire [`RiskLimitsDesc`] (each cap is a
/// presence-tracked `optional double`).
fn risk_limits_to_wire(l: &RiskLimits) -> RiskLimitsDesc {
    RiskLimitsDesc {
        max_net_notional: l.max_net_notional,
        max_gross_notional: l.max_gross_notional,
        max_dv01: l.max_dv01,
    }
}

/// Map a wire [`RiskLimitsDesc`] back onto the stored [`RiskLimits`].
fn risk_limits_from_wire(d: RiskLimitsDesc) -> RiskLimits {
    RiskLimits {
        max_net_notional: d.max_net_notional,
        max_gross_notional: d.max_gross_notional,
        max_dv01: d.max_dv01,
    }
}

/// Map a stored [`RiskBookDef`] onto its wire [`RiskBookDesc`] field-for-field.
fn risk_book_to_wire(def: &RiskBookDef) -> RiskBookDesc {
    RiskBookDesc {
        id: def.id.clone(),
        name: def.name.clone(),
        parent_id: def.parent_id.clone(),
        desk_id: def.desk_id.clone(),
        description: def.description.clone(),
        limits: def.limits.as_ref().map(risk_limits_to_wire),
        enabled: def.enabled,
    }
}

/// Map a [`RiskBookSpec`] onto the [`RiskBookEdit`] the store's `create_risk_book` /
/// `update_risk_book` take. The spec's own `id` is intentionally dropped — create mints a
/// fresh id and update keeps the request `id` (mirrors [`pricing_group_spec_parts`]). An
/// empty optional string decodes to `None` (no parent / unowned desk).
fn risk_book_spec_parts(spec: RiskBookSpec) -> RiskBookEdit {
    RiskBookEdit {
        name: spec.name,
        parent_id: spec.parent_id.filter(|s| !s.is_empty()),
        desk_id: spec.desk_id.filter(|s| !s.is_empty()),
        description: spec.description,
        limits: spec.limits.map(risk_limits_from_wire),
        enabled: spec.enabled,
    }
}

/// The wire enum value for a domain [`RouteField`] (exhaustive — no lossy default).
fn route_field_to_wire(field: RouteField) -> i32 {
    let e = match field {
        RouteField::InstrumentId => RouteFieldEnum::RouteFieldInstrumentId,
        RouteField::Ccy => RouteFieldEnum::RouteFieldCcy,
        RouteField::Product => RouteFieldEnum::RouteFieldProduct,
        RouteField::Side => RouteFieldEnum::RouteFieldSide,
        RouteField::Notional => RouteFieldEnum::RouteFieldNotional,
        RouteField::Tenor => RouteFieldEnum::RouteFieldTenor,
        RouteField::Strike => RouteFieldEnum::RouteFieldStrike,
        RouteField::Counterparty => RouteFieldEnum::RouteFieldCounterparty,
        RouteField::User => RouteFieldEnum::RouteFieldUser,
        RouteField::Desk => RouteFieldEnum::RouteFieldDesk,
        RouteField::Price => RouteFieldEnum::RouteFieldPrice,
    };
    e as i32
}

/// Resolve a wire enum value onto a domain [`RouteField`].
///
/// # Errors
/// `invalid_argument` for an unrecognised enum value.
fn route_field_from_wire(v: i32) -> Result<RouteField, Status> {
    match RouteFieldEnum::try_from(v) {
        Ok(RouteFieldEnum::RouteFieldInstrumentId) => Ok(RouteField::InstrumentId),
        Ok(RouteFieldEnum::RouteFieldCcy) => Ok(RouteField::Ccy),
        Ok(RouteFieldEnum::RouteFieldProduct) => Ok(RouteField::Product),
        Ok(RouteFieldEnum::RouteFieldSide) => Ok(RouteField::Side),
        Ok(RouteFieldEnum::RouteFieldNotional) => Ok(RouteField::Notional),
        Ok(RouteFieldEnum::RouteFieldTenor) => Ok(RouteField::Tenor),
        Ok(RouteFieldEnum::RouteFieldStrike) => Ok(RouteField::Strike),
        Ok(RouteFieldEnum::RouteFieldCounterparty) => Ok(RouteField::Counterparty),
        Ok(RouteFieldEnum::RouteFieldUser) => Ok(RouteField::User),
        Ok(RouteFieldEnum::RouteFieldDesk) => Ok(RouteField::Desk),
        Ok(RouteFieldEnum::RouteFieldPrice) => Ok(RouteField::Price),
        Err(_) => Err(Status::invalid_argument(format!(
            "unrecognised route field enum value {v}"
        ))),
    }
}

/// The wire enum value for a domain [`RouteOp`] (exhaustive — no lossy default).
fn route_op_to_wire(op: RouteOp) -> i32 {
    let e = match op {
        RouteOp::Eq => RouteOpEnum::RouteOpEq,
        RouteOp::Ne => RouteOpEnum::RouteOpNe,
        RouteOp::Gt => RouteOpEnum::RouteOpGt,
        RouteOp::Ge => RouteOpEnum::RouteOpGe,
        RouteOp::Lt => RouteOpEnum::RouteOpLt,
        RouteOp::Le => RouteOpEnum::RouteOpLe,
        RouteOp::Contains => RouteOpEnum::RouteOpContains,
        RouteOp::In => RouteOpEnum::RouteOpIn,
        RouteOp::Between => RouteOpEnum::RouteOpBetween,
    };
    e as i32
}

/// Resolve a wire enum value onto a domain [`RouteOp`].
///
/// # Errors
/// `invalid_argument` for an unrecognised enum value.
fn route_op_from_wire(v: i32) -> Result<RouteOp, Status> {
    match RouteOpEnum::try_from(v) {
        Ok(RouteOpEnum::RouteOpEq) => Ok(RouteOp::Eq),
        Ok(RouteOpEnum::RouteOpNe) => Ok(RouteOp::Ne),
        Ok(RouteOpEnum::RouteOpGt) => Ok(RouteOp::Gt),
        Ok(RouteOpEnum::RouteOpGe) => Ok(RouteOp::Ge),
        Ok(RouteOpEnum::RouteOpLt) => Ok(RouteOp::Lt),
        Ok(RouteOpEnum::RouteOpLe) => Ok(RouteOp::Le),
        Ok(RouteOpEnum::RouteOpContains) => Ok(RouteOp::Contains),
        Ok(RouteOpEnum::RouteOpIn) => Ok(RouteOp::In),
        Ok(RouteOpEnum::RouteOpBetween) => Ok(RouteOp::Between),
        Err(_) => Err(Status::invalid_argument(format!(
            "unrecognised route op enum value {v}"
        ))),
    }
}

/// Map a stored [`RouteValue`] onto its wire [`RouteValueDesc`] oneof.
fn route_value_to_wire(value: &RouteValue) -> RouteValueDesc {
    let v = match value {
        RouteValue::Num(x) => route_value_desc::V::Num(*x),
        RouteValue::Text(s) => route_value_desc::V::Text(s.clone()),
        RouteValue::List(items) => route_value_desc::V::List(StringList {
            values: items.clone(),
        }),
        RouteValue::Range { lo, hi } => route_value_desc::V::Range(RouteRange { lo: *lo, hi: *hi }),
    };
    RouteValueDesc { v: Some(v) }
}

/// Map a wire [`RouteValueDesc`] back onto a stored [`RouteValue`].
///
/// # Errors
/// `invalid_argument` when the oneof carries no live arm.
fn route_value_from_wire(d: RouteValueDesc) -> Result<RouteValue, Status> {
    match d.v {
        Some(route_value_desc::V::Num(x)) => Ok(RouteValue::Num(x)),
        Some(route_value_desc::V::Text(s)) => Ok(RouteValue::Text(s)),
        Some(route_value_desc::V::List(l)) => Ok(RouteValue::List(l.values)),
        Some(route_value_desc::V::Range(r)) => Ok(RouteValue::Range { lo: r.lo, hi: r.hi }),
        None => Err(Status::invalid_argument(
            "route condition value carries no arm (num / text / list / range)",
        )),
    }
}

/// Map a stored [`RoutingNode`] (+ its id) onto its wire [`RoutingNodeDesc`].
fn routing_node_to_wire(id: NodeId, node: &RoutingNode) -> RoutingNodeDesc {
    let node = match node {
        RoutingNode::Condition {
            field,
            op,
            value,
            on_true,
            on_false,
        } => routing_node_desc::Node::Condition(RouteConditionDesc {
            field: route_field_to_wire(*field),
            op: route_op_to_wire(*op),
            value: Some(route_value_to_wire(value)),
            on_true: *on_true,
            on_false: *on_false,
        }),
        RoutingNode::Book { risk_book_id } => {
            routing_node_desc::Node::BookRiskBookId(risk_book_id.clone())
        }
    };
    RoutingNodeDesc {
        id,
        node: Some(node),
    }
}

/// Map a wire [`RoutingNodeDesc`] back onto `(id, RoutingNode)`.
///
/// # Errors
/// `invalid_argument` when the `node` oneof carries no arm, a condition's `value`
/// message is absent, or an enum value is unrecognised.
fn routing_node_from_wire(d: RoutingNodeDesc) -> Result<(NodeId, RoutingNode), Status> {
    let node = match d.node {
        Some(routing_node_desc::Node::Condition(c)) => {
            let value = c
                .value
                .ok_or_else(|| Status::invalid_argument("route condition value is required"))?;
            RoutingNode::Condition {
                field: route_field_from_wire(c.field)?,
                op: route_op_from_wire(c.op)?,
                value: route_value_from_wire(value)?,
                on_true: c.on_true,
                on_false: c.on_false,
            }
        }
        Some(routing_node_desc::Node::BookRiskBookId(risk_book_id)) => {
            RoutingNode::Book { risk_book_id }
        }
        None => {
            return Err(Status::invalid_argument(format!(
                "routing node {} carries no body (condition / book)",
                d.id
            )));
        }
    };
    Ok((d.id, node))
}

/// Map a stored [`RiskRoutingGraph`] onto its wire [`RiskRoutingGraphDesc`]. The store
/// keys nodes by a `BTreeMap`, so the emitted node list is deterministically id-ordered.
fn routing_graph_to_wire(graph: &RiskRoutingGraph) -> RiskRoutingGraphDesc {
    RiskRoutingGraphDesc {
        entry: graph.entry,
        nodes: graph
            .nodes
            .iter()
            .map(|(id, node)| routing_node_to_wire(*id, node))
            .collect(),
    }
}

/// Map a wire [`RiskRoutingGraphDesc`] back onto a stored [`RiskRoutingGraph`], rebuilding
/// the id→node map. Structural well-formedness (acyclic, known books, type-consistent) is
/// left to the store's `set_risk_routing_graph` validation on write.
///
/// # Errors
/// `invalid_argument` when two nodes share an id, or any node fails to decode.
fn routing_graph_from_wire(d: RiskRoutingGraphDesc) -> Result<RiskRoutingGraph, Status> {
    let mut nodes = std::collections::BTreeMap::new();
    for wire_node in d.nodes {
        let (id, node) = routing_node_from_wire(wire_node)?;
        if nodes.insert(id, node).is_some() {
            return Err(Status::invalid_argument(format!(
                "duplicate routing node id {id}"
            )));
        }
    }
    Ok(RiskRoutingGraph {
        entry: d.entry,
        nodes,
    })
}

/// Map the store's risk-book validation error string onto a gRPC [`Status`]: a name
/// collision is `already_exists`, a missing id on update is `not_found`, and every other
/// document-resolvable failure (unknown/cyclic parent, unknown desk, negative limit, an
/// orphaning delete, a graph-targeted delete/disable) is `invalid_argument`. The store's
/// message is surfaced verbatim.
fn risk_book_status(msg: String) -> Status {
    if msg.contains("duplicate risk book name") {
        Status::already_exists(msg)
    } else if msg.starts_with("no risk book with id") {
        Status::not_found(msg)
    } else {
        Status::invalid_argument(msg)
    }
}

/// Map the store's routing-graph well-formedness error onto a gRPC [`Status`]. Every graph
/// defect (missing entry, dangling edge, cycle, unknown/disabled target book, type
/// mismatch) is a client-correctable `invalid_argument`; the store's message is verbatim.
fn risk_routing_status(msg: String) -> Status {
    Status::invalid_argument(msg)
}

/// The wire enum value for a domain [`DomainRagBand`].
fn rag_band_to_wire(band: DomainRagBand) -> i32 {
    let e = match band {
        DomainRagBand::Green => RagBand::Green,
        DomainRagBand::Amber => RagBand::Amber,
        DomainRagBand::Red => RagBand::Red,
    };
    e as i32
}

/// Map a domain [`LimitUtilization`] onto its wire [`LimitUtilizationDesc`] field-for-field.
pub(crate) fn limit_util_to_wire(u: &LimitUtilization) -> LimitUtilizationDesc {
    LimitUtilizationDesc {
        metric: u.metric.to_owned(),
        used: u.used,
        limit: u.limit,
        fraction: u.fraction,
        band: rag_band_to_wire(u.band),
    }
}

/// Map an aggregated [`RiskBookRisk`] onto its wire [`RiskBookRiskDesc`]. The `dv01`/`pnl`
/// options ride straight through — absent means not-yet-evaluated (rates DV01 / a mark
/// pass are later seams, §5.3/§5.4), never a fabricated zero.
pub(crate) fn risk_book_risk_to_wire(r: &RiskBookRisk) -> RiskBookRiskDesc {
    RiskBookRiskDesc {
        book_id: r.book_id.clone(),
        name: r.name.clone(),
        net_notional: r.net_notional,
        gross_notional: r.gross_notional,
        position_count: r.position_count,
        delta: r.delta,
        gamma: r.gamma,
        vega: r.vega,
        theta: r.theta,
        dv01: r.dv01,
        pnl: r.pnl,
        limits: r.limits.iter().map(limit_util_to_wire).collect(),
    }
}

/// A stable, human-facing label for a [`Role`] — used as the `role` field in the
/// security logs (never a bare enum discriminant).
fn role_label(role: Role) -> &'static str {
    match role {
        Role::Admin => "admin",
        Role::Trader => "trader",
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

/// Normalize a wire desk membership (`desk_ids` + `all_desks`) into a canonical
/// `(all_desks, desk_ids)` pair for the store.
///
/// `all_desks` **wins**: when set, the explicit set is dropped (an all-desks user
/// carries no `desk_ids` — the canonical form). Otherwise the ids are trimmed,
/// blanks dropped, and duplicates removed while preserving first-seen order, so the
/// stored set round-trips stably and set-equality comparisons (authority-change
/// detection) are meaningful.
fn normalize_membership(desk_ids: Vec<String>, all_desks: bool) -> (bool, Vec<String>) {
    if all_desks {
        return (true, Vec::new());
    }
    let mut out: Vec<String> = Vec::new();
    for d in desk_ids {
        let trimmed = d.trim().to_string();
        if !trimmed.is_empty() && !out.contains(&trimmed) {
            out.push(trimmed);
        }
    }
    (false, out)
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
        // Membership normalization: trim, drop blanks, dedup, preserve order.
        assert_eq!(
            normalize_membership(
                vec![" g10 ".into(), "".into(), "g10".into(), "em".into()],
                false
            ),
            (false, vec!["g10".to_owned(), "em".to_owned()])
        );
        assert_eq!(normalize_membership(vec![], false), (false, vec![]));
        // all_desks wins: the explicit set is dropped to the canonical empty form.
        assert_eq!(
            normalize_membership(vec!["g10".into()], true),
            (true, vec![])
        );
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
            desk_ids: Vec::new(),
            all_desks: false,
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
            desk_ids: Vec::new(),
            all_desks: false,
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
        let path = dir.join(format!(
            "identity-curve-{}-{}.json",
            std::process::id(),
            tag
        ));
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
                date_pillars: Vec::new(),
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

    /// A standalone date-anchored pillar calibrates alongside a registry instrument:
    /// the response carries an empty-id, `Date YYYY-MM-DD`-labelled point, and the
    /// synthetic deposit reprices to its quote on the wire-reconstructed curve.
    #[tokio::test]
    async fn build_curve_calibrates_a_standalone_date_pillar() {
        let ois = (cv_ois("usd-ois-2y", "2Y"), 0.0418);
        let (edge, path) = curve_edge("date-pillar", vec![ois.0.clone()]);
        let token = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;

        // A 2027-12-31 date pillar (a broken date no whole tenor names) at 4.15%.
        let maturity = BrokenDate {
            year: 2027,
            month: 12,
            day: 31,
        };
        let date_quote = 0.0415;

        let resp = edge
            .build_curve(Request::new(BuildCurveRequest {
                request_id: "curve-date-1".to_string(),
                currency: "USD".to_string(),
                reference_date: Some(curve_reference()),
                pillars: vec![InstrumentQuote {
                    instrument_id: ois.0.instrument_id.clone(),
                    quote: ois.1,
                }],
                session_token: token,
                date_pillars: vec![celnet_proto::DatePillar {
                    maturity_date: Some(maturity),
                    quote: date_quote,
                }],
            }))
            .await
            .expect("build_curve should succeed")
            .into_inner();

        // Two pillars, ordered by maturity (the date pillar at ~1.5y precedes the 2Y OIS).
        assert_eq!(resp.points.len(), 2);
        for w in resp.points.windows(2) {
            assert!(w[0].time_years < w[1].time_years);
        }

        // The date pillar carries an empty instrument id and its `Date …` display label.
        let date_point = resp
            .points
            .iter()
            .find(|p| p.instrument_id.is_empty())
            .expect("a date-anchored point is present");
        assert_eq!(date_point.label, "Date 2027-12-31");

        // Reconstruct the curve from the wire points and reprice the synthetic deposit
        // to its quote (1e-8) — the same oracle gate the registry ladder uses.
        let mut td: Vec<(Time, Df)> = vec![(Time(0.0), Df(1.0))];
        td.extend(
            resp.points
                .iter()
                .map(|p| (Time(p.time_years), Df(p.discount_factor))),
        );
        let curve = Curve::from_log_linear_dfs(&td).expect("curve reconstructs from wire points");

        let value_date = curve_value_date();
        let date_maturity =
            time::Date::from_calendar_date(2027, time::Month::December, 31).unwrap();
        let deposit = crate::config::curve_calibration::date_pillar_instrument(
            value_date,
            date_maturity,
            date_quote,
        )
        .expect("date pillar resolves to a calibration instrument");
        let model = model_quote(&curve, &deposit);
        assert!(
            (model - date_quote).abs() < 1e-8,
            "date pillar repriced {model} vs quote {date_quote}"
        );

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
                date_pillars: Vec::new(),
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
                date_pillars: Vec::new(),
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
                date_pillars: Vec::new(),
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
                date_pillars: Vec::new(),
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
                desk_ids: Vec::new(),
                all_desks: false,
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
                desk_ids: Vec::new(),
                all_desks: false,
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
        // The seed admin resolves to grant-all: every action × asset (14 × 2 = 28).
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        assert_eq!(
            admin.capabilities.len(),
            Action::ALL.len() * AssetClass::ALL.len()
        );

        // A fresh trader holds the role bundle: all actions but the FIVE narrow,
        // explicitly-granted authorities held back from the default —
        // `administer`, `risk_transfer`, `risk_manage`, `manage_pricing`,
        // `manage_liquidity` — on both assets (9 × 2 = 18).
        const HELD_BACK: usize = 5;
        let (trader_id, _t) = make_trader(&edge, &admin.session_token, "lc@celnet.com").await;
        let trader = login(&edge, "lc@celnet.com", "trader-pw-123")
            .await
            .unwrap();
        assert_eq!(
            trader.capabilities.len(),
            (Action::ALL.len() - HELD_BACK) * AssetClass::ALL.len()
        );
        assert!(has_cap(&trader.capabilities, "execute", "fixed_income"));
        assert!(!has_cap(&trader.capabilities, "administer", "fx_options"));
        assert!(
            !has_cap(&trader.capabilities, "risk_transfer", "fixed_income"),
            "cross-desk transfer is a narrow authority, not in the default bundle"
        );
        for held in ["risk_manage", "manage_pricing", "manage_liquidity"] {
            for asset in ["fx_options", "fixed_income"] {
                assert!(
                    !has_cap(&trader.capabilities, held, asset),
                    "{held}·{asset} is admin-granted, not in the default trader bundle"
                );
            }
        }

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
                desk_ids: Vec::new(),
                all_desks: false,
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

    /// `update_user` sets a MULTI-desk membership, rejects an unknown desk id, and
    /// clearing the set (empty + `!all_desks`) makes the user deskless; `all_desks`
    /// canonicalizes away any provided set.
    #[tokio::test]
    async fn update_user_multi_desk_set_reject_and_clear() {
        let (edge, path, _s) = edge("multi-desk");
        let admin = login(&edge, "admin@celnet.com", "password").await.unwrap();
        let tok = admin.session_token.clone();
        let mk_desk = |name: &str| {
            let edge = &edge;
            let tok = tok.clone();
            let name = name.to_owned();
            async move {
                edge.create_desk(Request::new(CreateDeskRequest {
                    session_token: tok,
                    name,
                    correlation_id: None,
                }))
                .await
                .unwrap()
                .into_inner()
                .desk
                .unwrap()
            }
        };
        let a = mk_desk("Desk A").await;
        let b = mk_desk("Desk B").await;

        let jane = edge
            .create_user(Request::new(CreateUserRequest {
                session_token: tok.clone(),
                email: "jane@celnet.com".into(),
                display_name: "Jane".into(),
                role: UserRole::Trader as i32,
                desk_ids: vec![a.id.clone()],
                all_desks: false,
                password: "trader-pw-123".into(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .user
            .unwrap();

        let jid = jane.id.clone();
        let update = |desk_ids: Vec<String>, all_desks: bool| {
            let edge = &edge;
            let tok = tok.clone();
            let id = jid.clone();
            async move {
                edge.update_user(Request::new(UpdateUserRequest {
                    session_token: tok,
                    id,
                    display_name: "Jane".into(),
                    role: UserRole::Trader as i32,
                    desk_ids,
                    all_desks,
                    disabled: false,
                    correlation_id: None,
                }))
                .await
            }
        };

        // Set a multi-desk membership {A, B}.
        let set = update(vec![a.id.clone(), b.id.clone()], false)
            .await
            .unwrap()
            .into_inner()
            .user
            .unwrap();
        assert_eq!(set.desk_ids, vec![a.id.clone(), b.id.clone()]);
        assert!(!set.all_desks);

        // An unknown desk id is rejected (like the single-desk validation).
        let bad = update(vec![a.id.clone(), "no-such-desk".into()], false)
            .await
            .unwrap_err();
        assert_eq!(bad.code(), tonic::Code::FailedPrecondition);

        // all_desks canonicalizes away a provided set.
        let all = update(vec![a.id.clone()], true)
            .await
            .unwrap()
            .into_inner()
            .user
            .unwrap();
        assert!(all.all_desks && all.desk_ids.is_empty());

        // Clearing to empty (+ !all_desks) makes the user deskless.
        let cleared = update(Vec::new(), false)
            .await
            .unwrap()
            .into_inner()
            .user
            .unwrap();
        assert!(cleared.desk_ids.is_empty() && !cleared.all_desks);

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
                desk_ids: vec![desk.id.clone()],
                all_desks: false,
                password: "trader-pw-123".into(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .user
            .unwrap();
        assert_eq!(jane.desk_ids, vec![desk.id.clone()]);
        assert!(!jane.all_desks);
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
        assert!(
            jane_after.desk_ids.is_empty() && !jane_after.all_desks,
            "deleting a desk unassigns its members"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn update_desk_renames_display_name_and_preserves_routing() {
        let (edge, path, _s) = edge("desk-rename");
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
        // A trader whose membership keys on the stable desk id.
        let jane = edge
            .create_user(Request::new(CreateUserRequest {
                session_token: tok.clone(),
                email: "jane@celnet.com".into(),
                display_name: "Jane".into(),
                role: UserRole::Trader as i32,
                desk_ids: vec![desk.id.clone()],
                all_desks: false,
                password: "trader-pw-123".into(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .user
            .unwrap();

        // Rename the desk's display name.
        let renamed = edge
            .update_desk(Request::new(UpdateDeskRequest {
                session_token: tok.clone(),
                id: desk.id.clone(),
                name: "G10 Vol".into(),
                correlation_id: Some(7),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(renamed.correlation_id, Some(7));
        let renamed = renamed.desk.unwrap();
        assert_eq!(renamed.id, desk.id, "the stable id must never change");
        assert_eq!(renamed.name, "G10 Vol", "the display name is updated");

        // Routing membership is untouched: the trader still points at the same id.
        let users = edge
            .list_users(Request::new(ListUsersRequest {
                session_token: tok.clone(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .users;
        let jane_after = users.into_iter().find(|u| u.id == jane.id).unwrap();
        assert_eq!(
            jane_after.desk_ids,
            vec![desk.id.clone()],
            "renaming a desk must not disturb user→desk routing"
        );

        // A second desk cannot be renamed onto an existing name (case-insensitive).
        let other = edge
            .create_desk(Request::new(CreateDeskRequest {
                session_token: tok.clone(),
                name: "EM Vol".into(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .desk
            .unwrap();
        let clash = edge
            .update_desk(Request::new(UpdateDeskRequest {
                session_token: tok.clone(),
                id: other.id.clone(),
                name: "g10 vol".into(),
                correlation_id: None,
            }))
            .await
            .expect_err("a duplicate display name is rejected");
        assert_eq!(clash.code(), tonic::Code::AlreadyExists);

        // An unknown desk id is a clean not-found.
        let missing = edge
            .update_desk(Request::new(UpdateDeskRequest {
                session_token: tok,
                id: "no-such-desk".into(),
                name: "Whatever".into(),
                correlation_id: None,
            }))
            .await
            .expect_err("an unknown desk id is rejected");
        assert_eq!(missing.code(), tonic::Code::NotFound);

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

    // --- FI aggregated books (ADR-0022) ----------------------------------------

    /// A default `AllMembersQuote` aggregated-book spec (needs no seeded instruments).
    fn agg_spec(name: &str, members: Vec<String>) -> AggregatedBookSpec {
        AggregatedBookSpec {
            id: String::new(),
            name: name.into(),
            member_connection_ids: members,
            scope_mode: AggregationScopeMode::AllMembersQuote as i32,
            instrument_ids: Vec::new(),
            params: Some(AggregationParamsDesc {
                staleness_tau_ms: 500,
                max_quote_age_ms: 2500,
                divergence_gating: true,
                min_contributors: 1,
                depth_levels: 1,
            }),
            enabled: true,
        }
    }

    /// A valid tiering config body (flat + inventory-skew + scaled-smoothed, price bps)
    /// for a pricing-group TIERING feature. The Scaled-Smoothed-Spread entry exercises the
    /// six params through the full store round-trip and wire echo.
    fn agg_tiering_spec() -> TieringConfigDesc {
        TieringConfigDesc {
            unit: TieringSpreadUnit::PriceBps as i32,
            strategies: vec![
                TieringStrategyDesc {
                    kind: TieringStrategyKind::FlatMarkup as i32,
                    half_spread: 25.0,
                    kappa: 0.0,
                    s_max: 0.0,
                    ..Default::default()
                },
                TieringStrategyDesc {
                    kind: TieringStrategyKind::InventorySkew as i32,
                    half_spread: 25.0,
                    kappa: 1.5,
                    s_max: 100.0,
                    ..Default::default()
                },
                TieringStrategyDesc {
                    kind: TieringStrategyKind::ScaledSmoothedSpread as i32,
                    smoothing_weight: 0.3,
                    expected_spread: 0.00008,
                    max_divergence: 0.00004,
                    core_spread: 0.0002,
                    max_output_spread: 0.0008,
                    spread_scale_factor: 1.2,
                    ..Default::default()
                },
            ],
            guardrails: Some(TieringGuardrailsDesc {
                h_min: 0.0,
                h_max: 5.0,
                s_max: 2.0,
                spread_floor: 0.01,
            }),
            stale_policy: TieringStalePolicy::WidenToMax as i32,
        }
    }

    #[tokio::test]
    async fn aggregated_book_create_list_update_delete_round_trip() {
        let (edge, path, _s) = edge("agg-crud");
        let tok = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;

        // Create returns the minted id and echoes the definition.
        let created = edge
            .create_aggregated_book(Request::new(CreateAggregatedBookRequest {
                session_token: tok.clone(),
                spec: Some(agg_spec(
                    "G10 Rates Composite",
                    vec!["lp-one".into(), "lp-two".into()],
                )),
                correlation_id: Some(7),
            }))
            .await
            .unwrap()
            .into_inner();
        let book = created.book.unwrap();
        assert_eq!(book.id, "g10-rates-composite");
        assert_eq!(book.member_connection_ids, vec!["lp-one", "lp-two"]);
        assert_eq!(
            book.scope_mode,
            AggregationScopeMode::AllMembersQuote as i32
        );
        assert_eq!(created.correlation_id, Some(7));

        // List (any authenticated caller) sees it.
        let listed = edge
            .list_aggregated_books(Request::new(ListAggregatedBooksRequest {
                session_token: tok.clone(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(listed.books.len(), 1);
        assert_eq!(listed.books[0].name, "G10 Rates Composite");

        // Update in place (id preserved): rename + disable + swap members.
        let mut spec = agg_spec("G10 Rates (paused)", vec!["lp-three".into()]);
        spec.enabled = false;
        let updated = edge
            .update_aggregated_book(Request::new(UpdateAggregatedBookRequest {
                session_token: tok.clone(),
                id: book.id.clone(),
                spec: Some(spec),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .book
            .unwrap();
        assert_eq!(updated.id, "g10-rates-composite");
        assert_eq!(updated.name, "G10 Rates (paused)");
        assert!(!updated.enabled);
        assert_eq!(updated.member_connection_ids, vec!["lp-three"]);

        // Delete reports removal; a second delete is a no-op.
        let del = edge
            .delete_aggregated_book(Request::new(DeleteAggregatedBookRequest {
                session_token: tok.clone(),
                id: book.id.clone(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(del.removed);
        let del2 = edge
            .delete_aggregated_book(Request::new(DeleteAggregatedBookRequest {
                session_token: tok.clone(),
                id: book.id,
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(!del2.removed);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn aggregated_book_rejects_duplicate_name() {
        let (edge, path, _s) = edge("agg-dup");
        let tok = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;
        edge.create_aggregated_book(Request::new(CreateAggregatedBookRequest {
            session_token: tok.clone(),
            spec: Some(agg_spec("EM Composite", vec![])),
            correlation_id: None,
        }))
        .await
        .unwrap();
        // Duplicate name (case-insensitive) surfaces the store's error as already_exists.
        let dup = edge
            .create_aggregated_book(Request::new(CreateAggregatedBookRequest {
                session_token: tok,
                spec: Some(agg_spec("em composite", vec![])),
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(dup.code(), tonic::Code::AlreadyExists);
        assert!(dup.message().contains("already exists"));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn aggregated_book_admin_gate_denies_non_admin() {
        let (edge, path, _s) = edge("agg-gate");
        let admin = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;
        let (_id, trader) = make_trader(&edge, &admin, "trader@celnet.com").await;

        // A non-admin may NOT define a book (the administration gate).
        let denied = edge
            .create_aggregated_book(Request::new(CreateAggregatedBookRequest {
                session_token: trader.clone(),
                spec: Some(agg_spec("Trader Book", vec![])),
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(denied.code(), tonic::Code::PermissionDenied);

        // …but a non-admin MAY read the roster (a book is global, decision C).
        let listed = edge
            .list_aggregated_books(Request::new(ListAggregatedBooksRequest {
                session_token: trader,
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(listed.books.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn aggregated_book_persists_across_reload() {
        let (edge, path, _s) = edge("agg-reload");
        let tok = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;
        edge.create_aggregated_book(Request::new(CreateAggregatedBookRequest {
            session_token: tok,
            spec: Some(agg_spec("Persisted Composite", vec!["lp-a".into()])),
            correlation_id: None,
        }))
        .await
        .unwrap();
        // The book survives a fresh load of the same identity file.
        let reloaded = IdentityStore::load(&path).unwrap();
        assert_eq!(reloaded.aggregated_books.len(), 1);
        let def = &reloaded.aggregated_books[0];
        assert_eq!(def.name, "Persisted Composite");
        assert_eq!(def.member_connection_ids, vec!["lp-a"]);
        assert_eq!(def.instrument_scope, Scope::AllMembersQuote);
        let _ = std::fs::remove_file(&path);
    }

    // --- FI pricing groups (Phase 2b — CRUD + trader pipeline retune) -----------

    /// A valid closing-guardrails body (consistent bounds; `h_max >= spread_floor/2`).
    fn pg_guardrails() -> TieringGuardrailsDesc {
        TieringGuardrailsDesc {
            h_min: 0.0,
            h_max: 5.0,
            s_max: 2.0,
            spread_floor: 0.01,
        }
    }

    /// A minimal-but-valid feature pipeline (no features + valid guardrails).
    fn pg_minimal_pipeline() -> FeaturePipelineDesc {
        FeaturePipelineDesc {
            features: Vec::new(),
            guardrails: Some(pg_guardrails()),
        }
    }

    /// A full pipeline exercising ALL five feature kinds (the round-trip surface).
    fn pg_full_pipeline() -> FeaturePipelineDesc {
        FeaturePipelineDesc {
            features: vec![
                FeatureSpecDesc {
                    kind: FeatureKind::MidShift as i32,
                    unit: TieringSpreadUnit::PriceBps as i32,
                    shift: 1.0,
                    reference: Some(100.0),
                    ..Default::default()
                },
                FeatureSpecDesc {
                    kind: FeatureKind::Tiering as i32,
                    tiering: Some(agg_tiering_spec()),
                    ..Default::default()
                },
                FeatureSpecDesc {
                    kind: FeatureKind::Axe as i32,
                    unit: TieringSpreadUnit::PriceBps as i32,
                    axe_side: AxeSide::Sell as i32,
                    magnitude: 2.0,
                    ..Default::default()
                },
                FeatureSpecDesc {
                    kind: FeatureKind::Position as i32,
                    unit: TieringSpreadUnit::PriceBps as i32,
                    kappa: 1.5,
                    s_max: 100.0,
                    ..Default::default()
                },
                FeatureSpecDesc {
                    kind: FeatureKind::PanicSkew as i32,
                    unit: TieringSpreadUnit::PriceBps as i32,
                    skew: 3.0,
                    triggered: true,
                    ..Default::default()
                },
            ],
            guardrails: Some(pg_guardrails()),
        }
    }

    /// A pricing-group spec with both pipelines valid (minimal by default).
    fn pg_spec(name: &str) -> PricingGroupSpec {
        PricingGroupSpec {
            id: String::new(),
            name: name.into(),
            description: "desk tier".into(),
            member_connection_ids: Vec::new(),
            member_user_ids: Vec::new(),
            member_desks: Vec::new(),
            esp_pipeline: Some(pg_minimal_pipeline()),
            rfq_pipeline: Some(pg_minimal_pipeline()),
            share_pipeline: false,
            enabled: true,
        }
    }

    /// (1) Admin creates / updates / deletes a pricing group; a non-admin trader is
    /// `PermissionDenied` on every STRUCTURE mutation, and each admin write survives a
    /// reload from disk.
    #[tokio::test]
    async fn pricing_group_admin_crud_and_non_admin_denied() {
        let (edge, path, _s) = edge("pg-crud");
        let admin = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;
        let (_id, trader) = make_trader(&edge, &admin, "pgcrud@celnet.com").await;

        // Admin creates.
        let created = edge
            .create_pricing_group(Request::new(CreatePricingGroupRequest {
                session_token: admin.clone(),
                spec: Some(pg_spec("GROUP-A")),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .group
            .unwrap();
        assert_eq!(created.name, "GROUP-A");
        assert!(!created.id.is_empty());

        // A non-admin is denied create / update / delete (structure is admin-owned).
        let c = edge
            .create_pricing_group(Request::new(CreatePricingGroupRequest {
                session_token: trader.clone(),
                spec: Some(pg_spec("GROUP-B")),
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(c.code(), tonic::Code::PermissionDenied);
        let mut renamed = pg_spec("GROUP-A2");
        renamed.description = "hijack".into();
        let u = edge
            .update_pricing_group(Request::new(UpdatePricingGroupRequest {
                session_token: trader.clone(),
                id: created.id.clone(),
                spec: Some(renamed.clone()),
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(u.code(), tonic::Code::PermissionDenied);
        let d = edge
            .delete_pricing_group(Request::new(DeletePricingGroupRequest {
                session_token: trader.clone(),
                id: created.id.clone(),
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(d.code(), tonic::Code::PermissionDenied);

        // Admin updates (rename) — lands and survives reload.
        let updated = edge
            .update_pricing_group(Request::new(UpdatePricingGroupRequest {
                session_token: admin.clone(),
                id: created.id.clone(),
                spec: Some(renamed),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .group
            .unwrap();
        assert_eq!(updated.name, "GROUP-A2");
        let reloaded = IdentityStore::load(&path).unwrap();
        assert_eq!(
            reloaded.pricing_group(&created.id).unwrap().name,
            "GROUP-A2"
        );

        // Admin deletes — gone from disk.
        let removed = edge
            .delete_pricing_group(Request::new(DeletePricingGroupRequest {
                session_token: admin.clone(),
                id: created.id.clone(),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .removed;
        assert!(removed);
        let reloaded = IdentityStore::load(&path).unwrap();
        assert!(reloaded.pricing_group(&created.id).is_none());
        let _ = std::fs::remove_file(&path);
    }

    /// A non-admin granted ONLY `risk_manage·fixed_income` can reach the risk-control
    /// RPCs (routing graph, risk-book roster) but is `PermissionDenied` on the
    /// pricing-group and aggregated-book surfaces — and the mirror holds for a trader
    /// granted ONLY `manage_pricing·fixed_income`. Proves the three management
    /// authorities are independent, deny-by-default seats (not one coarse admin gate).
    #[tokio::test]
    async fn granular_manage_capabilities_are_separated() {
        let (edge, path, _s) = edge("granular-manage");
        let admin = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;

        // A trader granted ONLY risk_manage·FI (re-login: the grant revoked the session).
        let (rm_id, _t0) = make_trader(&edge, &admin, "riskmgr@celnet.com").await;
        edge.set_user_capabilities(Request::new(SetUserCapabilitiesRequest {
            session_token: admin.clone(),
            id: rm_id,
            grants: vec![cap("risk_manage", "fixed_income")],
            denies: vec![],
            correlation_id: None,
        }))
        .await
        .unwrap();
        let rm = login(&edge, "riskmgr@celnet.com", "trader-pw-123")
            .await
            .unwrap()
            .session_token;

        // risk_manage·FI CAN read the routing graph and the risk-book roster …
        edge.get_risk_routing_graph(Request::new(GetRiskRoutingGraphRequest {
            session_token: rm.clone(),
            correlation_id: None,
        }))
        .await
        .expect("risk_manage·FI may read the risk-routing graph");
        edge.list_risk_books(Request::new(ListRiskBooksRequest {
            session_token: rm.clone(),
            correlation_id: None,
        }))
        .await
        .expect("risk_manage·FI may list risk books");
        // … but is refused the pricing-group and aggregated-book surfaces.
        assert_eq!(
            edge.create_pricing_group(Request::new(CreatePricingGroupRequest {
                session_token: rm.clone(),
                spec: Some(pg_spec("RM-TRIES-PG")),
                correlation_id: None,
            }))
            .await
            .unwrap_err()
            .code(),
            tonic::Code::PermissionDenied,
            "risk_manage must not authorize pricing-group CRUD"
        );
        assert_eq!(
            edge.create_aggregated_book(Request::new(CreateAggregatedBookRequest {
                session_token: rm.clone(),
                spec: Some(agg_spec("RM-TRIES-AGG", vec![])),
                correlation_id: None,
            }))
            .await
            .unwrap_err()
            .code(),
            tonic::Code::PermissionDenied,
            "risk_manage must not authorize aggregated-book config"
        );

        // The mirror: a trader granted ONLY manage_pricing·FI CAN create a pricing group
        // but is refused the risk-control RPCs.
        let (mp_id, _t1) = make_trader(&edge, &admin, "pricingdesk@celnet.com").await;
        edge.set_user_capabilities(Request::new(SetUserCapabilitiesRequest {
            session_token: admin.clone(),
            id: mp_id,
            grants: vec![cap("manage_pricing", "fixed_income")],
            denies: vec![],
            correlation_id: None,
        }))
        .await
        .unwrap();
        let mp = login(&edge, "pricingdesk@celnet.com", "trader-pw-123")
            .await
            .unwrap()
            .session_token;

        edge.create_pricing_group(Request::new(CreatePricingGroupRequest {
            session_token: mp.clone(),
            spec: Some(pg_spec("MP-OWNS-PG")),
            correlation_id: None,
        }))
        .await
        .expect("manage_pricing·FI may create a pricing group");
        assert_eq!(
            edge.list_risk_books(Request::new(ListRiskBooksRequest {
                session_token: mp.clone(),
                correlation_id: None,
            }))
            .await
            .unwrap_err()
            .code(),
            tonic::Code::PermissionDenied,
            "manage_pricing must not authorize the risk-book roster"
        );
        assert_eq!(
            edge.get_risk_routing_graph(Request::new(GetRiskRoutingGraphRequest {
                session_token: mp,
                correlation_id: None,
            }))
            .await
            .unwrap_err()
            .code(),
            tonic::Code::PermissionDenied,
            "manage_pricing must not authorize the risk-routing graph"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// (2) A non-admin trader HOLDING `quote_respond·fixed_income` CAN retune a group's
    /// pipeline (the retune re-prices — the persisted ESP pipeline changes and the
    /// resolver rebuilds); a trader DENIED it is refused; an unauthenticated caller is
    /// rejected.
    #[tokio::test]
    async fn pricing_group_pipeline_retune_trader_gated() {
        let (edge, path, _s) = edge("pg-pipeline");
        let admin = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;
        let created = edge
            .create_pricing_group(Request::new(CreatePricingGroupRequest {
                session_token: admin.clone(),
                spec: Some(pg_spec("TIER1-EU")),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .group
            .unwrap();

        // A plain trader (default bundle holds quote_respond·fixed_income) CAN retune.
        let (_id, trader) = make_trader(&edge, &admin, "pgtune@celnet.com").await;
        let retuned = edge
            .update_pricing_group_pipeline(Request::new(UpdatePricingGroupPipelineRequest {
                session_token: trader.clone(),
                group_id: created.id.clone(),
                mode: EspOrRfq::Esp as i32,
                pipeline: Some(pg_full_pipeline()),
                share_pipeline: true,
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .group
            .unwrap();
        // The ESP pipeline now carries the full five-feature ladder; share flag flipped.
        assert_eq!(retuned.esp_pipeline.unwrap().features.len(), 5);
        assert!(retuned.share_pipeline);
        // The retune re-priced: the persisted group + rebuilt resolver reflect it.
        let reloaded = IdentityStore::load(&path).unwrap();
        let def = reloaded.pricing_group(&created.id).unwrap();
        assert_eq!(def.esp_pipeline.features.len(), 5);
        assert!(def.share_pipeline);
        // The RFQ (unselected) pipeline block was NOT touched — still the minimal one.
        assert!(def.rfq_pipeline.features.is_empty());

        // A trader DENIED the capability (separation of duties) is refused.
        let (denied_id, _tok) = make_trader(&edge, &admin, "pgdeny@celnet.com").await;
        edge.set_user_capabilities(Request::new(SetUserCapabilitiesRequest {
            session_token: admin.clone(),
            id: denied_id,
            grants: Vec::new(),
            denies: vec![cap("quote_respond", "fixed_income")],
            correlation_id: None,
        }))
        .await
        .unwrap();
        let denied_tok = login(&edge, "pgdeny@celnet.com", "trader-pw-123")
            .await
            .unwrap()
            .session_token;
        let denied = edge
            .update_pricing_group_pipeline(Request::new(UpdatePricingGroupPipelineRequest {
                session_token: denied_tok,
                group_id: created.id.clone(),
                mode: EspOrRfq::Rfq as i32,
                pipeline: Some(pg_full_pipeline()),
                share_pipeline: false,
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(denied.code(), tonic::Code::PermissionDenied);

        // An unauthenticated token is rejected before any mutation.
        let anon = edge
            .update_pricing_group_pipeline(Request::new(UpdatePricingGroupPipelineRequest {
                session_token: "not-a-token".into(),
                group_id: created.id.clone(),
                mode: EspOrRfq::Esp as i32,
                pipeline: Some(pg_full_pipeline()),
                share_pipeline: false,
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(anon.code(), tonic::Code::Unauthenticated);
        let _ = std::fs::remove_file(&path);
    }

    /// (3) `ListPricingGroups` returns the roster for an authenticated non-admin; an
    /// unauthenticated token is rejected.
    #[tokio::test]
    async fn list_pricing_groups_readable_by_trader() {
        let (edge, path, _s) = edge("pg-list");
        let admin = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;
        let created = edge
            .create_pricing_group(Request::new(CreatePricingGroupRequest {
                session_token: admin.clone(),
                spec: Some(pg_spec("GROUP-R")),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .group
            .unwrap();

        let (_id, trader) = make_trader(&edge, &admin, "pglist@celnet.com").await;
        let listed = edge
            .list_pricing_groups(Request::new(ListPricingGroupsRequest {
                session_token: trader,
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(listed.groups.len(), 1);
        assert_eq!(listed.groups[0].id, created.id);

        let anon = edge
            .list_pricing_groups(Request::new(ListPricingGroupsRequest {
                session_token: "not-a-token".into(),
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(anon.code(), tonic::Code::Unauthenticated);
        let _ = std::fs::remove_file(&path);
    }

    /// (4) A group carrying a full (all-five-feature) pipeline round-trips
    /// create → wire → disk reload → decode with the pipeline preserved exactly.
    #[tokio::test]
    async fn pricing_group_full_pipeline_round_trips_through_disk() {
        let (edge, path, _s) = edge("pg-roundtrip");
        let admin = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;
        let mut spec = pg_spec("GROUP-FULL");
        spec.esp_pipeline = Some(pg_full_pipeline());
        spec.rfq_pipeline = Some(pg_full_pipeline());

        let created = edge
            .create_pricing_group(Request::new(CreatePricingGroupRequest {
                session_token: admin.clone(),
                spec: Some(spec),
                correlation_id: None,
            }))
            .await
            .unwrap()
            .into_inner()
            .group
            .unwrap();

        // The wire echo carries the full five-feature ESP pipeline verbatim.
        let esp = created.esp_pipeline.as_ref().unwrap();
        assert_eq!(esp.features.len(), 5);
        assert_eq!(esp.features[0].kind, FeatureKind::MidShift as i32);
        assert_eq!(esp.features[0].reference, Some(100.0));
        assert_eq!(esp.features[1].kind, FeatureKind::Tiering as i32);
        assert!(esp.features[1].tiering.is_some());
        assert_eq!(esp.features[4].kind, FeatureKind::PanicSkew as i32);
        assert!(esp.features[4].triggered);

        // Reload from disk and re-encode: byte-identical to the create echo.
        let reloaded = IdentityStore::load(&path).unwrap();
        let def = reloaded.pricing_group(&created.id).unwrap();
        let re_encoded = pricing_group_to_wire(def);
        assert_eq!(re_encoded, created);
        assert_eq!(def.esp_pipeline.features.len(), 5);
        assert_eq!(def.rfq_pipeline.features.len(), 5);
        let _ = std::fs::remove_file(&path);
    }

    /// (5) Validation rejects a bad pipeline (inconsistent guardrails) and a duplicate
    /// member; neither lands on disk.
    #[tokio::test]
    async fn pricing_group_create_validation() {
        let (edge, path, _s) = edge("pg-validate");
        let admin = login(&edge, "admin@celnet.com", "password")
            .await
            .unwrap()
            .session_token;
        let (trader_id, _tok) = make_trader(&edge, &admin, "pgval@celnet.com").await;

        // Inconsistent guardrails (h_max < spread_floor/2) → invalid_argument.
        let mut bad_pipe = pg_spec("BAD-PIPE");
        bad_pipe.esp_pipeline = Some(FeaturePipelineDesc {
            features: Vec::new(),
            guardrails: Some(TieringGuardrailsDesc {
                h_min: 0.0,
                h_max: 0.0,
                s_max: 1.0,
                spread_floor: 1.0,
            }),
        });
        let e1 = edge
            .create_pricing_group(Request::new(CreatePricingGroupRequest {
                session_token: admin.clone(),
                spec: Some(bad_pipe),
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(e1.code(), tonic::Code::InvalidArgument);

        // A duplicate user member → invalid_argument.
        let mut dup_member = pg_spec("DUP-MEMBER");
        dup_member.member_user_ids = vec![trader_id.clone(), trader_id];
        let e2 = edge
            .create_pricing_group(Request::new(CreatePricingGroupRequest {
                session_token: admin.clone(),
                spec: Some(dup_member),
                correlation_id: None,
            }))
            .await
            .unwrap_err();
        assert_eq!(e2.code(), tonic::Code::InvalidArgument);

        // Neither bad group landed.
        let reloaded = IdentityStore::load(&path).unwrap();
        assert!(reloaded.pricing_groups.is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
