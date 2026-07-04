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

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import { Panel } from "../components/Panel";
import { principalForScope } from "../data/riskView";
import { fmtRate, fmtClock } from "../lib/format";
import { sideLabel, fmtMm } from "./QuotingWorkspace";
import type { DeskRequest } from "../data/contract";
import styles from "./QuotesBlotterWorkspace.module.css";

const MM = 1_000_000;

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

  const [requests, setRequests] = useState<DeskRequest[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    void app.transport
      .listDeskRequests({ ...(principal ? { principal } : {}) })
      .then((res) => {
        setRequests(res.requests);
        setError(null);
      })
      .catch((err) =>
        setError(err instanceof Error ? err.message : "failed to load quotes"),
      );
  }, [app.transport, principal]);

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  useEffect(() => {
    refreshRef.current();
    const dispose = app.transport.streamNotifications(undefined, () => refreshRef.current());
    return dispose;
  }, [app.transport]);

  // Only shown quotes (QUOTED / ACCEPTED), newest received first.
  const quotes = useMemo(
    () =>
      requests
        .filter(hasShownQuote)
        .slice()
        .sort((a, b) =>
          b.receivedAtNanos > a.receivedAtNanos
            ? 1
            : b.receivedAtNanos < a.receivedAtNanos
              ? -1
              : 0,
        ),
    [requests],
  );

  const isOffline = !app.transport.label.startsWith("live");
  const totalMm = quotes.reduce((acc, r) => acc + (r.quote?.notional ?? 0), 0) / MM;

  return (
    <div className={styles.wrap}>
      <Panel className={styles.panel} title="Shown quotes">
        <div className={styles.head}>
          <span className={styles.engine}>{isOffline ? "in-app desk" : "live desk"}</span>
          <span className={styles.summary}>
            {quotes.length} quote{quotes.length === 1 ? "" : "s"} ·{" "}
            {totalMm.toLocaleString(undefined, { maximumFractionDigits: 0 })}mm quoted
          </span>
        </div>
        {error && (
          <p className={styles.error} role="alert">
            {error}
          </p>
        )}
        {quotes.length === 0 ? (
          <p className={styles.empty}>
            No quotes shown yet — price a request in the Quoting workspace to show one.
          </p>
        ) : (
          <div className={styles.tableWrap}>
            <table className={styles.table}>
              <thead>
                <tr>
                  <th>Received</th>
                  <th>Counterparty</th>
                  <th>Desk</th>
                  <th>Instrument</th>
                  <th>Ccy</th>
                  <th>Side</th>
                  <th className={styles.num}>Quoted rate</th>
                  <th className={styles.num}>Notional</th>
                  <th className={styles.num}>Good for</th>
                  <th>Trader</th>
                  <th>State</th>
                </tr>
              </thead>
              <tbody>
                {quotes.map((r) => (
                  <tr key={r.requestId}>
                    <td className={styles.mono}>{fmtClock(r.receivedAtNanos)}</td>
                    <td className={styles.strong}>{r.counterparty}</td>
                    <td>{r.desk}</td>
                    <td>
                      <span
                        className={`${styles.kind} ${r.kind === "IOI" ? styles.kindIoi : styles.kindRfq}`}
                      >
                        {r.kind}
                      </span>
                      {r.instrument.tenorYears}y OIS
                    </td>
                    <td className={styles.mono}>{r.curveSet.currency}</td>
                    <td>{sideLabel(r.side)}</td>
                    <td className={`${styles.num} ${styles.mono} ${styles.price}`}>
                      {r.quote ? fmtRate(r.quote.price) : "—"}
                    </td>
                    <td className={`${styles.num} ${styles.mono}`}>
                      {fmtMm(r.quote?.notional ?? r.notional)}
                    </td>
                    <td className={`${styles.num} ${styles.mono}`}>
                      {r.quote ? `${Math.round(r.quote.validForMs / 1000)}s` : "—"}
                    </td>
                    <td>{r.quote?.trader ?? "—"}</td>
                    <td>
                      <span className={`${styles.state} ${stateClass(r.state)}`}>{r.state}</span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Panel>
    </div>
  );
}
