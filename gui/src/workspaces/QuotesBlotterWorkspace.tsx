/**
 * QuotesBlotterWorkspace — the Quotes LENS of the unified, one-per-book
 * `BookWorkspace`; the Book's `Quotes` view opens this lens. It is the desk's
 * SHOWN-QUOTES blotter (fixed-income): every RFQ/IOI the desk has priced and
 * quoted — whether the trader clicked Send or the desk auto-quoted it — newest
 * first: received time, counterparty, instrument, side, quoted rate, notional the
 * quote is good for, the good-for validity window, the pricing trader, and the
 * lifecycle state (QUOTED while live, ACCEPTED once lifted).
 *
 * It is the read-only sibling of the Deals blotter: Deals shows what was BOOKED
 * (accepted quotes → deals), Quotes shows what was SHOWN (every quote, including
 * those still live or that lapsed). Both are lenses of the one Book.
 *
 * One contract, two transports (GUI-DESIGN §6.2): the blotter talks ONLY to the
 * `CelnetTransport.listDeskRequests` seam — the SAME desk-requests source the
 * QuotingWorkspace inbox reads — so the SAME quotes render through the
 * deterministic in-app source and the live `RfqDeskService.ListDeskRequests` edge,
 * scoped to the caller's desk via the same principal. It refreshes on every push
 * `Notification` (a quote sent/auto-quoted/accepted mutates a request), so a shown
 * quote appears live without polling churn.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import { useAcceptanceSeed } from "../app/AcceptanceSeedContext";
import { DataTable } from "../components/DataTable";
import { Panel } from "../components/Panel";
import { FlowRowContextMenu, type FlowRowMenuTarget } from "../components/FlowRowContextMenu";
import { TableSearch } from "../components/TableSearch";
import { TableSkeleton } from "../components/TableSkeleton";
import { useGridState } from "../hooks/useGridState";
import { useTableFilter } from "../hooks/useTableFilter";
import { cacheKeyPart, useCachedResource } from "../hooks/useCachedResource";
import { useTableUiState } from "../hooks/useTableUiState";
import type { ColumnDef } from "../lib/grid";
import { principalForScope } from "../data/riskView";
import { fmtRate, fmtClock, fmtCompact } from "../lib/format";
import { sideLabel } from "./QuotingWorkspace";
import type { DeskRequest } from "../data/contract";
import { capabilityAssetForDomain, deskRequestAsset } from "../data/assetClass";
import styles from "./QuotesBlotterWorkspace.module.css";

/** All of a shown quote's user-visible textual fields, concatenated for search. */
function quoteSearchText(r: DeskRequest): string {
  return [
    fmtClock(r.receivedAtNanos),
    r.counterparty,
    r.desk,
    r.kind,
    `${r.instrument.tenorYears}y OIS`,
    r.curveSet.currency,
    sideLabel(r.side),
    r.quote ? fmtRate(r.quote.price) : "",
    fmtCompact(r.quote?.notional ?? r.notional),
    r.quote ? `${Math.round(r.quote.validForMs / 1000)}s` : "",
    r.quote?.trader ?? "",
    r.state,
  ].join(" ");
}

/** The state badge class for a quoted-request lifecycle state. */
function stateClass(state: DeskRequest["state"]): string {
  if (state === "ACCEPTED") return styles.stateAccepted ?? "";
  return styles.stateQuoted ?? "";
}

/** A request carries a shown quote iff it is QUOTED or ACCEPTED (`quote` present). */
function hasShownQuote(r: DeskRequest): boolean {
  return r.quote !== undefined && (r.state === "QUOTED" || r.state === "ACCEPTED");
}

