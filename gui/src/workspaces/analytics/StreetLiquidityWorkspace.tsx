/**
 * StreetLiquidityWorkspace — the LP-side league table (Analytics pillar 2; the
 * street-side mirror of ClientFlow's client side). Reads the server rollup via
 * `listLpFlowMetrics(window?, lpId?)` and renders one row per liquidity provider —
 * who we trade WITH on the LP side — with the desk's street-liquidity signals:
 *
 *   • tick rate — the LP's quote-update frequency over the window.
 *   • quotes / deals won / won notional — activity and competition outcome.
 *   • missed — panel appearances where the LP was quoted but the deal went elsewhere.
 *   • last-look rejects — the LP's ranked wins rejected at last-look (a rejecter is
 *     flagged; last-look toxicity is a real cost, not a neutral count).
 *   • win-rate (won ÷ quoted) — a chronic misser (low win-rate) is coloured; the
 *     win-rate is ABSENT ("—") when the LP made no quotes (a dormant feed).
 *   • mean cover — the LP's distance from the winner when it was the runner-up
 *     (ABSENT when it was never the cover).
 *
 * Sortable by every column (default deals-won desc — the desk reads "who we win the
 * most from" first). Every optional metric renders "—" when ABSENT (a zero-denominator
 * guard), never a fabricated 0 or NaN. Read-only and gated on `view_analytics` — the
 * tab, its rail entry and the query are all hidden/denied without it.
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../app/AppContext";
import type { LpFlowMetrics } from "../../data/contract";
import { StreetExecution } from "./StreetExecution";
import styles from "./StreetLiquidityWorkspace.module.css";

// --- formatting (— for absent; never NaN/0-as-real) --------------------------

const compact = new Intl.NumberFormat("en-US", {
  notation: "compact",
  maximumFractionDigits: 1,
});

/** Integer count with grouping (ticks, quotes, deals, misses, rejects). */
function fmtCount(n: number): string {
  return new Intl.NumberFormat("en-US").format(n);
}

/** Compact tick count (a high-frequency magnitude reads better as "120k"). */
function fmtTicks(n: number): string {
  return compact.format(n);
}

/** Compact USD notional (k/m/b). */
function fmtUsd(n: number): string {
  const sign = n < 0 ? "-" : "";
  return `${sign}$${compact.format(Math.abs(n))}`;
}

/** A ratio as a percent to 1dp (absent ⇒ "—") — used for win-rate. */
function fmtPct(n: number | undefined): string {
  return n === undefined ? "—" : `${(n * 100).toFixed(1)}%`;
}

/** Cover distance in basis points (absent ⇒ "—"). */
function fmtBps(n: number | undefined): string {
  return n === undefined ? "—" : `${n.toFixed(1)} bps`;
}

/** Win-rate band: a chronic misser (low, but non-empty) is flagged. */
type WinBand = "high" | "mid" | "low";
function winBand(rate: number): WinBand {
  if (rate >= 0.4) return "high";
  if (rate >= 0.15) return "mid";
  return "low";
}

/** Last-look rejects worth flagging (a rejecter's ranked wins pulled at last-look). */
const LAST_LOOK_FLAG = 50;

// --- column model ------------------------------------------------------------

type SortDir = "asc" | "desc";

interface ColumnDef {
  key: string;
  label: string;
  /** A short header tooltip. */
  title: string;
  /** The sort key (undefined sorts last, independent of direction). */
  sortValue: (r: LpFlowMetrics) => number | undefined;
}

const COLUMNS: readonly ColumnDef[] = [
  { key: "tickCount", label: "Tick rate", title: "Quote-update ticks observed from this LP over the window", sortValue: (r) => r.tickCount },
  { key: "quoteCount", label: "Quotes", title: "Panel responses from this LP (the win-rate denominator)", sortValue: (r) => r.quoteCount },
  { key: "dealsWon", label: "Deals won", title: "Deals this LP won (booked to it)", sortValue: (r) => r.dealsWon },
  { key: "wonNotional", label: "Won notional", title: "Summed notional magnitude of the deals this LP won (USD)", sortValue: (r) => r.wonNotional },
  { key: "missed", label: "Missed", title: "Panel appearances where the LP was quoted but the deal went elsewhere", sortValue: (r) => r.missed },
  { key: "lastLookRejects", label: "Last-look", title: "Times this LP's ranked win was rejected at last-look", sortValue: (r) => r.lastLookRejects },
  { key: "winRate", label: "Win rate", title: "Win-rate = deals won ÷ quotes. Absent when the LP made no quotes", sortValue: (r) => r.winRate },
  { key: "meanCover", label: "Mean cover", title: "Mean distance (bps) from the winner when this LP was the runner-up", sortValue: (r) => r.meanCover },
];

/** Compare two sort values; `undefined` always sorts LAST regardless of direction. */
function compareSort(a: number | undefined, b: number | undefined, dir: SortDir): number {
  const aMissing = a === undefined;
  const bMissing = b === undefined;
  if (aMissing && bMissing) return 0;
  if (aMissing) return 1;
  if (bMissing) return -1;
  const cmp = a - b;
  return dir === "asc" ? cmp : -cmp;
}

