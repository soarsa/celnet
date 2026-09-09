# Celnet — Post-GA Roadmap: from "GA pricing platform" to "most complete in the category"

> **Purpose.** The GA-readiness synthesis (`docs/GA-READINESS.md`) returned **GO** for the
> Celnet pricing platform with one gating caveat (end-to-end latency-under-load). This document
> picks up *after* that GA tag: it is the concrete, honestly-scoped plan that takes Celnet from a
> demonstrably state-of-the-art **engine** to **demonstrably the most complete product in the
> FX-options category** — out-functioning, out-intuiting and out-performing SynOption, Fenics,
> Bloomberg, 360T and Digital Vega not just on the deep-narrow universe we ship today, but on
> coverage breadth and structured-product depth where incumbents currently lead.
>
> **Honesty boundary (carried forward from `MASTER-PLAN.md`).** Every epic below states the
> competitor gap it closes, the owner crate(s), and the **validation gate that proves it done**.
> "New" vs "incremental" is flagged explicitly. Nothing is claimed done from this environment that
> depends on real GPU hardware or a live Celer/FIX estate; those are flagged as deployment gates.
> Naming stays `celnet`-logical and vendor-neutral (guardrail 8); the wire contract stays single
> and unversioned (guardrail 9). This doc is the source of truth for post-GA sequencing and is
> kept in sync as epics land (zero-legacy).

---

## 0. The competitive picture in May 2026 (verified)

A snapshot of where the incumbents actually are today, used to calibrate this roadmap. Sources
listed in §9.

| Competitor | Pairs / asset breadth | Products | Architecture posture | Where they still beat Celnet |
|---|---|---|---|---|
| **SynOption** (Optimus venue / Primus vol data / Orion pricing / Synchro crypto) | **~75 FX pairs (deliverable + NDF) + crypto options** (Synchro; OrBit Markets LP) | Vanillas, multi-leg, mainstream exotics (digital/OT/DNT/barrier), **TARF / pivot / accumulator** | MAS-licensed RMO venue; RFQ seconds-scale; closed analytics; SaaS-only | **Pair breadth + crypto + structured products + a live venue/GUI** |
| **Fenics** (FMD FXO 2.0 + kACE) | **300+ FX pairs + 27 precious-metal pairs**; 350+ spot/fwd/NDF, ~120 with option surfaces | kACE Kalahari maths: 20 digitals/barriers, **16 window barriers, quanto, LSV** | Data vendor + heavyweight desktop analytics; snapshot/SFTP, ML wing-fill (opaque) | **Pair + metals breadth; quanto/window-barrier catalog depth** |
| **Bloomberg** (OVML / BVOL / MARS) | **BVOL ~200+ FX pairs** | OVML strategy/structuring/backtest; MARS cross-asset risk + regulatory market risk | Closed seat-priced terminal; MARS API/Python; not an embeddable µs engine | **Pair breadth; cross-asset/regulatory risk credibility; backtesting** |
| **360T** (Deutsche Börse; 360TGTX, SUN, 3DX) | **162 pairs on 360TGTX**; spot/swaps/options/NDF + **Crypto NDFs** | RFQ/RFS/streams; pre-configured 2-leg strategies (RR, zero-cost) | Exchange-grade venue; swaps/spot/algo-centric; FX-options analytics not core | **Venue network + crypto NDF + APAC (CNH) reach** |
| **Digital Vega** (Hydra) | FX-options-focused | **Hydra**: structure / price / RFQ / automate FX-options workflows, "close to street price" | Workflow network; outsources real pricing to LPs | **Structuring-workflow GUI maturity** |

**The pattern.** No incumbent combines open extensibility + Rust µs latency + zero-downtime +
GPU-as-a-service + native lifecycle — that is Celnet's built moat. But on three axes the
incumbents *visibly* lead and a buyer's first checklist question will expose: **(1) pair / asset
breadth**, **(2) structured-product catalog**, and **(3) crypto options**. This roadmap closes
those while pushing the differentiators incumbents cannot copy quickly.

---

## 1. Currency-pair + asset-class breadth

### 1.1 The honest gap

