# Celnet — Surface Workflow (SOTA UX research + critique + redesign)

> **Status:** research + design proposal. This lane writes ONLY this file. It does **not**
> edit `docs/ROADMAP.md`; the proposed backlog lives in §10 below for the orchestrator to
> merge.
> **Horizon:** May-2026 SOTA. **Audience:** FX-options-desk traders + the GUI build wave.
> **Scope:** the *Surface experience* — the vol-marking workspace at desk/book scale. It does
> NOT re-open the wire contract math (that lives in `celnet-surface`); it specifies the
> *interaction model* the GUI must implement against the existing `MarkSurface` / smile / scenario
> seam and the `MarkedSurface` / `Smile` / `BrokerQuoteSet` / `ArbReport` shapes in
> `gui/src/data/contract.ts`. Where contract *shape* must evolve, it is flagged as a
> single-current-contract evolution (GUIDE.md rule 9: no versioning, evolve in place).
> **Honesty discipline:** every external market/quant claim carries a SOURCE tag and a
> CONFIDENCE (high/medium/low). Illustrative numbers and UX inventions are labelled. Competitor
> *capabilities* are cited where verifiable; competitor *screen layouts* are explicitly held at
> low confidence and never asserted as fact.

---

## 1. Purpose & scope

The product owner judged the current Surface workspace "illogical and not intuitive." This doc
(1) establishes the **canonical FX vol-surface marking workflow** a desk/book trader actually
runs, grounded in cited sources; (2) **critiques** the current Celnet Surface against it and
against incumbents; and (3) specifies the **optimal redesign**, mapped onto Celnet's existing
`celnet-surface` capabilities (Vanna-Volga, SABR, SVI, SSVI, **eSSVI** — five smile models,
arbitrage-free term structure, broker→smile-strangle calibration) and the `surface_version`
marking seam.

The single load-bearing fact behind everything below: **FX volatility is marked in delta space,
not strike space — per tenor, as a small set of broker handles (ATM + 25Δ/10Δ risk-reversal and
butterfly), from which the continuous smile is *derived by calibration*.** [Reiswich-Wystup;
quantpie; FinPricing; project CONVENTIONS.md §1.4; high] The current workspace inverts this:
it makes a rotating 3-D mesh the hero and the marking handles a read-mostly afterthought. That
inversion is the root cause of the "illogical" verdict (developed in §8).

---

## 2. The trader's mental model of a surface

A desk/book trader does not think of a surface as a height-field over (tenor, strike). They
think in **handles and pillars**:

1. **Delta-space, sticky-delta.** The smile is anchored to *deltas*, not strikes. As spot moves,
   the smile *in delta space* is (to first order) invariant; the *strike→vol* map re-rolls
   underneath it. FX surfaces are built and marked on the **sticky-delta (sticky-moneyness)**
   rule. [Clark, *Foreign Exchange Option Pricing*; FX-convention literature; CFRM17; project
   CONVENTIONS.md §3; high] This is *the* reason the editable workspace must be a
   **delta × tenor grid**, not a strike grid and not a 3-D mesh: the grid in delta pillars **is**
   the sticky-delta representation the trader reasons in.

2. **Three handles per tenor.** Each tenor slice is summarised by:
   - **ATM** — the level (delta-neutral straddle vol, or ATM-forward, per convention). [high]
   - **Risk reversal (RR)** — `σ(25Δ call) − σ(25Δ put)`: the **skew** handle. [quantpie;
     CONVENTIONS.md §1.4; high]
   - **Butterfly (BF) / strangle** — the **convexity** handle. [high]
   Liquid pairs carry both **25Δ and 10Δ** RR/BF (a 5-handle smile: ATM, 25Δ C/P, 10Δ C/P).
   [Reiswich-Wystup; CONVENTIONS.md §1.4; high]

3. **Broker strangle ≠ smile strangle.** What brokers *trade and quote* is the **market (broker)
   strangle**: a *single* vol `σ_ATM + BF` applied to *both* wing strikes, which must reprice to
   the same value as the calibrated smile. The **smile strangle** is the convexity the calibrated
   smile actually carries. They differ — sometimes materially for high-RR / EM pairs — and
   recovering the smile is a **fixed-point root-find**, never an arithmetic `σ_ATM ± ½RR + BF`.
   [Reiswich-Wystup 2010; Castagna-Mercurio 2007; Bossens et al. arXiv:0904.1074 §3.3; project
   CONVENTIONS.md (mandatory, "the #1 production bug"); high] Celnet already implements this
   correctly server-side in `celnet-surface/src/strangle.rs`; the *UI* must surface which number
   it shows (quoted broker BF vs. calibrated smile strangle) and never silently conflate them.

4. **Premium-adjusted delta is non-monotone.** For premium-adjusted pairs (most quote-ccy-premium
   / EM pairs), the call delta is **non-monotone in strike** (a max-delta strike, two solutions);
   the delta→strike map needs a guarded, branch-aware root-find. [CONVENTIONS.md §1.3/§1.4;
   Clark; high] This interacts with the strangle root-find: the handle the trader types and the
   strike the engine solves do not move 1:1, so the workspace must show the **conversion
   residual** for premium-adjusted pairs, not pretend the handle is the strike.

5. **Term structure, in total variance.** Across tenors the trader sees ATM/RR/BF *curves*. The
   correct interpolation variable is **total variance** (variance is additive in time; vol is
   not), in **business time** (weekends slow, weekdays normal, scheduled events fast).
   Interpolating raw vol — or interpolating RR/BF independently per tenor without a no-arb
   constraint — produces **crossed total-variance curves = calendar arbitrage**.
   [Gatheral-Jacquier arXiv:1204.0646; MathFinance "Calendar arbitrage in the FX vol surface";
   Healy, "Counterexamples for FX Options Interpolations" Parts I & II, arXiv:2512.19621/19625;
   CONVENTIONS.md §3 / ANALYTICS-SPEC §3.6; high]

