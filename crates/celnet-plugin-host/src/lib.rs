//! Celnet plugin host — runs user-extensible analytics against the
//! `celnet-plugin-api` contract through a **tiered** execution model
//! (work-stream WS-G; design in `docs/PLUGIN-HOST-ALT.md`):
//!
//! - **Tier 0 — native registry**: first-party models compiled in, hot-path speed.
//! - **Tier 2 — deterministic sandbox**: untrusted user models run on **wasmi**
//!   (pure-Rust, fuel-metered, no ambient authority) — the advisory-clean
//!   replacement for wasmtime. Identical inputs ⇒ bit-identical output; a per-call
//!   fuel budget is the model's compute/latency SLA.
//!
//! Both tiers implement one registry behind the frozen `celnet-plugin-api`
//! contract, so first-party and user models are interchangeable. Skeleton —
//! implementation lands in this lane.
#![forbid(unsafe_code)]
