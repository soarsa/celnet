/**
 * grid.ts — the pure, DOM-free model behind the shared accessible virtualised
 * `<DataGrid>` (GW0-S2, docs/GW-FOUNDATION-PLAN.md §1). This module is the ONE
 * grid grammar every lane (Stream/Book/Cube/Risk — migrated in GW3/4/5) speaks:
 *
 *   - `ColumnDef<T>`        — a column's identity, header, accessor + presentation.
 *   - `GroupModel`          — collapsible group state (which group keys are folded).
 *   - the **roving-tabindex** reducer (`moveRoving`) — the WAI-ARIA APG "grid"
 *     keyboard model: exactly one cell is the active (tabindex=0) cell at a time,
 *     arrow keys move the active cell, Home/End jump to row ends (Ctrl variants to
 *     the grid corners), PageUp/PageDown jump by a page of rows. Pure arithmetic
 *     over a (row,col) coordinate — the component renders it onto real DOM
 *     `tabindex`/`aria-*`; the reducer never touches the DOM.
 *   - the **tick-coalescer** (`coalesce`) — collapses a burst of per-cell updates
 *     to one last-write-wins value per cell so a high-rate stream paints at most
 *     once per animation frame (the same conflate-to-latest discipline as the
 *     server-side `celnet-fanout` SPMC ring). It also reports an exact accounting
 *     pair (`produced`/`coalesced`) so callers can assert `applied + coalesced ==
 *     produced` — no silent drops.
 *
 * Everything here is referentially transparent and tested headlessly (no jsdom),
 * so the keyboard/virtualisation/coalescing logic is verifiable against
 * independent algebraic oracles, not against the component's rendered output.
 */

import type { ReactNode } from "react";

/** Horizontal alignment of a column's cells + header. */
export type ColumnAlign = "left" | "center" | "right";

/**
 * A column definition. `T` is the row datum. `accessor` derives the cell's
 * display string from the row; presentation is carried as data (no JSX here so
 * the model stays DOM-free and unit-testable).
 */
export interface ColumnDef<T> {
  /** Stable column id (used as the React key + the roving column coordinate). */
  readonly key: string;
  /** Human-readable header label. */
  readonly header: string;
  /** Optional unit suffix shown under the header (e.g. "%", "bp"). */
  readonly unit?: string;
  /**
   * Optional long-form explanation of what the column actually means, surfaced
   * as the header's tooltip. Some columns are genuinely ambiguous without one —
   * the hedge ledger's "Panel (eligible)" lists the LPs the exit policy made
   * ELIGIBLE, not the LPs that quoted or filled — and a header string short
   * enough to fit a fixed column track cannot carry that distinction.
   */
  readonly description?: string;
  /** Fixed track width in CSS px (uniform; drives column virtualisation math). */
  readonly width: number;
  /** Cell alignment. Default "right" (numeric grids are right-aligned). */
  readonly align?: ColumnAlign;
  /**
   * Derive the cell TEXT from the row datum — the column's canonical text
   * projection. It stays the single source of truth for searching, text/select
   * filtering and export even when [`ColumnDef.cell`] renders something richer,
   * so those never have to reason about markup.
   */
  readonly accessor: (row: T) => string;
  /**
   * Optional RICH cell renderer. Presentation only — it never participates in
   * matching or sorting, which is exactly why `accessor` stays mandatory beside
   * it.
   *
   * Without this the model could only ever render strings, which is the single
   * reason every blotter hand-rolled its own `<table>`: their columns carry
   * badges, provenance chips and links, not text.
   */
  readonly cell?: (row: T) => ReactNode;
  /**
   * Optional ORDERED projection for sorting and range filtering. Absent ⇒ the
   * column orders by `accessor`'s string, which is wrong for numbers ("100"
   * sorts before "9") and for dates. A `range` filter REQUIRES this.
   */
  readonly sortValue?: (row: T) => number | string;
  /**
   * Optional sort key. When present the header is an interactive sort control;
   * absent ⇒ a static header cell.
   */
  readonly sortKey?: string;
  /** Optional per-column filter control, rendered inside the header cell. */
  readonly filter?: ColumnFilter;
}

/**
 * The filter control a column offers.
 *
 * `select` derives its options from `accessor` over the UNFILTERED rows when
 * `options` is omitted, so a column gains a working dropdown for free.
 * `range` operates on `sortValue` and is meaningless without one.
 */
