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
use crate::services::access::{RequiredAuthority, authorize_caller, resolve_caller};
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
        let connections = self
            .registry
            .list()
            .await
            .iter()
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
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "FixAdminService/CreateConnection",
            RequiredAuthority::Admin,
            req.correlation_id,
        )?;
        let spec = req
            .spec
            .ok_or_else(|| Status::invalid_argument("create: missing connection spec"))?;
        let def = def_from_spec(&spec, None)?;
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
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "FixAdminService/UpdateConnection",
            RequiredAuthority::Admin,
            req.correlation_id,
        )?;
        if req.id.trim().is_empty() {
            return Err(Status::invalid_argument("update: missing connection id"));
        }
        let spec = req
            .spec
            .ok_or_else(|| Status::invalid_argument("update: missing connection spec"))?;
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
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "FixAdminService/DeleteConnection",
            RequiredAuthority::Admin,
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
        authorize_caller(
            self.store.access_mode(),
            &caller,
            "FixAdminService/SetEnabled",
            RequiredAuthority::Admin,
            req.correlation_id,
        )?;
        if req.id.trim().is_empty() {
            return Err(Status::invalid_argument(
                "set_enabled: missing connection id",
            ));
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
        let (events, latest_seq) = self.monitor.since(connection_id, req.after_seq, limit);
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
    Ok(FixConnectionDef {
        id,
        name: spec.name.clone(),
        kind: kind_from_wire(spec.kind)?,
        bind_addr: spec.bind_addr.clone(),
        sender_comp_id: spec.sender_comp_id.clone(),
        target_comp_id: spec.target_comp_id.clone(),
        enabled: spec.enabled,
    })
}

/// The wire enum value for a domain [`AcceptorKind`].
fn kind_to_wire(kind: AcceptorKind) -> i32 {
    match kind {
        AcceptorKind::Options => FixAcceptorKind::Options as i32,
    }
}

/// Resolve a wire enum value to a domain [`AcceptorKind`].
///
/// # Errors
/// `invalid_argument` for an unrecognised enum value.
fn kind_from_wire(kind: i32) -> Result<AcceptorKind, Status> {
    match FixAcceptorKind::try_from(kind) {
        Ok(FixAcceptorKind::Options) => Ok(AcceptorKind::Options),
        Err(_) => Err(Status::invalid_argument(format!(
            "unrecognised FIX acceptor kind `{kind}`"
        ))),
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
        }
    }

    #[test]
    fn spec_with_explicit_id_is_preserved() {
        let def = def_from_spec(&spec("opt-1", "Bank A"), None).unwrap();
        assert_eq!(def.id, "opt-1");
        assert_eq!(def.kind, AcceptorKind::Options);
        assert!(def.enabled);
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
}
