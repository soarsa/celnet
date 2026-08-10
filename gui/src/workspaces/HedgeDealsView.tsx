/**
 * HedgeDealsView — the "Hedge deals" lens of the Deals blotter: the executed HEDGES,
 * distinct from the client fills. Where the Client-deals table shows what we traded
 * WITH THE CLIENT (and a per-row internalise badge = did we warehouse or shed it),
 * this shows what we did to SHED that risk: which LP we hit, the LP panel we fanned
 * to, the realised hedge price vs the mid at fire (slippage), and the internal-cross
 * / external / residual amounts — the full "who did we hedge with, at what price, for
 * how much" picture.
 *
 * SOURCE: the immutable fired-hedge audit trail `listHedgeProvenance()` (gated on the
 * narrow `hedge` capability × FI, server-enforced), refetched on each live hedge-intent
 * tick so the ledger tracks fires without its own poll — the SAME seam the Hedging
 * workspace's monitor reads. A `HedgeProvenance` links to a client deal by
 * book+instrument+time (the wire carries no deal→hedge id), so this is the desk-level
 * hedge ledger, not a per-deal join. Numbers are right-aligned + tabular so they line
 * up; DV01-family amounts read in the shared compact units. Theme-aware via tokens.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useApp } from "../app/AppContext";
import { Panel } from "../components/Panel";
import { TableSearch } from "../components/TableSearch";
import { useTableFilter } from "../hooks/useTableFilter";
import { fmtCompact, fmtRate } from "../lib/format";
import { describeExitAction, isExternalExitAction } from "../lib/hedgeExit";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import type { HedgeProvenance } from "../data/contract";
import styles from "./HedgeDealsView.module.css";

/** A hedge band label → RAG class key (mirrors the Hedging monitor). */
function ragKey(band: string): "green" | "amber" | "red" | "breach" {
  if (band === "amber") return "amber";
  if (band === "red") return "red";
  if (band === "breach") return "breach";
  return "green";
}

/** A hedge `firedAt` (epoch MILLIS, UTC) as a 24h clock. */
function timeOf(ms: number): string {
  return new Date(ms).toLocaleTimeString("en-GB", { hour12: false });
}

/** Realised slippage vs the mid, as a signed bp label (0 for a no-trade action). */
function slippageLabel(bp: number): string {
  if (bp === 0) return "—";
  const s = Math.abs(bp).toFixed(1);
  return bp < 0 ? `−${s}bp` : `+${s}bp`;
}

/**
 * Whether a hedge row is an EXTERNAL hedge (trades away — the desk's job) vs an
 * internalised decision (warehouse / cross-internal / skew / escalate). The hedge desk
 * shows external-only by default; the "Show internalised" toggle reveals the rest.
 */
function isExternalHedge(p: HedgeProvenance): boolean {
  return p.action !== null && isExternalExitAction(p.action.kind);
}

/**
 * The parent client deal a per-fill hedge execution reconciles to — its `position_id`
 * as a `#id` link. Book-level advisory records carry no single parent, so they read a
 * plain em dash. This is the reconciliation link back to the Deals blotter.
 */
function parentDealLabel(p: HedgeProvenance): string {
  return p.parentPositionId !== undefined ? `#${p.parentPositionId.toString()}` : "—";
}

/** All of a hedge row's user-visible textual fields, concatenated for substring search. */
function hedgeSearchText(p: HedgeProvenance): string {
  return [
    timeOf(p.firedAt),
    p.book,
    p.instrument,
    parentDealLabel(p),
    p.band,
    describeExitAction(p.action),
    p.lpWon ?? "",
    p.lps.join(" "),
    p.advisory ? "advisory" : "live",
    fmtCompact(p.internalCrossed),
    fmtCompact(p.externalHedged),
    p.hedgePrice > 0 ? fmtRate(p.hedgePrice) : "",
    p.hedgeId,
  ].join(" ");
}

