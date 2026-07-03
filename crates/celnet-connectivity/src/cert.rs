//! Certification harness — vendor-neutral port of the venue-onboarding cert
//! workflow.
//!
//! Every connection is seeded with a checklist: six standard FIX session-level
//! checks plus exactly one role-specific app-flow check ([`crate::ConnectionRole`]),
//! plus any adapter-specific extras ([`crate::CertCheckDef`]). *Automatic* checks
//! flip `PENDING → PASSED` the first time the live session observes the mapped
//! wire event (driven by [`crate::driver`] calling [`CertLedger::record_event`]);
//! the rest are operator-marked. Promote-to-PROD gates on every **required** check
//! passing, unless the platform cert gate is disabled.
//!
//! The ledger is a plain in-memory state machine — no IO, no allocation beyond
//! the one `Vec` of checks — so it is trivially testable and safe to snapshot
//! onto the control-plane API.

use crate::descriptor::{
    CertCheckDef, ConnectionRole, EventCheckOverride, VendorAdapterSpec, WireDirection,
};
use serde::Serialize;

// Stable check keys. The ops console + audit strings key off these — keep stable.

/// Logon (35=A) round-trip completed.
pub const CHECK_LOGON: &str = "LOGON_RECEIVED";
/// At least one inbound Heartbeat (35=0) observed.
pub const CHECK_HEARTBEAT: &str = "HEARTBEAT_RECEIVED";
/// An inbound TestRequest (35=1) was serviced.
pub const CHECK_TEST_REQUEST: &str = "TEST_REQUEST_OK";
/// A clean Logout (35=5) was exchanged.
pub const CHECK_LOGOUT: &str = "LOGOUT_CLEAN";
/// A ResendRequest replay was operator-verified (manual).
pub const CHECK_RESEND: &str = "RESEND_REPLAY_VERIFIED";
/// Optional sustained-throughput soak, operator-verified (manual).
pub const CHECK_HIGH_VOLUME_SOAK: &str = "HIGH_VOLUME_SOAK";
/// ORDER app-flow: a NewOrderSingle round-tripped to an ExecutionReport (35=8).
pub const CHECK_ORDER_ER: &str = "NEW_ORDER_ER_OBSERVED";
/// PRICE app-flow: a market-data Snapshot (35=W) / Incremental (35=X) was observed.
pub const CHECK_PRICE_SUBSCRIPTION: &str = "MD_SUBSCRIPTION_OK";
/// RFQ app-flow: a QuoteRequest produced a Quote (35=S).
pub const CHECK_RFQ_FLOW: &str = "QUOTE_REQUEST_OK";
/// STP app-flow: an inbound TradeCaptureReport (35=AE) was observed.
pub const CHECK_STP_TCR: &str = "TRADE_CAPTURE_REPORT_OBSERVED";

/// Lifecycle status of one certification check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CheckStatus {
    /// Not yet satisfied.
    Pending,
    /// Satisfied — auto-passed on a wire event or operator-marked.
    Passed,
    /// Explicitly failed by an operator.
    Failed,
    /// Explicitly waived by an operator (optional checks only).
    Skipped,
}

/// One seeded certification check with its live status.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertCheck {
    /// Stable key (one of the `CHECK_*` constants or an adapter-specific key).
    pub check_key: &'static str,
    /// Short human-readable label.
    pub label: &'static str,
    /// What passing this check means.
    pub description: &'static str,
    /// Whether promote-to-PROD requires this check.
    pub required: bool,
    /// Whether an observed wire event auto-passes it.
    pub automatic: bool,
    /// The live status.
    pub status: CheckStatus,
}

/// The certification checklist for one connection.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertLedger {
    /// The connection role this checklist was seeded for.
    pub role: ConnectionRole,
    checks: Vec<CertCheck>,
    #[serde(skip)]
    overrides: &'static [EventCheckOverride],
}

