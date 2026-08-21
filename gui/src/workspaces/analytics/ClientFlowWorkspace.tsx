/**
 * ClientFlowWorkspace — the cross-asset (FI + FXO) client-flow / P&L-attribution
 * table (docs/ANALYTICS-REQUIREMENTS.md §11.1a). Reads the server rollup via
 * `listClientFlowMetrics(groupBy, window?)` and renders one row per group key with
 * the desk's canonical margin-efficiency and quote-fishing signals:
 *
 *   • $/mm gross & net — margin captured (and net of markout + hedge cost) per
 *     USD 1mm traded, so a 5mm and a 50mm line are comparable. Net-negative $/mm is
 *     coloured (a fat gross that turns negative once adverse selection is subtracted).
 *   • captured-vs-offered, mean cover distance, breakeven spread — spread economics.
 *   • quote-to-trade ratio, hit-rate, fishing score — quote-fishing detection; a high
 *     fisher (many quotes, ~0 net $/mm) is flagged.
 *
 * A group-by selector (client / counterparty / instrument / asset) re-QUERIES the
 * server; the Asset grouping is the product (FI vs FXO) split, and a product filter
 * re-FILTERS those two rows. Every optional metric renders "—" when ABSENT (a
 * zero-denominator guard), never a fabricated 0 or NaN. Sortable by every column;
 * the desk cares most about $/mm net (desc) and fishing score (desc).
 *
 * Read-only and gated on `view_analytics` — the tab, its rail entry and the query
 * are all hidden/denied without it (docs/PERMISSIONS-GRANULAR-REVIEW.md).
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../app/AppContext";
import { TableSearch } from "../../components/TableSearch";
import type { ClientFlowMetrics, FlowGroupBy } from "../../data/contract";
import styles from "./ClientFlowWorkspace.module.css";

/** The group-by options in bar order, with the axis label each row key represents. */
const GROUP_BY_OPTIONS: readonly { id: FlowGroupBy; label: string; axis: string }[] = [
  { id: "client", label: "Client", axis: "Client" },
  { id: "counterparty", label: "Counterparty", axis: "Counterparty" },
  { id: "instrument", label: "Instrument", axis: "Instrument" },
  { id: "asset", label: "Asset", axis: "Product" },
];

/** The product filter (client-side over the Asset grouping's two rows). */
type ProductFilter = "all" | "fixed_income" | "fx_options";
const PRODUCT_OPTIONS: readonly { id: ProductFilter; label: string; assetLabel?: string }[] = [
  { id: "all", label: "All" },
  { id: "fixed_income", label: "Fixed Income", assetLabel: "Fixed Income" },
  { id: "fx_options", label: "FX Options", assetLabel: "FX Options" },
];

// --- formatting (— for absent; never NaN/0-as-real) --------------------------

const compact = new Intl.NumberFormat("en-US", {
  notation: "compact",
  maximumFractionDigits: 1,
});

/** $/mm — a signed whole-dollar margin-per-million (absent ⇒ "—"). */
function fmtDpm(n: number | undefined): string {
  if (n === undefined) return "—";
  const sign = n < 0 ? "-" : "";
  return `${sign}$${Math.abs(n).toFixed(0)}`;
}

/** A bare ratio to 2dp (absent ⇒ "—"). */
function fmtRatio(n: number | undefined): string {
  return n === undefined ? "—" : n.toFixed(2);
}

/** A ratio as a percent to 1dp (absent ⇒ "—") — used for hit-rate. */
function fmtPct(n: number | undefined): string {
  return n === undefined ? "—" : `${(n * 100).toFixed(1)}%`;
}

/** Cover distance in basis points (absent ⇒ "—"). */
function fmtBps(n: number | undefined): string {
  return n === undefined ? "—" : `${n.toFixed(1)} bps`;
}

/** Signed compact USD (P&L / notional). */
function fmtUsd(n: number): string {
  const sign = n < 0 ? "-" : "";
  return `${sign}$${compact.format(Math.abs(n))}`;
}

