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

## Corpus the agent produces (not written yet)

These are the deliverables the brief commissions (§6), mirroring the FX corpus shape:

| Doc | Purpose |
|---|---|
| `FI-ANALYTICS-SPEC.md` | Products, payoffs, models, numerical methods, sensitivities (cited). |
| `FI-CURVES-SPEC.md` | Multi-curve construction (OIS/RFR discounting, projection, calibration). |
| `FI-CONVENTIONS.md` | Day-count / BDC / RFR-observation / fixing / calendar config schema. |
| `FI-ARCHITECTURE.md` | New crates + dependency arrows, additive proto arms, five-client surface. |
| `FI-COMPETITIVE-ANALYSIS.md` | Vendor capability benchmark + OSS reference/gap analysis. |
| `FI-VERIFICATION-CONTRACT.md` | Per-product oracle + independent-identity validation plan. |
| `FI-ROADMAP.md` | Phased `W-FI-*` workstreams with tracks, crates, gates, parallel lanes. |
| `../_research/fixed-income-findings.{md,json}` | The raw, cited research findings. |

## Status

- 2026-06-21 — base created on branch `feature/fixedincome`: the research brief + this index +
  the open-questions seed. No specs written, no implementation authorised yet.
