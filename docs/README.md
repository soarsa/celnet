# Celnet Documentation — Master Reference Index

Welcome to the Celnet documentation corpus. Celnet is a state-of-the-art FX-options and multi-asset pricing, analytics, and risk platform written in pure Rust.

This index is the **master navigation map**. It is organized into **Core Reference Anchors** (the essential system specifications kept at the root) and **Specialized Domain Reference Libraries** (curated into dedicated subdirectories). All historical wave plans, retired audits, and legacy drafts have been consolidated into the [Archive](archive/README.md).

---

## 1. Core Reference Anchors (Root)

These 15 canonical documents define the system's architecture, contracts, quantitative standards, and execution rules. They are the primary entrypoints for engineering and operations:

| Document | Topic & Scope | Currency |
|---|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | **System Architecture (As-Built)**: 65+ crate Cargo workspace, hot-core / async-edge split, lock-free fan-out, Wasm plugin sandbox, zero-downtime upgrades. | `REFERENCE` (65+ crates) |
| [INTERFACES.md](INTERFACES.md) | **Frozen Interface Registry**: The single, current unversioned contracts across the crates, wire protocols, and core traits. | `LIVE` |
| [ROADMAP.md](ROADMAP.md) | **Implementation Roadmap**: Phased evolution, crate workstream ownership, and parallel session lanes. | `LIVE` |
| [PARALLEL-SESSIONS.md](PARALLEL-SESSIONS.md) | **Parallel Development Mesh**: Live lane board, lock-free worktree rules, and the Tiered-Gate Law (§4.2). | `LIVE` |
| [IMPLEMENTATION-LEDGER.md](IMPLEMENTATION-LEDGER.md) | **Implementation Ledger**: Append-only chronological history of landed milestones, gates, and oracle verifications. | `LIVE` |
| [ANALYTICS-SPEC.md](ANALYTICS-SPEC.md) | **FX Options Analytics Spec**: Closed-form Garman-Kohlhagen, Greeks (1st/2nd/higher), strike↔delta root solvers, and smile dynamics. | `REFERENCE` |
| [CONVENTIONS.md](CONVENTIONS.md) | **FX Market Conventions**: Per-(pair, tenor) delta conventions (spot/forward, premium-adjusted), ATM styles (DNS/ATMF), and cut-offs. | `REFERENCE` |
| [HEDGING-AND-RISK-EXIT.md](HEDGING-AND-RISK-EXIT.md) | **Hedging & Risk Exit (As-Built)**: How risk lands in a book, is aggregated, and the exact offsetting-leg exit mechanism. | `LIVE` |
| [COMPETITIVE-ANALYSIS.md](COMPETITIVE-ANALYSIS.md) | **Competitive Analysis & Moats**: Competitor critique (SynOption, Fenics/kACE, Murex, Bloomberg) and platform positioning. | `REFERENCE` |
| [CELER-INTEGRATION.md](CELER-INTEGRATION.md) | **Celer Integration Map**: Native integration with Celer Trader, trade-lifecycle estate, and market-data vendor feeds. | `REFERENCE` |
| [SCALE-OUT.md](SCALE-OUT.md) | **Horizontal Scale-Out Architecture**: Shard-by-pair HRW partitioning, Raft log replication, and cross-fleet risk fan-out. | `REFERENCE` |
| [VERIFICATION-CONTRACT.md](VERIFICATION-CONTRACT.md) | **Verification Contract**: Per-asset-class golden vector and independent golden-oracle parity gates (QuantLib golden oracle). | `LIVE` |
| [HARDENING.md](HARDENING.md) | **Hardening Gates (WS-T)**: Mutation-to-zero testing floors, fuzzing suites, sanitizer verification, and memory safety gates. | `LIVE` |
| [CELNET-CAPABILITIES.md](CELNET-CAPABILITIES.md) | **Master Capabilities Overview**: Comprehensive capability map, brochure index, and cross-surface proofs. | `LIVE` |
| [celnet-capabilities.html](celnet-capabilities.html) | **Visual Capability Showcase**: Single-page standalone interactive brochure with vector figures. | `BROCHURE` |
| [architecture/API-TARGET-ARCHITECTURE.html](architecture/API-TARGET-ARCHITECTURE.html) | **Target API Architecture**: Comprehensive API review, structural critique, 8 target bounded services, universal protobuf contracts, and integration blueprint. | `TARGET SPEC` |
| [presentation/README.md](presentation/README.md) | **Presentation Decks**: Self-contained HTML decks for showing the product — FI screen wireframes + architecture. | `LIVE` |