export function HedgeDealsView(): React.ReactElement {
  const app = useApp();
  const canView = app.auth.can("hedge", "fixed_income");

  const [rows, setRows] = useState<HedgeProvenance[]>([]);
  const [error, setError] = useState<string | null>(null);
  // The hedge desk is hedging-only by default: internalised (warehouse / cross-internal
  // / skew / escalate) decisions are hidden unless the trader opts to audit them.
  const [showInternalised, setShowInternalised] = useState(false);

  const refetch = useCallback(() => {
    void app.transport
      .listHedgeProvenance()
      .then((p) => {
        setRows(p);
        setError(null);
      })
      .catch((e: unknown) =>
        setError(e instanceof Error ? e.message : "failed to load hedge provenance"),
      );
  }, [app.transport]);

  const refetchRef = useRef(refetch);
  refetchRef.current = refetch;

  // Load once and refetch on each live hedge-intent tick (a fire appends provenance),
  // mirroring the Hedging monitor — the ledger tracks fires without its own poll.
  useEffect(() => {
    if (!canView) return;
    refetchRef.current();
    const stream = app.transport.streamHedgeIntents;
    const dispose =
      typeof stream === "function"
        ? stream.call(app.transport, () => refetchRef.current())
        : undefined;
    return () => dispose?.();
  }, [app.transport, canView]);

  // Newest first, mirroring the client-deals blotter; then apply the hedging-only
  // desk filter (external-only unless "Show internalised" is on) BEFORE search.
  const sorted = useMemo(() => [...rows].sort((a, b) => b.firedAt - a.firedAt), [rows]);
  const visible = useMemo(
    () => (showInternalised ? sorted : sorted.filter(isExternalHedge)),
    [sorted, showInternalised],
  );
  const { query, setQuery, filtered, shown, total } = useTableFilter(visible, hedgeSearchText);

  const isOffline = !app.transport.label.startsWith("live");
  // The "external" total reflects the CURRENTLY-VISIBLE set (the filtered desk view).
  const externalTotal = visible.reduce((acc, p) => acc + p.externalHedged, 0);
  const internalisedCount = sorted.length - sorted.filter(isExternalHedge).length;

  if (!canView) {
    return (
      <div className={styles.wrap}>
        <Panel className={styles.panel} title="Hedge deals">
          <p className={styles.empty} title={capabilityDenialTitle("hedge", "fixed_income")}>
            Viewing executed hedges requires the <strong>hedge</strong> capability. This is the
            desk-level hedge ledger — who we hedged with, at what price, and for how much — shown to
            hedge-entitled users.
          </p>
        </Panel>
      </div>
    );
  }

  return (
    <div className={styles.wrap}>
      <Panel className={styles.panel} title="Hedge deals">
        <div className={styles.head}>
          <span className={styles.engine}>{isOffline ? "in-app hedge desk" : "live hedge desk"}</span>
          <label className={styles.internalisedToggle}>
            <input
              type="checkbox"
              checked={showInternalised}
              data-testid="hedge-show-internalised"
              onChange={(e) => setShowInternalised(e.target.checked)}
            />
            <span>
              Show internalised{internalisedCount > 0 ? ` (${internalisedCount})` : ""}
            </span>
          </label>
          <span className={styles.summary} data-testid="hedge-deals-summary">
            {visible.length} {showInternalised ? "hedge decision" : "external hedge"}
            {visible.length === 1 ? "" : "s"} · {fmtCompact(externalTotal)} external
          </span>
        </div>
        {error && (
          <p className={styles.error} role="alert">
            {error}
          </p>
        )}
        {visible.length === 0 ? (
          <p className={styles.empty}>
            {rows.length === 0
              ? "No fired hedges yet — as the auto-hedge engine sheds warehoused risk, each external execution (the LP/composite we hit, its price, the amount) appears here."
              : `No external hedges yet — ${internalisedCount} internalised decision${internalisedCount === 1 ? "" : "s"} ${internalisedCount === 1 ? "is" : "are"} hidden. Toggle “Show internalised” to audit ${internalisedCount === 1 ? "it" : "them"}.`}
          </p>
        ) : (
          <>
            <TableSearch
              query={query}
              onQueryChange={setQuery}
              shown={shown}
              total={total}
              label="Search hedge deals"
              placeholder="Filter hedges…"
            />
            {filtered.length === 0 ? (
              <p className={styles.empty}>No hedges match “{query}”.</p>
            ) : (
              <div className={styles.tableWrap} tabIndex={0} role="region" aria-label="Hedge deals table">
                <table className={styles.table}>
                  <thead>
                    <tr>
                      <th>Time</th>
                      <th>Book · Instrument</th>
                      <th>Parent deal</th>
                      <th>Band</th>
                      <th>Action</th>
                      <th>Hedged with</th>
                      <th>LP panel hit</th>
                      <th className={styles.num}>Internal</th>
                      <th className={styles.num}>External</th>
                      <th className={styles.num}>Residual</th>
                      <th className={styles.num}>Hedge px</th>
                      <th className={styles.num}>Mid</th>
                      <th className={styles.num}>Slippage</th>
                      <th>Mode</th>
                    </tr>
                  </thead>
                  <tbody>
                    {filtered.map((p) => (
                      <tr key={p.hedgeId} data-testid={`hedge-deal-row-${p.hedgeId}`}>
                        <td className={styles.mono}>{timeOf(p.firedAt)}</td>
                        <td className={styles.strong}>
                          {p.book} · {p.instrument}
                        </td>
                        <td className={styles.mono} data-testid={`hedge-parent-${p.hedgeId}`}>
                          {p.parentPositionId !== undefined ? (
                            <span className={styles.parentLink}>{parentDealLabel(p)}</span>
                          ) : (
                            <span className={styles.muted}>—</span>
                          )}
                        </td>
                        <td>
                          <span className={`${styles.rag} ${styles[`rag_${ragKey(p.band)}`]}`}>
                            {p.band.toUpperCase()}
                          </span>
                        </td>
                        <td>{describeExitAction(p.action)}</td>
                        <td>
                          {p.lpWon ? (
                            <span className={styles.lpWon} data-testid={`hedge-lpwon-${p.hedgeId}`}>
                              {p.lpWon}
                            </span>
                          ) : (
                            <span className={styles.muted}>internal / no-trade</span>
                          )}
                        </td>
                        <td>
                          {p.lps.length > 0 ? (
                            <span className={styles.lpChips}>
                              {p.lps.map((lp) => (
                                <span
                                  key={lp}
                                  className={`${styles.lpChip} ${lp === p.lpWon ? styles.lpChipWon : ""}`}
                                >
                                  {lp}
                                </span>
                              ))}
                            </span>
                          ) : (
                            <span className={styles.muted}>—</span>
                          )}
                        </td>
                        <td className={`${styles.num} ${styles.mono}`}>{fmtCompact(p.internalCrossed)}</td>
                        <td className={`${styles.num} ${styles.mono}`}>{fmtCompact(p.externalHedged)}</td>
                        <td className={`${styles.num} ${styles.mono}`}>{fmtCompact(p.residual)}</td>
                        <td className={`${styles.num} ${styles.mono} ${styles.px}`}>
                          {p.hedgePrice > 0 ? fmtRate(p.hedgePrice) : "—"}
                        </td>
                        <td className={`${styles.num} ${styles.mono}`}>
                          {p.midAtFire > 0 ? fmtRate(p.midAtFire) : "—"}
                        </td>
                        <td className={`${styles.num} ${styles.mono}`}>{slippageLabel(p.slippageBp)}</td>
                        <td>
                          {p.advisory ? (
                            <span className={styles.advisory}>ADVISORY</span>
                          ) : (
                            <span className={styles.live}>LIVE</span>
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </>
        )}
      </Panel>
    </div>
  );
}
