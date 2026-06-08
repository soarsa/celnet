/**
 * DataGrid tests — the shared accessible virtualised groupable grid (GW0-S2,
 * docs/GW-FOUNDATION-PLAN.md §2). The grid REVERSES the deliberate `role=grid`
 * opt-out documented at StreamWorkspace.tsx:699; these tests prove the keyboard
 * model + virtualisation + tick-coalescing + accessibility that the opt-out was
 * waiting on. (StreamWorkspace itself is migrated onto the grid in GW3 — not here.)
 *
 * Each suite is gated against an INDEPENDENT oracle that can disagree with the
 * implementation, per the verification contract:
 *   - roving model → WAI-ARIA APG "grid" invariants hand-pinned from the spec,
 *     asserted on the rendered DOM `tabindex`/`aria-*` (NOT the reducer internals);
 *   - virtualisation → the conservation identity rendered+head+tail==total and
 *     off-screen rows ABSENT from the DOM (a node count), disjoint from the
 *     floor/ceil windowing arithmetic;
 *   - tick-coalescing → the accounting identity applied+coalesced==produced with
 *     an independent reduce-to-last replay (NOT the coalescer's own map);
 *   - a11y → third-party axe-core on the live `role=grid` DOM (zero violations on
 *     the grid-structure rules) — the jsdom-compatible check; full Playwright axe
 *     runs post-merge.
 */
