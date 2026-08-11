/**
 * DealsBlotterWorkspace — the Deals LENS of the unified, one-per-book
 * `BookWorkspace` (`fe-fi-migration` #4); the `deals` rail row opens the Book on
 * this lens. Behaviour is unchanged — it is still the received-deals blotter
 * (fixed-income). Every deal booked by an accepted desk quote, newest first:
 * counterparty, instrument, notional, dealt price, side, booking trader and
 * execution time.
 *
 * One contract, two transports (GUI-DESIGN §6.2): the blotter talks ONLY to the
 * `CelnetTransport.listDeals` seam, so the SAME deals render through the
 * deterministic in-app source and the live `RfqDeskService.ListDeals` edge. It
 * refreshes on every push `Notification` (a `QUOTE_ACCEPTED` mints a deal), so a
 * fill appears live without polling churn.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import { useAcceptanceSeed } from "../app/AcceptanceSeedContext";
import { useHedgeSeed } from "../app/HedgeSeedContext";
import { hedgeSeedFromDeal } from "../lib/hedgeSeed";
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
import type { Deal, Internalise, Side } from "../data/contract";
import { capabilityAssetForDomain, dealAsset } from "../data/assetClass";
import { fmtEdgeBps, hedgeBandLabel, internaliseLabel } from "../lib/internalise";
import { DealTicket } from "./DealTicket";
import { HedgeDealsView } from "./HedgeDealsView";
import styles from "./DealsBlotterWorkspace.module.css";

/** Which lens of the Deals blotter is shown: the client fills, or the executed hedges. */
type DealsLens = "client" | "hedge";

/**
 * How the blotter exposes its two lenses. `"both"` (the default — the Book's Deals
 * lens) shows the Client-deals / Hedge-deals TOGGLE and remembers the last-used lens.
 * `"client"` / `"hedge"` FORCE that one lens and hide the toggle — used to surface the
 * blotter as a DEDICATED Risk tab (a standalone Client blotter), where the hedge
 * ledger is its own separate tab so a lens toggle would be redundant.
 */
type DealsLensMode = DealsLens | "both";

/**
 * The rates pay/receive-fixed reading of a `Side` — the SECONDARY detail kept
 * alongside the primary BUY/SELL badge. The wire `Side` already IS the buy/sell
 * axis (contract: `SIDE_BUY` pays fixed / is long the swap, `SIDE_SELL` receives
 * fixed), so this only spells out the rates convention: BUY → "Pay fixed",
 * SELL → "Receive fixed".
 */
function sideLabel(side: Side): string {
  if (side === "BUY") return "Pay fixed";
  if (side === "SELL") return "Receive fixed";
  return "Two-way";
}

/** The primary BUY / SELL / 2-WAY read of a deal's `Side` (native to the wire). */
function sideBuySell(side: Side): string {
  if (side === "BUY") return "BUY";
  if (side === "SELL") return "SELL";
  return "2-WAY";
}

/**
 * The BUY / SELL indicator per deal row — the PRIMARY read of `Side`, tinted green
 * (bid) for a buy and red (offer) for a sell via the shared trading semantic
 * tokens, with the rates pay/receive-fixed detail kept as secondary text (and the
 * accessible label). Rates deals map long-the-swap = BUY (pay fixed); FX / other
 * deals use their native `Side` directly. A two-way quote stays neutral.
 */
function SideBadge({ side }: { readonly side: Side }): React.ReactElement {
  const tag = sideBuySell(side);
  const detail = sideLabel(side);
  const label = `${tag} · ${detail}`;
  return (
    <span className={styles.side} data-side={side} aria-label={label} title={label}>
      <span className={styles.sideTag}>{tag}</span>
      <span className={styles.sideDetail}>{detail}</span>
    </span>
  );
}

/**
 * The product family a booked deal carries — its decoded `RatesInstrument` oneof arm
 * ({@link Deal.productKind}): `OIS` / `IRS` / `FRA` / `BOND`. Surfaced honestly off the
 * discriminant the codec threads through, so a mixed book classifies each fill by its
 * real family rather than assuming OIS.
 */
function productLabel(d: Deal): string {
  return d.productKind;
}

/**
 * The CSS chip class for a deal's request kind: RFQ (accent), IOI (warn), or ESP —
 * an executable streaming-price lift, tinted with the bid/streaming token so it reads
 * distinctly from the request-driven flows.
 */