impl CertLedger {
    /// Seed the full checklist for a venue adapter: standard session checks +
    /// the role's app-flow check + the adapter's extras (extras replace a
    /// standard check that shares a key, so a redundant adapter declaration is
    /// harmless — mirrors the source's upsert semantics).
    pub fn seed(spec: &VendorAdapterSpec) -> Self {
        let mut checks: Vec<CertCheck> = Vec::with_capacity(8 + spec.cert_checks.len());

        // Six standard FIX session-level checks (four auto, two manual).
        push(
            &mut checks,
            CHECK_LOGON,
            "Logon round-trip",
            "Counterparty Logon (35=A) accepted; CompIDs matched and our Logon ack flushed.",
            true,
            true,
        );
        push(
            &mut checks,
            CHECK_HEARTBEAT,
            "Heartbeat exchange",
            "At least one inbound Heartbeat (35=0) received after Logon.",
            true,
            true,
        );
        push(
            &mut checks,
            CHECK_TEST_REQUEST,
            "TestRequest serviced",
            "An inbound TestRequest (35=1) was answered with a Heartbeat carrying the TestReqID.",
            true,
            true,
        );
        push(
            &mut checks,
            CHECK_LOGOUT,
            "Clean Logout",
            "A Logout (35=5) was exchanged and acked before disconnect.",
            true,
            true,
        );
        push(
            &mut checks,
            CHECK_RESEND,
            "Resend / GapFill verified",
            "Operator confirmed a ResendRequest replay produced the expected GapFill.",
            true,
            false,
        );
        push(
            &mut checks,
            CHECK_HIGH_VOLUME_SOAK,
            "High-volume soak",
            "Optional. Sustained N msgs/sec for M minutes without sequence drift.",
            false,
            false,
        );

        // Exactly one role-specific app-flow check, auto on the matching inbound event.
        match spec.role {
            ConnectionRole::Order => push(
                &mut checks,
                CHECK_ORDER_ER,
                "NewOrderSingle → ExecutionReport",
                "A NewOrderSingle (35=D) round-tripped to an ExecutionReport (35=8).",
                true,
                true,
            ),
            ConnectionRole::Price => push(
                &mut checks,
                CHECK_PRICE_SUBSCRIPTION,
                "Market data subscription",
                "Snapshot (35=W) or incremental refresh (35=X) observed for an MDRequest (35=V).",
                true,
                true,
            ),
            ConnectionRole::Rfq => push(
                &mut checks,
                CHECK_RFQ_FLOW,
                "Quote round-trip",
                "A QuoteRequest (35=R) produced a Quote (35=S) response.",
                true,
                true,
            ),
            ConnectionRole::Stp => push(
                &mut checks,
                CHECK_STP_TCR,
                "Trade capture report observed",
                "A TradeCaptureReport (35=AE) was received as drop-copy / STP.",
                true,
                true,
            ),
        }

        // Adapter extras: replace-by-key so a redundant declaration collapses.
        for d in spec.cert_checks {
            upsert(&mut checks, d);
        }

        CertLedger {
            role: spec.role,
            checks,
            overrides: spec.event_overrides,
        }
    }

