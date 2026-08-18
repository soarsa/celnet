# Post-W2 proto-window execution checklist

> **Purpose.** A precise, ordered, **plain-cargo-gated** runbook so the coordinator's
> batched leaf-integration lands FAST the instant session-B opens the proto window. Reserves
> and wires the five landed leaves in ONE coordinated arc:
> `celnet-{equity,commodity,crypto}-vanilla`, `celnet-rfq`, `celnet-linear`.
> Pairs with [`POST-W2-INTEGRATION-MANIFEST.md`](../POST-W2-INTEGRATION-MANIFEST.md) (the reservation)
> — this file is the *execution order*.
>
> **House rules honored.** One unversioned contract — every item is **additive** (no
> `schema_version`, no renumber, FX byte-identical, default-value-omitted ⇒ wire-identical).
> Vendor-/person-neutral identifiers only (SBE/Aeron-style framing may be cited in prose,
> NEVER in our crate/type/fn names). OSS-only. This is a **PLAN** — no code below.
>
> **Gate policy (task-mandated).** Each step ends with a **plain-cargo** gate — `cargo build`
> / `cargo test` with `-p <crate>` scoping — **NOT** `cargo nextest`. Use
> `source "$HOME/.cargo/env" && cargo …` (env does not persist between shells). The full
> `just check` (nextest, deny, clippy -D) is the **milestone gate run ONCE after Step 6**,
> before the integration commit(s).

---

## Pre-flight (do before touching the proto)

- [ ] **P0.** Confirm session-B has merged W2 and the proto window is OPEN (W2 owns it).
      Re-confirm the actual free tags at edit time against `crates/celnet-proto/proto/celnet.proto`:
  - `Underlying.ref` oneof: `fx=1`, `metal=3` are taken ⇒ **free = 4, 5, 6** (line ~357).
  - `Instrument` top-level fields: `pricing_model=22`, product oneof occupies `7..28`
    (`ndf=28`, the newest W2 arm) ⇒ **next free top-level = 29** (line ~1140).
  - `QuoteService` (line ~2663) has `RequestQuote`/`AcceptQuote`/`RejectQuote` ⇒ add one RPC.
- [ ] **P1.** Use the `protobuf` skill for enum-prefix / field-number / lint hygiene on the
      edit (the repo compiles `.proto` with the pure-Rust `protox` in `build.rs`, no system
      `protoc` — so a syntactically clean schema is mandatory or the whole workspace fails to build).
- [ ] **P2.** Note the carry seam is **already present**: `celnet_types::Carry::CostOfCarry { r, b }`,
      wire `CostOfCarry { b }` + `CarryModel.model.generalized=2`, and `RateSensitivities::Carry`
      all exist (W1). The leaves therefore need **no new carry message** — only the new
      `Underlying.ref` arms + the crypto `settlement_style` + the RFQ RPC/messages.

---

## Step 1 — Proto arms (the one batched edit) — `crates/celnet-proto/proto/celnet.proto`

> Single file, single commit-window. All additive. **Blocks every later step** (Rust codegen
> + TS gen derive from it). Order the sub-edits as listed so the file stays diff-clean.

### 1a — Multi-asset `Underlying.ref` arms (W5-B equity + commodity, W3 crypto)

- [ ] Add helper messages above/near `message Underlying` (line ~355), mirroring the manifest:
  - [ ] `message Symbol { string code = 1; }` — a length-validated instrument code (NOT a
        3-char `Ccy`); vendor-neutral name (do **not** call it `Ticker`/a venue name).
  - [ ] `message EquityRef { Symbol symbol = 1; string settlement_ccy = 2; }`
  - [ ] `message CommodityRef { Symbol symbol = 1; string settlement_ccy = 2; }`
  - [ ] `message CryptoPair { string base_coin = 1; string quote = 2; }`
- [ ] Extend the `oneof ref` inside `message Underlying` (keep `fx=1`, `metal=3` untouched):
  - [ ] `EquityRef equity = 4;`
  - [ ] `CommodityRef commodity = 5;`
  - [ ] `CryptoPair digital_asset = 6;`
