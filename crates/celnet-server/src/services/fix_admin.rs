//! The `FixAdminService` edge: the admin surface that manages the inbound FIX
//! acceptor connections the edge binds (define / persist / list / enable /
//! disable / delete).
//!
//! It is a thin wire adapter over [`crate::services::fix_registry::FixAcceptorRegistry`]
//! (which owns the live bind/stop lifecycle + JSON persistence): each RPC maps the
//! wire payload onto a registry call and maps the registry's typed result back to
//! the contract. Like `RiskService`, every RPC is **entitlement-gated** — it passes
//! the deny-by-default authorization boundary ([`crate::services::access::authorize`])
//! before mutating anything, reading the one coherent [`AccessMode`] the shared
//! [`PositionStore`] carries (so gRPC and the WS mirror enforce the same policy). The
//! WS mirror dispatches onto these same trait methods, so one boundary covers both
//! encodings.

// `tonic::Status` is the contract's typed error; its size is the wire library's
// choice (the same allowance every service module carries).
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use celnet_entitlements::{Action, AssetClass};
use celnet_proto::fix_admin_service_server::FixAdminService;
use celnet_proto::{
    CreateFixConnectionRequest, CreateFixConnectionResponse, DeleteFixConnectionRequest,
    DeleteFixConnectionResponse, FixAcceptorKind, FixConnectionDesc, FixConnectionSpec, FixMessage,
    FixMsgDirection, ListFixConnectionsRequest, ListFixConnectionsResponse, ListFixMessagesRequest,
    ListFixMessagesResponse, SetFixConnectionEnabledRequest, SetFixConnectionEnabledResponse,
    UpdateFixConnectionRequest, UpdateFixConnectionResponse,
};
use tonic::{Request, Response, Status};

use crate::clock::Clock;
use crate::config::fix_connections::{AcceptorKind, FixConnectionDef};
use crate::readiness::ReadinessGate;
use crate::services::access::{DeskScope, RequiredAuthority, authorize_caller, resolve_caller};
use crate::services::fix_monitor::{FixDirection, FixMessageEvent, FixMonitor};
use crate::services::fix_registry::{ConnectionStatus, FixAcceptorRegistry};
use crate::services::risk::store::PositionStore;
use crate::services::sessions::SessionRegistry;

/// The default page size when a `ListMessages` request leaves `limit` at 0.
const DEFAULT_MESSAGE_LIMIT: usize = 500;
/// The hard cap on a single `ListMessages` page.
const MAX_MESSAGE_LIMIT: usize = 4096;

/// The `FixAdminService` edge over the shared acceptor registry.
///
/// Holds the [`FixAcceptorRegistry`] (the live set + persistence), the
/// [`ReadinessGate`] (so admin ops are refused while starting/draining, like every
/// other service), and the shared [`PositionStore`] purely as the source of the one
/// coherent [`celnet_entitlements::AccessMode`] the entitlement boundary reads.
pub struct FixAdminEdge {
    registry: Arc<FixAcceptorRegistry>,
    gate: Arc<ReadinessGate>,
    store: Arc<PositionStore>,
    monitor: Arc<FixMonitor>,
    /// The live session registry the entitlement boundary validates `session_token`
    /// against. [`FixAdminEdge::new`] defaults to a fresh empty registry (consulted
    /// only when a request presents a token); the boot path overrides it with the
    /// edge-wide registry via [`FixAdminEdge::with_sessions`] so both fronts share
    /// one authentication state.
    sessions: Arc<SessionRegistry>,
}

impl FixAdminEdge {
    /// Construct the admin edge over the shared registry, readiness gate, the
    /// access-mode-bearing position store, and the session-traffic capture sink the
    /// `ListMessages` poll serves.
    #[must_use]
    pub fn new(
        registry: Arc<FixAcceptorRegistry>,
        gate: Arc<ReadinessGate>,
        store: Arc<PositionStore>,
        monitor: Arc<FixMonitor>,
    ) -> Self {
        Self {
            registry,
            gate,
            store,
            monitor,
            sessions: Arc::new(SessionRegistry::new(Clock::system())),
        }
    }