Celnet ships **~9 G10 pairs + a few EM NDFs**, no crypto, no metals. SynOption ~75 + crypto,
Fenics 300+ + 27 metals, Bloomberg ~200+, 360T 162 + crypto NDF. On a procurement checklist this
is the single most visible deficit, and it understates Celnet because **the math is
pair-agnostic** — Garman-Kohlhagen, the surface models (VV/SABR/SVI/SSVI), the arbitrage gates,
the exotics engines and the LSV calibration are all parameterised by `(F, DF_d, DF_f, σ, T)` and
convention records. Adding a pair is **data + convention + calendar work, not new math**.

### 1.2 Why this is mostly incremental — and the quantification

What a new pair actually requires in the current architecture (`celnet-conventions`,
`celnet-calendar`, `celnet-types`):

- **A convention record** per `(pair, tenor)`: delta type (of the 4), ATM type (DNS/ATMF),
  premium ccy + style, spot lag, cut (NY/Tokyo), day counts, settlement style (deliverable vs
  NDO). The *types and solver already exist and are test-verified*; this is filling a row, not
  writing code.
- **Calendar data**: each currency's holiday set + spot-lag rule. The dual-calendar intersection
  engine, modified-following, EOM and the +USD cross rule are all built and tested (43 calendar
  tests today, 8 currency calendars). A new currency = a holiday table + a spot-lag entry.
- **For NDF/NDO pairs**: the named fixing source (KFTC18 / WMR / EMTA / central-bank), the
  fixing-to-settlement lag, and settlement-curve discounting. The NDO settlement *logic* exists;
  this is registering the fixing metadata.

**Quantified work (honest estimate).** Per pair: ~5–20 lines of convention/calendar data + a
golden-row or two. The cost is **breadth × validation**, not depth:

| Tranche | Universe | Net-new code | Dominant cost |
|---|---|---|---|
| **B1 — full G10 matrix** | All ~28 G10 crosses (EUR/GBP/JPY/CHF/AUD/NZD/CAD/USD/NOK/SEK + crosses) | calendar tables for NOK/SEK; cross-via-USD rules already exist | convention-correctness validation per pair |
| **B2 — liquid EM NDF/NDO** | USDKRW, USDTWD, USDINR, USDIDR, USDPHP, USDCNH, USDBRL, USDCLP, USDCOP, USDRUB-class, USDTRY, USDZAR, USDMXN | fixing-source registry entries + holiday tables | fixing semantics + EM RR/BF (high-skew) calibration robustness |
| **B3 — deliverable EM + exotics-grade crosses** | toward ~75–120 pairs (parity with SynOption / Fenics option-surface set) | mostly data | broker→smile calibration stability on thin/high-RR smiles |

The genuinely-new engineering inside breadth is **not** per-pair — it is the *cross-cutting*
robustness work that high-skew EM pairs expose: the broker→smile strangle calibration (already
the documented "#1 production bug" we solved) must stay stable on 10-delta wings for pairs like
USDTRY/USDBRL where RR is large; and the strike↔delta solver's premium-adjusted non-monotone
branch is exercised harder. Those are **hardening tasks on existing code**, validated by
extending `celnet-golden` and `celnet-parity` with EM rows.

> **Verdict: incremental, high-leverage.** The breadth gap is real on a checklist but cheap to
> close because the platform was designed convention-first. The work is a *data + calendar +
> golden-validation* campaign, not new quant. This is the highest ROI line in the whole roadmap.

### 1.3 Crypto options — genuinely new, and worth doing

SynOption (Synchro), 360T (Crypto NDF) and the broader market (Deribit dominance, Paradigm
institutional flow, multi-$B BTC/ETH expiries) make crypto options a live institutional asset
class in 2026, and an FX-options desk increasingly wants it on the **same surface/risk fabric**.
This is **new** work, not a convention row, because crypto options have structural differences:

- **Inverse / coin-settled options** (Deribit standard): premium and settlement in the base coin
  (BTC/ETH), USD-equivalents via index; payoff is `max(S−K,0)/S`-style, **not** plain GK. This is
  a new payoff family and a new Greek-reporting convention (coin-denominated delta/gamma).
- **No carry/rate analogue**: "foreign rate" is replaced by a funding/forward basis; the forward
  is exchange-implied, not interest-parity. The surface lives in **forward-vol / DVOL** terms.
- **Extreme skew + fat tails + 24/7 calendar**: SVI/SSVI and SABR still apply, but the calendar
  engine needs a 24/7 (no-holiday) mode and the arbitrage gates must tolerate steeper wings.