- [ ] Doc-comment each arm with the owning leaf crate + that the option **reuses the existing
      `vanilla` product arm** carried by `CarryModel.generalized` (`b = r−q` equity, `r−convenience`
      commodity, `r−funding` crypto). **No new product oneof arm** for the four leaves.

### 1b — Crypto settlement style (W3 inverse / coin-margined `1/S_T`)

- [ ] Add `enum SettlementStyle { SETTLEMENT_STYLE_LINEAR = 0; SETTLEMENT_STYLE_INVERSE_COIN = 1; }`
      (zero-value default = `LINEAR` ⇒ FX/equity/commodity presence-omit it ⇒ byte-identical).
      The `SETTLEMENT_STYLE_` enum prefix is mandatory (proto enum-hygiene; the `protobuf` skill checks it).
- [ ] Add to `message Instrument` (after the product oneof, top-level field): `SettlementStyle settlement_style = 29;`
      with a doc note that default `LINEAR` reproduces the pre-field contract bit-for-bit.

### 1c — Multi-dealer RFQ (W4-B `celnet-rfq`)

- [ ] Add (near `Quote`/`TwoWayPrice`, lines ~651/1358):
  - [ ] `message DealerQuote { string lp_id = 1; TwoWayPrice two_way_price = 2; int64 epoch_nanos = 3; int64 valid_until_nanos = 4; bool responded = 5; }`
  - [ ] `message MultiDealerQuote { repeated DealerQuote panel = 1; string best_bid_lp = 2; string best_offer_lp = 3; uint32 lp_count = 4; string lp_won_bid = 5; string lp_won_offer = 6; }`
- [ ] Add RPC to `service QuoteService` (line ~2663): `rpc RequestMultiDealerQuote(QuoteRequest) returns (MultiDealerQuote);`
- [ ] Extend `message QuoteAccept` (line ~1395) with an **additive** `string lp_id = 4;`
      (default empty ⇒ single-dealer accept stays byte-identical) to book a named panel winner.

**▶ Gate (Step 1)** — proves the schema compiles via `protox`/`tonic-build` and all generated
Rust types are present, before any hand-written codec depends on them:

```bash
source "$HOME/.cargo/env" && cargo build -p celnet-proto
```

---

## Step 2 — `celnet-types` variants + `convert.rs` codecs

> **Depends on Step 1** (the wire types must exist). Two files: domain enum, then codecs.

### 2a — `crates/celnet-types/src/lib.rs` — `Underlying` variants

- [ ] Extend `pub enum Underlying` (line ~279) with the three new domain arms, alongside
      `Fx(CcyPair)` / `Metal(MetalPair)`:
  - [ ] `Equity(EquitySymbol)` — add a vendor-neutral `pub struct EquitySymbol { code, settlement_ccy }`
        (length-validated code; do not name it after any data vendor).
  - [ ] `Commodity(CommoditySymbol)` — analogous struct.
  - [ ] `DigitalAsset(CryptoPair)` — add `pub struct CryptoPair { base_coin, quote }`.
- [ ] Update the `Underlying` inherent methods for the new arms (return `None`/the right
      projection — do **not** silently coerce):
  - [ ] `as_fx` / `as_metal` ⇒ `None` for the new arms.
  - [ ] `as_ccy_pair` (line ~310): the new arms do **not** project onto a `CcyPair` — either
        return `Option<CcyPair>` (preferred, propagate) or keep a typed "no FX projection" path.
        **Do not fabricate a CcyPair** for equity/commodity/crypto (would resurrect the W1
        silent-FX-fallback bug). Capture the decision in a one-line ADR via `manage_adr` if the
        signature changes (callers in conventions/calendar registries depend on it).
  - [ ] `Display` (line ~330) + `From<…>` conveniences for the new arms.

### 2b — `crates/celnet-proto/src/convert.rs` — Underlying codecs (additive)

- [ ] `impl From<Underlying> for WireUnderlying` (line ~515): add the three `match` arms
      → `WireUnderlying { ref: Some(underlying::Ref::Equity/Commodity/DigitalAsset(...)) , settlement_ccy }`.
