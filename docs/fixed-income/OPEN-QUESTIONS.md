# Fixed Income — open questions for the operator

Decisions only the business can make, surfaced by (and to be expanded by) the research agent.
Resolve these to lock phasing before any implementation. See
[`FIXED-INCOME-RESEARCH-BRIEF.md`](FIXED-INCOME-RESEARCH-BRIEF.md) §9.

## Locked build direction (operator, 2026-06-22)

These are settled — the research agent's `FI-ARCHITECTURE.md` must target them:

- **D1 — Own crate(s).** Fixed income gets its **own crate family**, separate from the FX
  crates: a dedicated pricing/analytics crate (e.g. `celnet-rates` for curves + linear rates,
  with a later `celnet-rates-vol` for swaptions/caps), depending only on the shared seams
  (`celnet-types`/`celnet-conventions`/`celnet-calendar`/`celnet-core`) — never folded into
  `celnet-vanilla`/`celnet-linear`. The agent proposes the exact crate split + dependency arrows.
- **D2 — GUI asset-class layer.** ✅ **RESOLVED / LANDED** (`fe-fi-migration`, `33d9a0a` / `fd18594`):
  instead of a top-level Options-vs-FI tab strip, the split was **collapsed into one class-parametric
  rail** — asset class chosen by scope + license, FI reached as *lenses* of the shared workspaces
  (Ticket/Market Data/Risk/Book). "FI integrated, not a peer." See [`FI-STATUS.md`](FI-STATUS.md)
  slice F and [`FI-ARCHITECTURE.md`](FI-ARCHITECTURE.md) §4.

## Locked P0 scope (operator, 2026-06-25 — REVISED; supersedes the 2026-06-23 full-breadth lock)

The 2026-06-23 pass chose **"both/all"** on every axis (3 currencies, cash+derivatives,
analytics+streaming, linear+vol). The operator has since **narrowed P0 to de-risk delivery**:
**USD-only**, **linear rates + cash bonds + cross-product strategy/RV analytics** — **no vol, no
credit, no multi-CSA**. Breadth (more currencies, rates vol, credit) returns as **sequenced
follow-on workstreams** once the USD linear core ships. `FI-ROADMAP.md` must sequence this
narrower core first, then the deferred lanes:

- **D3 — Currency (Q1):** **USD-SOFR only** in P0. Once USD ships, **duplicate the engine to other
  currencies — hard currencies first** — and add **cross-currency correlations** for XCCY / cross-ccy
  RV. The multi-currency generalisation is an explicit follow-on, **not** P0. (Supersedes the
  2026-06-23 USD+EUR+GBP lock — collapses the P0 calibration + reference-data surface to one curve
  family.)
- **D4 — Products (Q2):** USD **bonds, futures, and swaps**, **plus cross-product strategies**
  (basis trade, asset-swap/ASW) and **relative-value analytics** (yield, G-spread, Z-spread). The
  multi-curve USD foundation is the shared prerequisite, built first.
- **D5 — Delivery (Q5):** analytics (request/response PV/risk) **hardens first**; the streaming/RFS
  hot path is framed by the **best-execution positioning** — a central event processor consuming
  **external venue liquidity + internal liquidity (internalisation / market-making)** via
  **RFQ/RFM/RFS** with real-time analytics embedded for best execution (real-time venue scanning).
  P0 builds the analytics contract; the streaming/RFS + venue-scanning layer sequences behind it but
  is **designed-for from day one**.
- **D6 — Vol (Q7):** **DEFERRED.** P0 is **linear rates only**. Rates vol (`celnet-rates-vol`:
  swaptions/caps + SABR cube) is a **later workstream** — and the vol-dependent items (deflation
  floor, callable/putable OAS) move with it. (Supersedes the 2026-06-23 "both" lock.)
- **D7 — Curves (Q4/Q10/Q11):** single **OIS/SOFR-discount** curve; **log-linear-on-log-DF** default
  interpolation (monotone-convex selectable); **deterministic** STIR convexity placeholder (no vol
  dependency — keeps the short end right without pulling vol into P0).
- **D8 — Credit (Q3/Q19/Q20):** **DEFERRED — low appetite** (bilateral / uncleared). `celnet-credit`
  scope stays locked (ISDA-PL source excluded, QuantLib `IsdaCdsEngine` (BSD) oracle) so it can
  start cleanly later, but it is **out of the near-term plan**.
- **D9 — Reference data (Q6/Q12):** the **operator supplies test-environment access to data
  providers** — static/referential (incl. calendars + corporate actions), real-time (quotes, axes,
  prints), historical (quotes, prints, missed) — alongside open/published calendars + RFR fixings.
  No commercial-feed **runtime** dependency; these are operator-supplied inputs / test data.
- **D10 — Deferred-with-vol (Q21/Q22):** linear-index ILB analytics can ship in cash-bond P0, but
  the **deflation-floor option** and **callable/putable OAS** (both need the 1F Gaussian / vol layer)
  move to the deferred vol workstream.
- **D11 — Oracles (Q8/Q9/Q13):** QuantLib + ORE only (rateslib + FinancePy excluded as deps); CME
  published CFs as a third futures anchor.
