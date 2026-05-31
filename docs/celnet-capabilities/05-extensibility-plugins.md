<sub>**[Celnet Capabilities](../CELNET-CAPABILITIES.md)** › Extensibility — The Open Quant SDK & Plugin Host</sub>

# 5. Extensibility — The Open Quant SDK & Plugin Host

A pricing platform is only as good as the models a desk can put inside it. Celnet treats the desk's own intellectual property as a first-class citizen: proprietary models run **inside the engine**, on the same wait-free hot path as the built-in analytics, without ever leaving the firm and without forking the platform. The vehicle is the **Open Quant SDK** — one frozen, runtime-agnostic contract — served by a **tiered plugin host** that trades performance against isolation while keeping every tier deterministic and bit-identical on replay.

![Tiered plugin host — one frozen contract, four execution tiers from native speed to OS-sandboxed isolation, all deterministic on replay.](../assets/celnet-capabilities/fig-05-plugin-tiers.png)
*The Open Quant SDK is a single frozen contract; the host routes a registered model to the tier its trust level warrants — native, WebAssembly sandbox, signed shared-object, or OS-sandboxed — with identical inputs, outputs, and replay semantics across all of them.*

### 5.1 The Open Quant SDK — one contract, any model

The SDK is a small, stable set of Rust traits that describe everything a model can be:

| SDK surface | What a quant implements | What Celnet does with it |
|-------------|-------------------------|--------------------------|
| `PricingModel` | Price and the full Greek vector for an instrument | Routes pricing and risk through the model exactly as it does the native vanilla and exotics engines |
| `SmileModel` | A smile/surface parameterisation | Marks, recalibrates, and arbitrage-gates the model alongside Vanna-Volga, SABR, SVI, and SSVI |
| `Calibration` | A fit from market observables to model parameters | Drives the model from the same broker-strangle and market-series inputs the built-in calibrators consume |
| Model registry | A self-describing manifest | Lets the engine discover, name, and route the model with no hard-coded wiring |
| WIT interface mirror | — | Mirrors the contract for the sandboxed tiers so a guest model speaks the identical shape |

Because the contract is **frozen and unversioned**, a model written against it today keeps working as Celnet evolves — there is exactly one current contract, never an N/N-1 negotiation. A registered model is indistinguishable, at the call site, from a built-in one: the registry is tier-blind, so the engine routes a private smile model or a bespoke pricer through the same path that serves the standard catalogue. The desk's quants extend coverage — beyond the first-generation vanilla, smile, and exotics catalogue — to bespoke and structured products purely by implementing these traits.

### 5.2 The tiered host — performance and isolation, never one or the other

Not every model carries the same trust. A house model authored by the firm's own quants wants native speed; a model under evaluation, or one carrying third-party code, wants containment. The plugin host offers a ladder of tiers behind the **same** contract, so the choice is a routing decision, not a rewrite:

| Tier | Execution | Isolation posture | Typical use |
|------|-----------|-------------------|-------------|
| Native | In-process, native speed | Trusted, first-party | House models on the hot path |
| WebAssembly sandbox | Compute-budgeted guest | No ambient OS access; capability-scoped | Untrusted or under-evaluation models |
| Signed shared-object | Loaded native object | Cryptographically signed provenance | Vetted partner models needing native speed |
| OS-sandboxed | Native under OS confinement | Kernel-enforced (Landlock / seccomp) | Native models needing belt-and-braces containment |

The **WebAssembly sandbox** is the centre of gravity for untrusted code. A guest model runs with **no ambient operating-system authority** — it cannot open files, sockets, or clocks. The capability boundary is deliberately narrow: only the core mathematical primitives the SDK guarantees — `exp`, `ln`, `sqrt`, `norm_pdf`, `norm_cdf` — cross into the guest, and nothing else. Values are marshalled across a strict ABI with boundary NaN-canonicalisation, so a malformed number can never leak non-determinism into the engine. Every guest call runs under a **per-call compute budget**: an errant or runaway model fails with a typed error rather than hanging the engine, so a buggy plugin can never become an availability incident on the pricing path.

### 5.3 Determinism, replay, and the arbitrage self-check

Determinism is a platform guarantee, not a hope. Every tier produces **bit-identical** results on replay — the same inputs reproduce the same outputs to the last bit, run after run, machine after machine — which is what makes plugin-priced trades auditable and reproducible across the firm. The host carries a deterministic replay harness so a model's behaviour can be re-derived exactly from its inputs. And a smile model loaded through the SDK is held to the same standard as the built-ins: a **butterfly no-arbitrage self-check** is built in, so a plugin cannot quietly publish an arbitrageable surface.

### 5.4 What it means for the desk

- **Private IP stays private.** Proprietary pricers and smiles run *inside* Celnet's engine — sandboxed, deterministic, hot-loadable — and the source never leaves the firm.
- **Model evolution without downtime.** New or revised models load into the running engine and inherit Celnet's zero-downtime, blue-green state handoff; there is no rebuild-and-redeploy of the platform to ship a model change.
- **No fork, ever.** The frozen SDK means a desk extends Celnet rather than maintaining a private branch of it — bespoke and structured-product coverage is additive, and upstream improvements arrive cleanly.
- **One discipline for every model.** House models, partner models, and experimental models all live behind the same contract, the same arbitrage gates, the same replay guarantee — so a model's trust tier changes how it is *contained*, never how it is *called* or *trusted to be correct*.

This is Celnet's sharpest line of differentiation. A closed terminal gives a desk the vendor's models; a modern library gives a desk code but not an engine. Celnet gives both — the desk's own models, running at native speed where they are trusted and under hard isolation where they are not, all through one clean, frozen, deterministic contract.

---
<sub>[← Quant & Pricing Methodology Coverage](04-quant-coverage.md)  ·  **[Contents](../CELNET-CAPABILITIES.md)**  ·  [Risk Management →](06-risk-management.md)</sub>
