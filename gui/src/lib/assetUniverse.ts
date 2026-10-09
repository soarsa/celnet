/**
 * assetUniverse.ts — the cross-class underlier-universe model the scope drill's
 * terminal leaf navigates (book → ASSET CLASS → underlier), the multi-asset
 * sibling of `lib/universe.ts` (which remains the FX pair universe, untouched —
 * FX behavior stays byte-identical).
 *
 * HONESTY (GUIDE.md rule 2): this module classifies, indexes and fuzzy-searches
 * over TODAY'S seeded `UnderlierSeed[]` (`data/assetUniverse.ts`) only — nothing
 * here invents an underlier, a level, or a venue the seed did not provide. The
 * shapes (`UnderlierRow`, `AssetUniverse`) are feed-ready: an estate market-data
 * feed drops a larger seed list in with zero downstream rework.
 *
 * Every row carries the FX-style pair PROJECTION (`underlyingPairProjection`) so
 * a non-FX underlier keys, labels and favourites exactly like a pair row (the
 * metal leg is its ISO-4217 X-code; equity/commodity project ticker/ccy; crypto
 * projects base/quote) — one id space across every class, no parallel grammar.
 */

import type { Underlying } from "../data/contract";
import type { UnderlierSeed } from "../data/assetUniverse";
import { underlyingPairProjection } from "../data/seed";
import type { AssetClass } from "../products/types";
import { fuzzyMatch, type FuzzyMatch } from "./fuzzy";
import { pairId, pairLabel } from "./universe";

/** The drillable asset classes, in rail order (FX first — the origin class). */
export const ASSET_CLASSES: readonly AssetClass[] = [
  "FX",
  "METAL",
  "EQUITY",
  "COMMODITY",
  "CRYPTO",
] as const;

/** Human label for an asset class (the class rail + section headers). */
export const ASSET_CLASS_LABEL: Record<AssetClass, string> = {
  FX: "FX",
  METAL: "Metals",
  EQUITY: "Equity",
  COMMODITY: "Commodity",
  CRYPTO: "Crypto",
};

/** The asset class an `Underlying` arm belongs to. */
export function underlierAssetClass(u: Underlying): AssetClass {
  switch (u.kind) {
    case "fx":
      return "FX";
    case "metal":
      return "METAL";
    case "equity":
      return "EQUITY";
    case "commodity":
      return "COMMODITY";
    case "digitalAsset":
      return "CRYPTO";
  }
}

/**
 * A seeded underlier lifted into the navigable universe model: its contract
 * identity plus the derived pair projection, class, and searchable haystack —
 * the cross-class analogue of `UniversePair`.
 */
export interface UnderlierRow {
  /** The contract `Underlying` arm (what a selection books / pre-targets). */
  underlying: Underlying;
  /** The asset class (the rail dimension). */
  assetClass: AssetClass;
  /** Stable key from the pair projection, e.g. "XAUUSD", "AAPLUSD", "BTCUSDT". */
  id: string;
  /**
   * Display / scope-crumb label. Pair-form for the paired classes ("XAU/USD",
   * "BTC/USDT"); the bare ticker for equity/commodity ("AAPL", "BRENT" — the
   * venue + ccy live in `detail`).
   */
  label: string;
  /** Short detail line: venue · ccy, the crypto settlement note, or "spot". */
  detail: string;
  /** Seeded indicative reference level (quote units per 1 base/asset unit). */
  refLevel: number;
  /** Display decimals for the reference level. */
  decimals: number;
  /** Settlement mechanics a ticket pre-target seeds (crypto: linear vs inverse). */
  settlementStyle: UnderlierSeed["settlementStyle"];
  /** Lower-cased search haystack: id, label, projection legs, detail, class. */
  haystack: string;
}

/** Lift a single seed into an `UnderlierRow`. Pure. */
export function toUnderlierRow(seed: UnderlierSeed): UnderlierRow {
  const assetClass = underlierAssetClass(seed.underlying);
  const projection = underlyingPairProjection(seed.underlying);
  const id = pairId(projection);
  const label =
    assetClass === "EQUITY" || assetClass === "COMMODITY"
      ? projection.base
      : pairLabel(projection);
  return {
    underlying: seed.underlying,
    assetClass,
    id,
    label,
    detail: seed.detail,
    refLevel: seed.refLevel,
    decimals: seed.decimals,
    settlementStyle: seed.settlementStyle,
    // Haystack covers the id ("xauusd"), the pair form ("xau/usd"), the bare
    // legs, the detail (venue/ccy/settlement words) and the class label, so a
    // query for "xau", "aapl", "xnas", "inverse" or "metals" all hit.
    haystack: `${id} ${pairLabel(projection)} ${projection.base} ${projection.quote} ${
      seed.detail
    } ${ASSET_CLASS_LABEL[assetClass]}`.toLowerCase(),
  };
}

/** The assembled non-FX universe: flat list + per-class grouping + O(1) lookup. */
export interface AssetUniverse {
  /** All non-FX rows, in seed order. */
  all: UnderlierRow[];
  /** Rows per asset class, in seed order (absent classes map to `[]`). */
  byClass: ReadonlyMap<AssetClass, UnderlierRow[]>;
  /** O(1) lookup by the projected pair id. */
  byId: ReadonlyMap<string, UnderlierRow>;
}

/**
 * Build the non-FX underlier universe from the seeds. Pure + deterministic.
 * (The FX class is served by `lib/universe.ts` over `PAIRS`, unchanged.)
 */
export function buildAssetUniverse(seeds: UnderlierSeed[]): AssetUniverse {
  const all = seeds.map(toUnderlierRow);
  const byId = new Map<string, UnderlierRow>(all.map((r) => [r.id, r]));
  const byClass = new Map<AssetClass, UnderlierRow[]>();
  for (const cls of ASSET_CLASSES) {
    if (cls === "FX") continue;
    byClass.set(
      cls,
      all.filter((r) => r.assetClass === cls),
    );
  }
  return { all, byClass, byId };
}

/** A search hit: the row plus its fuzzy score + label highlight indices. */
export interface UnderlierHit {
  row: UnderlierRow;
  score: number;
  /** Matched character indices into `row.label` (for highlight), or empty. */
  indices: number[];
}

/**
 * Fuzzy-search a class's rows — the same grammar as `searchUniverse`: an empty
 * query returns every row in seed order (score 0, no highlight); otherwise the
 * haystack scores/includes and the visible label is matched separately so the
 * highlight lines up with what the user sees (a detail-only hit keeps the row
 * with no highlight — honest, no fabricated positions). Sorted score desc, then
 * label asc (stable).
 */
export function searchUnderliers(rows: UnderlierRow[], query: string): UnderlierHit[] {
  const q = query.trim();
  if (q.length === 0) {
    return rows.map((row) => ({ row, score: 0, indices: [] }));
  }
  const hits: UnderlierHit[] = [];
  for (const row of rows) {
    const onHaystack: FuzzyMatch | null = fuzzyMatch(q, row.haystack);
    if (!onHaystack) continue;
    const onLabel = fuzzyMatch(q, row.label);
    hits.push({ row, score: onHaystack.score, indices: onLabel ? onLabel.indices : [] });
  }
  hits.sort((a, b) => b.score - a.score || a.row.label.localeCompare(b.row.label));
  return hits;
}
