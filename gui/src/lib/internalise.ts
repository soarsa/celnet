/**
 * Presentational helpers for the per-deal internalise / auto-hedge provenance the
 * server stamps on FI lifts (`Deal.internalise`). Shared by the deals blotter row
 * badge and the deal ticket detail so both read the same labels/formatting.
 *
 * Pure formatting over the `Internalise` contract — no fetch, no state.
 */

import type { HedgeBand, Internalise } from "../data/contract";

/** The row/badge label: warehoused internally vs shed external back-to-back. */
export function internaliseLabel(inl: Internalise): string {
  return inl.internalised ? "Internalised" : "B2B";
}

/** A human, sentence-case name for a DV01-utilisation band. */
export function hedgeBandLabel(band: HedgeBand): string {
  switch (band) {
    case "green":
      return "Green";
    case "amber":
      return "Amber";
    case "red":
      return "Red";
    case "breach":
      return "Breach";
  }
}

/**
 * Captured edge in basis points, to 2dp with an explicit sign glyph and a `bp`
 * suffix: 1.5 → "+1.50 bp", -0.4 → "−0.40 bp", 0 → "0.00 bp". Positive reads as
 * money made (bid/green), negative as losing (offer/red) at the call site.
 */
export function fmtEdgeBps(edgeBps: number): string {
  if (!Number.isFinite(edgeBps)) return "—";
  const abs = Math.abs(edgeBps).toFixed(2);
  const sign = edgeBps < 0 ? "−" : edgeBps > 0 ? "+" : "";
  return `${sign}${abs} bp`;
}
