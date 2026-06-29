//! The **entitlements trust boundary**: the authorization decision every
//! entitlement-gated RPC passes, plus the per-decision audit record
//! (`docs/RISK-HIERARCHY.md` §4).
//!
//! # The trust model, stated honestly
//!
//! * **Transport-level authentication is the deployment environment's job.**
//!   The wire [`EntitlementPrincipal`] is a client-asserted grant/deny rule set
//!   — the contract carries no identity, and this repo does not pretend to
//!   authenticate one. Binding an assertion to a real caller (mTLS, an
//!   authenticating gateway that injects/validates the principal) is ENV /
//!   deploy configuration, named here rather than faked.
//! * **What the repo enforces is the authorization DECISION boundary.** Every
//!   entitlement-gated service — the four `RiskService` RPCs (`ListPositions`,
//!   `AggregateRisk`, `DrillRisk`, `LimitStatus`), over gRPC **and** the WS
//!   mirror, which dispatches onto the same trait methods — calls
//!   [`authorize`] before serving. A request that asserts **no** principal is
//!   **denied by default** with a typed `unauthenticated` error
//!   ([`AccessMode::Enforce`], the production posture and the [`Default`]).
//!   A malformed assertion is denied with `invalid_argument`, never partially
//!   honored.
//! * **Permissive dev-mode is explicit and loud, never silent.** The historical
//!   absent-⇒-grant-all behaviour survives only as [`AccessMode::Permissive`],
//!   an opt-in flag the demo edge (`examples/demo_edge.rs`) enables with a
//!   startup banner — the production boot (`src/main.rs` / `Edge::start*`)
//!   never flips it. Even then, every absent-principal admission is audited
//!   per decision ([`AccessReason::PermissiveAbsent`]), so a permissive edge is
//!   visible in the security log on every request, not just at startup. A
//!   permissive edge never fabricates a wire principal: a distributed fleet of
//!   enforcing backends will still deny a forwarded absent assertion.
//! * **Every decision is audited — allow and deny.** [`authorize`] emits one
//!   structured [`AccessAudit`] record per decision with
//!   principal / resource / decision / reason (+ the request correlation id),
//!   under the `security` log class ([`LogClass::Security`]) through the same
//!   structured-event path `AuditRecord::emit` uses. The decision runs on the
//!   tokio async edge — never the pinned zero-alloc pricing core — so the
//!   emission costs the hot path nothing.
//!
//! # Where the mode lives
//!
//! The resolved [`AccessMode`] is carried by the shared
//! [`PositionStore`](super::risk::store::PositionStore) (the live book the
//! boundary guards), as a lock-free atomic read per request: the gRPC server,
//! the WS mirror and a federating frontend all read **one** coherent policy,
//! and a dev edge can flip it explicitly between binding its listeners and
//! marking itself ready.

// `tonic::Status` is the contract's typed error; its size is the wire library's
// choice (the same allowance every service module carries).
#![allow(clippy::result_large_err)]

use celnet_entitlements::{
    AccessDecision, AccessMode, AccessReason, Action, AssetClass, Capability,
};
use celnet_observability::LogClass;
use celnet_proto::EntitlementPrincipal;
use tonic::Status;

use super::risk::convert;
use super::sessions::{AuthenticatedUser, SessionRegistry};

/// Resolve an [`AccessMode`] from the `CELNET_ACCESS_MODE` environment knob,
/// falling back to `default` when the variable is absent or unrecognised.
///
/// `"permissive"` ⇒ [`AccessMode::Permissive`]; `"enforce"` ⇒
/// [`AccessMode::Enforce`]; anything else ⇒ `default`. The same
/// deploy-time-knob discipline as `CELNET_FLEET_MODE` / `CELNET_DEMO_LPS`:
/// the env is read in one place, and explicit setters
/// ([`super::risk::store::PositionStore::set_access_mode`]) stay race-free for
/// tests. Only the demo edge calls this with a permissive default; the
/// production boot never reads this knob, so a production edge is always
/// [`AccessMode::Enforce`].
#[must_use]
pub fn mode_from_env_or(default: AccessMode) -> AccessMode {
    match std::env::var("CELNET_ACCESS_MODE").as_deref() {
        Ok("permissive") => AccessMode::Permissive,
        Ok("enforce") => AccessMode::Enforce,
        _ => default,
    }
}

