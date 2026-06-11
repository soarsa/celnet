# Fast-follow — crypto strike-axis surface wiring (post-1.0-RC)

**Status:** TRACKED FAST-FOLLOW (operator-directed 2026-06-11; 1.0-RC `a0817d6` released
with this as a documented point-release item, NOT a blocker). The strike-axis surface
**leaf** is built + parity-gated on `main` (`c5a5efc`); this is its unbuilt **surfacing** half.

## Why fast-follow, not blocking
Crypto vanilla **pricing** already works (FX-style surface). What's unwired is the
strike-axis quote **ingestion** path to clients — a capability gap, not a defect. The RC is
gated (T2 16/16) + Round-5-dry.

## Scope (one cross-asset surfacing lane, needs the proto window + agent capacity)
1. **proto**: a `quote_basis` discriminator + `StrikeQuoteSet` message (delta-pillar vs
   strike-axis) on the surface-calibration request — coordinator/proto-window owned.
2. **server**: route a strike-axis quote set to `fit_strike_slice` / `strike_surface`
   (the merged leaf), FX delta path byte-identical.
3. **clients**: SDK + CLI + GUI + Excel reachability for strike-quoted crypto surfaces;
   cross-client conformance; live e2e + axe (gui-touching → live Playwright, per
   [[deferred-e2e-defect-reservoir]]).
4. **gate**: one milestone gate per §4.2/§4.3; adversarial verify the wiring.

## Sibling fast-follow / post-RC backlog (also spend-paused, banked on origin)
- W6 exotics **E2** mutation waves + perpetual/future_option coverage (`lane/w6-exotics`).
- Fuzz lane `fix_decoder` dedup vs RC's `fix_frame_decode`, then CI wiring (closes the
  RC's filed P2 "fix-frame-fuzz-not-in-ci") (`lane/w6-fuzz`).