    /// Feed an observed wire event. Auto-passes the mapped `automatic` check if
    /// it is still `PENDING`. Returns the key that flipped, if any (idempotent:
    /// a second identical event returns `None`).
    pub fn record_event(&mut self, dir: WireDirection, msg_type: &str) -> Option<&'static str> {
        let key = cert_check_for_event(self.role, dir, msg_type, self.overrides)?;
        let c = self.checks.iter_mut().find(|c| c.check_key == key)?;
        if c.automatic && c.status == CheckStatus::Pending {
            c.status = CheckStatus::Passed;
            Some(key)
        } else {
            None
        }
    }

    /// Operator marks a check PASSED (e.g. a manual resend/soak). Returns whether
    /// a matching check existed.
    pub fn mark_passed(&mut self, key: &str) -> bool {
        self.set_status(key, CheckStatus::Passed)
    }

    /// Operator marks a check FAILED.
    pub fn mark_failed(&mut self, key: &str) -> bool {
        self.set_status(key, CheckStatus::Failed)
    }

    /// Operator waives an **optional** check. Required checks cannot be skipped.
    pub fn skip(&mut self, key: &str) -> bool {
        match self.checks.iter_mut().find(|c| c.check_key == key) {
            Some(c) if !c.required => {
                c.status = CheckStatus::Skipped;
                true
            }
            _ => false,
        }
    }

    fn set_status(&mut self, key: &str, status: CheckStatus) -> bool {
        match self.checks.iter_mut().find(|c| c.check_key == key) {
            Some(c) => {
                c.status = status;
                true
            }
            None => false,
        }
    }

    /// Promote-to-PROD gate: every **required** check must be `PASSED`. When the
    /// platform cert gate is disabled (`cert_required = false`), always allowed.
    pub fn ready_for_prod(&self, cert_required: bool) -> bool {
        if !cert_required {
            return true;
        }
        self.checks
            .iter()
            .filter(|c| c.required)
            .all(|c| c.status == CheckStatus::Passed)
    }

    /// The current checklist — for the ops-console + API snapshot.
    pub fn checks(&self) -> &[CertCheck] {
        &self.checks
    }

    /// Count of checks in each terminal state — for the ops-console summary.
    pub fn passed_of_required(&self) -> (usize, usize) {
        let required: Vec<_> = self.checks.iter().filter(|c| c.required).collect();
        let passed = required
            .iter()
            .filter(|c| c.status == CheckStatus::Passed)
            .count();
        (passed, required.len())
    }
}

fn push(
    v: &mut Vec<CertCheck>,
    key: &'static str,
    label: &'static str,
    description: &'static str,
    required: bool,
    automatic: bool,
) {
    v.push(CertCheck {
        check_key: key,
        label,
        description,
        required,
        automatic,
        status: CheckStatus::Pending,
    });
}

fn upsert(v: &mut Vec<CertCheck>, d: &CertCheckDef) {
    if let Some(existing) = v.iter_mut().find(|c| c.check_key == d.check_key) {
        existing.label = d.label;
        existing.description = d.description;
        existing.required = d.required;
        existing.automatic = d.automatic;
    } else {
        v.push(CertCheck {
            check_key: d.check_key,
            label: d.label,
            description: d.description,
            required: d.required,
            automatic: d.automatic,
            status: CheckStatus::Pending,
        });
    }
}

/// Map an observed `(direction, msg_type)` to the cert check it should auto-pass,
/// if any. Resolution order: explicit adapter overrides → the role's inbound
/// app-flow event → standard session-level events.
pub fn cert_check_for_event(
    role: ConnectionRole,
    dir: WireDirection,
    msg_type: &str,
    overrides: &'static [EventCheckOverride],
) -> Option<&'static str> {
    for o in overrides {
        if o.direction == dir && o.msg_type == msg_type {
            return Some(o.check_key);
        }
    }
    if dir == WireDirection::Inbound {
        match (role, msg_type) {
            (ConnectionRole::Order, "8") => return Some(CHECK_ORDER_ER),
            (ConnectionRole::Price, "W") | (ConnectionRole::Price, "X") => {
                return Some(CHECK_PRICE_SUBSCRIPTION);
            }
            (ConnectionRole::Rfq, "S") => return Some(CHECK_RFQ_FLOW),
            (ConnectionRole::Stp, "AE") => return Some(CHECK_STP_TCR),
            _ => {}
        }
    }
    standard_cert_check_for_event(dir, msg_type)
}

