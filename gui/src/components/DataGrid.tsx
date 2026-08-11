/**
 * DataGrid — the ONE shared accessible virtualised groupable grid (GW0-S2,
 * docs/GW-FOUNDATION-PLAN.md §1/§2). It is the substrate every blotter/table lane
 * (Stream/Book/Cube/Risk) migrates onto in GW3-GW5; this wave builds the PRIMITIVE
 * and its tests only — no workspace is migrated here.
 *
 * It REVERSES the deliberate `role=grid` opt-out documented at
 * StreamWorkspace.tsx:699 by actually implementing the WAI-ARIA APG "grid"
 * keyboard model the opt-out was waiting on:
 *
 *   - **role=grid** with `aria-rowcount`/`aria-colcount` reflecting the FULL
 *     logical set (not the windowed slice), and each rendered row/cell carrying
 *     its absolute `aria-rowindex`/`aria-colindex` (1-based, header = row 1).
 *   - **roving tabindex** — exactly ONE cell has `tabIndex=0` at a time (the
 *     active cell from `moveRoving`); all others are `tabIndex=-1`. Arrow keys
 *     move the active cell, Home/End to row ends (Ctrl to grid corners), PageUp/
 *     PageDown by a viewport of rows. Focus follows the active cell.
 *   - **row windowing** via the dependency-free `useVirtualWindow` (lib/virtual.ts)
 *     and **column windowing** via `columnWindow` (lib/grid.ts) — only in-view
 *     cells are in the DOM; spacers carry the off-screen extent.
 *   - **group rows** — collapsible (`role=row` + `aria-expanded`); Enter/Space on
 *     a group header toggles it.
 *   - **tick-coalescing** — `useCoalescedCells` buffers per-cell updates and
 *     flushes one last-write-wins value per cell per animation frame, so a
 *     high-rate stream paints at most once per frame.
 *
 * The pure logic lives in lib/grid.ts (headlessly tested against APG/algebraic
 * oracles); this file is the thin DOM binding.
 */

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";

import { useVirtualWindow } from "../lib/virtual";
import {
  HEADER_ROW,
  coalesce,
  columnWindow,
  flattenGroups,
  isCollapsed,
  moveRoving,
  sameCell,
  type CellCoord,
  type CellUpdate,
  type ColumnDef,
  type FlatRow,
  type GroupModel,
  type RovingKey,
  type RowGroup,
} from "../lib/grid";
import styles from "./DataGrid.module.css";

const ROVING_KEYS: ReadonlySet<string> = new Set([
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "Home",
  "End",
  "PageUp",
  "PageDown",
]);

/** Default uniform row height (px) when the density token can't be measured. */
const DEFAULT_ROW_HEIGHT = 30;

export interface DataGridProps<T> {
  /** Accessible name for the grid region. */
  readonly label: string;
  /** Column definitions (the visible columns, in order). */
  readonly columns: ReadonlyArray<ColumnDef<T>>;
  /**
   * Grouped data. A single implicit group (key "") may carry all rows for an
   * ungrouped grid; group headers are only rendered when `groupable` is true.
   */
  readonly groups: ReadonlyArray<RowGroup<T>>;
  /** Whether to render collapsible group header rows. Default false. */
  readonly groupable?: boolean;
  /** Collapsed-group state (controlled); required when `groupable`. */
  readonly groupModel?: GroupModel;
  /** Toggle a group's collapsed state (controlled); required when `groupable`. */
  readonly onToggleGroup?: (key: string) => void;
  /** Uniform row height in px. Default reads the `--row-h` density token. */
  readonly rowHeight?: number;
  /** Optional sort affordance: current sort key + direction. */
  readonly sort?: { readonly key: string; readonly desc: boolean } | null;
  /** Invoked when a sortable header cell is activated. */
  readonly onSort?: (key: string) => void;
  /** Honest empty-state node when there are no rows. */
  readonly emptyState?: ReactNode;
}

/**
 * Resolve the uniform row height: explicit prop wins; otherwise read the
 * `--row-h` density token off `:root` (set by design/density.ts); fall back to a
 * constant headlessly (jsdom reports no computed value).
 */
function useRowHeight(explicit: number | undefined): number {
  const [tokenHeight, setTokenHeight] = useState<number>(DEFAULT_ROW_HEIGHT);
  useLayoutEffect(() => {
    if (explicit != null || typeof getComputedStyle === "undefined") return;
    const raw = getComputedStyle(document.documentElement).getPropertyValue("--row-h").trim();
    const parsed = Number.parseFloat(raw);
    if (Number.isFinite(parsed) && parsed > 0) setTokenHeight(parsed);
  }, [explicit]);
  return explicit ?? tokenHeight;
}