/** Integer count. */
function fmtCount(n: number): string {
  return new Intl.NumberFormat("en-US").format(n);
}

/** Fishing-score band: high fishers are flagged. Mirrors the score's [0,1] range. */
type FishBand = "low" | "mid" | "high";
function fishBand(score: number): FishBand {
  if (score >= 0.66) return "high";
  if (score >= 0.33) return "mid";
  return "low";
}

// --- column model ------------------------------------------------------------

type SortDir = "asc" | "desc";

interface ColumnDef {
  key: string;
  label: string;
  /** A short header tooltip. */
  title: string;
  /** Numeric alignment (metrics right-align; the label left-aligns). */
  numeric: boolean;
  /** The sort key (undefined sorts last, independent of direction). */
  sortValue: (r: ClientFlowMetrics) => number | undefined | string;
}

/** The metric columns (the label column is rendered separately as the row header). */
function metricColumns(axis: string): ColumnDef[] {
  return [
    { key: "dpmGross", label: "$/mm gross", title: "Gross margin captured per USD 1mm traded", numeric: true, sortValue: (r) => r.dpmGross },
    { key: "dpmNet", label: "$/mm net", title: "Net (gross − markout − hedge cost) per USD 1mm traded", numeric: true, sortValue: (r) => r.dpmNet },
    { key: "capturedVsOffered", label: "Capt/offered", title: "Realised margin ÷ the spread we quoted", numeric: true, sortValue: (r) => r.capturedVsOffered },
    { key: "meanCoverDistance", label: "Cover dist", title: "Mean distance (bps) from the winning/cover price", numeric: true, sortValue: (r) => r.meanCoverDistance },
    { key: "breakevenSpread", label: "Breakeven", title: "Spread ($/mm) at which net $/mm hits zero", numeric: true, sortValue: (r) => r.breakevenSpread },
    { key: "quoteToTradeRatio", label: "Q/T ratio", title: "Quotes ÷ trades — high with a low hit-rate = fishing", numeric: true, sortValue: (r) => r.quoteToTradeRatio },
    { key: "hitRate", label: "Hit rate", title: "Trades ÷ quotes (the RFQ hit-rate)", numeric: true, sortValue: (r) => r.hitRate },
    { key: "fishingScore", label: "Fishing", title: "Quote-fishing score [0,1] — high = harvesting prices, not dealing", numeric: true, sortValue: (r) => r.fishingScore },
    { key: "tradedNotional", label: "Notional", title: "Traded notional (USD)", numeric: true, sortValue: (r) => r.tradedNotional },
    { key: "quoteCount", label: "Quotes", title: `Quotes/RFQs issued to this ${axis.toLowerCase()}`, numeric: true, sortValue: (r) => r.quoteCount },
    { key: "tradedCount", label: "Trades", title: "Fills done", numeric: true, sortValue: (r) => r.tradedCount },
    { key: "grossPnl", label: "Gross P&L", title: "Gross margin captured (USD)", numeric: true, sortValue: (r) => r.grossPnl },
    { key: "netPnl", label: "Net P&L", title: "Net P&L = gross − markout − hedge (USD)", numeric: true, sortValue: (r) => r.netPnl },
  ];
}

/**
 * Trailing-debounce for the live revalidate: a fill storm from the sub-second FIX sim
 * coalesces into at most ~1 refetch per this window rather than one per fill.
 */
const REVALIDATE_DEBOUNCE_MS = 750;

/** Compare two sort values; `undefined` always sorts LAST regardless of direction. */
function compareSort(
  a: number | undefined | string,
  b: number | undefined | string,
  dir: SortDir,
): number {
  const aMissing = a === undefined;
  const bMissing = b === undefined;
  if (aMissing && bMissing) return 0;
  if (aMissing) return 1;
  if (bMissing) return -1;
  let cmp: number;
  if (typeof a === "string" || typeof b === "string") {
    cmp = String(a).localeCompare(String(b));
  } else {
    cmp = a - b;
  }
  return dir === "asc" ? cmp : -cmp;
}