/// Session-level (role-independent) event → check mapping.
fn standard_cert_check_for_event(dir: WireDirection, msg_type: &str) -> Option<&'static str> {
    match msg_type {
        // Logon completes the handshake in either direction.
        "A" => Some(CHECK_LOGON),
        "0" if dir == WireDirection::Inbound => Some(CHECK_HEARTBEAT),
        "1" if dir == WireDirection::Inbound => Some(CHECK_TEST_REQUEST),
        "5" if dir == WireDirection::Inbound => Some(CHECK_LOGOUT),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::STANDARD_INITIATOR_FIELDS;

    fn spec(role: ConnectionRole, extras: &'static [CertCheckDef]) -> VendorAdapterSpec {
        VendorAdapterSpec {
            code: "demo",
            display_name: "Demo",
            counterparty: "Demo Venue",
            role,
            fix_version: (4, 4),
            embedded_spec: "<fix/>",
            config_template: STANDARD_INITIATOR_FIELDS,
            cert_checks: extras,
            event_overrides: &[],
        }
    }

    #[test]
    fn seed_has_six_standard_plus_one_appflow() {
        let s = spec(ConnectionRole::Order, &[]);
        let led = CertLedger::seed(&s);
        assert_eq!(led.checks().len(), 7, "6 standard + 1 app-flow");
        assert!(led.checks().iter().any(|c| c.check_key == CHECK_ORDER_ER));
        assert!(
            led.checks()
                .iter()
                .all(|c| c.status == CheckStatus::Pending)
        );
    }

    #[test]
    fn logon_and_appflow_auto_pass_but_manual_gate_blocks_prod() {
        let s = spec(ConnectionRole::Price, &[]);
        let mut led = CertLedger::seed(&s);
        // Outbound Logon flips LOGON; inbound snapshot flips MD subscription.
        assert_eq!(
            led.record_event(WireDirection::Outbound, "A"),
            Some(CHECK_LOGON)
        );
        assert_eq!(
            led.record_event(WireDirection::Inbound, "0"),
            Some(CHECK_HEARTBEAT)
        );
        assert_eq!(
            led.record_event(WireDirection::Inbound, "1"),
            Some(CHECK_TEST_REQUEST)
        );
        assert_eq!(
            led.record_event(WireDirection::Inbound, "5"),
            Some(CHECK_LOGOUT)
        );
        assert_eq!(
            led.record_event(WireDirection::Inbound, "W"),
            Some(CHECK_PRICE_SUBSCRIPTION)
        );
        // Idempotent: a second snapshot does not re-flip.
        assert_eq!(led.record_event(WireDirection::Inbound, "X"), None);
        // Still blocked: the manual RESEND check is required and PENDING.
        assert!(!led.ready_for_prod(true));
        assert!(led.mark_passed(CHECK_RESEND));
        assert!(led.ready_for_prod(true), "all required now passed");
    }

    #[test]
    fn incremental_alone_passes_subscription() {
        let mut led = CertLedger::seed(&spec(ConnectionRole::Price, &[]));
        assert_eq!(
            led.record_event(WireDirection::Inbound, "X"),
            Some(CHECK_PRICE_SUBSCRIPTION)
        );
    }

    #[test]
    fn disabled_gate_allows_prod_immediately() {
        let led = CertLedger::seed(&spec(ConnectionRole::Rfq, &[]));
        assert!(led.ready_for_prod(false));
        assert!(!led.ready_for_prod(true));
    }

    #[test]
    fn adapter_extra_with_same_key_collapses() {
        static EXTRA: &[CertCheckDef] = &[CertCheckDef {
            check_key: CHECK_PRICE_SUBSCRIPTION,
            label: "Custom MD",
            description: "vendor-specific wording",
            required: true,
            automatic: true,
        }];
        let led = CertLedger::seed(&spec(ConnectionRole::Price, EXTRA));
        let md: Vec<_> = led
            .checks()
            .iter()
            .filter(|c| c.check_key == CHECK_PRICE_SUBSCRIPTION)
            .collect();
        assert_eq!(
            md.len(),
            1,
            "redundant adapter check must collapse, not duplicate"
        );
        assert_eq!(md[0].label, "Custom MD");
    }

    #[test]
    fn optional_soak_can_be_skipped_required_cannot() {
        let mut led = CertLedger::seed(&spec(ConnectionRole::Order, &[]));
        assert!(led.skip(CHECK_HIGH_VOLUME_SOAK));
        assert!(!led.skip(CHECK_LOGON), "required check cannot be skipped");
    }
}
