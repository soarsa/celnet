/**
 * DataTable / useGridState tests — the SEMANTIC binding of the shared grid model
 * (`lib/grid.ts`) and the controller that drives it.
 *
 * Each suite is gated against an oracle that can DISAGREE with the
 * implementation, rather than restating it:
 *
 *   - the pure pipeline → hand-computed expected row sets and an independent
 *     `Array.prototype.sort` reference, not a replay of `applySort`'s own
 *     comparator;
 *   - pipeline ORDER → asserted through an observable consequence (the "N of M"
 *     count after BOTH stages), because order is exactly what a refactor breaks
 *     silently;
 *   - the sticky/fixed-layout contract → asserted on the rendered DOM STRUCTURE
 *     (one header `<tr>`, a `<colgroup>` whose widths match the model), since
 *     jsdom computes no layout and a screenshot cannot be a unit test;
 *   - accessibility → third-party axe-core on the live `<table>` DOM.
 *
 * WHY these behaviours matter, concretely:
 *   - ONE header row: a second `<tr>` of filters needs its sticky `top:` measured
 *     against the first row's rendered height — the classic multi-row sticky
 *     failure. The structural assertion is what stops that regressing.
 *   - a generated `<colgroup>` + `table-layout: fixed`: without it the browser
 *     re-solves column widths from cell content on EVERY render, so columns
 *     jitter as prices tick. Nothing in the GUI set these before this component.
 *   - persistence via the EXISTING `useTableUiState` store under a DERIVED id:
 *     a blotter that already persists `{ lens, query }` under its bare tableId
 *     must not have those keys knocked out by the grid state.
 */
import { act } from "react";
import axe from "axe-core";
import { afterEach, describe, expect, it } from "vitest";
import { fireEvent, render, renderHook, screen, within } from "@testing-library/react";

import { DataTable } from "../src/components/DataTable";
import { useGridState } from "../src/hooks/useGridState";
import { __resetTableUiStore, useTableUiState } from "../src/hooks/useTableUiState";
import {
  activeFilterCount,
  applyColumnFilters,
  applySort,
  deriveSelectOptions,
  isActiveFilter,
  type ColumnDef,
  type ColumnFilterState,
} from "../src/lib/grid";

// --------------------------------------------------------------------------
// Fixtures
// --------------------------------------------------------------------------

interface Deal {
  readonly id: string;
  readonly cpty: string;
  readonly ccy: string;
  readonly notional: number;
  readonly kind: string;
}

const DEALS: readonly Deal[] = [
  { id: "d1", cpty: "Citadel", ccy: "USD", notional: 100, kind: "RFQ" },
  { id: "d2", cpty: "Balyasny", ccy: "EUR", notional: 9, kind: "IOI" },
  { id: "d3", cpty: "Point72", ccy: "USD", notional: 50, kind: "RFQ" },
  { id: "d4", cpty: "Millennium", ccy: "GBP", notional: 1000, kind: "RFS" },
];

const COLUMNS: ReadonlyArray<ColumnDef<Deal>> = [
  {
    key: "cpty",
    header: "Counterparty",
    width: 140,
    align: "left",
    accessor: (d) => d.cpty,
    sortKey: "cpty",
    filter: { kind: "text" },
  },
  {
    key: "ccy",
    header: "Ccy",
    width: 60,
    align: "left",
    accessor: (d) => d.ccy,
    sortKey: "ccy",
    filter: { kind: "select" },
  },
  {
    key: "notional",
    header: "Notional",
    width: 110,
    align: "right",
    accessor: (d) => d.notional.toString(),
    sortValue: (d) => d.notional,
    sortKey: "notional",
    filter: { kind: "range" },
  },
  {
    key: "kind",
    header: "Type",
    width: 70,
    align: "left",
    // A RICH cell — the whole reason the model carries `cell` beside `accessor`.
    accessor: (d) => d.kind,
    cell: (d) => <span data-testid={`kind-badge-${d.id}`}>{d.kind}</span>,
  },
];

afterEach(() => {
  __resetTableUiStore();
});

