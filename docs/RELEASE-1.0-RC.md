# Celnet 1.0-RC — the logical commercial release milestone

> The first cut a commercial counterparty could be onboarded against: the complete
> multi-asset FX-options platform, every capability priced through ONE unversioned
> contract, surfaced in all five clients, and verified per `docs/VERIFICATION-CONTRACT.md`
> (independent oracles, cross-client golden corpus, live e2e, the three-pass coverage lint).
> Cut criterion at the bottom; live status on the §6 board + `docs/WORLD-CLASS-BACKLOG.md`.

## 1. What the RC contains (shipped + verified on `main`)

**Pricing core** — one agnostic carry seam (ADR-0008, end-to-end): FX (vanilla +
18 exotic families incl. the corrected at-hit one-touch), metals (>75-pair universe,
XAU/XAG/XPT/XPD + fiat crosses, lease carry), equity (generalized-BSM, dividend/repo),
commodity (Black-76, listed-future options with equity/futures-style margining),
crypto (linear + inverse/coin-margined 1/S_T), linear book (forward/swap/NDF),
perpetual American (exact closed form; b>r calls refused as valueless — typed),
pivot TRA engine, multi-asset baskets, var/vol swaps, LSV-priced window barriers.
**23 product arms + 3 cross-asset option families**, every one carrying a golden
vector + an independent-oracle parity row, enforced by the proto-driven lint.

**Workflow** — single-dealer RFQ + the multi-dealer ranked panel (engine law:
best-bid/offer, deterministic tie-break, last-look; synthetic loopback LPs in-repo,
live LP connectivity = ENV) booked by `(quote_id, lp_id)` across every client.

**Clients (parity-gated)** — GUI (5-asset-class universe, ProductSpec registry,
editable leg-ladder + true inline strike-solve, ranked panel, axe-clean live e2e),
Excel (polymorphic `CELNET.INSTRUMENT` + verbs incl. STRATEGY; per-product table
retired at proven byte-parity), SDK + CLI (typed builders/grammar, conformance ==
server == oracle), WS mirror == gRPC field-for-field with resource-capped edge.

**Rigor floor** — W6 infra crates at zero-survivor mutation + loom + fuzz;
~120 frozen `to_bits` FX byte-identity assertions; the tiered T0/T1/T2 gate SDLC
with the resumable ledger; deny/audit clean; HdrHistogram observability with
bench-gated latency budgets.

## 2. Outstanding to the cut (release-blocking)

| # | Item (backlog dedup-key) | Why blocking | Owner | Status |
|---|---|---|---|---|
| R1 | THIS landing: T2 green → push (arms 30/31 + P0 touch + fan-out batch + SDLC + batch C) | the RC's content | session-B | T2 mid-run |
| R2 | `pivot-wire-surfacing` | an engine a counterparty cannot reach is not a capability; the last unreachable engine | session-B (proto window) | batch D |
| R3 | `cross-asset-client-priced-vectors-ws-e2e` | the cross-asset DONE claims must be e2e-true before commercial onboarding | session-B | batch D |
| R4 | `entitlements-trust-boundary-audit` | client-asserted principal + grant-all-on-absent is not commercially shippable | session-B | batch D |
| R5 | GUI WS family-conformance suite RUNS green at T2 (authored, 135 specs) | proves every GUI-bookable family against the live edge | session-B | rides R1's T2 |
| R6 | `exotics/vanna-volga-overlay-magnitude-unvalidated` | an unvalidated smile overlay on quoted exotics is a pricing-risk hole | session-B | batch D |
| R7 | W6-ANALYTICS mutation floor (exotics/surface/risk-cube/xva/MC) | the pricing core's test depth must match the infra floor | session-A (banked qmc; spend-paused) | resumes on capacity |
| R8 | `surface/crypto-leaf` + `asset-class-neutral-core` | crypto without a quotable surface is half-shipped | session-A (spec banked) | resumes on capacity |
| R9 | Convergence Round 3 verdict carries **zero P0/P1** | the release-readiness oracle | session-B | after R1–R6 |
| R10 | Final joint T2 + ledger/capabilities-doc sync + the release tag note | the cut itself | both | last |

**Explicitly NOT blocking (post-RC roadmap, tracked OPEN):** QMC pathwise wiring
(P2 perf-quality; Philox MC is correct, Sobol is better), LSV market-calibration
front-end, P&L attribution, event-weighted clock, per-leg strategy expiry
(calendar spreads), exotics second-gen (P3), rates leaf (deferred), CLI MC argv
corpus gate (P3). ENV items (live LP/fixing/venue connectivity) are deploy-bound
by definition and never in-repo blockers.

## 3. Cut criterion (all four, evidence journaled in §6)

1. R1–R6 DONE on `main`, each with its gate evidence line.
2. R7+R8 DONE **or** explicitly re-scoped by the operator (they are session-A's
   lanes; the spend-wall may force a scope call — the RC does not silently wait).
3. Convergence Round 3: **zero P0/P1 genuine findings** (P2/P3 → the post-RC backlog).
4. One final clean `just t2` on the cut commit + the implementation-ledger entry
   naming it.