export function QuotesBlotterWorkspace(): React.ReactElement {
  const app = useApp();
  const principal = useMemo(() => principalForScope(app.scope), [app.scope]);
  // Hard asset separation: the Book's Quotes lens shows ONLY the active domain's
  // asset class. Desk quotes are structurally OIS (rates), so under the FX Options
  // domain this lens is correctly empty and under Fixed Income it shows them.
  const activeAsset = capabilityAssetForDomain(app.activeDomain);

  const seed = useAcceptanceSeed();
  const canManageAcceptance = app.auth.can("manage_acceptance", "fixed_income");

  // The row context menu (right-click / ⋯ kebab): the counterparty + anchor point of the
  // row whose "Create acceptance rule" the trader is spawning.
  const [rowMenu, setRowMenu] = useState<FlowRowMenuTarget | null>(null);
  // Persisted search query — survives a tab switch and is restored on return.
  const [ui, setUi] = useTableUiState("fi-quotes-blotter", { query: "" });

  // Stale-while-revalidate cache keyed on the entitlement principal (scope): the shown
  // quotes SURVIVE the workspace unmounting on a tab switch, so returning shows them
  // instantly (no blank flash) while a background revalidation refreshes them.
  const {
    data,
    isLoading,
    error: fetchError,
    refresh,
  } = useCachedResource<DeskRequest[]>(
    `deskRequests|${cacheKeyPart(principal)}`,
    () =>
      app.transport
        .listDeskRequests({ ...(principal ? { principal } : {}) })
        .then((res) => res.requests),
  );
  const requests = useMemo(() => data ?? [], [data]);
  const error =
    fetchError === undefined || fetchError === null
      ? null
      : fetchError instanceof Error
        ? fetchError.message
        : "failed to load quotes";

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  // Refresh on every push Notification (a quote sent/auto-quoted/accepted mutates a
  // request) — a background revalidate that keeps the current rows visible.
  useEffect(() => {
    const dispose = app.transport.streamNotifications(undefined, () => refreshRef.current());
    return dispose;
  }, [app.transport]);

  // Only shown quotes (QUOTED / ACCEPTED) for the ACTIVE domain's asset class,
  // newest received first. The asset filter runs BEFORE the search filter below.
  const quotes = useMemo(
    () =>
      requests
        .filter(hasShownQuote)
        .filter((r) => deskRequestAsset(r) === activeAsset)
        .slice()
        .sort((a, b) =>
          b.receivedAtNanos > a.receivedAtNanos
            ? 1
            : b.receivedAtNanos < a.receivedAtNanos
              ? -1
              : 0,
        ),
    [requests, activeAsset],
  );

  const { query, setQuery, filtered } = useTableFilter(
    quotes,
    quoteSearchText,
    { query: ui.query, setQuery: (q) => setUi({ query: q }) },
  );

  // The column model. `accessor` stays the canonical TEXT projection (search,
  // filter, export) even where `cell` renders a chip, so matching never has to
  // reason about markup; `sortValue` is the ORDERED projection so a rate sorts
  // numerically rather than lexically.
  const columns = useMemo<ReadonlyArray<ColumnDef<DeskRequest>>>(
    () => [
      {
        key: "received",
        header: "Received",
        width: 90,
        align: "left",
        accessor: (r) => fmtClock(r.receivedAtNanos),
        cell: (r) => <span className={styles.mono}>{fmtClock(r.receivedAtNanos)}</span>,
        // Order by the raw nanosecond stamp, not the rendered clock text — the
        // clock wraps at midnight and would sort a new session before the old.
        sortValue: (r) => Number(r.receivedAtNanos),
        sortKey: "received",
        filter: { kind: "text" },
      },
      {
        key: "counterparty",
        header: "Counterparty",
        width: 150,
        align: "left",
        accessor: (r) => r.counterparty,
        cell: (r) => <span className={styles.strong}>{r.counterparty}</span>,
        sortKey: "counterparty",
        filter: { kind: "select" },
      },
      {
        key: "desk",
        header: "Desk",
        width: 110,
        align: "left",
        accessor: (r) => r.desk,
        sortKey: "desk",
        filter: { kind: "select" },
      },
      {
        key: "instrument",
        header: "Instrument",
        width: 150,
        align: "left",
        accessor: (r) => `${r.kind} ${r.instrument.tenorYears}y OIS`,
        cell: (r) => (
          <>
            <span
              className={`${styles.kind} ${r.kind === "IOI" ? styles.kindIoi : styles.kindRfq}`}
            >
              {r.kind}
            </span>
            {r.instrument.tenorYears}y OIS
          </>
        ),
        sortValue: (r) => r.instrument.tenorYears,
        sortKey: "instrument",
        filter: { kind: "select" },
      },
      {
        key: "ccy",
        header: "Ccy",
        width: 70,
        align: "left",
        accessor: (r) => r.curveSet.currency,
        cell: (r) => <span className={styles.mono}>{r.curveSet.currency}</span>,
        sortKey: "ccy",
        filter: { kind: "select" },
      },
      {
        key: "side",
        header: "Side",
        width: 110,
        align: "left",
        accessor: (r) => sideLabel(r.side),
        sortKey: "side",
        filter: { kind: "select" },
      },
      {
        key: "price",
        header: "Quoted rate",
        width: 110,
        align: "right",
        accessor: (r) => (r.quote ? fmtRate(r.quote.price) : "—"),
        cell: (r) => (
          <span className={styles.price}>{r.quote ? fmtRate(r.quote.price) : "—"}</span>
        ),
        // An unquoted request has no price; NaN sorts LAST in both directions
        // rather than pretending to be zero (which would head an ascending sort).
        sortValue: (r) => r.quote?.price ?? Number.NaN,
        sortKey: "price",
        filter: { kind: "range" },
      },
      {
        key: "notional",
        header: "Notional",
        width: 110,
        align: "right",
        accessor: (r) => fmtCompact(r.quote?.notional ?? r.notional),
        sortValue: (r) => r.quote?.notional ?? r.notional,
        sortKey: "notional",
        filter: { kind: "range" },
      },
      {
        key: "goodFor",
        header: "Good for",
        width: 90,
        align: "right",
        accessor: (r) => (r.quote ? `${Math.round(r.quote.validForMs / 1000)}s` : "—"),
        sortValue: (r) => r.quote?.validForMs ?? Number.NaN,
        sortKey: "goodFor",
        filter: { kind: "range" },
      },
      {
        key: "trader",
        header: "Trader",
        width: 110,
        align: "left",
        accessor: (r) => r.quote?.trader ?? "—",
        sortKey: "trader",
        filter: { kind: "select" },
      },
      {
        key: "state",
        header: "State",
        width: 110,
        align: "left",
        accessor: (r) => r.state,
        cell: (r) => <span className={`${styles.state} ${stateClass(r.state)}`}>{r.state}</span>,
        sortKey: "state",
        filter: { kind: "select" },
      },
      {
        key: "actions",
        header: "Actions",
        width: 76,
        align: "center",
        // Previously a visually-hidden "Row actions" label. `ColumnDef.header` is
        // a plain string (it is reused verbatim in the filter controls' ARIA
        // labels), so the label is now VISIBLE and shortened to fit the track —
        // an empty <th> is an axe violation and an unnamed column for AT.
        accessor: () => "",
        cell: (r) => (
          <button
            type="button"
            className={styles.kebab}
            aria-haspopup="menu"
            aria-label={`Row actions for ${r.counterparty}`}
            data-testid="quote-row-kebab"
            onClick={(e) => {
              const rect = e.currentTarget.getBoundingClientRect();
              setRowMenu({ counterparty: r.counterparty, x: rect.left, y: rect.bottom });
            }}
          >
            <span aria-hidden>⋯</span>
          </button>
        ),
      },
    ],
    [],
  );

  const grid = useGridState<DeskRequest>({
    tableId: "fi-quotes-blotter",
    columns,
    rows: filtered,
    allRows: quotes,
  });

  const isOffline = !app.transport.label.startsWith("live");
  const totalQuoted = quotes.reduce((acc, r) => acc + (r.quote?.notional ?? 0), 0);

  return (
    <div className={styles.wrap}>
      <Panel className={styles.panel} title="Shown quotes">
        <div className={styles.head}>
          <span className={styles.engine}>{isOffline ? "in-app desk" : "live desk"}</span>
          <span className={styles.summary}>
            {quotes.length} quote{quotes.length === 1 ? "" : "s"} ·{" "}
            {fmtCompact(totalQuoted)} quoted
          </span>
        </div>
        {error && (
          <p className={styles.error} role="alert">
            {error}
          </p>
        )}
        {isLoading ? (
          <TableSkeleton label="Loading quotes…" />
        ) : quotes.length === 0 ? (
          <p className={styles.empty}>
            {activeAsset === "fixed_income"
              ? "No quotes shown yet — price a request in the Quoting workspace to show one."
              : "No FX-option desk quotes — the RFQ desk quotes rates (OIS). Switch to the Fixed Income domain to see shown quotes."}
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
              label="Search quotes"
              placeholder="Filter quotes…"
            />
            <DataTable
              label="Shown quotes"
              columns={columns}
              grid={grid}
              rowKey={(r) => r.requestId}
              hideRowCount
              rowProps={(r) => ({
                onContextMenu: (e) => {
                  e.preventDefault();
                  setRowMenu({ counterparty: r.counterparty, x: e.clientX, y: e.clientY });
                },
              })}
              emptyState={
                query.trim() === ""
                  ? "No quotes match the current column filters."
                  : `No quotes match “${query}”.`
              }
            />
          </>
        )}
      </Panel>
      <FlowRowContextMenu
        target={rowMenu}
        onClose={() => setRowMenu(null)}
        onCreateAcceptanceRule={(cp) => {
          // Seed the rule, then navigate to the Acceptance surface (the `acceptance` alias
          // → the consolidated Risk host's Acceptance tab), which consumes the seed.
          seed.requestAcceptanceSeed(cp);
          app.setWorkspace("acceptance");
        }}
        canManageAcceptance={canManageAcceptance}
      />
    </div>
  );
}
