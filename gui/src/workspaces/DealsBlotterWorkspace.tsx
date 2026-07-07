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
import { DealTicket } from "./DealTicket";
import styles from "./DealsBlotterWorkspace.module.css";

function sideLabel(side: Side): string {
  if (side === "BUY") return "Pay";
  if (side === "SELL") return "Receive";
  return "Two-way";
}

/** All of a deal's user-visible textual fields, concatenated for substring search. */
function dealSearchText(d: Deal): string {
  return [
    fmtClock(d.executedAtNanos),
    d.counterparty,
    d.desk,
    d.kind,
    `${d.instrument.tenorYears}y OIS`,
    d.curveSet.currency,
    fmtCompact(d.notional),
    fmtRate(d.price),
    sideLabel(d.side),
    d.trader,
    d.dealId,
  ].join(" ");
}

export function DealsBlotterWorkspace(): React.ReactElement {
  const app = useApp();
  const principal = useMemo(() => principalForScope(app.scope), [app.scope]);

  const [deals, setDeals] = useState<Deal[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Deal | null>(null);

  const refresh = useCallback(() => {
    void app.transport
      .listDeals({ ...(principal ? { principal } : {}) })
      .then((res) => {
        setDeals(res.deals);
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

  const { query, setQuery, filtered, shown, total } = useTableFilter(deals, dealSearchText);

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
            No deals yet — accept a quote in the Quoting workspace to book one.
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
              <div className={styles.tableWrap}>
                <table className={styles.table}>
                  <thead>
                    <tr>
                      <th>Time</th>
                      <th>Counterparty</th>
                      <th>Desk</th>
                      <th>Instrument</th>
                      <th>Ccy</th>
                      <th className={styles.num}>Notional</th>
                      <th className={styles.num}>Price</th>
                      <th>Side</th>
                      <th>Trader</th>
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
                      {d.instrument.tenorYears}y OIS
                    </td>
                    <td className={styles.mono}>{d.curveSet.currency}</td>
                    <td className={`${styles.num} ${styles.mono}`}>{fmtCompact(d.notional)}</td>
                    <td className={`${styles.num} ${styles.mono} ${styles.price}`}>{fmtRate(d.price)}</td>
                    <td>{sideLabel(d.side)}</td>
                    <td>{d.trader}</td>
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