6. **My-marks vs. official.** The number on a trader's screen has a *role*: a **working mark**
   (their intraday view, used to price/risk their own book), an **official desk mark** (the
   published surface the desk trades and books off), and an independent **IPV / valuation-control
   mark** verified by a unit *independent of the dealing room* at least monthly (or more
   frequently), which feeds the general ledger and is price-tested against a **consensus golden
   copy** (e.g. Totem-style month-end consensus, ~30 submitters per contract). [BIS bcbs153
   (Basel supervisory guidance on valuation); S&P Global / consensus-pricing studies; high for
   the ≥-monthly independent-verification floor; daily-mark-to-market/close cadence is
   standard-but-firm-specific, medium] **The same surface object plays different roles to
   different people at different cadences** — the workspace must make the role and provenance
   first-class, not implicit.

The trader's verbs, then, are: *nudge ATM/RR/BF on a tenor; pull the whole curve; check no-arb;
compare to broker/history/consensus; promote my working surface to official; publish a version;
price a broken date off it.* The workspace must be built around those verbs.

---

## 3. The optimal marking workflow (desk/book traders)

The end-to-end loop, in the order a trader runs it:

1. **Seed from broker / composite.** Pull the latest broker quotes (ATM + 25Δ/10Δ RR/BF per
   tenor) or an aggregated multi-source composite mid. The marking grid populates with the
   **quoted broker handles**. *(The composite/consensus feed is a dependency on
   `docs/CELNET-INTEGRATION.md`, not assumed to already exist — see §8/§9.)*

2. **Calibrate.** For each (pair, tenor), run the **market-strangle → smile-strangle**
   fixed-point so the calibrated smile reprices the broker strangle exactly, then build the
   continuous slice (VV / SABR / SVI / SSVI / eSSVI per the chosen model). This is
   `celnet-surface/src/strangle.rs` + the smile models; the GUI consumes the result via the
   `MarkSurface` seam. The grid must show **both** the quoted broker BF and the calibrated smile
   strangle when they diverge. [calibration mandatory — CONVENTIONS.md; high]

3. **No-arb check (per-slice *and* cross-tenor).** Surface **butterfly** (non-negative
   risk-neutral density per slice) *and* **calendar** (total variance non-decreasing in maturity)
   violations, per node and per tenor-pair. Celnet's `celnet-surface` already computes **both**:
   `SurfaceArbitrageReport` carries per-slice butterfly/vertical diagnostics **and** a cross-slice
   `min_calendar_increment` (total-variance monotonicity). The publish guard must gate on the
   **whole** report, not butterfly-only. [Gatheral-Jacquier (static arb ⟺ butterfly-free per
   slice AND total variance ↑ in maturity); verified in `celnet-surface/src/surface.rs`,
   `parametric_surface.rs::is_calendar_free`; high]

