# Post-W2 integration manifest — one proto window for all landed lanes

> **Purpose (mesh optimization).** Five new pricing/aggregation crates are on `main` or in flight
> (`celnet-{equity,commodity,crypto}-vanilla`, `celnet-rfq`, plus the W2 `celnet-linear`). Each has a
> *deferred wire/client integration* that needs additive `celnet.proto` arms. Opening the proto window
> once per lane = N serialized stalls. **This manifest reserves ALL of them in ONE coordinated edit**, so
> whoever holds the proto window (currently **session-B**, integrating W2) lands them together — turning
> N proto serializations into 1. Every item is **additive** (no `schema_version`, no renumber), FX
> byte-identical. The coordinator confirms next-free numbers against `celnet.proto` at edit time (use the
> `protobuf` skill for enum-prefix/field hygiene).

## Critical path & non-blocking map (2026-06-08)

| Lane | Engine on main? | Blocks on | Non-blocking? |
|---|---|---|---|
| **W2** (linear + conventions + server + clients) | yes (`513c47c`/`02ce7a5`) | — (it OWNS the proto window) | it is the critical path |
| **W5-B** equity/commodity leaves | yes (`48a61bc`/`2c61193`) | proto Underlying arms (below) | engine landed independently |
| **W3** crypto leaf | yes (`32250bc`) | proto Underlying + settlement arm | engine landed independently |
| **W4-B** `celnet-rfq` | in verify | proto `QuoteService` RPC | engine lands as a new crate (no proto) |
| **W6** rigor-infra, **W5-A** x-risk, **W4-A** pivot | open | none / coordinator proto | run NOW, fully disjoint |

**Rule that kept us collision-free:** every leaf/RFQ crate auto-joins `members=["crates/*"]` and uses only
already-registered deps, so it needs **no root `Cargo.toml` edit and no proto** to land its engine. Only
the *wire surfacing* serializes — and this manifest batches that.

## Reserved additive proto (confirm next-free at edit time)

Current free tags: `Underlying.ref` → **4,5,6** (`fx=1`, `metal=3` taken); top-level `Instrument` field →
**29+** (product oneof occupies 7..28); `QuoteService` gains one RPC.

```proto
// --- multi-asset underlyings (W5-B + W3) — additive Underlying.ref arms ---
message Symbol      { string code = 1; }                 // length-validated, not a 3-char Ccy
message EquityRef   { Symbol symbol = 1; string settlement_ccy = 2; }   // cash-settled in index ccy
message CommodityRef{ Symbol symbol = 1; string settlement_ccy = 2; }   // listed future / spot+convenience
message CryptoPair  { string base_coin = 1; string quote = 2; }         // e.g. BTC/USD (Deribit)

oneof ref {            // in message Underlying
  // … fx = 1; metal = 3 …
  EquityRef    equity        = 4;   // W5-B celnet-equity-vanilla
  CommodityRef commodity     = 5;   // W5-B celnet-commodity-vanilla
  CryptoPair   digital_asset = 6;   // W3  celnet-crypto-vanilla
}

// --- crypto settlement style (W3) — the inverse/coin-margined 1/S_T payoff ---
enum SettlementStyle { SETTLEMENT_STYLE_LINEAR = 0; SETTLEMENT_STYLE_INVERSE_COIN = 1; }
// in message Instrument (default LINEAR ⇒ FX/equity/commodity byte-identical, presence-omitted):
SettlementStyle settlement_style = 29;

// NOTE: equity/commodity/crypto options REUSE the existing `vanilla` product arm — the new
// Underlying arm + the W1 CarryModel::CostOfCarry (b = r−q / r−funding / r−convenience) carry the
// asset class. NO new product oneof arm is needed for the four leaves (only crypto adds settlement_style).

// --- multi-dealer RFQ (W4-B celnet-rfq) — additive QuoteService RPC + messages ---
message DealerQuote {
  string lp_id = 1; TwoWayPrice two_way_price = 2;
  int64 epoch_nanos = 3; int64 valid_until_nanos = 4; bool responded = 5;
}
message MultiDealerQuote {
  repeated DealerQuote panel = 1;
  string best_bid_lp = 2; string best_offer_lp = 3;
  uint32 lp_count = 4; string lp_won_bid = 5; string lp_won_offer = 6;
}
// in service QuoteService:
rpc RequestMultiDealerQuote(QuoteRequest) returns (MultiDealerQuote);
// extend QuoteAccept to book a panel winner by (quote_id, lp_id) — additive field, default empty lp_id
// ⇒ single-dealer accept byte-identical.
```

## After the window opens (coordinator integration, batched)

Per lane, once the arms above are on `main`, the coordinator (or the lane owner) wires — each its own
green commit, disjoint files, no further proto contention:

- **W5-B / W3 leaves:** `celnet-types::Underlying::{Equity,Commodity,DigitalAsset}` + `convert.rs` codecs;
  `celnet-risk-normalize` asset-class-leaf selection (the engines already exist) → server price routing →
  golden vectors + `celnet-parity` rows → 5-client surfacing (FX-default preserved). W5-A consumes these.
- **W4-B RFQ:** `celnet-rfq::MultiDealerEngine` behind `QuoteService.RequestMultiDealerQuote` + the
  `InternalPricerSource` wired to the live pricer; SDK/CLI/GUI render the same ranked panel; `AcceptQuote`
  books the audited `(quote_id, lp_id)` winner.

**Honest boundary (verbatim, all lanes):** live LP-panel WAN connectivity / regulated-venue status, and
the live crypto/metal/equity fixing + lease/funding VALUES, are **ENV** — designed + seamed in-repo,
validated at deploy, never claimed in-repo.

## Parallelism plan (leverage agents without blocking)

- **session-B:** land W2 + this batched proto window (critical path) — frees ALL downstream wiring at once.
- **coordinator (me), concurrent disjoint lanes (no proto, no shared files):** W4-B-RFQ (finishing) →
  W6-RIGOR-INFRA → W5-A-XRISK, each a board-claimed isolated-worktree dynamic workflow, FF-merged on
  completion. None touches the proto window, so they never block W2 and W2 never blocks them.