/** Mount a DataTable driven by a real `useGridState` over `rows`. */
function Harness({
  rows,
  allRows,
  tableId = "t",
  columns = COLUMNS,
}: {
  rows?: readonly Deal[];
  allRows?: readonly Deal[];
  tableId?: string;
  columns?: ReadonlyArray<ColumnDef<Deal>>;
}): React.ReactElement {
  const grid = useGridState<Deal>({
    tableId,
    columns,
    rows: rows ?? DEALS,
    ...(allRows ? { allRows } : {}),
  });
  return (
    <DataTable
      label="Deals"
      columns={columns}
      grid={grid}
      rowKey={(d) => d.id}
      caption="Received deals"
      emptyState="No deals match the current filters."
    />
  );
}

// --------------------------------------------------------------------------
// 1. The pure pipeline — hand-computed oracles
// --------------------------------------------------------------------------

describe("applyColumnFilters — ANDs active filters, ignores inert ones", () => {
  it("AND-combines across columns", () => {
    const state: ColumnFilterState = {
      ccy: { kind: "select", selected: "USD" },
      kind: { kind: "text", query: "rfq" },
    };
    // Hand-computed: USD ∧ RFQ ⇒ d1, d3.
    expect(applyColumnFilters(DEALS, COLUMNS, state).map((d) => d.id)).toEqual(["d1", "d3"]);
  });

  it("an empty filter value is a no-op, not a zero-row filter", () => {
    const state: ColumnFilterState = {
      cpty: { kind: "text", query: "   " },
      ccy: { kind: "select", selected: "" },
      notional: { kind: "range", min: null, max: null },
    };
    expect(applyColumnFilters(DEALS, COLUMNS, state)).toHaveLength(DEALS.length);
    expect(activeFilterCount(state)).toBe(0);
  });

  it("a range filter is inclusive at both bounds and uses sortValue, not text", () => {
    // Text ordering would put "1000" between "100" and "50"; the ORDERED
    // projection is what makes the bound mean the number.
    const state: ColumnFilterState = { notional: { kind: "range", min: 50, max: 100 } };
    expect(applyColumnFilters(DEALS, COLUMNS, state).map((d) => d.id)).toEqual(["d1", "d3"]);
  });

  it("a range filter on a column with no sortValue is IGNORED, not row-annihilating", () => {
    // A mis-declared column must not masquerade as "no matching data" — that
    // sends a trader hunting for missing deals that were never filtered out.
    const noOrder: ReadonlyArray<ColumnDef<Deal>> = [
      { key: "cpty", header: "Counterparty", width: 100, accessor: (d) => d.cpty, filter: { kind: "range" } },
    ];
    const state: ColumnFilterState = { cpty: { kind: "range", min: 0, max: 1 } };
    expect(applyColumnFilters(DEALS, noOrder, state)).toHaveLength(DEALS.length);
  });

  it("text matching is case- and whitespace-insensitive", () => {
    const state: ColumnFilterState = { cpty: { kind: "text", query: "  CITA " } };
    expect(applyColumnFilters(DEALS, COLUMNS, state).map((d) => d.id)).toEqual(["d1"]);
  });
});

describe("deriveSelectOptions — derived from the UNFILTERED set", () => {
  it("returns distinct accessor values in first-seen order", () => {
    const col = COLUMNS.find((c) => c.key === "ccy") as ColumnDef<Deal>;
    expect(deriveSelectOptions(DEALS, col)).toEqual(["USD", "EUR", "GBP"]);
  });

  it("a non-select column offers no options", () => {
    const col = COLUMNS.find((c) => c.key === "cpty") as ColumnDef<Deal>;
    expect(deriveSelectOptions(DEALS, col)).toEqual([]);
  });

  it("explicit options win over derivation (a fixed domain, e.g. a state enum)", () => {
    const col: ColumnDef<Deal> = {
      key: "ccy",
      header: "Ccy",
      width: 60,
      accessor: (d) => d.ccy,
      filter: { kind: "select", options: ["USD", "EUR", "JPY"] },
    };
    // JPY is offered even though no row carries it — the domain is declared.
    expect(deriveSelectOptions(DEALS, col)).toEqual(["USD", "EUR", "JPY"]);
  });
});