export function DataGrid<T>(props: DataGridProps<T>): ReactNode {
  const {
    label,
    columns,
    groups,
    groupable = false,
    groupModel,
    onToggleGroup,
    rowHeight: rowHeightProp,
    sort = null,
    onSort,
    emptyState,
  } = props;

  const rowHeight = useRowHeight(rowHeightProp);
  const colCount = columns.length;

  // Flatten the (possibly grouped) data into the linear row sequence we window.
  const flat = useMemo<Array<FlatRow<T>>>(() => {
    if (groupable && groupModel) return flattenGroups(groups, groupModel);
    // Ungrouped: just the data rows of every group, no header rows.
    const out: Array<FlatRow<T>> = [];
    for (const g of groups) {
      for (const r of g.rows) out.push({ kind: "data", key: r.key, datum: r.datum });
    }
    return out;
  }, [groupable, groupModel, groups]);

  const rowCount = flat.length;

  // --- Row windowing (vertical) -------------------------------------------
  const vwin = useVirtualWindow<HTMLDivElement>({ count: rowCount, rowHeight });

  // --- Column windowing (horizontal) --------------------------------------
  const widths = useMemo(() => columns.map((c) => c.width), [columns]);
  const [scrollLeft, setScrollLeft] = useState(0);
  const [hViewport, setHViewport] = useState(0);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const onHScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    setScrollLeft(Math.max(0, e.currentTarget.scrollLeft));
    setHViewport(Math.max(0, e.currentTarget.clientWidth));
  }, []);
  const cwin = useMemo(
    () => columnWindow(widths, scrollLeft, hViewport, 1),
    [widths, scrollLeft, hViewport],
  );

  // --- Roving tabindex -----------------------------------------------------
  const [active, setActive] = useState<CellCoord>({ row: HEADER_ROW, col: 0 });
  // Clamp the active cell if the grid shape shrinks underneath it.
  useEffect(() => {
    setActive((prev) => {
      const lastRow = rowCount - 1;
      const rowCeil = lastRow < 0 ? HEADER_ROW : lastRow;
      const col = Math.min(prev.col, Math.max(0, colCount - 1));
      const row = prev.row > rowCeil ? rowCeil : prev.row;
      return row === prev.row && col === prev.col ? prev : { row, col };
    });
  }, [rowCount, colCount]);

  const pageSize = useMemo(() => {
    const vp = vwin.totalHeight > 0 ? (vwin.padTop + (vwin.end - vwin.start) * rowHeight) : 0;
    // A page = the number of fully-visible rows, ≥ 1. Use the rendered window
    // span as the viewport estimate (headless ⇒ window span; live ⇒ clientHeight).
    const visible = Math.max(1, Math.floor(vp / rowHeight) || vwin.end - vwin.start || 1);
    return visible;
  }, [vwin.totalHeight, vwin.padTop, vwin.end, vwin.start, rowHeight]);

  // Keep the active cell scrolled into view (vertical) when it moves.
  const bodyRef = useRef<HTMLDivElement | null>(null);
  const setBodyRef = useCallback(
    (el: HTMLDivElement | null) => {
      bodyRef.current = el;
      vwin.ref(el);
    },
    [vwin],
  );

  const onKeyDown = useCallback(
    (e: ReactKeyboardEvent<HTMLDivElement>) => {
      if (!ROVING_KEYS.has(e.key)) {
        // Enter/Space toggle a group header when the active row is one.
        if ((e.key === "Enter" || e.key === " ") && groupable && groupModel && onToggleGroup) {
          const r = flat[active.row];
          if (r && r.kind === "group") {
            e.preventDefault();
            onToggleGroup(r.key);
          }
        }
        return;
      }
      e.preventDefault();
      setActive((prev) =>
        moveRoving(
          { active: prev },
          e.key as RovingKey,
          { rowCount, colCount, pageSize },
          e.ctrlKey || e.metaKey,
        ).active,
      );
    },
    [rowCount, colCount, pageSize, groupable, groupModel, onToggleGroup, flat, active.row],
  );

  // After the active cell changes, ensure its row is within the windowed slice by
  // scrolling the body; the virtualiser then renders it and focus lands on it.
  useLayoutEffect(() => {
    const body = bodyRef.current;
    if (!body || active.row === HEADER_ROW) return;
    const top = active.row * rowHeight;
    const bottom = top + rowHeight;
    if (top < body.scrollTop) body.scrollTop = top;
    else if (bottom > body.scrollTop + body.clientHeight)
      body.scrollTop = bottom - body.clientHeight;
  }, [active.row, rowHeight]);

  // Move DOM focus to the active cell once it is rendered.
  const activeCellRef = useRef<HTMLDivElement | null>(null);
  const gridFocused = useRef(false);
  useLayoutEffect(() => {
    if (gridFocused.current && activeCellRef.current) activeCellRef.current.focus();
  });

  const renderedCols = columns.slice(cwin.start, cwin.end);
  const totalAriaRows = rowCount + 1; // + the header row (APG: header is row 1).

  function cellTabIndex(row: number, col: number): number {
    return sameCell(active, { row, col }) ? 0 : -1;
  }
  function cellRef(row: number, col: number): ((el: HTMLDivElement | null) => void) | undefined {
    return sameCell(active, { row, col })
      ? (el): void => {
          activeCellRef.current = el;
        }
      : undefined;
  }

  function alignClass(align: ColumnDef<T>["align"]): string {
    if (align === "left") return styles.alignLeft ?? "";
    if (align === "center") return styles.alignCenter ?? "";
    return styles.alignRight ?? "";
  }

  return (
    <div
      className={styles.grid}
      role="grid"
      aria-label={label}
      aria-rowcount={totalAriaRows}
      aria-colcount={colCount}
      onKeyDown={onKeyDown}
      onFocus={() => {
        gridFocused.current = true;
      }}
      onBlur={() => {
        gridFocused.current = false;
      }}
    >
      <div className={styles.viewport} ref={viewportRef} onScroll={onHScroll}>
        {/* Header band: aria-rowindex 1. */}
        <div
          className={styles.headerRow}
          role="row"
          aria-rowindex={1}
          style={{ width: cwin.totalWidth }}
        >
          <div style={{ width: cwin.padLeft, flex: "0 0 auto" }} aria-hidden="true" />
          {renderedCols.map((col, i) => {
            const colIndex = cwin.start + i;
            const isSorted = sort?.key === col.sortKey;
            const inner = (
              <>
                {col.header}
                {col.unit && <span className={styles.unit}>{col.unit}</span>}
              </>
            );
            return (
              <div
                key={col.key}
                role="columnheader"
                aria-colindex={colIndex + 1}
                aria-sort={
                  col.sortKey
                    ? isSorted
                      ? sort?.desc
                        ? "descending"
                        : "ascending"
                      : "none"
                    : undefined
                }
                tabIndex={cellTabIndex(HEADER_ROW, colIndex)}
                ref={cellRef(HEADER_ROW, colIndex)}
                className={`${styles.headerCell} ${alignClass(col.align)}`}
                style={{ width: col.width }}
              >
                {col.sortKey && onSort ? (
                  <button
                    type="button"
                    className={styles.sortBtn}
                    aria-pressed={isSorted}
                    tabIndex={-1}
                    onClick={() => onSort(col.sortKey as string)}
                  >
                    {inner}
                  </button>
                ) : (
                  inner
                )}
              </div>
            );
          })}
          <div style={{ width: cwin.padRight, flex: "0 0 auto" }} aria-hidden="true" />
        </div>

        {/* Body: the windowed data/group rows. */}
        <div className={styles.body} ref={setBodyRef}>
          {rowCount === 0 ? (
            <div className={styles.empty} role="row" aria-rowindex={2}>
              <span role="gridcell" aria-colindex={1}>
                {emptyState ?? "—"}
              </span>
            </div>
          ) : (
            <div style={{ height: vwin.totalHeight, position: "relative" }}>
              <div style={{ height: vwin.padTop }} aria-hidden="true" />
              <div style={{ width: cwin.totalWidth }}>
                {flat.slice(vwin.start, vwin.end).map((fr, i) => {
                  const rowIndex = vwin.start + i;
                  // aria-rowindex is 1-based and includes the header at 1.
                  const ariaRowIndex = rowIndex + 2;
                  if (fr.kind === "group") {
                    const collapsed = groupModel ? isCollapsed(groupModel, fr.key) : false;
                    return (
                      <div
                        key={`g:${fr.key}`}
                        role="row"
                        aria-rowindex={ariaRowIndex}
                        aria-expanded={!collapsed}
                        className={styles.groupRow}
                        tabIndex={cellTabIndex(rowIndex, active.col)}
                        ref={cellRef(rowIndex, active.col)}
                      >
                        <span aria-hidden="true">{collapsed ? "▸" : "▾"}</span>
                        <span>{fr.label}</span>
                        <span className={styles.groupCount}>· {fr.count}</span>
                      </div>
                    );
                  }
                  return (
                    <div
                      key={`d:${fr.key}`}
                      role="row"
                      aria-rowindex={ariaRowIndex}
                      className={styles.row}
                      style={{ width: cwin.totalWidth }}
                    >
                      <div style={{ width: cwin.padLeft, flex: "0 0 auto" }} aria-hidden="true" />
                      {renderedCols.map((col, ci) => {
                        const colIndex = cwin.start + ci;
                        return (
                          <div
                            key={col.key}
                            role="gridcell"
                            aria-colindex={colIndex + 1}
                            tabIndex={cellTabIndex(rowIndex, colIndex)}
                            ref={cellRef(rowIndex, colIndex)}
                            className={`${styles.cell} ${alignClass(col.align)}`}
                            style={{ width: col.width }}
                          >
                            {/* The model's RICH renderer wins when present; the
                                mandatory `accessor` stays the text projection
                                search/filter/export use. Both bindings resolve a
                                cell the same way, so a column moved between them
                                renders identically. */}
                            {col.cell ? col.cell(fr.datum) : col.accessor(fr.datum)}
                          </div>
                        );
                      })}
                      <div style={{ width: cwin.padRight, flex: "0 0 auto" }} aria-hidden="true" />
                    </div>
                  );
                })}
              </div>
              <div style={{ height: vwin.padBottom }} aria-hidden="true" />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/**
 * Tick-coalescing hook: buffer per-cell updates and flush a single last-write-wins
 * snapshot per animation frame, so a high-rate stream re-renders at most once per
 * frame. Returns the current per-cell value map + an `enqueue` to push updates
 * and a `flush` for tests (deterministic, no rAF). The accounting identity
 * `applied + coalesced == produced` is preserved by `coalesce` (lib/grid.ts).
 *
 * The map key is `${rowKey}\t${colKey}`; the caller indexes cells through
 * `cellValue(map, rowKey, colKey)`.
 */
export function useCoalescedCells<V>(): {
  values: ReadonlyMap<string, V>;
  enqueue: (updates: ReadonlyArray<CellUpdate<V>>) => void;
  flush: () => { produced: number; coalesced: number };
  stats: { produced: number; coalesced: number };
} {
  const buffer = useRef<Array<CellUpdate<V>>>([]);
  const frame = useRef<number | null>(null);
  const [values, setValues] = useState<ReadonlyMap<string, V>>(new Map());
  const stats = useRef({ produced: 0, coalesced: 0 });

  const flush = useCallback((): { produced: number; coalesced: number } => {
    frame.current = null;
    const batch = buffer.current;
    buffer.current = [];
    if (batch.length === 0) return { produced: 0, coalesced: 0 };
    const result = coalesce(batch);
    setValues((prev) => {
      const next = new Map(prev);
      for (const u of result.applied) next.set(`${u.rowKey}\t${u.colKey}`, u.value);
      return next;
    });
    stats.current = {
      produced: stats.current.produced + result.produced,
      coalesced: stats.current.coalesced + result.coalesced,
    };
    return { produced: result.produced, coalesced: result.coalesced };
  }, []);

  const enqueue = useCallback(
    (updates: ReadonlyArray<CellUpdate<V>>) => {
      for (const u of updates) buffer.current.push(u);
      if (frame.current == null && typeof requestAnimationFrame !== "undefined") {
        frame.current = requestAnimationFrame(() => flush());
      }
    },
    [flush],
  );

  useEffect(
    () => () => {
      if (frame.current != null && typeof cancelAnimationFrame !== "undefined") {
        cancelAnimationFrame(frame.current);
      }
    },
    [],
  );

  return { values, enqueue, flush, stats: stats.current };
}

/** Read a coalesced cell value by address. */
export function cellValue<V>(
  values: ReadonlyMap<string, V>,
  rowKey: string,
  colKey: string,
): V | undefined {
  return values.get(`${rowKey}\t${colKey}`);
}
