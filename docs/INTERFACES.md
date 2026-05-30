# Celnet — Frozen Interface Registry

The contracts that parallel work-streams depend on. There is exactly **one clean, current
contract** — no versioned APIs, no back-compat shims (we have no external users). Changing
anything here means editing the interface crate **and every dependent in the same change**,
updating this file, and announcing it in the `CLAUDE.md` ledger. Within a parallel-build
window the interface crates are treated as **stable** so streams don't churn; a deliberate
interface change coordinates all affected crates at once (see `docs/ROADMAP.md` §3).

## The 19-crate workspace

The implemented flat workspace is **19 crates** (`ls crates`):

```
celnet-types  celnet-core  celnet-conventions  celnet-calendar  celnet-vanilla
celnet-surface  celnet-exotics  celnet-gpu  celnet-engine  celnet-integration
celnet-server  celnet-cli  celnet-client  celnet-proto  celnet-plugin-api
celnet-observability  celnet-golden  celnet-testkit  celnet-bench
```

Several domains the early design split across many crates were **consolidated**:
`celnet-surface` holds VV/SABR/SVI/SSVI + arbitrage gates + term structure;
`celnet-exotics` holds LSV + PDE + MC + the shared numerics; `celnet-golden` is the
QuantLib oracle/generator; `celnet-observability` owns telemetry rings/histograms;
`celnet-gpu` is the wgpu backend + f64 CPU reconciliation. `celnet-plugin-host` is **built**:
the tiered host (Tier-0 native registry + Tier-2 **wasmi** fuel-metered sandbox + replay
harness) behind the frozen `celnet-plugin-api` contract — wasmtime was rejected for open
RustSec advisories (see `docs/PLUGIN-HOST-ALT.md`).

## Dependency direction (must never invert)

```
celnet-types  ←  celnet-core  ←  { celnet-conventions, celnet-calendar, celnet-vanilla,
                                   celnet-surface, celnet-exotics, celnet-gpu }  ←  celnet-engine
                                   ←  { celnet-server, celnet-cli }
celnet-proto      →  depends only on celnet-types
celnet-plugin-api →  depends only on celnet-types (+ celnet-core traits)
celnet-client     →  depends on celnet-proto (typed SDK over tonic)
celnet-observability →  telemetry seam; celnet-engine stays free of its deps
celnet-integration   →  Celer estate + vendor MD adapters, over celnet-surface/-types
celnet-golden, celnet-testkit, celnet-bench  →  test/validation/bench only
```

## Status

| Crate | Version | Frozen? | Surface |
|-------|---------|---------|---------|
| `celnet-types` | 0.0.0 | **freeze-candidate** | `OptionType`, `Ccy`, `CcyPair`, `Tenor`; newtypes `Vol`/`Strike`/`Rate`/`Delta`/`Df`/`Time`; convention enums `DeltaConvention`/`AtmConvention`/`PremiumStyle`/`Cut`/`DayCount`/`Settlement`; DTOs `VanillaInputs`, `Greeks`. POD/`Copy`, `serde`. |
| `celnet-core` | 0.0.0 | **freeze-candidate** | `math` (`norm_cdf`, `norm_pdf`, `exp`/`ln`/`sqrt` via `libm`); `is_close` + `assert_close!` (ULP/rel/abs); trait `Smile` + `FlatSmile`. Zero IO. |
| `celnet-proto` | 0.0.0 | **freeze-candidate** | single current wire contract (`prost 0.13` / `tonic 0.12`); `celnet.proto` services `PricingService`/`QuoteService`/`StreamService`/`SurfaceService`; `Instrument` oneof; **no** version field / negotiation. |
| `celnet-plugin-api` | 0.0.0 | **freeze-candidate** | SDK traits (`PricingModel`/`PricingBackend`) + WIT world. |

> `celnet-proto` and `celnet-plugin-api` are built — **Gate G0 is reached** (consistent with
> `docs/CAPABILITIES-VS-COMPETITION.md`). The native trait-registry (Tier 0) and the **wasmi**
> Wasm host (Tier 2) both implement the identical `PricingModel` contract so first-party and
> user plugins are interchangeable behind one registry; the host (`celnet-plugin-host`) is
> **built** (wasmtime rejected for open RustSec advisories — wasmi is the advisory-clean
> replacement, see `docs/PLUGIN-HOST-ALT.md`).

## Determinism rules baked into the interfaces

- All float comparison via `celnet_core::assert_close!` / `is_close` (ULP + rel + abs). Never `==`; never assert on NaN payloads.
- Transcendentals via `rust-lang/libm` (correctly-rounded) for bit-identical cross-platform results.
- GPU numerics standardize on **f32** with an **f64 CPU reconciliation** oracle.
- Scalar policy: `f64` is the CPU canonical type; convention/units encoded in types, not comments.
