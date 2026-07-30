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
import { Panel } from "../components/Panel";
import { TableSearch } from "../components/TableSearch";
import { useTableFilter } from "../hooks/useTableFilter";
import { principalForScope } from "../data/riskView";
import { fmtRate, fmtClock, fmtCompact } from "../lib/format";
import type { Deal, Side } from "../data/contract";
import { capabilityAssetForDomain, dealAsset } from "../data/assetClass";
import { DealTicket } from "./DealTicket";
import styles from "./DealsBlotterWorkspace.module.css";

function sideLabel(side: Side): string {
  if (side === "BUY") return "Pay";
  if (side === "SELL") return "Receive";
  return "Two-way";
}

/**
 * The product family a booked deal carries. Every desk-booked deal is structurally
 * an `OisInstrument` (the rates P0 arm), so the family is OIS — surfaced honestly as
 * a real column rather than invented. When the contract grows other rates families
 * (IRS / FRA / Bond) this reads them off the instrument discriminant.
 */
function productLabel(_d: Deal): string {
  return "OIS";
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

/** All of a deal's user-visible textual fields, concatenated for substring search. */
function dealSearchText(d: Deal, names: ReadonlyMap<string, string>): string {
  return [
    fmtClock(d.executedAtNanos),
    d.counterparty,
    d.desk,
    d.kind,
    productLabel(d),
    tenorLabel(d),
    d.curveSet.currency,
    fmtCompact(d.notional),
    fmtRate(d.price),
    sideLabel(d.side),
    d.trader,
    positionLabel(d),
    riskPortfolioLabel(d, names),
    d.dealId,
  ].join(" ");
}

export function DealsBlotterWorkspace(): React.ReactElement {
  const app = useApp();
  const principal = useMemo(() => principalForScope(app.scope), [app.scope]);
  // Hard asset separation: the Book's Deals lens shows ONLY the active domain's
  // asset class. Booked deals are structurally OIS (rates), so under the FX Options
  // domain this lens is correctly empty and under Fixed Income it shows them.
  const activeAsset = capabilityAssetForDomain(app.activeDomain);

  const [allDeals, setAllDeals] = useState<Deal[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Deal | null>(null);
  // The Risk Portfolio roster (id → human name) resolves each deal's routed
  // `riskBookId` to its portfolio name. Best-effort: an unavailable roster (e.g. no
  // routing configured) leaves the column falling back to the raw id / `—`.
  const [riskBookNames, setRiskBookNames] = useState<ReadonlyMap<string, string>>(
    () => new Map(),
  );

  useEffect(() => {
    let live = true;
    void app.transport
      .listRiskBooks()
      .then((books) => {
        if (live) setRiskBookNames(new Map(books.map((b) => [b.id, b.name])));
      })
      .catch(() => {
        /* no roster ⇒ the column falls back to the raw id / `—` (non-fatal). */
      });
    return () => {
      live = false;
    };
  }, [app.transport]);

  const refresh = useCallback(() => {
    void app.transport
      .listDeals({ ...(principal ? { principal } : {}) })
      .then((res) => {
        setAllDeals(res.deals);
        setError(null);
      })
      .catch((err) => setError(err instanceof Error ? err.message : "failed to load deals"));
  }, [app.transport, principal]);

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  useEffect(() => {
    refreshRef.current();
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
  const { query, setQuery, filtered, shown, total } = useTableFilter(deals, searchText);

  const isOffline = !app.transport.label.startsWith("live");
  const totalNotional = deals.reduce((acc, d) => acc + d.notional, 0);

  return (
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
        {deals.length === 0 ? (
          <p className={styles.empty}>
            {activeAsset === "fixed_income"
              ? "No deals yet — accept a quote in the Quoting workspace to book one."
              : "No FX-option deals — the RFQ desk books rates (OIS). Switch to the Fixed Income domain to see booked deals."}
          </p>
        ) : (
          <>
            <TableSearch
              query={query}
              onQueryChange={setQuery}
              shown={shown}
              total={total}
              label="Search deals"
              placeholder="Filter deals…"
            />
            {filtered.length === 0 ? (
              <p className={styles.empty}>No deals match “{query}”.</p>
            ) : (
              <div className={styles.tableWrap} tabIndex={0} role="region" aria-label="Deals table">
                <table className={styles.table}>
                  <thead>
                    <tr>
                      <th>Time</th>
                      <th>Counterparty</th>
                      <th>Desk</th>
                      <th>Type</th>
                      <th>Product</th>
                      <th className={styles.num}>Tenor</th>
                      <th>Ccy</th>
                      <th className={styles.num}>Notional</th>
                      <th className={styles.num}>Rate</th>
                      <th>Side</th>
                      <th>Trader</th>
                      <th>Position</th>
                      <th>Risk Portfolio</th>
                      <th>Deal</th>
                    </tr>
                  </thead>
                  <tbody>
                    {filtered.map((d) => (
                  <tr
                    key={d.dealId}
                    className={`${styles.row} ${selected?.dealId === d.dealId ? styles.rowSelected : ""}`}
                    onClick={() => setSelected(d)}
                    tabIndex={0}
                    role="button"
                    aria-label={`Open deal ${d.dealId}`}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        setSelected(d);
                      }
                    }}
                  >
                    <td className={styles.mono}>{fmtClock(d.executedAtNanos)}</td>
                    <td className={styles.strong}>{d.counterparty}</td>
                    <td>{d.desk}</td>
                    <td>
                      <span className={`${styles.kind} ${d.kind === "IOI" ? styles.kindIoi : styles.kindRfq}`}>
                        {d.kind}
                      </span>
                    </td>
                    <td>
                      <span className={styles.product}>{productLabel(d)}</span>
                    </td>
                    <td className={`${styles.num} ${styles.mono}`}>{tenorLabel(d)}</td>
                    <td className={styles.mono}>{d.curveSet.currency}</td>
                    <td className={`${styles.num} ${styles.mono}`}>{fmtCompact(d.notional)}</td>
                    <td className={`${styles.num} ${styles.mono} ${styles.price}`}>{fmtRate(d.price)}</td>
                    <td>{sideLabel(d.side)}</td>
                    <td>{d.trader}</td>
                    <td className={styles.mono}>{positionLabel(d)}</td>
                    <td>{riskPortfolioLabel(d, riskBookNames)}</td>
                    <td className={styles.dealId}>{d.dealId}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </>
        )}
      </Panel>
      {selected && <DealTicket deal={selected} onClose={() => setSelected(null)} />}
    </div>
  );
}
