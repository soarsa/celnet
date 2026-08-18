# Celnet documentation — index

86 documents sit at the root of `docs/`. This index is the map. It is grouped by **what you
are trying to do**, and it marks each document's currency honestly, because most of this
corpus was written during a specific wave and has not been revisited since.

> **Currency legend** — `LIVE` maintained and current · `REFERENCE` stable, rarely needs
> change · `HISTORICAL` describes a completed or superseded wave; kept for the decision
> trail, do not plan against it.
>
> Measured 2026-08-17: **142 of 167 markdown files under `docs/` had not been touched in
> over 30 days**, while the code moved 1,370 commits. Treat an undated claim in a
> `HISTORICAL` document as unverified.

## Start here

| Document | What it is | Currency |
|---|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | System architecture, crate layout, concurrency and latency model | `REFERENCE` ⚠ predates 12 crates |
| [ROADMAP.md](ROADMAP.md) | Phase plan and crate-ownership workstreams | `LIVE` |
| [INTERFACES.md](INTERFACES.md) | The frozen-interface registry — current contracts and their deferrals | `LIVE` |
| [adr/README.md](adr/README.md) | Numbered architecture decisions | `LIVE` |
| [IMPLEMENTATION-LEDGER.md](IMPLEMENTATION-LEDGER.md) | Append-only progress log, newest first | `LIVE` |
| [PARALLEL-SESSIONS.md](PARALLEL-SESSIONS.md) | Lane board and the tiered-gate law (§4.2) | `LIVE` |

⚠ **The architecture family is triplicated and all three are stale.** `ARCHITECTURE.md`
(as-built), [ARCHITECTURE-TARGET.md](ARCHITECTURE-TARGET.md) (intended) and
[ARCHITECTURE-DETERMINATION.md](ARCHITECTURE-DETERMINATION.md) (how the target was chosen)
overlap and were last revised 2026-06-27 … 2026-07-01. Consolidating them into one current
as-built document plus an archived decision trail is an open task.

## Analytics, pricing and risk

| Document | Currency |
|---|---|
| [ANALYTICS-SPEC.md](ANALYTICS-SPEC.md) · [ANALYTICS-REQUIREMENTS.md](ANALYTICS-REQUIREMENTS.md) | `REFERENCE` |
| [CONVENTIONS.md](CONVENTIONS.md) — FX conventions mapped to `celnet-types` | `REFERENCE` |
| [RISK-HIERARCHY.md](RISK-HIERARCHY.md) — the risk aggregation model | `REFERENCE` |
| [RISK-MODEL-REQUIREMENTS-AND-GAPS.md](RISK-MODEL-REQUIREMENTS-AND-GAPS.md) · [RISK-MODELS-AND-DV01-STRATEGY.md](RISK-MODELS-AND-DV01-STRATEGY.md) | `LIVE` |
| [RISK-TRANSFER-REQUIREMENTS.md](RISK-TRANSFER-REQUIREMENTS.md) | `LIVE` |
| [SURFACE-WORKFLOW.md](SURFACE-WORKFLOW.md) — vol-surface marking and publication | `REFERENCE` |
| [GPU-AT-SCALE-PLAN.md](GPU-AT-SCALE-PLAN.md) | `HISTORICAL` ⚠ the GPU path is built but unreachable from the server |
| [SCALE-OUT.md](SCALE-OUT.md) · [TRADING-UNIVERSE-SCALE.md](TRADING-UNIVERSE-SCALE.md) | `REFERENCE` |

## Fixed income

