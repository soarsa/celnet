/**
 * useGridState — the sort + column-filter + filter-band-visibility controller
 * shared by every table surface, and the place the pure pipeline in `lib/grid.ts`
 * is actually driven from.
 *
 * ## The pipeline order is load-bearing
 *
 *     rows → global search (useTableFilter) → column filters → sort
 *
 * The caller owns the FIRST stage (`useTableFilter` + `<TableSearch>` already
 * exist and several blotters use them), hands the searched rows in as `rows`,
 * and hands the pre-search set in as `allRows`. This hook runs the remaining two
 * stages. Keeping the split explicit is what makes the "N of M" count honest:
 * `shown` is the count AFTER BOTH stages, `total` is the pre-search population,
 * so a trader who types a query AND sets a column filter sees the real survivor
 * count rather than the search-only one.
 *
 * `allRows` also feeds `deriveSelectOptions`: a `select` column's options MUST be
 * derived from the unfiltered set, or choosing a value would prune away the very
 * options you could switch to (pick "Citadel" and Citadel becomes the only
 * option — a one-way door).
 *
 * ## Persistence
 *
 * State is persisted through the EXISTING {@link useTableUiState} store — the
 * same module-level store that already survives the Shell's conditional
 * workspace unmount (see `RiskDashboardWorkspace.tsx:339-348`). There is
 * deliberately no second store.
 *
 * It persists under a DERIVED id (`<tableId>:grid`), not `tableId` itself,
 * because `useTableUiState` writes a whole merged object per id: two hooks
 * sharing one id with different initial shapes would let whichever writes first
 * knock the other's keys out of the snapshot. A blotter that already persists
 * `{ lens, query }` under `"fi-deals-blotter"` therefore keeps that entry
 * untouched and gains a disjoint `"fi-deals-blotter:grid"` entry.
 *
 * ## Cost
 *
 * Measured over 5,000 rows × 16 columns (`test/gridPipelineBench.test.ts`):
 * column filtering with 13 filters active is ~0.5ms and the idle path ~0.00ms,
 * so per-column predicate memoisation would be pure ceremony. The one real cost
 * found was text sorting (107ms — a per-comparison `Intl.Collator` construction
 * inside `localeCompare`), fixed at source in `applySort` by hoisting one shared
 * collator; it is now ~1.7ms. Everything here is memoised on identity so a
 * stream tick that does not change the row array does no work at all.
 */

import { useCallback, useMemo } from "react";

import {
  activeFilterCount,
  applyColumnFilters,
  applySort,
  deriveSelectOptions,
  isActiveFilter,
  type ColumnDef,
  type ColumnFilterState,
  type ColumnFilterValue,
  type SortDirection,
} from "../lib/grid";
import { useTableUiState } from "./useTableUiState";

/** The persisted, serialisable UI state of one table. */
export interface GridState {
  /** The active `ColumnDef.sortKey`, or `null` for the caller's natural order. */
  readonly sortKey: string | null;
  /** Direction of the active sort. Meaningless while `sortKey` is null. */
  readonly sortDir: SortDirection;
  /** Every column's live filter value, keyed by `ColumnDef.key`. */
  readonly filters: ColumnFilterState;
  /** Whether the per-column filter band is revealed. */
  readonly filtersOpen: boolean;
}

/** The state + derived rows + commands a `<DataTable>` (or any binding) consumes. */
export interface GridController<T> {
  readonly state: GridState;
  /** Rows after column filters AND sort — what the table renders. */
  readonly rows: readonly T[];
  /** Survivors after BOTH the caller's global search and the column filters. */
  readonly shown: number;
  /** The pre-search population — the "M" in "N of M". */
  readonly total: number;
  /** How many columns are actively filtered (the Filters badge count). */
  readonly activeFilters: number;
  /** The `select` options for a column, derived from the UNFILTERED rows. */
  readonly optionsFor: (columnKey: string) => readonly string[];
  /** Cycle a column's sort: none → asc → desc → none. */
  readonly toggleSort: (sortKey: string) => void;
  /** Set (or clear, with `null`) one column's filter value. */
  readonly setFilter: (columnKey: string, value: ColumnFilterValue | null) => void;
  /** Clear every column filter at once (leaves the band open). */
  readonly clearFilters: () => void;
  /** Show/hide the per-column filter band. */
  readonly setFiltersOpen: (open: boolean) => void;
}