/// One structured per-decision audit record: who asked (`principal`, as the
/// asserted rule-set shape — the contract carries no identity), for what
/// (`resource`), what was decided (`decision`) and why (`reason`), correlated
/// to the request. Built and emitted on the async edge only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessAudit {
    /// The entitlement-gated resource, e.g. `"RiskService/AggregateRisk"`.
    pub resource: &'static str,
    /// The asserted principal's shape: `"absent"`, `"grant-all(+N denies)"` or
    /// `"scoped(N grants, M denies)"`. Deliberately shape-only — the wire
    /// principal asserts rules, not an identity (module docs), and the audit
    /// record never invents one.
    pub principal: String,
    /// The decision taken.
    pub decision: AccessDecision,
    /// The typed reason behind the decision.
    pub reason: AccessReason,
    /// The client correlation id, when the request carried one.
    pub correlation_id: Option<u64>,
}

impl AccessAudit {
    /// Emit this record as one structured `tracing` event under the `security`
    /// log class — the same emission path the lossless trade-lifecycle
    /// `AuditRecord::emit` mirror uses, flowing through whatever JSON
    /// subscriber the edge installed. Called on the async edge for **every**
    /// decision, allow and deny alike.
    pub fn emit(&self) {
        tracing::info!(
            class = LogClass::Security.label(),
            audit = "entitlement_decision",
            resource = self.resource,
            principal = %self.principal,
            decision = self.decision.label(),
            reason = self.reason.label(),
            correlation_id = self.correlation_id,
            "entitlement decision"
        );
    }
}

/// The shape label of an asserted (or absent) wire principal for the audit
/// record: `"absent"`, `"grant-all(+N denies)"`, or `"scoped(N grants, M denies)"`.
#[must_use]
fn principal_label(principal: Option<&EntitlementPrincipal>) -> String {
    match principal {
        None => "absent".to_owned(),
        Some(p) if p.grant_all => format!("grant-all(+{} denies)", p.denies.len()),
        Some(p) => format!(
            "scoped({} grants, {} denies)",
            p.grants.len(),
            p.denies.len()
        ),
    }
}

/// **The authorization decision boundary** every entitlement-gated RPC passes
/// before serving (module docs). Decides allow/deny for the request's asserted
/// principal under `mode`, **emits one audit record for every decision**, and
/// returns the typed error on deny:
///
/// * absent principal + [`AccessMode::Enforce`] ⇒ deny,
///   [`Status::unauthenticated`] (the deny-by-default inversion);
/// * absent principal + [`AccessMode::Permissive`] ⇒ allow (the explicit,
///   audited dev-mode grant-all substitution — the post-boundary mapping in
///   [`convert::principal_of`] then applies grant-all exactly as before);
/// * asserted, well-formed principal ⇒ allow (honored as asserted; the
///   assertion-to-identity binding is the transport/deploy layer's job);
/// * asserted, malformed principal ⇒ deny, [`Status::invalid_argument`]
///   (rejected loudly at the boundary, never partially honored).
///
/// # Errors
/// [`Status::unauthenticated`] for an absent principal under
/// [`AccessMode::Enforce`]; [`Status::invalid_argument`] for a malformed
/// asserted principal.
pub fn authorize(
    mode: AccessMode,
    principal: Option<&EntitlementPrincipal>,
    resource: &'static str,
    correlation_id: Option<u64>,
) -> Result<(), Status> {
    let audit = |reason: AccessReason| {
        AccessAudit {
            resource,
            principal: principal_label(principal),
            decision: reason.decision(),
            reason,
            correlation_id,
        }
        .emit();
    };

    match principal {
        None => match mode {
            AccessMode::Enforce => {
                audit(AccessReason::PrincipalAbsent);
                Err(Status::unauthenticated(format!(
                    "{resource}: no entitlement principal asserted — denied by default \
                     (assert a principal, or run a dev edge with the explicit permissive \
                     access mode)"
                )))
            }
            AccessMode::Permissive => {
                audit(AccessReason::PermissiveAbsent);
                Ok(())
            }
        },
        Some(p) => match convert::principal_of(Some(p)) {
            Ok(_) => {
                audit(AccessReason::PrincipalAsserted);
                Ok(())
            }
            Err(status) => {
                audit(AccessReason::MalformedPrincipal);
                Err(status)
            }
        },
    }
}

