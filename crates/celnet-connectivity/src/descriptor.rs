//! Vendor-neutral FIX venue-adapter descriptors.
//!
//! An adapter is **data, not a type**: every venue is a [`VendorAdapterSpec`]
//! value in the registry ([`crate::registry`]). The counterparty identity — the
//! external bank/ECN the session talks to — lives in the [`VendorAdapterSpec::counterparty`]
//! *field* (a value), never in a Rust identifier. That keeps the crate's type
//! namespace purpose-named and vendor-neutral (guardrail #8) while still letting
//! an operator address a concrete venue by its `code`.
//!
//! These descriptors are the **app-flow + onboarding** layer only. All wire
//! framing / session recovery / Logon-Heartbeat machinery is `celnet-fix`; the
//! driver ([`crate::driver`], added on the celnet-fix seam) turns a descriptor +
//! endpoint config into a live session and feeds observed wire events into the
//! certification ledger ([`crate::cert`]).

use serde::Serialize;

/// Coarse connection role — what a venue session is for. Selects the primary
/// app-flow certification check and the message set the driver exercises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ConnectionRole {
    /// Order entry: `NewOrderSingle (35=D)` → `ExecutionReport (35=8)`.
    Order,
    /// Market data: `MarketDataRequest (35=V)` → `Snapshot (35=W)` / `Incremental (35=X)`.
    Price,
    /// Request-for-quote: `QuoteRequest (35=R)` → `Quote (35=S)`.
    Rfq,
    /// Straight-through-processing / drop-copy: inbound `TradeCaptureReport (35=AE)`.
    Stp,
}

impl ConnectionRole {
    /// The UPPERCASE wire string (`"ORDER"`, `"PRICE"`, `"RFQ"`, `"STP"`).
    pub fn as_str(self) -> &'static str {
        match self {
            ConnectionRole::Order => "ORDER",
            ConnectionRole::Price => "PRICE",
            ConnectionRole::Rfq => "RFQ",
            ConnectionRole::Stp => "STP",
        }
    }
}

/// Direction of an observed wire event, from CelNet's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum WireDirection {
    /// Received from the counterparty.
    Inbound,
    /// Sent to the counterparty.
    Outbound,
}

impl WireDirection {
    /// The UPPERCASE wire string (`"INBOUND"` / `"OUTBOUND"`).
    pub fn as_str(self) -> &'static str {
        match self {
            WireDirection::Inbound => "INBOUND",
            WireDirection::Outbound => "OUTBOUND",
        }
    }
}

/// Render kind for a configuration field — the ops-console form control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    /// A single-line free-text input.
    Text,
    /// A numeric input.
    Number,
    /// A closed choice from `options`.
    Select,
    /// A multi-line free-text input.
    Multiline,
}

/// One configurable field on a connection's edit form. `credential` fields are
/// encrypted at rest and masked in the UI (handled at the control-plane, not here).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldTemplate {
    /// Stable key the stored config uses for this field.
    pub field_key: &'static str,
    /// Human-readable label for the form control.
    pub label: &'static str,
    /// Help text shown beneath the control.
    pub description: &'static str,
    /// Whether the connection cannot start without this field.
    pub required: bool,
    /// A secret: encrypted at rest and masked in the UI.
    pub credential: bool,
    /// The form control to render.
    pub kind: FieldKind,
    /// Options for a [`FieldKind::Select`]; empty otherwise.
    pub options: &'static [&'static str],
    /// Prefilled default value, if any.
    pub default_value: Option<&'static str>,
}

/// One adapter-specific certification check, appended to the standard
/// session-level set seeded by [`crate::cert`]. `automatic` checks flip to
/// PASSED the first time the session observes the mapped wire event; the rest
/// are manual (an operator marks them after validating the business flow).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertCheckDef {
    /// Stable, unique key the ops console + audit strings reference.
    pub check_key: &'static str,
    /// Short human-readable label.
    pub label: &'static str,
    /// What passing this check means.
    pub description: &'static str,
    /// Whether promote-to-PROD requires this check.
    pub required: bool,
    /// Whether an observed wire event auto-passes it (vs. an operator mark).
    pub automatic: bool,
}

/// A vendor-specific `(direction, msg_type) → check_key` override. Rare — most
/// adapters rely on the role's default app-flow mapping in [`crate::cert`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventCheckOverride {
    /// The direction this override matches.
    pub direction: WireDirection,
    /// FIX `MsgType (35=…)` value, e.g. `"W"`, `"X"`, `"8"`.
    pub msg_type: &'static str,
    /// The check to auto-pass on the matching event.
    pub check_key: &'static str,
}

/// A complete venue-adapter descriptor. Entirely `&'static` data so the whole
/// registry is a compile-time table with no allocation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VendorAdapterSpec {
    /// Stable, unique, `snake_case` identifier the connection row + config
    /// reference. Names the venue (data), e.g. `"bloomberg_fxgo_price"`.
    pub code: &'static str,
    /// Human-readable name for the picker UI.
    pub display_name: &'static str,
    /// The external counterparty (bank/ECN) this session talks to — a value,
    /// not a Rust identifier.
    pub counterparty: &'static str,
    /// The connection role — selects the app-flow cert check + message set.
    pub role: ConnectionRole,
    /// `(major, minor)` FIX version.
    pub fix_version: (u8, u8),
    /// The QuickFIX-style XML dictionary, embedded via `include_str!`. Large;
    /// not serialized inline (streamed separately by the control-plane).
    #[serde(skip)]
    pub embedded_spec: &'static str,
    /// Configuration fields the connection edit form renders.
    pub config_template: &'static [FieldTemplate],
    /// Adapter-specific cert checks (beyond the standard session set).
    pub cert_checks: &'static [CertCheckDef],
    /// Vendor-specific event→check overrides (usually empty).
    #[serde(skip)]
    pub event_overrides: &'static [EventCheckOverride],
}

