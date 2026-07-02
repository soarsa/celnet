# ADR-0014 — Generated wire contract & client parity

- **Status:** Proposed / Accepted as a **design direction** (2026-07-01). **NOT yet
  implemented.** Records the intended codec-generation + client-parity program; the five
  hand-written codec/contract projections remain authoritative until the differential
  byte-identity harness is green and `handle_unary` is swapped. Completes the **"G deferral"**
  (`ws-codec-from-proto`, INC1 landed `655e292`). Extends **ADR-0009** (edge wire codec — the
  "one generator, N projections of the one contract" principle) and honours **ADR-0007** (one
  unversioned contract) and CLAUDE.md guardrails #2 (no placeholders) / #8 (vendor-neutral
  naming) / #9 (no versioned APIs) / #10 (zero legacy) / #11 (API-first parity).
- **Aligns with:** target-architecture **D2** (`docs/ARCHITECTURE-TARGET.md` §D2, §3 rank 1–2,
  §5 P1(c)+(d)); `docs/INTERFACES.md` deferrals (`ws-codec-from-proto`). The contract SHAPE is
  already right — this ADR removes the **hand-maintained duplication** of that one shape and
  closes the ranked **client-parity gaps**, not new capability.

## Context (grounded in the code)

- **The contract is one clean, unversioned shape.** `proto/celnet.proto` (**4,484 lines**) is
  9 services / ~40 RPCs; `Instrument.oneof` = **25 product arms** (fields 7–32);
  `Underlying.oneof` = **5 asset classes** (FX / metal / equity / commodity / crypto). It is
  **no-versioning clean** — zero `schema_version`, per ADR-0007. The `Price` vs `PriceRates`
  and `AggregateRisk` vs `AggregateRatesRisk` splits are **domain-justified** (options need a
  vol **surface + `MarketContext`**; FI needs a **`CurveSet`**), **not** accidental duplication.

- **THE LARGEST DRY VIOLATION — the hand WS codec: ~9,293 lines tracking the 4,484-line
  proto.** Five hand-maintained projections of the one shape:
  - `celnet-server/src/ws/codec.rs` — **4,378 lines**, `serde_json::Value` (not typed).
  - `gui/src/data/wsCodec.ts` — **2,458 lines**.
  - `excel/src/contract/wsCodec.ts` — **1,237 lines** ("minimal duplicate of gui").
  - `instrumentCodec` (**577**) + `riskCodec` (**643**).
  - plus **dual `contract.ts` mirrors** (gui + excel `contract.ts`, **1,412 lines** — a
    Vite-boundary-forced duplication).
  **Drift cost:** a single proto field needs **5 manual edits**, and **none is
  compile-time-caught**. (4,378 + 2,458 + 1,237 + 577 + 643 = 9,293.)

- **The generator path already exists (G INC1).** `build.rs` already emits the
  `WIRE_RPCS` / `MESSAGES` / `ENUMS` / `STREAM_CONTROL_VERBS` **manifest** from the proto
  descriptor (landed `655e292`). What remains for G is: **field-level codec generation** + a
  **curated override table** + a **differential harness** (`generated_encode == hand_encode`
  over the conformance corpus) + the **`handle_unary` swap**.

- **Client-parity gaps (ranked in the audit).** (1) `RfqDeskService` — **GUI-only**: no typed
  SDK (raw `client.channel()`), no CLI. (2) **FI risk** (`AggregateRatesRisk` /
  `BookRatesPosition` / `ListRatesPositions`) — GUI-only, raw SDK, no CLI (**FI pricing IS in
  the SDK; FI risk is not**). (3) `NotificationService` — GUI-only. (4)
  `SurfaceService.Scenario` — no CLI/Excel. (5) `DrillRisk` — no Excel. (6) market series — no
  CLI.

- **Excel** is a compiled bundle, `=CELNET.*` namespace, `Office.onReady`-gated, **14 custom
  fns**, wire bit-identical to GUI/SDK; gaps: `=CELNET.SCENARIO` (spill shock grid — the
  natural Excel shape), `RATESRISK`, `BOOKRATES`, drill. **SDK** (`celnet-client`) has a clean
  builder, internal idempotency (`KeyMinter`), a typed vocab, workflow-driven tests
  (`surface_workflow.rs`); gaps: rates breadth (only OIS / `UsdSofrCurve`), **no typed
  `DeskClient` / `NotificationClient`**.

- **`gui/src/data/pricing.ts` (~2,500 lines)** is a **client-side closed-form pricing engine**
  (a mock data source + surface-display helpers) — NOT the live path, but an **independent
  numerical implementation** → **drift risk vs the server `pricer.rs`**.