/// The authority an entitlement-gated RPC demands of its resolved caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredAuthority {
    /// Any admitted caller may serve — the read RPCs (`RiskService`'s four reads,
    /// `FixAdminService.ListConnections` / `ListMessages`). The no-session path
    /// keeps honoring the asserted principal under the access mode.
    ReadAny,
    /// The caller must be an **administrator** — the mutating `FixAdminService`
    /// RPCs (`CreateConnection` / `UpdateConnection` / `DeleteConnection` /
    /// `SetEnabled`). Under [`AccessMode::Enforce`] this requires a real admin
    /// session; permissive dev-mode still admits an absent caller (audited).
    Admin,
    /// The caller must hold a specific **action capability** on an asset class —
    /// the gate for the quote / respond / stream / **execute** / book paths
    /// (`docs/plan/PERMISSIONS-ADMINISTRATION-REQUIREMENT.md` §4). Like
    /// [`RequiredAuthority::Admin`] it requires an authenticated session under
    /// [`AccessMode::Enforce`] (an asserted body principal cannot self-grant a
    /// capability); permissive dev-mode still admits an absent caller (audited).
    /// Desk-scope narrowing of the *target resource* is applied separately by the
    /// resource handler via [`ResolvedCaller::desk_scope`].
    Capability(Action, AssetClass),
}

/// What set of **desk-owned** resources a resolved caller may see. Desk ownership
/// (a FIX connection owned by a desk; a trader sees only their desk's RFQ traffic)
/// is enforced by [`FixAdminService`](super::fix_admin) reading this off the
/// resolved caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeskScope {
    /// Full visibility — an **admin** session, or the no-session legacy/demo path
    /// (which keeps the historical "sees everything" behaviour so the permissive
    /// demo edge and the principal-only tests are unchanged).
    All,
    /// Only resources owned by **this** desk — a trader session bound to a desk.
    Desk(String),
    /// Only **unowned** ("house") resources — a trader session with no desk
    /// assigned sees only connections whose owning desk is empty.
    Deskless,
}

impl DeskScope {
    /// Whether a resource owned by `owner_desk` (empty ⇒ unowned) is visible
    /// under this scope.
    #[must_use]
    pub fn allows(&self, owner_desk: &str) -> bool {
        match self {
            DeskScope::All => true,
            DeskScope::Desk(d) => owner_desk == d,
            DeskScope::Deskless => owner_desk.is_empty(),
        }
    }

    /// Whether this scope sees everything (the fast path that skips any filtering).
    #[must_use]
    pub fn is_all(&self) -> bool {
        matches!(self, DeskScope::All)
    }
}

/// The resolved caller of a gated RPC: the server-validated session identity
/// (when a valid token accompanied the request) plus the effective entitlement
/// principal the post-boundary aggregation prunes by.
///
/// Built by [`resolve_caller`] and consumed by [`authorize_caller`]. A present
/// [`ResolvedCaller::user`] means the SERVER authenticated the caller — the
/// strongest authority in the model; an absent one is the legacy mode-gated
/// principal path, unchanged.
#[derive(Debug)]
pub struct ResolvedCaller {
    /// The authenticated user, when the request carried a valid session token.
    user: Option<AuthenticatedUser>,
    /// The effective entitlement principal for data pruning (the client-asserted
    /// one). Session-derived **desk** scoping is exposed separately via
    /// [`ResolvedCaller::desk_scope`] (consumed by the FIX-admin reads); the
    /// principal remains the risk-aggregation pruning rule-set.
    principal: Option<EntitlementPrincipal>,
}

