//! The trust-boundary **decision vocabulary**: the enforcement mode, the
//! per-request access decision, and the typed reason behind it
//! (`docs/RISK-HIERARCHY.md` §4).
//!
//! # Why this lives here
//!
//! The entitlement *predicate* ([`crate::Principal::admits`]) answers "may this
//! principal see this fact". The trust **boundary** asks the prior question:
//! "is there an authorizable principal at all, and what do we do when there is
//! none?" This module is the pure vocabulary of that boundary — the server's
//! service edge makes the decision, emits the audit record, and returns the
//! typed error; the types here keep that decision **deterministic, enumerable
//! and auditable** rather than an ad-hoc boolean.
//!
//! # The trust model (stated honestly)
//!
//! * The wire principal is **client-asserted**: the contract carries the grant /
//!   deny rule set, not an authenticated identity. **Binding an assertion to an
//!   authenticated caller (mTLS, an authenticating gateway) is the
//!   transport/deployment layer's job** — it is environment configuration, not
//!   in-repo code, and nothing here pretends otherwise.
//! * What the repo *does* own is the **authorization decision boundary**: a
//!   request with **no** principal assertion is **denied by default**
//!   ([`AccessReason::PrincipalAbsent`]) on every entitlement-gated service.
//!   The historical absent-⇒-grant-all behaviour survives only as an
//!   **explicit, loud, dev-mode opt-in** ([`AccessMode::Permissive`]) — a demo
//!   affordance, never silent production behaviour.
//! * Every decision — allow **and** deny — is audited by the server with
//!   principal / resource / decision / reason (the §4 "every entitlement
//!   decision logged" contract).
//!
//! This module stays pure (no IO, no clock, no logger), like the rest of the
//! crate: the server records what these types describe.

/// How the service edge treats a request that asserts **no** entitlement
/// principal. This is the explicit configuration knob behind the deny-by-default
/// inversion — never an implicit behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AccessMode {
    /// **Deny by default** (the production posture and the `Default`): a request
    /// with no asserted principal is refused with a typed error; nothing is
    /// served. An asserted principal (including an asserted grant-all) is
    /// honored as asserted — the assertion-to-identity binding is the
    /// transport/deploy layer's job (module docs).
    #[default]
    Enforce,
    /// **Explicit permissive dev-mode**: an absent principal is admitted as
    /// grant-all, exactly the pre-inversion demo behaviour — but now as a loud,
    /// opt-in flag (the demo edge enables it and prints a startup banner; the
    /// production boot never does). Every such admission is still audited with
    /// [`AccessReason::PermissiveAbsent`], so a permissive deployment is visible
    /// per decision, not just at startup.
    Permissive,
}

impl AccessMode {
    /// Stable snake_case label for audit/log fields.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            AccessMode::Enforce => "enforce",
            AccessMode::Permissive => "permissive",
        }
    }

    /// Whether this is the explicit permissive dev-mode.
    #[must_use]
    pub const fn is_permissive(self) -> bool {
        matches!(self, AccessMode::Permissive)
    }
}

/// The outcome of one authorization decision at the trust boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessDecision {
    /// The request proceeds (with the asserted — or, in permissive dev-mode,
    /// the substituted grant-all — principal applied as the pruning predicate).
    Allow,
    /// The request is refused with a typed error; nothing is served.
    Deny,
}

impl AccessDecision {
    /// Stable snake_case label for audit/log fields.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            AccessDecision::Allow => "allow",
            AccessDecision::Deny => "deny",
        }
    }
}