describe("applySort — stable, NaN-last, ordered by sortValue", () => {
  it("orders numerically via sortValue, NOT lexically via accessor text", () => {
    // The oracle: an INDEPENDENT numeric sort of the raw field.
    const oracle = [...DEALS].sort((a, b) => a.notional - b.notional).map((d) => d.id);
    expect(applySort(DEALS, COLUMNS, "notional", "asc").map((d) => d.id)).toEqual(oracle);
    // Lexical order would be 100, 1000, 50, 9 — prove we are not doing that.
    expect(applySort(DEALS, COLUMNS, "notional", "asc").map((d) => d.notional)).toEqual([
      9, 50, 100, 1000,
    ]);
  });

  it("is STABLE: equal keys keep their incoming order in both directions", () => {
    // A streaming blotter that reshuffles ties on every tick is unusable.
    const tied: Deal[] = [
      { id: "a", cpty: "X", ccy: "USD", notional: 5, kind: "RFQ" },
      { id: "b", cpty: "X", ccy: "USD", notional: 5, kind: "RFQ" },
      { id: "c", cpty: "X", ccy: "USD", notional: 5, kind: "RFQ" },
    ];
    expect(applySort(tied, COLUMNS, "notional", "asc").map((d) => d.id)).toEqual(["a", "b", "c"]);
    expect(applySort(tied, COLUMNS, "notional", "desc").map((d) => d.id)).toEqual(["a", "b", "c"]);
  });

  it("sorts NaN LAST in BOTH directions — an unknown is never 'the biggest'", () => {
    const withNaN: Deal[] = [
      { id: "n", cpty: "N", ccy: "USD", notional: Number.NaN, kind: "RFQ" },
      { id: "lo", cpty: "L", ccy: "USD", notional: 1, kind: "RFQ" },
      { id: "hi", cpty: "H", ccy: "USD", notional: 9, kind: "RFQ" },
    ];
    expect(applySort(withNaN, COLUMNS, "notional", "asc").map((d) => d.id)).toEqual([
      "lo",
      "hi",
      "n",
    ]);
    expect(applySort(withNaN, COLUMNS, "notional", "desc").map((d) => d.id)).toEqual([
      "hi",
      "lo",
      "n",
    ]);
  });

  it("text ordering is numeric-aware ('9' before '100') and matches a collator oracle", () => {
    const rows: Deal[] = [
      { id: "a", cpty: "Book 100", ccy: "USD", notional: 0, kind: "RFQ" },
      { id: "b", cpty: "Book 9", ccy: "USD", notional: 0, kind: "RFQ" },
      { id: "c", cpty: "Book 20", ccy: "USD", notional: 0, kind: "RFQ" },
    ];
    // INDEPENDENT oracle: a freshly-built collator, not the module's hoisted one.
    const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
    const oracle = [...rows].sort((a, b) => collator.compare(a.cpty, b.cpty)).map((d) => d.id);
    expect(applySort(rows, COLUMNS, "cpty", "asc").map((d) => d.id)).toEqual(oracle);
    expect(applySort(rows, COLUMNS, "cpty", "asc").map((d) => d.cpty)).toEqual([
      "Book 9",
      "Book 20",
      "Book 100",
    ]);
  });

  it("an unknown sort key leaves the caller's natural order untouched", () => {
    expect(applySort(DEALS, COLUMNS, "nope", "asc").map((d) => d.id)).toEqual(
      DEALS.map((d) => d.id),
    );
    expect(applySort(DEALS, COLUMNS, null, "asc").map((d) => d.id)).toEqual(DEALS.map((d) => d.id));
  });
});

describe("isActiveFilter / activeFilterCount", () => {
  it("counts only filters that would actually remove rows", () => {
    expect(isActiveFilter(undefined)).toBe(false);
    expect(isActiveFilter({ kind: "text", query: "  " })).toBe(false);
    expect(isActiveFilter({ kind: "text", query: "a" })).toBe(true);
    expect(isActiveFilter({ kind: "select", selected: "" })).toBe(false);
    expect(isActiveFilter({ kind: "range", min: null, max: null })).toBe(false);
    // A one-sided bound is still a real filter.
    expect(isActiveFilter({ kind: "range", min: 0, max: null })).toBe(true);
    expect(
      activeFilterCount({
        a: { kind: "text", query: "x" },
        b: { kind: "text", query: "" },
        c: { kind: "range", min: null, max: 5 },
      }),
    ).toBe(2);
  });
});

