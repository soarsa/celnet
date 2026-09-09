# Sprint Implementation Plans & Workstream Specifications

This directory contains targeted implementation specifications, lane workstream plans, and engineering execution packs authored during active development sprints.

These documents served as the binding technical blueprints for parallel agent sessions implementing specific architectural changes. The functionality designed here is now merged and active in the codebase.

---

## Sprint Specifications Index

| Document | Workstream & Topic | Status |
|---|---|---|
| [CARRY-SEAM-TO-EDGE.md](CARRY-SEAM-TO-EDGE.md) | Extension of the ADR-0008 carry seam to the streamed edge (multi-asset price fan-out and rate sensitivities). | `MERGED` |
| [CENTRAL-CORE-UNIFICATION.md](CENTRAL-CORE-UNIFICATION.md) | Central best-practice pricing and risk contract (`Priceable`, `MarketResolver`, `ResolvedMarket`). | `MERGED` |
| [MULTI-ASSET-CORE-INTEGRATION.md](MULTI-ASSET-CORE-INTEGRATION.md) | Complete multi-asset re-architecture plan across options, rates, linear, credit, and digital assets. | `MERGED` |
| [TERM-STRUCTURE-UNIFICATION.md](TERM-STRUCTURE-UNIFICATION.md) | Unification of term structures onto `DiscountCurve` substrate (operationalizing ADR-0010). | `MERGED` |
| [B-AUTH-QUOTE-RISK.md](B-AUTH-QUOTE-RISK.md) | Quote service authorization, requester binding, and desk-identity risk narrowing. | `MERGED` |
| [PERMISSIONS-ADMINISTRATION-REQUIREMENT.md](PERMISSIONS-ADMINISTRATION-REQUIREMENT.md) | Specification for granular per-action and per-asset capabilities in `celnet-entitlements`. | `MERGED` |
| [ADR0008-EXOTICS-SURFACE-REMEDIATION.md](ADR0008-EXOTICS-SURFACE-REMEDIATION.md) | Remediation plan bringing `celnet-exotics` and `celnet-surface` onto the asset-agnostic carry seam. | `MERGED` |
| [CRYPTO-SURFACE-LEAF-SPEC.md](CRYPTO-SURFACE-LEAF-SPEC.md) | Strike-axis smile leaf and asset-class-neutral surface core. | `MERGED` |
| [CRYPTO-SURFACE-WIRING-FASTFOLLOW.md](CRYPTO-SURFACE-WIRING-FASTFOLLOW.md) | Fast-follow specification for crypto strike-axis surface ingestion. | `MERGED` |
| [W4A-PIVOT-IMPL-SPEC.md](W4A-PIVOT-IMPL-SPEC.md) | Pivot target-redemption accumulator exotic payoff engine. | `MERGED` |
| [W5A-XRISK-IMPL-SPEC.md](W5A-XRISK-IMPL-SPEC.md) | Cross-asset risk normalization and FRTB bucket assignment (`celnet-risk-normalize`). | `MERGED` |
| [W6-ANALYTICS-RIGOR-PLAN.md](W6-ANALYTICS-RIGOR-PLAN.md) | Mutation-to-zero testing floor and fuzzing suite for numerics crates. | `MERGED` |
| [W6-RESUME-DESIGN-PACK.md](W6-RESUME-DESIGN-PACK.md) | Infrastructure rigor design pack and checkpoint resume state. | `MERGED` |
| [LODESTAR-MIGRATION.md](LODESTAR-MIGRATION.md) | Codebase knowledge graph migration and distributed-agent memory program. | `MERGED` |
| [NEUROSYMBOLIC-GROUNDING.md](NEUROSYMBOLIC-GROUNDING.md) | Lodestar knowledge grounding and formal invariant checking. | `MERGED` |
| [NEXT-ARCHITECTURE-IMPLEMENTATION.md](NEXT-ARCHITECTURE-IMPLEMENTATION.md) | Master phased implementation plan for optimal integrated architecture. | `MERGED` |
| [P1-LANES.md](P1-LANES.md) | Island-wiring implementation lane specification. | `MERGED` |
| [ARCH-REVIEW-FIXEDINCOME.md](ARCH-REVIEW-FIXEDINCOME.md) | Architectural review and consolidation check for fixed-income/rates subsystem. | `MERGED` |
| [JOINT-EXECUTION-PLAN.md](JOINT-EXECUTION-PLAN.md) | Joint multi-session execution division of labor. | `HISTORICAL` |

---

## Related Documents

- [ROADMAP.md](../ROADMAP.md) — The live implementation roadmap.
- [IMPLEMENTATION-LEDGER.md](../IMPLEMENTATION-LEDGER.md) — Append-only chronological record of landed milestones.
- [PARALLEL-SESSIONS.md](../PARALLEL-SESSIONS.md) — Parallel session lane claims and tiered-gate laws.