/** The empty filter value for a column, used when a control is first touched. */
export function emptyFilterValue(column: ColumnDef<unknown>): ColumnFilterValue {
  const kind = column.filter?.kind ?? "text";
  if (kind === "select") return { kind: "select", selected: "" };
  if (kind === "range") return { kind: "range", min: null, max: null };
  return { kind: "text", query: "" };
}

export interface UseGridStateArgs<T> {
  /** Stable id for persistence, e.g. `"fi-deals-blotter"`. */
  readonly tableId: string;
  /** The column model (drives which filters exist and how sorting projects). */
  readonly columns: readonly ColumnDef<T>[];
  /** Rows AFTER the caller's global search — stage 2/3 of the pipeline. */
  readonly rows: readonly T[];
  /**
   * Rows BEFORE the global search. Drives `total` and the `select` option
   * derivation. Defaults to `rows` when a surface has no global search.
   */
  readonly allRows?: readonly T[];
  /** Initial sort applied on the very first mount for this `tableId`. */
  readonly initialSort?: { readonly key: string; readonly dir: SortDirection } | undefined;
}

export function useGridState<T>({
  tableId,
  columns,
  rows,
  allRows,
  initialSort,
}: UseGridStateArgs<T>): GridController<T> {
  const population = allRows ?? rows;

  // Persisted under a DERIVED id so it cannot collide with the lens/query entry
  // a blotter may already keep under the bare `tableId` (see the module doc).
  const [state, patch] = useTableUiState<GridState>(`${tableId}:grid`, {
    sortKey: initialSort?.key ?? null,
    sortDir: initialSort?.dir ?? "asc",
    filters: {},
    filtersOpen: false,
  });

  const filtered = useMemo(
    () => applyColumnFilters(rows, columns, state.filters),
    [rows, columns, state.filters],
  );

  const sorted = useMemo(
    () => applySort(filtered, columns, state.sortKey, state.sortDir),
    [filtered, columns, state.sortKey, state.sortDir],
  );

  // Options are derived from the UNFILTERED population, once per column, and
  // only for columns that actually declare a `select` filter.
  const options = useMemo(() => {
    const map = new Map<string, readonly string[]>();
    for (const c of columns) {
      if (c.filter?.kind === "select") map.set(c.key, deriveSelectOptions(population, c));
    }
    return map;
  }, [columns, population]);

  const optionsFor = useCallback(
    (columnKey: string): readonly string[] => options.get(columnKey) ?? [],
    [options],
  );

  const toggleSort = useCallback(
    (sortKey: string) => {
      // none → asc → desc → none. The third press RESTORES the caller's natural
      // order (newest-first on a blotter), which is otherwise unreachable once
      // a trader has sorted — a two-state toggle traps them in a sorted view.
      if (state.sortKey !== sortKey) {
        patch({ sortKey, sortDir: "asc" });
        return;
      }
      if (state.sortDir === "asc") {
        patch({ sortKey, sortDir: "desc" });
        return;
      }
      patch({ sortKey: null, sortDir: "asc" });
    },
    [patch, state.sortKey, state.sortDir],
  );

  const setFilter = useCallback(
    (columnKey: string, value: ColumnFilterValue | null) => {
      const next: Record<string, ColumnFilterValue> = { ...state.filters };
      // An inactive value is DELETED rather than stored, so `activeFilterCount`
      // and the badge stay truthful without every consumer re-checking emptiness.
      if (value === null || !isActiveFilter(value)) delete next[columnKey];
      else next[columnKey] = value;
      patch({ filters: next });
    },
    [patch, state.filters],
  );

  const clearFilters = useCallback(() => patch({ filters: {} }), [patch]);
  const setFiltersOpen = useCallback((open: boolean) => patch({ filtersOpen: open }), [patch]);

  return {
    state,
    rows: sorted,
    shown: sorted.length,
    total: population.length,
    activeFilters: activeFilterCount(state.filters),
    optionsFor,
    toggleSort,
    setFilter,
    clearFilters,
    setFiltersOpen,
  };
}