- [ ] `impl TryFrom<WireUnderlying> for Underlying` (line ~524): add the three decode arms;
      keep the `None ⇒ WireError::MissingField{ "Underlying.ref" }` (no silent default).
- [ ] Add `From`/`TryFrom` for the helper messages: `EquitySymbol↔EquityRef`,
      `CommoditySymbol↔CommodityRef`, `CryptoPair↔CryptoPair(wire)`, `EquitySymbol↔Symbol` — length-validate
      `Symbol.code` / `CryptoPair.base_coin`/`quote` (reject malformed as `WireError::InvalidCcy`-style typed error).
- [ ] **Carry is already done** — `Carry::CostOfCarry ↔ CarryModel.generalized` (line ~546) and
      `RateSensitivities::Carry` (line ~561) round-trip already. No edit unless a new
      asset-class needs a *distinct* sensitivity shape (it does not — all three reuse `CarryRho`).
- [ ] Add round-trip unit tests in the `convert.rs` test module (mirroring
      `underlying_round_trips_fx`/`_metal` at lines ~860/949): one per new arm + a
      `settlement_style` default-omitted byte-identity check.

**▶ Gate (Step 2)** — compiles + unit-tests the codecs/enums in isolation:

```bash
source "$HOME/.cargo/env" && \
  cargo test -p celnet-types && \
  cargo test -p celnet-proto
```

---

## Step 3 — Server price routing per `Underlying` + the no-silent-fallback carry guard