Crypto is therefore a **deliberate new module**, scoped as its own epic (E-3), reusing the
surface/arbitrage/exotics substrate but adding an inverse-payoff pricer and a coin-settlement
convention. It is *optional* relative to FX-options leadership but is the fastest way to claim a
breadth story SynOption/360T already market — and it is a clean fit for the open SDK (a partner
could ship a crypto payoff as a plugin).

---

## 2. Remaining analytics gaps vs the incumbents

Celnet's analytics core (vanilla + 13 Greeks, VV/SABR/SVI/SSVI arb-free surface, first-gen
exotics + Asian + window-barrier + LSV) is QuantLib-gated and, on the *transparency/arb-free*
axis, already ahead. The honest remaining gaps, mapped to the incumbent that exposes them:

| Analytic | Status in Celnet | Incumbent that has it | New vs incremental |
|---|---|---|---|
| **TARF / accumulator / decumulator / pivot / TARN** | ⛔ deferred (LSV+MC+PDE substrate ready) | SynOption, Fenics, 360T, Murex | **New payoff layer** on existing MC engine — see E-1. The substrate exists; the path-dependent target-redemption/knockout/gap-risk logic and gearing do not. |
| **Quanto** (option payoff in a third currency) | ⛔ deferred | Fenics kACE, Bloomberg, Murex | **Incremental** — quanto is a drift/correlation adjustment on the existing GK + a correlation input; closed-form for vanillas, MC for path-dependent. |
| **Window / partial barrier breadth** | ✅ (LSV PDE + MC) | Fenics (16 window barriers) | **Incremental** — engine exists; this is parameterising multi-window / per-window rebate variants and golden-validating the matrix. |
| **Lookback / forward-start / cliquet** | ⛔ deferred | Fenics, Murex/Numerix | **Incremental-to-moderate** — MC/PDE engines exist; forward-smile-sensitive payoffs need the LSV path engine (built) + payoff structs. |
| **Variance swap / volatility swap** | 🟡 specified (log-contract; Carr-Lee) | Bloomberg, Murex/Numerix, ICE | **New** — model-free log-contract replication (`1/K²` strip) + the vol-swap convexity adjustment; needs the arb-free surface (built) as the strip source. |
| **Correlation / basket / dual-FX (best-of, TARN on two FX)** | ⛔ deferred | Murex/Numerix, Fenics double-FX TARN | **New + research-grade** — needs a multi-asset correlated MC (correlated Philox streams + a correlation/covariance input) and basket payoffs. Highest-effort analytics epic. |
| **American / Bermudan** | ✱ near-non-goal for FX | (rare in FX OTC; relevant for listed FX futures options) | **Deliberately low priority** — OTC FX vanillas are European; only listed FX-future options and some retail wrappers are American. Scope only if a listed-options or futures-option lane is greenlit; LSV-PDE + a longstaff-schwartz MC would cover it. **Flagged: not an FX-OTC gap.** |
| **Inflation-style** | ✱ non-goal | (rates/inflation desks) | **Out of category** for FX options; explicitly excluded. Bloomberg MARS covers inflation as a *cross-asset* product, not an FX-options analytic. Do not chase. |
| **Cross-asset / regulatory market risk (FRTB SA/IMA, scenario)** | 🟡 arb-free report is an IPV asset; full FRTB not built | Bloomberg MARS | **New, adjacent** — a risk/regulatory layer, scoped as a differentiator epic (E-7) rather than an analytic, since it leverages the auditable arb-free surface. |

**Net analytics read.** The only *true* analytics gaps that an FX-options buyer will test are
**structured products (TARF/accumulator/pivot)** and **variance/vol swaps + quanto**. Correlation
/ basket is genuinely new and research-grade — pursue it as a depth differentiator, not a
checkbox. American/Bermudan and inflation are **not FX-OTC gaps** and are explicitly *not*
prioritised — chasing them would be gold-plating against the wrong category.

---

## 3. Productized differentiators to push harder (the un-copyable moat)

These are capabilities **no incumbent offers** and which a closed terminal / venue cannot quickly
replicate. Post-GA, they shift from "built" to "productized and marketed as a category-defining
edge."

