---
name: g-ws-codec-swap-progress
description: "G full unary WS-codec swap (deferral 'ws-codec-from-proto') — increment tracker. INC1 DONE (655e292 on branch arch/G-ws-codec-full-swap, T1 green, unpushed). Remaining: override table → generated codecs → differential byte-identity harness → handle_unary swap. Carries the descriptor quirks the override table must handle."
metadata: 
  node_type: memory
  type: project
  originSessionId: f183df9e-4db0-4f5e-9594-dd336044ca0d
---

Deferral G (docs/INTERFACES.md "WS codec from proto descriptor (item G)" → Activation plan). Board task `G-full` on coord/board `5fc96b8`; deliverable `ws-codec-from-proto`. Single M4 ⇒ cargo serial; do one increment per loop iteration, each T1-scoped (celnet-proto + celnet-server), land the whole branch via ONE batched t2 (never per-increment). See [[ship-program-resume]].

## Increment plan
1. **DONE** (`655e292`, branch `arch/G-ws-codec-full-swap`, T1 6/6 green — proto/server build, proto-test 68, ws-test, fmt, clippy). `celnet-proto/build.rs` emits, from the same FileDescriptorSet, a per-message field table into `wire_contract`: `WireField{proto_name,json_key,proto_type,label:WireLabel{Singular|Optional|Repeated},oneof_group:Option<&str>}`, `MessageFields`, `MESSAGE_FIELDS` (221 entries, declaration order, index-aligned with `MESSAGES`), `fields_for(msg)`. Strictly additive — hand codec + handle_unary UNTOUCHED; `json_key==proto_name` for now. Test `field_table_tests` (3) asserts alignment/shape/real-oneof grouping.
2. **DONE** (`819d8ea`, T1 green — build, `ws_codec_differential` 12 passed, ws tests, fmt, clippy). Generated descriptor-driven encoder (`ws/generated_codec.rs`) + curated override table (`ws/codec_overrides.rs`) + differential harness (`tests/ws_codec_differential.rs`). Byte-identical `generated==hand` for the 4 FX-legacy cases (Underlying legacy pair; MarketContext r_dom/r_for; Greeks flat rho synthesized from RateSensitivities.FxRho; Tenor camelCase brokenDate) + representative simple/nested/repeated/oneof (CcyPair, Strategy, RateSensitivities). **Anti-cheat VERIFIED by me**: generated path never calls hand `*_to_json`; `codec::diff_support::hand_*` delegate to the REAL hand fns; harness is a true byte comparison. A `WireAdapter` reflection bridge reads raw struct fields; overrides = Rename / Suppress+synthesize.
3. **NEXT — full-surface coverage.** Extend `WireAdapter` impls + `WireVal` scalar kinds (bool, repeated scalars) + override entries to EVERY unary-edge message; generalize the message-level projection/synthesis to the RECURSION path (nested Underlying-in-Instrument etc.). Agent suggestion worth doing: auto-generate the `WireAdapter` layer from the descriptor in `build.rs` (removes the hand accessors).
4. **Verb-naming + full-corpus harness.** Add the per-service unary verb-naming map (INTERFACES.md divergence 1: PriceRates→price_rates vs ListConnections→list_fix_connections) and extend the differential harness across the FULL cross-client conformance corpus.
5. **Swap `handle_unary`** onto the generated path once the full-corpus harness is green. Activates the `ws-codec-from-proto` roll-up (auto-verify blocked on lodestar#19 → coordinator-attest).

Branch pushed to origin as backup (feature branch, not main); lands via ONE batched t2 with the other deferrals.

## Descriptor quirks the override table (inc 2+) MUST handle (from the inc-1 build agent)
- **Nested-message field types are dotted; message keys are simple.** `proto_type` is package-stripped but nesting-preserved (`Greeks.rate_sensitivities`→`"RateSensitivities"`; `RateSensitivities.fx`→`"RateSensitivities.FxRho"`; `Tenor.unit`→`"Tenor.Unit"` [enum]). `fields_for`/`MESSAGE_FIELDS` key on the **simple** name — to resolve a nested field's own table, strip the parent prefix (`"RateSensitivities.FxRho"`→`"FxRho"`). Codec generator should key on the FQ path if simple-name collisions ever appear (none today).
- **proto3 `optional` = synthetic single-arm oneof** → reported `label:Optional, oneof_group:None` via `field.proto3_optional()` (e.g. `Tenor.broken_date`, the four `Cliquet` floor/cap fields). Generator must NOT treat these as real oneof arms.
- **The 4 FX-legacy overrides (INTERFACES.md items 1–4), all visible in the tables:** (a) `Underlying` encodes to the legacy `{base,quote}` `pair` key, NOT the `ref` oneof arms; (b) `MarketContext` / `VanillaInputs.carry` — the `CarryModel.fx` arm → `r_dom`/`r_for` accessors (not the generalized `{discount_rate,carry}`); (c) `Greeks` emits flat `rho_dom`/`rho_for` beside the `rate_sensitivities` oneof — **note proto fields 7/8 are `reserved`, so they are NOT in the field table; the override must SYNTHESIZE them from the `RateSensitivities.FxRho` arm**; (d) camelCase `brokenDate` on the wire vs the snake_case `broken_date` the table carries.
- **Verb naming** (per-service snake_case vs request-message snake_case) is the separate `WIRE_RPCS` concern, not the field tables (field tables cover message *bodies*).
- No `map<>` / `group` / `required` fields exist in this contract (those descriptor paths are handled in code but unexercised — no fixtures).