---

## 2. Specialized Domain Reference Libraries

Detailed specifications, algorithmic blueprints, and operational guides are organized into logical domain libraries:

```
docs/
├── architecture/      # Deep-dive system architecture, determinations & scale
├── quant/             # Quantitative models, curves, volatility & risk analytics
├── fixed-income/      # Fixed Income & Rates design, pricing, curves & reference data
├── hedging/           # Auto-hedging, inventory skew, execution & risk transfer
├── clients/           # Front-end, Excel add-in, SDKs & APIs
├── operations/        # Governance, hardening, observability & security
├── capabilities/      # Platform capabilities, market positioning & brochures
├── adr/               # Architecture Decision Records (ADR-0007 through ADR-0022)
├── plan/              # Sprint implementation specifications & workstream specs
└── archive/           # Historical wave plans, completed sprint checklists & retired audits
```

### 2.1 Architecture & Systems ([architecture/](architecture/README.md))
- [ARCHITECTURE-TARGET.md](architecture/ARCHITECTURE-TARGET.md) — Target architecture and multi-dimensional convergence plan.
- [ARCHITECTURE-DETERMINATION.md](architecture/ARCHITECTURE-DETERMINATION.md) — Chief-architect synthesis and decision trail across all platform dimensions.
- [DEPLOYMENT-MODES.md](architecture/DEPLOYMENT-MODES.md) — Standalone, hybrid, Celer-integrated, and external-feed-only operational deployment topologies.
- [PLUGIN-HOST-ALT.md](architecture/PLUGIN-HOST-ALT.md) — Sandboxed plugin-host architecture using `wasmi` (WebAssembly) and native model registries.
- [GPU-AT-SCALE-PLAN.md](architecture/GPU-AT-SCALE-PLAN.md) — GPU compute abstraction (`celnet-gpu`), `wgpu` pipelines, and CPU SIMD fallbacks.
- [TRADING-UNIVERSE-SCALE.md](architecture/TRADING-UNIVERSE-SCALE.md) — Symbol universe breadth, multi-pair concurrency, and memory budget scaling analysis.
- [SIMULATOR-SERVICE-IDENTITIES.md](architecture/SIMULATOR-SERVICE-IDENTITIES.md) — Architecture and identity mapping for synthetic LP simulation.
- [ORCHESTRATION.md](architecture/ORCHESTRATION.md) — Cross-session task orchestration and multi-agent coordination protocol.
- [CELER-FIX-INTEGRATION-PLAN.md](architecture/CELER-FIX-INTEGRATION-PLAN.md) — Architectural plan for Celer trade-lifecycle ingress and FIX engine integration.

### 2.2 Quantitative Models & Risk ([quant/](quant/README.md))
- [RISK-HIERARCHY.md](quant/RISK-HIERARCHY.md) — Hierarchical OLAP risk aggregation design (firm -> division -> desk -> book -> portfolio).
- [RISK-MODELS-AND-DV01-STRATEGY.md](quant/RISK-MODELS-AND-DV01-STRATEGY.md) — Desk risk strategy, DV01 key-rate ladders, and scenario revaluation.
- [RISK-MODEL-REQUIREMENTS-AND-GAPS.md](quant/RISK-MODEL-REQUIREMENTS-AND-GAPS.md) — Risk model requirements, sensitivity types, and portfolio analytics gaps.
- [SURFACE-WORKFLOW.md](quant/SURFACE-WORKFLOW.md) — Volatility surface marking, calibration workflows, and smile representations (VV, SABR, SVI, SSVI, eSSVI).
- [ANALYTICS-REQUIREMENTS.md](quant/ANALYTICS-REQUIREMENTS.md) — Trading performance analytics (TCA), mark-outs, spread capture, and execution quality metrics.
- [LATENCY-AND-HEDGING-ANALYTICS-REQUIREMENTS.md](quant/LATENCY-AND-HEDGING-ANALYTICS-REQUIREMENTS.md) — Infrastructure latency histograms (p50/p99/p99.9) and real-time hedging analytics.

