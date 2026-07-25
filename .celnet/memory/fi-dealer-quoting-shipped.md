---
name: fi-dealer-quoting-shipped
description: "FI dealer quoting desk (RFQ/IOI inbox, deal blotter, notifications) + rates Book shipped 2026-06-27"
metadata: 
  node_type: memory
  type: project
  originSessionId: 3759f335-a305-4b8e-b3aa-727dea5a173c
---

Shipped 2026-06-27 (`25293f8`, branch `fixedincom_risk_ui`, deployed UAT via
deploy option 2). The **maker side** of the franchise, full vertical slice over
the one unversioned contract. Operator chose (AskUserQuestion): a **dedicated
notification stream** (not multiplexed on the price RFS session) and **build the
full slice at once**.

**Contract (celnet-proto):** `RfqDeskService` (SubmitDeskRequest ·
RespondDeskRequest quote|reject · AcceptDeskQuote→books Deal+RatesPosition ·
ListDeskRequests · ListDeals); `NotificationService.StreamNotifications`
(server→client push); `RiskService.{BookRatesPosition,ListRatesPositions}` (the
rates store/Book — see [[rates-position-store-book-plan]]). Every new proto
field/enum variant needs a **leading** `//` doc comment or prost codegen fails
`missing_docs` under clippy `-D warnings` (trailing comments do NOT propagate).

**Server:** in-memory RwLock stores mirror the options `PositionStore`;
`NotificationBroker` = bounded 256-deep per-subscriber mpsc, non-blocking
`try_send` (full⇒skip, closed⇒prune), entitlement-scoped desk fan-out so the hot
path never blocks (§11). Desk pricing **reuses** `price_rates`. WS adds 7 unary
tags + a `subscribe_notifications`/`notification` push channel.

**GUI:** three Fixed-Income workspaces — Quoting (RFQ/IOI inbox + price panel +
respond/reject/accept + counterparty simulator), Deals blotter, Rates Book — +
`NotificationCenter` (signed-in toasts + bell/unread). `MockTransport` has a full
real offline desk lifecycle. Gate GUI with `npm run build` (see
[[gui-gate-uses-production-build]]).

**Env gotcha:** `just` and `cargo-nextest` are NOT installed on this workstation
(the deploy uses Ansible, not `just`). Gate with plain `cargo`/`npm` — equivalent
test set. The big `target/debug/incremental` (≈9G) fills the disk; clearing it is
safe (regenerable) and `CARGO_INCREMENTAL=0` keeps it lean.

**RESOLVED follow-up (branch `fix/varswap-strip-long-tenor`, `c1e0c8c`):** the
`celnet-parity` `var_vol_swap` proptest flake. Root-caused by measurement — the
**production strip is correct** (converged to ~1e-15 at the failing input, unmoved
by 8–16× nodes AND 32·σ√T wings). The **test oracle** under-resolved: adaptive
Simpson in raw strike space seeded on a single whole-wing interval steps over the
ATM spike at long tenor (~2e-6). Fixed test-only by **seeding the oracle on 1·σ√T
log-moneyness sub-intervals** (no production change, no tolerance loosened) →
agrees with the strip to ~1.6e-11; added `oracle_resolves_long_tenor_peak`
deterministic regression; flake-free over 3×256 proptest cases. Lesson: when a
quadrature-vs-quadrature parity check flakes at a domain extreme, MEASURE which
side is converged (self-convergence sweep) before touching tolerances — here the
"reference" oracle was the wrong one. Not yet merged to `main`.
