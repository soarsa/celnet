# Celnet — Capabilities, Architecture & Celer Integration

> **A Celer Technologies product.**
> Celnet is a state-of-the-art FX-**options** pricing & risk platform: ultra-low-latency, mission-critical and hot-upgradable. One clean, API-first contract sits behind a React/WebGPU trader GUI, an Excel add-in, a Rust SDK and a CLI — and integrates natively into the Celer trade-lifecycle estate as the FX-options pricing system-of-record.

![Celnet capability landscape](assets/celnet-capabilities/fig-12-capability-landscape.png)
*The Celnet capability landscape — pricing & analytics, risk, the real-time edge, extensibility, clients, and the platform substrate, unified behind one contract.*

This document is a linked set of focused chapters. Start with the **[Executive Summary](celnet-capabilities/01-executive-summary.md)**, scan the **[Capability Map](celnet-capabilities/02-capability-map.md)**, or jump straight to any topic below.

---

## Contents

1. **[Executive Summary — Celnet at a Glance](celnet-capabilities/01-executive-summary.md)** — The one-page positioning and headline capability proof-points.
2. **[Capability Map — What Celnet Does](celnet-capabilities/02-capability-map.md)** — A scannable inventory of every capability, grouped by cluster.
3. **[System Architecture](celnet-capabilities/03-system-architecture.md)** — The two-tier hot-core / async-edge model, lock-free state, hot upgrade and durability.
4. **[Quant & Pricing Methodology Coverage](celnet-capabilities/04-quant-coverage.md)** — Vanilla, the full FX Greek set, the smile/surface engine, and the exotics catalogue.
5. **[Extensibility — The Open Quant SDK & Plugin Host](celnet-capabilities/05-extensibility-plugins.md)** — Run private-IP custom models in-engine, sandboxed, deterministic and hot-loadable.
6. **[Risk Management](celnet-capabilities/06-risk-management.md)** — Book-shaped scenario risk and the firm-wide OLAP position-fact cube.
7. **[Performance & Latency](celnet-capabilities/07-performance-latency.md)** — Why pricing is never the bottleneck, with zero-cost observability.
8. **[Scalability & Scale-Out](celnet-capabilities/08-scalability-scaleout.md)** — The node-local hot substrate and the horizontal scale-out fabric.
9. **[API & Wire Contract + API-First Client Parity](celnet-capabilities/09-api-contract-parity.md)** — One unversioned contract behind every surface, bit-identical.
10. **[Excel Integration](celnet-capabilities/10-excel-integration.md)** — The CELNET.* worksheet functions and the trader workflows they unlock.
11. **[The Trader GUI](celnet-capabilities/11-trader-gui.md)** — A component-level walkthrough of the five-workspace trading cockpit.
12. **[Celer Trader & Estate Integration](celnet-capabilities/12-celer-integration.md)** — Celnet as the FX-options pricing system-of-record in the Celer lifecycle.
13. **[Competitive Positioning](celnet-capabilities/13-competitive-positioning.md)** — How Celnet out-functions, out-intuits and out-performs, proven by executable parity.
14. **[Engineering Rigor & Assurance](celnet-capabilities/14-engineering-rigor.md)** — The validation, determinism and supply-chain guarantees behind every claim.

---

## How Celnet fits together

- **One contract, every surface.** Every capability lives in a single unversioned API; the GUI, Excel, the SDK and the CLI all consume it, so a value is bit-identical across surfaces. → *[API & Client Parity](celnet-capabilities/09-api-contract-parity.md)*
- **Adaptable by design.** Pluggable market-data and product adapters flow in; bit-identical pricing surfaces flow out; the same codebase deploys three ways. → *[System Architecture](celnet-capabilities/03-system-architecture.md)*, *[Celer Integration](celnet-capabilities/12-celer-integration.md)*
- **Extensible at the core.** Desks run their own private-IP models inside the engine through the Open Quant SDK. → *[Extensibility](celnet-capabilities/05-extensibility-plugins.md)*
- **Whole-firm risk.** Book-shaped scenario risk and a firm-wide position-fact cube are two zooms of one model. → *[Risk Management](celnet-capabilities/06-risk-management.md)*

---

## Figure index

