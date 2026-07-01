# P1 "Wire the islands" — implementation lane spec

**Status:** Ready to encode on `coord/board` on operator confirmation. Design-first;
NO code until confirmed. Each lane: build → T1 (scoped) → batched-t2 → land,
coordinator-gated, FX byte-identity + ≤1e-12 preserved. From ADR-0013/0014/0016 +
ARCHITECTURE-TARGET §5 (post-hardening, `d2929e5`).

P1 is deliberately the **low-blast, additive** tier — no shared-enum change (that is P2).
The four lanes are mostly crate-disjoint; the shared touch-point is `celnet-server`
(coordinator sequences those merges). Ordering: **Lane 1 leads** (safety), the rest parallel.

---

## Lane 1 — ⚠ Pre-trade limit gate (SAFETY-FIRST; ADR-0016 A1)
Closes a **real current-code gap**: `AcceptQuote`/`AcceptDeskQuote`/`BookRatesPosition`
book with no limit check.
- **Scope:** `celnet-server/src/services/{clicktrade.rs,stream.rs,fix.rs,quote.rs,desk/mod.rs,risk/mod.rs}`;
  gate at the two POSITION SINKS `PositionStore::book` + `RatesPositionStore::book`
  (the convergence point of all four front-ends). `celnet-limits` is ALREADY a
  `celnet-server` dep (`Cargo.toml:39`) — no manifest change.
- **Approach:** build a `ScopePath` from the lifting token's `TokenBinding`
  (trader→book→desk→firm[→ccy-pair]); call `pre_trade_check` (`celnet-limits/src/check.rs:218`)
  at each sink; `PreTradeDecision::Reject` → `BookOutcome::LimitBreached` → wire
  (`ExecutionReport ExecType=8` "limit breached" on the FIX/exec paths, typed error on gRPC).
  First verify the `TokenBinding` schema actually carries the desk/book/entity/ccy fields the
  limit-tree lookup needs (ADR-0016 open item).
- **Gates:** new tests — a hard-limit-blown book is REJECTED at each of the 4 front-ends
  (clicktrade RFS, FIX `on_new_order`, `AcceptQuote`, `AcceptDeskQuote`, `BookRatesPosition`);
  a within-limit book still succeeds; soft breach warns. No hot-path change (this is the
  booking path, not the pricing thread).
- **Disjointness:** celnet-server services only; no other lane touches the booking sinks.
  Shippable standalone as an immediate safety fix.

## Lane 2 — GPU live path (ADR-0013)
- **Scope:** `celnet-gpu` (BatchPricer/ScenarioPricer already built), `celnet-risk-cube/src/nonadditive.rs`
  (the reprice loop), a NEW `PriceBatch` RPC (`celnet-proto` + `celnet-server`).
- **Approach:** paradigm-matched — the risk-cube reprice loop routes EXACT closed-form nodes
  through a batched closed-form GPU kernel and MC/exotic nodes through `ScenarioPricer`'s MC grid
  (`nonadditive.rs:44-56` refuses MC noise in an exact reval); a batch-size branch selects GPU only
  for large batches (small → CPU). The single-tick streaming/`price_instrument` path is UNCHANGED
  (CPU) — the pinned core `PricingCore::price` (`core.rs:161-211`) is source-disjoint from this.
- **Gates:** GPU result ≤1e-12 vs the CPU f64 oracle + QuantLib goldens; NO test pins a GPU-FX
  result to `to_bits` (CPU flat-carry `carry.rs:525` stays the byte-identity oracle); risk-cube
  reprice output unchanged within ≤1e-12.
- **Disjointness:** celnet-gpu + celnet-risk-cube are lane-owned; the `PriceBatch` proto arm +
  server handler is a coordinator-sequenced `celnet-server`/`celnet-proto` touch.

## Lane 3 — FI serve + client parity (ADR-0014 clients)
- **Scope:** `celnet-server/src/rates_pricing.rs` (+ proto `RatesInstrument` arms), `celnet-client`
  (DeskClient/NotificationClient + FI-risk methods), `celnet-cli` (`risk rates-*`), `excel`
  (`=CELNET.SCENARIO`). Analytics ALREADY exist in `celnet-rates` (Fra/VanillaSwap/CashBond/z-spread).
- **Approach:** add the `RatesInstrument` oneof arms (Fra/Swap/Bond) + `price_rates` dispatch arms
  (wiring, the analytics exist); typed `DeskClient`/`NotificationClient` in `celnet-client/src/lib.rs`
  (same pattern as `Rfq`/`MultiDealerRfq`); FI-risk SDK methods + CLI commands; the Excel scenario fn.
- **Gates:** FI analytics vs their independent oracles ≤1e-12 (already gated in celnet-rates);
  client-parity workflow tests (SDK/CLI book+risk a swap/FRA/bond end-to-end); no wire change to
  existing arms.
- **Disjointness:** celnet-rates untouched; celnet-client/cli/excel lane-owned; proto+server arms
  coordinator-sequenced.

## Lane 4 — Generated wire codec (ADR-0014 codec)
- **Scope:** `celnet-proto/build.rs` (extend the existing G-INC1 manifest gen), `celnet-server/src/ws/codec.rs`.
- **Approach:** generate the field-level encode/decode from the proto descriptor + a curated
  OVERRIDE TABLE (the server-side field-name/projection decisions the descriptor can't infer);
  a DIFFERENTIAL harness asserts `generated_encode == hand_encode` over the client conformance
  corpus BEFORE swapping `handle_unary` → byte-identical wire, no GUI/Excel regression. Measure the
  override-table size against the corpus (ADR-0014 open item) — if the long tail is large, stage it.
- **Gates:** the differential harness green over the full conformance corpus (byte-identical wire);
  existing GUI/Excel e2e unchanged.
- **Disjointness:** celnet-proto + celnet-server/ws lane-owned; highest-risk lane (touches the wire)
  — gate hardest.

---

## Coordination notes
- Shared crate = `celnet-server` (all 4 lanes touch it): the coordinator sequences the server-side
  merges; keep each lane's server change minimal + in distinct modules (services vs rates_pricing vs
  ws/codec vs proto handlers) to minimise collision.
- `celnet-proto` touched by Lanes 2/3/4 (new arms) — coordinator owns the `celnet.proto` window.
- P2 (the `DiscountCurve` substrate + shared-enum breadth) is NOT in P1 — it is the deeper,
  coordinator-gated shared-interface-crate work (`celnet-types`/`celnet-core`/`celnet-risk-cube`).
- Encode each lane as a `coord/board` task (scope/deps/gate_tier/deliverable) on confirmation;
  the parallel sessions claim via `celnet-task`.
