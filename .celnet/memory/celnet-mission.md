---
name: celnet-mission
description: What Celnet is and its non-negotiable product pillars.
metadata: 
  node_type: memory
  type: project
  originSessionId: b315eccc-f521-4987-b5b5-1a21d5710edb
---

**Celnet** is a state-of-the-art FX Options pricing platform, written in Rust, that integrates into the existing CelNet trade-lifecycle estate and front end. Started 30 May 2026 (greenfield, `/Users/adrian/code/celnet`).

Non-negotiable pillars:
- **Ultra-low-latency**, scalable, mission-critical stability; 100% non-blocking code.
- **Zero-downtime upgrades** (hot-upgradable while running).
- **Fenics integration** for FX-options market data; competes with Synoption, Fenics, Bloomberg OVML et al. on analytics. Goal: out-function, out-intuit, out-perform all competitors.
- **User-extensible analytics & workflows** via SDKs; we ship the full market-standard analytics (as of May 2026) ourselves first.
- **Crate-structured workspace**, AI-maintainable file sizes, optimal for parallel agent sessions.
- **GPU-accelerated** where available (cross-platform: Metal/Vulkan/DX12 via wgpu locally, CUDA on NVIDIA); deployable on macOS/Windows/Linux + containers.

Knowledge base + design docs live in `docs/` (architecture, analytics spec, competitive analysis, CelNet integration map, roadmap), produced by the research/design workflow. See [[no-mocks-policy]], [[parallel-session-model]], [[dev-environment]].
