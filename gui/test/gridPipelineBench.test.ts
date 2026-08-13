/**
 * gridPipelineBench — a MEASURED budget for the shared grid pipeline
 * (`global search → applyColumnFilters → applySort`, lib/grid.ts).
 *
 * WHY this exists rather than a "looks fast enough" assumption: `useTableFilter`
 * memoises on `[rows, query]`, but a streaming blotter hands it a NEW `rows`
 * identity on every tick, so the whole pipeline re-runs per tick — it is on the
 * paint path, not a cold path. Adding N per-column predicates over 16 columns ×
 * thousands of rows is therefore an unmeasured per-frame cost, and the only
 * honest way to know whether the semantic `<DataTable>` binding can carry the
 * Deals blotter (vs the virtualised `<DataGrid>`) is to time it.
 *
 * The budget is one animation frame (16ms) for the WHOLE pipeline over a
 * synthetic 5,000-row × 16-column set — the shape of the largest real surface.
 * The assertion bound is deliberately loose (a CI box under load is ~3-5× a warm
 * laptop) so this is a REGRESSION guard, not a flake generator; the exact
 * measured numbers are printed so a reviewer sees the real cost.
 */
import { describe, expect, it } from "vitest";

import {
  applyColumnFilters,
  applySort,
  deriveSelectOptions,
  type ColumnDef,
  type ColumnFilterState,
} from "../src/lib/grid";

/** A synthetic blotter row shaped like the Deals blotter's 16-column datum. */
interface BenchRow {
  readonly id: string;
  readonly time: string;
  readonly counterparty: string;
  readonly desk: string;
  readonly kind: string;
  readonly product: string;
  readonly security: string;
  readonly tenor: number;
  readonly ccy: string;
  readonly notional: number;
  readonly price: number;
  readonly side: string;
  readonly trader: string;
  readonly position: number;
  readonly portfolio: string;
  readonly hedge: string;
}

const CPTYS = ["Millennium", "Balyasny", "Point72", "Citadel", "Brevan Howard", "Man AHL"];
const DESKS = ["Rates NY", "Rates LDN", "Credit LDN", "Macro NY"];
const KINDS = ["RFQ", "IOI", "RFS"];
const PRODUCTS = ["OIS", "IRS", "FRA", "BOND"];
const CCYS = ["USD", "EUR", "GBP", "JPY"];
const SIDES = ["BUY", "SELL", "2-WAY"];
const TRADERS = ["a.chen", "r.patel", "m.okafor", "l.novak"];
const BANDS = ["Internalised", "B2B"];

/** Deterministic pseudo-random so the benchmark measures the SAME work every run. */
function lcg(seed: number): () => number {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 4294967296;
  };
}

function makeRows(n: number): BenchRow[] {
  const rnd = lcg(20260811);
  const pick = <T,>(xs: readonly T[]): T => xs[Math.floor(rnd() * xs.length)] as T;
  return Array.from({ length: n }, (_, i) => ({
    id: `D-${i.toString().padStart(6, "0")}`,
    time: `${String(9 + (i % 8)).padStart(2, "0")}:${String(i % 60).padStart(2, "0")}:${String((i * 7) % 60).padStart(2, "0")}`,
    counterparty: pick(CPTYS),
    desk: pick(DESKS),
    kind: pick(KINDS),
    product: pick(PRODUCTS),
    security: `UST ${1 + (i % 30)}Y ${(2 + rnd() * 3).toFixed(3)}%`,
    tenor: 1 + (i % 30),
    ccy: pick(CCYS),
    notional: Math.round(rnd() * 500_000_000),
    price: 90 + rnd() * 20,
    side: pick(SIDES),
    trader: pick(TRADERS),
    position: 9000 + i,
    portfolio: `Book ${i % 12}`,
    hedge: pick(BANDS),
  }));
}