export type ColumnFilter =
  | { readonly kind: "text" }
  | { readonly kind: "select"; readonly options?: readonly string[] }
  | { readonly kind: "range" };

/** The live value of one column's filter. */
export type ColumnFilterValue =
  | { readonly kind: "text"; readonly query: string }
  | { readonly kind: "select"; readonly selected: string }
  | { readonly kind: "range"; readonly min: number | null; readonly max: number | null };

/** Every active column filter, keyed by `ColumnDef.key`. An absent key ⇒ unfiltered. */
export type ColumnFilterState = Readonly<Record<string, ColumnFilterValue>>;

/** A 2-D cell coordinate within the FULL (not windowed) logical grid. */
export interface CellCoord {
  /**
   * Logical row index. `-1` addresses the column-header row (APG: the header is
   * row 0 in `aria-rowindex` terms, but we model it as a distinct band so the
   * data rows stay 0-based for the windowing math).
   */
  readonly row: number;
  /** Logical column index into the (visible) columns array. */
  readonly col: number;
}

/** The roving-tabindex state: the single active cell. */
export interface RovingState {
  readonly active: CellCoord;
}

/** The keyboard intents the roving reducer understands (transport-neutral). */
export type RovingKey =
  | "ArrowUp"
  | "ArrowDown"
  | "ArrowLeft"
  | "ArrowRight"
  | "Home"
  | "End"
  | "PageUp"
  | "PageDown";

/** Geometry the roving reducer needs to clamp moves to the real grid bounds. */
export interface GridBounds {
  /** Number of DATA rows (excludes the header band). */
  readonly rowCount: number;
  /** Number of (visible) columns. */
  readonly colCount: number;
  /** Rows moved per PageUp/PageDown (a viewport's worth). Must be ≥ 1. */
  readonly pageSize: number;
}

/** The header band's logical row index (distinct from any data row). */
export const HEADER_ROW = -1;

function clamp(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}

/**
 * Apply a keyboard intent to the roving state, returning the NEW active cell
 * (clamped to grid bounds). Modifiers (`ctrl`) widen Home/End to grid corners,
 * matching the WAI-ARIA APG grid pattern. Pure: same inputs ⇒ same output.
 *
 * Row coordinates span `[HEADER_ROW, rowCount-1]`: vertical moves can land on the
 * header band (so a keyboard user reaches the sort controls) but never above it.
 */
export function moveRoving(
  state: RovingState,
  key: RovingKey,
  bounds: GridBounds,
  ctrl = false,
): RovingState {
  const { rowCount, colCount, pageSize } = bounds;
  // An empty grid (no columns) has nowhere to move; identity.
  if (colCount <= 0) return state;
  const lastRow = rowCount - 1;
  const lastCol = colCount - 1;
  const page = pageSize >= 1 ? pageSize : 1;
  const { row, col } = state.active;

  let nextRow = row;
  let nextCol = col;
  switch (key) {
    case "ArrowUp":
      nextRow = row - 1;
      break;
    case "ArrowDown":
      nextRow = row + 1;
      break;
    case "ArrowLeft":
      nextCol = col - 1;
      break;
    case "ArrowRight":
      nextCol = col + 1;
      break;
    case "Home":
      // Row start; Ctrl+Home → top-left data corner.
      nextCol = 0;
      if (ctrl) nextRow = 0;
      break;
    case "End":
      // Row end; Ctrl+End → bottom-right data corner.
      nextCol = lastCol;
      if (ctrl) nextRow = lastRow < 0 ? HEADER_ROW : lastRow;
      break;
    case "PageUp":
      nextRow = row - page;
      break;
    case "PageDown":
      nextRow = row + page;
      break;
  }

  // Clamp. The header band (HEADER_ROW) is the floor; the last data row the
  // ceiling. With zero data rows the only reachable row is the header.
  const rowFloor = HEADER_ROW;
  const rowCeil = lastRow < 0 ? HEADER_ROW : lastRow;
  return {
    active: {
      row: clamp(nextRow, rowFloor, rowCeil),
      col: clamp(nextCol, 0, lastCol),
    },
  };
}

/** Two coordinates address the same cell. */
export function sameCell(a: CellCoord, b: CellCoord): boolean {
  return a.row === b.row && a.col === b.col;
}

