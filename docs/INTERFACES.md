# Celnet — Frozen Interface Registry

The contracts that parallel work-streams depend on. Changing anything here is a **dedicated
interface PR**: bump the crate's semver, update this file, run the N/N-1 compatibility gate
(once `celer-proto` lands), and announce the change in the `CLAUDE.md` ledger. Implementation
sessions consume these by version and **never edit them ad hoc** (see `docs/ROADMAP.md` §3).

## Dependency direction (must never invert)

```
celer-types  ←  celer-core  ←  { celer-conventions, celer-calendar, celer-vanilla,
                                  celer-surface, celer-exotics, celer-gpu }  ←  celer-engine
                                  ←  { celer-server, celer-cli }
celer-proto, celer-plugin-api  →  depend only on celer-types (+ celer-core traits)
```

## Status

| Crate | Version | Frozen? | Surface |
|-------|---------|---------|---------|
| `celer-types` | 0.0.0 | **freeze-candidate** | `OptionType`, `Ccy`, `CcyPair`, `Tenor`; newtypes `Vol`/`Strike`/`Rate`/`Delta`/`Df`/`Time`; convention enums `DeltaConvention`/`AtmConvention`/`PremiumStyle`/`Cut`/`DayCount`/`Settlement`; DTOs `GkInputs`, `Greeks`. POD/`Copy`, `serde`. |
| `celer-core` | 0.0.0 | **freeze-candidate** | `math` (`norm_cdf`, `norm_pdf`, `exp`/`ln`/`sqrt` via `libm`); `is_close` + `assert_close!` (ULP/rel/abs); trait `Smile` + `FlatSmile`. Zero IO. |
| `celer-proto` | — | pending (G0) | versioned wire envelope (prost 0.14); `schema_version` header; N/N-1 gate. |
| `celer-plugin-api` | — | pending (G0) | SDK traits (`PricingModel`/`VolModel`) + WIT world; semver product. |

> `celer-proto` and `celer-plugin-api` complete **Gate G0**. `PricingModel`/`PricingBackend`
> trait shapes will be finalized here when they land so the native trait-registry and the
> Wasm host implement the identical contract (first-party and user plugins interchangeable).

## Determinism rules baked into the interfaces

- All float comparison via `celer_core::assert_close!` / `is_close` (ULP + rel + abs). Never `==`; never assert on NaN payloads.
- Transcendentals via `rust-lang/libm` (correctly-rounded) for bit-identical cross-platform results.
- GPU numerics standardize on **f32** with an **f64 CPU reconciliation** oracle.
- Scalar policy: `f64` is the CPU canonical type; convention/units encoded in types, not comments.
