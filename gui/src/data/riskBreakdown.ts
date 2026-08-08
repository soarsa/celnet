/**
 * riskBreakdown — the pure, client-side fold that breaks a risk portfolio's routed
 * flow down by TENOR bucket and by INSTRUMENT, for the Risk Dashboard drill-down.
 *
 * DATA SOURCE (honest): the per-book risk roster on the wire ({@link
 * "./contract".RiskBookRisk}) carries only per-portfolio AGGREGATES (net/gross
 * notional, position count, DV01) — NOT tenor/instrument buckets. The rates
 * position ledger ({@link "./contract".RatesPosition}) DOES carry the
 * instrument-level tenor/notional, but it books into `(entity, book)` NETTING
 * cells and carries NO risk-portfolio id, so it cannot be attributed to a risk
 * portfolio. The ONE GUI-available source that links instrument-level detail to a
 * risk PORTFOLIO is the executed {@link "./contract".Deal}: every routed fill
 * carries `riskBookId` (the portfolio its risk routed into), its `OisInstrument`
 * (tenor + direction) and `notional`. So the dashboard groups the DEALS it already
 * receives (the same `listDeals` seam the Deals blotter uses), filtered to a book.
 *
 * DV01 is deliberately NOT fabricated here: the `Deal` wire carries no per-fill
 * DV01, and the roster's aggregate DV01 arrives `null` at this seam (rates-book
 * pass not yet wired) — so a bucket's DV01 stays `null` (rendered "—") until a
 * fill carries a real DV01. The summing is null-aware (mirrors the dashboard's
 * `globalExposureOf`), so the moment a DV01-bearing source is threaded through
 * {@link BreakdownItem}, the per-bucket DV01 sums correctly with zero rework.
 */

import type { Deal, RatesProductKind } from "./contract";

/**
 * One normalized instrument-level contribution to a portfolio's breakdown — the
 * fields the tenor/instrument fold needs, decoupled from the wire `Deal` so the
 * fold is unit-testable and future-proof (a DV01-bearing source only has to fill
 * {@link dv01}). `notional` is the GROSS size (absolute); `dv01` is `null` when the
 * source carries none (honest "—", never a fabricated 0).
 */
export interface BreakdownItem {
  /** Whole-year tenor of the underlying instrument (drives the tenor bucket). */
  tenorYears: number;
  /** The instrument label to group the "by instrument" view on (e.g. `10y OIS`). */
  instrument: string;
  /** The rates product family the contribution belongs to (drives the product bucket). */
  productKind: RatesProductKind;
  /** Gross (absolute) base-currency notional of this contribution. */
  notional: number;
  /** DV01 contribution, or `null` when the source carries none (rendered "—"). */
  dv01: number | null;
}

/**
 * One aggregated breakdown row (a tenor bucket, or a distinct instrument): the
 * summed gross notional, the count of contributing fills, and the null-aware DV01
 * sum (`null` until at least one contribution carries a real DV01).
 */
export interface BreakdownRow {
  /** The tenor-bucket label or instrument label this row aggregates. */
  key: string;
  /** Summed gross notional across the contributions in this row. */
  notional: number;
  /** Number of contributing fills in this row. */
  count: number;
  /** Null-aware summed DV01, or `null` when no contribution carries one. */
  dv01: number | null;
}

/** The full breakdown of a portfolio's routed flow: both lenses plus the totals. */
export interface RiskBreakdown {
  /** The coarse tenor-bucket lens, ordered short → long. */
  byTenor: BreakdownRow[];
  /** The distinct-instrument lens, ordered short → long tenor. */
  byInstrument: BreakdownRow[];
  /** The product-family lens (`OIS`/`IRS`/`FRA`/`BOND`), ordered by gross desc. */
  byProduct: BreakdownRow[];
  /** Total gross notional across all contributions. */
  totalNotional: number;
  /** Total number of contributing fills. */
  count: number;
}

/**
 * The ordered tenor buckets a rates portfolio is sliced into — a whole-year tenor
 * falls into the FIRST bucket whose `max` it does not exceed (`≤ 2y`, `2–5y`,
 * `5–10y`, `10–20y`, `> 20y`). Boundaries are inclusive on the upper edge.
 */
const TENOR_BUCKETS: readonly { label: string; max: number }[] = [
  { label: "≤ 2y", max: 2 },
  { label: "2–5y", max: 5 },
  { label: "5–10y", max: 10 },
  { label: "10–20y", max: 20 },
  { label: "> 20y", max: Number.POSITIVE_INFINITY },
];