| # | Asset | Description | Chapter |
|---|-------|-------------|---------|
| **Fig 1** | [`fig-01-system-architecture-adaptability.png`](assets/celnet-capabilities/fig-01-system-architecture-adaptability.png) | Celnet's adaptable architecture — pluggable market-data and product adapters flow into one celnet-proto contract wrapping a pinned zero-allocation hot core, emit bit-identical pricing surfaces across the GUI, Excel, SDK, CLI, WebSocket, FIX and Celer Trader, and deploy three ways through reversible adapter swaps behind a conflating egress governor. | [§3](celnet-capabilities/03-system-architecture.md) |
| **Fig 2** | [`fig-02-api-first-parity.png`](assets/celnet-capabilities/fig-02-api-first-parity.png) | Celnet exposes one clean, unversioned contract served by the Celnet server and fanned out to six surfaces — gRPC, a byte-identical WebSocket JSON mirror, the Rust client SDK, the CLI, the Excel CELNET.* add-in, and the React/WebGPU GUI — so the front-end uses the same APIs as any client and every value is bit-identical across them all. | [§9](celnet-capabilities/09-api-contract-parity.md) |
| **Fig 3** | [`fig-03-quant-coverage.png`](assets/celnet-capabilities/fig-03-quant-coverage.png) | Celnet's quant and pricing methodology coverage — Garman-Kohlhagen vanilla with the full FX desk Greek set, the volatility smile and arbitrage-free surface engine, and a first-generation exotics catalogue priced by complementary analytic, PDE, Monte-Carlo and Vanna-Volga methods, all validated to machine precision against an independent reference and extensible via the Open Quant SDK. | [§4](celnet-capabilities/04-quant-coverage.md) |
| **Fig 4** | [`fig-04-surface-pipeline.png`](assets/celnet-capabilities/fig-04-surface-pipeline.png) | Celnet's surface pipeline turns broker ATM and risk-reversal/butterfly quotes into a versioned, arbitrage-free marked surface — market-to-smile strangle calibration, a switchable smile-model selector, and butterfly, calendar and vertical no-arbitrage gates — which pricing, RFQ and RFS consume by pinned version, rejecting any unknown version rather than silently falling back to live. | [§4](celnet-capabilities/04-quant-coverage.md) |
| **Fig 5** | [`fig-05-plugin-tiers.png`](assets/celnet-capabilities/fig-05-plugin-tiers.png) | Celnet's Open Quant SDK: one frozen trait contract and a tier-blind model registry route every model — native, WebAssembly-sandboxed, signed shared-object or OS-isolated — through the same calling convention, with only the core math primitives crossing a zero-ambient-authority capability boundary and a built-in butterfly no-arbitrage self-check, so desks run private-IP custom models in-engine without forking Celnet. | [§5](celnet-capabilities/05-extensibility-plugins.md) |
| **Fig 6** | [`fig-06-risk-architecture.png`](assets/celnet-capabilities/fig-06-risk-architecture.png) | Celnet's risk architecture spans book-shaped scenario risk — real repricing across a spot-vol shock grid, bucketed vega, cross-gamma and theta-roll horizons against versioned marked surfaces — and a firm-wide OLAP position-fact cube with convention-canonicalized netting, eight-axis roll-up, cascading limits and entitlement pruning, where Book and Risk are two zooms of one cube exposed identically to GUI, SDK and Excel. | [§6](celnet-capabilities/06-risk-management.md) |
| **Fig 7** | [`fig-07-performance-ladder.png`](assets/celnet-capabilities/fig-07-performance-ladder.png) | A qualitative latency ladder showing that in-core pricing and full-Greek computation sit far at the fast end while the network round-trip dominates the budget, so pricing is never the bottleneck — flanked by zero-cost tail-percentile observability, regression-gated benchmarks, machine-precision validation, and a deterministic build. | [§7](celnet-capabilities/07-performance-latency.md) |
| **Fig 8** | [`fig-08-scaleout.png`](assets/celnet-capabilities/fig-08-scaleout.png) | Celnet's node-local hot substrate prices investment-bank-sized portfolios in-core, while a rendezvous partition map shards the book across the fleet and a conflating egress governor fans continuous two-way prices out to many high-performance counterparties. | [§8](celnet-capabilities/08-scalability-scaleout.md) |
| **Fig 9** | [`fig-09-celer-lifecycle.png`](assets/celnet-capabilities/fig-09-celer-lifecycle.png) | Celnet sits as the FX-options pricing system-of-record inside the Celer trade lifecycle, injecting price, Greeks and surface into the market-data price path and option risk into the pre-trade and position path. | [§12](celnet-capabilities/12-celer-integration.md) |
| **Fig 10** | [`fig-10-deployment-modes.png`](assets/celnet-capabilities/fig-10-deployment-modes.png) | Celnet runs from one codebase in three deployment modes — Standalone, Hybrid and CelerIntegrated — selected only by binding three adapter seams (market-data source, price sink, order and execution), with a conflating egress governor between the pricer and the estate distributor, so Celnet stays the FX-options pricing system-of-record in every mode and migration is a reversible adapter swap. | [§12](celnet-capabilities/12-celer-integration.md) |
| **Fig 11** | [`fig-11-streamsession-clicktrade.png`](assets/celnet-capabilities/fig-11-streamsession-clicktrade.png) | One bidirectional StreamSession multiplexes many subscriptions through Subscribe, Snapshot, sequenced Update, Heartbeat, gap-driven Resync and in-place Modify, then turns a streamed price into a confirmed fill via an unguessable line-bound tradable token with last-look validity and idempotent, typed execution. | [§9](celnet-capabilities/09-api-contract-parity.md) |
| **Fig 12** | [`fig-12-capability-landscape.png`](assets/celnet-capabilities/fig-12-capability-landscape.png) | The Celnet capability landscape: pricing and analytics, risk, the real-time edge, extensibility, clients and platform clustered around the Celnet pricing core and the Celer trade lifecycle. | [§2](celnet-capabilities/02-capability-map.md) |
| **Fig 13** | [`fig-13-engine-concurrency.png`](assets/celnet-capabilities/fig-13-engine-concurrency.png) | Celnet's two-tier engine joins a concurrency-rich async edge (gRPC, byte-identical WebSocket mirror, FIX) to pinned zero-allocation hot cores through wait-free single-producer/single-consumer request and response rings, publishing market state lock-free via an atomically-swapped snapshot, a single-writer seqlock top-of-book and cache-padded counters, with blue-green zero-downtime handoff, a durable checksummed journal, and partition-map scale-out. | [§3](celnet-capabilities/03-system-architecture.md) |