### 2.3 Fixed Income & Rates ([fixed-income/](fixed-income/README.md))
- [FI-ARCHITECTURE.md](fixed-income/FI-ARCHITECTURE.md) — Fixed Income system architecture, crate layout, and multi-curve data flow.
- [FI-CONVENTIONS.md](fixed-income/FI-CONVENTIONS.md) — Rates and bond market conventions (day-counts, BDC, settlement lag, fixing calendars).
- [FI-CURVES-SPEC.md](fixed-income/FI-CURVES-SPEC.md) — Multi-curve construction, OIS/SOFR bootstrap, discount factors, and interpolation methods.
- [FI-PRICING-ENGINE-DESIGN.md](fixed-income/FI-PRICING-ENGINE-DESIGN.md) — Cash bond and credit pricing engine design (`celnet-bond`, Newton-Raphson YTM, DV01/duration).
- [FI-PRICING-GROUPS-DESIGN.md](fixed-income/FI-PRICING-GROUPS-DESIGN.md) — FI Pricing Groups: trader-composable pricing pipeline, spreads, and feature cards.
- [FI-CREDIT-ENGINE-DESIGN.md](fixed-income/FI-CREDIT-ENGINE-DESIGN.md) — Credit pricing engine design (`celnet-credit`), survival curves, and credit default swaps.
- [FI-BOOK-CONCEPTS.md](fixed-income/FI-BOOK-CONCEPTS.md) — Concepts: Composite Aggregated Book vs Trading Book vs Risk Portfolios.
- [FI-AGGREGATED-BOOK-REQUIREMENTS.md](fixed-income/FI-AGGREGATED-BOOK-REQUIREMENTS.md) — Composite multi-venue liquidity book aggregation requirements.
- [FI-RISK-ROUTING-REQUIREMENTS.md](fixed-income/FI-RISK-ROUTING-REQUIREMENTS.md) — FI risk routing rules engine, portfolio trees, and routed-risk rollups.
- [FI-TIERING-RESEARCH.md](fixed-income/FI-TIERING-RESEARCH.md) — Outbound price tiering: client markups, inventory skewing, and client tier matrix.
- [BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md](fixed-income/BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md) — Reference data sourcing, bond master, and corporate actions.
- [FI-VERIFICATION-CONTRACT.md](fixed-income/FI-VERIFICATION-CONTRACT.md) — Per-instrument golden vector and oracle validation plan.
- [FI-STATUS.md](fixed-income/FI-STATUS.md) — Implementation and wiring status across all fixed income crates and client surfaces.

### 2.4 Hedging, Skew & Execution ([hedging/](hedging/README.md))
- [HEDGING-CONFIGURATION-GUIDE.md](hedging/HEDGING-CONFIGURATION-GUIDE.md) — Trader walkthrough for configuring auto-hedging rules, threshold bands, and hedge venues.
- [AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md](hedging/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md) — Auto-hedging and internalisation engine specifications and state machines.
- [INVENTORY-SKEW-ENGINE-GAP-ANALYSIS.md](hedging/INVENTORY-SKEW-ENGINE-GAP-ANALYSIS.md) — Position-driven inventory skewing model (Avellaneda-Stoikov adaptation) and spread adjustment.
- [TRADER-RULE-ENGINE-SETUP.md](hedging/TRADER-RULE-ENGINE-SETUP.md) — Trader rule engine setup for quote acceptance, risk routing, and execution actions.
- [RISK-TRANSFER-REQUIREMENTS.md](hedging/RISK-TRANSFER-REQUIREMENTS.md) — Inter-desk and inter-book manual risk transfer mechanics and offsetting leg generation.
- [DECISION-AUDIT.md](hedging/DECISION-AUDIT.md) — Decision audit engine: explaining why a rule fired, rejected, or adjusted an action.

