/**
 * StreamWorkspace — the RFS blotter, the RESTING STATE (GUI-DESIGN §4.2), scaled
 * to an investment-banking-sized book (TRADING-UNIVERSE-SCALE §5). A living table
 * of streaming two-ways multiplexed over one StreamSession. Only changed numbers
 * flash (calm under fire); per-row stream health is honest (◉/◐/○ from real
 * seq/resync state). Click a side to trade: the row carries the maker's short-lived
 * TradableTokens and a click sends Execute, surfacing a typed Executed/StreamReject
 * toast.
 *
 * SCALE (this lane — TRADING-UNIVERSE-SCALE §5):
 *  (a) ROW VIRTUALISATION via `useVirtualWindow` (src/lib/virtual.ts) — the body
 *      renders ONLY the rows in view plus an overscan band, with top/bottom
 *      spacers carrying the off-screen height. A global IB book of thousands of
 *      lines stays one-paint-per-frame; off-screen rows are not in the DOM. The
 *      windowing is REAL (not cosmetic): the live seeded set is small, but the
 *      slice is driven by the scroll container's scrollTop/clientHeight, so it
 *      scales unchanged when the registry/book grows.
 *  (b) GROUP / COLLAPSE by pair (or tenor) with AGGREGATION HEADER rows — the
 *      blotter folds the book into desk-navigable groups, each header showing the
 *      group's row count, net spot-Δ and mean implied σ (real aggregates over the
 *      group's streamed rows, never fabricated). Headers + data rows are flattened
 *      into ONE uniform-height list so a single virtualiser windows both.
 *  (c) CONFIGURABLE / SORTABLE COLUMNS with a sensible default + STICKY header —
 *      click a numeric/price header to sort (asc/desc/none), and the column set is
 *      togglable. Sort is applied WITHIN each group (the grouping is the primary
 *      key) so the desk taxonomy is preserved.
 *
 * Comparison note (positioning, not a product identifier): a streaming-first
 * (RFS) blotter as the resting state, the structural inversion of the
 * request-quote-per-click (RFQ) cadence common to incumbent options front-ends.
 *
 * HONESTY (CLAUDE.md rule 2): PREMIUM plots the row's OWN streamed premium-mid
 * history; the market-observable modes (ATM vol, RR, BF, spot, forward) plot a
 * REAL series streamed from the contract's market-series feed
 * (`MarketSeriesSubscribe`, served by celnet-server) — never a fabricated line.
 * VEGA/PNL stay DISABLED (they need the position-fact store, a later phase). The
 * active mode's label + unit are always shown on the trend column header and the
 * tile. The blotter's columns name their units honestly: "Premium mid" in the
 * conventions' premium units, an "impl σ" implied-vol column, and a "Δ spot"
 * delta column (P0-8). Group aggregates are honest reductions over the real rows.
 */

import { useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import type { ScopeContext } from "../app/AppContext";
import { PriceTile } from "../components/PriceTile";
import { Sparkline, sparklineDirection, type SparklineDir } from "../components/Sparkline";
import { StatusBadge } from "../components/StatusBadge";
import { Button } from "../components/Button";
import { ownerLabel, type StreamRow } from "../hooks/useStreamSession";
import { useTrendSeries, type TrendRowKey, type TrendSeries } from "../hooks/useTrendSeries";
import { pairId, pairLabel } from "../lib/universe";
import { useVirtualWindow } from "../lib/virtual";
import { fmtPremiumPct, fmtRate, fmtSigned, fmtVol, fmtVolPoint, premiumUnit } from "../lib/format";
import {
  TREND_MODES,
  DEFAULT_TREND_MODE,
  trendModeSpec,
  type TrendMode,
  type TrendUnit,
} from "../lib/trend";
import { nowNanos } from "../hooks/useClock";
import styles from "./StreamWorkspace.module.css";

/** Uniform list-item height in px — group headers AND data rows share it so a
 *  single fixed-height virtualiser windows the whole list correctly. */
const ROW_HEIGHT = 38;

/** Format a trend value in its natural unit (for the tile's numeric readout). */
function fmtTrendValue(unit: TrendUnit, value: number): string {
  switch (unit) {
    case "premium":
      return fmtPremiumPct(value);
    case "vol":
      return fmtVolPoint(value);
    case "rate":
      return fmtRate(value);
    case "vega":
    case "pnl":
      return value.toFixed(2);
  }
}

/** The short unit caption shown under the trend column header for a mode. */
function trendUnitCaption(mode: TrendMode): string {
  switch (mode) {
    case "PREMIUM":
      return "premium";
    case "ATM_VOL":
      return "ATM vol";
    case "RR":
      return "25Δ RR";
    case "BF":
      return "25Δ BF";
    case "SPOT":
      return "spot";
    case "FORWARD":
      return "fwd";
    case "VEGA":
      return "vega";
    case "PNL":
      return "P&L";
  }
}

function tenorLabel(row: StreamRow): string {
  const t = row.instrument.tenor;
  // The one tenorless product (the perpetual option) carries no tenor label —
  // its honest blotter label is the no-expiry contract itself.
  if (!t) return "PERP";
  switch (t.unit) {
    case "OVERNIGHT":
      return "ON";
    case "TOM_NEXT":
      return "TN";
    case "SPOT_NEXT":
      return "SN";
    case "WEEKS":
      return `${t.count}W`;
    case "MONTHS":
      return `${t.count}M`;
    case "YEARS":
      return `${t.count}Y`;
    case "IMM":
      return `${t.count}IMM`;
    case "BROKEN_DATE":
      return t.brokenDate
        ? `${t.brokenDate.year}-${String(t.brokenDate.month).padStart(2, "0")}-${String(t.brokenDate.day).padStart(2, "0")}`
        : "broken";
  }
}

/**
 * Entitlement-ready scope filter (P0-6 seam). Today the principal is `grant-all`
 * and firm/desk/book are not subdivided, so this narrows ONLY when the scope has
 * drilled to a specific pair crumb (e.g. "EUR/USD") — a real, honest restriction
 * that a future entitlement principal will extend, with the blotter unchanged.
 */
function inScope(row: StreamRow, scope: ScopeContext): boolean {
  const tail = scope.path[scope.path.length - 1];
  if (!tail || tail.level !== "pair") return true;
  const label = `${row.instrument.pair.base}/${row.instrument.pair.quote}`;
  return label === tail.label;
}

/** The shared up/down glyph for the trend column (same rule as the sparkline). */
const TREND_GLYPH: Record<SparklineDir, string> = { up: "▲", down: "▼", flat: "▪" };

// --- column model -----------------------------------------------------------

/** A sortable/togglable column key. PAIR/STRUCTURE/TENOR/TREND/HEALTH are not
 *  independently sortable (grouping or non-scalar), so the sort set is numeric. */
type ColumnKey =
  | "pair"
  | "structure"
  | "tenor"
  | "bid"
  | "mid"
  | "offer"
  | "trend"
  | "delta"
  | "sigma"
  | "health";

/** The numeric/price columns the user can sort the book by (within each group). */
type SortKey = "mid" | "delta" | "sigma";
type SortDir = "asc" | "desc";
interface SortState {
  key: SortKey;
  dir: SortDir;
}

interface ColumnSpec {
  key: ColumnKey;
  /** Header label (Anaheim-cased by CSS). */
  label: string;
  /** Honest unit subtitle, when the column carries one. */
  unit?: string;
  /** The header alignment class. */
  align: "left" | "right" | "center";
  /** A sort key, when the column is sortable. */
  sort?: SortKey;
  /** Columns the user may hide (PAIR is the group key and stays mandatory). */
  optional?: boolean;
}

/** The DEFAULT column set + order — the sensible blotter default (P0-8 units). */
const COLUMNS: readonly ColumnSpec[] = [
  { key: "pair", label: "Pair", align: "left" },
  { key: "structure", label: "Structure", align: "left", optional: true },
  { key: "tenor", label: "Tenor", align: "right" },
  { key: "bid", label: "Bid", align: "right" },
  { key: "mid", label: "Premium mid", unit: "%", align: "right", sort: "mid" },
  { key: "offer", label: "Offer", align: "right" },
  { key: "trend", label: "Trend", align: "right", optional: true },
  { key: "delta", label: "Δ", unit: "spot", align: "right", sort: "delta", optional: true },
  { key: "sigma", label: "σ", unit: "impl", align: "right", sort: "sigma", optional: true },
  { key: "health", label: "◉", align: "center" },
];

/** The numeric scalar a row presents for a given sort key. */
function rowSortValue(row: StreamRow, key: SortKey): number {
  switch (key) {
    case "mid":
      return (row.price.bid + row.price.offer) / 2;
    case "delta":
      return row.greeks.deltaSpot;
    case "sigma":
      return row.vol;
  }
}

// --- grouping ---------------------------------------------------------------

type GroupBy = "pair" | "tenor";

interface GroupAgg {
  count: number;
  netDeltaSpot: number;
  meanVol: number;
  healthy: number;
}

interface GroupModel {
  id: string;
  label: string;
  rows: StreamRow[];
  agg: GroupAgg;
}

/** The group key + display label a row falls into for the active grouping. */
function groupKeyOf(row: StreamRow, by: GroupBy): { id: string; label: string } {
  if (by === "pair") {
    return { id: pairId(row.instrument.pair), label: pairLabel(row.instrument.pair) };
  }
  const t = tenorLabel(row);
  return { id: `tenor:${t}`, label: t };
}

function aggregate(rows: StreamRow[]): GroupAgg {
  let netDeltaSpot = 0;
  let volSum = 0;
  let healthy = 0;
  for (const r of rows) {
    netDeltaSpot += r.greeks.deltaSpot;
    volSum += r.vol;
    if (r.health === "HEALTHY") healthy += 1;
  }
  return {
    count: rows.length,
    netDeltaSpot,
    meanVol: rows.length > 0 ? volSum / rows.length : 0,
    healthy,
  };
}

/** Build grouped, sorted models. Grouping is the primary key; the active sort is
 *  applied WITHIN each group so the desk taxonomy is preserved. Groups keep
 *  first-seen order (stream order) so the layout is stable as numbers tick. */
function buildGroups(rows: StreamRow[], by: GroupBy, sort: SortState | null): GroupModel[] {
  const order: string[] = [];
  const byKey = new Map<string, { label: string; rows: StreamRow[] }>();
  for (const r of rows) {
    const { id, label } = groupKeyOf(r, by);
    let g = byKey.get(id);
    if (!g) {
      g = { label, rows: [] };
      byKey.set(id, g);
      order.push(id);
    }
    g.rows.push(r);
  }
  return order.map((id) => {
    const g = byKey.get(id)!;
    let groupRows = g.rows;
    if (sort) {
      const factor = sort.dir === "asc" ? 1 : -1;
      groupRows = [...g.rows].sort(
        (a, b) => factor * (rowSortValue(a, sort.key) - rowSortValue(b, sort.key)),
      );
    }
    return { id, label: g.label, rows: groupRows, agg: aggregate(g.rows) };
  });
}

// --- flattening (group headers + data rows → one uniform list) --------------

type FlatItem =
  | { kind: "group"; group: GroupModel }
  | { kind: "row"; row: StreamRow };

/** Flatten visible groups into a single uniform-height list: a header item per
 *  (non-collapsed first) group followed by its data rows. Collapsed groups
 *  contribute only their header. ONE virtualiser then windows the whole list. */
function flatten(groups: GroupModel[], collapsed: ReadonlySet<string>): FlatItem[] {
  const items: FlatItem[] = [];
  for (const g of groups) {
    items.push({ kind: "group", group: g });
    if (!collapsed.has(g.id)) {
      for (const row of g.rows) items.push({ kind: "row", row });
    }
  }
  return items;
}

// --- presentational rows ----------------------------------------------------

/** Build the CSS grid-template-columns string from the visible column set so the
 *  header, group headers and data rows all share the exact same track layout. */
function gridTemplate(visible: readonly ColumnSpec[]): string {
  const TRACK: Record<ColumnKey, string> = {
    pair: "72px",
    structure: "minmax(120px, 1.4fr)",
    tenor: "56px",
    bid: "minmax(72px, 1fr)",
    mid: "minmax(72px, 1fr)",
    offer: "minmax(72px, 1fr)",
    trend: "104px",
    delta: "64px",
    sigma: "64px",
    health: "32px",
  };
  return visible.map((c) => TRACK[c.key]).join(" ");
}

function GroupHeaderRow({
  group,
  collapsed,
  onToggle,
  template,
  span,
}: {
  group: GroupModel;
  collapsed: boolean;
  onToggle: () => void;
  template: string;
  /** How many grid tracks the meta cell spans (visible columns minus the label). */
  span: number;
}): React.ReactElement {
  const { agg } = group;
  return (
    <div className={styles.groupRow} style={{ gridTemplateColumns: template }}>
      <button
        type="button"
        className={styles.groupToggle}
        onClick={onToggle}
        aria-expanded={!collapsed}
        title={collapsed ? "Expand group" : "Collapse group"}
      >
        <span className={`${styles.groupCaret} ${collapsed ? styles.groupCaretClosed : ""}`} aria-hidden="true">
          ▾
        </span>
        <span className={styles.groupLabel}>{group.label}</span>
        <span className={`num ${styles.groupCount}`}>{agg.count}</span>
      </button>
      <span className={styles.groupMeta} style={{ gridColumn: `span ${Math.max(1, span)}` }}>
        <span className={`num ${styles.groupStat}`} title="Net spot delta across the group">
          ΣΔ <span className={styles.groupStatVal}>{fmtSigned(agg.netDeltaSpot, 3)}</span>
        </span>
        <span className={`num ${styles.groupStat}`} title="Mean implied vol across the group">
          σ̄ <span className={styles.groupStatVal}>{fmtVol(agg.meanVol)}</span>
        </span>
        <span className={`num ${styles.groupStat}`} title="Healthy / total streamed lines">
          ◉ <span className={styles.groupStatVal}>{agg.healthy}/{agg.count}</span>
        </span>
      </span>
    </div>
  );
}

function BlotterRow({
  row,
  mode,
  trend,
  columns,
  template,
}: {
  row: StreamRow;
  mode: TrendMode;
  /** The row's streamed market-series (non-PREMIUM modes), or undefined. */
  trend: TrendSeries | undefined;
  columns: readonly ColumnSpec[];
  template: string;
}): React.ReactElement {
  const app = useApp();
  const now = nowNanos();
  const sell = row.tradable.find((t) => t.side === "SELL");
  const buy = row.tradable.find((t) => t.side === "BUY");
  const tradable = row.health === "HEALTHY" && (sell?.validUntilNanos ?? 0n) > now;

  const spec = trendModeSpec(mode);
  // PREMIUM plots the row's OWN streamed premium mid (always live); a market-
  // observable mode plots the REAL streamed market-series for this row; VEGA/PNL
  // are gated (no series). Each path is honest — no fabricated values.
  const trendValues = mode === "PREMIUM" ? row.midHistory : (trend?.values ?? []);
  // ONE direction truth shared by the sparkline tint AND the trend glyph: the
  // net move across the visible window (Sparkline.sparklineDirection).
  const trendDir = sparklineDirection(trendValues);
  const hasTrend = trendValues.length >= 2;
  const trendLatest =
    mode === "PREMIUM"
      ? trendValues.length > 0
        ? trendValues[trendValues.length - 1]
        : undefined
      : trend?.latest;

  // Premium-mid in this row's OWN premium units (each row carries its conventions).
  const unit = premiumUnit(row.conventions);
  const mid = (row.price.bid + row.price.offer) / 2;

  const cell = (col: ColumnSpec): React.ReactElement => {
    switch (col.key) {
      case "pair":
        return (
          <span key={col.key} className={`num ${styles.pair}`}>
            {row.instrument.pair.base}/{row.instrument.pair.quote}
          </span>
        );
      case "structure":
        return (
          <span key={col.key} className={styles.structure}>
            {row.label}
            {/* Honest attribution: show the maker/owner only when the wire carries it
                (engine-quoted edge flow is the auto-pricer); nothing when absent. */}
            {ownerLabel(row.attribution?.quotedBy) && (
              <span className={styles.owner} title="Quoted by">
                {ownerLabel(row.attribution?.quotedBy)}
              </span>
            )}
          </span>
        );
      case "tenor":
        return (
          <span key={col.key} className={`num ${styles.tenor}`}>
            {tenorLabel(row)}
          </span>
        );
      case "bid":
        return (
          <button
            key={col.key}
            className={`${styles.priceCell} ${styles.bidCell}`}
            disabled={!tradable || !sell}
            onClick={() => sell && app.stream.execute(row.subscriptionId, "SELL")}
            title={tradable ? "Hit the bid (SELL)" : "not tradable"}
          >
            <PriceTile value={row.price.bid} format={fmtPremiumPct} side="bid" />
          </button>
        );
      case "mid":
        return (
          <span key={col.key} className={`num ${styles.mid}`} title={`Premium mid (${unit})`}>
            {fmtPremiumPct(mid)}
          </span>
        );
      case "offer":
        return (
          <button
            key={col.key}
            className={`${styles.priceCell} ${styles.offerCell}`}
            disabled={!tradable || !buy}
            onClick={() => buy && app.stream.execute(row.subscriptionId, "BUY")}
            title={tradable ? "Lift the offer (BUY)" : "not tradable"}
          >
            <PriceTile value={row.price.offer} format={fmtPremiumPct} side="offer" showGlyph />
          </button>
        );
      case "trend":
        return (
          <span key={col.key} className={styles.spark}>
            {!spec.available ? (
              <span className={styles.noTrend} aria-label={`${spec.label} not available`}>
                —
              </span>
            ) : hasTrend ? (
              <>
                <span
                  className={`num ${styles.trendGlyph} ${styles[`trend_${trendDir}`] ?? ""}`}
                  aria-hidden="true"
                >
                  {TREND_GLYPH[trendDir]}
                </span>
                <Sparkline
                  values={trendValues}
                  direction={trendDir}
                  ariaLabel={`${spec.label} trend, ${trendDir}`}
                />
                {trendLatest !== undefined && (
                  <span className={`num ${styles.trendValue}`} title={`${spec.label} (${trendUnitCaption(mode)})`}>
                    {fmtTrendValue(spec.unit, trendLatest)}
                  </span>
                )}
              </>
            ) : (
              <span className={styles.noTrend} aria-label="awaiting series">
                …
              </span>
            )}
          </span>
        );
      case "delta":
        return (
          <span key={col.key} className={`num ${styles.greek}`} title="Spot delta (Δ)">
            {fmtSigned(row.greeks.deltaSpot, 3)}
          </span>
        );
      case "sigma":
        return (
          <span key={col.key} className={`num ${styles.greek}`} title="Implied volatility (annualised, vol points)">
            {fmtVol(row.vol)}
          </span>
        );
      case "health":
        return (
          <span key={col.key} className={styles.health}>
            <StatusBadge health={row.health} />
          </span>
        );
    }
  };

  return (
    <div
      className={`${styles.row} ${row.health !== "HEALTHY" ? styles.dim : ""}`}
      style={{ gridTemplateColumns: template }}
    >
      {columns.map(cell)}
    </div>
  );
}

/**
 * The trend-mode selector. PREMIUM and the market-observable modes (ATM vol / RR /
 * BF / spot / forward) are SELECTABLE — clicking one re-plots every row's trend
 * column from the real streamed series for that observable. VEGA/PNL are shown
 * DISABLED (they need the position-fact store, a later phase). No fabricated trends.
 */
function TrendModeChips({
  mode,
  onSelect,
}: {
  mode: TrendMode;
  onSelect: (m: TrendMode) => void;
}): React.ReactElement {
  return (
    <div className={styles.trendModes} role="group" aria-label="trend series mode">
      <span className={styles.trendModesLabel}>Trend</span>
      {TREND_MODES.map((m) => {
        const active = m.id === mode;
        return (
          <button
            key={m.id}
            type="button"
            className={[
              styles.trendChip,
              active ? styles.trendChipActive : "",
              m.available ? "" : styles.trendChipDisabled,
            ]
              .filter(Boolean)
              .join(" ")}
            disabled={!m.available}
            aria-pressed={active}
            onClick={() => m.available && onSelect(m.id)}
            title={
              m.available
                ? `${m.label} — live streamed series`
                : `${m.label} — needs the position-fact store (later phase)`
            }
          >
            {m.label}
          </button>
        );
      })}
    </div>
  );
}

/** Sort glyph for a header cell given the active sort state. */
function sortGlyph(col: ColumnSpec, sort: SortState | null): string {
  if (!col.sort || !sort || sort.key !== col.sort) return "";
  return sort.dir === "asc" ? " ▲" : " ▼";
}

export function StreamWorkspace(): React.ReactElement {
  const app = useApp();
  // Entitlement-readiness: route the live rows through the firm-grant scope. It
  // narrows nothing today unless the scope has drilled to a specific pair crumb,
  // so a future desk/book principal can subdivide the universe without touching
  // this view (foundation P0-6 seam).
  const scope = app.scope;
  const rows = useMemo(
    () => app.stream.rows.filter((r) => inScope(r, scope)),
    [app.stream.rows, scope],
  );

  // The active trend-column mode. PREMIUM uses the row's own streamed mid; the
  // market-observable modes stream the contract's market-series feed (below).
  const [trendMode, setTrendMode] = useState<TrendMode>(DEFAULT_TREND_MODE);

  // Scale controls: how the book is folded, which columns show, how it's sorted,
  // and which groups are collapsed.
  const [groupBy, setGroupBy] = useState<GroupBy>("pair");
  const [sort, setSort] = useState<SortState | null>(null);
  const [hidden, setHidden] = useState<ReadonlySet<ColumnKey>>(new Set());
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());

  const columns = useMemo(() => COLUMNS.filter((c) => !hidden.has(c.key)), [hidden]);
  const template = useMemo(() => gridTemplate(columns), [columns]);

  // The per-row keys for the market-series subscriptions (pair + pillar tenor).
  const trendKeys = useMemo<TrendRowKey[]>(
    () =>
      rows.map((r) => ({
        subscriptionId: r.subscriptionId,
        pair: r.instrument.pair,
        tenorYears: r.instrument.expiryYears,
      })),
    [rows],
  );
  // The live streamed series per row for the active observable (empty for PREMIUM
  // and the gated modes — the row falls back to its own premium history).
  const trendSeries = useTrendSeries(app.transport, trendMode, trendKeys);
  const trendUnitLabel = trendUnitCaption(trendMode);

  // Fold the book into aggregation-headed groups, then flatten headers + rows into
  // a single uniform-height list the virtualiser windows.
  const groups = useMemo(() => buildGroups(rows, groupBy, sort), [rows, groupBy, sort]);
  const items = useMemo(() => flatten(groups, collapsed), [groups, collapsed]);

  // The single fixed-height virtualiser over the flat list. Only the rows in view
  // (+ overscan) ever enter the DOM, so a global IB book stays one-paint-per-frame.
  const v = useVirtualWindow<HTMLDivElement>({ count: items.length, rowHeight: ROW_HEIGHT });
  const slice = items.slice(v.start, v.end);

  // The group-meta cell spans every track after the label track (column 1).
  const metaSpan = Math.max(1, columns.length - 1);

  const toggleColumn = (key: ColumnKey): void =>
    setHidden((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  const toggleCollapse = (id: string): void =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  // Sort cycles asc → desc → off on the same column; a new column starts at asc.
  const cycleSort = (key: SortKey): void =>
    setSort((prev) => {
      if (!prev || prev.key !== key) return { key, dir: "asc" };
      if (prev.dir === "asc") return { key, dir: "desc" };
      return null;
    });

  return (
    <div className={styles.wrap}>
      {/* Scale toolbar: grouping, column toggles. Sort lives on the header cells. */}
      <div className={styles.controls}>
        <div className={styles.controlGroup} role="group" aria-label="group by">
          <span className={styles.controlLabel}>Group</span>
          {(["pair", "tenor"] as const).map((g) => (
            <button
              key={g}
              type="button"
              className={`${styles.segBtn} ${groupBy === g ? styles.segBtnActive : ""}`}
              aria-pressed={groupBy === g}
              onClick={() => setGroupBy(g)}
            >
              {g === "pair" ? "Pair" : "Tenor"}
            </button>
          ))}
        </div>
        <div className={styles.controlGroup} role="group" aria-label="columns">
          <span className={styles.controlLabel}>Columns</span>
          {COLUMNS.filter((c) => c.optional).map((c) => {
            const on = !hidden.has(c.key);
            return (
              <button
                key={c.key}
                type="button"
                className={`${styles.segBtn} ${on ? styles.segBtnActive : ""}`}
                aria-pressed={on}
                onClick={() => toggleColumn(c.key)}
                title={`${on ? "Hide" : "Show"} ${c.label}`}
              >
                {c.label === "Δ" ? "Δ" : c.label === "σ" ? "σ" : c.label}
              </button>
            );
          })}
        </div>
        <span className={`num ${styles.controlMeta}`}>
          {rows.length} lines · {groups.length} {groupBy === "pair" ? "pairs" : "tenors"}
        </span>
      </div>

      {/* The blotter is a labelled, sortable region (a table-like presentation). We
          deliberately do NOT claim role="grid": the full grid keyboard-navigation
          model isn't implemented, and a half-claimed grid is an a11y anti-pattern
          (axe flags `aria-sort`/required-children misuse). Sort state is conveyed
          on each header button via aria-pressed + an explicit accessible name. */}
      <section className={styles.gridRegion} aria-label="streaming two-way markets">
      {/* Sticky header — sortable numeric/price cells; the track layout is shared
          with every body row via the same grid template. */}
      <div className={styles.headerRow} style={{ gridTemplateColumns: template }}>
        {columns.map((col) => {
          const isSorted = col.sort && sort?.key === col.sort;
          const alignCls =
            col.align === "left"
              ? styles.colLeft
              : col.align === "center"
                ? styles.colCenter
                : "";
          // The Trend column's unit follows the active observable (PREMIUM / ATM
          // vol / RR / …), so it is resolved dynamically rather than static.
          const unitText = col.key === "trend" ? trendUnitLabel : col.unit;
          const inner = (
            <>
              {col.label}
              {unitText && <span className={styles.colUnit}>{unitText}</span>}
            </>
          );
          if (!col.sort) {
            return (
              <span
                key={col.key}
                className={`${styles.colCell} ${alignCls}`}
                aria-label={col.key === "health" ? "stream health" : undefined}
              >
                {inner}
              </span>
            );
          }
          // Sort state is carried on the button itself: aria-pressed when this is
          // the active sort key, plus a descriptive accessible name announcing the
          // current direction (valid on a button; `aria-sort` is grid-only).
          const sortDir = isSorted ? (sort!.dir === "asc" ? "ascending" : "descending") : null;
          return (
            <button
              key={col.key}
              type="button"
              className={`${styles.colCell} ${styles.colSortable} ${alignCls} ${isSorted ? styles.colSorted : ""}`}
              onClick={() => cycleSort(col.sort!)}
              aria-pressed={isSorted ? true : false}
              aria-label={`Sort by ${col.label}${sortDir ? ` (${sortDir})` : ""}`}
              title={`Sort by ${col.label}`}
            >
              {col.label}
              {col.unit && <span className={styles.colUnit}>{col.unit}</span>}
              <span className={`num ${styles.sortGlyph}`} aria-hidden="true">
                {sortGlyph(col, sort)}
              </span>
            </button>
          );
        })}
      </div>

      {/* Virtualised body: only the in-view slice (+ overscan) is in the DOM. The
          spacers carry the off-screen height so the scrollbar reflects the whole
          book. Verifiable: off-screen items never render. */}
      <div className={styles.body} ref={v.ref}>
        {items.length === 0 ? (
          <div className={styles.empty}>
            No streaming lines. Subscribe with <kbd>⌘K</kbd> to populate the blotter.
          </div>
        ) : (
          <div style={{ height: v.totalHeight, position: "relative" }}>
            <div style={{ height: v.padTop }} />
            {slice.map((item, i) => {
              const index = v.start + i;
              if (item.kind === "group") {
                return (
                  <GroupHeaderRow
                    key={`g:${item.group.id}`}
                    group={item.group}
                    collapsed={collapsed.has(item.group.id)}
                    onToggle={() => toggleCollapse(item.group.id)}
                    template={template}
                    span={metaSpan}
                  />
                );
              }
              return (
                <BlotterRow
                  key={`r:${item.row.subscriptionId.toString()}:${index}`}
                  row={item.row}
                  mode={trendMode}
                  trend={trendSeries.get(item.row.subscriptionId)}
                  columns={columns}
                  template={template}
                />
              );
            })}
            <div style={{ height: v.padBottom }} />
          </div>
        )}
      </div>
      </section>

      <div className={styles.foot}>
        <Button variant="ghost" onClick={() => app.setPaletteOpen(true)}>
          + Subscribe (⌘K)
        </Button>
        <TrendModeChips mode={trendMode} onSelect={setTrendMode} />
        <span className={`num ${styles.footMeta}`}>
          Conflated 60Hz · click a side to trade · {app.stream.lpCount} LPs in competition
        </span>
      </div>

      {/* Click-to-trade outcome toasts (Executed / typed StreamReject). */}
      <div className={styles.toasts} aria-live="polite">
        {app.stream.toasts.map((t) => (
          <div
            key={t.id}
            className={`${styles.toast} ${t.kind === "executed" ? styles.toastOk : styles.toastReject}`}
            onClick={() => app.stream.dismissToast(t.id)}
          >
            <span className={styles.toastGlyph}>{t.kind === "executed" ? "✓" : "✕"}</span>
            <span>{t.text}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
