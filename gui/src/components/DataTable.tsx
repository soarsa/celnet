/**
 * DataTable — the SEMANTIC binding of the shared grid model (`lib/grid.ts`).
 *
 * It is NOT a replacement for {@link DataGrid}. The two are deliberate siblings
 * over the ONE model, so they provably present the same rows in the same order
 * from the same `ColumnDef`s:
 *
 *   - `<DataGrid>` is the VIRTUALISED `role=grid` binding, for unbounded
 *     streaming surfaces where only a window of rows may be in the DOM.
 *   - `<DataTable>` (this file) is the real `<table>` binding, for the ~35 small
 *     surfaces that need table SEMANTICS a `role=grid` of divs structurally
 *     cannot provide: `<caption>`, `<th scope>`, `<tfoot>`, and the row/cell
 *     relationships assistive tech (and `closest("tr")`) actually rely on.
 *
 * ## Why the filter control lives INSIDE the `<th>`
 *
 * The obvious shape — a second `<tr>` of filter inputs under the header row — is
 * a trap. Two stacked sticky rows require the second row's `top:` to equal the
 * FIRST row's rendered height, which is not knowable in CSS: it changes with the
 * density token, with font loading, with browser zoom and with any column whose
 * label wraps. Every implementation of it ends up measuring the header with a
 * ResizeObserver and writing `top` back as an inline style — a layout-write
 * feedback loop that flickers on exactly the surfaces that stream. Putting the
 * control on a second LINE inside the same `<th>` keeps ONE sticky band at
 * `top: 0` with nothing to measure, and it keeps each control inside the cell it
 * filters, which is where a screen reader expects to find it.
 *
 * ## Column widths
 *
 * A generated `<colgroup>` plus `table-layout: fixed` (see the CSS module) —
 * nothing in the GUI set either before, so widths were re-solved from cell
 * content on every render and visibly jittered as prices ticked.
 *
 * ## Accessibility
 *
 * `aria-sort` on every sortable header; `aria-label={`Filter <header>`}` on every
 * filter control; the filter affordance carries one `role="search"` landmark
 * (see the CSS module for why it is on the toolbar rather than per-`<th>`); and
 * the scrollport becomes focusable ONLY while it genuinely overflows, via the
 * shared {@link useScrollableRegion} measurement that `Panel` also uses — a
 * hardcoded `tabIndex={0}` would make every non-overflowing table a dead tab
 * stop, and its absence regresses axe's `scrollable-region-focusable`.
 *
 * There is deliberately NO `aria-live` region here: the single live announcer
 * for a table's result count is the existing `<TableSearch>` "N of M".
 */

import { type ReactNode, useId } from "react";

import { scrollableRegionProps, useScrollableRegion } from "../hooks/useScrollableRegion";
import type { GridController } from "../hooks/useGridState";
import { isActiveFilter, type ColumnDef, type ColumnFilterValue } from "../lib/grid";
import styles from "./DataTable.module.css";

/**
 * Attributes a caller may attach to a body `<tr>` — click/context-menu handlers,
 * `data-testid`, selection classes. The `data-*` index signature is what lets a
 * migrated blotter keep its existing test hooks verbatim.
 */
export type RowAttrs = React.ComponentPropsWithoutRef<"tr"> & {
  readonly [dataAttribute: `data-${string}`]: string | undefined;
};

export interface DataTableProps<T> {
  /** Accessible name for the table (also the scrollport's region name). */
  readonly label: string;
  /** The column model — identity, header, width, accessor, cell, sort, filter. */
  readonly columns: readonly ColumnDef<T>[];
  /** The sort/filter controller driving this table (see `useGridState`). */
  readonly grid: GridController<T>;
  /** Stable React key for a row. */
  readonly rowKey: (row: T) => string;
  /** Optional per-row DOM attributes (handlers, testids, selected class). */
  readonly rowProps?: (row: T) => RowAttrs;
  /** Optional visible `<caption>`. Omitted ⇒ the table is named by `label`. */
  readonly caption?: ReactNode;
  /** Optional `<tfoot>` content — one `<tr>` of `<td>`s supplied by the caller. */
  readonly footer?: ReactNode;
  /** Honest empty state when the pipeline yields no rows. */
  readonly emptyState?: ReactNode;
  /** Hide the toolbar's row count (when the caller already shows "N of M"). */
  readonly hideRowCount?: boolean;
  /** Extra class on the outer surface. */
  readonly className?: string;
}

