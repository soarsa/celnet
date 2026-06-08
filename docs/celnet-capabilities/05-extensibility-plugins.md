<sub>**[Celnet Capabilities](../CELNET-CAPABILITIES.md)** › Extensibility — The Open Quant SDK & Plugin Host</sub>

# 5. Extensibility — The Open Quant SDK & Plugin Host

A pricing platform is only as good as the models a desk can put inside it. Celnet ships a functionally complete analytics estate already — vanilla, the full first-generation exotics, structured and path-dependent products, American/Bermudan early exercise, correlated multi-asset baskets, an LSV booking model and a standalone Heston engine, all on one wire — yet treats the desk's *own* intellectual property as a first-class extension of that estate. Proprietary models run **inside the engine**, on the same registry and the same routing as the built-in analytics, without ever leaving the firm and without forking the platform. The vehicle is the **Open Quant SDK** — one frozen, runtime-agnostic contract — served by a **tiered plugin host** that trades performance against isolation while keeping every shipped tier deterministic and bit-identical on replay.

This is the edge the deep-catalogue incumbents structurally lack: a desk extends an already-deep catalogue with private code that is sandboxed, deterministic and hot-loadable, behind exactly one current contract.

![Tiered plugin host — one frozen contract; the native and WebAssembly tiers ship today, with signed-shared-object and OS-sandbox tiers designed to slot in behind the same contract.](../assets/celnet-capabilities/fig-05-plugin-tiers.png)
*The Open Quant SDK is a single frozen contract; the host routes a registered model to the tier its trust level warrants. The native (Tier-0) and WebAssembly-sandbox (Tier-2) tiers are shipped and gated today; the signed-shared-object and OS-sandbox tiers are designed and seamed behind the identical contract, proven at deploy.*

### 5.1 The Open Quant SDK — one contract, any model

The SDK (`celnet-plugin-api`) is a small, stable set of Rust traits describing the three things a model can be, plus the metadata and discovery that let the engine route to it without hard-wiring (`crates/celnet-plugin-api/src/{pricing,smile,calibration,descriptor,registry}.rs`):

| SDK surface | What a quant implements | What Celnet does with it |
|-------------|-------------------------|--------------------------|
| `PricingModel` | Price and the full Greek vector for an instrument | Routes pricing and risk through the model exactly as it does the native vanilla and exotics engines |
| `SmileModel` | A smile/surface parameterisation (extends the core `Smile` seam) | Marks, recalibrates, and arbitrage-gates the model alongside the built-in **Vanna-Volga, SABR, SVI, SSVI, and eSSVI** families |
| `Calibration` | A fit from market observables to model parameters | Drives the model from the same broker risk-reversal/butterfly/ATM and market-series inputs the built-in calibrators consume |
| `ModelDescriptor` / `ModelRegistry` | A self-describing manifest (`ModelId`, `ModelKind`, Greek support) | Lets the engine discover, name, and route the model by purpose with no hard-coded wiring |
| WIT interface mirror (`wit/celnet.wit`) | — | Mirrors the contract one-to-one for the sandboxed tier so a guest model speaks the identical shape |

Because the contract is **frozen and unversioned**, a model written against it today keeps working as Celnet evolves — there is exactly one current contract, never an N/N-1 negotiation. A registered model is indistinguishable, at the call site, from a built-in one: the host registry is **tier-blind** (`crates/celnet-plugin-host/src/registry.rs` — `register_native`/`register_wasm`/`load_wasm` all land behind one `model(id)` lookup), so the engine routes a private smile model or a bespoke pricer through the same path that serves the standard catalogue. The desk's quants do not reach the catalogue *through* the SDK — the catalogue is shipped core — they use the SDK to extend it with bespoke and structured products the firm wants to keep private.

### 5.2 The tiered host — performance and isolation, never one or the other