export function ClientFlowWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;

  const [groupBy, setGroupBy] = useState<FlowGroupBy>("client");
  const [product, setProduct] = useState<ProductFilter>("all");
  const [rows, setRows] = useState<ClientFlowMetrics[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [sortKey, setSortKey] = useState<string>("dpmNet");
  const [sortDir, setSortDir] = useState<SortDir>("desc");
  // The grouping axis changes what a row IS (a client, a desk, a product), so the search
  // deliberately matches the row LABEL rather than a fixed field — it stays correct as
  // the axis changes underneath it.
  const [query, setQuery] = useState("");
  // Fishing band is derived from the score, so it is not a value you could filter as
  // text — and "show me only the high-fishing clients" is the question this table exists
  // to answer.
  const [bandFilter, setBandFilter] = useState<FishBand | "">("");

  const axis = GROUP_BY_OPTIONS.find((g) => g.id === groupBy)?.axis ?? "Client";
  const columns = useMemo(() => metricColumns(axis), [axis]);
  // The product filter is only meaningful over the Asset grouping's two rows: for
  // the other groupings a row is a cross-asset roll-up with no single product, so
  // the control is disabled (honest — the frozen server contract carries no
  // per-row asset), never a silent no-op.
  const productFilterEnabled = groupBy === "asset";

  const load = useCallback(
    (gb: FlowGroupBy): void => {
      if (!signedIn) {
        setRows([]);
        setLoadError(null);
        return;
      }
      setLoading(true);
      void app.transport
        .listClientFlowMetrics(gb)
        .then((m) => {
          setRows(m);
          setLoadError(null);
        })
        .catch((e: unknown) => {
          setRows([]);
          setLoadError(e instanceof Error ? e.message : "failed to load client-flow metrics");
        })
        .finally(() => setLoading(false));
    },
    [app.transport, signedIn],
  );

  useEffect(() => {
    load(groupBy);
  }, [load, groupBy]);

  // Live revalidate: refetch the current group-by on every push `Notification`, exactly
  // as the Risk Dashboard drill-down does — fills mint deals and fire notifications, so
  // this keeps the flow table live without its own poll. Trailing-debounced so the
  // sub-second FIX sim's fill storm coalesces into ≤1 refetch/REVALIDATE_DEBOUNCE_MS
  // instead of one per fill. Re-subscribes when `groupBy` changes so the refetch always
  // uses the active filter; the mount effect above owns the FIRST fetch (we only refetch
  // on a frame, never on subscribe). Best-effort: a transport without the push seam (or
  // one that lacks it) simply doesn't live-refresh.
  useEffect(() => {
    if (!signedIn) return;
    const stream = app.transport.streamNotifications;
    if (typeof stream !== "function") return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const dispose = stream.call(app.transport, undefined, () => {
      if (timer !== undefined) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = undefined;
        load(groupBy);
      }, REVALIDATE_DEBOUNCE_MS);
    });
    return () => {
      if (timer !== undefined) clearTimeout(timer);
      dispose?.();
    };
  }, [app.transport, signedIn, groupBy, load]);

  // Sort a header: same key toggles direction; a new key starts descending (the
  // desk reads "worst/highest first" for $/mm net and fishing).
  const onSort = useCallback(
    (key: string): void => {
      setSortKey((prev) => {
        if (prev === key) {
          setSortDir((d) => (d === "asc" ? "desc" : "asc"));
          return prev;
        }
        setSortDir("desc");
        return key;
      });
    },
    [],
  );

  const visibleRows = useMemo(() => {
    const byProduct =
      productFilterEnabled && product !== "all"
        ? rows.filter((r) => r.label === PRODUCT_OPTIONS.find((p) => p.id === product)?.assetLabel)
        : rows;
    const needle = query.trim().toLowerCase();
    const filtered = byProduct.filter((r) => {
      if (bandFilter !== "" && fishBand(r.fishingScore) !== bandFilter) return false;
      return needle === "" || r.label.toLowerCase().includes(needle);
    });
    const col = columns.find((c) => c.key === sortKey);
    const labelSort = sortKey === "label";
    const withSort = [...filtered];
    withSort.sort((a, b) => {
      if (labelSort) return compareSort(a.label, b.label, sortDir);
      if (!col) return 0;
      return compareSort(col.sortValue(a), col.sortValue(b), sortDir);
    });
    return withSort;
  }, [rows, columns, sortKey, sortDir, product, productFilterEnabled, query, bandFilter]);

  const ariaSort = (key: string): "ascending" | "descending" | "none" =>
    sortKey === key ? (sortDir === "asc" ? "ascending" : "descending") : "none";

  return (
    <section className={styles.wrap} aria-labelledby="clientflow-title">
      <header className={styles.head}>
        <div className={styles.headMain}>
          <h1 id="clientflow-title" className={styles.title}>
            Client Flow
          </h1>
          <p className={styles.subtitle}>
            Cross-asset P&amp;L attribution — margin efficiency and quote-fishing across
            what the desk priced and traded.
          </p>
        </div>
        <div className={styles.controls}>
          <div
            className={styles.segmented}
            role="group"
            aria-label="Group client flow by"
          >
            <span className={styles.segLabel} id="groupby-label">
              Group by
            </span>
            {GROUP_BY_OPTIONS.map((g) => (
              <button
                key={g.id}
                type="button"
                className={`${styles.segBtn} ${groupBy === g.id ? styles.segOn : ""}`}
                aria-pressed={groupBy === g.id}
                onClick={() => setGroupBy(g.id)}
              >
                {g.label}
              </button>
            ))}
          </div>
          <div
            className={styles.segmented}
            role="group"
            aria-label="Filter by product"
          >
            <span className={styles.segLabel}>Product</span>
            {PRODUCT_OPTIONS.map((p) => (
              <button
                key={p.id}
                type="button"
                className={`${styles.segBtn} ${product === p.id && productFilterEnabled ? styles.segOn : ""}`}
                aria-pressed={product === p.id}
                disabled={!productFilterEnabled}
                title={
                  productFilterEnabled
                    ? undefined
                    : "Product filter applies to the Asset grouping — client / counterparty / instrument rows roll up both products."
                }
                onClick={() => setProduct(p.id)}
              >
                {p.label}
              </button>
            ))}
          </div>
        </div>
      </header>

      <p className={styles.explainer}>
        <strong>$/mm</strong> is margin captured per USD 1&nbsp;million traded (so a 5mm
        and a 50mm trade compare); <strong>net</strong> subtracts adverse-selection
        markout and hedging cost. <strong>Fishing score</strong> [0–1] flags clients
        firing many quotes but rarely dealing at ~0 net $/mm — harvesting prices, not
        price discovery.
      </p>

      {loadError !== null && (
        <p className={styles.error} role="alert">
          {loadError}
        </p>
      )}

      {!signedIn ? (
        <p className={styles.empty}>Sign in to view client-flow analytics.</p>
      ) : loading && rows.length === 0 ? (
        <p className={styles.empty}>Loading client-flow metrics…</p>
      ) : rows.length === 0 ? (
        <p className={styles.empty}>No client-flow metrics for this selection.</p>
      ) : (
        <>
        <div className={styles.tableTools}>
          <TableSearch
            query={query}
            onQueryChange={setQuery}
            shown={visibleRows.length}
            total={rows.length}
            label={`Search by ${axis.toLowerCase()}`}
            placeholder={`Filter by ${axis.toLowerCase()}…`}
          />
          <label className={styles.bandFilter}>
            <span className={styles.bandFilterLabel}>Fishing</span>
            <select
              className={styles.bandFilterSelect}
              value={bandFilter}
              aria-label="Filter by fishing band"
              data-testid="client-flow-band-filter"
              onChange={(e) => setBandFilter(e.target.value as FishBand | "")}
            >
              <option value="">Any band</option>
              <option value="high">High</option>
              <option value="mid">Mid</option>
              <option value="low">Low</option>
            </select>
          </label>
        </div>
        <div className={styles.tableScroll}>
          <table className={styles.table}>
            <caption className={styles.caption}>
              Client-flow metrics grouped by {axis.toLowerCase()}
              {productFilterEnabled && product !== "all"
                ? ` — ${PRODUCT_OPTIONS.find((p) => p.id === product)?.assetLabel}`
                : ""}
            </caption>
            <thead>
              <tr>
                <th scope="col" className={styles.thLabel} aria-sort={ariaSort("label")}>
                  <button type="button" className={styles.sortBtn} onClick={() => onSort("label")}>
                    {axis}
                    <SortGlyph active={sortKey === "label"} dir={sortDir} />
                  </button>
                </th>
                {columns.map((c) => (
                  <th
                    key={c.key}
                    scope="col"
                    className={styles.thNum}
                    aria-sort={ariaSort(c.key)}
                    title={c.title}
                  >
                    <button type="button" className={styles.sortBtnNum} onClick={() => onSort(c.key)}>
                      <SortGlyph active={sortKey === c.key} dir={sortDir} />
                      {c.label}
                    </button>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {visibleRows.length === 0 && (
                <tr>
                  <td className={styles.emptyRow} colSpan={columns.length + 1}>
                    No {axis.toLowerCase()} matches this search or band.
                  </td>
                </tr>
              )}
              {visibleRows.map((r) => {
                const band = fishBand(r.fishingScore);
                return (
                  <tr key={r.label}>
                    <th scope="row" className={styles.rowLabel}>
                      {r.label}
                    </th>
                    <td className={styles.num}>{fmtDpm(r.dpmGross)}</td>
                    <td
                      className={`${styles.num} ${r.dpmNet !== undefined && r.dpmNet < 0 ? styles.negative : ""}`}
                    >
                      {fmtDpm(r.dpmNet)}
                    </td>
                    <td className={styles.num}>{fmtRatio(r.capturedVsOffered)}</td>
                    <td className={styles.num}>{fmtBps(r.meanCoverDistance)}</td>
                    <td className={styles.num}>{fmtDpm(r.breakevenSpread)}</td>
                    <td className={styles.num}>{fmtRatio(r.quoteToTradeRatio)}</td>
                    <td className={styles.num}>{fmtPct(r.hitRate)}</td>
                    <td className={styles.num}>
                      <span
                        className={`${styles.fish} ${styles[`fish-${band}`]}`}
                        title={
                          band === "high"
                            ? "High fisher — many quotes, ~0 net $/mm"
                            : band === "mid"
                              ? "Some fishing"
                              : "Franchise flow"
                        }
                      >
                        <span
                          className={styles.fishBar}
                          style={{ width: `${Math.round(r.fishingScore * 100)}%` }}
                          aria-hidden
                        />
                        <span className={styles.fishVal}>{r.fishingScore.toFixed(2)}</span>
                      </span>
                    </td>
                    <td className={styles.num}>{fmtUsd(r.tradedNotional)}</td>
                    <td className={styles.num}>{fmtCount(r.quoteCount)}</td>
                    <td className={styles.num}>{fmtCount(r.tradedCount)}</td>
                    <td className={styles.num}>{fmtUsd(r.grossPnl)}</td>
                    <td
                      className={`${styles.num} ${r.netPnl < 0 ? styles.negative : ""}`}
                    >
                      {fmtUsd(r.netPnl)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        </>
      )}
    </section>
  );
}

/** A tiny sort-direction caret shown on the active sort column. */
function SortGlyph({ active, dir }: { active: boolean; dir: SortDir }): React.ReactElement {
  return (
    <span className={styles.sortGlyph} aria-hidden>
      {active ? (dir === "asc" ? "▲" : "▼") : "↕"}
    </span>
  );
}
