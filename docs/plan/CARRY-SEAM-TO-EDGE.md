# Scope — carry seam to the streamed edge (the #1 architecture target)

From `docs/ARCHITECTURE-DETERMINATION.md`: the carry seam (ADR-0008) makes *"add an asset
class = a new `Carry` builder, zero payoff-engine edits"* true for **batch/request** pricing,
but it **stops at the request/leaf layer** — the streamed hot edge is still FX-only. This is
the single highest-leverage move to finish the multi-asset architecture.

## The gap (grounded in the graph)
- **Stream seed is FX-only:** `celnet-server::services::pricefanout::pair_seed(&CcyPair)` seeds
  the price-fanout ring from an FX pair, not from an `Underlying`/`Carry`.
- **The wire `Update` ships FX two-rho `Greeks`**, not the carry-tagged rate sensitivities —
  even though the contract **already defines** them: `celnet-proto::proto::celnet::RateSensitivities`
  and `celnet-types::RateSensitivities` (FxRates `{rho_dom,rho_for}` | Carry `{discount_rho,carry_rho}`),
  with `rate_sensitivities_arms` + the `rate_sensitivities_round_trips_both_arms` round-trip test.
  The machinery exists; the streaming path never routes it.
- **Market context is FX-only** on the streamed path (`MarketContext.fx(...)`, `observe_live`),
  so a streamed non-FX underlying cannot be seeded or observed.

## Leverage
The type + wire + conversion for `RateSensitivities` are **built and tested**. The exotics/PDE
path already emits `RateSensitivities::Carry` (ADR0008 remediation §3.4, `american.rs:255–301`).
So this is **threading existing pieces through the streaming path**, not new contract design.

## Target
The same `forward(t)`/`discount_df(t)` abstraction that holds for batch must hold for **streaming,
registries, and sensitivities**: seed the fanout ring from `Underlying`/`Carry`, price the streamed
update through the agnostic seam, and emit `RateSensitivities` on the wire `Update` — with the FX
path **byte-identical** to today (the no-regression gate).

## Change set (the symbols)
1. **Stream seed** — generalize `pair_seed(&CcyPair)` → `underlying_seed(&Underlying)` (FX is the
   degenerate arm); `services::pricefanout` + the `MarketContext` it builds become carry-aware.
2. **Streamed sensitivities** — populate the existing `RateSensitivities` oneof on the wire `Update`
   from the priced `Greeks` (the FX arm = `{rho_dom,rho_for}`, exactly the current two-rho values).
3. **Observation** — `core_link::observe_live` / the series path resolve observables for non-FX
   underlyings via the seam, not `MarketContext.fx`.
4. **Clients** — GUI/Excel/SDK render `RateSensitivities` (already decodable; the GUI `Greeks`
   strip extends to the carry arm). API-first parity: one contract change, all clients in lockstep.

## No-regression gate (binding)
Extend the existing FX byte-identity invariants (`fx_carry_inputs_byte_identical`,
`forward_and_df_match_core_carry_inputs_byte_for_byte`) to the **streaming** path: a streamed FX
`Update` must be `to_bits`-identical before/after, and the cross-asset conformance corpus must pass
across all 5 clients. No `match carry {…}` in the streamed pricing path (the ADR-0008 review-blocker).

## Phasing
- **P1** carry-aware stream seed + `MarketContext` (FX byte-identical; no wire change yet).
- **P2** emit `RateSensitivities` on the streamed `Update` (FX arm = current two-rho).
- **P3** clients render the carry arm; stream a non-FX underlying end-to-end (the proof the seam reaches the edge).

Each phase T1-gated per crate; T2 + both live e2e at the P3 landing.