### 2.5 Clients, GUI & Integrations ([clients/](clients/README.md))
- [GUI-EXPERIENCE-DESIGN.md](clients/GUI-EXPERIENCE-DESIGN.md) — World-class sell-side trading cockpit: 5 workspaces, multi-asset views, and visual design language.
- [EXPERIENCE-ARCHITECTURE.md](clients/EXPERIENCE-ARCHITECTURE.md) — Unified front-end information architecture, component hierarchy, and navigation flow.
- [CLIENT-PARITY-MATRIX.md](clients/CLIENT-PARITY-MATRIX.md) — Executable parity matrix proving all 18 products reach all five client surfaces bit-identically.
- [API-CLIENTS.md](clients/API-CLIENTS.md) — Trader-centric API philosophy, client SDK design, and typed client ergonomic targets.
- [EXCEL-INTEGRATION.md](clients/EXCEL-INTEGRATION.md) — Excel add-in architecture, real-time streaming, and the `CELNET.*` worksheet function library.
- [EXCEL-ADDIN-LOCAL-BRINGUP.md](clients/EXCEL-ADDIN-LOCAL-BRINGUP.md) — Developer runbook for local bring-up and testing of the Excel add-in and workbook.
- [CELNET-CONNECTIVITY-INTEGRATION.md](clients/CELNET-CONNECTIVITY-INTEGRATION.md) — External gateway and connectivity extension framework architecture.
- [FIX-API.md](clients/FIX-API.md) — Inbound FIX 4.4 acceptor client integration and tag mapping.
- [FIX-SIM-DESIGN.md](clients/FIX-SIM-DESIGN.md) — FIX client-simulator bot design (`fix-sim`) for automated load and flow testing.
- [gui-redesign/](gui-redesign/) — Front-end discovery, design tokens, critiques, and component mockups.

### 2.6 Operations & Governance ([operations/](operations/README.md))
- [PERMISSIONS-GRANULAR-REVIEW.md](operations/PERMISSIONS-GRANULAR-REVIEW.md) — Granular per-feature and per-action capability model across FX options and fixed income.
- [NOTIFICATIONS-REQUIREMENTS.md](operations/NOTIFICATIONS-REQUIREMENTS.md) — Configurable trader notifications, breach alerts, and push messaging delivery.
- [OBSERVABILITY.md](operations/OBSERVABILITY.md) — Zero-hot-path-cost telemetry, wait-free ring buffers, HdrHistogram metrics, and distributed tracing.

### 2.7 Capabilities & Market Analysis ([capabilities/](capabilities/README.md))
- [CAPABILITIES-VS-COMPETITION.md](capabilities/CAPABILITIES-VS-COMPETITION.md) — Feature-by-feature comparison matrix against vendor-neutral incumbent archetypes.
- [SOTA-2026.md](capabilities/SOTA-2026.md) — State-of-the-art technical scan: low-latency runtimes, zero-copy messaging, and numerical engines.
- [SOTA-MESSAGING-ENCODING.md](capabilities/SOTA-MESSAGING-ENCODING.md) — Assessment of IPC fan-out, durable logging, and wire encoding architectures.
- [celnet-capabilities/](celnet-capabilities/) — 14-chapter comprehensive product capability monograph.

### 2.8 Architecture Decision Records ([adr/](adr/README.md))
The numbered, immutable Architecture Decision Records (ADR-0007 through ADR-0022), recording the key architectural turns of the platform.

### 2.9 Historical Archive ([archive/](archive/README.md))
Consolidated repository of completed development waves ([archive/waves/](archive/waves/)), retired release milestones ([archive/milestones/](archive/milestones/)), completed gap audits ([archive/audits/](archive/audits/)), and superseded early drafts ([archive/superseded/](archive/superseded/)).

---

## 3. Documentation Governance & House Rules

1. **Zero Legacy / Zero Stale Duplication:** Always refactor documentation to its cleanest, current state as code evolves. Delete or archive dead documents; do not leave superseded wave plans or pre-build gap analyses polluting active reference directories.
2. **State Currency Honestly:** Every reference document must explicitly declare its status and last-verified date. Never allow undated documents to mislead developers.
3. **One Home Per Topic:** Documentation is partitioned by functional domain. If you are adding details to an existing capability, edit its canonical document rather than creating competing copies.
4. **Automated Link & Structure Integrity:** All documentation files and cross-links are continuously validated by `tools/check-all-doc-links.mjs`. Broken relative links, missing anchors, and ragged tables are build gate failures.
