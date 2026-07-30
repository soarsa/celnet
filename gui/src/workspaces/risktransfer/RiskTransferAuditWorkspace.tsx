/**
 * RiskTransferAuditWorkspace — the FI Risk Transfer AUDIT TRAIL (docs/RISK-TRANSFER-
 * REQUIREMENTS.md §9.3). An immutable, queryable blotter of every transfer, showing
 * the provenance the platform stamps: who moved what, when, at what price/basis,
 * approved by whom, the realised P&L in the source, and the moved-risk vector. Filter
 * by book / desk / trader / state (server-side via `listRiskTransfers`) and by date
 * (client-side — the wire filter has no date axis). Each row expands to the moved
 * positions + the full provenance. Read-only for any FI trader (gated on `view`).
 * Reads the transport via `useApp()`.
 */

import { Fragment, useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../app/AppContext";
import type {
  DeskDesc,
  RiskBook,
  RiskTransfer,
  TransferState,
} from "../../data/contract";
import { fmtClock, fmtCompact, fmtSigned } from "../../lib/format";
import styles from "./RiskTransferAuditWorkspace.module.css";

const STATES: readonly (TransferState | "ALL")[] = [
  "ALL",
  "PENDING",
  "BOOKED",
  "REJECTED",
  "CANCELLED",
  "ACCEPTED",
  "DRAFT",
];

/** ns-epoch → yyyy-mm-dd (local), for the client-side date-floor filter. */
function dayOf(epochNanos: bigint): string {
  const d = new Date(Number(epochNanos / 1_000_000n));
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

export function RiskTransferAuditWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== null && auth.user !== undefined;

  const [transfers, setTransfers] = useState<RiskTransfer[]>([]);
  const [books, setBooks] = useState<RiskBook[]>([]);
  const [desks, setDesks] = useState<DeskDesc[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  // Filters — desk/trader/book/state drive the server query; fromDate is client-side.
  const [stateFilter, setStateFilter] = useState<TransferState | "ALL">("ALL");
  const [deskFilter, setDeskFilter] = useState("");
  const [traderFilter, setTraderFilter] = useState("");
  const [bookFilter, setBookFilter] = useState("");
  const [fromDate, setFromDate] = useState("");

  // The book roster + desks are one-shot (names + filter dropdowns).
  useEffect(() => {
    if (!signedIn) {
      setBooks([]);
      setDesks([]);
      return;
    }
    let cancelled = false;
    void Promise.all([app.transport.listRiskBooks(), app.transport.listDesks()])
      .then(([b, d]) => {
        if (cancelled) return;
        setBooks(b);
        setDesks(d);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [app.transport, signedIn]);

  // The blotter itself re-queries whenever a server-side filter changes.
  useEffect(() => {
    if (!signedIn) {
      setTransfers([]);
      return;
    }
    let cancelled = false;
    setLoadError(null);
    void app.transport
      .listRiskTransfers({
        desk: deskFilter.length > 0 ? deskFilter : null,
        trader: traderFilter.length > 0 ? traderFilter : null,
        riskBookId: bookFilter.length > 0 ? bookFilter : null,
        states: stateFilter === "ALL" ? [] : [stateFilter],
      })
      .then((rows) => {
        if (!cancelled) setTransfers(rows);
      })
      .catch((e: unknown) => {
        if (!cancelled) setLoadError(e instanceof Error ? e.message : "failed to load transfers");
      });
    return () => {
      cancelled = true;
    };
  }, [app.transport, signedIn, deskFilter, traderFilter, bookFilter, stateFilter]);

  const bookName = useCallback(
    (id: string): string => books.find((b) => b.id === id)?.name ?? id,
    [books],
  );

  const rows = useMemo(
    () => (fromDate.length === 0 ? transfers : transfers.filter((t) => dayOf(t.initiatedAt) >= fromDate)),
    [transfers, fromDate],
  );

  if (!signedIn) {
    return (
      <div className={styles.wrap}>
        <p className={styles.empty}>Sign in to view the transfer audit trail.</p>
      </div>
    );
  }

  return (
    <div className={styles.wrap}>
      <header className={styles.head}>
        <h1 className={styles.title}>Transfer Audit</h1>
        <p className={styles.note}>
          Every risk transfer, immutable and attributable — who moved what, when, at what price,
          approved by whom. Expand a row for the moved positions and full provenance.
        </p>
      </header>

      <div className={styles.filters} role="search" aria-label="Filter transfers">
        <label className={styles.filter}>
          <span className={styles.filterLabel}>State</span>
          <select
            className={styles.select}
            value={stateFilter}
            onChange={(e) => setStateFilter(e.target.value as TransferState | "ALL")}
            data-testid="audit-state"
          >
            {STATES.map((s) => (
              <option key={s} value={s}>
                {s === "ALL" ? "All states" : s.toLowerCase()}
              </option>
            ))}
          </select>
        </label>
        <label className={styles.filter}>
          <span className={styles.filterLabel}>Desk</span>
          <select
            className={styles.select}
            value={deskFilter}
            onChange={(e) => setDeskFilter(e.target.value)}
            data-testid="audit-desk"
          >
            <option value="">All desks</option>
            {desks.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
              </option>
            ))}
          </select>
        </label>
        <label className={styles.filter}>
          <span className={styles.filterLabel}>Portfolio</span>
          <select
            className={styles.select}
            value={bookFilter}
            onChange={(e) => setBookFilter(e.target.value)}
            data-testid="audit-book"
          >
            <option value="">All portfolios</option>
            {books.map((b) => (
              <option key={b.id} value={b.id}>
                {b.name}
              </option>
            ))}
          </select>
        </label>
        <label className={styles.filter}>
          <span className={styles.filterLabel}>Trader</span>
          <input
            className={styles.select}
            value={traderFilter}
            onChange={(e) => setTraderFilter(e.target.value)}
            placeholder="email"
            data-testid="audit-trader"
          />
        </label>
        <label className={styles.filter}>
          <span className={styles.filterLabel}>From date</span>
          <input
            type="date"
            className={styles.select}
            value={fromDate}
            onChange={(e) => setFromDate(e.target.value)}
            data-testid="audit-from"
          />
        </label>
      </div>

      {loadError && (
        <p className={styles.error} role="alert">
          {loadError}
        </p>
      )}

      <section className={styles.tableWrap} aria-label="Transfer audit trail" tabIndex={0}>
        <table className={styles.table} data-testid="audit-table">
          <thead>
            <tr>
              <th scope="col">Transfer</th>
              <th scope="col">Kind</th>
              <th scope="col">Route</th>
              <th scope="col" className={styles.numCol}>
                Notional
              </th>
              <th scope="col" className={styles.numCol}>
                Price
              </th>
              <th scope="col" className={styles.numCol}>
                Realised P&amp;L
              </th>
              <th scope="col">State</th>
              <th scope="col">By / approver</th>
              <th scope="col">When</th>
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && (
              <tr>
                <td colSpan={9} className={styles.empty}>
                  No transfers match the current filters. Initiate one on the Risk Transfer ticket.
                </td>
              </tr>
            )}
            {rows.map((t) => {
              const p = t.provenance;
              const moved = p ? p.riskMoved.notionalBase : t.quantityFull ? null : t.partialNotional;
              const expanded = expandedId === t.id;
              return (
                <Fragment key={t.id}>
                  <tr
                    className={expanded ? styles.rowActive : undefined}
                    data-testid={`audit-row-${t.id}`}
                    onClick={() => setExpandedId(expanded ? null : t.id)}
                    aria-expanded={expanded}
                  >
                    <td className={styles.mono}>#{t.id}</td>
                    <td>
                      <span className={`${styles.kindDot} ${styles[`kind_${t.kind}`]}`} aria-hidden />
                      {t.kind.replace(/_/g, " ").toLowerCase()}
                    </td>
                    <td className={styles.route}>
                      {bookName(t.source.riskBookId)} → {bookName(t.target.riskBookId)}
                    </td>
                    <td className={`${styles.num} ${styles.mono}`}>
                      {moved === null ? "Full" : fmtCompact(moved)}
                    </td>
                    <td className={`${styles.num} ${styles.mono}`}>
                      {t.transferPrice !== null ? t.transferPrice.toFixed(2) : "—"}
                    </td>
                    <td className={`${styles.num} ${styles.mono}`}>
                      {p ? fmtSigned(p.realizedPnlSource, 0) : "—"}
                    </td>
                    <td>
                      <span className={`${styles.stateTag} ${styles[`state_${t.state}`]}`}>
                        {t.state.toLowerCase()}
                      </span>
                    </td>
                    <td className={styles.who}>
                      {t.initiatedBy}
                      {t.approver && <span className={styles.approver}>✓ {t.approver}</span>}
                    </td>
                    <td className={`${styles.mono} ${styles.when}`}>{fmtClock(t.initiatedAt)}</td>
                  </tr>
                  {expanded && (
                    <tr className={styles.detailRow} key={`${t.id}-detail`}>
                      <td colSpan={9}>
                        <div className={styles.detail} data-testid={`audit-detail-${t.id}`}>
                          <div className={styles.detailCol}>
                            <h3 className={styles.detailTitle}>Moved positions</h3>
                            {t.source.positionIds.length === 0 ? (
                              <p className={styles.muted}>No position ids recorded.</p>
                            ) : (
                              <ul className={styles.posList}>
                                {t.source.positionIds.map((id) => (
                                  <li key={id.toString()} className={styles.mono}>
                                    {id.toString()}
                                  </li>
                                ))}
                              </ul>
                            )}
                          </div>
                          <div className={styles.detailCol}>
                            <h3 className={styles.detailTitle}>Provenance</h3>
                            {p ? (
                              <dl className={styles.provList}>
                                <Prov label="Price basis" value={p.priceBasis.toLowerCase()} />
                                <Prov label="Transfer price" value={p.transferPrice.toFixed(2)} mono />
                                <Prov
                                  label="Moved notional"
                                  value={fmtCompact(p.riskMoved.notionalBase)}
                                  mono
                                />
                                <Prov label="Moved DV01" value={fmtSigned(p.riskMoved.risk.dv01, 0)} mono />
                                <Prov label="Moved Δ" value={fmtSigned(p.riskMoved.risk.delta, 0)} mono />
                                <Prov label="Moved Vega" value={fmtSigned(p.riskMoved.risk.vega, 0)} mono />
                                <Prov
                                  label="Realised P&L (source)"
                                  value={fmtSigned(p.realizedPnlSource, 0)}
                                  mono
                                />
                                <Prov label="Approver" value={p.approver ?? "— (single-control)"} />
                                {p.reason.trim().length > 0 && <Prov label="Reason" value={p.reason} />}
                              </dl>
                            ) : (
                              <p className={styles.muted}>
                                Provenance is stamped on booking — this transfer is {t.state.toLowerCase()}.
                                {t.reason.trim().length > 0 && ` Reason: ${t.reason}`}
                              </p>
                            )}
                          </div>
                        </div>
                      </td>
                    </tr>
                  )}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      </section>
    </div>
  );
}

/** One provenance key/value line. */
function Prov({ label, value, mono }: { label: string; value: string; mono?: boolean }): React.ReactElement {
  return (
    <div className={styles.prov}>
      <dt>{label}</dt>
      <dd className={mono ? styles.mono : undefined}>{value}</dd>
    </div>
  );
}