// --------------------------------------------------------------------------
// 2. useGridState — pipeline order, counts, persistence, sort cycle
// --------------------------------------------------------------------------

describe("useGridState — the pipeline order is search → column filters → sort", () => {
  it("'N of M' counts AFTER BOTH stages, with M the pre-search population", () => {
    // The caller has already globally searched DEALS down to the two USD rows.
    const searched = DEALS.filter((d) => d.ccy === "USD");
    const { result } = renderHook(() =>
      useGridState<Deal>({ tableId: "p", columns: COLUMNS, rows: searched, allRows: DEALS }),
    );
    expect(result.current.shown).toBe(2);
    expect(result.current.total).toBe(4);

    // Now a column filter narrows further — `shown` must reflect BOTH stages.
    act(() => result.current.setFilter("cpty", { kind: "text", query: "Point" }));
    expect(result.current.shown).toBe(1);
    expect(result.current.total).toBe(4);
    expect(result.current.rows.map((d) => d.id)).toEqual(["d3"]);
  });

  it("select options come from the UNFILTERED population, so the choice is not a one-way door", () => {
    const searched = DEALS.filter((d) => d.ccy === "USD");
    const { result } = renderHook(() =>
      useGridState<Deal>({ tableId: "p2", columns: COLUMNS, rows: searched, allRows: DEALS }),
    );
    // Even though only USD rows are visible, every currency stays switchable.
    expect(result.current.optionsFor("ccy")).toEqual(["USD", "EUR", "GBP"]);
  });

  it("filtering runs BEFORE sorting: the sorted output is exactly the survivors, ordered", () => {
    const { result } = renderHook(() =>
      useGridState<Deal>({ tableId: "p3", columns: COLUMNS, rows: DEALS }),
    );
    act(() => result.current.setFilter("ccy", { kind: "select", selected: "USD" }));
    act(() => result.current.toggleSort("notional"));
    expect(result.current.rows.map((d) => d.id)).toEqual(["d3", "d1"]); // 50 then 100
  });
});

describe("useGridState — sort cycles none → asc → desc → none", () => {
  it("the third press RESTORES the caller's natural order", () => {
    // Without a third state a trader can never get back to newest-first, which
    // is the order a blotter is actually read in.
    const { result } = renderHook(() =>
      useGridState<Deal>({ tableId: "s", columns: COLUMNS, rows: DEALS }),
    );
    expect(result.current.state.sortKey).toBeNull();
    act(() => result.current.toggleSort("notional"));
    expect(result.current.state).toMatchObject({ sortKey: "notional", sortDir: "asc" });
    act(() => result.current.toggleSort("notional"));
    expect(result.current.state).toMatchObject({ sortKey: "notional", sortDir: "desc" });
    act(() => result.current.toggleSort("notional"));
    expect(result.current.state.sortKey).toBeNull();
    expect(result.current.rows.map((d) => d.id)).toEqual(DEALS.map((d) => d.id));
  });

  it("switching column starts that column at ascending", () => {
    const { result } = renderHook(() =>
      useGridState<Deal>({ tableId: "s2", columns: COLUMNS, rows: DEALS }),
    );
    act(() => result.current.toggleSort("notional"));
    act(() => result.current.toggleSort("notional")); // desc
    act(() => result.current.toggleSort("cpty"));
    expect(result.current.state).toMatchObject({ sortKey: "cpty", sortDir: "asc" });
  });
});

