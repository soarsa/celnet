/**
 * universe.ts — a registry-ready pair-universe abstraction over the CURRENT
 * seeded pairs (TRADING-UNIVERSE-SCALE §5: navigate a large pair universe).
 *
 * HONESTY (GUIDE.md rule 2 — no fakes): the full pair-universe REGISTRY of
 * hundreds of pairs is a Phase-1 backlog item (P1-10) that is NOT built. This
 * module classifies, groups, favourites and fuzzy-searches over *today's*
 * seeded `PairContext[]` only — but its shapes (`UniversePair`, `PairGroup`,
 * `PairBucket`) and the pure `buildUniverse()` builder are structured so that
 * when the registry lands it drops in by feeding a larger `PairContext[]` (or a
 * richer source mapped to `UniversePair`) with ZERO rework downstream. Nothing
 * here invents a pair, a spot, or a bucket the source did not provide.
 *
 * Classification is derived purely from the pair's two currency codes against a
 * compact, well-known currency-class table (G10 majors vs the rest as EM); the
 * bucket is then majors / crosses / EM by the standard FX desk taxonomy:
 *   - MAJOR  — a G10 pair that includes USD (the dollar majors).
 *   - CROSS  — a G10×G10 pair WITHOUT USD (EUR/GBP, EUR/JPY, …).
 *   - EM     — any pair touching a non-G10 (emerging-market) currency.
 * This matches how a vol desk navigates its universe and needs no extra data.
 */

import type { CcyPair } from "../data/contract";
import type { PairContext } from "../data/seed";
import { fuzzyMatch, type FuzzyMatch } from "./fuzzy";

/** The desk taxonomy bucket a pair belongs to. */
export type PairBucket = "major" | "cross" | "em";

/** Stable, ordered list of the buckets for grouped rendering + headers. */
export const PAIR_BUCKETS: readonly PairBucket[] = ["major", "cross", "em"] as const;

/** Human header for each bucket (Anaheim-cased by the view). */
export const BUCKET_LABEL: Record<PairBucket, string> = {
  major: "Majors",
  cross: "Crosses",
  em: "Emerging",
};

/**
 * The G10 reserve/funding currencies. A pair is a "major" or "cross" iff both
 * legs are in this set; anything touching a currency outside it is bucketed EM.
 * (USD is implicit in the majors split below.) This is the standard G10 set and
 * is the natural seam the future registry's class map widens.
 */
export const G10: ReadonlySet<string> = new Set([
  "USD",
  "EUR",
  "JPY",
  "GBP",
  "CHF",
  "AUD",
  "NZD",
  "CAD",
  "SEK",
  "NOK",
]);

/** Canonical market form for a pair, e.g. {EUR,USD} → "EUR/USD". */
export function pairLabel(p: CcyPair): string {
  return `${p.base}/${p.quote}`;
}

/** A stable, collision-free key for a pair (used for maps + React keys). */
export function pairId(p: CcyPair): string {
  return `${p.base}${p.quote}`;
}

/** True iff the two pairs are the same market (base AND quote match). */
export function samePair(a: CcyPair, b: CcyPair): boolean {
  return a.base === b.base && a.quote === b.quote;
}

/** Classify a pair into the desk taxonomy bucket from its two currency codes. */
export function bucketOf(p: CcyPair): PairBucket {
  const bothG10 = G10.has(p.base) && G10.has(p.quote);
  if (!bothG10) return "em";
  return p.base === "USD" || p.quote === "USD" ? "major" : "cross";
}

/**
 * A pair lifted into the universe model: its market context plus the derived
 * classification and the searchable haystack. This is the registry-ready row —
 * the future P1-10 registry produces exactly this shape (with its own richer
 * metadata) and everything downstream (navigator, search, favourites) is
 * unchanged.
 */
export interface UniversePair {
  /** The pair identity. */
  pair: CcyPair;
  /** Market form, e.g. "EUR/USD". */
  label: string;
  /** Stable key, e.g. "EURUSD". */
  id: string;
  /** Desk taxonomy bucket. */
  bucket: PairBucket;
  /** The seeded market context (spot, vol, rates) — the real source numbers. */
  market: PairContext["market"];
  /** Pip display precision (from the seed). */
  pipDecimals: number;
  /** Lower-cased search haystack: id + market form (both with and without the slash). */
  haystack: string;
}

