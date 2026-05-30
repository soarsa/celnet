//! Host-side error contract for the tiered plugin host.
//!
//! The host runs untrusted Tier-2 Wasm models, so its failure modes are richer
//! than the portable [`celnet_plugin_api::PluginError`] that a *model* returns:
//! a module can fail to load, fail to link (because it imported a capability the
//! host did not grant), trap, or exhaust its fuel budget. [`HostError`] captures
//! those host concerns; a model's own domain failures still cross the boundary as
//! [`celnet_plugin_api::PluginError`] and are surfaced through
//! [`HostError::Model`].
//!
//! Fuel exhaustion is a **typed, non-panicking** outcome
//! ([`HostError::FuelExhausted`]) — never a hang. A runaway guest is interrupted
//! deterministically at its fuel budget and this error is returned, which is the
//! whole point of the metered sandbox (the budget is the compute SLA).

use core::fmt;

use celnet_plugin_api::PluginError;

/// Why a host operation against a plugin tier failed.
///
/// Kept `Clone` and free of live runtime handles so it can be logged, compared
/// in tests, and surfaced uniformly regardless of which tier produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostError {
    /// The Wasm bytes were not a valid core module (parse/validation failure).
    /// The payload is a short, provenance-neutral reason.
    InvalidModule(String),
    /// The module declared an import the host's capability surface
    /// ([`crate::host::GRANTED_IMPORTS`]) does not grant. The payload is the
    /// offending `module::name`. This is the capability-denial outcome: zero
    /// ambient authority means an unknown import fails to link rather than
    /// silently resolving.
    CapabilityDenied(String),
    /// The module is missing an export the host ABI requires (e.g. `memory`, the
    /// scratch allocator, or a pricing entry point). The payload names it.
    MissingExport(String),
    /// The guest trapped during execution for a reason other than fuel
    /// (out-of-bounds memory access, integer divide-by-zero, `unreachable`, …).
    /// The payload is the trap's short description.
    Trapped(String),
    /// The guest exhausted its per-call **fuel budget** — the deterministic
    /// compute SLA. The interpreter was interrupted at its budget; this is a
    /// bounded, typed outcome, never a hang or a panic.
    FuelExhausted,
    /// The guest tried to exceed a **resource budget** — linear-memory bytes,
    /// table elements, memories, tables, or instances — beyond the documented
    /// caps in [`crate::host`]. The allocation was denied in-engine (a grow
    /// operation traps; an over-budget declared minimum is rejected at
    /// instantiation), so a hostile or buggy guest can never OOM or hang the
    /// host. The payload is a short description of the denied allocation.
    ResourceLimit(String),
    /// A boundary value or marshalled buffer was malformed (e.g. a guest returned
    /// a pointer/length outside its linear memory). The payload is the reason.
    AbiViolation(String),
    /// The model itself returned a domain error across the boundary (invalid
    /// input, non-convergence, …). This is the *model's* verdict, distinct from a
    /// host/sandbox failure, so Tier-0 and Tier-2 surface identical model errors.
    Model(PluginError),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::InvalidModule(why) => write!(f, "invalid wasm module: {why}"),
            HostError::CapabilityDenied(what) => {
                write!(f, "capability denied: import `{what}` is not granted")
            }
            HostError::MissingExport(what) => write!(f, "missing required export: `{what}`"),
            HostError::Trapped(why) => write!(f, "guest trapped: {why}"),
            HostError::FuelExhausted => f.write_str("fuel budget exhausted"),
            HostError::ResourceLimit(why) => write!(f, "resource limit exceeded: {why}"),
            HostError::AbiViolation(why) => write!(f, "abi violation: {why}"),
            HostError::Model(e) => write!(f, "model error: {e}"),
        }
    }
}

impl core::error::Error for HostError {}

impl From<PluginError> for HostError {
    fn from(e: PluginError) -> Self {
        HostError::Model(e)
    }
}

/// The host result alias used across the tiered registry surface.
pub type HostResult<T> = Result<T, HostError>;
