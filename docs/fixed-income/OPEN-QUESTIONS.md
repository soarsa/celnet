# Fixed Income — open questions for the operator

Decisions only the business can make, surfaced by (and to be expanded by) the research agent.
Resolve these to lock phasing before any implementation. See
[`FIXED-INCOME-RESEARCH-BRIEF.md`](./FIXED-INCOME-RESEARCH-BRIEF.md) §9.

## Locked build direction (operator, 2026-06-22)

These are settled — the research agent's `FI-ARCHITECTURE.md` must target them:

- **D1 — Own crate(s).** Fixed income gets its **own crate family**, separate from the FX
  crates: a dedicated pricing/analytics crate (e.g. `celnet-rates` for curves + linear rates,
  with a later `celnet-rates-vol` for swaptions/caps), depending only on the shared seams
  (`celnet-types`/`celnet-conventions`/`celnet-calendar`/`celnet-core`) — never folded into
  `celnet-vanilla`/`celnet-linear`. The agent proposes the exact crate split + dependency arrows.
- **D2 — GUI asset-class tabs.** The GUI gains a **top-level tab structure to switch between
  Options and Fixed Income** (an asset-class layer above the workspace rail), so each domain has
  its own workspace set. The agent's UX/architecture section designs how the existing rail
  (Ticket/Stream/Surface/Risk/Book…) coexists with a Fixed-Income workspace set under the tabs.

---

| # | Question | Why it matters | Options / default | Decision |
|---|---|---|---|---|
| Q1 | **P0 market & currency scope** — which RFR curves/currencies first? | Sizes the curve/calibration work and reference data. | USD-SOFR + EUR-€STR + GBP-SONIA, or USD-only to start. | _open_ |
| Q2 | **Cash vs derivatives first** — bond analytics or the swap/curve engine as the wedge? | Determines the P0 crate and the first client-visible value. | Curve+swap engine first (most reuse of carry/discount seams). | _open_ |
| Q3 | **Credit (CDS/ISDA model) — now or later?** | A distinct model + data dependency (survival curves, recovery). | Defer to a later workstream. | _open_ |
| Q4 | **Collateral / CSA discounting depth for v1.** | Single-curve OIS vs full multi-CSA is a large complexity step. | Single OIS-discount curve in v1; multi-CSA later. | _open_ |
| Q5 | **Real-time vs analytics-first.** | Whether FI needs the streaming RFS hot path on day one. | Request/response pricing + risk first; streaming after. | _open_ |
| Q6 | **Reference-data sourcing** (bond static, calendars, fixings) under the no-commercial-feed guardrail. | Determines feasibility/openness of inputs. | Open/published sources + operator-supplied static. | _open_ |
| Q7 | **Vol / optionality in scope for v1?** | Swaptions/caps need a vol cube + SABR; big step beyond linear. | Linear rates first; vol as the next workstream. | _open_ |
| Q8 | **Exclude rateslib entirely?** | Research pass-1 found rateslib is **source-available NON-commercial (not OSS)** — a cargo-deny/licence trap; excellent rates coverage but unusable as a dep, arguably even as a commercial-pipeline oracle without a paid licence. | Exclude as dep + oracle; QuantLib+ORE suffice. | _open_ |
| Q9 | **FinancePy as out-of-process oracle only?** | FinancePy is **GPL-3.0**: fine as a disposable separately-invoked CLI oracle (no linkage), forbidden as a dependency. | Allow as secondary CLI oracle only; never linked. | _open_ |
| Q10 | **Default curve interpolation for P0.** | Log-linear-DF (simple, local) vs Hagan-West monotone-convex forwards (smoother, no negative forwards). | Monotone-convex forwards default; log-linear-DF available. | _open_ |
| Q11 | **STIR-futures convexity adjustment in P0?** | The futures↔FRA convexity adj. needs a short-rate vol model — pulls vol forward into the "linear" P0. | Defer (use a placeholder/zero adj. flagged) until the vol workstream. | _open_ |
| Q12 | **Calendar & fixing-data sourcing** under the no-commercial-feed guardrail. | Holiday calendars + RFR fixings are needed inputs; must come from open/published sources or operator static. | Published central-bank/exchange calendars + operator-supplied fixings. | _open_ |
| Q13 | **CME published conversion factors as a third futures oracle?** | CME CFs give a rare independent anchor for bond-future CF/CTD validation. | Use CME CFs as a third anchor alongside QuantLib + the structural identity. | _open_ |

The agent appends new questions here as research surfaces them; the operator fills the
**Decision** column. Q8–Q13 were surfaced by research pass 1
(`docs/_research/fixed-income-findings.md`).