4. **Compare.** Overlay the working surface against: (a) the **live broker composite / LP mid**
   (intraday); (b) **history** (yesterday's official, T-5, etc.); (c) **month-end consensus**
   (Totem-style, for price-testing/MTM control — *monthly* cadence, not a live band). Differences
   render as a Δ-overlay on the smile chart and a coloured Δ-column on the grid. *(Which
   reference and which cadence must be explicit so the feature is not specified as if a feed
   exists that does not — §8/§9.)* [overlay/compare is standard desk practice; medium —
   design-opinion, no single canonical source]

5. **Working vs. official.** The trader marks privately (working surface) and risks their own
   book against it; when satisfied they **promote** it to the official desk surface. The two are
   distinct *roles* of the same `MarkedSurface`, distinguished by a **surface-role** field (see
   §7, contract evolution). [auto-base + manual-override + promote pattern is standard terminal
   behaviour; the specific role taxonomy is a Celnet design proposal grounded in the cited
   trader/IPV split, BIS bcbs153; medium]

6. **Publish / version.** Promote deposits an **immutable** `MarkedSurface` under a fresh,
   monotonic `surface_version`. All downstream pricing/RFQ/RFS pin to a version (the server's
   `surface_book` resolver rejects unknown versions — never a silent live fallback; per the
   implementation ledger). Two versions can be **diffed** for clean attribution. *(Positioning
   note: the per-mark calibration provenance and version-diff is Celnet's intended
   differentiator vs. LP-aggregating front-ends — framed as positioning, **not** an
   established competitor gap, since no competitor screen was inspected.)* [versioned registry
   is real per ledger; differentiator claim = positioning, low]

7. **Downstream pricing/risk.** The ticket and the risk/scenario grids price off the pinned
   official surface (smile read by leg delta, `impliedVolForInstrument`), so a re-mark is a
   *version bump* with attributable P&L, not an in-place mutation.

8. **Manual overrides.** On top of the auto-calibrated base, the trader can **nudge** a single
   handle (e.g. 1M ATM +0.2v, 3M 25RR −0.1v) or **macro-edit** a whole curve (parallel shift,
   twist, steepen/flatten the ATM curve; widen/compress the RR or BF curve). Each override is a
   tracked layer over the base so it can be shown, reverted, and attributed. *(The specific nudge
   verbs/tick-sizes are a Celnet UX proposal; the auto-base+override **pattern** is standard,
   medium. Macro curve edits must be backed by an engine call — see §10 backlog dependency.)*

9. **Intraday re-marks & event-driven marks.** Re-mark on material spot/vol moves, on the run-up
   to a **scheduled event** (FOMC/ECB/BoE/NFP/CPI/fixing), and at the **close snapshot**. Event
   marks add/relax a discrete **event-variance bump** in the short-tenor ATM term structure
   (§5). The blotter must flag stale pairs and surface a "re-mark due" cue.

**Scale note (ties §6):** the mandatory market→smile fixed-point runs **per (pair, tenor)**. A
whole-book intraday re-mark is therefore a real compute/latency cost, not free — the desk/book
blotter (§6) must account for it (batch calibrate, stream deltas, flag staleness) rather than
assume instant whole-cube recalibration. [derived from CONVENTIONS.md calibration requirement +
TRADING-UNIVERSE-SCALE.md sizing; high on the cost existing, medium on magnitude]

---

## 4. Smile / term construction & editing spec

### 4.1 The canonical editable grid is the primary view

**The marking grid — rows = tenors, columns = handles (ATM, 25RR, 25BF, 10RR, 10BF) — is the
primary, editable surface.** Every cell that is a *broker handle* is type-editable with
keyboard navigation; the trader marks by typing into the grid, exactly as they reason. This is
the sticky-delta representation (§2.1) and the natural home for the marking verbs (§3).

- **Handle-based, not a wide display grid.** The *marking* inputs are the ~5 broker handles per
  tenor. Vendors *display* a wider derived delta grid (e.g. Bloomberg BVOL/OVDV expose
  5Δ/10Δ/15Δ/25Δ/35Δ + ATM ≈ 11 columns) for inspection. [Bloomberg Real-Time Volatilities PDF;
  Mathema OVML docs; high] Celnet should mark on handles and *expand* to a wider read-only delta
  grid for inspection — the 5 handles are not a display ceiling, only the marking input set.
- **Both BF numbers when they diverge.** Show the quoted **broker BF** and, when it differs, the
  calibrated **smile strangle** (§2.3) — never one masquerading as the other.
- **Nudge increments tied to vol conventions** (proposal): ATM ~0.05 vol, RR/BF ~0.025 vol per
  arrow-key step; `⌘↩` to commit, `Esc` to revert. *(UX invention, labelled; tick sizes chosen to
  match typical desk vol-point granularity, medium.)*

### 4.2 Smile chart = secondary inspector

A per-tenor 2-D **smile chart** (vol vs. delta axis 10P·25P·ATM·25C·10C) sits beside the grid as
the *shape inspector* for the selected tenor: it shows the calibrated curve, the broker handle
markers, the no-arb status, and any compare overlay. It is **secondary** to the grid — you read
shape here, you mark in the grid. The chart's y-axis must use a **stable, sensible vol scale**
(not auto-fit to the slice's own min/max, which exaggerates tiny moves — a current bug, §8).

### 4.3 The 3-D mesh: honest take

The rotating 3-D mesh is a **shape-communication / QC view, not a marking tool.** You cannot
type into an occluded, rotating height-field; depth (vol) precision is poor; and a mesh hides
exactly the per-handle numbers a trader marks on. [usability argument — occlusion, no type
target, low z-precision; this is reasoning, not a vendor claim; medium] Its legitimate uses:
spotting a gross dislocation across the whole cube at a glance, communicating shape to a
non-quant, and screenshotting for a morning note. **Recommendation:** demote it from hero to an
optional, collapsible QC panel; the **term-structure curves** (ATM/RR/BF vs. tenor) are far more
useful as the second-most-prominent view and are currently **absent** (§8).

### 4.4 Live recalibration & no-arb flagging

Editing any handle **live-recalibrates** that tenor (re-run strangle fixed-point + smile build)
and re-checks no-arb **per slice and across neighbouring tenors** (calendar). Violations surface
**at the offending node** (which tenor-pair crosses; which wing breaks convexity), not as a
single global boolean. Where an automatic fix is offered ("minimum perturbation to restore
no-arb"), it is a **projection onto the no-arb set** — a real optimisation, scoped as an
engine/research item (§10), **not** a free UI affordance, to avoid a placeholder promise. [the
projection is non-trivial; flagged so it isn't faked; high that it's non-trivial]

### 4.5 Overlay / compare & sticky-delta

Overlay any reference (broker composite, history, consensus) on both grid (Δ-column) and chart
(ghost curve). Provide a **sticky-delta vs. sticky-strike toggle** on the spot-move simulation:
sticky-delta (the FX default) holds the smile in delta space as spot moves; sticky-strike holds
strike-space vols. Showing the difference is a strong teaching/QC affordance and makes Celnet's
delta-space thesis explicit. [sticky-delta is the FX default — Clark/CFRM17; high]

---

## 5. Broken-date & event pricing off the surface

**Why it matters at scale:** a large share of FX forward/option flow trades on **broken dates**
(off the standard tenor ladder). On a major venue, **~51% of FX forward trades are broken-dated
and ~20% of volume trades during "turn" periods** (EOM/quarter/year/IMM). [LSEG turn-impact
insight; high that the figures exist; single-venue, so not "all FX globally" — medium scope]

### 5.1 Dual-mode expiry control (tenor ⇄ date)

The workspace must let the trader specify expiry either as a **tenor string** ("3M", "1Y") or an
**explicit value/expiry date**, bidirectionally linked through the FX date machinery (spot lag
T+2 default; T+1 for USDCAD and some EM; EOM and IMM rules; both-currency + USD calendar
intersection). [OVDV/MX.3 expose both tenor and expiry as capabilities — Bloomberg/QuantNet,
Murex spotlights, high; the *bidirectional-live-round-trip UI* is an inferred design pattern,
medium.] **IMM dates** (third Wednesday of Mar/Jun/Sep/Dec) are a turn concentration point and are
**now resolved in `celnet-calendar`** — `Tenor::Imm(n)` selects the `n`-th IMM expiry strictly after
the horizon (`crates/celnet-calendar/src/fx.rs`, "Exchange-defined IMM date"; `Tenor::Imm` in
`celnet-types`), closing the earlier gap; what the *GUI* still needs is the dual-mode tenor⇄date
control that round-trips through it (§7/§10). [IMM = 3rd Wed — CME/Wikipedia, high; resolver verified
in code, high]

### 5.2 Two clocks: vol time vs. discount time

Vol/time-to-expiry uses **ACT/365** ("vol time"); settlement discounting uses each currency's
money-market basis (ACT/360 USD/EUR, ACT/365 GBP/AUD) ("discount time"). The GUI's single
`expiryYears` scalar is insufficient — a broken-date pricer needs **both** clocks.
[Clark; ANALYTICS-SPEC §1.3; high] Time interpolation is in **total variance on a business
clock**, not linear-in-vol on calendar time (§2.5).

### 5.3 Event-aware business clock

Scheduled events (central-bank meetings, fixings, NFP/CPI) deposit a discrete step in cumulative
variance, producing a **kink/local spike in the short-tenor ATM term structure** that relaxes as
it amortises. Model: total variance = diffusive · τ(t) + Σ (event variances for events in life),
with a business clock weighting weekends down (small positive) and event days up (>1).
[method is standard — Moontower vol-time; Clark "temporal interpolation — holidays/weekends";
mechanically implied by variance additivity; high for the method.] **Cross-asset** evidence that
event days carry a large, positive variance risk premium (esp. FOMC) comes from Wright NBER
w28306 — **note this is Treasury/equity-index futures, not FX**; it motivates, but does not
measure, FX event vol. [Wright w28306; high for existence, flagged cross-asset.] FX-specific
event-vol *magnitudes* and the FX wing reaction are **firm-proprietary / thinly sourced** — any
specific numbers (e.g. "FOMC ≈ 0.55% on EURUSD," "weekend weight ~0.1–0.3") are **illustrative
placeholders pending calibration, never Celnet defaults.** [low]

**Engine seam & open decisions (must be designed, not faked — GUIDE.md gates #2/#5):**
`celnet-surface` already has the `BusinessClock` trait with `with_clock(...)`, but only the
identity `CalendarClock` is implemented — there is **no `EventClock`** (verified in
`termstructure.rs`). Building the event path requires:
- a concrete **`EventClock`** (proposed name) + an **event-variance stripping/calibration** that
  reproduces the marked ATM pillars while keeping the base curve smooth, **validated against a
  reference** (QuantLib / published prices) per gate #5 — scoped as "designed, then validated,"
  not asserted;
- a **decision on where event variance lands**: ATM-only (proposed v1 default) vs. full-smile
  (RR/BF carry event bumps) — FX wing-reaction literature is thin, so **ATM-only at v1** is the
  stated modelling choice;
- a **cut-aware in-life rule** with precise timezone handling (FOMC 14:00 ET vs. NY 10:00 cut vs.
  Tokyo 15:00 cut), plus edge-case tests (event on the expiry day; event at the cut minute);
- an **open / free event-calendar source** (central-bank published schedules, BLS release
  calendar) or a manual/ingested-event seam — a commercial event-calendar data product would
  violate guardrail #7. [high — guardrail-driven]

### 5.4 Link to the ticket

A broken-date pricer reads the official pinned surface, interpolates ATM/RR/BF separately in
total variance on the event clock, reconstructs the smile at the target date, and prices the
structure — and offers **mark-to-impact**: change a handle, see the broken-date price/Greeks
move, with the vega bucketed onto the two bracketing pillars by business-time weights
`w = w_lo·(1−α) + w_hi·α`. [bucketing follows directly from the interpolation weights in
`termstructure.rs`; high for the math; the attribution waterfall UI is a design proposal.]

---

## 6. Scale: marking/monitoring many pairs (desk view)

A desk runs **tens of pairs** actively (a G10+EM desk universe spans tens-to-low-hundreds; do
**not** assert a false-precise count). Vendor *coverage* commonly spans ~30 base currencies
updated intraday (→ dozens of crosses); large incumbents advertise full intraday surfaces for
hundreds of pairs (e.g. LSEG/FENICS "340+ pairs," Bloomberg BVOL "200+ pairs") as **coverage**,
which is larger than any single trader's active book. [LSEG; Bloomberg PDF; high for the
coverage figures; "active book = tens" is the honest framing, medium.] *(Earlier framing of a
"340+ pairs traded" feed figure is avoided: the only sourced "340+" is LSEG **surface
coverage**, not trade count.)*

The **Desk Surface Monitor** (proposed) is a multi-pair blotter:
- one **row per pair**, columns = ATM term-structure sparkline, 25RR / 25BF level, no-arb status,
  staleness, working-vs-official divergence, vs-broker / vs-consensus deltas;
- click a row → open that pair's full marking workspace (§4);
- **staleness & re-mark cues**: highlight pairs whose marks are old or whose live composite has
  drifted beyond a threshold; flag pairs with a scheduled event imminent.

This ties to `docs/TRADING-UNIVERSE-SCALE.md` (the vol-cube sizing and streaming fan-out): the
monitor is a **consumer** of the streamed cube. The per-(pair,tenor) **smile-strangle
fixed-point cost** (§2.3/§3) means a whole-desk re-mark is batched/streamed and staleness-aware,
fitting Celnet's zero-alloc/streaming budgets — a real throughput/staleness design cost, owned
jointly with TRADING-UNIVERSE-SCALE.md (which owns cube sizing; this doc owns the *interaction*).
[scale boundary per that doc; high.] **Spreading** for client pricing is **counterparty-tiered**
(spread by counterparty profile/tier), not a flat overlay. [Fenics spreads "based upon
counterparty profile"; medium.]

---

## 7. The Celnet redesign — what SurfaceWorkspace becomes

Concretely, `SurfaceWorkspace.tsx` is reorganised around the marking grid:

1. **Primary: the editable delta × tenor marking grid.** Rows = tenors; columns = ATM, 25RR,
   25BF, 10RR, 10BF. *Every broker-handle cell is editable* (today only ATM-of-the-active-row is),
   keyboard-navigable, with vol-convention nudge steps. Backed by `celnet-surface` calibration via
   `MarkSurface`. Shows quoted-BF vs. smile-strangle when they diverge.
2. **RR / BF / ATM handle model + live recalibrate.** Editing any handle re-runs the
   market→smile-strangle fixed-point (`strangle.rs`) and rebuilds the slice (VV/SABR/SVI/SSVI/eSSVI),
   with a visible **model selector** + per-mark **model provenance** (which model calibrated this
   mark), since Celnet ships **five** smile models (the fifth, eSSVI, is the wire
   `SMILE_MODEL_EXTENDED_SURFACE` selector — maturity-dependent ρ, closed-form arbitrage-free).
   [model-choice surfacing is a real desk need;
   medium]
3. **No-arb surfacing (per-slice + cross-tenor).** Replace the hardcoded
   `calendarArbitrageFree: true` with the **real surface-wide check** Celnet already computes
   (`SurfaceArbitrageReport`: per-slice butterfly + cross-slice calendar `min_calendar_increment`).
   Violations render at the offending node; **Publish gates on the whole report.**
4. **Working vs. official + publish/version.** Distinguish the trader's **working** surface from
   the **official** one; **promote** publishes an immutable `MarkedSurface` under a fresh
   `surface_version`. Re-mark and Publish must do **different** things (today both call the same
   `remarkSurface()` and **discard the trader's edit** — the headline bug, §8). The edit must
   reach the engine.
5. **Compare / history overlay.** Δ-overlay vs. live broker composite (intraday), history, and
   month-end consensus (monthly cadence, clearly labelled). Depends on the composite/consensus
   feed (`CELNET-INTEGRATION.md`).
6. **Per-tenor smile inspector.** The 2-D smile chart as secondary shape inspector (stable
   y-scale; handle markers; overlay; no-arb), with **term-structure curves** (ATM/RR/BF vs. tenor)
   promoted to the second-most-prominent view; the 3-D mesh demoted to an optional QC panel.
7. **Broken-date, event-aware pricer.** Dual-mode tenor⇄date control, two clocks (ACT/365 vol /
   MM-basis discount), total-variance interpolation on an **`EventClock`** (event bumps ATM-only
   at v1), wired to the ticket with mark-to-impact. Requires the new `EventClock` + IMM resolver
   (§10).
8. **Multi-pair Desk Surface Monitor.** The §6 blotter as the desk-scale entry point.

### Contract evolutions (single current contract, rule 9 — evolve in place, no versioning)

The wire/`gui` contract `MarkedSurface` / `Smile` / `BrokerQuoteSet` / `ArbReport` need, as
**in-place** evolutions (not versioned additions):
- **`ArbReport`**: carry **per-tenor-pair calendar status** (not a single boolean) so the GUI can
  flag the offending tenor-pair — backed by `SurfaceArbitrageReport.min_calendar_increment`.
- **`MarkedSurface` / `Smile`**: a **surface-role** field (working / official / valuation-control)
  and the **market context** the surface was marked against (spot, fwd points, depo curves) so a
  version pins its market and is reproducible/diffable. [spot/fwd/depo coupling — CFRM17;
  CONVENTIONS.md; high]
- **`BrokerQuoteSet`**: carry both the **quoted broker BF** and the **calibrated smile strangle**,
  plus the **delta/ATM/strangle conventions** used (today the provenance line shows only
  "5-pt/3-pt broker"), and an **override layer** (base + tracked nudges).
- **Event model**: concrete fields for **scheduled events** (datetime, weight/variance) and the
  **interpolation weights** under the single contract.

---

## 8. Critique — current Celnet Surface vs. SOTA & incumbents

### 8.1 What's illogical today, and why (verified against the source files)

| # | Finding (current code) | Why it's wrong | Evidence |
|---|---|---|---|
| 1 | **3-D mesh is the hero; marking grid is a side panel** (`SurfaceWorkspace.tsx:73–89` mesh in primary panel; grid in `markPanel`). | Traders mark in delta×tenor handles (sticky-delta); a rotating mesh has no type target and hides the numbers. Inverts the mental model. | code; Clark/CFRM17 sticky-delta [high] |
| 2 | **Only ATM of the active row is editable**; RR/BF are static `<span>` (`:113–128`, `editAtm` state `:41`). | Skew (RR) and convexity (BF) are *the* marking handles; making them read-only defeats marking. | code [high] |
| 3 | **Publish/Re-mark silently discard the trader's edit.** Both buttons call `app.remarkSurface()` (`:146`,`:149`) which re-sends the seed `brokerLadder`; `editAtm` lives only in a local `useMemo` (`:58`) and is **never sent**. | A trader can "publish" and ship the *un-edited* surface — a real **mismark hazard** (the headline defect). | code: `AppContext.remarkSurface` always passes `brokerLadder(pairCtx)`; `editAtm` never transported [high] |
| 4 | **`calendarArbitrageFree: true` is hardcoded** (`surface.ts:92`); `checkArb` is a 5-pillar butterfly-only second difference; Publish gates on butterfly only (`:149`). | You can publish a **calendar-arbitraging** surface. Static arb ⟺ butterfly-free per slice AND total variance ↑ in maturity. | code; Gatheral-Jacquier 1204.0646 [high]. *Backable*: `celnet-surface` already computes the cross-slice check (`SurfaceArbitrageReport`). |
| 5 | **Single pair only; no desk/book monitor.** | A desk runs tens of pairs; no multi-pair surface view. | code [high] |
| 6 | **No compare / overlay / history.** `SmileChart` plots one smile; no reference curve. | Traders mark *relative* to broker/history/consensus. | code [high] |
| 7 | **No term-structure curves.** Only mesh + single smile. | ATM/RR/BF-vs-tenor curves are core to marking; sticky-delta never named/simulated. | code [high] |
| 8 | **No broken-date / value-date pricer; linear-in-vol calendar-time interpolation** (`surface.ts:184–201`: `vLo + (vHi−vLo)·w`, `w` linear in calendar `tenorYears`). | Wrong on two counts: interpolate **variance** not vol, in **business** not calendar time → can manufacture calendar arb. ~51% of forward flow is broken-dated. | code; Gatheral-Jacquier; LSEG [high] |
| 9 | **No concrete `EventClock`.** `BusinessClock` trait + `with_clock` exist server-side; only the identity `CalendarClock` is implemented. *(The IMM resolver gap is now closed — `Tenor::Imm` resolves third-Wednesday IMM dates in `celnet-calendar`; the remaining gap is the `EventClock` itself.)* | Event/turn structure (the most-traded broken dates) cannot yet be priced through an event-weighted clock. | `termstructure.rs` [high]; IMM resolver verified built in `celnet-calendar/src/fx.rs` |
| 10 | **Provenance thin**: shows only "5-pt/3-pt broker" + clock + version (`:136–143`); no delta/ATM/strangle convention; no working-vs-official role; no model. | Marks are role- and convention-laden; hiding it invites silent mismarks. | code [high] |
| 11 | **Fragile float-equality selection** at four sites, two tolerances (`SurfaceWorkspace.tsx:58,102,160`; `SmileChart.tsx:96` — `<1e-9` and `<1e-6`); two cursors; no keyboard nav. Smile chart y-axis **auto-fits** slice min/max (`SmileChart.tsx:50–58`), exaggerating tiny moves. Mesh uses a **9-pt** `DELTA_AXIS` vs. the **5-pt** `DELTA_PILLARS` (`SurfaceMesh.tsx:28` vs `surface.ts:27`). | Brittle interaction + misleading scaling + axis mismatch. | code [high] |

**The single biggest reason it's unintuitive:** the workspace is built around the **wrong primary
object** — a rotating 3-D mesh — when traders mark in an **editable delta × tenor handle grid**.
Everything else (read-only RR/BF, discarded edits, no compare, no term curves) follows from that
inversion.

**The single highest-impact redesign move:** make the **editable delta × tenor marking grid the
primary surface**, with **all** of ATM/RR/BF editable and **edits actually transported to
`celnet-surface` and published under a new `surface_version`** (fixing the §8.1#3 mismark hazard
in the same stroke).

### 8.2 Celnet vs. incumbents — capabilities (verified facts, cited)

> *Capabilities* are cited; *exact screen layouts* are **not** verified and are held low. No
> competitor calibration-canvas screenshot was inspected.

| Capability | Celnet (today → proposed) | Incumbents (verified capability) | Conf. |
|---|---|---|---|
| Mark by ATM + 25Δ/10Δ RR/BF per tenor | partial (ATM-only editable) → **full editable grid** | Standard across the market; Bloomberg OVDV takes ATM/RR/BF → extended delta grid | high (Bloomberg PDF; Mathema) |
| Delta×tenor handle grid as marking primary | **proposed** (today mesh-first) | Bloomberg BVOL/OVDV present delta×tenor grids | high (Bloomberg) for grid existence; layout low |
| Market-strangle → smile-strangle calibration | **DONE** (`strangle.rs`) | Required correctness; the part most front-ends get wrong | high (Reiswich-Wystup; Bossens; Castagna-Mercurio) |
| Arb-free term structure (SVI/SSVI; calendar) | **DONE** server-side; GUI not yet wired | SSVI = explicit no-arb conditions both axes | high (Gatheral-Jacquier 1204.0646; eSSVI: Hendriks-Martini 2019; Mingone 2204.00312) |
| Surface-wide calendar-arb gate in the UI | **proposed** (engine ready) | — | high (engine verified) |
| Compare / overlay vs. broker / history / consensus | **proposed** | LP-aggregation / overlay common (e.g. IBKR Vol Lab, vendor composites); month-end consensus = Totem (S&P Global, ~30 submitters, monthly) | medium overlay; high Totem cadence |
| Broken-date + event-aware pricing | **proposed** (EventClock + IMM gap) | OVDV/MX.3 expose tenor + explicit expiry; MX.3 real-time book mgmt | high capability; UI round-trip inferred |
| Multi-pair desk surface monitor | **proposed** | JPM multi-currency vol grids; LSEG FENICS 340+ pairs coverage; BVOL 200+ | high coverage figures |
| Smile-model choice (VV/SABR/SVI/SSVI/**eSSVI**) + provenance | **engine + wire ready** (five models; eSSVI = `SMILE_MODEL_EXTENDED_SURFACE`); UI provenance thin → **model selector + per-mark provenance** | Murex ships SLV (GPU-accel) as a marking-model choice; advanced vol marking | high (SLV); IPV tight-coupling = **inference, medium** |
| Immutable versioned marks + version-diff attribution | **DONE** (`surface_version` / `surface_book`) → expose diff | Vendors version marks; "diff your own mark vs. your own position" = Celnet **positioning**, not a verified competitor gap | medium engine; positioning low |
| Per-pair surface viz analog | mesh + smile | LSEG **FXVE** (FX Volatility Explorer) is the closest single-pair viz analog | medium |

**Honest separation.** *Verified facts (cited):* the ATM/RR/BF marking model; broker-vs-smile
strangle and its fixed-point; SSVI/eSSVI no-arb; sticky-delta; total-variance interpolation;
calendar-arb = total-variance monotonicity; Bloomberg OVDV/BVOL & SD/ICE 3-input (ATMF + 25Δ
collar + 25Δ strangle) vol-surface inputs; Murex SLV; LSEG/Bloomberg coverage counts; Totem
month-end consensus (~30 submitters); ~51% broken-dated / 20% turn volume (single venue); IMM =
3rd Wed Mar/Jun/Sep/Dec. *Celnet proposal (not a competitor fact):* the surface-role
(working/official/IPV) taxonomy, the override-layer, the version-diff "differentiator"
positioning, the EventClock design, nudge tick-sizes, the Desk Surface Monitor, and all
incumbent *screen-layout* specifics (held low / excluded). No fabricated product names: LSEG's FX
vol products are **FXVS/FXVE (FENICS-powered)** — there is **no LSEG product called "SURF"**;
Totem is a **month-end consensus** service, **not** a live intraday peer band.

---

## 9. Citations

**Quant / convention (high):**
- D. Reiswich, U. Wystup, *FX Volatility Smile Construction* (2010) — ATM/RR/BF pillars; market
  vs. smile strangle; calibration. (canonical; project CONVENTIONS.md §1.4 hard rule.)
- A. Castagna, F. Mercurio, *The Vanna-Volga Method for Implied Volatilities* (2007) — VV; broker
  strangle; wing behaviour. https://www.deriscope.com/docs/The_Vanna_Volga_method_for_implied_volatilities_Castagna_Mercurio_2007.pdf
- F. Bossens, G. Rayée, N. Skantzos, G. Deelstra, arXiv:0904.1074 §3.3 — smile-related quotes /
  broker's strangle root-find. https://arxiv.org/pdf/0904.1074
- J. Gatheral, A. Jacquier, *Arbitrage-free SVI volatility surfaces*, arXiv:1204.0646 — SSVI;
  static arb ⟺ butterfly-free per slice + total variance ↑ in maturity. https://arxiv.org/abs/1204.0646
- Hendriks & Martini, *The Extended SSVI Volatility Surface* (2019) — original eSSVI
  (maturity-dependent correlation). **Distinct from** S. Mingone, arXiv:2204.00312 (global no-arb
  *parametrisation* of eSSVI). https://arxiv.org/pdf/2204.00312
- J. F. Clark, *Foreign Exchange Option Pricing* — sticky-delta; ACT/365 vol time vs. MM-basis
  discount time; temporal interpolation (holidays/weekends); event vol.
- I. Clark / MathFinance, *Calendar arbitrage in the FX volatility surface* — FX-specific
  calendar-arb (total-variance crossing).
- J. Healy, *Counterexamples for FX Options Interpolations*, Parts I & II, arXiv:2512.19621 /
  arXiv:2512.19625 (Dec 2025) — broker-quote interpolation pitfalls. https://arxiv.org/abs/2512.19621
- quantpie FX summary (RR/strangle/delta conventions): https://www.quantpie.co.uk/fx/fx_summary.php
- FinPricing FX vol intro: https://finpricing.com/lib/FxVolIntroduction.html
- S. Mital, *FX Volatility Surface*: https://www.linkedin.com/pulse/fx-volatility-surface-13-swati-mital

**Market structure / valuation control (high/medium):**
- BIS / Basel, *Supervisory guidance on valuation* (bcbs153) — independent-of-dealing-room
  verification, ≥ monthly; VCG feeds GL. https://www.bis.org/publ/bcbs153.htm
- S&P Global *Totem* / consensus-pricing studies — month-end consensus, ~30 submitters/contract.
- LSEG turn-impact insight — ~51% FX forwards broken-dated, ~20% volume on turns (single venue).
- LSEG FENICS / FXVS / FXVE — 340+ pairs surface coverage; FX Volatility Explorer.
- Bloomberg *Real-Time Volatilities* PDF; Mathema OVML/OVDV docs — OVDV ATM/RR/BF → extended
  delta grid; BVOL 200+ pairs. https://help.mathema.com.cn/latest/docs/toolbox/bbg_ovml
- Murex MX.3 spotlights — SLV (GPU-accel), advanced vol marking, real-time book management.
- CME / Wikipedia — IMM = third Wednesday of Mar/Jun/Sep/Dec.
- Moontower — vol time (weekend slow / weekday fast).

**Event variance (cross-asset; flagged):**
- J. Wright, NBER w28306, *Event-Day Options* — large positive variance risk premium, esp. FOMC
  (Treasury/equity-index futures, **not FX**). https://www.nber.org/papers/w28306

**Internal (project):** `docs/CONVENTIONS.md` §1.3/§1.4/§3; `docs/ANALYTICS-SPEC.md` §1.3,
§3.x/§3.6; `docs/TRADING-UNIVERSE-SCALE.md`; `docs/RISK-HIERARCHY.md`;
`crates/celnet-surface/src/{strangle.rs,surface.rs,parametric_surface.rs,termstructure.rs}`;
`crates/celnet-calendar/src/fx.rs`; `gui/src/{workspaces/SurfaceWorkspace.tsx,
viz/SmileChart.tsx,viz/SurfaceMesh.tsx,data/surface.ts,data/contract.ts}`.

**Low / excluded:** all incumbent *screen-layout* specifics; SuperDerivatives/360T ticket UX;
"VOLC" mnemonic; any FX event-vol magnitudes; the version-diff "differentiator" (positioning).

---

## 10. Backlog (proposed)

> This section IS the proposed backlog. Do **not** edit `docs/ROADMAP.md` — the orchestrator
> merges from here. Each item: one-line scope + crate/area + dependency. Proposed identifiers
> are vendor-neutral.

1. **Editable marking grid (`MarkingGrid`)** — primary delta×tenor handle grid; all of
   ATM/25RR/25BF/10RR/10BF editable, keyboard-nav, vol-convention nudge steps. *Area:*
   `gui/` (`SurfaceWorkspace`). *Dep:* none (UI); transports via existing `MarkSurface`.
2. **Handle model + live recalibrate** — editing a handle re-runs market→smile-strangle
   fixed-point + smile build; model selector + per-mark model provenance. *Area:* `gui/` +
   `celnet-server` marking seam (`celnet-surface/src/strangle.rs` exists). *Dep:* #1.
3. **Transport-the-edit + working/official + publish/version** — fix the mismark hazard: edits
   reach the engine; **Re-mark ≠ Publish**; promote a working surface to official under a fresh
   `surface_version`. *Area:* `gui/` + `celnet-server` (`surface_book`). *Dep:* #1; contract
   evolution #11.
4. **No-arb violation surfacing (per-slice + cross-tenor)** — replace hardcoded
   `calendarArbitrageFree: true`; surface the real `SurfaceArbitrageReport` (butterfly + calendar
   `min_calendar_increment`) at the offending node; Publish gates on the whole report. *Area:*
   `gui/` + expose `SurfaceArbitrageReport` over the contract (`celnet-surface` ready). *Dep:* #11.
5. **Term-structure curves + smile inspector + mesh demotion** — ATM/RR/BF-vs-tenor curves as the
   second view; stable-y-scale smile inspector with handle markers; 3-D mesh → optional QC panel.
   *Area:* `gui/` (`viz/`). *Dep:* none.
6. **Compare / history / consensus overlay** — Δ-overlay vs. live composite, history, month-end
   consensus (cadence-labelled). *Area:* `gui/` + feed. *Dep:* composite/consensus feed in
   `docs/CELNET-INTEGRATION.md` (data dependency — do not specify as if it exists).
7. **Per-tenor smile inspector with sticky-delta/strike toggle** — spot-move simulation showing
   sticky-delta (default) vs. sticky-strike re-mapping. *Area:* `gui/`. *Dep:* #5.
8. **`EventClock` + event-variance stripping (validated)** — concrete `EventClock` over the
   `BusinessClock` trait; ATM-only event bumps at v1; calibration reproducing marked pillars;
   **QuantLib/reference validation** (gate #5); open/free event-calendar source. *Area:*
   `celnet-surface` (+ `celnet-calendar`). *Dep:* none new; research/validation item.
9. **IMM resolver in `celnet-calendar`** — third-Wednesday IMM dates (turn concentration). *Area:*
   `celnet-calendar`. *Dep:* none. **— DONE** (shipped as `Tenor::Imm(n)`, `celnet-calendar/src/fx.rs`);
   what remains is wiring the dual-mode tenor⇄date GUI control to it under item #10.
10. **Broken-date, event-aware pricer + dual-mode expiry (tenor⇄date) + two clocks** — total-
    variance interpolation on the event clock; ACT/365 vol vs. MM-basis discount; mark-to-impact
    to the ticket. *Area:* `gui/` + `celnet-server` + `celnet-surface`. *Dep:* #8, #9; contract
    evolution #11.
11. **Contract evolutions (in place, no versioning)** — `ArbReport` per-tenor-pair calendar
    status; `MarkedSurface`/`Smile` surface-role + market-context (spot/fwd/depo); `BrokerQuoteSet`
    quoted-BF vs. smile-strangle + conventions + override layer; scheduled-event + interpolation-
    weight fields. *Area:* `celnet-proto` / `celnet-server` / `gui` contract mirror. *Dep:* drives
    #3, #4, #10 (coordinate — frozen interface crate, GUIDE.md).
12. **Override-layer model** — base (auto-calibrated) + tracked nudges (single handle) and macro
    curve edits (parallel/twist/steepen ATM; widen/compress RR/BF), each revertible/attributable.
    *Area:* `gui/` + `celnet-surface` macro-edit calls (**verify/build the whole-curve edit API
    before committing the macro controls** — gate #2, no faked UI). *Dep:* #2, #11.
13. **Min-perturbation no-arb repair** — projection onto the no-arb set ("smallest fix");
    **engine/research item**, not a free UI affordance. *Area:* `celnet-surface`. *Dep:* #4.
14. **Multi-pair Desk Surface Monitor** — per-pair blotter (ATM sparkline, RR/BF level, no-arb,
    staleness, working-vs-official, vs-broker/consensus), batch/streamed, staleness-aware,
    counterparty-tiered spreading. *Area:* `gui/` + streamed cube. *Dep:* `TRADING-UNIVERSE-SCALE.md`
    (cube sizing/fan-out); #4, #6.