function kindClass(kind: Deal["kind"]): string {
  if (kind === "IOI") return styles.kindIoi ?? "";
  if (kind === "ESP") return styles.kindEsp ?? "";
  return styles.kindRfq ?? "";
}

/**
 * The security descriptor a BOND deal renders in the SECURITY cell — the human
 * `display_name` when the bond is in the curated refdata (matching the Agg Book tile),
 * else the raw `instrument_id`, else empty. OIS/IRS/FRA carry no security identity.
 */
function securityDescriptor(d: Deal): string {
  return d.bondDisplayName ?? d.bondSecurityId ?? "";
}

/**
 * The SECURITY cell — for a BOND fill, the human descriptor over the stable security
 * id (matching the Agg Book tile); the descriptor falls back to the raw id when the
 * bond is outside the curated refdata (never blank/fabricated). OIS/IRS/FRA rows have
 * no security identity, so they render a plain em dash.
 */
function SecurityCell({ deal }: { readonly deal: Deal }): React.ReactElement {
  const descriptor = securityDescriptor(deal);
  if (descriptor.length === 0) {
    return <span className={styles.securityNone}>—</span>;
  }
  const id = deal.bondSecurityId ?? "";
  // Show the id sub-line only when it exists AND is not already the descriptor (a
  // refdata-less bond shows the id as the descriptor, so avoid repeating it).
  const showId = id.length > 0 && id !== descriptor;
  const label = showId ? `${descriptor} · ${id}` : descriptor;
  return (
    <span className={styles.security} aria-label={label} title={label}>
      <span className={styles.securityName}>{descriptor}</span>
      {showId && <span className={styles.securityId}>{id}</span>}
    </span>
  );
}

/** The tenor a rates deal carries, as a compact `Ny` label (e.g. `10y`). */
function tenorLabel(d: Deal): string {
  return `${d.instrument.tenorYears}y`;
}

/** The booked position id the fill landed in the ledger Book, or `—` when none. */
function positionLabel(d: Deal): string {
  return d.positionId !== undefined ? `#${d.positionId.toString()}` : "—";
}

/**
 * The Risk Portfolio (risk book) the fill's risk routed into, resolved id → the
 * portfolio's human NAME via the `listRiskBooks` roster. Falls back to the raw id
 * when the roster does not carry it (a routed-but-since-renamed book), and `—` when
 * the fill routed to no portfolio (unrouted — no graph / routing fall-back).
 */
function riskPortfolioLabel(d: Deal, names: ReadonlyMap<string, string>): string {
  if (d.riskBookId === undefined) return "—";
  return names.get(d.riskBookId) ?? d.riskBookId;
}

/**
 * A compact inline badge surfacing an FI lift's internalise decision: whether the
 * fill was warehoused internally ("Internalised") or shed external ("B2B"), tinted
 * by the DV01-utilisation `hedgeBand` (green→bid, amber→warn, red→offer,
 * breach→danger), with a "losing" dot when the captured edge is off-tolerance.
 * Rendered ONLY for deals that carry `internalise` (FI lifts).
 */
function InternaliseBadge({ inl }: { readonly inl: Internalise }): React.ReactElement {
  const toleranceNote = inl.withinTolerance ? "" : " · below tolerance (losing)";
  const label = `${internaliseLabel(inl)} · edge ${fmtEdgeBps(inl.edgeBps)} · ${hedgeBandLabel(inl.hedgeBand)} band${toleranceNote}`;
  return (
    <span
      className={styles.inl}
      data-band={inl.hedgeBand}
      data-losing={inl.withinTolerance ? undefined : "true"}
      aria-label={label}
      title={label}
    >
      {internaliseLabel(inl)}
      {!inl.withinTolerance && <span className={styles.inlLosing} aria-hidden="true" />}
    </span>
  );
}

/**
 * The Hedge-column legend — a concise key for the internalise badge, since traders
 * ask what the tints mean. Documents the REAL badge semantics read off the code:
 *   • the LABEL is the routing decision — "Internalised" (warehoused from risk) vs
 *     "B2B" (shed external back-to-back) — independent of colour;
 *   • the COLOUR is the DV01 warehouse-utilisation band (NOT the edge): green =
 *     comfortable, amber = approaching the cap, red = at/over the cap, breach = the
 *     hard limit exceeded — the same bid/warn/offer/danger tokens as the badge;
 *   • the DOT (+ ring) marks a LOSING fill — captured edge below the min-edge
 *     tolerance (`withinTolerance === false`), shown regardless of band.
 * The band swatches reuse the badge classes (`.inl` + `data-band`) so the legend
 * colours are byte-identical to the rows and theme automatically.
 */