/// The typed reason behind an [`AccessDecision`] — the auditable "why", never a
/// free-form string. Each reason maps to exactly one decision
/// ([`AccessReason::decision`]), so the audit stream cannot carry a
/// contradictory (reason, decision) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessReason {
    /// Allow: the caller asserted a well-formed principal; it is applied as the
    /// pre-aggregation pruning predicate. (The assertion-to-identity binding is
    /// the transport/deploy layer's responsibility — module docs.)
    PrincipalAsserted,
    /// Allow: **no** principal was asserted and the edge runs in the explicit
    /// permissive dev-mode, so grant-all is substituted. Audited per decision so
    /// a permissive edge is visible in the security log, not silent.
    PermissiveAbsent,
    /// Deny: **no** principal was asserted and the edge enforces the
    /// deny-by-default boundary.
    PrincipalAbsent,
    /// Deny: a principal was asserted but its rule set is malformed (e.g. an
    /// unknown dimension); rejected loudly rather than partially honored.
    MalformedPrincipal,
    /// Allow: the request carried a **server-validated session token** (issued by
    /// `AuthService.Login`, resolved against the live session registry). The
    /// caller's identity was authenticated by the server, not self-asserted — the
    /// strongest admit reason in the audit stream.
    SessionAuthenticated,
    /// Deny: a valid session authenticated the caller, but the resource requires
    /// administrator authority the caller's role does not hold (a role-gated
    /// `permission_denied`, distinct from an unauthenticated absence).
    SessionInsufficientRole,
}

impl AccessReason {
    /// Stable snake_case label for audit/log fields.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            AccessReason::PrincipalAsserted => "principal_asserted",
            AccessReason::PermissiveAbsent => "permissive_dev_mode_absent_principal",
            AccessReason::PrincipalAbsent => "principal_absent",
            AccessReason::MalformedPrincipal => "malformed_principal",
            AccessReason::SessionAuthenticated => "session_authenticated",
            AccessReason::SessionInsufficientRole => "session_insufficient_role",
        }
    }

    /// The one decision this reason justifies (the invariant that keeps the
    /// audit stream free of contradictory pairs).
    #[must_use]
    pub const fn decision(self) -> AccessDecision {
        match self {
            AccessReason::PrincipalAsserted
            | AccessReason::PermissiveAbsent
            | AccessReason::SessionAuthenticated => AccessDecision::Allow,
            AccessReason::PrincipalAbsent
            | AccessReason::MalformedPrincipal
            | AccessReason::SessionInsufficientRole => AccessDecision::Deny,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default mode is the deny-by-default production posture — permissive
    /// dev-mode can only ever be an explicit opt-in.
    #[test]
    fn default_mode_is_enforce() {
        assert_eq!(AccessMode::default(), AccessMode::Enforce);
        assert!(!AccessMode::default().is_permissive());
    }

    /// Every reason maps to exactly one decision, and the mapping matches the
    /// documented semantics (asserted/permissive ⇒ allow; absent/malformed ⇒ deny).
    #[test]
    fn reason_determines_decision() {
        assert_eq!(
            AccessReason::PrincipalAsserted.decision(),
            AccessDecision::Allow
        );
        assert_eq!(
            AccessReason::PermissiveAbsent.decision(),
            AccessDecision::Allow
        );
        assert_eq!(
            AccessReason::PrincipalAbsent.decision(),
            AccessDecision::Deny
        );
        assert_eq!(
            AccessReason::MalformedPrincipal.decision(),
            AccessDecision::Deny
        );
        assert_eq!(
            AccessReason::SessionAuthenticated.decision(),
            AccessDecision::Allow
        );
        assert_eq!(
            AccessReason::SessionInsufficientRole.decision(),
            AccessDecision::Deny
        );
    }

    /// Labels are stable, distinct snake_case identifiers (audit fields key on
    /// them; a collision would alias two different reasons in the log).
    #[test]
    fn labels_are_distinct() {
        let reasons = [
            AccessReason::PrincipalAsserted.label(),
            AccessReason::PermissiveAbsent.label(),
            AccessReason::PrincipalAbsent.label(),
            AccessReason::MalformedPrincipal.label(),
            AccessReason::SessionAuthenticated.label(),
            AccessReason::SessionInsufficientRole.label(),
        ];
        let mut sorted = reasons.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), reasons.len());
        assert_eq!(AccessDecision::Allow.label(), "allow");
        assert_eq!(AccessDecision::Deny.label(), "deny");
        assert_eq!(AccessMode::Enforce.label(), "enforce");
        assert_eq!(AccessMode::Permissive.label(), "permissive");
    }
}
