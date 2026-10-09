# Architecture Reference Library

This directory holds the architectural determinations, deep dives, deployment models, compute acceleration specifications, and scale-out analyses for Celnet.

The primary **as-built** architecture document is [ARCHITECTURE.md](../ARCHITECTURE.md) at the documentation root.

---

## Architecture Documents

| Document | Description | Status |
|---|---|---|
| [ARCHITECTURE-TARGET.md](ARCHITECTURE-TARGET.md) | Target architecture and multi-dimensional convergence plan (pricing, API, scale-out, latency, governance). | `REFERENCE` |
| [ARCHITECTURE-DETERMINATION.md](ARCHITECTURE-DETERMINATION.md) | Chief-architect synthesis and decision trail across all platform dimensions. | `REFERENCE` |
| [DEPLOYMENT-MODES.md](DEPLOYMENT-MODES.md) | Standalone, hybrid, CelNet-integrated, and external-feed-only operational deployment topologies. | `REFERENCE` |
| [PLUGIN-HOST-ALT.md](PLUGIN-HOST-ALT.md) | Sandboxed plugin-host architecture using `wasmi` (WebAssembly) and native model registries. | `REFERENCE` |
| [GPU-AT-SCALE-PLAN.md](GPU-AT-SCALE-PLAN.md) | GPU compute abstraction (`celnet-gpu`), `wgpu` pipelines, and CPU SIMD fallbacks. | `REFERENCE` |
| [TRADING-UNIVERSE-SCALE.md](TRADING-UNIVERSE-SCALE.md) | Symbol universe breadth, multi-pair concurrency, and memory budget scaling analysis. | `REFERENCE` |
| [SIMULATOR-SERVICE-IDENTITIES.md](SIMULATOR-SERVICE-IDENTITIES.md) | Architecture and identity mapping for synthetic LP simulation (`celnet-lp-sim`, `celnet-cme-sim`). | `REFERENCE` |
| [ORCHESTRATION.md](ORCHESTRATION.md) | Cross-session task orchestration and multi-agent coordination protocol. | `REFERENCE` |
| [CELNET-FIX-INTEGRATION-PLAN.md](CELNET-FIX-INTEGRATION-PLAN.md) | Architectural plan for CelNet trade-lifecycle ingress and FIX engine integration. | `REFERENCE` |
| [celnet-architecture.html](celnet-architecture.html) | Standalone rendered architectural overview visualization. | `ASSET` |

---

## Related Root Anchors

- [ARCHITECTURE.md](../ARCHITECTURE.md) — As-built system architecture and 55-crate Cargo workspace map.
- [INTERFACES.md](../INTERFACES.md) — Frozen interface registry and current contracts.
- [SCALE-OUT.md](../SCALE-OUT.md) — Distributed scaling, Raft log replication, and HRW fleet routing.
- [CELNET-INTEGRATION.md](../CELNET-INTEGRATION.md) — CelNet trade-lifecycle estate integration map.