impl ResolvedCaller {
    /// The **anonymous** caller: no authenticated user, no asserted principal — the
    /// starting state of a freshly-opened stream session before any `StreamAuth`
    /// frame. Under [`AccessMode::Enforce`] this caller is denied
    /// [`Status::unauthenticated`] by [`authorize_caller`]; under
    /// [`AccessMode::Permissive`] it is admitted (the legacy/demo path). Built here
    /// rather than by reaching into the private fields, so the streaming edge can
    /// seed a session without widening the struct's surface.
    #[must_use]
    pub fn anonymous() -> Self {
        Self {
            user: None,
            principal: None,
        }
    }

    /// The authenticated user, if a valid session token accompanied the request.
    #[must_use]
    pub fn user(&self) -> Option<&AuthenticatedUser> {
        self.user.as_ref()
    }

    /// The effective pruning principal for the post-boundary aggregation path
    /// ([`convert::principal_of`]).
    #[must_use]
    pub fn principal(&self) -> Option<&EntitlementPrincipal> {
        self.principal.as_ref()
    }

    /// The desk-visibility scope this caller sees over desk-owned resources:
    ///
    /// * an **admin** session, or **no** session (the legacy/demo path) ⇒
    ///   [`DeskScope::All`] — full visibility, so the permissive demo edge and the
    ///   principal-only tests keep seeing every connection;
    /// * a **trader** session bound to a desk ⇒ [`DeskScope::Desk`] of that desk;
    /// * a **trader** session with no desk ⇒ [`DeskScope::Deskless`] (only
    ///   unowned "house" connections).
    #[must_use]
    pub fn desk_scope(&self) -> DeskScope {
        match self.user.as_ref() {
            None => DeskScope::All,
            Some(u) if u.is_admin() => DeskScope::All,
            Some(u) => match &u.desk_id {
                Some(desk) => DeskScope::Desk(desk.clone()),
                None => DeskScope::Deskless,
            },
        }
    }
}

/// Resolve the caller of a gated RPC from its `session_token` and asserted
/// `principal`.
///
/// A present, non-empty token is validated against `sessions`: an unknown or
/// expired token is rejected with [`Status::unauthenticated`] — a bad credential
/// is never silently downgraded to the anonymous path. An absent/empty token
/// yields an unauthenticated caller carrying the asserted principal (the legacy
/// mode-gated path). The asserted principal rides along either way, for the
/// downstream pruning the boundary itself does not perform.
///
/// # Errors
/// [`Status::unauthenticated`] when a token is presented but is invalid/expired.
pub fn resolve_caller(
    sessions: &SessionRegistry,
    token: Option<&str>,
    asserted: Option<EntitlementPrincipal>,
) -> Result<ResolvedCaller, Status> {
    match token.map(str::trim).filter(|t| !t.is_empty()) {
        None => Ok(ResolvedCaller {
            user: None,
            principal: asserted,
        }),
        Some(t) => match sessions.validate(t) {
            Some(user) => Ok(ResolvedCaller {
                user: Some(user),
                principal: asserted,
            }),
            None => Err(Status::unauthenticated(
                "invalid or expired session token — re-authenticate via AuthService.Login",
            )),
        },
    }
}

