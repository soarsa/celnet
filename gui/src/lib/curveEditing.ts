/**
 * Pure helpers shared by the Curves multi-curve manager surfaces (server commit
 * 38bcff9a): the dashboard, the definition editor, and the per-curve pillar editor.
 * No React, no transport — just slug minting, a starter pillar ladder for a new
 * curve, and a one-line pillar-ladder summary — so the UI pieces and the vitest
 * suite share ONE implementation.
 */

import type { RatesCurveSet } from "../data/contract";
import { pillarTenorLabel } from "../data/contract";
import { DEFAULT_USD_SOFR_CURVE, pillarMaturityYears } from "../data/ratesPricing";

/**
 * Mint a curve slug from a display name: lower-case, non-alphanumerics collapse to
 * single hyphens, trimmed. The curve id is the IMMUTABLE unique key, so the editor
 * generates a candidate here on create and the server rejects a collision
 * (`already_exists`). An all-punctuation name yields `""` — the caller keeps the
 * field editable and blocks a blank id (`invalid_argument`).
 */
export function slugifyCurveId(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

/**
 * The starter calibrating pillar set for a NEW curve, in the chosen currency, over
 * the default reference date and the canonical par-OIS ladder (a real, bootstrappable
 * set — never an empty placeholder). The trader refines it in the Pillars tab after
 * create; every field is a genuine quote, so a create with the starter validates.
 */
export function starterPillarSet(currency: string): RatesCurveSet {
  return {
    currency,
    referenceDate: DEFAULT_USD_SOFR_CURVE.referenceDate,
    pillars: DEFAULT_USD_SOFR_CURVE.pillars.map((p) => ({
      tenor: p.tenor,
      parRate: p.parRate,
    })),
  };
}

/**
 * A one-line summary of a curve's pillar ladder for the dashboard row: the pillar
 * count and the maturity span (first → last tenor), read off the maturity-ordered
 * ladder. An empty set (which the server never persists) renders as "no pillars".
 */
export function pillarLadderSummary(set: RatesCurveSet): string {
  const n = set.pillars.length;
  if (n === 0) return "no pillars";
  const ordered = [...set.pillars].sort(
    (a, b) =>
      pillarMaturityYears(a.tenor, set.referenceDate) -
      pillarMaturityYears(b.tenor, set.referenceDate),
  );
  const first = pillarTenorLabel(ordered[0]!.tenor);
  const last = pillarTenorLabel(ordered[ordered.length - 1]!.tenor);
  const span = n === 1 ? first : `${first}–${last}`;
  return `${n} pillar${n === 1 ? "" : "s"} · ${span}`;
}