export function StreetLiquidityWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;

  const [rows, setRows] = useState<LpFlowMetrics[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [sortKey, setSortKey] = useState<string>("dealsWon");
  const [sortDir, setSortDir] = useState<SortDir>("desc");

  const load = useCallback((): void => {
    if (!signedIn) {
      setRows([]);
      setLoadError(null);
      return;
    }
    setLoading(true);
    void app.transport
      .listLpFlowMetrics()
      .then((m) => {
        setRows(m);
        setLoadError(null);
      })
      .catch((e: unknown) => {
        setRows([]);
        setLoadError(e instanceof Error ? e.message : "failed to load street-liquidity metrics");
      })
      .finally(() => setLoading(false));
  }, [app.transport, signedIn]);

  useEffect(() => {
    load();
  }, [load]);

  // Sort a header: same key toggles direction; a new key starts descending (the
  // desk reads "most won / highest win-rate first").
  const onSort = useCallback((key: string): void => {
    setSortKey((prev) => {
      if (prev === key) {
        setSortDir((d) => (d === "asc" ? "desc" : "asc"));
        return prev;
      }
      setSortDir("desc");
      return key;
    });
  }, []);

  const visibleRows = useMemo(() => {
    const labelSort = sortKey === "lpId";
    const col = COLUMNS.find((c) => c.key === sortKey);
    const withSort = [...rows];
    withSort.sort((a, b) => {
      if (labelSort) {
        const cmp = a.lpId.localeCompare(b.lpId);
        return sortDir === "asc" ? cmp : -cmp;
      }
      if (!col) return 0;
      return compareSort(col.sortValue(a), col.sortValue(b), sortDir);
    });
    return withSort;
  }, [rows, sortKey, sortDir]);

  const ariaSort = (key: string): "ascending" | "descending" | "none" =>
    sortKey === key ? (sortDir === "asc" ? "ascending" : "descending") : "none";

  return (
    <section className={styles.wrap} aria-labelledby="streetliq-title">
      <header className={styles.head}>
        <div className={styles.headMain}>
          <h1 id="streetliq-title" className={styles.title}>
            Street Liquidity
          </h1>
          <p className={styles.subtitle}>
            The LP-side league table — who we trade with on the street side: tick rate,
            competition outcome, and last-look behaviour per liquidity provider.
          </p>
        </div>
      </header>

      <p className={styles.explainer}>
        <strong>Win rate</strong> is deals won ÷ quotes — a chronic misser (many quotes,
        few wins) is coloured. <strong>Last-look</strong> counts an LP's ranked wins
        pulled at the last moment — a rejecter is flagged. <strong>Mean cover</strong> is
        how far the LP sat from the winner when it was runner-up. A metric is
        <strong> —</strong> when genuinely absent (an LP that made no quotes, or was never
        the cover), never a fabricated zero.
      </p>

      {loadError !== null && (
        <p className={styles.error} role="alert">
          {loadError}
        </p>
      )}

      {!signedIn ? (
        <p className={styles.empty}>Sign in to view street-liquidity analytics.</p>
      ) : loading && rows.length === 0 ? (
        <p className={styles.empty}>Loading street-liquidity metrics…</p>
      ) : visibleRows.length === 0 ? (
        <p className={styles.empty}>No street-liquidity metrics available.</p>
      ) : (
        <div className={styles.tableScroll}>
          <table className={styles.table}>
            <caption className={styles.caption}>
              Street-side LP liquidity — one row per liquidity provider
            </caption>
            <thead>
              <tr>
                <th scope="col" className={styles.thLabel} aria-sort={ariaSort("lpId")}>
                  <button type="button" className={styles.sortBtn} onClick={() => onSort("lpId")}>
                    LP
                    <SortGlyph active={sortKey === "lpId"} dir={sortDir} />
                  </button>
                </th>
                {COLUMNS.map((c) => (
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
              {visibleRows.map((r) => {
                const band = r.winRate === undefined ? undefined : winBand(r.winRate);
                const flaggedRejects = r.lastLookRejects >= LAST_LOOK_FLAG;
                return (
                  <tr key={r.lpId}>
                    <th scope="row" className={styles.rowLabel}>
                      {r.lpId}
                    </th>
                    <td className={styles.num}>{fmtTicks(r.tickCount)}</td>
                    <td className={styles.num}>{fmtCount(r.quoteCount)}</td>
                    <td className={styles.num}>{fmtCount(r.dealsWon)}</td>
                    <td className={styles.num}>{fmtUsd(r.wonNotional)}</td>
                    <td className={styles.num}>{fmtCount(r.missed)}</td>
                    <td
                      className={`${styles.num} ${flaggedRejects ? styles.reject : ""}`}
                      title={
                        flaggedRejects
                          ? "Last-look rejecter — a notable slice of ranked wins pulled at last-look"
                          : undefined
                      }
                    >
                      {fmtCount(r.lastLookRejects)}
                    </td>
                    <td className={styles.num}>
                      {band === undefined ? (
                        "—"
                      ) : (
                        <span
                          className={`${styles.win} ${styles[`win-${band}`]}`}
                          title={
                            band === "low"
                              ? "Chronic misser — many quotes, few wins"
                              : band === "high"
                                ? "Top-of-book winner"
                                : "Moderate hit-rate"
                          }
                        >
                          <span
                            className={styles.winBar}
                            style={{ width: `${Math.round((r.winRate ?? 0) * 100)}%` }}
                            aria-hidden
                          />
                          <span className={styles.winVal}>{fmtPct(r.winRate)}</span>
                        </span>
                      )}
                    </td>
                    <td className={styles.num}>{fmtBps(r.meanCover)}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      {/* The ORDER-level half of the same surface: what we actually sent to the
          street, and what came back. Kept inside this workspace (rather than a
          second, competing screen) so "who we trade with" and "what we sent them"
          read as one story. */}
      <StreetExecution />
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