// ---------------------------------------------------------------------------
// Grouping
// ---------------------------------------------------------------------------

/**
 * Collapsible-group state: the set of group keys that are currently FOLDED.
 * Kept as a plain set so the model is serialisable + order-independent.
 */
export interface GroupModel {
  readonly collapsed: ReadonlySet<string>;
}

/** Empty (all groups expanded). */
export const EMPTY_GROUP_MODEL: GroupModel = { collapsed: new Set() };

/** Is this group key currently folded? */
export function isCollapsed(model: GroupModel, key: string): boolean {
  return model.collapsed.has(key);
}

/** Toggle a group key's collapsed state (returns a new model). */
export function toggleGroup(model: GroupModel, key: string): GroupModel {
  const next = new Set(model.collapsed);
  if (next.has(key)) next.delete(key);
  else next.add(key);
  return { collapsed: next };
}

/** A logical row in the flattened (grouped) view: a group header, or a datum. */
export type FlatRow<T> =
  | { readonly kind: "group"; readonly key: string; readonly label: string; readonly count: number }
  | { readonly kind: "data"; readonly key: string; readonly datum: T };

/** A group of data rows produced by the caller's grouping function. */
export interface RowGroup<T> {
  readonly key: string;
  readonly label: string;
  readonly rows: ReadonlyArray<{ readonly key: string; readonly datum: T }>;
}

/**
 * Flatten grouped data into the linear row sequence the virtualiser windows: each
 * non-empty group contributes a `group` header row, then — unless collapsed — its
 * data rows. A collapsed group shows only its header (with the member count).
 * Disjoint groups in, a flat list out: an associative, order-preserving fold the
 * windowing conservation identity is asserted against.
 */
export function flattenGroups<T>(
  groups: ReadonlyArray<RowGroup<T>>,
  model: GroupModel,
): Array<FlatRow<T>> {
  const out: Array<FlatRow<T>> = [];
  for (const g of groups) {
    out.push({ kind: "group", key: g.key, label: g.label, count: g.rows.length });
    if (!isCollapsed(model, g.key)) {
      for (const r of g.rows) out.push({ kind: "data", key: r.key, datum: r.datum });
    }
  }
  return out;
}

// ---------------------------------------------------------------------------
// Column virtualisation
// ---------------------------------------------------------------------------

/** The visible horizontal column window + spacer widths. */
export interface ColumnWindow {
  /** First column index to render (inclusive). */
  readonly start: number;
  /** One past the last column index to render (exclusive). */
  readonly end: number;
  /** Total content width in px (Σ widths). */
  readonly totalWidth: number;
  /** Left spacer width in px (Σ widths of the hidden head). */
  readonly padLeft: number;
  /** Right spacer width in px (Σ widths of the hidden tail). */
  readonly padRight: number;
}

/**
 * Compute the visible column `[start,end)` window for a horizontal scroll, padded
 * by `overscan` columns each side. Variable column widths (unlike the uniform-row
 * vertical windower) ⇒ a prefix-sum walk. Pure arithmetic; the conservation
 * identity `padLeft + Σ(rendered widths) + padRight == totalWidth` holds for any
 * scroll offset.
 */
export function columnWindow(
  widths: ReadonlyArray<number>,
  scrollLeft: number,
  viewport: number,
  overscan = 1,
): ColumnWindow {
  const n = widths.length;
  // Prefix sums: edge[i] = Σ widths[0..i). edge[n] = totalWidth.
  const edge: number[] = new Array(n + 1);
  edge[0] = 0;
  for (let i = 0; i < n; i++) edge[i + 1] = (edge[i] ?? 0) + Math.max(0, widths[i] ?? 0);
  const totalWidth = edge[n] ?? 0;
  if (n === 0) return { start: 0, end: 0, totalWidth: 0, padLeft: 0, padRight: 0 };

  const left = Math.max(0, scrollLeft);
  // Pre-measure (viewport 0): render the overscan head so the grid paints columns.
  const right = viewport > 0 ? left + viewport : edge[Math.min(n, overscan)] ?? 0;

  // First column whose right edge passes the viewport's left edge.
  let firstVisible = 0;
  while (firstVisible < n && (edge[firstVisible + 1] ?? 0) <= left) firstVisible++;
  // One past the last column whose left edge is before the viewport's right edge.
  let lastVisible = firstVisible;
  while (lastVisible < n && (edge[lastVisible] ?? 0) < right) lastVisible++;

  const start = Math.max(0, firstVisible - overscan);
  const end = Math.min(n, lastVisible + overscan);
  return {
    start,
    end,
    totalWidth,
    padLeft: edge[start] ?? 0,
    padRight: totalWidth - (edge[end] ?? 0),
  };
}