describe("useGridState — persistence through the EXISTING useTableUiState store", () => {
  it("sort + filters survive an unmount and are restored on remount", () => {
    // The Shell mounts workspaces conditionally, so a table's local useState is
    // destroyed on a tab switch. This is the behaviour that stops a trader's
    // filter being silently discarded when they glance at another tab.
    const first = renderHook(() =>
      useGridState<Deal>({ tableId: "persist", columns: COLUMNS, rows: DEALS }),
    );
    act(() => first.result.current.toggleSort("cpty"));
    act(() => first.result.current.setFilter("ccy", { kind: "select", selected: "USD" }));
    act(() => first.result.current.setFiltersOpen(true));
    first.unmount();

    const second = renderHook(() =>
      useGridState<Deal>({ tableId: "persist", columns: COLUMNS, rows: DEALS }),
    );
    expect(second.result.current.state.sortKey).toBe("cpty");
    expect(second.result.current.state.filtersOpen).toBe(true);
    expect(second.result.current.activeFilters).toBe(1);
    expect(second.result.current.rows.map((d) => d.id)).toEqual(["d1", "d3"]);
  });

  it("does NOT clobber a sibling entry a blotter already keeps under the bare tableId", () => {
    // Deals persists `{ lens, query }` under "fi-deals-blotter". `useTableUiState`
    // writes a whole merged object per id, so sharing the id would let whichever
    // hook writes first knock the other's keys out of the snapshot. The grid
    // state therefore lives under a DERIVED id.
    const lens = renderHook(() =>
      useTableUiState("fi-deals-blotter", { lens: "client", query: "citadel" }),
    );
    const grid = renderHook(() =>
      useGridState<Deal>({ tableId: "fi-deals-blotter", columns: COLUMNS, rows: DEALS }),
    );
    act(() => grid.result.current.toggleSort("cpty"));
    act(() => grid.result.current.setFilter("ccy", { kind: "select", selected: "EUR" }));

    // The lens/query entry is intact.
    expect(lens.result.current[0]).toEqual({ lens: "client", query: "citadel" });
    // …and the grid entry is intact alongside it.
    expect(grid.result.current.state.sortKey).toBe("cpty");
    expect(grid.result.current.activeFilters).toBe(1);
  });

  it("an emptied filter is DELETED, so the badge count stays truthful", () => {
    const { result } = renderHook(() =>
      useGridState<Deal>({ tableId: "d", columns: COLUMNS, rows: DEALS }),
    );
    act(() => result.current.setFilter("cpty", { kind: "text", query: "cit" }));
    expect(result.current.activeFilters).toBe(1);
    // Clearing the box must drop the entry, not store an inert one.
    act(() => result.current.setFilter("cpty", { kind: "text", query: "" }));
    expect(result.current.activeFilters).toBe(0);
    expect(result.current.state.filters).toEqual({});
  });

  it("clearFilters drops every filter but leaves sort and band visibility alone", () => {
    const { result } = renderHook(() =>
      useGridState<Deal>({ tableId: "c", columns: COLUMNS, rows: DEALS }),
    );
    act(() => result.current.setFiltersOpen(true));
    act(() => result.current.toggleSort("cpty"));
    act(() => result.current.setFilter("ccy", { kind: "select", selected: "USD" }));
    act(() => result.current.clearFilters());
    expect(result.current.activeFilters).toBe(0);
    expect(result.current.state.filtersOpen).toBe(true);
    expect(result.current.state.sortKey).toBe("cpty");
    expect(result.current.rows).toHaveLength(4);
  });
});

// --------------------------------------------------------------------------
// 3. DataTable — the sticky/fixed-layout structural contract
// --------------------------------------------------------------------------

