//! Celnet plugin host — the **tiered** execution model for user-extensible
//! analytics, behind the frozen `celnet-plugin-api` contract (work-stream WS-G;
//! design in `docs/PLUGIN-HOST-ALT.md`).
//!
//! A caller (the engine) resolves a model by [`celnet_plugin_api::ModelId`]
//! through one [`ModelRegistry`] and prices through it **without knowing which
//! tier serves the call**:
//!
//! - **Tier 0 — native registry** ([`native`]): first-party models implementing
//!   [`celnet_plugin_api::PricingModel`], compiled in, full native speed, the hot
//!   path. No ABI or sandbox cost.
//! - **Tier 2 — deterministic sandbox** ([`wasm`]): untrusted user models run on
//!   **wasmi** (pure-Rust, fuel-metered, *no ambient authority* — no WASI, no
//!   clock/RNG/threads/filesystem) under documented linear-memory/table/instance
//!   resource caps (see [`crate::host`]). It is the advisory-clean replacement for
//!   the originally-planned wasmtime runtime. A single Tier-2 model is **fully
//!   deterministic**: identical inputs through it produce **bit-identical** output
//!   on every run and platform ([`mod@replay`]). A per-call **fuel budget is the
//!   compute SLA**, and its exhaustion is a typed [`HostError::FuelExhausted`] —
//!   never a hang or panic.
//!
//! Both tiers present the same [`crate::model::HostModel`] shape behind the
//! unified [`ModelRegistry`], so first-party and user models are interchangeable
//! at the *interface* level. Whether a native (Tier-0) model and a Wasm (Tier-2)
//! model of the *same pricer* agree **bit-for-bit** is not automatic: it holds
//! only when both implementations perform the same sequence of IEEE-754 float
//! operations through the same `rust-lang/libm` (the host routes the guest's
//! transcendentals through [`crate::host`] to make this achievable). When op
//! order or the libm differ, the two tiers agree within the model's documented
//! numerical tolerance rather than to the last bit. The test suite proves the
//! strong bit-identity claim only for a twin pair constructed to share op order
//! (see `tests/sandbox.rs`).
//!
//! # Module map
//!
//! - [`model`] — the tier-agnostic [`crate::model::HostModel`] seam.
//! - [`native`] — Tier-0 [`crate::native::NativeModel`] adapter.
//! - [`wasm`] — Tier-2 [`crate::wasm::WasmModel`] sandbox (wasmi engine, fuel,
//!   no-WASI capability linker, ABI marshalling).
//! - [`host`] — the explicit capability surface granted to Tier-2 guests.
//! - [`abi`] — the host-controlled `(ptr,len)` core-module ABI and boundary
//!   NaN-canonicalization.
//! - [`registry`] — the unified [`ModelRegistry`].
//! - [`mod@replay`] — the deterministic replay / bit-identity harness.
//! - [`error`] — the host error taxonomy ([`HostError`]).
//!
//! This crate contains **no `unsafe`**: wasmi's interpreter is the only execution
//! substrate and it is driven entirely through its safe API.

#![forbid(unsafe_code)]

pub mod abi;
pub mod error;
pub mod host;
pub mod model;
pub mod native;
pub mod rates_model;
pub mod registry;
pub mod replay;
pub mod wasm;

pub use error::{HostError, HostResult};
pub use model::HostModel;
pub use native::NativeModel;
pub use rates_model::{NativeRatesModel, RatesHostModel};
pub use registry::ModelRegistry;
pub use replay::{ReplayError, ReplayOutcome, Snapshot, assert_agree, replay};
pub use wasm::{FuelBudget, WasmModel};