function InternaliseLegend(): React.ReactElement {
  const bands: readonly { band: Internalise["hedgeBand"]; note: string }[] = [
    { band: "green", note: "comfortable" },
    { band: "amber", note: "approaching cap" },
    { band: "red", note: "at / over cap" },
    { band: "breach", note: "limit breached" },
  ];
  return (
    <div className={styles.legend} role="note" aria-label="Hedge badge legend">
      <span className={styles.legendTitle}>Hedge</span>
      <span className={styles.legendItem}>
        <span className={styles.inl} data-band="green" aria-hidden="true">
          Internalised
        </span>
        warehoused
      </span>
      <span className={styles.legendItem}>
        <span className={styles.inl} data-band="green" aria-hidden="true">
          B2B
        </span>
        external back-to-back
      </span>
      <span className={styles.legendSep} aria-hidden="true" />
      <span className={styles.legendGroupLabel}>DV01 band:</span>
      {bands.map(({ band, note }) => (
        <span key={band} className={styles.legendItem}>
          <span className={styles.legendSwatch} data-band={band} aria-hidden="true" />
          {hedgeBandLabel(band)} — {note}
        </span>
      ))}
      <span className={styles.legendItem}>
        <span className={styles.inl} data-band="red" data-losing="true" aria-hidden="true">
          <span className={styles.inlLosing} />
        </span>
        dot = losing (edge below tolerance)
      </span>
    </div>
  );
}

/** The searchable text of a deal's internalise decision (empty when it has none). */
function internaliseSearchText(d: Deal): string {
  if (d.internalise === undefined) return "";
  return [
    internaliseLabel(d.internalise),
    hedgeBandLabel(d.internalise.hedgeBand),
    fmtEdgeBps(d.internalise.edgeBps),
    d.internalise.withinTolerance ? "within tolerance" : "below tolerance losing",
  ].join(" ");
}

/** All of a deal's user-visible textual fields, concatenated for substring search. */
function dealSearchText(d: Deal, names: ReadonlyMap<string, string>): string {
  return [
    fmtClock(d.executedAtNanos),
    d.counterparty,
    d.desk,
    d.kind,
    productLabel(d),
    securityDescriptor(d),
    d.bondSecurityId ?? "",
    tenorLabel(d),
    d.curveSet.currency,
    fmtCompact(d.notional),
    fmtRate(d.price),
    sideBuySell(d.side),
    sideLabel(d.side),
    d.trader,
    positionLabel(d),
    riskPortfolioLabel(d, names),
    internaliseSearchText(d),
    d.dealId,
  ].join(" ");
}