describe("DataTable — structure the sticky header and stable widths depend on", () => {
  it("renders EXACTLY ONE header row, even with the filter band open", () => {
    // A second <tr> of filters would need its sticky `top:` measured against the
    // first row's rendered height — the multi-row sticky failure this shape
    // exists to avoid. One band, one top: 0, nothing to measure.
    render(<Harness />);
    const head = screen.getByRole("table").querySelector("thead") as HTMLElement;
    expect(within(head).getAllByRole("row")).toHaveLength(1);

    fireEvent.click(screen.getByTestId("datatable-filters-toggle"));
    expect(within(head).getAllByRole("row")).toHaveLength(1);
    // …and the controls really are inside the header cells.
    const th = within(head).getAllByRole("columnheader")[0] as HTMLElement;
    expect(within(th).getByLabelText("Filter Counterparty")).toBeInTheDocument();
  });

  it("generates a <colgroup> whose widths mirror the column model", () => {
    // Without this + table-layout:fixed the browser re-solves widths from cell
    // CONTENT on every render, so columns jitter as values tick.
    render(<Harness />);
    const cols = Array.from(screen.getByRole("table").querySelectorAll("colgroup > col"));
    expect(cols).toHaveLength(COLUMNS.length);
    expect(cols.map((c) => (c as HTMLElement).style.width)).toEqual(
      COLUMNS.map((c) => `${c.width}px`),
    );
  });

  it("every header is a scoped <th> and carries aria-sort only when sortable", () => {
    render(<Harness />);
    const headers = screen.getAllByRole("columnheader");
    for (const h of headers) expect(h).toHaveAttribute("scope", "col");
    // Sortable columns advertise "none" until sorted; the un-sortable one omits
    // the attribute entirely rather than lying with "none".
    expect(headers[0]).toHaveAttribute("aria-sort", "none");
    expect(headers[3]).not.toHaveAttribute("aria-sort");
  });

  it("clicking a sort header updates aria-sort through the ascending/descending cycle", () => {
    render(<Harness />);
    const notional = screen.getAllByRole("columnheader")[2] as HTMLElement;
    fireEvent.click(screen.getByTestId("datatable-sort-notional"));
    expect(notional).toHaveAttribute("aria-sort", "ascending");
    fireEvent.click(screen.getByTestId("datatable-sort-notional"));
    expect(notional).toHaveAttribute("aria-sort", "descending");
    fireEvent.click(screen.getByTestId("datatable-sort-notional"));
    expect(notional).toHaveAttribute("aria-sort", "none");
  });

  it("renders an optional <caption> and an optional <tfoot>", () => {
    function WithFooter(): React.ReactElement {
      const grid = useGridState<Deal>({ tableId: "f", columns: COLUMNS, rows: DEALS });
      return (
        <DataTable
          label="Deals"
          columns={COLUMNS}
          grid={grid}
          rowKey={(d) => d.id}
          caption="Received deals"
          footer={
            <tr>
              <td colSpan={COLUMNS.length}>Total 1159</td>
            </tr>
          }
        />
      );
    }
    render(<WithFooter />);
    const table = screen.getByRole("table");
    expect(table.querySelector("caption")?.textContent).toBe("Received deals");
    expect(within(table.querySelector("tfoot") as HTMLElement).getByText("Total 1159")).toBeInTheDocument();
  });
});

