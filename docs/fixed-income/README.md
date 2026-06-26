# Fixed Income — design corpus (work in progress)

This directory is the home of the **fixed-income (rates & credit)** design corpus for Celnet.
It starts as a research effort: a deep-research agent works the brief below and fills in the
spec/plan docs to the same SOTA, OSS-only, golden-oracle-validated bar as the FX-options core.

## Start here

- **[`FIXED-INCOME-RESEARCH-BRIEF.md`](./FIXED-INCOME-RESEARCH-BRIEF.md)** — the **charter** for
  the research agent: mission, guardrails, scope, the research-question map, the deliverables it
  must produce, the methodology, and acceptance criteria. Read this first.
- **[`OPEN-QUESTIONS.md`](./OPEN-QUESTIONS.md)** — decisions the research surfaces that only the
  operator/business can make. Resolve these to lock phasing before implementation.

## Corpus the agent produces

These are the deliverables the brief commissions (§6), mirroring the FX corpus shape. The first four
P0 spec docs are **written** (synthesised from research pass 1); the rest are **not written yet**.

| Doc | Status | Purpose |
|---|---|---|
| [`FI-CURVES-SPEC.md`](./FI-CURVES-SPEC.md) | **present** (P0, pass-1 synthesis) | Multi-curve construction: instruments→curves, DF/zero/forward repr, log-linear-DF vs monotone-convex interpolation, turn/meeting jumps, OIS/CSA discounting (v1 single-OIS), bootstrap vs global solver, Newton/Brent + LM. |
| [`FI-CONVENTIONS.md`](./FI-CONVENTIONS.md) | **present** (P0, pass-1 synthesis) | The `RatesConvention` config schema keyed per (currency, index, tenor): day-count, BDC + adjusted/unadjusted accrual, RFR observation methods, fixing sources, calendars, spot lag — never global. |
| [`FI-VERIFICATION-CONTRACT.md`](./FI-VERIFICATION-CONTRACT.md) | **present** (P0, pass-1 synthesis) | Per-product anti-circular oracle plan (QuantLib primary + ORE risk/FRTB; rateslib/FinancePy excluded with license rationale); each product gets an engine oracle **and** an engine-agnostic identity. Subordinate to the parent `VERIFICATION-CONTRACT.md` (a)–(g). |
| [`FI-ARCHITECTURE.md`](./FI-ARCHITECTURE.md) | **present** (P0, pass-1 synthesis) | D1 `celnet-rates` crate (modules + dependency arrows, shared seams only) + later `celnet-rates-vol`; additive `celnet.proto` arms (CurveSet/RatesInstrument/PricingResult) to all five clients; risk into the server-owned `RiskService`; the D2 GUI Options \| Fixed-Income asset-class tab layer above the rail. |
| `FI-ANALYTICS-SPEC.md` | not written yet | Products, payoffs, models, numerical methods, sensitivities (cited). |
| `FI-COMPETITIVE-ANALYSIS.md` | not written yet | Vendor capability benchmark + OSS reference/gap analysis. |
| `FI-ROADMAP.md` | not written yet | Phased `W-FI-*` workstreams with tracks, crates, gates, parallel lanes. |
| `../_research/fixed-income-findings.{md,json}` | present (pass 1) | The raw, cited research findings the specs are synthesised from. |

## Status

- 2026-06-21 — base created on branch `feature/fixedincome`: the research brief + this index +
  the open-questions seed. No specs written, no implementation authorised yet.
- 2026-06-22 — research pass 1 (`../_research/fixed-income-findings.md`) synthesised into the first
  four P0 spec docs: `FI-CURVES-SPEC.md`, `FI-CONVENTIONS.md`, `FI-VERIFICATION-CONTRACT.md`,
  `FI-ARCHITECTURE.md` (honouring locked D1/D2). Documentation only — no Rust, no crate scaffolding,
  no implementation authorised. Each carries explicit "pending Q*" notes for the unresolved open
  questions (Q1/Q4/Q8/Q9/Q10/Q11/Q12/Q13).