impl VendorAdapterSpec {
    /// Structural self-validation used by the registry test: a non-empty
    /// snake_case code, a plausible FIX version, and cert-check keys that are
    /// non-empty. Returns the first problem found.
    pub fn validate(&self) -> Result<(), String> {
        if self.code.is_empty()
            || !self
                .code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(format!("code {:?} must be non-empty snake_case", self.code));
        }
        if self.fix_version.0 == 0 {
            return Err(format!(
                "{}: implausible fix_version {:?}",
                self.code, self.fix_version
            ));
        }
        if self.embedded_spec.trim().is_empty() {
            return Err(format!("{}: empty embedded_spec", self.code));
        }
        for c in self.cert_checks {
            if c.check_key.is_empty() {
                return Err(format!("{}: cert check with empty key", self.code));
            }
        }
        Ok(())
    }
}

/// The standard initiator config field set shared by every outbound (initiator)
/// venue session: endpoint + FIX identity + credentials. Vendor-neutral.
pub const STANDARD_INITIATOR_FIELDS: &[FieldTemplate] = &[
    FieldTemplate {
        field_key: "host",
        label: "Host",
        description: "Counterparty FIX acceptor hostname or IP.",
        required: true,
        credential: false,
        kind: FieldKind::Text,
        options: &[],
        default_value: None,
    },
    FieldTemplate {
        field_key: "port",
        label: "Port",
        description: "Counterparty FIX acceptor TCP port.",
        required: true,
        credential: false,
        kind: FieldKind::Number,
        options: &[],
        default_value: None,
    },
    FieldTemplate {
        field_key: "senderCompId",
        label: "SenderCompID (49)",
        description: "Our CompID on this session.",
        required: true,
        credential: false,
        kind: FieldKind::Text,
        options: &[],
        default_value: None,
    },
    FieldTemplate {
        field_key: "targetCompId",
        label: "TargetCompID (56)",
        description: "The counterparty's CompID.",
        required: true,
        credential: false,
        kind: FieldKind::Text,
        options: &[],
        default_value: None,
    },
    FieldTemplate {
        field_key: "username",
        label: "Username (553)",
        description: "Logon username, if the venue requires one.",
        required: false,
        credential: false,
        kind: FieldKind::Text,
        options: &[],
        default_value: None,
    },
    FieldTemplate {
        field_key: "password",
        label: "Password (554)",
        description: "Logon password / session secret. Encrypted at rest, masked in the UI.",
        required: false,
        credential: true,
        kind: FieldKind::Text,
        options: &[],
        default_value: None,
    },
    FieldTemplate {
        field_key: "heartbeatSecs",
        label: "HeartBtInt (108)",
        description: "Heartbeat interval in seconds.",
        required: true,
        credential: false,
        kind: FieldKind::Number,
        options: &[],
        default_value: Some("30"),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_and_direction_strings() {
        assert_eq!(ConnectionRole::Order.as_str(), "ORDER");
        assert_eq!(ConnectionRole::Price.as_str(), "PRICE");
        assert_eq!(WireDirection::Inbound.as_str(), "INBOUND");
        assert_eq!(WireDirection::Outbound.as_str(), "OUTBOUND");
    }

    #[test]
    fn standard_fields_have_one_credential() {
        let creds = STANDARD_INITIATOR_FIELDS
            .iter()
            .filter(|f| f.credential)
            .count();
        assert_eq!(creds, 1, "exactly the password field is a credential");
        assert!(
            STANDARD_INITIATOR_FIELDS
                .iter()
                .any(|f| f.field_key == "host")
        );
    }

    #[test]
    fn validate_rejects_bad_code_and_empty_spec() {
        let bad = VendorAdapterSpec {
            code: "Bad Code",
            display_name: "x",
            counterparty: "x",
            role: ConnectionRole::Order,
            fix_version: (4, 4),
            embedded_spec: "<fix/>",
            config_template: STANDARD_INITIATOR_FIELDS,
            cert_checks: &[],
            event_overrides: &[],
        };
        assert!(bad.validate().is_err());

        let empty_spec = VendorAdapterSpec {
            code: "ok",
            embedded_spec: "   ",
            ..bad.clone()
        };
        assert!(
            empty_spec
                .validate()
                .unwrap_err()
                .contains("empty embedded_spec")
        );

        let good = VendorAdapterSpec {
            code: "ok",
            embedded_spec: "<fix/>",
            ..bad
        };
        assert!(good.validate().is_ok());
    }

    #[test]
    fn spec_serializes_without_the_huge_dict() {
        let spec = VendorAdapterSpec {
            code: "demo_price",
            display_name: "Demo",
            counterparty: "Demo Venue",
            role: ConnectionRole::Price,
            fix_version: (4, 4),
            embedded_spec: "<fix>HUGE</fix>",
            config_template: STANDARD_INITIATOR_FIELDS,
            cert_checks: &[],
            event_overrides: &[],
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains("\"code\":\"demo_price\""));
        assert!(json.contains("\"counterparty\":\"Demo Venue\""));
        // The embedded dictionary is #[serde(skip)] — must not bloat the API payload.
        assert!(
            !json.contains("HUGE"),
            "embedded_spec must not serialize inline"
        );
    }
}