## Screen gallery (live system)

| Asset | Description |
|-------|-------------|
| [`shot-01-stream-blotter.png`](assets/celnet-capabilities/shot-01-stream-blotter.png) | Stream — the live, multiplexed RFS blotter with click-to-trade and the trend selector. |
| [`shot-02-ticket-structuring.png`](assets/celnet-capabilities/shot-02-ticket-structuring.png) | Ticket — the structuring & pricing card: structure, notional, tenor, legs, inline Solve, two-way, conventions on the face. |
| [`shot-03-surface-marking.png`](assets/celnet-capabilities/shot-03-surface-marking.png) | Surface — the smile chart and broker marking grid with the real arbitrage-free Publish gate. |
| [`shot-04-risk-scenario.png`](assets/celnet-capabilities/shot-04-risk-scenario.png) | Risk — the spot×vol reprice grid, vega ladder, cross-gamma stencil and theta-roll. |
| [`shot-05-book-aggregate.png`](assets/celnet-capabilities/shot-05-book-aggregate.png) | Book — the desk-wide net Greeks, per-pair breakdown and aggregate vega ladder, drilling to Risk. |
| [`shot-06-pair-navigator.png`](assets/celnet-capabilities/shot-06-pair-navigator.png) | Pair navigator — the watchlist strip and pair dropdown. |
| [`shot-07-command-palette.png`](assets/celnet-capabilities/shot-07-command-palette.png) | Command palette (⌘K) — fuzzy navigation across pairs, workspaces and actions. |
| [`shot-08-clicktrade-lastlook.png`](assets/celnet-capabilities/shot-08-clicktrade-lastlook.png) | Click-to-trade — the keyed-MAC tradable token and last-look response. |
| [`shot-09-excel-taskpane.png`](assets/celnet-capabilities/shot-09-excel-taskpane.png) | Excel — the connected CELNET task pane: RFQ, Contribute-Mark and the smile-model selector. |
| [`shot-10-excel-grid-branded.png`](assets/celnet-capabilities/shot-10-excel-grid-branded.png) | Excel — CELNET.* functions computing live in the grid: RFQ spill, full Greek spill, smile spill and streaming cells. |

---

<sub>Brand: Celer Technologies (coral `#ff7357` · indigo `#6b6bf5` · Anaheim). Diagrams are vector-rendered from the sources in [`assets/celnet-capabilities/_src/`](assets/celnet-capabilities/_src/); screenshots are captured from the live Celnet GUI and Excel add-in. Figures and chapters cross-link both ways.</sub>