### 3.1 The open quant-model marketplace (E-5)
The plugin host (wasmi, fuel-metered, deterministic, capability-isolated) is built and the
contract is frozen. The *product* is not yet built: a **marketplace** where a desk's quants and
third parties publish/consume vol models, exotic payoffs and calibrations as signed,
deterministically-replayable plugins. This is the headline "out-functions" story — Bloomberg,
SynOption and Fenics structurally cannot offer a customer-extensible model SDK. Epic E-5 turns
the runtime into a product: a `celnet-plugin-guest` SDK crate, a signing/provenance scheme
(Tier-1 trusted `.so` via stabby + Tier-3 Landlock ring, both designed), a registry/catalog, and
a determinism-attestation gate (every published plugin must pass bit-identical replay).

### 3.2 GPU-as-a-service (E-6)
Cross-platform GPU Monte-Carlo (wgpu → Metal/Vulkan/DX12) with f64-CPU reconciliation is built; no
competitor exposes GPU pricing as a live service. The product gap is **proven batch
throughput/latency at IB-portfolio scale on real hardware** (Metal lacks f64; CUDA is CI-only) —
honestly a deployment/hardware gate. Push it as: a batch-revaluation service (whole-book vega/gamma
ladders, scenario grids) priced in deterministic milliseconds, validated against the ≤50 ms
booking-grade budget, with the f64-CPU oracle as the always-on correctness check.

### 3.3 The zero-downtime story (E-4 substantiation)
Blue-green zero-downtime hot-upgrade (single-current-contract cutover, no mixed-version window) is
built and is the direct answer to Murex's multi-year/MXTEST upgrade pain. Post-GA this becomes a
*demonstrated operational property*: a scripted live-upgrade drill under sustained streaming load
showing zero dropped quotes / zero P&L gap across a model+engine version cutover, wired into the
fleet layer (E-4) so it holds across nodes, not just in-process.

### 3.4 Convention transparency + auditable arb-freeness as an IPV/FRTB product
Already a built edge (convention on every message; butterfly/calendar/vertical gates as a
consolidated report). Push it as a **valuation/IPV/model-validation product** — the thing a
bank's model-validation function signs first — and the on-ramp to the regulatory-risk epic (E-7).

---

## 4. Prioritized, sequenced epic list

Each epic: the competitor gap it closes · owner crate(s) · the validation gate that proves it
done. Sequencing respects lane discipline (no two concurrent epics mutate a crate the other
compiles) and builds on the `MASTER-PLAN.md` Gap→closure map (G-A…G-G), which this supersedes for
post-GA scope.

> **Priority key:** **P0** = closes a visible checklist gap, low risk, high leverage · **P1** =
> high-value depth, moderate effort · **P2** = differentiator hardening / research-grade.

### Wave 1 — close the visible checklist gaps (parallel, disjoint dirs)

**E-1 · Structured products: TARF / accumulator / decumulator / pivot / TARN** — **P0, new**
- **Gap closed:** SynOption Optimus, Fenics kACE, 360T, Murex all ship these; it is the largest
  *functional* breadth gap and the first thing a structuring desk asks for.
- **Owner crate(s):** `celnet-exotics` (payoff + path-dependent target/knockout/gearing logic on
  the existing Philox MC + LV/LSV substrate), `celnet-golden` (reference prices).
- **Validation gate:** each payoff MC-cross-validated against an independent reference
  (QuantLib where available, else a published/analytic-approx benchmark); explicit
  **target-redemption + knockout + gap/digital-risk** tested; accumulator **gearing** and pivot
  above/below-pivot settlement tested; new rows added to `celnet-parity`. Suitability/disclosure
  metadata carried on the instrument.

**E-2 · Currency-pair + NDF breadth campaign (B1→B2→B3)** — **P0, incremental**
- **Gap closed:** SynOption ~75 / Fenics 300+ / Bloomberg ~200+ pair breadth — the #1 checklist
  deficit.
- **Owner crate(s):** `celnet-conventions` (convention rows), `celnet-calendar` (holiday tables +
  spot-lag), `celnet-types` (NDO fixing-source registry), `celnet-golden`/`celnet-parity` (per-pair
  validation rows). **No core math changes** — purely data + calendar + validation.
- **Validation gate:** for each tranche, every pair has a convention record + calendar + ≥1 golden
  row validated to ~1e-10; EM high-RR pairs additionally pass a broker→smile calibration-stability
  test on 10-delta wings and a strike↔delta non-monotone-branch test. Tranche done when
  `celnet-parity` shows the pair count meeting/exceeding the named incumbent.

