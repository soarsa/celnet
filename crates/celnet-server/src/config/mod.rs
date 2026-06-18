//! On-disk operator configuration that survives process restarts.
//!
//! Currently just the persisted **FIX acceptor connection** definitions
//! ([`fix_connections`]) — the inbound venues the edge binds on startup, managed
//! at runtime through the `FixAdminService` and the GUI Connections workspace.

pub mod fix_connections;
