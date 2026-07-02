# Plugin host — wasmi (wasmtime blocker CLOSED)

`celnet-plugin-host` is BUILT (WS-G done, 2026-05-30). It is a **tiered host** behind the
**frozen** `celnet-plugin-api` contract (Rust traits + `wit/celnet.wit`) — the contract did
NOT change, only the host runtime.

- **Tier 0 (native, hot path):** `NativeModel<M: PricingModel>` adapts compiled-in first-party
  models to the tier-blind `HostModel` seam.
- **Tier 2 (untrusted sandbox):** **wasmi 1.0.9** (pure-Rust, fuel-metered interpreter). It is
  the advisory-clean replacement for **wasmtime**, which is REJECTED (open 2026 RustSec
  advisories incl. an out-of-sandbox-load class; blocked by guardrail 7 + cargo-deny).
- One unified `ModelRegistry` routes by `ModelId`/`ModelDescriptor`; callers can't tell which
  tier serves a call (interchangeability = R5).

Tier-2 design (do not regress):
- `Config::consume_fuel(true)`; per-call `FuelBudget` is the compute SLA; exhaustion =>
  typed `HostError::FuelExhausted` (detected via `as_trap_code() == TrapCode::OutOfFuel`),
  never a hang/panic.
- No-WASI capability `Linker`: only `celnet_math.{exp,ln,sqrt,norm_pdf,norm_cdf}` (libm via
  `celnet_core::math`). Zero ambient authority. Unknown import => `HostError::CapabilityDenied`
  (pre-checked against `host::GRANTED_IMPORTS`).
- Boundary NaN-canonicalization (`abi::CANONICAL_NAN_BITS = 0x7ff8...0`) on EVERY f64 in/out.
- Host-controlled core-module `(ptr,len)` ABI (wasmi hosts core modules, NOT the Component
  Model). Guest exports: `memory`, `celnet_scratch_in/out`, `celnet_price`,
  `celnet_price_greeks` (returns 0 ok / negative AbiStatus => mapped to `PluginError`).
- `WasmModel` holds its `Store` in a `RefCell` (single-threaded, one handle per worker; not
  Sync by design). Refuels before every call.

Replay harness (`replay.rs`): `replay()` asserts `to_bits` identity across N runs;
`assert_agree()` checks two models bit-identically. Cross-platform identity REQUIRES libm
transcendentals (host enforces by exposing celnet_core::math).

Gates (4, all green; tests in `crates/celnet-plugin-host/tests/sandbox.rs`, WAT fixtures via
`wat` dev-dep): capability-denial, fuel-exhaustion bounded (watchdog thread + recv_timeout),
replay bit-identity, Tier-0==Tier-2 interchangeability. 14 tests; fmt/clippy-D/nextest/deny
all green. Crate has NO `unsafe` (`#![forbid(unsafe_code)]`).

Not yet wired (designed in `docs/PLUGIN-HOST-ALT.md` §4): Tier-1 trusted signed `.so` via
`stabby` 72.x; Tier-3 optional Linux Landlock+seccomp child-process ring.
