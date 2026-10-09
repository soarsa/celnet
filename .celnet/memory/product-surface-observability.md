---
name: product-surface-observability
description: "Celnet must ship evolving API clients (trader-workflow-driven), zero-cost observability, and a distributed scale-out posture."
metadata: 
  node_type: memory
  type: feedback
  originSessionId: b315eccc-f521-4987-b5b5-1a21d5710edb
---

Binding product requirements (user directives, 30 May 2026):

- **API clients, evolved by real use.** Ship typed client SDK(s) over the gRPC/WebSocket edge for GUI/API users, and drive **optimal API design by exercising real-like trader/API-user workflows** (RFQ→quote, subscribe-and-stream prices/Greeks, build surface, price+book exotics, risk/scenario) as integration tests. Research + critique competitor APIs (Synoption/Fenics/Bloomberg) and **evolve the wire + client API as capabilities grow** — no versioned APIs ([[api-naming-and-evolution]]), so refactor the single current contract freely toward the cleanest trader-centric ergonomics. (`celnet-client`, task #17.)

- **Observability without performance cost.** Mission-critical instrumentation per best practices — structured logging, distributed tracing (tracing/OpenTelemetry), metrics, and HdrHistogram p50/p99/p99.9 latency — but it must **never diminish performance**: the zero-alloc, core-pinned hot path stays log/lock/alloc-free; telemetry is offloaded over a bounded SPSC queue to a non-critical core (per `docs/ARCHITECTURE.md` §3.3). A perf-guard test asserts instrumentation adds **zero allocations** on the hot path. (task #18.)

- **Distributed / scale-out where it suits the targets.** Continuously assess the architecture for horizontal scale-out to IB-sized portfolios + HFT streaming (stateless pricing replicas, sharding by ccy-pair/tenant, shared arb-free surface distribution, the CelNet distributor fan-out, blue-green across the fleet), validated against the latency/throughput budgets — choosing single-node vs distributed per workload. Document in `docs/SCALE-OUT.md`. (task #19.)

See [[scale-and-performance]], [[no-mocks-policy]], [[celnet-mission]].