export function DealsBlotterWorkspace({
  lens: lensMode = "both",
}: {
  /** Toggle both lenses (default), or force one and hide the toggle. */
  lens?: DealsLensMode;
} = {}): React.ReactElement {
  const app = useApp();
  const principal = useMemo(() => principalForScope(app.scope), [app.scope]);
  // Hard asset separation: the Book's Deals lens shows ONLY the active domain's
  // asset class. Booked deals are structurally OIS (rates), so under the FX Options
  // domain this lens is correctly empty and under Fixed Income it shows them.
  const activeAsset = capabilityAssetForDomain(app.activeDomain);

  const seed = useAcceptanceSeed();
  const canManageAcceptance = app.auth.can("manage_acceptance", "fixed_income");
  const hedge = useHedgeSeed();
  // Authoring a hedge policy gates on `hedge` × FI (same as the Hedging surface); the
  // "Change hedging strategy" row action is hidden entirely without it.
  const canHedge = app.auth.can("hedge", "fixed_income");

  // Client fills vs executed hedges — the two separated lenses of the blotter. The
  // active lens + the search query are persisted so they SURVIVE a tab switch.
  const [ui, setUi] = useTableUiState<{ lens: DealsLens; query: string }>(
    "fi-deals-blotter",
    { lens: "client", query: "" },
  );
  // In "both" mode the active lens is the persisted, toggle-driven one; a forced mode
  // ("client"/"hedge") pins the lens and the toggle is hidden.
  const lens: DealsLens = lensMode === "both" ? ui.lens : lensMode;
  const setLens = useCallback((l: DealsLens) => setUi({ lens: l }), [setUi]);
  const [selected, setSelected] = useState<Deal | null>(null);
  // The row context menu (right-click / context-menu key): the counterparty + anchor
  // point of the row whose "Create acceptance rule" the trader is spawning.
  const [rowMenu, setRowMenu] = useState<FlowRowMenuTarget | null>(null);

  // The Risk Portfolio roster (id → human name) resolves each deal's routed
  // `riskBookId` to its portfolio name. Cached + shared with other surfaces that list
  // risk books; best-effort — an unavailable roster leaves the column at the raw id / `—`.
  const { data: riskBooks } = useCachedResource(
    "riskBooks",
    () => app.transport.listRiskBooks(),
  );
  const riskBookNames = useMemo<ReadonlyMap<string, string>>(
    () => new Map((riskBooks ?? []).map((b) => [b.id, b.name])),
    [riskBooks],
  );

  // Stale-while-revalidate cache keyed on the entitlement principal (scope): booked
  // deals SURVIVE the workspace unmounting on a tab switch, so returning shows them
  // instantly (no blank flash) while a background revalidation refreshes them.
  const {
    data,
    isLoading,
    error: fetchError,
    refresh,
  } = useCachedResource<Deal[]>(
    `deals|${cacheKeyPart(principal)}`,
    () =>
      app.transport
        .listDeals({ ...(principal ? { principal } : {}) })
        .then((res) => res.deals),
  );
  const allDeals = useMemo(() => data ?? [], [data]);
  const error =
    fetchError === undefined || fetchError === null
      ? null
      : fetchError instanceof Error
        ? fetchError.message
        : "failed to load deals";

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  // Refresh on every push Notification (a QUOTE_ACCEPTED mints a deal) — a background
  // revalidate that keeps the current rows visible rather than clearing the table.
  useEffect(() => {
    const dispose = app.transport.streamNotifications(undefined, () => refreshRef.current());
    return dispose;
  }, [app.transport]);

  // Asset-scope to the active domain BEFORE the search filter composes on top.
  const deals = useMemo(
    () => allDeals.filter((d) => dealAsset(d) === activeAsset),
    [allDeals, activeAsset],
  );

  const searchText = useCallback(
    (d: Deal) => dealSearchText(d, riskBookNames),
    [riskBookNames],
  );
  const { query, setQuery, filtered } = useTableFilter(deals, searchText, {
    query: ui.query,
    setQuery: (q) => setUi({ query: q }),
  });

  // The column model — the SAME 16 columns, with every rich cell preserved
  // verbatim through `ColumnDef.cell` (the badges, provenance chips and
  // two-line security descriptor are the whole reason that API exists).
  // `accessor` stays the canonical TEXT projection driving search, the column
  // filters and export; `sortValue` is the ORDERED projection, without which a
  // notional would sort by its compact "1.2b" label rather than its magnitude.
  const columns = useMemo<ReadonlyArray<ColumnDef<Deal>>>(
    () => [
      {
        key: "time",
        header: "Time",
        width: 90,
        align: "left",
        accessor: (d) => fmtClock(d.executedAtNanos),
        cell: (d) => <span className={styles.mono}>{fmtClock(d.executedAtNanos)}</span>,
        // Order by the raw nanosecond stamp — the rendered 24h clock wraps at
        // midnight and would sort a new session's fills before the old ones.
        sortValue: (d) => Number(d.executedAtNanos),
        sortKey: "time",
        filter: { kind: "text" },
      },
      {
        key: "counterparty",
        header: "Counterparty",
        width: 150,
        align: "left",
        accessor: (d) => d.counterparty,
        cell: (d) => <span className={styles.strong}>{d.counterparty}</span>,
        sortKey: "counterparty",
        filter: { kind: "select" },
      },
      {
        key: "desk",
        header: "Desk",
        width: 110,
        align: "left",
        accessor: (d) => d.desk,
        sortKey: "desk",
        filter: { kind: "select" },
      },
      {
        key: "kind",
        header: "Type",
        width: 80,
        align: "left",
        accessor: (d) => d.kind,
        cell: (d) => <span className={`${styles.kind} ${kindClass(d.kind)}`}>{d.kind}</span>,
        sortKey: "kind",
        filter: { kind: "select" },
      },
      {
        key: "product",
        header: "Product",
        width: 90,
        align: "left",
        accessor: (d) => productLabel(d),
        cell: (d) => <span className={styles.product}>{productLabel(d)}</span>,
        sortKey: "product",
        filter: { kind: "select" },
      },
      {
        key: "security",
        header: "Security",
        width: 180,
        align: "left",
        accessor: (d) => securityDescriptor(d),
        cell: (d) => <SecurityCell deal={d} />,
        sortKey: "security",
        filter: { kind: "text" },
      },
      {
        key: "tenor",
        header: "Tenor",
        width: 80,
        align: "right",
        accessor: (d) => tenorLabel(d),
        sortValue: (d) => d.instrument.tenorYears,
        sortKey: "tenor",
        filter: { kind: "range" },
      },
      {
        key: "ccy",
        header: "Ccy",
        width: 70,
        align: "left",
        accessor: (d) => d.curveSet.currency,
        cell: (d) => <span className={styles.mono}>{d.curveSet.currency}</span>,
        sortKey: "ccy",
        filter: { kind: "select" },
      },
      {
        key: "notional",
        header: "Notional",
        width: 110,
        align: "right",
        accessor: (d) => fmtCompact(d.notional),
        sortValue: (d) => d.notional,
        sortKey: "notional",
        filter: { kind: "range" },
      },
      {
        key: "price",
        header: "Rate",
        width: 100,
        align: "right",
        accessor: (d) => fmtRate(d.price),
        cell: (d) => <span className={styles.price}>{fmtRate(d.price)}</span>,
        sortValue: (d) => d.price,
        sortKey: "price",
        filter: { kind: "range" },
      },
      {
        key: "side",
        header: "Side",
        width: 150,
        align: "left",
        // The searchable text keeps BOTH readings of Side (the BUY/SELL axis and
        // the rates pay/receive-fixed convention), matching the badge's label.
        accessor: (d) => `${sideBuySell(d.side)} ${sideLabel(d.side)}`,
        cell: (d) => <SideBadge side={d.side} />,
        sortValue: (d) => sideBuySell(d.side),
        sortKey: "side",
        filter: { kind: "select", options: ["BUY Pay fixed", "SELL Receive fixed", "2-WAY Two-way"] },
      },
      {
        key: "trader",
        header: "Trader",
        width: 110,
        align: "left",
        accessor: (d) => d.trader,
        sortKey: "trader",
        filter: { kind: "select" },
      },
      {
        key: "position",
        header: "Position",
        width: 100,
        align: "left",
        accessor: (d) => positionLabel(d),
        cell: (d) => <span className={styles.mono}>{positionLabel(d)}</span>,
        // An unbooked fill carries no position; NaN sorts LAST rather than
        // heading an ascending sort as a fabricated zero would.
        sortValue: (d) => (d.positionId !== undefined ? Number(d.positionId) : Number.NaN),
        sortKey: "position",
        filter: { kind: "text" },
      },
      {
        key: "riskPortfolio",
        header: "Risk Portfolio",
        width: 150,
        align: "left",
        accessor: (d) => riskPortfolioLabel(d, riskBookNames),
        sortKey: "riskPortfolio",
        filter: { kind: "select" },
      },
      {
        key: "hedge",
        header: "Hedge",
        width: 130,
        align: "left",
        accessor: (d) => (d.internalise ? internaliseLabel(d.internalise) : "—"),
        cell: (d) =>
          d.internalise ? (
            <InternaliseBadge inl={d.internalise} />
          ) : (
            <span className={styles.inlNone}>—</span>
          ),
        sortKey: "hedge",
        filter: { kind: "select" },
      },
      {
        key: "dealId",
        header: "Deal",
        width: 140,
        align: "left",
        accessor: (d) => d.dealId,
        cell: (d) => <span className={styles.dealId}>{d.dealId}</span>,
        sortKey: "dealId",
        filter: { kind: "text" },
      },
    ],
    [riskBookNames],
  );

  const grid = useGridState<Deal>({
    tableId: "fi-deals-blotter",
    columns,
    rows: filtered,
    allRows: deals,
  });

  const isOffline = !app.transport.label.startsWith("live");
  const totalNotional = deals.reduce((acc, d) => acc + d.notional, 0);

  return (
    <div className={styles.shell}>
      {lensMode === "both" && (
        <div className={styles.lensBar} role="group" aria-label="deals lens">
          <button
            type="button"
            className={`${styles.lensBtn} ${lens === "client" ? styles.lensBtnActive : ""}`}
            aria-pressed={lens === "client"}
            data-testid="deals-lens-client"
            onClick={() => setLens("client")}
          >
            Client deals
          </button>
          <button
            type="button"
            className={`${styles.lensBtn} ${lens === "hedge" ? styles.lensBtnActive : ""}`}
            aria-pressed={lens === "hedge"}
            data-testid="deals-lens-hedge"
            onClick={() => setLens("hedge")}
          >
            Hedge deals
          </button>
        </div>
      )}
      {lens === "hedge" ? (
        <HedgeDealsView />
      ) : (
        <div className={styles.wrap}>
          <Panel className={styles.panel} title="Received deals">
        <div className={styles.head}>
          <span className={styles.engine}>{isOffline ? "in-app desk" : "live desk"}</span>
          <span className={styles.summary}>
            {deals.length} deal{deals.length === 1 ? "" : "s"} ·{" "}
            {fmtCompact(totalNotional)} notional
          </span>
        </div>
        {error && (
          <p className={styles.error} role="alert">
            {error}
          </p>
        )}
        {isLoading ? (
          <TableSkeleton label="Loading deals…" />
        ) : deals.length === 0 ? (
          <p className={styles.empty}>
            {activeAsset === "fixed_income"
              ? "No deals yet — accept a quote in the Quoting workspace to book one."
              : "No FX-option deals — the RFQ desk books rates (OIS). Switch to the Fixed Income domain to see booked deals."}
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
              label="Search deals"
              placeholder="Filter deals…"
            />
            <InternaliseLegend />
            <DataTable
              label="Deals"
              columns={columns}
              grid={grid}
              rowKey={(d) => d.dealId}
              hideRowCount
              rowProps={(d) => ({
                className: `${styles.row} ${selected?.dealId === d.dealId ? styles.rowSelected : ""}`,
                onClick: () => setSelected(d),
                tabIndex: 0,
                role: "button",
                "aria-label": `Open deal ${d.dealId}. Right-click or press the menu key for row actions`,
                onContextMenu: (e) => {
                  e.preventDefault();
                  setRowMenu({
                    counterparty: d.counterparty,
                    x: e.clientX,
                    y: e.clientY,
                    hedgeSeed: hedgeSeedFromDeal(d),
                    positionId: d.positionId,
                  });
                },
                onKeyDown: (e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    setSelected(d);
                    return;
                  }
                  // The standard context-menu key (or Shift+F10) opens the row menu —
                  // keyboard parity for the right-click, without a nested-interactive
                  // kebab inside this role="button" row (axe-clean).
                  if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
                    e.preventDefault();
                    const r = e.currentTarget.getBoundingClientRect();
                    setRowMenu({
                      counterparty: d.counterparty,
                      x: r.left + 12,
                      y: r.bottom - 8,
                      hedgeSeed: hedgeSeedFromDeal(d),
                      positionId: d.positionId,
                    });
                  }
                },
              })}
              emptyState={
                query.trim() === ""
                  ? "No deals match the current column filters."
                  : `No deals match “${query}”.`
              }
            />
          </>
        )}
      </Panel>
          <FlowRowContextMenu
            target={rowMenu}
            onClose={() => setRowMenu(null)}
            onCreateAcceptanceRule={(cp) => {
              // Seed the rule, then navigate to the Acceptance surface (the `acceptance`
              // alias → the consolidated Risk host's Acceptance tab), which consumes the seed.
              seed.requestAcceptanceSeed(cp);
              app.setWorkspace("acceptance");
            }}
            canManageAcceptance={canManageAcceptance}
            onChangeHedgingStrategy={(s) => {
              // Seed a hedge rule scoped to this deal's flow, then deep-link to the Hedging
              // workspace — its Exit Policy tab (the default) consumes the seed and opens a
              // pre-filled draft rule for the trader to tailor + save.
              hedge.requestHedgeSeed(s);
              app.setWorkspace("hedging");
            }}
            canHedge={canHedge}
            onViewTrace={(t) => {
              // Deep-link to the Event Trace timeline, resolved by the deal's durable
              // positionId (the blotter↔trace join key). Only present on booked-position
              // rows (the item hides otherwise).
              if (t.positionId === undefined) return;
              app.openTrace({ kind: "position", positionId: t.positionId, label: t.counterparty });
            }}
          />
          {selected && <DealTicket deal={selected} onClose={() => setSelected(null)} />}
        </div>
      )}
    </div>
  );
}