/// **The session-aware authorization boundary** every entitlement-gated RPC
/// passes once a [`ResolvedCaller`] is in hand. It layers server-validated
/// sessions over the existing [`authorize`] principal/mode decision:
///
/// * **A valid session authenticates the caller** (the strongest authority).
///   For [`RequiredAuthority::Admin`] the session's role is checked: a non-admin
///   is denied [`Status::permission_denied`]
///   ([`AccessReason::SessionInsufficientRole`]); otherwise the call is allowed
///   ([`AccessReason::SessionAuthenticated`]). Either way one audit record is
///   emitted, keyed on the authenticated identity — never a fabricated one.
/// * **No session ⇒ the legacy mode-gated path.** [`RequiredAuthority::ReadAny`]
///   delegates verbatim to [`authorize`] (asserted principal honored, absent
///   denied under enforce / admitted under permissive, malformed rejected).
///   [`RequiredAuthority::Admin`] requires a real admin session under
///   [`AccessMode::Enforce`] (absent ⇒ [`Status::unauthenticated`]); permissive
///   dev-mode still admits the absent caller, audited, so the demo edge is
///   unchanged.
///
/// Every decision — allow and deny — emits exactly one [`AccessAudit`] record,
/// on the async edge only (the pinned pricing core is untouched).
///
/// # Errors
/// [`Status::permission_denied`] for an authenticated non-admin on an
/// [`RequiredAuthority::Admin`] resource; [`Status::unauthenticated`] for an
/// absent caller on an admin resource under enforce; plus any error
/// [`authorize`] returns on the no-session read path.
pub fn authorize_caller(
    mode: AccessMode,
    caller: &ResolvedCaller,
    resource: &'static str,
    required: RequiredAuthority,
    correlation_id: Option<u64>,
) -> Result<(), Status> {
    // Authenticated-session path: the server validated WHO the caller is.
    if let Some(user) = caller.user.as_ref() {
        let label = format!("session({}/{})", user.email, user.role.as_str());
        let emit = |reason: AccessReason| {
            AccessAudit {
                resource,
                principal: label.clone(),
                decision: reason.decision(),
                reason,
                correlation_id,
            }
            .emit();
        };
        match required {
            RequiredAuthority::Admin if !user.is_admin() => {
                emit(AccessReason::SessionInsufficientRole);
                return Err(Status::permission_denied(format!(
                    "{resource}: requires an administrator session (caller is not an admin)"
                )));
            }
            RequiredAuthority::Capability(action, asset)
                if !user.capabilities().allows(Capability::new(action, asset)) =>
            {
                emit(AccessReason::SessionMissingCapability);
                return Err(Status::permission_denied(format!(
                    "{resource}: caller lacks the {}·{} capability",
                    action.label(),
                    asset.label()
                )));
            }
            _ => {}
        }
        emit(AccessReason::SessionAuthenticated);
        return Ok(());
    }

    // No session: the legacy mode-gated path.
    match required {
        RequiredAuthority::ReadAny => {
            authorize(mode, caller.principal.as_ref(), resource, correlation_id)
        }
        // Both the admin gate and the action-capability gate require a
        // server-authenticated session: an asserted body principal cannot
        // self-grant either. Permissive dev-mode admits the absent caller
        // (audited); enforce denies it unauthenticated.
        RequiredAuthority::Admin | RequiredAuthority::Capability(..) => {
            let emit = |reason: AccessReason| {
                AccessAudit {
                    resource,
                    principal: principal_label(caller.principal.as_ref()),
                    decision: reason.decision(),
                    reason,
                    correlation_id,
                }
                .emit();
            };
            let need = match required {
                RequiredAuthority::Capability(action, asset) => {
                    format!("the {}·{} capability", action.label(), asset.label())
                }
                _ => "an administrator session".to_owned(),
            };
            match mode {
                AccessMode::Permissive => {
                    emit(AccessReason::PermissiveAbsent);
                    Ok(())
                }
                AccessMode::Enforce => {
                    emit(AccessReason::PrincipalAbsent);
                    Err(Status::unauthenticated(format!(
                        "{resource}: requires {need} — no authenticated session presented \
                         (log in via AuthService.Login, or run a dev edge in permissive mode)"
                    )))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use celnet_proto::{EntitlementRule, RiskScope};

    /// Enforce + absent principal ⇒ the typed deny-by-default error.
    #[test]
    fn enforce_denies_absent_principal_unauthenticated() {
        let err = authorize(AccessMode::Enforce, None, "RiskService/AggregateRisk", None)
            .expect_err("absent principal must be denied by default");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
        assert!(
            err.message().contains("denied by default"),
            "the error names the boundary: {}",
            err.message()
        );
    }

    /// Permissive dev-mode + absent principal ⇒ the explicit, audited grant.
    #[test]
    fn permissive_allows_absent_principal() {
        authorize(
            AccessMode::Permissive,
            None,
            "RiskService/ListPositions",
            Some(7),
        )
        .expect("permissive dev-mode explicitly admits an absent principal");
    }

    /// An asserted well-formed principal is honored as asserted, in both modes.
    #[test]
    fn asserted_principal_is_allowed() {
        let asserted = EntitlementPrincipal {
            grant_all: true,
            grants: vec![],
            denies: vec![],
        };
        for mode in [AccessMode::Enforce, AccessMode::Permissive] {
            authorize(mode, Some(&asserted), "RiskService/DrillRisk", None)
                .expect("an asserted principal is honored as asserted");
        }
    }

    /// A malformed assertion (unknown dimension) is denied loudly at the
    /// boundary, never partially honored.
    #[test]
    fn malformed_principal_is_denied_invalid_argument() {
        let malformed = EntitlementPrincipal {
            grant_all: false,
            grants: vec![EntitlementRule {
                scopes: vec![RiskScope {
                    dimension: 9999, // not a RiskDimension
                    value: 1,
                }],
            }],
            denies: vec![],
        };
        let err = authorize(
            AccessMode::Enforce,
            Some(&malformed),
            "RiskService/LimitStatus",
            None,
        )
        .expect_err("a malformed principal must be rejected");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    /// The audit label captures the asserted shape (never a fabricated identity).
    #[test]
    fn principal_labels_describe_the_asserted_shape() {
        assert_eq!(principal_label(None), "absent");
        let ga = EntitlementPrincipal {
            grant_all: true,
            grants: vec![],
            denies: vec![EntitlementRule { scopes: vec![] }],
        };
        assert_eq!(principal_label(Some(&ga)), "grant-all(+1 denies)");
        let scoped = EntitlementPrincipal {
            grant_all: false,
            grants: vec![EntitlementRule { scopes: vec![] }],
            denies: vec![],
        };
        assert_eq!(principal_label(Some(&scoped)), "scoped(1 grants, 0 denies)");
    }

    // --- session-aware boundary (resolve_caller / authorize_caller) -----------

    use crate::clock::Clock;
    use crate::config::identity::Role;

    fn user(role: Role) -> AuthenticatedUser {
        AuthenticatedUser {
            user_id: "u-1".into(),
            email: "trader@celnet.com".into(),
            display_name: "Trader".into(),
            role,
            desk_id: Some("g10".into()),
            role_caps: crate::config::identity::default_trader_bundle(),
            cap_grants: Vec::new(),
            cap_denies: Vec::new(),
        }
    }

    /// An absent/empty token yields an unauthenticated caller carrying the
    /// asserted principal — never an error.
    #[test]
    fn resolve_caller_absent_token_is_anonymous() {
        let reg = SessionRegistry::new(Clock::manual(0));
        let asserted = EntitlementPrincipal {
            grant_all: true,
            grants: vec![],
            denies: vec![],
        };
        let caller = resolve_caller(&reg, None, Some(asserted.clone()))
            .expect("absent token resolves to the anonymous path");
        assert!(caller.user().is_none());
        assert_eq!(caller.principal(), Some(&asserted));
        // A whitespace-only token is treated as absent, not as a bad credential.
        let blank = resolve_caller(&reg, Some("  "), None).expect("blank token ⇒ anonymous");
        assert!(blank.user().is_none());
    }

    /// A presented-but-invalid token is rejected — never silently downgraded.
    #[test]
    fn resolve_caller_invalid_token_is_unauthenticated() {
        let reg = SessionRegistry::new(Clock::manual(0));
        let err = resolve_caller(&reg, Some("not-a-real-token"), None)
            .expect_err("an invalid token must be rejected, not downgraded");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    /// A valid token resolves to its authenticated user.
    #[test]
    fn resolve_caller_valid_token_authenticates() {
        let reg = SessionRegistry::new(Clock::manual(0));
        let token = reg.issue(user(Role::Admin)).unwrap().token;
        let caller = resolve_caller(&reg, Some(&token), None).expect("valid token authenticates");
        assert_eq!(
            caller.user().map(|u| u.email.as_str()),
            Some("trader@celnet.com")
        );
        assert!(caller.user().is_some_and(AuthenticatedUser::is_admin));
    }

    /// An authenticated admin passes an Admin-gated resource even under enforce.
    #[test]
    fn authorize_caller_admin_session_allows_admin_resource() {
        let reg = SessionRegistry::new(Clock::manual(0));
        let token = reg.issue(user(Role::Admin)).unwrap().token;
        let caller = resolve_caller(&reg, Some(&token), None).unwrap();
        authorize_caller(
            AccessMode::Enforce,
            &caller,
            "FixAdminService/CreateConnection",
            RequiredAuthority::Admin,
            Some(1),
        )
        .expect("an admin session authorizes an admin RPC under enforce");
    }

    /// An authenticated trader is denied an Admin-gated resource — role-gated
    /// `permission_denied`, distinct from an unauthenticated absence.
    #[test]
    fn authorize_caller_trader_session_denied_admin_resource() {
        let reg = SessionRegistry::new(Clock::manual(0));
        let token = reg.issue(user(Role::Trader)).unwrap().token;
        let caller = resolve_caller(&reg, Some(&token), None).unwrap();
        let err = authorize_caller(
            AccessMode::Enforce,
            &caller,
            "FixAdminService/DeleteConnection",
            RequiredAuthority::Admin,
            None,
        )
        .expect_err("a trader session must not pass an admin RPC");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    /// An authenticated trader passes a ReadAny resource (authentication suffices).
    #[test]
    fn authorize_caller_trader_session_allows_read() {
        let reg = SessionRegistry::new(Clock::manual(0));
        let token = reg.issue(user(Role::Trader)).unwrap().token;
        let caller = resolve_caller(&reg, Some(&token), None).unwrap();
        authorize_caller(
            AccessMode::Enforce,
            &caller,
            "RiskService/ListPositions",
            RequiredAuthority::ReadAny,
            None,
        )
        .expect("any authenticated caller may read");
    }

    /// No session + enforce + an Admin resource ⇒ unauthenticated (a real admin
    /// session is required; an asserted principal cannot self-grant admin).
    #[test]
    fn authorize_caller_no_session_enforce_admin_denied() {
        let asserted = EntitlementPrincipal {
            grant_all: true,
            grants: vec![],
            denies: vec![],
        };
        let caller = ResolvedCaller {
            user: None,
            principal: Some(asserted),
        };
        let err = authorize_caller(
            AccessMode::Enforce,
            &caller,
            "FixAdminService/CreateConnection",
            RequiredAuthority::Admin,
            None,
        )
        .expect_err("enforce admin RPC needs a real admin session");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    /// No session + permissive + an Admin resource ⇒ admitted (the demo edge is
    /// unchanged).
    #[test]
    fn authorize_caller_no_session_permissive_admin_allowed() {
        let caller = ResolvedCaller {
            user: None,
            principal: None,
        };
        authorize_caller(
            AccessMode::Permissive,
            &caller,
            "FixAdminService/CreateConnection",
            RequiredAuthority::Admin,
            None,
        )
        .expect("permissive dev-mode admits the absent admin caller");
    }

    /// No session, ReadAny: the boundary delegates verbatim to `authorize` — an
    /// absent principal under enforce is still denied by default.
    #[test]
    fn authorize_caller_no_session_read_delegates_to_authorize() {
        let caller = ResolvedCaller {
            user: None,
            principal: None,
        };
        let err = authorize_caller(
            AccessMode::Enforce,
            &caller,
            "RiskService/ListPositions",
            RequiredAuthority::ReadAny,
            None,
        )
        .expect_err("no session + no principal + enforce ⇒ deny by default");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    // --- action-capability gate -----------------------------------------------

    /// A trader session holds the deal capability → an `Execute` gate passes.
    #[test]
    fn authorize_caller_trader_session_allows_held_capability() {
        let reg = SessionRegistry::new(Clock::manual(0));
        let token = reg.issue(user(Role::Trader)).unwrap().token;
        let caller = resolve_caller(&reg, Some(&token), None).unwrap();
        authorize_caller(
            AccessMode::Enforce,
            &caller,
            "QuoteService/AcceptQuote",
            RequiredAuthority::Capability(Action::Execute, AssetClass::FxOptions),
            None,
        )
        .expect("a trader holds Execute on FX options");
    }

    /// A trader session does NOT hold `Administer` → a capability gate for it is a
    /// capability-typed `permission_denied`, distinct from the role-gated admin path.
    #[test]
    fn authorize_caller_trader_session_denied_missing_capability() {
        let reg = SessionRegistry::new(Clock::manual(0));
        let token = reg.issue(user(Role::Trader)).unwrap().token;
        let caller = resolve_caller(&reg, Some(&token), None).unwrap();
        let err = authorize_caller(
            AccessMode::Enforce,
            &caller,
            "IdentityAdminService/CreateUser",
            RequiredAuthority::Capability(Action::Administer, AssetClass::FxOptions),
            None,
        )
        .expect_err("a trader lacks the Administer capability");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert!(err.message().contains("administer"), "{}", err.message());
    }

    /// No session + enforce + a capability gate ⇒ unauthenticated (a body principal
    /// cannot self-grant a capability — the finding #3 widening guard).
    #[test]
    fn authorize_caller_no_session_enforce_capability_denied() {
        let asserted = EntitlementPrincipal {
            grant_all: true,
            grants: vec![],
            denies: vec![],
        };
        let caller = ResolvedCaller {
            user: None,
            principal: Some(asserted),
        };
        let err = authorize_caller(
            AccessMode::Enforce,
            &caller,
            "RfqDeskService/AcceptDeskQuote",
            RequiredAuthority::Capability(Action::Execute, AssetClass::FixedIncome),
            None,
        )
        .expect_err("an asserted grant-all body principal cannot self-grant Execute");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    /// No session + permissive + a capability gate ⇒ admitted (the demo edge is
    /// unchanged; the admission is audited).
    #[test]
    fn authorize_caller_no_session_permissive_capability_allowed() {
        let caller = ResolvedCaller {
            user: None,
            principal: None,
        };
        authorize_caller(
            AccessMode::Permissive,
            &caller,
            "RfqDeskService/AcceptDeskQuote",
            RequiredAuthority::Capability(Action::Execute, AssetClass::FixedIncome),
            None,
        )
        .expect("permissive dev-mode admits the absent capability caller");
    }

    // --- desk-visibility scope ------------------------------------------------

    /// An admin session sees every connection (full visibility).
    #[test]
    fn desk_scope_admin_sees_all() {
        let reg = SessionRegistry::new(Clock::manual(0));
        let token = reg.issue(user(Role::Admin)).unwrap().token;
        let caller = resolve_caller(&reg, Some(&token), None).unwrap();
        let scope = caller.desk_scope();
        assert_eq!(scope, DeskScope::All);
        assert!(scope.allows("g10") && scope.allows("") && scope.is_all());
    }

    /// A no-session caller keeps full visibility (the legacy/demo path).
    #[test]
    fn desk_scope_no_session_sees_all() {
        let caller = ResolvedCaller {
            user: None,
            principal: None,
        };
        assert_eq!(caller.desk_scope(), DeskScope::All);
    }

    /// A trader bound to a desk sees only that desk's connections, not other
    /// desks' nor the unowned house connections.
    #[test]
    fn desk_scope_trader_sees_only_their_desk() {
        let reg = SessionRegistry::new(Clock::manual(0));
        // `user(Role::Trader)` belongs to desk `g10`.
        let token = reg.issue(user(Role::Trader)).unwrap().token;
        let caller = resolve_caller(&reg, Some(&token), None).unwrap();
        let scope = caller.desk_scope();
        assert_eq!(scope, DeskScope::Desk("g10".into()));
        assert!(scope.allows("g10"));
        assert!(!scope.allows("em")); // another desk
        assert!(!scope.allows("")); // unowned/house
        assert!(!scope.is_all());
    }

    /// A trader with no desk sees only unowned ("house") connections.
    #[test]
    fn desk_scope_deskless_trader_sees_only_unowned() {
        let reg = SessionRegistry::new(Clock::manual(0));
        let deskless = AuthenticatedUser {
            desk_id: None,
            ..user(Role::Trader)
        };
        let token = reg.issue(deskless).unwrap().token;
        let caller = resolve_caller(&reg, Some(&token), None).unwrap();
        let scope = caller.desk_scope();
        assert_eq!(scope, DeskScope::Deskless);
        assert!(scope.allows(""));
        assert!(!scope.allows("g10"));
    }
}
