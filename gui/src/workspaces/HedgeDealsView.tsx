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
import { useEffect, useMemo, useRef, useState } from "react";

import { useApp } from "../app/AppContext";
import { DataTable } from "../components/DataTable";
import { Panel } from "../components/Panel";
import { TableSearch } from "../components/TableSearch";
import { TableSkeleton } from "../components/TableSkeleton";
import { useGridState } from "../hooks/useGridState";
import { useTableFilter } from "../hooks/useTableFilter";
import { useCachedResource } from "../hooks/useCachedResource";
import { useTableUiState } from "../hooks/useTableUiState";
import type { ColumnDef } from "../lib/grid";
import { fmtCompact, fmtRate } from "../lib/format";
import { describeExitAction, isExternalExitAction } from "../lib/hedgeExit";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import type { HedgeProvenance } from "../data/contract";
import type { StreetOrder, StreetOrdersView } from "../data/contract";
import { TradeDetailsModal } from "../components/TradeDetailsModal";
import { hedgeTradeDetails } from "../lib/tradeDetails";
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

/**
 * Which leg of the flow a row contributed to — the three totals the Hedge flow band
 * shows, used as a drill-down filter. `null` (the default) means no leg is selected,
 * which leaves the blotter's own hedging-only desk filter in charge.
 */
export type HedgeLeg = "crossed" | "hedged" | "warehoused";

/**
 * Does this record contribute to `leg`? The Hedge flow tiles are sums of exactly these
 * fields, so this predicate is what keeps a total and the rows behind it in agreement.
 * Exported so that correspondence is testable rather than assumed.
 */
export function contributesTo(p: HedgeProvenance, leg: HedgeLeg): boolean {
  switch (leg) {
    case "crossed":
      return p.internalCrossed > 0;
    case "hedged":
      return p.externalHedged > 0;
    case "warehoused":
      return p.residual > 0;
  }
}


