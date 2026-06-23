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

## Locked P0 scope (operator, 2026-06-23 — all open questions resolved)

Every question below now carries a ✅ decision. The resulting **P0 is full-breadth** (operator
chose "both" on the cash/derivatives, analytics/streaming, and linear/vol axes) — `FI-ROADMAP.md`
must sequence this by dependency, not treat it as one monolith:

- **D3 — Currencies (Q1):** USD-SOFR + EUR-€STR + GBP-SONIA in P0 (TARGET2/London calendars + the
  EUR/GBP projection-curve basis from day one).
- **D4 — Products (Q2):** **both** the curve/swap engine (FRA/OIS/IRS/basis/futures) **and** cash
  bonds; multi-curve foundation is the shared prerequisite, built first.
- **D5 — Delivery (Q5):** **both** request/response analytics **and** the streaming RFS hot path;
  analytics contract hardens first, streaming wraps the same immutable snapshot.
- **D6 — Vol (Q7):** **both** linear rates **and** rates vol (`celnet-rates-vol`: swaptions/caps +
  SABR cube); the linear engine is validated first, the cube calibrates against it.
- **D7 — Curves (Q4/Q10/Q11):** single OIS/RFR-discount curve per ccy; **log-linear-on-log-DF**
  default interpolation (monotone-convex selectable); **deterministic** STIR convexity placeholder.
- **D8 — Vol detail (Q14–Q18):** normal (bp) primary; `β` fixed per-ccy; no-arb SABR wing; 1F
  Gaussian short rate; CMS via cube replication.
- **D9 — Credit (Q3/Q19/Q20):** deferred to a dedicated `W-FI` lane (`celnet-credit`); ISDA-PL
  source excluded, QuantLib `IsdaCdsEngine` (BSD) is the oracle.
- **D10 — Inflation/OAS (Q21/Q22):** linear-index ILB first (deflation floor fast-follows within
  P0 once the vol layer lands); OAS reuses the 1F Gaussian engine.
- **D11 — Oracles/data (Q6/Q8/Q9/Q12/Q13):** QuantLib + ORE only (rateslib + FinancePy excluded as
  deps); open/published calendars + operator-supplied fixings/bond-static; CME CFs as a third
  futures anchor.

---

