/**
 * Client-price preview for the session-pivoted FI Tiering surface.
 *
 * The Tiering workspace summarises a resolved pricing group's TIERING feature
 * (strategy + params + guardrails) but not the PRICE a client ends up seeing.
 * {@link clientTwoWayFromTiering} closes that gap: from a representative sample raw
 * two-way it computes the OUTBOUND client bid/offer a config produces, so the
 * surface can render a worked example — `SAMPLE LP 99.50 / 99.60 → client 99.30 /
 * 99.80` (the reference example, Flat 25 price-bps around mid 99.55).
 *
 * This mirrors the server `celnet-tiering` arithmetic closely enough to make the
 * page legible; it is NOT the authoritative price (that is computed server-side per
 * live tick). It reuses {@link spreadUnitToPoints} so magnitudes convert to price
 * points exactly as the in-editor {@link outboundTwoWayPreview} does — the two share
 * one unit-conversion source of truth.
 *
 * INVENTORY_SKEW is deliberately shown at ZERO net position: a position-linear lean
 * cannot be illustrated from a static sample without inventing a position, so the
 * readout shows the symmetric BASE half-spread (which, at zero position, is exactly
 * what INVENTORY_SKEW reduces to) and the caller flags the omitted lean as
 * position-dependent via {@link hasPositionDependentSkew}. SCALED_SMOOTHED_SPREAD is
 * shown directly — its output spread depends on the observed raw spread, not on
 * position, so a static sample yields a representative (single-shot) number.
 */

import type { TieringConfig } from "../data/contract";
import { spreadUnitToPoints, type PreviewTwoWay } from "./tiering";

function clamp(x: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, x));
}

/**
 * Whether a config carries an INVENTORY_SKEW strategy whose position-linear lean the
 * static zero-position client-price readout deliberately omits — the caller surfaces
 * a "+ inventory skew (position-dependent)" note when this is true.
 */
export function hasPositionDependentSkew(config: TieringConfig | null): boolean {
  return config !== null && config.strategies.some((s) => s.kind === "INVENTORY_SKEW");
}

/**
 * Compute the indicative OUTBOUND client two-way a tiering `config` produces from a
 * sample raw two-way (`sampleBid` / `sampleOffer`), mirroring the server pipeline
 * closely enough to be legible:
 *
 *  - SCALED_SMOOTHED_SPREAD present ⇒ it is the spread SOURCE: the output spread
 *    `O = min(m, c·(1 + f·D/e))` from the divergence `D` of the observed raw spread
 *    from the expected level `e` (single-shot, no smoothing history); half = O/2.
 *  - otherwise ⇒ half = Σ FLAT_MARKUP / INVENTORY_SKEW base half-spreads, each
 *    unit-converted to price points via {@link spreadUnitToPoints}.
 *  - the readout is SYMMETRIC around mid (zero-position assumption — see the module
 *    doc): `bid = mid − half`, `offer = mid + half`; INVENTORY_SKEW adds no lean here.
 *  - guardrails clamp `half ∈ [hMin, hMax]` and enforce `spreadFloor`
 *    (`offer − bid ≥ spreadFloor`) exactly as the server's price-space guardrails do.
 *
 * A `null` config (no TIERING feature ⇒ the raw composite streams unmarked) returns
 * the sample raw two-way unchanged.
 */
export function clientTwoWayFromTiering(
  config: TieringConfig | null,
  sampleBid: number,
  sampleOffer: number,
): PreviewTwoWay {
  if (config === null) return { bid: sampleBid, offer: sampleOffer };

  const mid = (sampleBid + sampleOffer) / 2;
  const rawSpread = sampleOffer - sampleBid;

  const scaled = config.strategies.find((s) => s.kind === "SCALED_SMOOTHED_SPREAD");
  let half: number;
  if (scaled) {
    const d = Math.abs(rawSpread - scaled.expectedSpread);
    const p =
      d <= scaled.maxDivergence || scaled.expectedSpread === 0
        ? 0
        : (scaled.spreadScaleFactor * d) / scaled.expectedSpread;
    const output = Math.min(scaled.maxOutputSpread, scaled.coreSpread * (1 + p));
    half = output / 2;
  } else {
    half = config.strategies.reduce(
      (sum, s) =>
        s.kind === "FLAT_MARKUP" || s.kind === "INVENTORY_SKEW"
          ? sum + spreadUnitToPoints(s.halfSpread, config.unit, mid)
          : sum,
      0,
    );
  }

  const g = config.guardrails;
  if (g !== null) {
    half = clamp(half, g.hMin, g.hMax);
    if (2 * half < g.spreadFloor) half = g.spreadFloor / 2;
  }

  return { bid: mid - half, offer: mid + half };
}
