---
name: ga-push-directives
description: "GA-push directives — optimize-not-version the API, plugin-host alternative to wasmtime, a stunning MacOS-inspired GUI, and a May-2026 SOTA mandate."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: b315eccc-f521-4987-b5b5-1a21d5710edb
---

GA-push directives (user, 30 May 2026):

- **Optimize the API, never version it.** No external users → there is one clean current contract; evolve/refactor it freely toward the cleanest trader-centric ergonomics. Reinforces [[api-naming-and-evolution]]. Apply the wave-5 trader critique (multiplex RFS session, click-to-trade, book-shaped risk, surface versioning as a field not an API version).

- **Resolve upstream blockers — plugin-host needs an alternative architecture.** wasmtime (all releases incl. 38) carries open 2026 RUSTSEC advisories, so it stays out. RESEARCH + CRITIQUE alternative/optimal sandboxed-extensibility architectures (e.g. wasmi pure-Rust interpreter, wasmer, capability-based native, RLBox-style, process isolation) for determinism + fuel/metering + capability security + OSS/advisory-clean, pick the optimal, ADR it, then build. The SDK *contract* (`celnet-plugin-api`) is already built.

- **GUI: out-intuit & out-function SynOption, stunningly beautiful, MacOS-inspired.** Research SynOption (and competitor) front-ends, then design (and build) a clean, beautiful FX-options trader GUI applying the latest (May 2026) usability/design practices and MacOS HIG inspiration, over the typed gRPC/WebSocket client SDK. Workstream doc: `docs/GUI-DESIGN.md`. The `frontend-design` skill is available.

- **State-of-the-art everywhere, May-2026 research.** Continuously apply the latest academic research (as recent as May 2026) across analytics, latency, architecture and design — while remaining 100% compliant with the analytics/compliance/pricing-model correctness and the ultra-low-latency, mission-critical guarantees. Capture in `docs/SOTA-2026.md`.

See [[scale-and-performance]], [[no-commercial-products]], [[product-surface-observability]], [[celnet-mission]].
