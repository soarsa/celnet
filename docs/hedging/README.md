# Hedging, Skew & Execution Reference Library

This directory contains specifications, operational guides, and algorithms for auto-hedging, inventory skewing, execution rule engines, and inter-book risk transfer.

The primary **as-built** guide explaining how risk lands, is measured, and exits a book is [HEDGING-AND-RISK-EXIT.md](../HEDGING-AND-RISK-EXIT.md) at the documentation root.

---

## Hedging & Execution Documents

| Document | Description | Status |
|---|---|---|
| [HEDGING-CONFIGURATION-GUIDE.md](HEDGING-CONFIGURATION-GUIDE.md) | Trader walkthrough for configuring auto-hedging rules, threshold bands, and hedge venues. | `REFERENCE` |
| [AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md](AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md) | Auto-hedging and internalisation engine specifications and state machines. | `REFERENCE` |
| [INVENTORY-SKEW-ENGINE-GAP-ANALYSIS.md](INVENTORY-SKEW-ENGINE-GAP-ANALYSIS.md) | Position-driven inventory skewing model (Avellaneda-Stoikov adaptation) and spread adjustment. | `REFERENCE` |
| [TRADER-RULE-ENGINE-SETUP.md](TRADER-RULE-ENGINE-SETUP.md) | Trader rule engine setup for quote acceptance, risk routing, and execution actions. | `REFERENCE` |
| [RISK-TRANSFER-REQUIREMENTS.md](RISK-TRANSFER-REQUIREMENTS.md) | Inter-desk and inter-book manual risk transfer mechanics and offsetting leg generation. | `REFERENCE` |
| [DECISION-AUDIT.md](DECISION-AUDIT.md) | Decision audit engine: explaining why a rule fired, rejected, or adjusted an action. | `REFERENCE` |

---

## Related Root Anchors

- [HEDGING-AND-RISK-EXIT.md](../HEDGING-AND-RISK-EXIT.md) — As-built specification of risk arrival, aggregation, and the offsetting leg exit mechanism.
- [ARCHITECTURE.md](../ARCHITECTURE.md) — Hot core execution loop and low-latency order routing.