> **Depends on Step 2.** This is the verifier-flagged blocker class: the **current** guard in
> `price_instrument` *rejects every non-FX carry* (line ~456: "market carry must be the FX
> two-rate arm"). It must instead **route by `Underlying` to the matching leaf BEFORE** the
> FX-only carry assertion, and keep the typed-error (never silent-FX-`r_for=0`) contract.

### 3a — `crates/celnet-server/src/pricer.rs` — route head

- [ ] In `price_instrument` (line ~442), decode `instrument.underlying` to the domain
      `Underlying` (via `validate_fx_underlying` / a new `validate_underlying`) **first**, then
      branch:
  - [ ] `Underlying::Fx | Metal` → the existing FX/GK path **unchanged** (the current
        carry guard at line ~456 applies **only on this branch** ⇒ FX byte-identical).
  - [ ] `Underlying::Equity` → delegate to `celnet_equity_vanilla::{price, greeks}` over
        `EquityInputs` (generalized-BSM, `CarryModel.generalized` `b = r−q`).
  - [ ] `Underlying::Commodity` → delegate to `celnet_commodity_vanilla::{price, greeks}`
        (Black-76 / `b = r−convenience`).
  - [ ] `Underlying::DigitalAsset` → delegate to `celnet_crypto_vanilla::route_price` keyed on
        the new `Instrument.settlement_style` (`LINEAR` → `linear`, `INVERSE_COIN` → `inverse`
        `1/S_T`); funding carry via `funding_carry(r, funding)`.
- [ ] **No-silent-fallback carry guard, generalized:** for each non-FX branch, assert the
      supplied `CarryModel` is the **`generalized`** arm (and reject an FX `fx` arm on an
      equity/commodity/crypto underlying with a typed `PriceError::Domain`), symmetric to the
      existing FX-arm assertion. An equity underlying carrying an FX two-rate carry must be a
      **typed error**, never read as `b=0`. Mirror the wording style at line ~451.
- [ ] Add `celnet-{equity,commodity,crypto}-vanilla` to `crates/celnet-server/Cargo.toml`
      `[dependencies]` (already-registered workspace members ⇒ no root `Cargo.toml` edit).
- [ ] Generalize the linear product guards already present: `validate_deliverable_underlying`
      / `validate_non_deliverable_underlying` (convert.rs, lines ~476/497) reject the new arms
      as they should (an NDF/forward is FX/metal only) — add explicit reject arms + a test so a
      new arm can't silently fall through.

### 3b — RFQ RPC wiring — `crates/celnet-server/src/services/quote.rs`

- [ ] Implement `request_multi_dealer_quote` on `impl QuoteService for QuoteEdge` (line ~272),
      delegating to `celnet_rfq::MultiDealerEngine` with the `InternalPricerSource` bound to the
      live pricer (concurrent fan-out → `RankedPanel` → `MultiDealerQuote` wire via
      `panel.to_wire()` / `DealerQuote`).
- [ ] Extend `accept_quote` (line ~407) to honor the new `QuoteAccept.lp_id` (book the audited
      `(quote_id, lp_id)` winner; empty `lp_id` ⇒ existing single-dealer path unchanged).
- [ ] Add `celnet-rfq` to `crates/celnet-server/Cargo.toml` `[dependencies]`.

**▶ Gate (Step 3)** — compiles + tests the server with the routes/guard wired:

```bash
source "$HOME/.cargo/env" && cargo test -p celnet-server
```

---

## Step 4 — `celnet-golden` vectors + `celnet-parity` rows per leaf

> **Depends on Step 3** (so the production path the parity rows reprice exists). Each new
> priceable family/asset-class needs BOTH a cross-client golden vector AND an
> independent-oracle parity row — the `tools/check-verification-coverage.mjs` lint enforces it.

### 4a — Golden vectors — `crates/celnet-golden/`

- [ ] These leaves price under the **existing `vanilla` family** with a non-FX `underlying`
      tag, so extend the corpus along the **underlying/asset-class axis**, not the family axis.
      Decide one of (capture the choice in the module docs):
  - [ ] add per-asset vector files `vectors/{equity_vanilla,commodity_vanilla,crypto_vanilla}.json`
        + the W2 linear set already present in the working tree
        (`vectors/{fx_forward,fx_swap,ndf}.json`); **and/or**
  - [ ] extend `vanilla.json` with non-`EURUSD` `underlying`-tagged rows.
- [ ] Add the corresponding `gen_*` writers in `crates/celnet-golden/src/bin/gen_vectors.rs`
      (mirror `gen_vanilla`, line ~102), each `expected.price` from an **INDEPENDENT** oracle —
      external QuantLib generalized-BSM / Black-76 for equity/commodity, an independent
      measure-re-derivation for crypto inverse `1/S_T` (NEVER the production pricer's own output;
      NEVER a circular oracle — the W1 FRTB-0.75ρ lesson).
- [ ] If a NEW vector family key is introduced (e.g. `fx_forward`/`fx_swap`/`ndf` from W2-linear,
      already on disk in the working tree), add it to the `FAMILIES`/`MC_FAMILIES` consts in
      `crates/celnet-golden/src/vectors.rs` (line ~47) AND to the coverage-lint family list so
      the corpus stays "all families covered".
- [ ] Keep the cross-client JSON shape stable (it's the shared contract the 5 clients re-read).

### 4b — Parity rows — `crates/celnet-parity/tests/`

- [ ] Add one independent-oracle `#[test]` row per leaf (new files, e.g. `equity.rs`,
      `commodity.rs`, `crypto_inverse.rs`, `linear.rs`, `multidealer_rfq.rs`), each repricing a
      reference input and checking against the **independent** oracle to the leaf's tolerance
      (1e-12 for closed-form, `k·stderr` for any MC).
- [ ] Append the row descriptions to the parity registry doc-table in
      `crates/celnet-parity/src/lib.rs` (line ~5) so the row catalogue stays complete.
- [ ] RFQ parity: a deterministic `MultiDealerEngine` row over ≥3 synthetic LP responders with
      injected ground-truth ladders — assert best-bid/offer ranking + deterministic tie-break.

**▶ Gate (Step 4)** — runs the golden self-check + the new parity rows + the coverage lint:

```bash
source "$HOME/.cargo/env" && \
  cargo test -p celnet-golden && \
  cargo test -p celnet-parity && \
  node tools/check-verification-coverage.mjs
```

---

## Step 5 — 5-client surfacing (FX-default preserved)

> **Depends on Steps 1–3.** Surface the new underlyings + crypto settlement + RFQ panel
> through **every** client behind the one contract; **FX stays the default** everywhere (an
> existing FX-only call site is untouched and byte-identical).

- [ ] **SDK (`celnet-client`)** — `crates/celnet-client/src/lib.rs` + `vocab.rs`: add
      vendor-neutral constructors for `Underlying::{Equity,Commodity,DigitalAsset}` and the
      crypto `settlement_style`; add a `request_multi_dealer_quote` handle returning a ranked
      `MultiDealerQuote`. Keep `request_quote`/the FX builders as-is (FX default).
- [ ] **CLI (`celnet-cli`)** — `src/args.rs` + `src/price.rs` (+ `src/cli.rs`): add
      `--equity`/`--commodity`/`--crypto` underlying flags (+ `--inverse` settlement) and an
      RFQ-panel subcommand; absent flag ⇒ FX pair as today.
- [ ] **Excel add-in (`excel`)** — `src/contract/contract.ts`, `enums.ts`, `wsCodec.ts`,
      `functions/functions.ts`: add the new `Underlying`/`SettlementStyle` shapes +
      `MultiDealerQuote` decode; default FX path unchanged.
- [ ] **GUI** — if a ticket/blotter surfaces underlying selection, extend its product spec /
      grid to the new asset classes + the ranked RFQ panel; FX remains the default scope.
- [ ] **Examples / smoke** — add an example per leaf (mirror the working-tree
      `celnet-client/examples/price_linear.rs`) and extend `tests/examples_smoke.rs` /
      `tests/conformance.rs` / `tests/rfq_workflow.rs` so each new surface has a smoke assert
      that the client result equals the server's.

**▶ Gate (Step 5)** — per-client, plain-cargo for Rust + the Excel/GUI suites via their runners:

```bash
source "$HOME/.cargo/env" && \
  cargo test -p celnet-client && \
  cargo test -p celnet-cli
# Excel / GUI (their own runners, not cargo):
cd excel && npm test        # vitest: linearProducts + new underlying suites
cd gui   && npm test        # vitest: ticket/grid round-trips (if surfaced)
```

---

## Step 6 — Milestone integration gate (run ONCE, then commit)

> **Depends on Steps 1–5 all green.** Only here do we pay the full cross-crate cost.

- [ ] Run the full sanctioned gate and verify the **literal** "All gates passed." line
      (the W1 lesson — do not infer success from exit code alone): `just check`
      (fmt + clippy `-D warnings` + nextest workspace + cargo-deny). This is the **only**
      nextest invocation in the arc; Steps 1–5 used plain cargo per the task's gate policy.
- [ ] Re-confirm `cargo-deny` is clean (OSS-license set; no new non-permissive dep pulled in
      by the leaves — they reuse already-registered deps).
- [ ] lodestar re-indexes automatically; run `detect_changes` to confirm the new arms/routes
      are covered, and update `manage_adr` if Step 2a changed an `Underlying` signature.
- [ ] Commit per lane as **separate green commits**, disjoint files, FX-byte-identity noted in
      each message (the proto edit is one commit; each leaf wiring its own).
- [ ] Update `docs/IMPLEMENTATION-LEDGER.md` (newest-first) + the CLAUDE.md resume anchor +
      memory; mark the manifest's reserved arms as **landed**.

---

## Dependency order (one glance)

```
Step 1 (proto)  ─► Step 2 (types + codecs)  ─► Step 3 (server routing + carry guard)
                                                   │
                                                   ├─► Step 4 (golden vectors + parity rows)
                                                   └─► Step 5 (5-client surfacing)
                                                              │
                                                              └─► Step 6 (just check, commit)
```

- Step 1 blocks all (Rust + TS codegen derive from the schema).
- Steps 4 and 5 are independent of each other and may proceed in parallel once Step 3 is green.
- Each step's plain-cargo gate is `-p`-scoped so only the touched crate(s) recompile/retest;
  the full nextest gate runs **once** at Step 6.

## Honest boundary (verbatim, all lanes)

Live LP-panel WAN connectivity / regulated-venue status, and the live
crypto/metal/equity fixing + lease/funding **values**, are **ENV** — designed and seamed
in-repo, validated at deploy, never claimed in-repo. The in-repo gates prove the codecs,
routing, guards, oracles, and client surfacing; they do not assert live market connectivity.