| # | Question | Why it matters | Options / default | Decision |
|---|---|---|---|---|
| Q1 | **P0 market & currency scope** — which RFR curves/currencies first? | Sizes the curve/calibration work and reference data. | USD-SOFR + EUR-€STR + GBP-SONIA, or USD-only to start. | ✅ **2026-06-23: USD-SOFR + EUR-€STR + GBP-SONIA** in P0 (full three-currency breadth; pulls in TARGET2/London calendars + the basis lane from day one). |
| Q2 | **Cash vs derivatives first** — bond analytics or the swap/curve engine as the wedge? | Determines the P0 crate and the first client-visible value. | Curve+swap engine first (most reuse of carry/discount seams). | ✅ **2026-06-23: BOTH in P0** — cash bonds **and** the curve/swap engine. Build order is dependency-led (the multi-curve foundation underpins both), not simultaneous. |
| Q3 | **Credit (CDS/ISDA model) — now or later?** | A distinct model + data dependency (survival curves, recovery). | Defer to a later workstream. | ✅ **2026-06-23: defer** to a dedicated `W-FI` lane after the rates engine (scope locked via Q19/Q20; `celnet-credit` sibling crate). |
| Q4 | **Collateral / CSA discounting depth for v1.** | Single-curve OIS vs full multi-CSA is a large complexity step. | Single OIS-discount curve in v1; multi-CSA later. | ✅ **2026-06-23: single OIS/RFR-discount curve per currency in P0**; full multi-CSA / cheapest-to-deliver collateral deferred. (Per-ccy basis still modelled for the EUR/GBP projection curves.) |
| Q5 | **Real-time vs analytics-first.** | Whether FI needs the streaming RFS hot path on day one. | Request/response pricing + risk first; streaming after. | ✅ **2026-06-23: BOTH in P0** — request/response analytics **and** the streaming RFS hot path. Analytics contract hardens first; streaming wraps the same immutable curve/pricing snapshot. |
| Q6 | **Reference-data sourcing** (bond static, calendars, fixings) under the no-commercial-feed guardrail. | Determines feasibility/openness of inputs. | Open/published sources + operator-supplied static. | ✅ **2026-06-23: open/published sources + operator-supplied static** (central-bank/exchange calendars, published RFR fixings, operator-loaded bond static). Calendars/fixings are data, not code. |
| Q7 | **Vol / optionality in scope for v1?** | Swaptions/caps need a vol cube + SABR; big step beyond linear. | Linear rates first; vol as the next workstream. | ✅ **2026-06-23: BOTH in P0** — linear rates **and** rates vol (swaptions/caps + the SABR cube via `celnet-rates-vol`). Linear engine is validated first; the vol cube calibrates against it. |
| Q8 | **Exclude rateslib entirely?** | Research pass-1 found rateslib is **source-available NON-commercial (not OSS)** — a cargo-deny/licence trap; excellent rates coverage but unusable as a dep, arguably even as a commercial-pipeline oracle without a paid licence. | Exclude as dep + oracle; QuantLib+ORE suffice. | ✅ **2026-06-23: excluded** as dependency **and** oracle; QuantLib + ORE suffice. Revisit only with legal sign-off if a clear gap appears. |
| Q9 | **FinancePy as out-of-process oracle only?** | FinancePy is **GPL-3.0**: fine as a disposable separately-invoked CLI oracle (no linkage), forbidden as a dependency. | Allow as secondary CLI oracle only; never linked. | ✅ **2026-06-23: excluded as a dependency** (GPL); permitted **only** as a disposable out-of-process CLI oracle, never linked — and secondary to QuantLib/ORE. |
| Q10 | **Default curve interpolation for P0.** | Log-linear-DF (simple, local) vs Hagan-West monotone-convex forwards (smoother, no negative forwards). | Monotone-convex forwards default; log-linear-DF available. | ✅ **2026-06-23: log-linear-on-log-DF is the shipping default** (speed/robustness/hot-path); monotone-convex-forward selectable per curve. |
| Q11 | **STIR-futures convexity adjustment in P0?** | The futures↔FRA convexity adj. needs a short-rate vol model — pulls vol forward into the "linear" P0. | Defer (use a placeholder/zero adj. flagged) until the vol workstream. | ✅ **2026-06-23: deterministic placeholder now** (closed-form short-rate adj. so the short end is right); replaced by the term-structure-consistent adjustment in `celnet-rates-vol`. |
| Q12 | **Calendar & fixing-data sourcing** under the no-commercial-feed guardrail. | Holiday calendars + RFR fixings are needed inputs; must come from open/published sources or operator static. | Published central-bank/exchange calendars + operator-supplied fixings. | ✅ **2026-06-23: adopted** — hand-curated US_FED/TARGET2/GB_LON calendars from central-bank/exchange publications + operator-supplied NY-Fed/ECB/BoE fixings; all free/published. |
| Q13 | **CME published conversion factors as a third futures oracle?** | CME CFs give a rare independent anchor for bond-future CF/CTD validation. | Use CME CFs as a third anchor alongside QuantLib + the structural identity. | ✅ **2026-06-23: adopted** — ingest CME published CFs in the golden harness as a third independent anchor for bond-future CF/CTD. |
| Q14 | **Default swaption/cap quoting basis for v1** — normal (bp) primary, lognormal/shifted selectable? | Drives every vol object's representation and the whole negative-rate story; mixing bases silently misprices. | **Normal (bp) primary**, lognormal/shifted selectable per (ccy,index) as data. | ✅ **2026-06-23: adopted** — normal (bp) primary; lognormal/shifted-lognormal selectable per (ccy,index) as data. |
| Q15 | **SABR backbone `β`** — fix per-currency (β=0 normal-SABR where rates ≤0) or calibrate? | `β` is conventionally pinned not fitted; the wrong choice distorts skew + negative-rate handling. | **Fix per (ccy): β=0 (normal-SABR)** where rates can be ≤0, else market-standard β; configurable, not free-fitted. | ✅ **2026-06-23: adopted** — `β` fixed per (ccy) config (β=0 where rates can be ≤0), not free-fitted. |
| Q16 | **Arbitrage-free SABR wing** — No-Arb-SABR (Doust) / Hagan-2014 PDE / collocation, or asymptotic-for-v1 + no-arb fast-follow? | Hagan-asymptotic admits negative densities at low strike (mis-prices far-OTM CMS/wings). | **No-Arb-SABR (Doust)** wing (QuantLib has it as a direct oracle); asymptotic for the core. | ✅ **2026-06-23: adopted** — no-arb wing (QuantLib has it as a direct oracle) over the asymptotic core. |
| Q17 | **Short-rate factor count for v1 optionality** — 1F Gaussian first, 2F as the decorrelation upgrade? | 1F can't decorrelate curve points (mis-prices CMS-spread & some Bermudans); 2F costs more to calibrate. | **1F first**; **2F** as a P-next+1 upgrade gated by CMS-spread / curve-shape demand. | ✅ **2026-06-23: adopted** — 1F Gaussian first; 2F as a later decorrelation upgrade gated by CMS-spread/curve-shape demand. |
| Q18 | **CMS depth for v1** — full swaption-strip replication from day one, or cheaper closed-form convexity first? | Replication is accurate + book-consistent but costs a bucketed swaption strip per coupon. | **Replication off the cube** (reuses the cube we build); approximation only as a fast pre-trade estimate. | ✅ **2026-06-23: adopted** — replication off the cube; closed-form convexity only as a fast pre-trade estimate. |
| Q19 | **ISDA CDS Standard Model licence** — exclude the ISDA-PL source entirely and use QuantLib `IsdaCdsEngine` (BSD) as the ISDA-methodology oracle? | NEW load-bearing trap: the ISDA-PL is non-OSI/FSF (assent+indemnity clauses) and **fails the `cargo-deny` MIT/Apache/BSD allowlist**; "open source" ≠ permissive. | **Exclude ISDA-PL source; use QuantLib `IsdaCdsEngine` (BSD)**. Legal sign-off if ever reconsidered. | ✅ **2026-06-23: adopted** — ISDA-PL source excluded; QuantLib `IsdaCdsEngine` (BSD) is the ISDA-methodology oracle. |
| Q20 | **Credit workstream timing (resolves Q3)** — build `celnet-credit` (single-name + index CDS) in parallel now, or defer until rates-vol lands? | Credit is a disjoint crate (depends only on the discount curve); parallelisable but adds surface area. | **Defer** to a dedicated `W-FI` lane after `celnet-rates-vol`; scope locked in pass-2 so it starts cleanly. | ✅ **2026-06-23: adopted** — defer `celnet-credit` to a dedicated `W-FI` lane after the rates engine; scope locked, starts cleanly. |
| Q21 | **Inflation scope for v1 cash bonds** — linear-index ILB first (ignore floor), or ship the deflation-floor option now? | The deflation floor is embedded optionality reaching into the vol layer; ignoring it mis-prices near/below-par index ratios. | **Linear-index ILB first**; add the **deflation-floor option** alongside `celnet-rates-vol`. | ✅ **2026-06-23: adopted** — linear-index ILB first; deflation-floor option alongside `celnet-rates-vol` (which is in P0 per Q7, so it can fast-follow within P0). |
| Q22 | **OAS engine for callable/putable bonds** — reuse the 1F Gaussian (Q17) for OAS, or a dedicated bond-option model? | OAS needs a term-structure model; reusing the swaption short-rate engine keeps one calibrated model across the book. | **Reuse the 1F Gaussian** OAS engine; validate `OAS = Z` on option-free bonds. | ✅ **2026-06-23: adopted** — reuse the 1F Gaussian OAS engine; validate `OAS = Z` on option-free bonds. |

The agent appends new questions here as research surfaces them; the operator fills the
**Decision** column. Q8–Q13 were surfaced by research pass 1
(`docs/_research/fixed-income-findings.md`); **Q14–Q22 by research pass 2**
(`docs/_research/fixed-income-findings-pass2.md`).

### ⚠ Two conflicts the spec-synthesis surfaced — operator must pick the shipping default

- **Q10 (curve interpolation default).** Pass-1 findings §B recommends **log-linear-on-log-DF**
  as the shipping default (speed/robustness); this table's Q10 seed default says
  **monotone-convex-forward**. `FI-CURVES-SPEC.md` is written so the default is a one-line config
  flip either way — **decision needed**.
- **Q11 (STIR convexity adjustment in P0).** Pass-1 findings §B recommends a **deterministic
  placeholder now**; this table's Q11 seed default says **defer / zero-adjustment**.
  Flagged in `FI-CURVES-SPEC.md` §6.3 — **decision needed**.