- **Cross-asset is already universal.** `Instrument.oneof` + `PricingService.Price` is the one
  entry point for every asset class; `ws_routes_every_cross_asset_arm`
  (`cross_asset_ws.rs:323`) proves the router already carries every cross-asset arm. Non-vanilla
  arms on equity/crypto/commodity need only new `price_instrument` dispatch + `Underlying`
  acceptance — **zero proto/API shape change**.

**Key insight.** The contract shape is right and ADR-0007-clean; the debt is **five
hand-maintained projections of that one shape**. The fix is **generation from the single
source of truth** — exactly the "one generator, N projections" principle ADR-0009 established
for the `EdgeFrame`, now applied to the WS codec and the TypeScript clients.

## Decision

Eliminate the largest DRY violation by **generating** the wire projections from the proto
descriptor, and close the ranked client-parity gaps so **every capability lives in the one API
with uniform client parity**. Four layers:

1. **Generate the server WS codec from the descriptor (complete G).**
   - Extend `build.rs` (already emitting the `WIRE_RPCS`/`MESSAGES`/`ENUMS`/
     `STREAM_CONTROL_VERBS` manifest) to emit **field-level encode/decode** for
     `celnet-server/src/ws/codec.rs` from the proto descriptor.
   - A curated **OVERRIDE TABLE** supplies the server-side field-name / projection decisions
     the descriptor cannot infer (JSON key casing, computed/derived fields, deliberately
     omitted internal fields, the `serde_json::Value` shape choices). Descriptor-driven for the
     mechanical majority; the override table is the small, audited, curated remainder.
   - A **DIFFERENTIAL byte-identity harness** asserts `generated_encode == hand_encode` over
     the **entire client conformance corpus** **BEFORE** `handle_unary` is swapped to the
     generated path. Only once the harness is green is the swap made and the hand `codec.rs`
     body deleted (**−4,378 hand lines**). After the swap, **field drift is structurally
     impossible**: a new proto field is either generated or fails the manifest gate at compile
     time.

2. **`buf`-generate the TypeScript message types + a shared WS-codec package.**
   - Replace **both** hand-typed `contract.ts` mirrors (gui + excel) with `buf`-generated
     message types from the one proto.
   - Replace **both** `wsCodec.ts` (gui 2,458 + excel 1,237) + `instrumentCodec` + `riskCodec`
     with **one shared generated WS-codec package** both clients import — dissolving the
     Vite-boundary-forced duplication. **−~6.7k TS lines.**

3. **Close the client-parity gaps (API-first — every capability in the one API, uniform
   parity).**
   - Typed **`DeskClient`** (`RfqDeskService`) + typed **`NotificationClient`** in
     `celnet-client`, replacing the raw `client.channel()` access.
   - **FI risk** (`AggregateRatesRisk` / `BookRatesPosition` / `ListRatesPositions`) in the
     **SDK + CLI** (FI pricing is already in the SDK — bring FI risk to the same parity).
   - **`=CELNET.SCENARIO`** Excel custom function (the `SurfaceService.Scenario` spill shock
     grid — the natural Excel shape).

4. **Scope `gui/src/data/pricing.ts` strictly to mock/display.**
   - Fence it to a **mock data source + surface-display helpers** with an explicit "**not a
     pricer**" boundary. It must never be a live-path numerical authority; this bounds its
     drift risk vs the server `pricer.rs`. (Fenced, not deleted — it powers offline/demo GUI.)

### Keep (do NOT change)

- **The ONE unversioned contract (ADR-0007)** — already clean, zero `schema_version`.
  Generation is *from that one shape*; **no version axis is introduced** (no `schema_version`,
  no N/N-1 negotiation, no codec-cadence decoupling by versioning).
- **The domain-justified `Price` vs `PriceRates` and `AggregateRisk` vs `AggregateRatesRisk`
  splits** — options need a **surface + `MarketContext`**, FI needs a **`CurveSet`**; these are
  two RPCs for two domains, **not** accidental dup. Generation does **not** collapse them.

## Consequences