**E-3 · Crypto options (inverse / coin-settled) on the shared surface fabric** — **P1, new**
- **Gap closed:** SynOption Synchro, 360T Crypto NDF — a marketed breadth story; positions Celnet
  as one risk fabric for FX + crypto vol.
- **Owner crate(s):** new `celnet-crypto` (inverse-payoff pricer + coin-denominated Greeks +
  exchange-implied forward/basis), reusing `celnet-surface` (SVI/SSVI on DVOL-style quotes) and
  `celnet-calendar` (24/7 no-holiday mode); a convention entry in `celnet-types`.
- **Validation gate:** inverse-option price/Greeks reconciled against the Black-on-forward
  convention used by the dominant venue and an independent MC; arb-free gates hold on steep crypto
  wings; 24/7 calendar tested. Honestly scoped: this is *pricing/analytics* for crypto vol, not a
  crypto execution venue.

### Wave 2 — depth analytics + the latency/scale substantiation (after Wave 1 stabilises crates)

**E-4 · Cross-node fleet / scale-out + zero-downtime drill under load** — **P0, new**
- **Gap closed:** substantiates the IB-portfolio-scale claim beyond per-node headroom; converts
  the zero-downtime story from in-process to fleet-wide (vs Murex upgrade pain). Also carries the
  GA gating caveat: **end-to-end latency under load** (HdrHistogram wire-path p50/p99/p99.9).
- **Owner crate(s):** new `celnet-router` (HRW partition map, stateless replica routing,
  hot-standby, backpressure); `celnet-bench` (drives the server edge under sustained streaming
  load); CI bench-regression gate.
- **Validation gate:** partition-map balance + failover + no-loss handoff tests; wire-path
  HdrHistogram under load proving §1.2 budgets through the edge; a scripted blue-green upgrade
  drill showing **zero dropped quotes / zero P&L gap** across a version cutover under load; CI gate
  so latency cannot silently regress.

**E-5 · Open quant-model marketplace (runtime → product)** — **P1, new (product) / built (runtime)**
- **Gap closed:** the un-copyable "out-functions" differentiator — no incumbent offers a
  customer-extensible model SDK. Closes nothing on the *incumbent* checklist; *opens a category*.
- **Owner crate(s):** new `celnet-plugin-guest` (author SDK), `celnet-plugin-host` (Tier-1 signed
  `stabby` `.so` + Tier-3 Landlock/seccomp ring — both designed in `PLUGIN-HOST-ALT.md`), a
  registry/catalog surface in `celnet-server`.
- **Validation gate:** a published plugin (vol model + exotic payoff) loads across all three tiers,
  passes **bit-identical determinism replay** as a publish-time attestation gate, and is callable
  through the live API; capability-denial + fuel-exhaustion gates still green; signing/provenance
  verified.

**E-6 · Variance / volatility swaps + quanto + lookback/forward-start breadth** — **P1, mixed**
- **Gap closed:** Bloomberg/Murex/Numerix/Fenics depth — variance/vol swaps (new), quanto
  (incremental), forward-smile-sensitive payoffs (incremental on the built LSV path engine).
- **Owner crate(s):** `celnet-exotics` (var/vol swap log-contract strip + Carr-Lee convexity;
  quanto drift/correlation adjustment; lookback/forward-start payoffs), `celnet-surface` (the
  arb-free strip source), `celnet-golden`/`celnet-parity`.
- **Validation gate:** var-swap strike = model-free OTM strip integral cross-checked vs MC;
  vol-swap ≠ √(var-swap) convexity adjustment tested; quanto reconciled vs closed-form +
  correlated MC; new parity rows.

### Wave 3 — research-grade depth + adjacent differentiators

**E-7 · Correlation / basket / dual-FX (best-of, double-FX TARN) + regulatory-risk layer** — **P2, new/research-grade**
- **Gap closed:** Murex/Numerix multi-asset depth (basket/correlation) + Bloomberg MARS
  cross-asset/regulatory market-risk credibility — the deepest, least-commoditised analytics.
- **Owner crate(s):** new `celnet-multiasset` (correlated Philox streams, covariance/correlation
  input, basket + dual-FX payoffs); a risk/regulatory layer leveraging the auditable arb-free
  report (FRTB-style sensitivities/scenario) — scoped as a *report*, not a full SA/IMA engine.