/** The tenor-bucket label a whole-year tenor falls into. */
export function tenorBucketLabel(years: number): string {
  // The last bucket's `max` is +Infinity, so a match is always found; the literal
  // fallback keeps the function total for the type-checker.
  return TENOR_BUCKETS.find((b) => years <= b.max)?.label ?? "> 20y";
}

/**
 * Map a routed {@link Deal} to a normalized {@link BreakdownItem}. The instrument
 * label reads `<n>y <PRODUCT>` off the deal's decoded arm ({@link Deal.productKind}) —
 * `10y OIS`, `5y IRS`, `10y BOND` — so a mixed book classifies each fill by its real
 * family. DV01 is `null` — the `Deal` wire carries none (never fabricated); the fold
 * stays null-aware for when one is threaded through.
 */
export function dealToBreakdownItem(deal: Deal): BreakdownItem {
  const tenorYears = deal.instrument.tenorYears;
  return {
    tenorYears,
    instrument: `${tenorYears}y ${deal.productKind}`,
    productKind: deal.productKind,
    notional: Math.abs(deal.notional),
    dv01: null,
  };
}

/**
 * Fold items into aggregated rows keyed by `keyOf`, summing notional + count and
 * the null-aware DV01, then order the rows by their smallest contributing tenor
 * (short → long). Pure; no mutation of the input.
 */
function aggregate(
  items: readonly BreakdownItem[],
  keyOf: (item: BreakdownItem) => string,
): BreakdownRow[] {
  const acc = new Map<
    string,
    { notional: number; count: number; dv01: number | null; sort: number }
  >();
  for (const item of items) {
    const key = keyOf(item);
    const prev = acc.get(key) ?? {
      notional: 0,
      count: 0,
      dv01: null as number | null,
      sort: item.tenorYears,
    };
    acc.set(key, {
      notional: prev.notional + item.notional,
      count: prev.count + 1,
      // Null-aware: stays null until a real DV01 arrives, then sums (mirrors
      // the dashboard's `globalExposureOf`).
      dv01: item.dv01 === null ? prev.dv01 : (prev.dv01 ?? 0) + item.dv01,
      sort: Math.min(prev.sort, item.tenorYears),
    });
  }
  return [...acc.entries()]
    .map(([key, r]) => ({ key, notional: r.notional, count: r.count, dv01: r.dv01, sort: r.sort }))
    .sort((a, b) => a.sort - b.sort)
    .map(({ sort: _sort, ...row }) => row);
}

/** Group contributions into the coarse tenor-bucket lens. */
export function groupByTenor(items: readonly BreakdownItem[]): BreakdownRow[] {
  return aggregate(items, (item) => tenorBucketLabel(item.tenorYears));
}

/** Group contributions into the distinct-instrument lens. */
export function groupByInstrument(items: readonly BreakdownItem[]): BreakdownRow[] {
  return aggregate(items, (item) => item.instrument);
}

/**
 * Group contributions into the product-family lens (`OIS`/`IRS`/`FRA`/`BOND`),
 * ordered by gross notional DESCENDING (largest family first; tie-broken by DV01
 * desc then key) — unlike the tenor/instrument lenses, a product bucket spans many
 * tenors, so the short→long tenor order is meaningless and gross exposure is the
 * natural rank. Pure; the summed gross reconciles to the same total the tenor and
 * instrument lenses do (every lens folds the identical item set).
 */
export function groupByProduct(items: readonly BreakdownItem[]): BreakdownRow[] {
  return [...aggregate(items, (item) => item.productKind)].sort(
    (a, b) => b.notional - a.notional || (b.dv01 ?? 0) - (a.dv01 ?? 0) || a.key.localeCompare(b.key),
  );
}

/**
 * The full tenor + instrument breakdown of a portfolio's routed deals — the
 * honest client-side fold the Risk Dashboard drill-down renders. Pass the deals
 * already filtered to the portfolio (`deal.riskBookId === bookId`).
 */
export function riskBreakdownFor(deals: readonly Deal[]): RiskBreakdown {
  const items = deals.map(dealToBreakdownItem);
  let totalNotional = 0;
  for (const item of items) totalNotional += item.notional;
  return {
    byTenor: groupByTenor(items),
    byInstrument: groupByInstrument(items),
    byProduct: groupByProduct(items),
    totalNotional,
    count: items.length,
  };
}
