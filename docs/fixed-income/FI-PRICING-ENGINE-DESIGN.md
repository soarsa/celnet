# FI Pricing & RFQ Engine — High-Level Design

- **Status:** Proposed design direction (2026-06-30). Synthesises an external
  fixed-income pricing/RFQ-engine blueprint against celnet's actual architecture and
  records the target shape + fit. **Not yet implemented.** Companion: `docs/adr/
  ADR-0018-fixed-income-as-a-new-asset-class-leaf.md`. Extends ADR-0008 (multi-asset
  carry), ADR-0010 (FI rates onto the carry seam); honours ADR-0007 (one unversioned
  contract) and GUIDE.md guardrails #6 (scale/latency), #8 (vendor-neutral naming),
  #11 (trader-centric, zero-cost observability).
- **Branch:** authored on `feature/fi-reference-data` (the FI platform expansion lane).
  This is a design artifact only — no core contract changes.

## 1. Purpose

A canonical sell-side **cash-bond / credit market-making** engine: auto-price fixed-income
instruments off live curves + credit spreads, stream tiered two-way quotes over FIX,
answer RFQs (auto-quote or escalate to a human trader), and recycle the resulting risk
through an automated hedger. This document maps that target onto celnet and states what is
already built, what extends an existing seam, and what is genuinely new.

## 2. Fit summary (the headline)

**Strong fit, additive — no rearchitecting.** celnet already implements the right two-thirds
of this engine (curve, RFQ workflow, FIX, streaming, rates risk, limits, entitlements). The
missing third — bond/credit **pricing** — slots into seams that already exist. The single
most important reason it fits: **celnet's pricer is already cross-asset** (ADR-0008 routes
FX/metal vs equity/commodity/crypto in `price_instrument` → the `ProductEngine` registry),
so **fixed-income/credit is "another asset class," not a second system.**

What does NOT fit: the blueprint's infra-hardening list (off-heap caches, LMAX Disruptor,
"off-heap Newton-Raphson") is **JVM garbage-collector mitigation**. celnet is no-GC Rust;
that ceremony is moot. Only **kernel-bypass networking** carries over, and only if SLAs
demand it.

## 3. High-level architecture

Vendor names (Tradeweb, Bloomberg, CDX, CDS) live ONLY at the integration edge and in docs;
core identifiers stay purpose-named (`SpreadCurve`, `CreditSensitivities`, `HedgeInstruction`).

```
            INTEGRATION EDGE  (vendor-specific adapters only)
  rates/credit feeds        venue RFQ / streaming             FIX 4.4 sessions
        |                          |                              ^
        v                          v                              |
 +--------------------------------------------------------------+-+------+
 |  celnet-server  (edge: gRPC + WS mirror, auth, journal, telem) |      |  HAVE
 |                                                                       |
 |  +------------- PRICING CORE (Rust, pinned zero-alloc hot path) ----+ |
 |  |  asset-class router (ADR-0008) --> ProductEngine registry (item C)| |  HAVE
 |  |     FX/metal . equity/commodity/crypto . FIXED-INCOME/CREDIT (new)| |  NEW leaf
 |  |            |                                                      | |
 |  |   +--------+---------+                                           | |
 |  |   | curve engine     | celnet-rates bootstrap (+ date pillars)   | |  HAVE
 |  |   | credit / spread  | spread curve . hazard . CR01              | |  NEW
 |  |   | bond analytics   | DCF . YTM (Newton-Raphson) . DV01/dur/cvx | |  NEW
 |  |   +--------+---------+            --> THEORETICAL MID             | |
 |  +-----------+------------------------------------------------------+ |
 |              v                                                        |
 |   quote construction: mid -> inventory skew -> client-tier -> 2-way   |  EXTEND
 |              |                               ^ entitlements / tiers   |
 |     +--------+--------+                                               |
 |     v                 v                                               |
 |  STREAMING          RFQ ENGINE                                        |  HAVE
 |  distributor        rule-eval -> auto-quote (<SLA) | escalate->trader |
 |     |                 |                            |                  |
 |     +-----> FIX gateway / WS mirror <--------------+                  |  HAVE
 |                                                                       |
 |   FILL -> Book/position -> net DV01/CR01 -> limit check -> AUTO-HEDGE  |  EXTEND
 |           (rates Book)     (risk cube)      (celnet-limits)  instr.    |
 +-----------------------------------------------------------------------+
```

