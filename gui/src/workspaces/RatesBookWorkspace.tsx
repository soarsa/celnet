/**
 * RatesBookWorkspace — the booked linear-rates book (fixed-income). The desk's
 * standing OIS positions on the right; a Book ticket on the left to add one. This
 * is the outstanding "rates Book" surface: positions are listed from the server's
 * in-memory rates book and a Book action persists a new one (an accepted desk
 * deal also books a position here, so a fill from the Quoting workspace appears).
 *
 * One contract, two transports (GUI-DESIGN §6.2): the workspace talks ONLY to the
 * `CelnetTransport` rates-Book seam (`bookRatesPosition` / `listRatesPositions`),
 * so the SAME book renders + grows through the deterministic in-app source and the
 * live `RiskService` edge. The Book ticket fields are GENUINE inputs (seeded from
 * the curve pillars), never hardcoded results.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import { principalForScope } from "../data/riskView";
import { DEFAULT_USD_SOFR_CURVE } from "../data/ratesPricing";
import { fmtRate } from "../lib/format";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import type { OisDirection, RatesPosition } from "../data/contract";
import styles from "./RatesBookWorkspace.module.css";

const MM = 1_000_000;

/** The Book ticket input model (the editable booking form). */
interface BookTicket {
  entity: number;
  book: number;
  tenorYears: number;
  fixedRatePct: number;
  notionalMm: number;
  direction: OisDirection;
}

function defaultTicket(): BookTicket {
  const pillar = DEFAULT_USD_SOFR_CURVE.pillars.find((p) => p.tenorYears === 5);
  const parPct = (pillar?.parRate ?? 0.04) * 100;
  return {
    entity: 1,
    book: 10,
    tenorYears: 5,
    fixedRatePct: Number(parPct.toFixed(4)),
    notionalMm: 50,
    direction: "RECEIVE_FIXED",
  };
}

function directionLabel(direction: OisDirection): string {
  return direction === "PAY_FIXED" ? "Pay" : "Receive";
}

