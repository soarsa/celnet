# Fixed Income — open questions for the operator

Decisions only the business can make, surfaced by (and to be expanded by) the research agent.
Resolve these to lock phasing before any implementation. See
[`FIXED-INCOME-RESEARCH-BRIEF.md`](./FIXED-INCOME-RESEARCH-BRIEF.md) §9.

| # | Question | Why it matters | Options / default | Decision |
|---|---|---|---|---|
| Q1 | **P0 market & currency scope** — which RFR curves/currencies first? | Sizes the curve/calibration work and reference data. | USD-SOFR + EUR-€STR + GBP-SONIA, or USD-only to start. | _open_ |
| Q2 | **Cash vs derivatives first** — bond analytics or the swap/curve engine as the wedge? | Determines the P0 crate and the first client-visible value. | Curve+swap engine first (most reuse of carry/discount seams). | _open_ |
| Q3 | **Credit (CDS/ISDA model) — now or later?** | A distinct model + data dependency (survival curves, recovery). | Defer to a later workstream. | _open_ |
| Q4 | **Collateral / CSA discounting depth for v1.** | Single-curve OIS vs full multi-CSA is a large complexity step. | Single OIS-discount curve in v1; multi-CSA later. | _open_ |
| Q5 | **Real-time vs analytics-first.** | Whether FI needs the streaming RFS hot path on day one. | Request/response pricing + risk first; streaming after. | _open_ |
| Q6 | **Reference-data sourcing** (bond static, calendars, fixings) under the no-commercial-feed guardrail. | Determines feasibility/openness of inputs. | Open/published sources + operator-supplied static. | _open_ |
| Q7 | **Vol / optionality in scope for v1?** | Swaptions/caps need a vol cube + SABR; big step beyond linear. | Linear rates first; vol as the next workstream. | _open_ |

The agent appends new questions here as research surfaces them; the operator fills the
**Decision** column.
