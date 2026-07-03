//! `celnet-connectivity` — the CelNet connectivity extension.
//!
//! Vendor FIX venue-adapter **descriptors** + a **certification harness**,
//! layered on the existing `celnet-fix` engine (reused, never re-implemented).
//! Distilled from a review of `soarsa/celnet-connectivity` (which was ~98.5%
//! scaffold with a FIX engine duplicating `celnet-fix`); see
//! `docs/CELNET-CONNECTIVITY-INTEGRATION.md` for the full critique + scope.
//!
//! Layers:
//! * [`descriptor`] — vendor-neutral, data-driven venue adapter specs. Vendor
//!   identity is carried as **data** (`counterparty`/`code`), never in a Rust
//!   identifier (guardrail #8).
//! * [`cert`] — the certification ledger: seed a checklist, auto-pass automatic
//!   checks as the session observes wire events, gate promote-to-PROD.
//! * [`registry`] — the compile-time table of seeded adapters.
//!
//! The live FIX session **driver** (on the `celnet-fix` seam) and the
//! control-plane **API** land in the `conn-framework` / `conn-server-api` lanes.

pub mod cert;
pub mod descriptor;
pub mod registry;

mod adapters;

pub use cert::{
    CHECK_HEARTBEAT, CHECK_HIGH_VOLUME_SOAK, CHECK_LOGON, CHECK_LOGOUT, CHECK_ORDER_ER,
    CHECK_PRICE_SUBSCRIPTION, CHECK_RESEND, CHECK_RFQ_FLOW, CHECK_STP_TCR, CHECK_TEST_REQUEST,
    CertCheck, CertLedger, CheckStatus, cert_check_for_event,
};
pub use descriptor::{
    CertCheckDef, ConnectionRole, EventCheckOverride, FieldKind, FieldTemplate,
    STANDARD_INITIATOR_FIELDS, VendorAdapterSpec, WireDirection,
};
pub use registry::AdapterRegistry;