[FI-PRICING-ENGINE-DESIGN.md](FI-PRICING-ENGINE-DESIGN.md) ·
[FI-PRICING-GROUPS-DESIGN.md](FI-PRICING-GROUPS-DESIGN.md) ·
[FI-CREDIT-ENGINE-DESIGN.md](FI-CREDIT-ENGINE-DESIGN.md) ·
[FI-BOOK-CONCEPTS.md](FI-BOOK-CONCEPTS.md) ·
[FI-AGGREGATED-BOOK-REQUIREMENTS.md](FI-AGGREGATED-BOOK-REQUIREMENTS.md) ·
[FI-RISK-ROUTING-REQUIREMENTS.md](FI-RISK-ROUTING-REQUIREMENTS.md) ·
[FI-TIERING-RESEARCH.md](FI-TIERING-RESEARCH.md) ·
[FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md](FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md) ·
[BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md](BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md) ·
[CORPORATE-ACTION-MONITOR-GAP-ANALYSIS.md](CORPORATE-ACTION-MONITOR-GAP-ANALYSIS.md) ·
[CURVES-AND-INSTRUMENT-REFERENCE-DATA-REVIEW.md](CURVES-AND-INSTRUMENT-REFERENCE-DATA-REVIEW.md) ·
[fixed-income/](fixed-income/) — mostly `LIVE`, the July–August work area.

## Hedging and execution

Start at [HEDGING-AND-RISK-EXIT.md](HEDGING-AND-RISK-EXIT.md) (as-built: how risk lands, is
measured, and exits a book). [HEDGING-CONFIGURATION-GUIDE.md](HEDGING-CONFIGURATION-GUIDE.md)
is the trader walkthrough. Then
[AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md](AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md) ·
[INVENTORY-SKEW-ENGINE-GAP-ANALYSIS.md](INVENTORY-SKEW-ENGINE-GAP-ANALYSIS.md) ·
[DECISION-AUDIT.md](DECISION-AUDIT.md) ·
[TRADER-RULE-ENGINE-SETUP.md](TRADER-RULE-ENGINE-SETUP.md) ·
[LATENCY-AND-HEDGING-ANALYTICS-REQUIREMENTS.md](LATENCY-AND-HEDGING-ANALYTICS-REQUIREMENTS.md). All `LIVE`.

## Clients, API and integration

| Document | Currency |
|---|---|
| [API-CLIENTS.md](API-CLIENTS.md) · [CLIENT-PARITY-MATRIX.md](CLIENT-PARITY-MATRIX.md) | `REFERENCE` ⚠ measured coverage is GUI 112/112, Excel 24/112, SDK ~24/116, CLI ~11/116 |
| [EXCEL-INTEGRATION.md](EXCEL-INTEGRATION.md) · [EXCEL-ADDIN-LOCAL-BRINGUP.md](EXCEL-ADDIN-LOCAL-BRINGUP.md) | `REFERENCE` |
| [GUI-EXPERIENCE-DESIGN.md](GUI-EXPERIENCE-DESIGN.md) · [EXPERIENCE-ARCHITECTURE.md](EXPERIENCE-ARCHITECTURE.md) · [gui-redesign/](gui-redesign/) | `REFERENCE` |
| [GUI-DESIGN.md](GUI-DESIGN.md) | `HISTORICAL` (2026-05-30) — superseded by GUI-EXPERIENCE-DESIGN |
| [CELER-INTEGRATION.md](CELER-INTEGRATION.md) · [CELER-FIX-INTEGRATION-PLAN.md](CELER-FIX-INTEGRATION-PLAN.md) | `REFERENCE` |
| [FIX-API.md](FIX-API.md) · [FIX-SIM-DESIGN.md](FIX-SIM-DESIGN.md) · [SIMULATOR-SERVICE-IDENTITIES.md](SIMULATOR-SERVICE-IDENTITIES.md) | `REFERENCE` |
| [CELNET-CONNECTIVITY-INTEGRATION.md](CELNET-CONNECTIVITY-INTEGRATION.md) | `LIVE` — design for the unmerged `conn/framework` work |
| [PLUGIN-HOST-ALT.md](PLUGIN-HOST-ALT.md) | `REFERENCE` |

## Operations, verification and delivery

[VERIFICATION-CONTRACT.md](VERIFICATION-CONTRACT.md) — the gate that forbids shipping a
product family without both a golden vector and an independent-oracle parity row; enforced by
`tools/check-verification-coverage.mjs`. Then [HARDENING.md](HARDENING.md) ·
[OBSERVABILITY.md](OBSERVABILITY.md) · [DEPLOYMENT-MODES.md](DEPLOYMENT-MODES.md) ·
[DELIVERY-MODEL.md](DELIVERY-MODEL.md) · [ORCHESTRATION.md](ORCHESTRATION.md) ·
[SECURITY-AUTHZ-FINDING.md](SECURITY-AUTHZ-FINDING.md) ·
[PERMISSIONS-GRANULAR-REVIEW.md](PERMISSIONS-GRANULAR-REVIEW.md).