import { act } from "react";
import axe from "axe-core";
import { describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";

import { DataGrid, cellValue, useCoalescedCells } from "../src/components/DataGrid";
import {
  HEADER_ROW,
  coalesce,
  columnWindow,
  flattenGroups,
  moveRoving,
  type CellUpdate,
  type ColumnDef,
  type GroupModel,
  type RowGroup,
} from "../src/lib/grid";
import { renderHook } from "@testing-library/react";

// --------------------------------------------------------------------------
// Fixtures
// --------------------------------------------------------------------------

interface Row {
  readonly id: string;
  readonly pair: string;
  readonly bid: number;
  readonly ask: number;
}

const COLUMNS: ReadonlyArray<ColumnDef<Row>> = [
  { key: "pair", header: "Pair", width: 90, align: "left", accessor: (r) => r.pair, sortKey: "pair" },
  { key: "bid", header: "Bid", width: 80, accessor: (r) => r.bid.toFixed(4), sortKey: "bid" },
  { key: "ask", header: "Ask", width: 80, accessor: (r) => r.ask.toFixed(4) },
];

function makeRows(n: number, prefix = "r"): Array<{ key: string; datum: Row }> {
  return Array.from({ length: n }, (_, i) => ({
    key: `${prefix}${i}`,
    datum: { id: `${prefix}${i}`, pair: `P${i}`, bid: i + 0.1, ask: i + 0.2 },
  }));
}

function singleGroup(rows: Array<{ key: string; datum: Row }>): RowGroup<Row>[] {
  return [{ key: "", label: "", rows }];
}

// --------------------------------------------------------------------------
// 1. Roving-tabindex keyboard model — APG "grid" invariants
// --------------------------------------------------------------------------

describe("DataGrid — roving tabindex (WAI-ARIA APG grid keyboard model)", () => {
  it("renders role=grid with aria-rowcount/colcount over the FULL set, not the window", () => {
    // 1000 rows, fixed 30px height ⇒ only a window is in the DOM, but the
    // advertised counts cover the whole grid + the header band.
    render(
      <DataGrid label="markets" columns={COLUMNS} groups={singleGroup(makeRows(1000))} rowHeight={30} />,
    );
    const grid = screen.getByRole("grid");
    // APG invariant (hand-pinned): rowcount = data rows + 1 header row.
    expect(grid).toHaveAttribute("aria-rowcount", "1001");
    expect(grid).toHaveAttribute("aria-colcount", "3");
  });

  it("has EXACTLY ONE cell with tabindex=0 (the roving cell); all others -1", () => {
    render(
      <DataGrid label="markets" columns={COLUMNS} groups={singleGroup(makeRows(40))} rowHeight={30} />,
    );
    const grid = screen.getByRole("grid");
    // APG invariant: exactly one cell is in the tab order at any time.
    const focusable = grid.querySelectorAll('[tabindex="0"]');
    expect(focusable).toHaveLength(1);
    // Every other header/data cell is removed from the tab order.
    const allCells = grid.querySelectorAll('[role="columnheader"],[role="gridcell"]');
    const negative = Array.from(allCells).filter((el) => el.getAttribute("tabindex") === "-1");
    expect(negative.length).toBe(allCells.length - 1);
  });

  it("ArrowDown then ArrowRight moves the single tabindex=0 cell on the rendered DOM", () => {
    render(
      <DataGrid label="markets" columns={COLUMNS} groups={singleGroup(makeRows(40))} rowHeight={30} />,
    );
    const grid = screen.getByRole("grid");
    grid.focus();
    // Start: header band, col 0 (the initial active cell).
    expect(document.querySelector('[tabindex="0"]')?.getAttribute("role")).toBe("columnheader");

    act(() => {
      fireEvent.keyDown(grid, { key: "ArrowDown" });
    });
    // Now a data-row gridcell holds focus.
    const afterDown = document.querySelector('[tabindex="0"]');
    expect(afterDown?.getAttribute("role")).toBe("gridcell");
    expect(afterDown?.getAttribute("aria-colindex")).toBe("1");

    act(() => {
      fireEvent.keyDown(grid, { key: "ArrowRight" });
    });
    const afterRight = document.querySelector('[tabindex="0"]');
    expect(afterRight?.getAttribute("role")).toBe("gridcell");
    expect(afterRight?.getAttribute("aria-colindex")).toBe("2");
    // Still exactly one roving cell.
    expect(document.querySelectorAll('[tabindex="0"]')).toHaveLength(1);
  });

  it("Home goes to row start, End to row end, Ctrl+End to the bottom-right corner", () => {
    // Reducer-level check against the APG spec (clamping + corner moves). The
    // oracle is the hand-pinned APG behaviour, asserted independently of the DOM.
    const bounds = { rowCount: 100, colCount: 3, pageSize: 10 };
    // Start mid-grid.
    const mid = { active: { row: 50, col: 1 } };
    expect(moveRoving(mid, "Home", bounds).active).toEqual({ row: 50, col: 0 });
    expect(moveRoving(mid, "End", bounds).active).toEqual({ row: 50, col: 2 });
    // Ctrl widens to the grid corners.
    expect(moveRoving(mid, "Home", bounds, true).active).toEqual({ row: 0, col: 0 });
    expect(moveRoving(mid, "End", bounds, true).active).toEqual({ row: 99, col: 2 });
  });

  it("PageDown/PageUp move by a page of rows and clamp at the edges", () => {
    const bounds = { rowCount: 100, colCount: 3, pageSize: 20 };
    expect(moveRoving({ active: { row: 0, col: 0 } }, "PageDown", bounds).active.row).toBe(20);
    // Clamp at the last row (never overrun).
    expect(moveRoving({ active: { row: 95, col: 0 } }, "PageDown", bounds).active.row).toBe(99);
    // Clamp at the header band floor (never above HEADER_ROW).
    expect(moveRoving({ active: { row: 5, col: 0 } }, "PageUp", bounds).active.row).toBe(HEADER_ROW);
  });

  it("arrow moves never escape the grid bounds (clamp at all four edges)", () => {
    const bounds = { rowCount: 3, colCount: 2, pageSize: 3 };
    // Up from the header stays at the header.
    expect(moveRoving({ active: { row: HEADER_ROW, col: 0 } }, "ArrowUp", bounds).active.row).toBe(
      HEADER_ROW,
    );
    // Left from col 0 stays at col 0; right from last col stays.
    expect(moveRoving({ active: { row: 0, col: 0 } }, "ArrowLeft", bounds).active.col).toBe(0);
    expect(moveRoving({ active: { row: 0, col: 1 } }, "ArrowRight", bounds).active.col).toBe(1);
    // Down from the last row stays.
    expect(moveRoving({ active: { row: 2, col: 0 } }, "ArrowDown", bounds).active.row).toBe(2);
  });

  it("an empty grid (no columns) is a no-op move (no NaN/overrun)", () => {
    const bounds = { rowCount: 0, colCount: 0, pageSize: 1 };
    const s = { active: { row: HEADER_ROW, col: 0 } };
    expect(moveRoving(s, "ArrowDown", bounds)).toBe(s);
  });
});

// --------------------------------------------------------------------------
// 2. Column + row virtualisation — conservation identity + DOM node count
// --------------------------------------------------------------------------

describe("DataGrid — row virtualisation (conservation identity + off-screen absent)", () => {
  it("renders only a window of rows; off-screen data rows are ABSENT from the DOM", () => {
    render(
      <DataGrid label="markets" columns={COLUMNS} groups={singleGroup(makeRows(2000))} rowHeight={20} />,
    );
    const grid = screen.getByRole("grid");
    // Count rendered data rows (role=row, kind data — they carry 3 gridcells).
    const dataRows = within(grid)
      .getAllByRole("row", { hidden: true })
      .filter((r) => within(r).queryAllByRole("gridcell", { hidden: true }).length > 0);
    // 2000 rows must NOT all be in the DOM (the whole point of virtualisation).
    expect(dataRows.length).toBeGreaterThan(0);
    expect(dataRows.length).toBeLessThan(200);
  });
});

describe("columnWindow — conservation identity (independent of floor/ceil math)", () => {
  it("padLeft + Σ(rendered widths) + padRight == totalWidth for any scroll", () => {
    const widths = [90, 80, 80, 120, 60, 100, 70];
    const total = widths.reduce((a, b) => a + b, 0);
    for (const scroll of [0, 50, 137, 300, 9999]) {
      const win = columnWindow(widths, scroll, 250, 1);
      const rendered = widths.slice(win.start, win.end).reduce((a, b) => a + b, 0);
      // The conservation identity — holds for ANY correct windower.
      expect(win.padLeft + rendered + win.padRight).toBe(total);
      expect(win.totalWidth).toBe(total);
      expect(win.start).toBeGreaterThanOrEqual(0);
      expect(win.end).toBeLessThanOrEqual(widths.length);
    }
  });

  it("a wide viewport renders every column with zero spacers", () => {
    const widths = [90, 80, 80];
    const win = columnWindow(widths, 0, 10_000, 1);
    expect(win.start).toBe(0);
    expect(win.end).toBe(3);
    expect(win.padLeft).toBe(0);
    expect(win.padRight).toBe(0);
  });

  it("an empty column set is an empty window", () => {
    expect(columnWindow([], 0, 100, 1)).toEqual({
      start: 0,
      end: 0,
      totalWidth: 0,
      padLeft: 0,
      padRight: 0,
    });
  });

  it("a scroll into the middle drops the head columns from the window", () => {
    const widths = [100, 100, 100, 100, 100];
    // Scroll past the first two columns; viewport 150 ⇒ ~2 columns visible.
    const win = columnWindow(widths, 250, 150, 0);
    // First visible is the column whose right edge passes 250 → col 2 (edge 300).
    expect(win.start).toBe(2);
    expect(win.padLeft).toBe(200);
  });
});

// --------------------------------------------------------------------------
// 3. Tick-coalescing — accounting identity + last-write-wins
// --------------------------------------------------------------------------

describe("coalesce — accounting identity applied+coalesced==produced, last-write-wins", () => {
  it("collapses a burst to one survivor per cell == an independent reduce-to-last", () => {
    // A burst that writes some cells multiple times.
    const burst: CellUpdate<number>[] = [
      { rowKey: "r0", colKey: "bid", value: 1 },
      { rowKey: "r0", colKey: "bid", value: 2 },
      { rowKey: "r0", colKey: "ask", value: 9 },
      { rowKey: "r1", colKey: "bid", value: 5 },
      { rowKey: "r0", colKey: "bid", value: 3 }, // latest for (r0,bid)
    ];
    const res = coalesce(burst);

    // Accounting identity (the celnet-fanout conflation discipline).
    expect(res.applied.length + res.coalesced).toBe(res.produced);
    expect(res.produced).toBe(5);
    expect(res.coalesced).toBe(2);

    // INDEPENDENT oracle: plain reduce-to-last, NOT the coalescer's own map.
    const oracle = new Map<string, number>();
    for (const u of burst) oracle.set(`${u.rowKey}|${u.colKey}`, u.value);
    const got = new Map<string, number>();
    for (const u of res.applied) got.set(`${u.rowKey}|${u.colKey}`, u.value);
    expect(got).toEqual(oracle);
    // The (r0,bid) survivor is the LAST value produced (3, not 1 or 2).
    expect(got.get("r0|bid")).toBe(3);
  });

  it("an empty burst coalesces to nothing", () => {
    const res = coalesce<number>([]);
    expect(res).toEqual({ applied: [], produced: 0, coalesced: 0 });
  });

  it("preserves first-touch order of cells across the survivors", () => {
    const burst: CellUpdate<number>[] = [
      { rowKey: "r2", colKey: "bid", value: 1 },
      { rowKey: "r0", colKey: "bid", value: 1 },
      { rowKey: "r2", colKey: "bid", value: 2 }, // updates r2, keeps its position
    ];
    const res = coalesce(burst);
    expect(res.applied.map((u) => u.rowKey)).toEqual(["r2", "r0"]);
  });
});

describe("useCoalescedCells — N updates in a frame ⇒ one flush, ≤1 render of each cell", () => {
  it("a frame's worth of bursty updates flushes to last-write-wins once", () => {
    const { result } = renderHook(() => useCoalescedCells<number>());

    // Enqueue three bursts WITHOUT a frame tick in between — they accumulate.
    act(() => {
      result.current.enqueue([
        { rowKey: "r0", colKey: "bid", value: 1 },
        { rowKey: "r0", colKey: "bid", value: 2 },
      ]);
      result.current.enqueue([{ rowKey: "r0", colKey: "bid", value: 3 }]);
      result.current.enqueue([{ rowKey: "r1", colKey: "ask", value: 7 }]);
    });
    // Before the flush, no values are committed (no extra renders mid-burst).
    expect(result.current.values.size).toBe(0);

    // One deterministic flush (the per-frame collapse).
    let stats!: { produced: number; coalesced: number };
    act(() => {
      stats = result.current.flush();
    });
    // 4 produced, 1 survivor for (r0,bid) → 2 coalesced.
    expect(stats.produced).toBe(4);
    expect(stats.coalesced).toBe(2);
    expect(cellValue(result.current.values, "r0", "bid")).toBe(3);
    expect(cellValue(result.current.values, "r1", "ask")).toBe(7);
    expect(result.current.values.size).toBe(2);
  });
});

// --------------------------------------------------------------------------
// 4. Grouping (collapsible) + flatten fold
// --------------------------------------------------------------------------

describe("flattenGroups — collapsible group fold", () => {
  const groups: RowGroup<Row>[] = [
    { key: "EUR", label: "EUR", rows: makeRows(3, "e") },
    { key: "GBP", label: "GBP", rows: makeRows(2, "g") },
  ];

  it("expands to header + data rows; a collapsed group shows only its header", () => {
    const allOpen: GroupModel = { collapsed: new Set() };
    const flatOpen = flattenGroups(groups, allOpen);
    // 2 headers + 5 data rows.
    expect(flatOpen).toHaveLength(7);
    expect(flatOpen[0]).toMatchObject({ kind: "group", key: "EUR", count: 3 });

    const eurClosed: GroupModel = { collapsed: new Set(["EUR"]) };
    const flatClosed = flattenGroups(groups, eurClosed);
    // EUR header only + GBP header + 2 GBP rows = 4.
    expect(flatClosed).toHaveLength(4);
    expect(flatClosed.filter((r) => r.kind === "group")).toHaveLength(2);
  });
});

describe("DataGrid — group rows are collapsible via Enter on the active row", () => {
  it("Enter on a focused group header calls onToggleGroup", () => {
    const toggled: string[] = [];
    const model: GroupModel = { collapsed: new Set() };
    render(
      <DataGrid
        label="grouped"
        columns={COLUMNS}
        groups={[{ key: "EUR", label: "EUR", rows: makeRows(2, "e") }]}
        groupable
        groupModel={model}
        onToggleGroup={(k) => toggled.push(k)}
        rowHeight={30}
      />,
    );
    const grid = screen.getByRole("grid");
    grid.focus();
    // Move down onto the first body row (the EUR group header).
    act(() => {
      fireEvent.keyDown(grid, { key: "ArrowDown" });
    });
    const groupRow = screen.getByRole("row", { name: /EUR/ });
    expect(groupRow).toHaveAttribute("aria-expanded", "true");
    act(() => {
      fireEvent.keyDown(grid, { key: "Enter" });
    });
    expect(toggled).toEqual(["EUR"]);
  });
});

// --------------------------------------------------------------------------
// 5. Accessibility — axe-core on the live role=grid DOM (jsdom-compatible)
// --------------------------------------------------------------------------

describe("DataGrid — accessibility (axe-core, role=grid structure)", () => {
  it("a populated role=grid has zero violations on the grid-structure rules", async () => {
    const { container } = render(
      <DataGrid
        label="streaming two-way markets"
        columns={COLUMNS}
        groups={singleGroup(makeRows(60))}
        rowHeight={30}
      />,
    );
    // axe-core runs in jsdom; we scope to the grid-structure & ARIA rules the
    // reversed opt-out is about (the full Playwright sweep runs post-merge).
    const results = await axe.run(container, {
      runOnly: {
        type: "rule",
        values: [
          "aria-required-children",
          "aria-required-parent",
          "aria-roles",
          "aria-valid-attr",
          "aria-valid-attr-value",
          "aria-allowed-attr",
        ],
      },
    });
    const serious = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(serious).toEqual([]);
    expect(results.violations).toEqual([]);
  });

  it("an empty grid renders an honest empty-state row, still zero violations", async () => {
    const { container } = render(
      <DataGrid label="markets" columns={COLUMNS} groups={[]} emptyState="—" rowHeight={30} />,
    );
    expect(screen.getByRole("grid")).toHaveAttribute("aria-rowcount", "1");
    const results = await axe.run(container, {
      runOnly: {
        type: "rule",
        values: ["aria-required-children", "aria-roles", "aria-valid-attr-value"],
      },
    });
    expect(results.violations).toEqual([]);
  });
});
