//! On-disk operator configuration that survives process restarts.
//!
//! Two persisted documents:
//!
//! * [`fix_connections`] — the inbound FIX venues the edge binds on startup,
//!   managed at runtime through the `FixAdminService` and the GUI Connections
//!   workspace.
//! * [`identity`] — the operator users and desks (`AuthService` + the GUI Admin
//!   workspace), with Argon2id-hashed passwords and a seeded default admin.

pub mod consistency;
pub mod fix_connections;
pub mod identity;