/** Does any column declare a filter? No ⇒ the Filters toggle is not rendered. */
function hasAnyFilter<T>(columns: readonly ColumnDef<T>[]): boolean {
  return columns.some((c) => c.filter !== undefined);
}

function alignClass<T>(column: ColumnDef<T>): string {
  if (column.align === "right") return styles.alignRight ?? "";
  if (column.align === "center") return styles.alignCenter ?? "";
  return "";
}

/** The `aria-sort` value for a column: absent unless the column is sortable. */
function ariaSort<T>(
  column: ColumnDef<T>,
  sortKey: string | null,
  desc: boolean,
): "ascending" | "descending" | "none" | undefined {
  if (column.sortKey === undefined) return undefined;
  if (column.sortKey !== sortKey) return "none";
  return desc ? "descending" : "ascending";
}

/**
 * One column's filter control, rendered on the second line of its `<th>`.
 *
 * Every control is labelled `Filter <header>` (a range column's two bounds
 * extend that to `… minimum` / `… maximum` so the pair stays distinguishable),
 * because a bare input inside a header cell is otherwise announced with no
 * indication of what it filters.
 */
function FilterControl<T>({
  column,
  value,
  options,
  onChange,
}: {
  readonly column: ColumnDef<T>;
  readonly value: ColumnFilterValue | undefined;
  readonly options: readonly string[];
  readonly onChange: (next: ColumnFilterValue | null) => void;
}): React.ReactElement | null {
  const filter = column.filter;
  if (!filter) return null;
  const active = isActiveFilter(value);
  const inputClass = `${styles.filterInput} ${active ? styles.filterActive : ""}`;

  if (filter.kind === "select") {
    const selected = value?.kind === "select" ? value.selected : "";
    return (
      <select
        className={`${styles.filterSelect} ${active ? styles.filterActive : ""}`}
        aria-label={`Filter ${column.header}`}
        value={selected}
        onChange={(e) => onChange({ kind: "select", selected: e.target.value })}
      >
        <option value="">All</option>
        {options.map((o) => (
          <option key={o} value={o}>
            {o}
          </option>
        ))}
      </select>
    );
  }

  if (filter.kind === "range") {
    const min = value?.kind === "range" ? value.min : null;
    const max = value?.kind === "range" ? value.max : null;
    // An empty box means "unbounded", NOT zero — so the empty string maps to
    // null rather than Number("") === 0, which would silently filter rows out.
    const parse = (raw: string): number | null => {
      if (raw.trim() === "") return null;
      const n = Number(raw);
      return Number.isFinite(n) ? n : null;
    };
    return (
      <>
        <input
          type="number"
          className={inputClass}
          aria-label={`Filter ${column.header} minimum`}
          value={min ?? ""}
          onChange={(e) => onChange({ kind: "range", min: parse(e.target.value), max })}
        />
        <input
          type="number"
          className={inputClass}
          aria-label={`Filter ${column.header} maximum`}
          value={max ?? ""}
          onChange={(e) => onChange({ kind: "range", min, max: parse(e.target.value) })}
        />
      </>
    );
  }

  const query = value?.kind === "text" ? value.query : "";
  return (
    <input
      type="search"
      className={inputClass}
      aria-label={`Filter ${column.header}`}
      value={query}
      spellCheck={false}
      autoComplete="off"
      onChange={(e) => onChange({ kind: "text", query: e.target.value })}
    />
  );
}