describe("DataTable — rich cells, filters and the empty state", () => {
  it("renders the model's RICH cell, not the text accessor, when `cell` is present", () => {
    // Downgrading a badge to plain text is exactly the regression the `cell` API
    // exists to prevent, so it is asserted structurally.
    render(<Harness />);
    expect(screen.getByTestId("kind-badge-d1")).toHaveTextContent("RFQ");
    expect(screen.getByTestId("kind-badge-d4")).toHaveTextContent("RFS");
  });

  it("the filter band is hidden until toggled, and the badge counts active filters", () => {
    render(<Harness />);
    const toggle = screen.getByTestId("datatable-filters-toggle");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByLabelText("Filter Counterparty")).toBeNull();

    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    fireEvent.change(screen.getByLabelText("Filter Counterparty"), {
      target: { value: "Cita" },
    });
    // The count is spelled out for AT, not left as a bare numeric glyph.
    expect(screen.getByTestId("datatable-filters-toggle")).toHaveAccessibleName(
      "Filters, 1 active column filter",
    );
    expect(screen.getAllByRole("row")).toHaveLength(2); // header + 1 survivor
  });

  it("a select filter offers every value in the population plus an 'All' escape", () => {
    render(<Harness />);
    fireEvent.click(screen.getByTestId("datatable-filters-toggle"));
    const select = screen.getByLabelText("Filter Ccy") as HTMLSelectElement;
    expect(Array.from(select.options).map((o) => o.value)).toEqual(["", "USD", "EUR", "GBP"]);
    fireEvent.change(select, { target: { value: "GBP" } });
    expect(screen.getAllByRole("row")).toHaveLength(2);
    expect(screen.getByText("Millennium")).toBeInTheDocument();
  });

  it("a range filter labels BOTH bounds distinctly, and an empty bound means unbounded", () => {
    render(<Harness />);
    fireEvent.click(screen.getByTestId("datatable-filters-toggle"));
    const min = screen.getByLabelText("Filter Notional minimum");
    const max = screen.getByLabelText("Filter Notional maximum");
    fireEvent.change(min, { target: { value: "50" } });
    // Only min set ⇒ max is unbounded, NOT Number("") === 0 (which would drop
    // every row and read as "no matching data").
    expect(screen.getAllByRole("row")).toHaveLength(4); // header + 50,100,1000
    fireEvent.change(max, { target: { value: "100" } });
    expect(screen.getAllByRole("row")).toHaveLength(3); // header + 50,100
  });

  it("'Clear filters' appears only while something is filtered and resets every column", () => {
    render(<Harness />);
    fireEvent.click(screen.getByTestId("datatable-filters-toggle"));
    expect(screen.queryByTestId("datatable-clear-filters")).toBeNull();
    fireEvent.change(screen.getByLabelText("Filter Counterparty"), { target: { value: "Cita" } });
    fireEvent.click(screen.getByTestId("datatable-clear-filters"));
    expect(screen.getAllByRole("row")).toHaveLength(DEALS.length + 1);
    expect(screen.queryByTestId("datatable-clear-filters")).toBeNull();
  });

  it("shows an HONEST empty state spanning the full width when nothing survives", () => {
    render(<Harness />);
    fireEvent.click(screen.getByTestId("datatable-filters-toggle"));
    fireEvent.change(screen.getByLabelText("Filter Counterparty"), {
      target: { value: "no-such-counterparty" },
    });
    const cell = screen.getByText("No deals match the current filters.");
    expect(cell).toHaveAttribute("colspan", String(COLUMNS.length));
  });

  it("passes row attributes through so a migrated blotter keeps its handlers and testids", () => {
    const opened: string[] = [];
    function WithRowProps(): React.ReactElement {
      const grid = useGridState<Deal>({ tableId: "rp", columns: COLUMNS, rows: DEALS });
      return (
        <DataTable
          label="Deals"
          columns={COLUMNS}
          grid={grid}
          rowKey={(d) => d.id}
          rowProps={(d) => ({
            "data-testid": `deal-row-${d.id}`,
            onContextMenu: () => opened.push(d.id),
          })}
        />
      );
    }
    render(<WithRowProps />);
    const row = screen.getByTestId("deal-row-d2");
    expect(row.tagName).toBe("TR");
    fireEvent.contextMenu(row);
    expect(opened).toEqual(["d2"]);
    // The row is still a real <tr> of <td>s — `closest("tr")` and cell queries
    // (which several existing blotter tests rely on) keep working.
    expect(within(row).getAllByRole("cell")).toHaveLength(COLUMNS.length);
  });
});

// --------------------------------------------------------------------------
// 4. Accessibility — axe-core on the live <table> DOM
// --------------------------------------------------------------------------

describe("DataTable — accessibility (axe-core on the real table DOM)", () => {
  it("has zero violations with the filter band open (labels, table structure, ARIA)", async () => {
    const { container } = render(<Harness />);
    fireEvent.click(screen.getByTestId("datatable-filters-toggle"));
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
          "aria-allowed-role",
          "label",
          "form-field-multiple-labels",
          "th-has-data-cells",
          "td-headers-attr",
          "landmark-unique",
          "empty-table-header",
        ],
      },
    });
    expect(results.violations).toEqual([]);
  });

  it("an empty table is still violation-free", async () => {
    const { container } = render(<Harness rows={[]} allRows={[]} />);
    const results = await axe.run(container, {
      runOnly: {
        type: "rule",
        values: ["aria-required-children", "aria-roles", "aria-valid-attr-value", "empty-table-header"],
      },
    });
    expect(results.violations).toEqual([]);
  });
});