The boundary owns all I/O, validation, auth, and error->`tonic::Status` mapping. The leaves
are pure, oracle-validated math and never touch the wire. Telemetry (tracing / metrics /
HdrHistogram) offloads over a bounded queue so the hot core stays log/lock/alloc-free.

## 4. Component catalogue

| Blueprint component | celnet today | celnet home | Action |
|---|---|---|---|
| Market-data handlers / bus | vendor feeds + `celnet-fanout` bounded queue | edge | HAVE (+ FI feed adapters) |
| Internal inventory feed | rates `Book` / position store | `celnet-server` | HAVE |
| Yield-curve builder | `celnet-rates` bootstrap (+ date pillars) | `celnet-rates` | HAVE |
| **Bond analytics** (DCF, YTM/Newton-Raphson, DV01, duration, convexity) | bond *definitions* only | **`celnet-bond` (new leaf)** | NEW |
| **Credit-spread / CR01 service** | — | **`celnet-credit` (new)** | NEW |
| Asset-class routing | ADR-0008 router -> `ProductEngine` | `celnet-server::pricer` | HAVE -> register FI leaf |
| **Quote construction** (inventory skew + client tiering) | RFQ/IOI desk + entitlements exist; pricing tier does not | `celnet-server` quoting svc | EXTEND |
| Streaming distributor | WS/gRPC + HdrHistogram | `celnet-server` | HAVE |
| FIX gateway | `celnet-fix` (desk-owned FIX) | `celnet-fix` | HAVE |
| RFQ rule-eval / auto-quote / trader escalation | FI dealer-quoting (inbox, blotter, notifications, desk GUI) | `celnet-server` + `gui/` | HAVE |
| **Fill -> inventory -> skew -> auto-hedge loop** | `Book` + risk cube + limits + notifications (plumbing) | `celnet-server` hedging svc | EXTEND |
| Latency posture | pinned zero-alloc hot core, telemetry offload | core | HAVE |
| Kernel-bypass NIC | — | edge | OPTIONAL |

## 5. Where it slots into the existing architecture (the seams)

- **Asset-class router (`price_instrument` -> `ProductEngine`).** FI is registered as a new
  asset class beside FX/metal/equity/commodity/crypto. A bond/credit `Instrument` arm
  decodes, routes to the FI leaf, and returns the same `Priced` shape. No new top-level
  service, no second contract (ADR-0007). See ADR-0018.
- **Curve substrate.** Bond/credit pricing consumes the existing `celnet-rates::Curve`
  (the benchmark discount/forward curve) — the bootstrap already shipped. Per ADR-0010,
  `Carry` is the flat degenerate of a term structure; the bond leaf is a pure consumer of
  the term-structure substrate.
- **Risk cube.** DV01 already lives in the cube (key-rate ladder, ADR-0010). CR01 is a new
  **dimension** of the same cube, not a silo — credit positions roll up firm-wide alongside
  rates/FX.
- **Book / limits / entitlements.** The auto-hedge loop and client-tier spreads reuse the
  shipped `Book` (positions), `celnet-limits` (pre-trade breach), and entitlements/capability
  overlay — no new authz model.
- **WS mirror + FIX.** Every new RPC (price-a-bond, build-spread-curve, quote-construction)
  gets the standard gRPC + WS-mirror frame; outbound streaming/RFQ rides `celnet-fix`.

## 6. New components

### 6.1 `celnet-bond` — bond analytics leaf
Pure functions over the existing `Curve`: dirty/clean price by DCF; YTM by Newton-Raphson
(`y_{n+1}=y_n - f(y)/f'(y)`, `f(y)=PV(y)-market`, `f'` analytic); DV01, modified/Macaulay
duration, convexity; accrued interest with the registry's day-count/calendar. **Oracle:**
every output validated against QuantLib and published street prices (not asserted plausible).
Fractional-period / between-coupon dirty pricing handled explicitly (a common spec error).

### 6.2 `celnet-credit` — spread / CR01
Issuer/instrument spread curve over the benchmark (Z-spread, asset-swap spread, or hazard-rate
from CDS); CR01 = PV sensitivity to a 1bp spread shift; feeds the bond leaf's discounting and
the risk cube's new credit dimension.

### 6.3 Quote construction (server service)
`mid -> inventory skew -> client-tier spread -> two-way`. Skew is a function of the `Book`'s
net position/axe (sell-side: long inventory => skew to encourage selling). Tiering reads the
entitlements/capability tier. **Architectural note:** fair value (mid) stays clean; skew and
tier are bid/ask *adjustments downstream* of pricing — not co-inputs to fair value.
The full tiering methodology (flat/inventory/vol/size/toxicity strategies, math, guardrails,
bond bps convention, and the `celnet-tiering` seam) is designed in
[`FI-TIERING-RESEARCH.md`](FI-TIERING-RESEARCH.md).

