# Fixed Income & Rates Reference Library

This directory is the consolidated home of the **fixed-income (rates, cash bonds, credit, and reference data)** design corpus for Celnet.

The fixed-income subsystem is implemented as first-class asset-class leaves riding the unified carry and contract seams (`celnet-rates`, `celnet-rates-risk`, `celnet-bond`, `celnet-corpactions`, `celnet-refdata`, `celnet-refstore`, `celnet-tiering`, `celnet-aggregation`, `celnet-lp-sim`, `celnet-cme-sim`).

---

## Core Specifications & Architecture

| Document | Description | Status |
|---|---|---|
| [FI-ARCHITECTURE.md](FI-ARCHITECTURE.md) | Fixed Income system architecture, crate layout, multi-curve data flow, and gRPC/WS services. | `REFERENCE` |
| [FI-CONVENTIONS.md](FI-CONVENTIONS.md) | Rates and bond market conventions (day-counts, BDC, settlement lag, fixing calendars). | `REFERENCE` |
| [FI-CURVES-SPEC.md](FI-CURVES-SPEC.md) | Multi-curve construction, OIS/SOFR bootstrap, discount factors, and interpolation methods. | `REFERENCE` |
| [FI-PRICING-ENGINE-DESIGN.md](FI-PRICING-ENGINE-DESIGN.md) | Cash bond and credit pricing engine design (`celnet-bond`, Newton-Raphson YTM, DV01/duration). | `REFERENCE` |
| [FI-PRICING-GROUPS-DESIGN.md](FI-PRICING-GROUPS-DESIGN.md) | FI Pricing Groups: trader-composable pricing pipeline, spreads, and feature cards. | `REFERENCE` |
| [FI-CREDIT-ENGINE-DESIGN.md](FI-CREDIT-ENGINE-DESIGN.md) | Credit pricing engine design (`celnet-credit`), survival curves, and credit default swaps. | `REFERENCE` |
| [FI-STATUS.md](FI-STATUS.md) | Implementation and wiring status across all fixed income crates and client surfaces. | `LIVE` |

---

## Books, Risk Routing & Execution

| Document | Description | Status |
|---|---|---|
| [FI-BOOK-CONCEPTS.md](FI-BOOK-CONCEPTS.md) | Concepts: Composite Aggregated Book vs Trading Book vs Risk Portfolios. | `REFERENCE` |
| [FI-AGGREGATED-BOOK-REQUIREMENTS.md](FI-AGGREGATED-BOOK-REQUIREMENTS.md) | Composite multi-venue liquidity book aggregation requirements. | `REFERENCE` |
| [FI-RISK-ROUTING-REQUIREMENTS.md](FI-RISK-ROUTING-REQUIREMENTS.md) | FI risk routing rules engine, portfolio trees, and routed-risk rollups. | `REFERENCE` |
| [FI-TIERING-RESEARCH.md](FI-TIERING-RESEARCH.md) | Outbound price tiering: client markups, inventory skewing, and client tier matrix. | `REFERENCE` |
| [BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md](BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md) | Reference data sourcing, bond master, and corporate action processing (`celnet-refdata`, `celnet-corpactions`). | `REFERENCE` |

---

## Verification & Research Foundation

| Document | Description | Status |
|---|---|---|
| [FI-VERIFICATION-CONTRACT.md](FI-VERIFICATION-CONTRACT.md) | Per-instrument golden vector and oracle validation plan (QuantLib golden oracle). | `REFERENCE` |
| [FIXED-INCOME-RESEARCH-BRIEF.md](FIXED-INCOME-RESEARCH-BRIEF.md) | Foundational research charter, vendor benchmarks, and scope boundaries. | `REFERENCE` |
| [OPEN-QUESTIONS.md](OPEN-QUESTIONS.md) | Product and operational design decisions and resolution log. | `REFERENCE` |

---

## Related Root Anchors

- [ARCHITECTURE.md](../ARCHITECTURE.md) — Multi-crate Cargo workspace and overall engine architecture.
- [INTERFACES.md](../INTERFACES.md) — Current frozen contracts for rates and bonds (`RatesService`).
- [VERIFICATION-CONTRACT.md](../VERIFICATION-CONTRACT.md) — Platform-wide golden oracle verification contract.