- **D12 — Inbound-RFQ alerting (CROSS-ASSET — Options + FI).** Desktop ("growl") notifications +
  optional audible cue **when an inbound RFQ/RFM that requires a price arrives** (and on any
  auto-quote→manual escalation), fired for **both Options and Fixed Income** regardless of focused
  tab; click-through deep-links to the RFQ ticket; per-desk/per-counterparty-tier mute + threshold;
  honour OS Do-Not-Disturb; **degrade to the in-app toast + `aria-live`** when notification
  permission is denied. Asset-class-agnostic; full requirement in
  [`../GUI-EXPERIENCE-DESIGN.md`](../clients/GUI-EXPERIENCE-DESIGN.md) §3 row 14. (Recorded here because the
  FI RFQ surface is the active lane; not an FI-only decision.)

**Vol-detail decisions (Q14–Q18) are unchanged as locked _design_ choices, but now belong to the
deferred `celnet-rates-vol` workstream (per the Q7 revision), not P0.**

---

| # | Question | Why it matters | Options / default | Decision |
|---|---|---|---|---|
| Q1 | **P0 market & currency scope** — which RFR curves/currencies first? | Sizes the curve/calibration work and reference data. | USD-SOFR + EUR-€STR + GBP-SONIA, or USD-only to start. | ✅ **2026-06-25 (REVISED): USD-SOFR only to start.** Once USD is delivered, **duplicate the engine to other currencies — hard currencies first** — adding **cross-currency correlations**. Multi-currency is a follow-on, not P0. (Supersedes the 2026-06-23 USD+EUR+GBP lock; collapses P0 to one curve family.) |
| Q2 | **Cash vs derivatives first** — bond analytics or the swap/curve engine as the wedge? | Determines the P0 crate and the first client-visible value. | Curve+swap engine first (most reuse of carry/discount seams). | ✅ **2026-06-25: BOTH, on USD only** — **bonds, futures, and swaps**, **plus cross-product strategies** (basis trade, ASW) and **relative-value analytics** (yield, G-spread, Z-spread). Build order is dependency-led: the multi-curve USD foundation underpins all of it, built first. |
| Q3 | **Credit (CDS/ISDA model) — now or later?** | A distinct model + data dependency (survival curves, recovery). | Defer to a later workstream. | ✅ **2026-06-25: defer — low appetite** (bilateral / uncleared product universe). `celnet-credit` scope stays locked (Q19/Q20) so it starts cleanly later; out of the near-term plan. |
| Q4 | **Collateral / CSA discounting depth for v1.** | Single-curve OIS vs full multi-CSA is a large complexity step. | Single OIS-discount curve in v1; multi-CSA later. | ✅ **2026-06-25 (reaffirmed): single OIS/SOFR-discount curve in P0**; multi-CSA later. Operator agrees on the complexity step — multi-CSA means managing several curves / counterparties depending on CSAs. |
| Q5 | **Real-time vs analytics-first.** | Whether FI needs the streaming RFS hot path on day one. | Request/response pricing + risk first; streaming after. | ✅ **2026-06-25: positioning-dependent — analytics-first, RFS/best-execution by design.** If celnet executes on behalf of clients, a central event processor consumes **external venue liquidity + internal liquidity (internalisation / market-making)** via **RFQ/RFM/RFS** with embedded real-time analytics for best execution (real-time venue scanning). P0 hardens the request/response analytics contract first; the streaming/RFS + venue layer sequences behind it, designed-for from day one. |
| Q6 | **Reference-data sourcing** (bond static, calendars, fixings) under the no-commercial-feed guardrail. | Determines feasibility/openness of inputs. | Open/published sources + operator-supplied static. | ✅ **2026-06-25: operator supplies data-provider test-environment access** — static (referential, calendar, corporate action), real-time (quotes, axes, prints), historical (quotes, prints, missed) — plus open/published calendars + RFR fixings. No commercial-feed **runtime** dep; operator-supplied inputs / test data. |
| Q7 | **Vol / optionality in scope for v1?** | Swaptions/caps need a vol cube + SABR; big step beyond linear. | Linear rates first; vol as the next workstream. | ✅ **2026-06-25 (REVISED): defer vol.** Linear rates only in P0; rates vol (swaptions/caps + SABR cube via `celnet-rates-vol`) is a later workstream — vol-dependent items (deflation floor, callable OAS) move with it. (Supersedes the 2026-06-23 "both" lock.) |
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

### Which rows are in the (narrowed) P0 vs deferred

After the **2026-06-25 revision** (see the locked-scope block above):

- **In P0 (USD linear core):** Q1 (USD-only), Q2 (USD bonds/futures/swaps + strategy/RV), Q4
  (single curve), Q5 (analytics-first), Q6 (operator data), Q10/Q11 (curve interpolation + STIR
  placeholder), Q12/Q13 (calendars/fixings + CME CFs oracle), Q8/Q9 (oracle/licence exclusions).
- **Deferred to the rates-vol workstream:** Q7 and its detail rows **Q14–Q18**, plus the
  vol-dependent halves of **Q21** (deflation-floor option) and **Q22** (callable/putable OAS).
  Linear-index ILB analytics (the non-floor half of Q21) can still ship in cash-bond P0.
- **Deferred to the credit workstream (low appetite):** Q3, Q19, Q20.

> **Resolved earlier conflicts (Q10/Q11).** The 2026-06-23 pass settled both: Q10 ships
> **log-linear-on-log-DF** (monotone-convex selectable); Q11 ships a **deterministic STIR convexity
> placeholder**. Both remain valid under the narrowed P0 — neither pulls vol forward.