export function RatesBookWorkspace(): React.ReactElement {
  const app = useApp();
  const principal = useMemo(() => principalForScope(app.scope), [app.scope]);

  const [positions, setPositions] = useState<RatesPosition[]>([]);
  const [ticket, setTicket] = useState<BookTicket>(defaultTicket);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(() => {
    void app.transport
      .listRatesPositions({ ...(principal ? { principal } : {}) })
      .then((res) => {
        setPositions(res.positions);
        setError(null);
      })
      .catch((err) =>
        setError(err instanceof Error ? err.message : "failed to load rates positions"),
      );
  }, [app.transport, principal]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const patch = useCallback((p: Partial<BookTicket>) => setTicket((t) => ({ ...t, ...p })), []);

  // Capability gating (slice 5): booking a rates position is gated on
  // `book·fixed_income` (disabled + tooltip, never hidden; handler no-ops
  // defensively, the server still enforces). Anonymous ⇒ permissive.
  const canBook = app.auth.can("book", "fixed_income");
  const bookDeniedTitle = capabilityDenialTitle("book", "fixed_income");

  const book = useCallback(async () => {
    if (!canBook) return;
    setBusy(true);
    setError(null);
    try {
      await app.transport.bookRatesPosition({
        position: {
          // A placeholder id (0) lets the server mint a stable position id.
          positionId: 0n,
          entity: ticket.entity,
          book: ticket.book,
          instrument: {
            tenorYears: ticket.tenorYears,
            fixedRate: ticket.fixedRatePct / 100,
            notional: ticket.notionalMm * MM,
            direction: ticket.direction,
          },
        },
        ...(principal ? { principal } : {}),
      });
      refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed to book position");
    } finally {
      setBusy(false);
    }
  }, [app.transport, principal, ticket, refresh, canBook]);

  const isOffline = !app.transport.label.startsWith("live");
  const totalMm = positions.reduce((acc, p) => acc + p.instrument.notional, 0) / MM;

  return (
    <div className={styles.wrap}>
      <Panel material="float" className={styles.ticket} title="Book position">
        <div className={styles.ticketGrid}>
          <Field label="Entity">
            <input
              className={styles.input}
              type="number"
              min={0}
              step={1}
              value={ticket.entity}
              aria-label="entity id"
              onChange={(e) => patch({ entity: Math.trunc(Number(e.target.value)) })}
            />
          </Field>
          <Field label="Book">
            <input
              className={styles.input}
              type="number"
              min={0}
              step={1}
              value={ticket.book}
              aria-label="book id"
              onChange={(e) => patch({ book: Math.trunc(Number(e.target.value)) })}
            />
          </Field>
          <Field label="Tenor (y)">
            <input
              className={styles.input}
              type="number"
              min={1}
              step={1}
              value={ticket.tenorYears}
              aria-label="tenor in years"
              onChange={(e) => patch({ tenorYears: Math.max(1, Math.trunc(Number(e.target.value))) })}
            />
          </Field>
          <Field label="Fixed %">
            <input
              className={styles.input}
              type="number"
              step={0.01}
              value={ticket.fixedRatePct}
              aria-label="fixed rate in percent"
              onChange={(e) => patch({ fixedRatePct: Number(e.target.value) })}
            />
          </Field>
          <Field label="Notional mm">
            <input
              className={styles.input}
              type="number"
              min={1}
              step={5}
              value={ticket.notionalMm}
              aria-label="notional in millions"
              onChange={(e) => patch({ notionalMm: Math.max(1, Number(e.target.value)) })}
            />
          </Field>
          <Field label="Side">
            <select
              className={styles.input}
              value={ticket.direction}
              aria-label="swap direction"
              onChange={(e) => patch({ direction: e.target.value as OisDirection })}
            >
              <option value="RECEIVE_FIXED">Receive</option>
              <option value="PAY_FIXED">Pay</option>
            </select>
          </Field>
        </div>
        <div className={styles.ticketFoot}>
          <Button
            variant="primary"
            disabled={busy || !canBook}
            onClick={book}
            title={canBook ? undefined : bookDeniedTitle}
          >
            Book position
          </Button>
          <span className={styles.curveTag}>{DEFAULT_USD_SOFR_CURVE.currency}-SOFR</span>
        </div>
        {error && (
          <p className={styles.error} role="alert">
            {error}
          </p>
        )}
      </Panel>

      <Panel className={styles.positions} title="Rates book">
        <div className={styles.head}>
          <span className={styles.engine}>{isOffline ? "in-app book" : "live book"}</span>
          <span className={styles.summary}>
            {positions.length} position{positions.length === 1 ? "" : "s"} ·{" "}
            {totalMm.toLocaleString(undefined, { maximumFractionDigits: 0 })}mm notional
          </span>
        </div>
        {positions.length === 0 ? (
          <p className={styles.empty}>The rates book is empty — book a position to populate it.</p>
        ) : (
          <div className={styles.tableWrap}>
            <table className={styles.table}>
              <thead>
                <tr>
                  <th className={styles.num}>Id</th>
                  <th className={styles.num}>Entity</th>
                  <th className={styles.num}>Book</th>
                  <th>Instrument</th>
                  <th className={styles.num}>Fixed</th>
                  <th className={styles.num}>Notional</th>
                  <th>Side</th>
                </tr>
              </thead>
              <tbody>
                {positions.map((p) => (
                  <tr key={p.positionId.toString()}>
                    <td className={`${styles.num} ${styles.mono} ${styles.idCell}`}>
                      {p.positionId.toString()}
                    </td>
                    <td className={`${styles.num} ${styles.mono}`}>{p.entity}</td>
                    <td className={`${styles.num} ${styles.mono}`}>{p.book}</td>
                    <td className={styles.strong}>{p.instrument.tenorYears}y OIS</td>
                    <td className={`${styles.num} ${styles.mono} ${styles.rate}`}>
                      {fmtRate(p.instrument.fixedRate)}
                    </td>
                    <td className={`${styles.num} ${styles.mono}`}>
                      {(p.instrument.notional / MM).toLocaleString(undefined, {
                        maximumFractionDigits: 1,
                      })}
                      mm
                    </td>
                    <td>{directionLabel(p.instrument.direction)}</td>
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

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}): React.ReactElement {
  return (
    <label className={styles.field}>
      <span className={styles.fieldLabel}>{label}</span>
      {children}
    </label>
  );
}
