//! The seeded venue adapters — **data, not types**.
//!
//! These are the three genuinely-real adapters carried over from the source
//! connectivity platform (their FIX dictionaries are embedded verbatim under
//! `specs/`), re-expressed as vendor-neutral [`VendorAdapterSpec`] values: the
//! venue name lives in the `counterparty`/`display_name`/`code` data, never in a
//! Rust identifier. New venues are added as more entries here (a demand-driven
//! backlog — see `docs/CELNET-CONNECTIVITY-INTEGRATION.md`), not as new modules.

use crate::cert::CHECK_ORDER_ER;
use crate::descriptor::{
    CertCheckDef, ConnectionRole, STANDARD_INITIATOR_FIELDS, VendorAdapterSpec,
};

const BLOOMBERG_DICT: &str = include_str!("../specs/fix44_bloomberg.xml");
const RABOFX_DICT: &str = include_str!("../specs/fix44_rabofx_esp.xml");

/// Rabobank ESP echoes a venue `SecondaryClOrdID (526)` on the ExecutionReport
/// that an operator must eye-verify, so the order→ER check is **manual** here —
/// this replaces the Order role's automatic default via the ledger's upsert.
const RABOFX_ER_MANUAL: &[CertCheckDef] = &[CertCheckDef {
    check_key: CHECK_ORDER_ER,
    label: "NewOrderSingle → ExecutionReport (ESP)",
    description: "Operator observed a NewOrderSingle (35=D) producing an ExecutionReport (35=8) \
                  carrying the venue's SecondaryClOrdID (526).",
    required: true,
    automatic: false,
}];

/// Every seeded adapter, compile-time. The registry ([`crate::registry`]) wraps this.
pub(crate) static ALL: &[VendorAdapterSpec] = &[
    VendorAdapterSpec {
        code: "bloomberg_fxgo_price",
        display_name: "Bloomberg FXGO — Streaming Price",
        counterparty: "Bloomberg FXGO",
        role: ConnectionRole::Price,
        fix_version: (4, 4),
        embedded_spec: BLOOMBERG_DICT,
        config_template: STANDARD_INITIATOR_FIELDS,
        cert_checks: &[],
        event_overrides: &[],
    },
    VendorAdapterSpec {
        code: "bloomberg_fxgo_order",
        display_name: "Bloomberg FXGO — Order Entry",
        counterparty: "Bloomberg FXGO",
        role: ConnectionRole::Order,
        fix_version: (4, 4),
        embedded_spec: BLOOMBERG_DICT,
        config_template: STANDARD_INITIATOR_FIELDS,
        cert_checks: &[],
        event_overrides: &[],
    },
    VendorAdapterSpec {
        code: "rabofx_esp_order",
        display_name: "Rabobank FX — ESP Order Routing",
        counterparty: "Rabobank FX",
        role: ConnectionRole::Order,
        fix_version: (4, 4),
        embedded_spec: RABOFX_DICT,
        config_template: STANDARD_INITIATOR_FIELDS,
        cert_checks: RABOFX_ER_MANUAL,
        event_overrides: &[],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_adapters_present_and_valid() {
        assert_eq!(ALL.len(), 3);
        for a in ALL {
            a.validate().unwrap_or_else(|e| panic!("{}: {e}", a.code));
        }
    }

    #[test]
    fn dictionaries_are_real_fix_44() {
        assert!(
            BLOOMBERG_DICT.contains("msgtype=\"W\""),
            "bloomberg dict missing MD snapshot"
        );
        assert!(
            RABOFX_DICT.contains("msgtype=\"D\""),
            "rabofx dict missing NewOrderSingle"
        );
    }
}
