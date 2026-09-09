# Quantitative Analytics & Risk Reference Library

This directory contains specifications and designs for quantitative pricing methodology, volatility surface construction, risk hierarchy aggregation, and trading performance analytics.

The primary **FX-options quant and pricing specification** is [ANALYTICS-SPEC.md](../ANALYTICS-SPEC.md), and the market convention specification is [CONVENTIONS.md](../CONVENTIONS.md) at the documentation root.

---

## Quantitative & Risk Documents

| Document | Description | Status |
|---|---|---|
| [RISK-HIERARCHY.md](RISK-HIERARCHY.md) | Hierarchical OLAP risk aggregation design (firm -> division -> desk -> book -> portfolio). | `REFERENCE` |
| [RISK-MODELS-AND-DV01-STRATEGY.md](RISK-MODELS-AND-DV01-STRATEGY.md) | As-built desk risk strategy, DV01 key-rate ladders, and scenario revaluation. | `REFERENCE` |
| [RISK-MODEL-REQUIREMENTS-AND-GAPS.md](RISK-MODEL-REQUIREMENTS-AND-GAPS.md) | Detailed risk model requirements, sensitivity types, and portfolio analytics gaps. | `REFERENCE` |
| [SURFACE-WORKFLOW.md](SURFACE-WORKFLOW.md) | Volatility surface marking, calibration workflows, and smile representation (VV, SABR, SVI, SSVI, eSSVI). | `REFERENCE` |
| [ANALYTICS-REQUIREMENTS.md](ANALYTICS-REQUIREMENTS.md) | Trading performance analytics (TCA), mark-outs, spread capture, and execution quality metrics. | `REFERENCE` |
| [LATENCY-AND-HEDGING-ANALYTICS-REQUIREMENTS.md](LATENCY-AND-HEDGING-ANALYTICS-REQUIREMENTS.md) | Infrastructure latency histograms (p50/p99/p99.9) and real-time hedging analytics. | `REFERENCE` |

---

## Related Root Anchors

- [ANALYTICS-SPEC.md](../ANALYTICS-SPEC.md) — Market-standard FX-options pricing models, Greek formulations, and numerical solvers.
- [CONVENTIONS.md](../CONVENTIONS.md) — FX market conventions (delta styles, ATM definitions, day-counts, cut-offs).
- [VERIFICATION-CONTRACT.md](../VERIFICATION-CONTRACT.md) — Oracle parity gates, QuantLib reference comparisons, and anti-circular validation.