- **Validation gate:** correlated MC validated against analytic basket benchmarks where they exist
  (geometric basket) + convergence/CI on arithmetic; double-FX TARN target-redemption tested;
  scenario/sensitivity report reconciled against the single-asset Greeks. **Flagged research-grade
  — pursue as a depth differentiator, not a checklist item.**

**E-8 · GPU-as-a-service: proven batch throughput at scale (real hardware)** — **P2, deployment-gated**
- **Gap closed:** the live-service batch capability no competitor exposes.
- **Owner crate(s):** `celnet-gpu`, driven by `celnet-router`/`celnet-bench`.
- **Validation gate:** large-batch GPU vs f64-CPU reconciliation (correctness) + throughput/latency
  vs the ≤50 ms booking-grade budget. **Honesty:** real-HW numbers are a CI/container + deployment
  gate (Metal lacks f64; CUDA is CI-only) — built and validated against simulation here, the
  real-hardware step flagged, never claimed done from this environment.

**E-9 · Live Celer / FIX trade-lifecycle integration** — **P1, deployment-gated**
- **Gap closed:** native STP — a structural edge over SaaS/venue incumbents (no feed-and-reconcile
  tax); table-stakes FIX connectivity that competitors all have.
- **Owner crate(s):** `celnet-integration` (FIX dialect + distributor adapter).
- **Validation gate:** FIX round-trip + distributor adapter against a **simulated** Celer estate;
  live wiring flagged as a staging/deployment gate, not claimed from here.

### Sequencing summary

```
Wave 1 (parallel):  E-1 celnet-exotics  ·  E-2 conventions/calendar  ·  E-3 new celnet-crypto
Wave 2 (parallel):  E-4 new celnet-router + bench/CI  ·  E-5 plugin marketplace  ·  E-6 celnet-exotics (after E-1)
Wave 3:             E-7 new celnet-multiasset  ·  E-8 celnet-gpu  ·  E-9 celnet-integration
```
Lane note: E-1 and E-6 both touch `celnet-exotics` and are therefore sequenced (E-1 then E-6),
not concurrent. E-2 (conventions/calendar) is disjoint from everything and can run continuously
as a background campaign.

**"Most-complete-in-category" definition-of-done:** Celnet claims category leadership when
E-1 (structured products), E-2-B2 (≥ liquid-EM-NDF breadth toward ~75 pairs), E-4 (latency-under-
load + fleet) and E-5 (the marketplace) are all green and gated — at which point Celnet matches or
exceeds the incumbents on the three visible checklist axes **and** holds the open-SDK / µs-latency
/ zero-downtime / GPU moat none of them can match.

---

## 5. New vs incremental — the honest ledger

| Epic | New or incremental | Why |
|---|---|---|
| E-1 Structured products | **New payoff layer** | Path-dependent target-redemption/knockout/gap/gearing logic does not exist; the MC/LSV substrate does. |
| E-2 Pair/NDF breadth | **Incremental (data campaign)** | Math is pair-agnostic; cost is convention/calendar data + per-pair validation. Cross-cutting EM-calibration robustness is the only code work. |
| E-3 Crypto options | **New module** | Inverse/coin-settled payoff, exchange-implied forward, 24/7 calendar are structurally different from GK. |
| E-4 Fleet + latency-under-load | **New (router) + substantiation** | New `celnet-router` crate; the latency proof is converting micro-bench to wire-path. |
| E-5 Plugin marketplace | **New product on built runtime** | Runtime + contract built; SDK/signing/catalog/attestation are new. |
| E-6 Var/vol swap + quanto + lookback | **Mixed** | Var/vol swap new; quanto incremental; forward-smile payoffs incremental on built LSV engine. |
| E-7 Correlation/basket + reg-risk | **New, research-grade** | Multi-asset correlated MC and a regulatory report are genuinely new and the deepest work. |
| E-8 GPU-at-scale | **Substantiation (deployment-gated)** | GPU engine built; the gap is proven real-HW throughput. |
| E-9 Celer/FIX | **Incremental + deployment-gated** | Adapters extend `celnet-integration`; live wiring is a staging step. |

**Deliberate non-goals (flagged so they are not mistaken for gaps):** American/Bermudan beyond a
listed-FX-futures-options lane (OTC FX is European); inflation-style products (out of category);
becoming a regulated multi-bank RMO venue à la SynOption Optimus (a conscious non-goal — Celnet
sits behind/alongside any venue as the pricing/risk brain).

---

## 6. Validation philosophy (carried into every epic)