### 6.4 Fill -> inventory -> auto-hedge loop
On fill (from the journal/transaction log): update `Book` -> recompute net DV01/CR01 -> check
`celnet-limits` -> if breached, emit a `HedgeInstruction` asynchronously (rates: OIS/IR-future
proxy; credit: index/single-name proxy). Closes the risk-recycling cycle the blueprint draws
but leaves one-directional. Dispatch is async/off-hot-path; "zero latency" is async, not zero.

### 6.5 Venue adapters (integration edge)
Vendor-specific RFQ/streaming adapters (Tradeweb/Bloomberg) marshalling to/from FIX 4.4
(`35=W` snapshots for streaming; RFQ request/quote messages). **Blocked** on the venue order /
execution-report model for bond deal-capture (`FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md`) — this
gates capture/distribution, NOT analytics.

## 7. What we deliberately do not adopt

| Blueprint item | Why dropped / translated |
|---|---|
| Chronicle Map off-heap cache | GC-avoidance; Rust has no GC. Use normal Rust ownership / `mmap` only if a real working-set need appears. |
| LMAX Disruptor | A JVM lock-free ring; celnet already does bounded-queue fan-out (`celnet-fanout`) / SPSC in Rust. |
| "Off-heap Newton-Raphson, sub-us" | Conflates math with memory layout; the solve is a few FLOPs on-stack. |
| Kernel bypass (Solarflare/OpenOnload) | KEEP as **optional** — language-agnostic; map to DPDK/io_uring/AF_XDP if p99 SLAs demand. |
| FIX 4.4 `35=W`, RFQ state machine, DV01/CR01 hedge matrix, sub-15ms p99 budget | KEEP — sound and language-agnostic. |

## 8. Latency & SLA posture
celnet's pinned zero-alloc hot core + telemetry-offload already targets the blueprint's
philosophy without JVM tricks. Adopt the stage-budget discipline (ingestion / risk-check /
math / distribution) as HdrHistogram p50/p99/p99.9 SLOs on the existing telemetry, not as new
infra. Bond-leaf math is us per instrument; the ms-scale budgets are book-level curve-rebuild
+ spread + DCF passes.

## 9. Build order (each an oracle-gated milestone)
1. **`celnet-bond`** — DCF / YTM (Newton-Raphson) / DV01 / duration / convexity, validated vs
   QuantLib. (Analytics first; unblocked today.)
2. **`celnet-credit`** — spread curve + CR01; new credit dimension in the risk cube.
3. **FI leaf registration** — bond/credit `Instrument` arm + `ProductEngine` entry (ADR-0018);
   contract + WS mirror + SDK + GUI/Excel views.
4. **Quote construction** — mid -> skew -> client-tier -> two-way (on `Book` + entitlements).
5. **Auto-hedge loop** — fill -> inventory -> DV01/CR01 -> limit -> `HedgeInstruction`.
6. **Venue adapters + bond deal-capture** — unblocks once the venue order/exec-report model lands.

## 10. Open questions / blockers
- **Bond deal-capture** is blocked pending the venue order + execution-report objects
  (`FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md`). Analytics (steps 1-2) proceed independently.
- **Credit-curve sourcing:** CDS-implied hazard vs quoted Z-spread first? (Recommend Z-spread
  off the existing curve for v1; hazard/CDS later.)
- **Auto-hedge execution venue:** proxy hedge instrument set + whether dispatch is advisory
  (trader-confirmed) or fully automated at first. (Recommend advisory-first.)

## 11. References
- `docs/adr/ADR-0008-*` (multi-asset carry), `docs/adr/ADR-0010-converge-fi-rates-onto-carry-seam.md`,
  `docs/adr/ADR-0018-fixed-income-as-a-new-asset-class-leaf.md`
- `docs/CURVES-AND-INSTRUMENT-REFERENCE-DATA-REVIEW.md`, `docs/FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md`,
  `docs/FIXED-INCOME-EXCEL-INTEGRATION-REVIEW.md`, `docs/W4-STRUCTURED-RFQ-PLAN.md`
- `docs/ARCHITECTURE.md` §1.2 (latency/throughput budgets), `docs/SCALE-OUT.md`