export function HedgeDealsView({
  legFilter = null,
}: {
  /**
   * Show only the rows behind one Hedge flow total. Optional, so the standalone
   * mounts (Risk dashboard, Deals blotter) are unaffected.
   */
  legFilter?: HedgeLeg | null;
} = {}): React.ReactElement {
  const app = useApp();
  const canView = app.auth.can("hedge", "fixed_income");

  // Persisted table UI state: the "show internalised" toggle and the search query
  // survive a tab switch (the workspace unmounting) and are restored on return.
  const [ui, setUi] = useTableUiState("fi-hedge-deals", {
    showInternalised: false,
    query: "",
  });
  // The hedge desk is hedging-only by default: internalised (warehouse / cross-internal
  // / skew / escalate) decisions are hidden unless the trader opts to audit them.
  const showInternalised = ui.showInternalised;

  // Stale-while-revalidate cache: the fired-hedge ledger SURVIVES the workspace
  // unmounting on a tab switch, so returning to it shows the rows instantly (no blank
  // flash) while a background revalidation refreshes them. Cache MISS shows a skeleton.
  const {
    data,
    isLoading,
    error: fetchError,
    refresh,
  } = useCachedResource<HedgeProvenance[]>(
    "hedgeProvenance",
    () => app.transport.listHedgeProvenance(),
    { enabled: canView },
  );
  const rows = useMemo(() => data ?? [], [data]);
  const error =
    fetchError === undefined || fetchError === null
      ? null
      : fetchError instanceof Error
        ? fetchError.message
        : "failed to load hedge provenance";

  /**
   * The street orders EVERY fired hedge produced, indexed by the hedge that produced
   * them.
   *
   * The ledger above records the hedge DECISION — the band, the action, what was
   * internalised, what was externalised, the residual. It has never shown the orders
   * that decision actually put on the wire: which provider was asked, for how much, and
   * what came back. That is the half a desk needs when a hedge does not reduce the book,
   * because "external 0" is a symptom and the reject reason is the cause.
   *
   * Fetched unfiltered and indexed here rather than per-row on demand: the join key is
   * already on the order (`parentHedgeId`), one request serves every row, and a
   * per-row fetch would issue one round trip per expand on a ledger that streams.
   */
  const { data: streetData } = useCachedResource<StreetOrdersView>(
    "hedgeStreetOrders",
    () => app.transport.listStreetOrders(),
    { enabled: canView },
  );
  const ordersByHedge = useMemo(() => {
    const index = new Map<string, StreetOrder[]>();
    for (const o of streetData?.orders ?? []) {
      const parent = o.parentHedgeId;
      if (parent === undefined || parent === "") continue;
      const bucket = index.get(parent);
      if (bucket === undefined) index.set(parent, [o]);
      else bucket.push(o);
    }
    // Oldest first WITHIN a hedge: a shed walks its panel in rank order, and reading the
    // refusals in the order they happened is what shows how far down it got.
    for (const bucket of index.values()) {
      bucket.sort((a, b) => Number(a.tsNanos - b.tsNanos));
    }
    return index;
  }, [streetData]);

  /** The hedge whose orders are expanded, or `null`. */
  const [openHedgeId, setOpenHedgeId] = useState<string | null>(null);

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  // Refresh on each live hedge-intent tick (a fire appends provenance), mirroring the
  // Hedging monitor — a background revalidate that keeps the current rows visible
  // rather than a raw refetch that would clear the table.
  useEffect(() => {
    if (!canView) return;
    const stream = app.transport.streamHedgeIntents;
    const dispose =
      typeof stream === "function"
        ? stream.call(app.transport, () => refreshRef.current())
        : undefined;
    return () => dispose?.();
  }, [app.transport, canView]);

  // Newest first, mirroring the client-deals blotter; then apply the hedging-only
  // desk filter (external-only unless "Show internalised" is on) BEFORE search.
  const sorted = useMemo(() => [...rows].sort((a, b) => b.firedAt - a.firedAt), [rows]);
  // A selected Hedge flow total REPLACES the desk filter rather than stacking with it.
  // The tiles are sums over every live record, so filtering the external-only view by
  // `crossed` would show an empty table under a non-zero total — the rows behind an
  // internal cross are internalised by definition. Selecting a leg means "show me the
  // rows behind THIS number", so it must be able to reach all of them.
  const visible = useMemo(() => {
    if (legFilter !== null) return sorted.filter((p) => contributesTo(p, legFilter));
    return showInternalised ? sorted : sorted.filter(isExternalHedge);
  }, [sorted, showInternalised, legFilter]);
  const { query, setQuery, filtered } = useTableFilter(
    visible,
    hedgeSearchText,
    { query: ui.query, setQuery: (q) => setUi({ query: q }) },
  );

  // The column model. `accessor` is the canonical TEXT projection used for
  // search/filter/export; `cell` carries the RAG chips, LP chips and provenance
  // treatments the hand-rolled table had; `sortValue` is the ORDERED projection
  // (an amount must sort by its magnitude, not by its compact "70m" label).
  const columns = useMemo<ReadonlyArray<ColumnDef<HedgeProvenance>>>(
    () => [
      {
        key: "time",
        header: "Time",
        width: 90,
        align: "left",
        accessor: (p) => timeOf(p.firedAt),
        cell: (p) => <span className={styles.mono}>{timeOf(p.firedAt)}</span>,
        // Order by the epoch stamp, not the rendered 24h clock (which wraps).
        sortValue: (p) => p.firedAt,
        sortKey: "time",
        filter: { kind: "text" },
      },
      {
        key: "bookInstrument",
        header: "Book · Instrument",
        width: 190,
        align: "left",
        accessor: (p) => `${p.book} · ${p.instrument}`,
        cell: (p) => (
          <span className={styles.strong}>
            {p.book} · {p.instrument}
          </span>
        ),
        sortKey: "bookInstrument",
        filter: { kind: "select" },
      },
      {
        key: "parent",
        header: "Parent deal",
        width: 100,
        align: "left",
        accessor: (p) => parentDealLabel(p),
        cell: (p) => (
          <span className={styles.mono} data-testid={`hedge-parent-${p.hedgeId}`}>
            {p.parentPositionId !== undefined ? (
              <span className={styles.parentLink}>{parentDealLabel(p)}</span>
            ) : (
              <span className={styles.muted}>—</span>
            )}
          </span>
        ),
        // `parentPositionId` is a bigint on the wire; a book-level advisory
        // record carries none, and NaN sorts LAST rather than heading an
        // ascending sort as a fabricated zero would.
        sortValue: (p) => (p.parentPositionId !== undefined ? Number(p.parentPositionId) : Number.NaN),
        sortKey: "parent",
        filter: { kind: "text" },
      },
      {
        key: "band",
        header: "Band",
        width: 90,
        align: "left",
        accessor: (p) => p.band.toUpperCase(),
        cell: (p) => (
          <span className={`${styles.rag} ${styles[`rag_${ragKey(p.band)}`]}`}>
            {p.band.toUpperCase()}
          </span>
        ),
        sortKey: "band",
        filter: { kind: "select" },
      },
      {
        key: "action",
        header: "Action",
        width: 170,
        align: "left",
        accessor: (p) => describeExitAction(p.action),
        sortKey: "action",
        filter: { kind: "select" },
      },
      {
        key: "lpWon",
        header: "Hedged with",
        width: 130,
        align: "left",
        accessor: (p) => p.lpWon ?? "internal / no-trade",
        cell: (p) =>
          p.lpWon ? (
            <span className={styles.lpWon} data-testid={`hedge-lpwon-${p.hedgeId}`}>
              {p.lpWon}
            </span>
          ) : (
            <span className={styles.muted}>internal / no-trade</span>
          ),
        sortKey: "lpWon",
        filter: { kind: "select" },
      },
      {
        key: "panel",
        header: "Panel (eligible)",
        description:
          "The LPs the exit policy made ELIGIBLE for this hedge — not the LPs that quoted or filled. The fill's actual venue is the “Hedged with” column.",
        width: 190,
        align: "left",
        accessor: (p) => p.lps.join(" "),
        cell: (p) =>
          p.lps.length > 0 ? (
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
          ),
        filter: { kind: "text" },
      },
      {
        key: "internal",
        header: "Internal",
        width: 100,
        align: "right",
        accessor: (p) => fmtCompact(p.internalCrossed),
        sortValue: (p) => p.internalCrossed,
        sortKey: "internal",
        filter: { kind: "range" },
      },
      {
        key: "external",
        header: "External",
        width: 100,
        align: "right",
        accessor: (p) => fmtCompact(p.externalHedged),
        sortValue: (p) => p.externalHedged,
        sortKey: "external",
        filter: { kind: "range" },
      },
      {
        key: "residual",
        header: "Residual",
        width: 100,
        align: "right",
        accessor: (p) => fmtCompact(p.residual),
        sortValue: (p) => p.residual,
        sortKey: "residual",
        filter: { kind: "range" },
      },
      {
        key: "hedgePx",
        header: "Hedge px",
        width: 100,
        align: "right",
        accessor: (p) => (p.hedgePrice > 0 ? fmtRate(p.hedgePrice) : "—"),
        cell: (p) => (
          <span className={styles.px}>{p.hedgePrice > 0 ? fmtRate(p.hedgePrice) : "—"}</span>
        ),
        // A no-trade action has no price; NaN sorts LAST rather than as a zero.
        sortValue: (p) => (p.hedgePrice > 0 ? p.hedgePrice : Number.NaN),
        sortKey: "hedgePx",
        filter: { kind: "range" },
      },
      {
        key: "mid",
        header: "Mid",
        width: 100,
        align: "right",
        accessor: (p) => (p.midAtFire > 0 ? fmtRate(p.midAtFire) : "—"),
        sortValue: (p) => (p.midAtFire > 0 ? p.midAtFire : Number.NaN),
        sortKey: "mid",
        filter: { kind: "range" },
      },
      {
        key: "slippage",
        header: "Slippage",
        width: 100,
        align: "right",
        accessor: (p) => slippageLabel(p.slippageBp),
        sortValue: (p) => p.slippageBp,
        sortKey: "slippage",
        filter: { kind: "range" },
      },
      {
        key: "mode",
        header: "Mode",
        width: 100,
        align: "left",
        accessor: (p) => (p.advisory ? "ADVISORY" : "LIVE"),
        cell: (p) =>
          p.advisory ? (
            <span className={styles.advisory}>ADVISORY</span>
          ) : (
            <span className={styles.live}>LIVE</span>
          ),
        sortKey: "mode",
        filter: { kind: "select" },
      },
      {
        key: "orders",
        header: "Orders sent",
        description:
          "The street orders this hedge decision actually put on the wire — how many were sent, and how many came back filled. Open a row to see each one: the provider, the size asked for, and the reason any of them declined.",
        width: 150,
        align: "left",
        // The canonical text carries the reject reasons too, so searching the ledger for
        // "whole lot" surfaces the HEDGES whose orders were refused for it — not just the
        // orders themselves on a different screen.
        accessor: (p) => {
          const orders = ordersByHedge.get(p.hedgeId) ?? [];
          if (orders.length === 0) return "";
          const filled = orders.filter((o) => o.filledQty > 0).length;
          const reasons = orders.map((o) => o.reason ?? "").filter((r) => r !== "");
          return `${orders.length} sent ${filled} filled ${reasons.join(" ")}`;
        },
        sortValue: (p) => (ordersByHedge.get(p.hedgeId) ?? []).length,
        sortKey: "orders",
        cell: (p) => {
          const orders = ordersByHedge.get(p.hedgeId) ?? [];
          if (orders.length === 0) {
            // An internalised decision never asks the street, so it has no orders BY
            // CONSTRUCTION — that is a fact, not missing data, and must not read as a
            // failed load.
            return (
              <button
                type="button"
                className={styles.ordersNone}
                data-testid={`hedge-orders-toggle-${p.hedgeId}`}
                title={
                  isExternalHedge(p)
                    ? "This hedge externalised, but no street order was recorded against it."
                    : "Internalised — nothing was sent to the street."
                }
                // Still openable: a hedge with no orders STILL has a decision worth
                // reading, and that is exactly the case a desk needs to inspect.
                onClick={() => setOpenHedgeId(p.hedgeId)}
              >
                {isExternalHedge(p) ? "none recorded" : "—"}
              </button>
            );
          }
          const filled = orders.filter((o) => o.filledQty > 0).length;
          return (
            <button
              type="button"
              className={styles.ordersToggle}
              // Opens the SHARED trade-details modal — the same surface, and the same
              // affordance, the client blotter opens for a client fill.
              data-testid={`hedge-orders-toggle-${p.hedgeId}`}
              onClick={() => setOpenHedgeId(p.hedgeId)}
            >
              {orders.length} sent
              <span className={filled > 0 ? styles.ordersFilled : styles.ordersUnfilled}>
                {filled} filled
              </span>
            </button>
          );
        },
      },
    ],
    [ordersByHedge, openHedgeId],
  );

  const grid = useGridState<HedgeProvenance>({
    tableId: "fi-hedge-deals",
    columns,
    rows: filtered,
    allRows: visible,
  });

  const openHedge = useMemo(
    () => (openHedgeId === null ? undefined : rows.find((p) => p.hedgeId === openHedgeId)),
    [rows, openHedgeId],
  );

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
              onChange={(e) => setUi({ showInternalised: e.target.checked })}
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
        {isLoading ? (
          <TableSkeleton label="Loading hedge deals…" />
        ) : visible.length === 0 ? (
          <p className={styles.empty}>
            {rows.length === 0
              ? "No fired hedges yet — as the auto-hedge engine sheds warehoused risk, each external execution (the LP/composite we hit, its price, the amount) appears here."
              : `No external hedges yet — ${internalisedCount} internalised decision${internalisedCount === 1 ? "" : "s"} ${internalisedCount === 1 ? "is" : "are"} hidden. Toggle “Show internalised” to audit ${internalisedCount === 1 ? "it" : "them"}.`}
          </p>
        ) : (
          <>
            {/* "N of M" counts the survivors of BOTH pipeline stages (global
                search AND the per-column filters), never the search alone. */}
            <TableSearch
              query={query}
              onQueryChange={setQuery}
              shown={grid.shown}
              total={grid.total}
              label="Search hedge deals"
              placeholder="Filter hedges…"
            />
            <DataTable
              label="Hedge deals"
              columns={columns}
              grid={grid}
              rowKey={(p) => p.hedgeId}
              hideRowCount
              rowProps={(p) => ({ "data-testid": `hedge-deal-row-${p.hedgeId}` })}
              emptyState={
                query.trim() === ""
                  ? "No hedges match the current column filters."
                  : `No hedges match “${query}”.`
              }
            />

          </>
        )}
      </Panel>
      <TradeDetailsModal
        details={
          openHedge === undefined ? null : hedgeTradeDetails(openHedge, ordersByHedge.get(openHedge.hedgeId) ?? [])
        }
        onClose={() => setOpenHedgeId(null)}
      />
    </div>
  );
}