// ---------------------------------------------------------------------------
// Tick-coalescing
// ---------------------------------------------------------------------------

/** A single per-cell update: address + the new display value. */
export interface CellUpdate<V> {
  readonly rowKey: string;
  readonly colKey: string;
  readonly value: V;
}

/** The result of coalescing a burst: the survivors + an exact accounting pair. */
export interface CoalesceResult<V> {
  /** One survivor per (rowKey,colKey) — the LAST value produced for that cell. */
  readonly applied: ReadonlyArray<CellUpdate<V>>;
  /** Total updates fed in. */
  readonly produced: number;
  /** Updates superseded by a later write to the same cell (`produced - applied`). */
  readonly coalesced: number;
}

/** Address two updates target the same cell. */
function cellId(rowKey: string, colKey: string): string {
  // Tab is not a valid key fragment in our keys, so it is an unambiguous join.
  return `${rowKey}\t${colKey}`;
}

/**
 * Coalesce a burst of per-cell updates to one last-write-wins survivor per cell,
 * preserving the FIRST-touch order of each cell (so the rendered order is stable
 * across frames). Returns the survivors plus the accounting identity
 * `applied.length + coalesced == produced` — the same exact conflation accounting
 * the server-side SPMC ring guarantees (`received + skipped == produced`).
 *
 * This is the per-frame collapse: the caller buffers updates as they arrive and
 * calls `coalesce` once per animation frame, so N high-rate cell updates produce
 * at most one render of each touched cell — ≤ 1 paint per frame.
 */
export function coalesce<V>(updates: ReadonlyArray<CellUpdate<V>>): CoalesceResult<V> {
  // Insertion-ordered map keyed by cell id; a later write overwrites the value
  // but Map keeps the first-seen position, so order is stable.
  const latest = new Map<string, CellUpdate<V>>();
  for (const u of updates) {
    // Map.set on an existing key overwrites the VALUE but keeps the original
    // insertion position, so the survivor order is the first-touch order.
    latest.set(cellId(u.rowKey, u.colKey), u);
  }
  const applied = Array.from(latest.values());
  return {
    applied,
    produced: updates.length,
    coalesced: updates.length - applied.length,
  };
}

// ---------------------------------------------------------------------------
// Column filtering + sorting — the pure pipeline shared by BOTH grid bindings
// (the virtualised `role=grid` and the semantic `<table>`), so the two provably
// present the same rows in the same order from the same model.
//
// Pipeline order is fixed and load-bearing:
//     rows -> global search -> column filters -> sort
// Filtering only ever REMOVES rows (never reorders), which preserves the
// invariant `useTableFilter` documents; sorting runs last so it orders exactly
// what survived. Reversing these would make the visible order depend on the
// filter, which is the classic source of "the table jumped while I typed".
// ---------------------------------------------------------------------------

/** Case/whitespace-insensitive containment — the shared text-matching rule. */
function matchesText(haystack: string, needle: string): boolean {
  const q = needle.trim().toLowerCase();
  return q === "" || haystack.toLowerCase().includes(q);
}

/**
 * The distinct `accessor` values of a `select` column, in first-seen order.
 *
 * Derived from the rows the caller passes — which must be the UNFILTERED set,
 * otherwise choosing a value would prune the very options you could switch to.
 */
export function deriveSelectOptions<T>(rows: readonly T[], column: ColumnDef<T>): string[] {
  if (column.filter?.kind !== "select") return [];
  if (column.filter.options) return [...column.filter.options];
  const seen = new Set<string>();
  for (const row of rows) {
    const v = column.accessor(row);
    if (v !== "") seen.add(v);
  }
  return [...seen];
}

/** Whether a filter value would actually remove anything (an empty one is a no-op). */
export function isActiveFilter(value: ColumnFilterValue | undefined): boolean {
  if (!value) return false;
  switch (value.kind) {
    case "text":
      return value.query.trim() !== "";
    case "select":
      return value.selected !== "";
    case "range":
      return value.min !== null || value.max !== null;
  }
}