const COLUMNS: ReadonlyArray<ColumnDef<BenchRow>> = [
  { key: "time", header: "Time", width: 80, accessor: (r) => r.time, sortKey: "time", filter: { kind: "text" } },
  {
    key: "cpty",
    header: "Counterparty",
    width: 140,
    accessor: (r) => r.counterparty,
    sortKey: "cpty",
    filter: { kind: "select" },
  },
  { key: "desk", header: "Desk", width: 110, accessor: (r) => r.desk, sortKey: "desk", filter: { kind: "select" } },
  { key: "kind", header: "Type", width: 70, accessor: (r) => r.kind, sortKey: "kind", filter: { kind: "select" } },
  {
    key: "product",
    header: "Product",
    width: 80,
    accessor: (r) => r.product,
    sortKey: "product",
    filter: { kind: "select" },
  },
  {
    key: "security",
    header: "Security",
    width: 170,
    accessor: (r) => r.security,
    sortKey: "security",
    filter: { kind: "text" },
  },
  {
    key: "tenor",
    header: "Tenor",
    width: 70,
    accessor: (r) => `${r.tenor}y`,
    sortValue: (r) => r.tenor,
    sortKey: "tenor",
    filter: { kind: "range" },
  },
  { key: "ccy", header: "Ccy", width: 60, accessor: (r) => r.ccy, sortKey: "ccy", filter: { kind: "select" } },
  {
    key: "notional",
    header: "Notional",
    width: 110,
    accessor: (r) => r.notional.toString(),
    sortValue: (r) => r.notional,
    sortKey: "notional",
    filter: { kind: "range" },
  },
  {
    key: "price",
    header: "Rate",
    width: 90,
    accessor: (r) => r.price.toFixed(4),
    sortValue: (r) => r.price,
    sortKey: "price",
    filter: { kind: "range" },
  },
  { key: "side", header: "Side", width: 120, accessor: (r) => r.side, sortKey: "side", filter: { kind: "select" } },
  { key: "trader", header: "Trader", width: 100, accessor: (r) => r.trader, sortKey: "trader", filter: { kind: "select" } },
  {
    key: "position",
    header: "Position",
    width: 90,
    accessor: (r) => `#${r.position}`,
    sortValue: (r) => r.position,
    sortKey: "position",
    filter: { kind: "range" },
  },
  {
    key: "portfolio",
    header: "Risk Portfolio",
    width: 130,
    accessor: (r) => r.portfolio,
    sortKey: "portfolio",
    filter: { kind: "select" },
  },
  { key: "hedge", header: "Hedge", width: 110, accessor: (r) => r.hedge, sortKey: "hedge", filter: { kind: "select" } },
  { key: "deal", header: "Deal", width: 120, accessor: (r) => r.id, sortKey: "deal", filter: { kind: "text" } },
];

/** The global-search stage, byte-identical to `useTableFilter`'s inner matcher. */
function globalSearch(rows: readonly BenchRow[], query: string): BenchRow[] {
  const needle = query.trim().toLowerCase();
  if (needle === "") return rows.slice();
  return rows.filter((r) =>
    COLUMNS.map((c) => c.accessor(r)).join(" ").toLowerCase().includes(needle),
  );
}

/** Median of `runs` timings of `fn`, in ms — median resists a GC outlier. */
function timeMs(fn: () => void, runs = 9): number {
  const samples: number[] = [];
  for (let i = 0; i < runs; i++) {
    const t0 = performance.now();
    fn();
    samples.push(performance.now() - t0);
  }
  samples.sort((a, b) => a - b);
  return samples[Math.floor(samples.length / 2)] as number;
}

const ROW_COUNT = 5_000;
/** One animation frame. The pipeline must fit inside it on a warm machine. */
const FRAME_BUDGET_MS = 16;
/** CI headroom over the frame budget — this asserts a REGRESSION, not a flake. */
const ASSERT_BUDGET_MS = FRAME_BUDGET_MS * 8;

