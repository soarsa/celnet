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

use celnet_entitlements::{AccessDecision, AccessMode, AccessReason};
use celnet_observability::LogClass;
use celnet_proto::EntitlementPrincipal;
use tonic::Status;

use super::risk::convert;

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
}
