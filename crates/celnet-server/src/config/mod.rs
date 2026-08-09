//! On-disk operator configuration that survives process restarts.
//!
//! Two persisted documents:
//!
//! * [`fix_connections`] — the inbound FIX venues the edge binds on startup,
//!   managed at runtime through the `FixAdminService` and the GUI Connections
//!   workspace.
//! * [`identity`] — the operator users and desks (`AuthService` + the GUI Admin
//!   workspace), with Argon2id-hashed passwords and a seeded default admin.
//! * [`reference_data`] — the admin-managed instrument reference-data registry
//!   (instrument definitions keyed by an internal id + external-id cross-refs),
//!   persisted inside the same identity document; curve-building and pricing
//!   resolve against it
//!   (`docs/CURVES-AND-INSTRUMENT-REFERENCE-DATA-REVIEW.md`).
//! * [`curve_definitions`] — the multi-curve registry: named, definable
//!   interest-rate curves (reference data + calibrating pillars + interpolation +
//!   a primary-per-currency flag), managed through the `SurfaceService` curve-CRUD
//!   verbs and resolved against by the FIX rates edge (`usd-sofr` primary seeded).

pub mod consistency;
pub mod curve_calibration;
pub mod curve_definitions;
pub mod fix_connections;
pub mod hedge_policy;
pub mod identity;
pub mod reference_data;