/** How many columns are actively filtered — the badge count on the Filters toggle. */
export function activeFilterCount(state: ColumnFilterState): number {
  return Object.values(state).filter(isActiveFilter).length;
}

/**
 * Apply every active column filter, ANDed across columns.
 *
 * A `range` filter on a column with no `sortValue` cannot be evaluated, so it is
 * IGNORED rather than silently dropping every row — a mis-declared column must
 * not look like "no matching data".
 */
export function applyColumnFilters<T>(
  rows: readonly T[],
  columns: readonly ColumnDef<T>[],
  state: ColumnFilterState,
): T[] {
  const active = columns
    .map((c) => [c, state[c.key]] as const)
    .filter(([, v]) => isActiveFilter(v));
  if (active.length === 0) return [...rows];

  return rows.filter((row) =>
    active.every(([column, value]) => {
      if (!value) return true;
      switch (value.kind) {
        case "text":
          return matchesText(column.accessor(row), value.query);
        case "select":
          return column.accessor(row) === value.selected;
        case "range": {
          if (!column.sortValue) return true;
          const raw = column.sortValue(row);
          const n = typeof raw === "number" ? raw : Number(raw);
          if (!Number.isFinite(n)) return false;
          if (value.min !== null && n < value.min) return false;
          if (value.max !== null && n > value.max) return false;
          return true;
        }
      }
    }),
  );
}

/** Sort direction. */
export type SortDirection = "asc" | "desc";

/**
 * The ONE shared collator for text ordering.
 *
 * `String.prototype.localeCompare(s, undefined, opts)` constructs a fresh
 * `Intl.Collator` on EVERY call, and a sort performs O(n log n) comparisons — on
 * a 5,000-row blotter that is ~61k collator constructions, measured at **107ms**
 * (≈6.7 dropped frames) before this was hoisted. Building the collator once
 * lifts that cost out of the comparison loop entirely; the ordering it produces
 * is identical (same locale, same options).
 */
const TEXT_COLLATOR = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

/**
 * Order `rows` by the column whose `sortKey` matches, STABLY.
 *
 * Stability matters on a streaming blotter: rows that compare equal must keep
 * their incoming order, or the table reshuffles on every tick. `Array.sort` is
 * specified stable, but ties are broken on the original index anyway so the
 * guarantee is explicit rather than inherited.
 *
 * Ordering uses `sortValue` when present (numbers compare numerically, dates by
 * ISO string) and falls back to `accessor`'s text with a `numeric` collator so
 * "9" still precedes "100" in the common numeric-text case.
 *
 * Both the projection AND its string form are computed ONCE per row (a
 * Schwartzian transform), never inside the comparator: the comparator runs
 * O(n log n) times, so any per-comparison `project()` / `String()` / collator
 * construction would multiply the cost by ~12 on a 5,000-row blotter.
 */
export function applySort<T>(
  rows: readonly T[],
  columns: readonly ColumnDef<T>[],
  sortKey: string | null,
  direction: SortDirection,
): T[] {
  const column = sortKey ? columns.find((c) => c.sortKey === sortKey) : undefined;
  if (!column) return [...rows];
  const sign = direction === "asc" ? 1 : -1;
  const project = column.sortValue ?? column.accessor;

  return rows
    .map((row, index) => {
      const key = project(row);
      const isNum = typeof key === "number";
      // `text` is only consulted on the non-numeric path, but precomputing it
      // unconditionally keeps the comparator branch-free of allocation.
      return { row, index, num: isNum ? key : Number.NaN, text: isNum ? String(key) : key, isNum };
    })
    .sort((a, b) => {
      let cmp: number;
      if (a.isNum && b.isNum) {
        // NaN sorts last in BOTH directions — an unknown is never "the biggest".
        const aNaN = Number.isNaN(a.num);
        const bNaN = Number.isNaN(b.num);
        if (aNaN || bNaN) return aNaN && bNaN ? a.index - b.index : aNaN ? 1 : -1;
        cmp = a.num - b.num;
      } else {
        cmp = TEXT_COLLATOR.compare(a.text, b.text);
      }
      return cmp === 0 ? a.index - b.index : cmp * sign;
    })
    .map((e) => e.row);
}
