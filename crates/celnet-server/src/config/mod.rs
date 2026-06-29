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

pub mod fix_connections;
pub mod identity;
pub mod reference_data;