    /// Install the edge-wide [`SessionRegistry`] so this admin edge validates
    /// session tokens against the SAME authentication state every other front
    /// shares. The boot path calls this; the constructor otherwise defaults to an
    /// empty registry (so principal-only tests need no session wiring).
    #[must_use]
    pub fn with_sessions(mut self, sessions: Arc<SessionRegistry>) -> Self {
        self.sessions = sessions;
        self
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

    /// The set of connection ids a desk-scoped caller may read traffic for: the
    /// connections whose owning desk the `scope` admits, optionally narrowed to a
    /// single `requested` connection (a connection outside the desk yields the
    /// empty set, so the monitor returns no rows for it). Only reached on the
    /// non-`All` path — an admin / no-session caller never builds this set.
    async fn allowed_connection_ids(
        &self,
        scope: &DeskScope,
        requested: Option<&str>,
    ) -> std::collections::HashSet<String> {
        self.registry
            .list()
            .await
            .iter()
            .filter(|s| scope.allows(&s.def.desk))
            .map(|s| s.def.id.clone())
            .filter(|id| requested.is_none_or(|r| r == id))
            .collect()
    }
}

#[tonic::async_trait]
impl FixAdminService for FixAdminEdge {
    async fn list_connections(
        &self,
        request: Request<ListFixConnectionsRequest>,
    ) -> Result<Response<ListFixConnectionsResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "FixAdminService/ListConnections",
            RequiredAuthority::ReadAny,
            req.correlation_id,
        )?;
        // Desk scoping: an admin (or the no-session demo/legacy path) sees every
        // connection; a trader session sees only its own desk's connections.
        let scope = caller.desk_scope();
        let connections = self
            .registry
            .list()
            .await
            .iter()
            .filter(|s| scope.allows(&s.def.desk))
            .map(status_to_wire)
            .collect();
        Ok(Response::new(ListFixConnectionsResponse {
            connections,
            correlation_id: req.correlation_id,
        }))
    }

    async fn create_connection(
        &self,
        request: Request<CreateFixConnectionRequest>,
    ) -> Result<Response<CreateFixConnectionResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        let spec = req
            .spec
            .ok_or_else(|| Status::invalid_argument("create: missing connection spec"))?;
        // Base gate: managing an inbound-liquidity venue is `manage_liquidity` on the
        // asset the venue serves (was the coarse admin role); an admin holds grant-all
        // and passes on either asset (`docs/PERMISSIONS-GRANULAR-REVIEW.md` §3.1/§4).
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "FixAdminService/CreateConnection",
            RequiredAuthority::Capability(
                Action::ManageLiquidity,
                wire_connection_manage_asset(spec.kind),
            ),
            req.correlation_id,
        )?;
        let def = def_from_spec(&spec, None)?;
        // Dialect-specific capability gate, in ADDITION to the liquidity-management gate
        // above: standing up a fixed-income venue requires the matching FI action
        // capability, so a caller denied that capability cannot create it.
        if let Some((action, asset)) = required_dialect_capability(def.kind) {
            authorize_caller(
                self.store.access_mode(),
                &caller,
                "FixAdminService/CreateConnection",
                RequiredAuthority::Capability(action, asset),
                req.correlation_id,
            )?;
        }
        let status = self.registry.create(def).await.map_err(registry_status)?;
        Ok(Response::new(CreateFixConnectionResponse {
            connection: Some(status_to_wire(&status)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn update_connection(
        &self,
        request: Request<UpdateFixConnectionRequest>,
    ) -> Result<Response<UpdateFixConnectionResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        let spec = req
            .spec
            .ok_or_else(|| Status::invalid_argument("update: missing connection spec"))?;
        // Base gate: venue-ops `manage_liquidity` on the (target) connection's asset,
        // replacing the coarse admin role (`docs/PERMISSIONS-GRANULAR-REVIEW.md` §3.1/§4).
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "FixAdminService/UpdateConnection",
            RequiredAuthority::Capability(
                Action::ManageLiquidity,
                wire_connection_manage_asset(spec.kind),
            ),
            req.correlation_id,
        )?;
        if req.id.trim().is_empty() {
            return Err(Status::invalid_argument("update: missing connection id"));
        }
        // The request `id` is authoritative; the spec's own id is ignored on update.
        let def = def_from_spec(&spec, Some(req.id.clone()))?;
        let status = self
            .registry
            .update(&req.id, def)
            .await
            .map_err(registry_status)?;
        Ok(Response::new(UpdateFixConnectionResponse {
            connection: Some(status_to_wire(&status)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn delete_connection(
        &self,
        request: Request<DeleteFixConnectionRequest>,
    ) -> Result<Response<DeleteFixConnectionResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        // Base gate: venue-ops `manage_liquidity` on the connection's asset (resolved from
        // the live set; an unknown id defaults to FI, where an admin still passes and the
        // delete below returns not_found) — replaces the coarse admin role
        // (`docs/PERMISSIONS-GRANULAR-REVIEW.md` §3.1/§4).
        let asset = self
            .registry
            .list()
            .await
            .into_iter()
            .find(|s| s.def.id == req.id)
            .map(|s| connection_manage_asset(s.def.kind))
            .unwrap_or(AssetClass::FixedIncome);
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "FixAdminService/DeleteConnection",
            RequiredAuthority::Capability(Action::ManageLiquidity, asset),
            req.correlation_id,
        )?;
        if req.id.trim().is_empty() {
            return Err(Status::invalid_argument("delete: missing connection id"));
        }
        self.registry
            .delete(&req.id)
            .await
            .map_err(registry_status)?;
        Ok(Response::new(DeleteFixConnectionResponse {
            correlation_id: req.correlation_id,
        }))
    }

    async fn set_enabled(
        &self,
        request: Request<SetFixConnectionEnabledRequest>,
    ) -> Result<Response<SetFixConnectionEnabledResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        // The connection's kind is read once from the live set (an unknown id falls
        // through to the registry's typed `not_found` below) and drives BOTH the base
        // venue-ops gate and the dialect gate.
        let kind = self
            .registry
            .list()
            .await
            .into_iter()
            .find(|s| s.def.id == req.id)
            .map(|s| s.def.kind);
        // Base gate: venue-ops `manage_liquidity` on the connection's asset (unknown id
        // defaults to FI; an admin passes regardless), replacing the coarse admin role
        // (`docs/PERMISSIONS-GRANULAR-REVIEW.md` §3.1/§4).
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "FixAdminService/SetEnabled",
            RequiredAuthority::Capability(
                Action::ManageLiquidity,
                kind.map(connection_manage_asset)
                    .unwrap_or(AssetClass::FixedIncome),
            ),
            req.correlation_id,
        )?;
        if req.id.trim().is_empty() {
            return Err(Status::invalid_argument(
                "set_enabled: missing connection id",
            ));
        }
        // Dialect-specific capability gate, in ADDITION to the venue-ops gate:
        // (re)binding a fixed-income venue requires the matching FI action capability.
        if let Some((action, asset)) = kind.and_then(required_dialect_capability) {
            authorize_caller(
                self.store.access_mode(),
                &caller,
                "FixAdminService/SetEnabled",
                RequiredAuthority::Capability(action, asset),
                req.correlation_id,
            )?;
        }
        let status = self
            .registry
            .set_enabled(&req.id, req.enabled)
            .await
            .map_err(registry_status)?;
        Ok(Response::new(SetFixConnectionEnabledResponse {
            connection: Some(status_to_wire(&status)),
            correlation_id: req.correlation_id,
        }))
    }

    async fn list_messages(
        &self,
        request: Request<ListFixMessagesRequest>,
    ) -> Result<Response<ListFixMessagesResponse>, Status> {
        let _guard = self.gate.enter();
        self.require_ready()?;
        let req = request.into_inner();
        let caller = resolve_caller(
            &self.sessions,
            req.session_token.as_deref(),
            req.principal.clone(),
        )?;
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "FixAdminService/ListMessages",
            RequiredAuthority::ReadAny,
            req.correlation_id,
        )?;
        let limit = match req.limit as usize {
            0 => DEFAULT_MESSAGE_LIMIT,
            n => n.min(MAX_MESSAGE_LIMIT),
        };
        let connection_id = req
            .connection_id
            .as_deref()
            .filter(|s| !s.trim().is_empty());
        // Desk scoping: an admin / no-session caller reads the whole capture
        // (optionally narrowed to one connection); a desk-scoped trader sees only
        // the traffic of the connections its desk owns.
        let scope = caller.desk_scope();
        let (events, latest_seq) = if scope.is_all() {
            self.monitor.since(connection_id, req.after_seq, limit)
        } else {
            let allowed = self.allowed_connection_ids(&scope, connection_id).await;
            self.monitor.since_in(&allowed, req.after_seq, limit)
        };
        Ok(Response::new(ListFixMessagesResponse {
            messages: events.iter().map(event_to_wire).collect(),
            latest_seq,
            correlation_id: req.correlation_id,
        }))
    }
}

/// Map a captured [`FixMessageEvent`] onto its wire [`FixMessage`].
fn event_to_wire(e: &FixMessageEvent) -> FixMessage {
    FixMessage {
        seq: e.seq,
        connection_id: e.connection_id.clone(),
        direction: direction_to_wire(e.direction) as i32,
        msg_type: e.msg_type.clone(),
        summary: e.summary.clone(),
        epoch_nanos: e.epoch_nanos,
        raw: e.raw.clone(),
    }
}

/// The wire enum value for a captured frame's travel direction.
fn direction_to_wire(direction: FixDirection) -> FixMsgDirection {
    match direction {
        FixDirection::Inbound => FixMsgDirection::Inbound,
        FixDirection::Outbound => FixMsgDirection::Outbound,
    }
}

// --- wire ⇄ domain mapping -------------------------------------------------

/// Map a runtime [`ConnectionStatus`] onto its wire [`FixConnectionDesc`]
/// (definition + live `running`/`bound_addr` status).
fn status_to_wire(s: &ConnectionStatus) -> FixConnectionDesc {
    FixConnectionDesc {
        id: s.def.id.clone(),
        name: s.def.name.clone(),
        kind: kind_to_wire(s.def.kind),
        bind_addr: s.def.bind_addr.clone(),
        sender_comp_id: s.def.sender_comp_id.clone(),
        target_comp_id: s.def.target_comp_id.clone(),
        enabled: s.def.enabled,
        running: s.running,
        bound_addr: s.bound_addr.map(|a| a.to_string()).unwrap_or_default(),
        desk: s.def.desk.clone(),
    }
}

/// Build a [`FixConnectionDef`] from a wire [`FixConnectionSpec`]. `id_override`
/// (the request id on update) wins; otherwise the spec's id is used, falling back
/// to a slug of the name when empty (the create-time mint). Structural validation
/// is left to the registry, which also knows the live set for cross-cutting checks.
///
/// # Errors
/// `invalid_argument` for an unrecognised kind, or when no id can be derived.
fn def_from_spec(
    spec: &FixConnectionSpec,
    id_override: Option<String>,
) -> Result<FixConnectionDef, Status> {
    let id = match id_override {
        Some(id) => id,
        None if !spec.id.trim().is_empty() => spec.id.trim().to_string(),
        None => slugify(&spec.name).ok_or_else(|| {
            Status::invalid_argument("create: name yields no usable id; set an explicit id")
        })?,
    };
    // Every managed FIX connection MUST belong to a desk — there are no unowned
    // "house" acceptors. A desk-scoped trader then sees exactly its desk's
    // connections + their RFQ traffic, and administration is always "for a desk".
    let desk = spec.desk.trim().to_string();
    if desk.is_empty() {
        return Err(Status::invalid_argument(
            "a FIX connection must belong to a desk (set the owning desk id)",
        ));
    }
    Ok(FixConnectionDef {
        id,
        name: spec.name.clone(),
        kind: kind_from_wire(spec.kind)?,
        bind_addr: spec.bind_addr.clone(),
        sender_comp_id: spec.sender_comp_id.clone(),
        target_comp_id: spec.target_comp_id.clone(),
        enabled: spec.enabled,
        desk,
    })
}

/// The wire enum value for a domain [`AcceptorKind`].
fn kind_to_wire(kind: AcceptorKind) -> i32 {
    match kind {
        AcceptorKind::Options => FixAcceptorKind::Options as i32,
        AcceptorKind::FixedIncomeQuote => FixAcceptorKind::FixedIncomeQuote as i32,
        AcceptorKind::FixedIncomeStream => FixAcceptorKind::FixedIncomeStream as i32,
    }
}

/// Resolve a wire enum value to a domain [`AcceptorKind`].
///
/// # Errors
/// `invalid_argument` for an unrecognised enum value.
fn kind_from_wire(kind: i32) -> Result<AcceptorKind, Status> {
    match FixAcceptorKind::try_from(kind) {
        Ok(FixAcceptorKind::Options) => Ok(AcceptorKind::Options),
        Ok(FixAcceptorKind::FixedIncomeQuote) => Ok(AcceptorKind::FixedIncomeQuote),
        Ok(FixAcceptorKind::FixedIncomeStream) => Ok(AcceptorKind::FixedIncomeStream),
        Err(_) => Err(Status::invalid_argument(format!(
            "unrecognised FIX acceptor kind `{kind}`"
        ))),
    }
}

/// The action capability that standing up a connection of `kind` requires **in
/// addition to** connection administration ([`RequiredAuthority::Admin`]):
///
/// * a fixed-income **quote** (one-shot RFQ) venue needs `QuoteRespond` on
///   [`AssetClass::FixedIncome`];
/// * a fixed-income **stream** (RFS) venue needs `Stream` on the same asset class;
/// * an FX-**options** venue needs nothing beyond administration (returns `None`).
///
/// So an administrator who is explicitly denied the relevant FI capability cannot
/// stand up that FI venue, even though they may administer connections generally —
/// the dialect-specific gate is enforced on create and on enable.
fn required_dialect_capability(kind: AcceptorKind) -> Option<(Action, AssetClass)> {
    match kind {
        AcceptorKind::Options => None,
        AcceptorKind::FixedIncomeQuote => Some((Action::QuoteRespond, AssetClass::FixedIncome)),
        AcceptorKind::FixedIncomeStream => Some((Action::Stream, AssetClass::FixedIncome)),
    }
}

/// The asset class a connection of `kind` serves — the scope its **liquidity /
/// venue-ops** administration ([`Action::ManageLiquidity`]) is exercised under. An
/// FX-options venue is administered under [`AssetClass::FxOptions`]; both fixed-income
/// dialects under [`AssetClass::FixedIncome`]. This makes an FX-liquidity and an
/// FI-liquidity seat separately grantable (`docs/PERMISSIONS-GRANULAR-REVIEW.md` §3.1)
/// while an admin (grant-all) administers every venue regardless.
fn connection_manage_asset(kind: AcceptorKind) -> AssetClass {
    match kind {
        AcceptorKind::Options => AssetClass::FxOptions,
        AcceptorKind::FixedIncomeQuote | AcceptorKind::FixedIncomeStream => AssetClass::FixedIncome,
    }
}

/// [`connection_manage_asset`] resolved from the **wire** acceptor-kind value (the
/// create/update spec carries a raw `i32`), so the venue-ops authorization decision is
/// made before the spec is fully validated. An unrecognised kind defaults to
/// [`AssetClass::FxOptions`]; a genuinely malformed spec is then rejected loudly by
/// `def_from_spec`, and an admin (grant-all) passes the gate on either asset regardless.
fn wire_connection_manage_asset(wire_kind: i32) -> AssetClass {
    match FixAcceptorKind::try_from(wire_kind) {
        Ok(FixAcceptorKind::FixedIncomeQuote | FixAcceptorKind::FixedIncomeStream) => {
            AssetClass::FixedIncome
        }
        _ => AssetClass::FxOptions,
    }
}

/// Map a registry error string onto a typed gRPC status: a "not found" is
/// `not_found`; everything else (validation, uniqueness/address conflict, bind or
/// persist failure) is `failed_precondition` — the request was well-formed but the
/// live set rejected it.
fn registry_status(msg: String) -> Status {
    if msg.starts_with("no connection with id") {
        Status::not_found(msg)
    } else {
        Status::failed_precondition(msg)
    }
}

/// A lowercase, hyphen-separated slug of `name` (alphanumerics kept, every other
/// run collapsed to a single `-`, ends trimmed). `None` if nothing usable remains.
fn slugify(name: &str) -> Option<String> {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let slug = out.trim_end_matches('-').to_string();
    if slug.is_empty() { None } else { Some(slug) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, name: &str) -> FixConnectionSpec {
        FixConnectionSpec {
            id: id.to_string(),
            name: name.to_string(),
            kind: FixAcceptorKind::Options as i32,
            bind_addr: "127.0.0.1:9099".to_string(),
            sender_comp_id: "CELNET".to_string(),
            target_comp_id: "CELNET-CPTY".to_string(),
            enabled: true,
            desk: "g10".to_string(),
        }
    }

    #[test]
    fn spec_with_explicit_id_is_preserved() {
        let def = def_from_spec(&spec("opt-1", "Bank A"), None).unwrap();
        assert_eq!(def.id, "opt-1");
        assert_eq!(def.kind, AcceptorKind::Options);
        assert!(def.enabled);
        assert_eq!(def.desk, "g10", "the owning desk rides through the mapper");
    }

    #[test]
    fn empty_id_is_slugged_from_name() {
        let def = def_from_spec(&spec("", "Bank A — Options!"), None).unwrap();
        assert_eq!(def.id, "bank-a-options");
    }

    #[test]
    fn update_id_override_wins_over_spec_id() {
        let def = def_from_spec(&spec("spec-id", "Bank A"), Some("req-id".to_string())).unwrap();
        assert_eq!(def.id, "req-id");
    }

    #[test]
    fn unrecognised_kind_is_rejected() {
        let mut s = spec("opt-1", "Bank A");
        s.kind = 999;
        assert_eq!(
            def_from_spec(&s, None).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
    }

    #[test]
    fn a_connection_must_belong_to_a_desk() {
        // No unowned "house" acceptors — an absent/blank desk is rejected so every
        // managed connection is administered for a desk.
        let mut s = spec("opt-1", "Bank A");
        s.desk = "   ".to_string();
        let err = def_from_spec(&s, None).unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("desk"), "names the missing desk");
        s.desk = String::new();
        assert_eq!(
            def_from_spec(&s, None).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
    }

    #[test]
    fn registry_error_maps_to_typed_status() {
        assert_eq!(
            registry_status("no connection with id `x`".to_string()).code(),
            tonic::Code::NotFound
        );
        assert_eq!(
            registry_status("address `..` is already used".to_string()).code(),
            tonic::Code::FailedPrecondition
        );
    }

    #[test]
    fn slugify_handles_unusable_names() {
        assert_eq!(slugify("  !!! "), None);
        assert_eq!(slugify("EUR/USD Bank"), Some("eur-usd-bank".to_string()));
    }

    #[test]
    fn fi_kinds_round_trip_through_the_wire_mapper() {
        for kind in [
            AcceptorKind::Options,
            AcceptorKind::FixedIncomeQuote,
            AcceptorKind::FixedIncomeStream,
        ] {
            assert_eq!(kind_from_wire(kind_to_wire(kind)).unwrap(), kind);
        }
    }

    #[test]
    fn dialect_capability_matches_the_kind() {
        assert_eq!(required_dialect_capability(AcceptorKind::Options), None);
        assert_eq!(
            required_dialect_capability(AcceptorKind::FixedIncomeQuote),
            Some((Action::QuoteRespond, AssetClass::FixedIncome))
        );
        assert_eq!(
            required_dialect_capability(AcceptorKind::FixedIncomeStream),
            Some((Action::Stream, AssetClass::FixedIncome))
        );
    }

    // --- capability gate on create / set-enabled (behavioral) ---------------

    use crate::config::identity::{Role, default_trader_bundle};
    use crate::services::sessions::AuthenticatedUser;
    use crate::spread::SpreadModel;
    use crate::surface_book::SurfaceBook;
    use celnet_entitlements::{AccessMode, Capability};

    /// A ready in-process admin edge under [`AccessMode::Enforce`] backed by a real
    /// (but unbound) acceptor registry, returning the edge + the shared session
    /// registry (to mint users) so the capability gate is exercised end-to-end.
    fn edge_under_enforce() -> (FixAdminEdge, Arc<SessionRegistry>) {
        let clock = Clock::system();
        let link = {
            let initial = celnet_engine::testing::make_state(
                1.10,
                celnet_conventions::resolve(
                    celnet_types::CcyPair::parse("EURUSD").unwrap(),
                    celnet_types::Tenor::Years(1),
                )
                .record,
            );
            crate::core_link::CoreLink::start(initial, None)
        };
        let monitor = Arc::new(FixMonitor::new());
        // A unique, per-process config path keeps the persistence isolated.
        let cfg = std::env::temp_dir().join(format!(
            "celnet-fixadmin-caps-{}-{:p}.json",
            std::process::id(),
            &link
        ));
        let _ = std::fs::remove_file(&cfg);
        let store = Arc::new(PositionStore::new());
        store.set_access_mode(AccessMode::Enforce);
        let registry = Arc::new(
            FixAcceptorRegistry::load(
                link,
                SpreadModel::default(),
                clock.clone(),
                Arc::new(SurfaceBook::new()),
                Arc::clone(&monitor),
                Arc::clone(&store),
                None,
                cfg,
            )
            .unwrap(),
        );
        let gate = Arc::new(ReadinessGate::new());
        gate.mark_ready();
        let sessions = Arc::new(SessionRegistry::new(clock));
        let edge =
            FixAdminEdge::new(registry, gate, store, monitor).with_sessions(Arc::clone(&sessions));
        (edge, sessions)
    }

    /// Mint a session token for a user with the given role and explicit capability
    /// denies (the role bundle is the base; denies win).
    fn token_for(sessions: &SessionRegistry, role: Role, cap_denies: Vec<Capability>) -> String {
        token_with(sessions, role, Vec::new(), cap_denies)
    }

    /// Mint a session token with explicit per-user grants and denies over the trader
    /// role bundle — used to model a narrowly-granted non-admin seat (e.g. a venue-ops
    /// trader holding `manage_liquidity` without the admin role).
    fn token_with(
        sessions: &SessionRegistry,
        role: Role,
        cap_grants: Vec<Capability>,
        cap_denies: Vec<Capability>,
    ) -> String {
        sessions
            .issue(AuthenticatedUser {
                user_id: "u-1".into(),
                email: "u-1@celnet.com".into(),
                display_name: "U".into(),
                role,
                desk_ids: vec!["g10".into()],
                all_desks: false,
                role_caps: default_trader_bundle(),
                cap_grants,
                cap_denies,
            })
            .unwrap()
            .token
    }

    /// A create request for a connection of `kind` (saved, not bound).
    fn create_req(token: &str, kind: FixAcceptorKind) -> CreateFixConnectionRequest {
        CreateFixConnectionRequest {
            spec: Some(FixConnectionSpec {
                id: "fi-1".into(),
                name: "Bank A — FI".into(),
                kind: kind as i32,
                bind_addr: "127.0.0.1:0".into(),
                sender_comp_id: "CELNET".into(),
                target_comp_id: "CELNET-CPTY".into(),
                enabled: false,
                desk: "g10".into(),
            }),
            principal: None,
            correlation_id: Some(1),
            session_token: Some(token.to_string()),
        }
    }

    #[tokio::test]
    async fn admin_with_fi_quote_capability_creates_fi_quote_venue() {
        let (edge, sessions) = edge_under_enforce();
        let token = token_for(&sessions, Role::Admin, Vec::new());
        let resp = edge
            .create_connection(Request::new(create_req(
                &token,
                FixAcceptorKind::FixedIncomeQuote,
            )))
            .await
            .expect("an admin with QuoteRespond·FixedIncome may create the FI-quote venue");
        assert_eq!(
            resp.into_inner().connection.unwrap().kind,
            FixAcceptorKind::FixedIncomeQuote as i32
        );
    }

    #[tokio::test]
    async fn admin_without_fi_quote_capability_is_denied() {
        let (edge, sessions) = edge_under_enforce();
        // An administrator explicitly denied QuoteRespond·FixedIncome: the
        // administration gate passes, but the dialect capability gate refuses.
        let token = token_for(
            &sessions,
            Role::Admin,
            vec![Capability::new(
                Action::QuoteRespond,
                AssetClass::FixedIncome,
            )],
        );
        let err = edge
            .create_connection(Request::new(create_req(
                &token,
                FixAcceptorKind::FixedIncomeQuote,
            )))
            .await
            .expect_err("an admin denied the FI-quote capability cannot stand it up");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message().contains("quote_respond"),
            "names the missing capability: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn admin_without_fi_stream_capability_is_denied() {
        let (edge, sessions) = edge_under_enforce();
        let token = token_for(
            &sessions,
            Role::Admin,
            vec![Capability::new(Action::Stream, AssetClass::FixedIncome)],
        );
        let err = edge
            .create_connection(Request::new(create_req(
                &token,
                FixAcceptorKind::FixedIncomeStream,
            )))
            .await
            .expect_err("an admin denied the FI-stream capability cannot stand it up");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(err.message().contains("stream"));
    }

    #[tokio::test]
    async fn plain_trader_is_denied_by_the_liquidity_gate() {
        let (edge, sessions) = edge_under_enforce();
        // A plain trader (no `manage_liquidity` — it is held back from the default
        // bundle) is refused by the venue-ops gate, before the dialect gate is consulted.
        let token = token_for(&sessions, Role::Trader, Vec::new());
        let err = edge
            .create_connection(Request::new(create_req(
                &token,
                FixAcceptorKind::FixedIncomeQuote,
            )))
            .await
            .expect_err("a trader without manage_liquidity cannot administer connections");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(
            err.message().contains("manage_liquidity"),
            "names the missing venue-ops capability: {}",
            err.message()
        );
    }

    #[tokio::test]
    async fn non_admin_with_manage_liquidity_creates_fi_venue() {
        let (edge, sessions) = edge_under_enforce();
        // A NON-admin trader explicitly granted `manage_liquidity·FixedIncome` (plus the
        // FI-quote dialect capability the trader bundle already holds) stands up an
        // FI-quote venue WITHOUT the admin role — the whole point of the new capability.
        let token = token_with(
            &sessions,
            Role::Trader,
            vec![Capability::new(
                Action::ManageLiquidity,
                AssetClass::FixedIncome,
            )],
            Vec::new(),
        );
        let resp = edge
            .create_connection(Request::new(create_req(
                &token,
                FixAcceptorKind::FixedIncomeQuote,
            )))
            .await
            .expect("a trader with manage_liquidity·FI + the FI-quote capability may create it");
        assert_eq!(
            resp.into_inner().connection.unwrap().kind,
            FixAcceptorKind::FixedIncomeQuote as i32
        );
    }

    #[tokio::test]
    async fn manage_liquidity_is_asset_scoped_for_connections() {
        let (edge, sessions) = edge_under_enforce();
        // `manage_liquidity·FxOptions` does NOT authorize managing an FI venue: the base
        // gate demands the venue's own asset (here FixedIncome).
        let token = token_with(
            &sessions,
            Role::Trader,
            vec![Capability::new(
                Action::ManageLiquidity,
                AssetClass::FxOptions,
            )],
            Vec::new(),
        );
        let err = edge
            .create_connection(Request::new(create_req(
                &token,
                FixAcceptorKind::FixedIncomeQuote,
            )))
            .await
            .expect_err("FX liquidity management must not authorize an FI venue");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(err.message().contains("manage_liquidity·fixed_income"));
    }

    #[tokio::test]
    async fn admin_with_grant_all_creates_options_venue_without_fi_capability() {
        // The FX-options kind imposes no FI capability beyond administration.
        let (edge, sessions) = edge_under_enforce();
        let token = token_for(
            &sessions,
            Role::Admin,
            vec![
                Capability::new(Action::QuoteRespond, AssetClass::FixedIncome),
                Capability::new(Action::Stream, AssetClass::FixedIncome),
            ],
        );
        edge.create_connection(Request::new(create_req(&token, FixAcceptorKind::Options)))
            .await
            .expect("an FX-options venue needs no FI capability");
    }
}
