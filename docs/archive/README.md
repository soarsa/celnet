# Celnet Documentation Archive

This directory contains **historical, completed, and superseded** documents.

> [!NOTE]
> **Currency Warning:** Documents in this archive are preserved solely for the **historical decision trail and provenance**. They describe completed development waves, pre-build gap analyses, superseded drafts, or point-in-time milestones. **Do NOT plan new work against these documents.** The active design and current contracts live in the root documents and specialized domain directories.

---

## 1. Development Waves (`waves/`)

Completed development wave plans and early foundational execution charters from May–June 2026:

| Document | Topic | Notes |
|---|---|---|
| [W1-CORE-PLAN.md](waves/W1-CORE-PLAN.md) | Wave 1: Multi-asset core engine foundation | Completed |
| [W2-LINEAR-PLAN.md](waves/W2-LINEAR-PLAN.md) | Wave 2: FX linear products (forwards, swaps, NDFs) | Completed in `celnet-linear` |
| [W3-CRYPTO-PLAN.md](waves/W3-CRYPTO-PLAN.md) | Wave 3: Digital asset / crypto options & linear funding | Completed in `celnet-crypto-vanilla` |
| [W4-STRUCTURED-RFQ-PLAN.md](waves/W4-STRUCTURED-RFQ-PLAN.md) | Wave 4: Structured products and multi-dealer RFQ engine | Completed in `celnet-rfq` / `celnet-exotics` |
| [W5-CROSSASSET-RISK-PLAN.md](waves/W5-CROSSASSET-RISK-PLAN.md) | Wave 5: Cross-asset risk normalization and FRTB buckets | Completed in `celnet-risk-normalize` |
| [GW-FOUNDATION-PLAN.md](waves/GW-FOUNDATION-PLAN.md) | Early GUI foundation execution plan | Superseded by `clients/GUI-EXPERIENCE-DESIGN.md` |
| [MASTER-PLAN.md](waves/MASTER-PLAN.md) | Early master end-to-end plan | Completed |
| [MASTER-EVOLUTION-PROGRAM.md](waves/MASTER-EVOLUTION-PROGRAM.md) | Early capability evolution program | Completed |
| [LEADERSHIP-PROGRAM.md](waves/LEADERSHIP-PROGRAM.md) | Early leadership execution program | Completed |
| [COMPLETION-PROGRAM.md](waves/COMPLETION-PROGRAM.md) | Initial completion program | Completed |
| [POST-W2-INTEGRATION-MANIFEST.md](waves/POST-W2-INTEGRATION-MANIFEST.md) | Wave 2 proto window integration manifest | Completed |
| [POST-W2-EXECUTION-CHECKLIST.md](waves/POST-W2-EXECUTION-CHECKLIST.md) | Wave 2 post-execution checklist | Completed |
| [DELIVERY-MODEL.md](waves/DELIVERY-MODEL.md) | Early lane-based delivery model | Superseded by `PARALLEL-SESSIONS.md` |

---

## 2. Release Milestones (`milestones/`)

Point-in-time release declarations, audit summaries, and historical roadmaps:

| Document | Milestone | Notes |
|---|---|---|
| [RELEASE-1.0-RC.md](milestones/RELEASE-1.0-RC.md) | Celnet 1.0-RC release declaration (2026-06-11) | Completed |
| [GA-READINESS.md](milestones/GA-READINESS.md) | General Availability readiness synthesis | Historical |
| [POST-GA-ROADMAP.md](milestones/POST-GA-ROADMAP.md) | Post-GA capability roadmap | Historical |
| [POST-COMPLETION-AUDIT.md](milestones/POST-COMPLETION-AUDIT.md) | Gap audit after initial completion | Resolved |
| [REVIEW-REMEDIATION.md](milestones/REVIEW-REMEDIATION.md) | Full implementation audit & remediation checklist | Resolved |
| [NEXT-WORKFLOWS.md](milestones/NEXT-WORKFLOWS.md) | Operator runbook for fresh sessions | Historical |
| [RESUME-ANCHOR.md](milestones/RESUME-ANCHOR.md) | Early session resume anchor | Superseded by live memory |

---

## 3. Audits & Pre-Build Gap Analyses (`audits/`)

Historical gap analyses and conformance audits performed prior to or during major crate builds:

| Document | Topic | Resolution |
|---|---|---|
| [AUDIT-ADR0008-CONFORMANCE.md](audits/AUDIT-ADR0008-CONFORMANCE.md) | Conformance audit of carry seam | Remediated in ADR-0008 waves |
| [SECURITY-AUTHZ-FINDING.md](audits/SECURITY-AUTHZ-FINDING.md) | Caller authorization finding | Fully remediated 2026-06-12 |
| [WORLD-CLASS-BACKLOG.md](audits/WORLD-CLASS-BACKLOG.md) | Historical master backlog | Completed / incorporated into crates |
| [DOWNSTREAM-EXECUTION-MAP.md](audits/DOWNSTREAM-EXECUTION-MAP.md) | Early dependency map | Historical |
| [CAPABILITIES-REVITALISATION-PLAN.md](audits/CAPABILITIES-REVITALISATION-PLAN.md) | Capabilities doc-set revitalization plan | Completed (`celnet-capabilities/` shipped) |
| [DOCS-EVOLUTION-PLAN.md](audits/DOCS-EVOLUTION-PLAN.md) | Early documentation evolution plan | Historical |
| [FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md](audits/FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md) | Pre-build bond deal-capture analysis | Completed in `celnet-bond` / `celnet-rates` |
| [CORPORATE-ACTION-MONITOR-GAP-ANALYSIS.md](audits/CORPORATE-ACTION-MONITOR-GAP-ANALYSIS.md) | Pre-build corporate actions gap analysis | Completed in `celnet-corpactions` |
| [CURVES-AND-INSTRUMENT-REFERENCE-DATA-REVIEW.md](audits/CURVES-AND-INSTRUMENT-REFERENCE-DATA-REVIEW.md) | Pre-build reference data review | Completed in `celnet-refdata` / `celnet-refstore` |
| [FIXED-INCOME-EXCEL-INTEGRATION-REVIEW.md](audits/FIXED-INCOME-EXCEL-INTEGRATION-REVIEW.md) | Pre-build Fixed Income Excel review | Completed in `excel/` add-in |

---

## 4. Superseded Drafts (`superseded/`)

Documents that have been directly replaced by newer, authoritative specifications:

| Document | Superseded By |
|---|---|
| [GUI-DESIGN.md](superseded/GUI-DESIGN.md) (May 2026) | [GUI-EXPERIENCE-DESIGN.md](../clients/GUI-EXPERIENCE-DESIGN.md) & [EXPERIENCE-ARCHITECTURE.md](../clients/EXPERIENCE-ARCHITECTURE.md) |