/** One bucket's worth of pairs, for grouped rendering. */
export interface PairGroup {
  bucket: PairBucket;
  label: string;
  pairs: UniversePair[];
}

/** The assembled universe: flat list + the canonical bucket grouping. */
export interface Universe {
  /** All pairs, in source order. */
  all: UniversePair[];
  /** Pairs grouped into the canonical buckets (only non-empty buckets, in order). */
  groups: PairGroup[];
  /** O(1) lookup by `pairId`. */
  byId: ReadonlyMap<string, UniversePair>;
}

/** Lift a single `PairContext` into a `UniversePair`. */
export function toUniversePair(ctx: PairContext): UniversePair {
  const id = pairId(ctx.pair);
  const label = pairLabel(ctx.pair);
  return {
    pair: ctx.pair,
    label,
    id,
    bucket: bucketOf(ctx.pair),
    market: ctx.market,
    pipDecimals: ctx.pipDecimals,
    // Haystack covers "EURUSD", "EUR/USD" and the bare legs so a query for
    // "eur", "usdjpy" or "jpy" all hit.
    haystack: `${id} ${label} ${ctx.pair.base} ${ctx.pair.quote}`.toLowerCase(),
  };
}

/**
 * Build the universe from the current `PairContext[]`. Pure + deterministic.
 * When the P1-10 registry lands, feed it the registry's pairs (or map the
 * registry rows to `PairContext`) — no other call site changes.
 */
export function buildUniverse(pairs: PairContext[]): Universe {
  const all = pairs.map(toUniversePair);
  const byId = new Map<string, UniversePair>(all.map((u) => [u.id, u]));
  const groups: PairGroup[] = [];
  for (const bucket of PAIR_BUCKETS) {
    const inBucket = all.filter((u) => u.bucket === bucket);
    if (inBucket.length > 0) {
      groups.push({ bucket, label: BUCKET_LABEL[bucket], pairs: inBucket });
    }
  }
  return { all, groups, byId };
}

/** A search hit: the pair plus its fuzzy score + matched indices (for highlight). */
export interface UniverseHit {
  pair: UniversePair;
  score: number;
  /** Matched character indices into `pair.label` (for highlight), or empty. */
  indices: number[];
}

/**
 * Fuzzy-search the universe. An empty query returns every pair in source order
 * (score 0, no highlight) so the navigator shows the full list when idle.
 * Otherwise we match against the haystack for scoring/inclusion AND separately
 * against the visible `label` so the highlight indices line up with what the
 * user sees. Results are sorted by score desc, then label asc (stable).
 */
export function searchUniverse(u: Universe, query: string): UniverseHit[] {
  const q = query.trim();
  if (q.length === 0) {
    return u.all.map((pair) => ({ pair, score: 0, indices: [] }));
  }
  const hits: UniverseHit[] = [];
  for (const pair of u.all) {
    const onHaystack: FuzzyMatch | null = fuzzyMatch(q, pair.haystack);
    if (!onHaystack) continue;
    // Indices for highlight are computed against the visible label; if the query
    // doesn't subsequence the label (e.g. it matched a bare leg), we keep the
    // hit but render no highlight (honest — we don't fabricate positions).
    const onLabel = fuzzyMatch(q, pair.label);
    hits.push({
      pair,
      score: onHaystack.score,
      indices: onLabel ? onLabel.indices : [],
    });
  }
  hits.sort((a, b) => b.score - a.score || a.pair.label.localeCompare(b.pair.label));
  return hits;
}

/**
 * Group a flat hit list back into the canonical buckets (for the grouped,
 * searched navigator view). Preserves the incoming hit order within each
 * bucket and drops empty buckets.
 */
export function groupHits(hits: UniverseHit[]): { bucket: PairBucket; label: string; hits: UniverseHit[] }[] {
  const out: { bucket: PairBucket; label: string; hits: UniverseHit[] }[] = [];
  for (const bucket of PAIR_BUCKETS) {
    const inBucket = hits.filter((h) => h.pair.bucket === bucket);
    if (inBucket.length > 0) out.push({ bucket, label: BUCKET_LABEL[bucket], hits: inBucket });
  }
  return out;
}