- **Structural drift-impossibility.** After the swap, a new proto field flows into the server
  codec and both TS clients from **one** edit; the 5-place hand update and the
  un-compile-checked path are gone (guardrail #10 zero-legacy).
- **Line elimination:** **−4,378 Rust** (server codec) **+ −~6.7k TS** (both `contract.ts`
  mirrors + both `wsCodec.ts` + `instrumentCodec` + `riskCodec`).
- **Invariant — byte-identical wire across codec generation.** The **differential harness**
  (`generated_encode == hand_encode` over the conformance corpus) is the **gate**;
  `handle_unary` is not swapped until it is green. The **single conformance corpus** (ADR-0007:
  one corpus, `to_bits` equality across all 5 clients) is the oracle — there is **one right
  answer, not a matrix**. **No GUI/Excel wire regression is admissible.**
- **Invariant — API-first parity.** Every capability lives in the one API; SDK / CLI / Excel /
  GUI consume it uniformly (guardrail #11). `DeskClient` / `NotificationClient` / FI-risk /
  `=CELNET.SCENARIO` close the ranked parity gaps.
- **Invariant — one unversioned contract preserved (ADR-0007).** Generation adds **no**
  `schema_version` and **no** negotiation ceremony.
- **Cross-asset: zero API/proto/codec shape change.** `Instrument.oneof` +
  `PricingService.Price` is already the universal entry point
  (`ws_routes_every_cross_asset_arm`, `cross_asset_ws.rs:323`). Non-vanilla arms on
  equity/crypto/commodity need only new `price_instrument` dispatch + `Underlying` acceptance —
  and because the codec is **generated**, they need **zero codec edits**. This ADR makes
  cross-asset breadth strictly a **server-side dispatch** concern.
- **Cost / risk — the override table.** It is the one curated, hand-maintained artifact; it is
  small, audited, and any wrong override **fails byte-identity** in the differential harness, so
  its correctness is machine-enforced, not asserted. `gui/src/data/pricing.ts` stays a
  display/mock impl (fenced, not deleted) so the offline/demo GUI keeps working while the drift
  risk vs `pricer.rs` is bounded by the explicit "not a pricer" fence.

## Supporting verified claims (lodestar knowledge layer)

To author graph-anchored, lifecycle **draft** (proposed direction; promotion to `active` awaits
the differential harness green **and** implementation), mirroring the ADR-0010 pattern:

- *(decision)* The five hand codecs are **projections of ONE proto**; generate them from the
  descriptor, with an **override table** for the server-side field/projection decisions the
  descriptor cannot infer. Anchors: `celnet-server/src/ws/codec.rs`, `build.rs` manifest
  (`WIRE_RPCS`/`MESSAGES`/`ENUMS`/`STREAM_CONTROL_VERBS`), `gui/src/data/wsCodec.ts`,
  `excel/src/contract/wsCodec.ts`, the `contract.ts` mirrors.
- *(invariant)* **Byte-identical wire across codec generation** — `generated_encode ==
  hand_encode` over the conformance corpus **gates** the `handle_unary` swap. Anchors:
  `handle_unary`, `ws/codec.rs`, the conformance corpus, `to_bits`.
- *(invariant)* **API-first parity** — every capability in the one API, uniform client parity.
  Anchors: `RfqDeskService`/`DeskClient`, `NotificationService`/`NotificationClient`,
  `AggregateRatesRisk`/`BookRatesPosition`/`ListRatesPositions`, `=CELNET.SCENARIO`.
- *(decision)* **Cross-asset needs zero proto/codec shape change** — `Instrument.oneof` +
  `PricingService.Price` already universal; the generated codec makes new arms codec-free.
  Anchors: `ws_routes_every_cross_asset_arm` (`cross_asset_ws.rs:323`), the `Instrument` oneof,
  `price_instrument`.
- *(decision)* **`gui/src/data/pricing.ts` is display/mock only, not a second pricer** — fence
  it to bound drift vs the server pricer. Anchors: `gui/src/data/pricing.ts`,
  `celnet-server` `pricer.rs`.

## Alternatives rejected

- **Keep hand-maintaining the five parallel codecs.** Rejected: 5 manual edits per proto field,
  **none compile-checked** — the drift is a standing defect reservoir (guardrail #10; the
  "deferred-e2e = defect reservoir" lesson). The manifest (G INC1) already proves the generator
  path; leaving G unfinished keeps the **largest DRY violation** standing.
- **A second, versioned contract** (a `schema_version` to decouple the client/server codec
  cadence). Rejected: violates **ADR-0007** / guardrail #9 — one deployment, no mixed-version
  window; a version axis adds dead branches to every codec path that no real peer exercises. The
  decoupling mechanism is **generation from the one shape**, not versioning.
- **Collapse `Price`/`PriceRates` (or the two risk RPCs) into one generic RPC "for symmetry".**
  Rejected: the split is **domain-justified** (surface + `MarketContext` vs `CurveSet`);
  collapsing forces a union payload and loses type-safety — a domain conflation, not a DRY win.
- **Hand-write the TS types "to keep control of ergonomics".** Rejected: `buf`-generate + a thin
  ergonomic wrapper keeps ergonomics **and** kills the dual `contract.ts` mirror; the Vite
  boundary is solved by a **shared package**, not by duplication.