describe("grid pipeline — measured cost over 5,000 rows × 16 columns", () => {
  const rows = makeRows(ROW_COUNT);

  it("reports the per-stage cost of search → column filters → sort", () => {
    // Stage 1: the global search alone (what the blotters already pay per tick).
    const searchMs = timeMs(() => globalSearch(rows, "citadel"));

    // Stage 2: column filters — the NEW cost. Worst realistic case: every filter
    // kind active at once across many columns (text + select + range).
    const heavy: ColumnFilterState = {
      cpty: { kind: "select", selected: "Citadel" },
      desk: { kind: "select", selected: "Rates NY" },
      kind: { kind: "select", selected: "RFQ" },
      ccy: { kind: "select", selected: "USD" },
      side: { kind: "select", selected: "BUY" },
      trader: { kind: "select", selected: "a.chen" },
      security: { kind: "text", query: "UST" },
      deal: { kind: "text", query: "D-0" },
      time: { kind: "text", query: ":1" },
      tenor: { kind: "range", min: 2, max: 25 },
      notional: { kind: "range", min: 1_000_000, max: null },
      price: { kind: "range", min: 95, max: 105 },
      position: { kind: "range", min: 9000, max: null },
    };
    const filterMs = timeMs(() => applyColumnFilters(rows, COLUMNS, heavy));

    // A single-column text filter — the ordinary case a trader actually types.
    const single: ColumnFilterState = { security: { kind: "text", query: "UST 1" } };
    const singleFilterMs = timeMs(() => applyColumnFilters(rows, COLUMNS, single));

    // The no-filter path (the common case): must be effectively free.
    const idleFilterMs = timeMs(() => applyColumnFilters(rows, COLUMNS, {}));

    // Stage 3: sort. Numeric (sortValue) and text (accessor + localeCompare) are
    // very different costs — localeCompare is the expensive one, so measure both.
    const sortNumMs = timeMs(() => applySort(rows, COLUMNS, "notional", "desc"));
    const sortTextMs = timeMs(() => applySort(rows, COLUMNS, "cpty", "asc"));

    // Select-option derivation runs over the UNFILTERED rows on every render.
    const optionsMs = timeMs(() => {
      for (const c of COLUMNS) deriveSelectOptions(rows, c);
    });

    // The realistic whole-pipeline pass a trader provokes: type in the search box
    // with one column filter set and a text sort applied.
    const pipelineMs = timeMs(() => {
      const searched = globalSearch(rows, "citadel");
      const filtered = applyColumnFilters(searched, COLUMNS, single);
      applySort(filtered, COLUMNS, "cpty", "asc");
    });

    const report = [
      `rows=${ROW_COUNT} cols=${COLUMNS.length}`,
      `globalSearch=${searchMs.toFixed(2)}ms`,
      `columnFilters(13 active)=${filterMs.toFixed(2)}ms`,
      `columnFilters(1 text)=${singleFilterMs.toFixed(2)}ms`,
      `columnFilters(none)=${idleFilterMs.toFixed(2)}ms`,
      `sort(numeric)=${sortNumMs.toFixed(2)}ms`,
      `sort(text/localeCompare)=${sortTextMs.toFixed(2)}ms`,
      `deriveSelectOptions(x16)=${optionsMs.toFixed(2)}ms`,
      `FULL PIPELINE=${pipelineMs.toFixed(2)}ms`,
    ].join("\n  ");
    // Printed so the numbers are visible in the gate output, not just asserted.
    console.info(`\n[grid pipeline bench]\n  ${report}\n`);

    // The pipeline must not regress into multi-frame territory.
    expect(pipelineMs).toBeLessThan(ASSERT_BUDGET_MS);
    // An idle table (no column filters at all) must stay near-free — this is the
    // state 99% of renders are in, so a regression here would tax every tick.
    expect(idleFilterMs).toBeLessThan(FRAME_BUDGET_MS);
  });

  it("column filtering is monotone: more filters never yield more rows", () => {
    // A correctness oracle alongside the timing — the pipeline must still be a
    // pure narrowing, or a "fast" implementation could be fast by being wrong.
    const one: ColumnFilterState = { ccy: { kind: "select", selected: "USD" } };
    const two: ColumnFilterState = { ...one, kind: { kind: "select", selected: "RFQ" } };
    const a = applyColumnFilters(rows, COLUMNS, one);
    const b = applyColumnFilters(rows, COLUMNS, two);
    expect(b.length).toBeLessThanOrEqual(a.length);
    expect(a.length).toBeLessThanOrEqual(rows.length);
    // Every survivor of the tighter filter survived the looser one.
    const aSet = new Set(a.map((r) => r.id));
    expect(b.every((r) => aSet.has(r.id))).toBe(true);
  });
});