export function DataTable<T>({
  label,
  columns,
  grid,
  rowKey,
  rowProps,
  caption,
  footer,
  emptyState,
  hideRowCount = false,
  className,
}: DataTableProps<T>): React.ReactElement {
  const [scrollRef, scrollable] = useScrollableRegion<HTMLDivElement>();
  const toolbarId = useId();
  const { state, rows, activeFilters } = grid;
  const filterable = hasAnyFilter(columns);
  const filtersOpen = filterable && state.filtersOpen;

  // The table is at least the sum of its column widths; narrower containers
  // scroll horizontally rather than crushing the fixed tracks.
  const minWidth = columns.reduce((acc, c) => acc + c.width, 0);

  return (
    <div className={`${styles.surface} ${className ?? ""}`}>
      {filterable && (
        <div className={styles.toolbar} role="search" aria-label={`${label} column filters`}>
          <button
            type="button"
            id={toolbarId}
            className={`${styles.filterToggle} ${filtersOpen ? styles.filterToggleOn : ""}`}
            aria-expanded={filtersOpen}
            aria-pressed={filtersOpen}
            // The badge is a decorative glyph; the count is spelled out here so a
            // screen reader hears "3 active" rather than an unexplained "3".
            aria-label={
              activeFilters > 0
                ? `Filters, ${activeFilters} active column filter${activeFilters === 1 ? "" : "s"}`
                : "Filters"
            }
            data-testid="datatable-filters-toggle"
            onClick={() => grid.setFiltersOpen(!filtersOpen)}
          >
            Filters
            {activeFilters > 0 && (
              <span className={styles.badge} aria-hidden="true">
                {activeFilters}
              </span>
            )}
          </button>
          {activeFilters > 0 && (
            <button
              type="button"
              className={styles.clearBtn}
              data-testid="datatable-clear-filters"
              onClick={() => grid.clearFilters()}
            >
              Clear filters
            </button>
          )}
          {!hideRowCount && (
            <span className={styles.rowCount}>
              {rows.length} row{rows.length === 1 ? "" : "s"}
            </span>
          )}
        </div>
      )}

      <div
        ref={scrollRef}
        className={styles.scroll}
        {...scrollableRegionProps(scrollable, { label })}
      >
        <table className={styles.table} aria-label={label} style={{ minWidth }}>
          {caption !== undefined && <caption className={styles.caption}>{caption}</caption>}
          {/* Generated colgroup — the widths `table-layout: fixed` resolves from. */}
          <colgroup>
            {columns.map((c) => (
              <col key={c.key} style={{ width: `${c.width}px` }} />
            ))}
          </colgroup>
          <thead>
            <tr>
              {columns.map((c) => {
                const sortable = c.sortKey !== undefined;
                const isSorted = sortable && c.sortKey === state.sortKey;
                const desc = isSorted && state.sortDir === "desc";
                const labelNode = (
                  <>
                    <span className={styles.headLabel}>{c.header}</span>
                    {c.unit !== undefined && <span className={styles.unit}>{c.unit}</span>}
                  </>
                );
                return (
                  <th
                    key={c.key}
                    scope="col"
                    className={`${styles.th} ${alignClass(c)}`}
                    {...(c.description !== undefined ? { title: c.description } : {})}
                    {...(ariaSort(c, state.sortKey, desc) !== undefined
                      ? { "aria-sort": ariaSort(c, state.sortKey, desc) }
                      : {})}
                  >
                    <div className={styles.headLine}>
                      {sortable ? (
                        <button
                          type="button"
                          className={styles.sortBtn}
                          data-testid={`datatable-sort-${c.key}`}
                          onClick={() => grid.toggleSort(c.sortKey as string)}
                        >
                          {labelNode}
                          <span
                            className={`${styles.sortGlyph} ${isSorted ? "" : styles.sortGlyphIdle}`}
                            aria-hidden="true"
                          >
                            {desc ? "▾" : "▴"}
                          </span>
                        </button>
                      ) : (
                        labelNode
                      )}
                    </div>
                    {filtersOpen &&
                      (c.filter !== undefined ? (
                        <div className={styles.filterLine}>
                          <FilterControl
                            column={c}
                            value={state.filters[c.key]}
                            options={grid.optionsFor(c.key)}
                            onChange={(next) => grid.setFilter(c.key, next)}
                          />
                        </div>
                      ) : (
                        <div className={styles.filterSpacer} aria-hidden="true" />
                      ))}
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 ? (
              <tr>
                <td className={`${styles.td} ${styles.empty}`} colSpan={columns.length}>
                  {emptyState ?? "No rows."}
                </td>
              </tr>
            ) : (
              rows.map((row) => {
                const extra = rowProps?.(row) ?? {};
                const { className: rowClass, ...rest } = extra;
                return (
                  <tr
                    key={rowKey(row)}
                    className={`${styles.tr} ${rowClass ?? ""}`}
                    {...(rest as React.ComponentPropsWithoutRef<"tr">)}
                  >
                    {columns.map((c) => (
                      <td key={c.key} className={`${styles.td} ${alignClass(c)}`}>
                        {c.cell ? c.cell(row) : c.accessor(row)}
                      </td>
                    ))}
                  </tr>
                );
              })
            )}
          </tbody>
          {footer !== undefined && <tfoot className={styles.tfoot}>{footer}</tfoot>}
        </table>
      </div>
    </div>
  );
}