## Positioning and research

[COMPETITIVE-ANALYSIS.md](COMPETITIVE-ANALYSIS.md) ·
[CAPABILITIES-VS-COMPETITION.md](CAPABILITIES-VS-COMPETITION.md) ·
[CELNET-CAPABILITIES.md](CELNET-CAPABILITIES.md) ·
[CAPABILITIES-REVITALISATION-PLAN.md](CAPABILITIES-REVITALISATION-PLAN.md) ·
[SOTA-2026.md](SOTA-2026.md) · [SOTA-MESSAGING-ENCODING.md](SOTA-MESSAGING-ENCODING.md) ·
[_research/](_research/). Analysis documents — they describe the market, not the build.

## Historical — completed waves and superseded programmes

Kept for the decision trail. **Do not plan against these.**

[W1-CORE-PLAN.md](W1-CORE-PLAN.md) · [W2-LINEAR-PLAN.md](W2-LINEAR-PLAN.md) ·
[W3-CRYPTO-PLAN.md](W3-CRYPTO-PLAN.md) · [W4-STRUCTURED-RFQ-PLAN.md](W4-STRUCTURED-RFQ-PLAN.md) ·
[W5-CROSSASSET-RISK-PLAN.md](W5-CROSSASSET-RISK-PLAN.md) ·
[POST-W2-INTEGRATION-MANIFEST.md](POST-W2-INTEGRATION-MANIFEST.md) ·
[GW-FOUNDATION-PLAN.md](GW-FOUNDATION-PLAN.md) · [MASTER-PLAN.md](MASTER-PLAN.md) ·
[MASTER-EVOLUTION-PROGRAM.md](MASTER-EVOLUTION-PROGRAM.md) ·
[LEADERSHIP-PROGRAM.md](LEADERSHIP-PROGRAM.md) · [COMPLETION-PROGRAM.md](COMPLETION-PROGRAM.md) ·
[POST-COMPLETION-AUDIT.md](POST-COMPLETION-AUDIT.md) · [POST-GA-ROADMAP.md](POST-GA-ROADMAP.md) ·
[GA-READINESS.md](GA-READINESS.md) · [RELEASE-1.0-RC.md](RELEASE-1.0-RC.md) ·
[NEXT-WORKFLOWS.md](NEXT-WORKFLOWS.md) · [REVIEW-REMEDIATION.md](REVIEW-REMEDIATION.md) ·
[DOWNSTREAM-EXECUTION-MAP.md](DOWNSTREAM-EXECUTION-MAP.md) ·
[AUDIT-ADR0008-CONFORMANCE.md](AUDIT-ADR0008-CONFORMANCE.md) ·
[FIXED-INCOME-EXCEL-INTEGRATION-REVIEW.md](FIXED-INCOME-EXCEL-INTEGRATION-REVIEW.md) ·
[WORLD-CLASS-BACKLOG.md](WORLD-CLASS-BACKLOG.md).

Fully retired documents live in [archive/](archive/).

## Subdirectories

`adr/` decisions · `acceptance/` machine-readable acceptance targets · `architecture/` rendered
architecture page · `assets/` figures (script-generated — see `tools/render-capability-figures.mjs`) ·
`celnet-capabilities/` + `_capabilities-build/` capabilities-document build inputs ·
`fixed-income/` FI design + mockups · `gui-redesign/` surface mockup corpus ·
`plan/` active plans · `_research/` source research · `archive/` retired.

## House rules

1. **State currency.** A document that does not say when it was last verified will be read as
   current and mislead someone. Date your claims.
2. **One home per topic.** If you are writing something that belongs in an existing document,
   edit that document. The architecture triplication above is what happens otherwise.
3. **Never put client-confidential material here.** Customer responses and named-institution
   material do not belong in the product repository.
4. **Derived artefacts are not documentation.** Rendered PDFs and figure PNGs are build output;
   the Markdown source and the render script are the artefacts that matter.