Not every model carries the same trust. A house model authored by the firm's own quants wants native speed; a model under evaluation, or one carrying third-party code, wants containment. The plugin host (`celnet-plugin-host`) is designed as a ladder of tiers behind the **same** contract, so the choice is a routing decision, not a rewrite. **The in-repo host ships the native and WebAssembly tiers; the signed-shared-object and OS-sandbox tiers are designed to slot in behind the same frozen contract** (`docs/PLUGIN-HOST-ALT.md` §3.4 / §3.5).

| Tier | Execution | Isolation posture | Typical use | Status |
|------|-----------|-------------------|-------------|--------|
| Native (Tier-0) | In-process, native speed, monomorphized zero-alloc dispatch | Trusted, first-party | House models on the hot path | **Shipped** (`native.rs`) |
| WebAssembly sandbox (Tier-2) | Fuel-metered wasmi interpreter | No ambient OS access; capability-scoped | Untrusted or under-evaluation models | **Shipped** (`wasm.rs`) |
| Signed shared-object (Tier-1) | Loaded native object via `stabby` stable ABI | Cryptographically signed, publisher-allowlisted provenance | Vetted partner models needing native speed | **Designed (deploy-gated)** |
| OS-sandbox (Tier-3) | Native under OS confinement | Kernel-enforced (Landlock / seccomp), Linux-only | Native models needing belt-and-braces containment | **Designed (deploy-gated)** |

> **Honest boundary.** Only the native (Tier-0) and WebAssembly (Tier-2) tiers are built and gated in-repo. The signed-shared-object tier (Tier-1, `stabby`) and the OS-sandbox ring (Tier-3, Landlock/seccomp) are **designed-only** — specified and seamed behind the identical frozen contract in `docs/PLUGIN-HOST-ALT.md`, to be proven at deploy. They are never claimed as shipped here.

**Tier-0 — native** (`native.rs`) adapts any first-party `PricingModel` to the tier-agnostic `HostModel` seam. It is generic over the concrete model, so the call is a direct, monomorphized, inlinable dispatch with **zero per-call allocation** — the hot-path property Tier-0 exists to preserve. There is no sandbox here by design; Tier-0 is trusted first-party code.

**Tier-2 — the WebAssembly sandbox** (`wasm.rs`) is the centre of gravity for untrusted code, and the advisory-clean replacement for the originally-planned wasmtime runtime. A user model compiled to a core Wasm module runs on the **`wasmi` pure-Rust interpreter** with **no ambient operating-system authority** — no WASI, no clock, no RNG, no threads, no filesystem, no network (`crates/celnet-plugin-host/src/host.rs`). The capability boundary is deliberately narrow: the *only* imports a guest may resolve are five host-provided mathematical primitives — `exp`, `ln`, `sqrt`, `norm_pdf`, `norm_cdf` (the exhaustive `GRANTED_IMPORTS` list) — each routed through the same `rust-lang/libm` the native tier uses. Any import outside that set is **denied at link time** with a precise, attributable error before the module is even instantiated.

The sandbox is hardened beyond capability-denial alone:

- **Compute SLA.** Every guest call runs under a per-call **fuel budget** (wasmi's deterministic instruction-cost counter, not wall-clock). Exhaustion interrupts the interpreter at the budget and yields a typed `HostError::FuelExhausted` — never a hang or panic. Even a module's `(start)` function is fuel-bounded at load, so a runaway initialiser cannot hang the host.
- **Resource ceilings.** A store-level limiter caps linear memory (64 MiB), table elements, memories, tables and instances; an over-budget `memory.grow`/`table.grow` traps deterministically as `HostError::ResourceLimit` rather than returning the Wasm `-1` sentinel.
- **Narrowed feature set.** The wasmi `Config` disables proposals a pricer never needs — `memory64`, `bulk_memory`, `reference_types`, `tail_call` — so a guest using them fails validation rather than widening the attack surface.
- **Boundary determinism.** Every `f64` crossing the ABI is NaN-canonicalised, so a malformed number can never leak non-determinism into the engine.
- **No `unsafe`.** The whole crate is `#![forbid(unsafe_code)]`; the wasmi interpreter is the only execution substrate and is driven entirely through its safe API.

Four work-stream gates (WS-G) prove these properties, exercised by 20 tests in `crates/celnet-plugin-host/tests/sandbox.rs`: **capability-denial** (an unknown import fails to load), **fuel-exhaustion bounded** (a runaway guest traps within budget and does not hang, verified including the `(start)` path), **replay bit-identity** (below), and **Tier-0 == Tier-2 interchangeability** (a native model and its Wasm twin agree to the last bit through one tier-blind registry). A guest's own domain rejection surfaces as a model error, not a sandbox fault — so a Tier-2 verdict is indistinguishable from a Tier-0 one.

### 5.3 Determinism, replay, and the arbitrage self-check

Determinism is a platform guarantee, not a hope. A single Tier-2 model produces **bit-identical** results on replay — the same inputs reproduce the same outputs to the last bit (`f64::to_bits`-equal, including any canonicalised NaN), run after run, on every platform wasmi runs on — which is what makes plugin-priced trades auditable and reproducible across the firm. The host carries a deterministic replay harness (`crates/celnet-plugin-host/src/replay.rs`) that replays a fixed market snapshot through a model and asserts bit-identity across runs; the same fuel budget interrupts at the same instruction, so even a near-budget model replays identically. Cross-platform identity additionally requires the guest's transcendentals to be the platform-independent `rust-lang/libm` ones — which is precisely why the host exposes them as capabilities rather than letting the guest reach for an FPU intrinsic.

Bit-identity *between* a native and a Wasm implementation of the same pricer is honestly **not** automatic — it holds only when both perform the same sequence of IEEE-754 operations through the same libm. The harness proves the strong bit-exact claim for a deliberately op-order-matched twin; for an arbitrary pair, agreement is within the model's documented numerical tolerance, and a tolerance-based comparator is the right gate. Celnet states this distinction rather than overclaiming universal bit-identity.

A smile model loaded through the SDK is held to the same standard as the built-ins: a **static (butterfly) no-arbitrage self-check** is built into the `SmileModel` trait (`crates/celnet-plugin-api/src/smile.rs` — a model-free density-positivity test over the strike grid, in the spirit of the Carr-Madan call-spread conditions). A plugin therefore cannot quietly publish an arbitrageable surface; an arbitrageable grid is rejected with a typed error.

### 5.4 What it means for the desk

- **Private IP stays private.** Proprietary pricers and smiles run *inside* Celnet's engine — sandboxed, deterministic, hot-loadable — and the source never leaves the firm.
- **Model evolution without downtime.** New or revised models load into the running engine and inherit Celnet's zero-downtime, blue-green state handoff; there is no rebuild-and-redeploy of the platform to ship a model change.
- **No fork, ever.** The frozen SDK means a desk extends Celnet rather than maintaining a private branch of it — bespoke and structured-product coverage is *additive on top of an already-complete built-in catalogue*, and upstream improvements arrive cleanly.
- **One discipline for every model.** House models and experimental models live behind the same contract, the same arbitrage gates, the same replay guarantee — so a model's trust tier changes how it is *contained*, never how it is *called* or *trusted to be correct*.

This is one of Celnet's sharpest lines of differentiation. A closed terminal gives a desk the vendor's models; a modern library gives a desk code but not an engine. Celnet gives both — the desk's own models, running at native speed where they are trusted and under hard, deterministic isolation where they are not, all through one clean, frozen contract — layered over a catalogue that already matches what the front-to-back platforms charge for.

---
<sub>[← Quant & Pricing Methodology Coverage](04-quant-coverage.md)  ·  **[Contents](../CELNET-CAPABILITIES.md)**  ·  [Risk Management →](06-risk-management.md)</sub>