Every epic above lands only when test-backed (zero-legacy, guardrail 10): QuantLib-gated where a
golden exists; independent-MC or published-benchmark cross-validation otherwise; arbitrage
invariants asserted; finite-difference Greek checks; determinism (bit-identical replay) for
anything touching MC or plugins; and a new executable row in `celnet-parity` so each capability is
a *gated test*, not a doc claim. The competitive matrix in `docs/CAPABILITIES-VS-COMPETITION.md`
graduates ⛔/🟡 → ✅ for each epic only when the gate is green.

---

## 7. Relationship to existing plans

- **Supersedes** the `MASTER-PLAN.md` Gap→closure map for *post-GA* scope (G-A→E-1/E-6;
  G-B/G-D→E-4; G-C→folded into the GA tag; G-E→GA hardening; G-F→E-8; G-G→E-9), extending it with
  the breadth (E-2), crypto (E-3), marketplace-product (E-5) and research-grade (E-7) epics that
  `MASTER-PLAN.md` only listed as "nice-to-have (post-GA)".
- **Consumes** `docs/CAPABILITIES-VS-COMPETITION.md` §3/§6 as the gap inventory and
  `docs/ANALYTICS-SPEC.md` §4–§6 as the implementation spec for E-1/E-6/E-7.
- **Honours** the honesty boundary, lane discipline and naming/contract guardrails throughout.

---

## 8. One-line "most complete" thesis

> Celnet already out-functions, out-intuits and out-performs the incumbents on the axes a closed
> terminal or RFQ venue **cannot** copy (open SDK, µs latency, zero-downtime, GPU service, native
> lifecycle, auditable arb-freeness). This roadmap closes the three axes where they currently lead
> a buyer's checklist — **pair/asset breadth (E-2, cheap & incremental), structured products (E-1,
> new), and crypto (E-3, new)** — and substantiates the moat (E-4 latency/fleet, E-5 marketplace),
> after which Celnet is demonstrably the most complete product in the FX-options category, not just
> the best engine.

---

## 9. Sources (May 2026 web research)

- SynOption Primus / Optimus / crypto: <https://synoption.com/primus.php>,
  <https://synoption.com/category/optimus/>,
  <https://www.linkedin.com/products/synoption-synchro-advanced-crypto-options-analytics-trading/>,
  <https://tradetechfx.wbresearch.com/sponsors/synoption>
- Fenics FXO 2.0 / kACE (300+ pairs, 27 metals, window barriers/quanto/LSV):
  <https://thefullfx.com/fenics-upgrades-fx-options-vol-offering/>,
  <https://www.fenicsmd.com/products/fx-options/>, <https://www.kacefinancial.com/kace-fxo/>,
  <https://wilmott.com/vol-surfaces-fmd-expands-fx-options-offering/>
- Bloomberg OVML / BVOL (~200+ pairs) / MARS:
  <https://professional.bloomberg.com/products/risk/mars/>,
  <https://www.bloomberg.com/professional/insights/webinar/pricing-fx-options-tips-tricks/>
- 360T (162 pairs, Crypto NDF, SUN/3DX) / Digital Vega Hydra:
  <https://www.360t.com/>, <https://www.360t.com/products/active-trading-suite/360tgtx/>,
  <https://www.digitalvega.com/execution>
- Crypto options (Deribit inverse/coin-settled, institutional flow, 2026 expiries):
  <https://support.deribit.com/hc/en-us/articles/31424939096093-Inverse-Options>,
  <https://insights.deribit.com/industry/crypto-options-at-a-crossroads-macro-stress-heavy-tape-and-relative-vol-opportunities/>
- FX structured products (TARF/accumulator/pivot/TARN semantics):
  <https://www.dbs.com.hk/iwov-resources/pdf/investments/6.%20Product%20Booklet%20-%20OTC%20AQ%20DQ_Target%20AQ%20DQ_Pivot%20Target%20AQ%20DQ%20on%20FX%20en.pdf>,
  <https://finpricing.com/lib/FxTarn.html>, <https://hedgebook.com/understanding-target-redemption-forwards-tarfs/>

*Note: competitor architecture/latency characterisations are positioning inferences from the
absence of published figures and documented RFQ/EOD/batch architectures, not vendor-confirmed
benchmarks — consistent with the disclaimer in `docs/COMPETITIVE-ANALYSIS.md`.*
