/**
 * SimulatorPanel — the permission-gated counterparty INJECTOR.
 *
 * The trader's flow-simulation surface: it generates dealer requests (RFQ / IOI)
 * and SENDS them into the LIVE desk via the canonical `submitDeskRequest` seam, so
 * each generated request enters the real RFQ/IOI inbox (PENDING) and the desk
 * prices/quotes it exactly as a counterparty request would. This is the opposite
 * of a sandbox: there is no offline pricer and no sandbox disclaimer — every
 * Generate is a real injection on the same authenticated transport the rest of
 * the app uses.
 *
 * Window model: this panel is rendered into a SEPARATE OS window opened with
 * `window.open` (see Shell's `SimulatorPopout`), so the main desk stays visible
 * alongside it. The popout hosts its own React root — required so the panel's DOM
 * events bind to the popout document — but it is handed the LIVE `transport` from
 * the opener's `AppProvider` BY REFERENCE, already carrying the signed-in session
 * token, so the popout injects over the existing connection with no second auth
 * path and no second connection bootstrap.
 *
 * Gating: the trigger lives in the app header and is gated on
 * `can('simulate', 'fixed_income')` (disabled + denial tooltip, never hidden).
 *
 * Accessibility: a labelled `<section>` titled "Simulator", Esc to close (bound to
 * the POPOUT window via the panel's `ownerDocument`), a Tab focus trap, and
 * initial focus on the first control.
 */

import { useEffect, useId, useMemo, useRef, useState } from "react";

import type { DeskRequestKind, DeskRequestState, OisInstrument, Side } from "../data/contract";
import type { CelnetTransport } from "../data/transport";
import { DEFAULT_USD_SOFR_CURVE } from "../data/ratesPricing";
import { Button } from "./Button";
import styles from "./SimulatorPanel.module.css";

const MM = 1_000_000;
/** The calibrating curve set every injected request prices against (live desk). */
const CURVE = DEFAULT_USD_SOFR_CURVE;
/** The rates desk injected requests route to (the canonical G10 rates desk). */
const DESK = "g10-rates";
/** The fixed leg the desk re-prices to par against `CURVE` when it quotes. */
const FIXED_RATE = 0.04;
/** The injected request's time-to-live before it expires in the inbox. */
const TTL_MS = 120_000;

/** A record of one injected request paired with the desk's server-minted id. */
interface InjectedItem {
  /** The server-minted `DeskRequest.requestId` returned by the injection. */
  requestId: string;
  kind: DeskRequestKind;
  counterparty: string;
  tenorYears: number;
  /** Notional in the curve currency (positive; direction carried by `side`). */
  notional: number;
  side: Side;
  /** The lifecycle state the desk assigned on receipt (PENDING). */
  state: DeskRequestState;
}

/** Pay-fixed (BUY) / receive-fixed (SELL) label for an OIS direction. */
function sideLabel(side: Side): string {
  return side === "BUY" ? "Pay fixed" : "Receive fixed";
}

/** Format a notional (curve ccy) as a compact millions figure. */
function fmtMm(notional: number): string {
  return `${(notional / MM).toLocaleString(undefined, { maximumFractionDigits: 1 })}mm`;
}

export interface SimulatorPanelProps {
  /**
   * The LIVE transport from the opener's app context, passed by reference so the
   * injection runs over the SAME authenticated connection — only the desk-submit
   * seam is needed here.
   */
  transport: Pick<CelnetTransport, "submitDeskRequest">;
  /** Close the simulator (closes the popout window). */
  onClose: () => void;
}

export function SimulatorPanel({ transport, onClose }: SimulatorPanelProps): React.ReactElement {
  const titleId = useId();
  const noteId = useId();
  const panelRef = useRef<HTMLElement | null>(null);
  const firstRef = useRef<HTMLSelectElement | null>(null);

  const [kind, setKind] = useState<DeskRequestKind>("RFQ");
  const [counterparty, setCounterparty] = useState("Acme Capital");
  const [tenorYears, setTenorYears] = useState(5);
  const [notionalMm, setNotionalMm] = useState(50);
  const [side, setSide] = useState<Side>("BUY");
  const [items, setItems] = useState<InjectedItem[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Initial focus on the first control once the panel mounts in the popout.
  useEffect(() => {
    firstRef.current?.focus();
  }, []);

  // Esc to close + a Tab focus trap. The key listener binds to the panel's OWN
  // window (the popout), not the opener — events in the popout DOM never reach the
  // opener's window, so we resolve the realm via the mounted node's ownerDocument.
  useEffect(() => {
    const win = panelRef.current?.ownerDocument?.defaultView ?? window;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.key !== "Tab") return;
      const root = panelRef.current;
      if (!root) return;
      const focusable = root.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      );
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (!first || !last) return;
      const active = root.ownerDocument.activeElement as HTMLElement | null;
      if (e.shiftKey && active === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    };
    win.addEventListener("keydown", onKey);
    return () => win.removeEventListener("keydown", onKey);
  }, [onClose]);

  const generate = async (): Promise<void> => {
    if (busy) return;
    const name = counterparty.trim() || "Counterparty";
    const notional = notionalMm * MM;
    const instrument: OisInstrument = {
      tenorYears,
      fixedRate: FIXED_RATE,
      notional,
      direction: side === "BUY" ? "PAY_FIXED" : "RECEIVE_FIXED",
    };
    setBusy(true);
    setError(null);
    try {
      // Real injection into the live desk over the shared authenticated transport
      // — the request enters the RFQ/IOI inbox PENDING and the desk prices it.
      const res = await transport.submitDeskRequest({
        kind,
        counterparty: name,
        desk: DESK,
        instrument,
        curveSet: CURVE,
        side,
        notional,
        ttlMs: TTL_MS,
      });
      const r = res.request;
      const item: InjectedItem = {
        requestId: r.requestId,
        kind: r.kind,
        counterparty: r.counterparty,
        tenorYears: r.instrument.tenorYears,
        notional: r.notional,
        side: r.side,
        state: r.state,
      };
      setItems((cur) => [item, ...cur]);
    } catch (err) {
      setError(err instanceof Error ? err.message : "could not inject the request");
    } finally {
      setBusy(false);
    }
  };

  const summary = useMemo(
    () => `${items.length} injected request${items.length === 1 ? "" : "s"}`,
    [items.length],
  );

  return (
    <section
      ref={panelRef}
      className={styles.panel}
      aria-labelledby={titleId}
      aria-describedby={noteId}
    >
      <header className={styles.head}>
        <div className={styles.headRow}>
          <h1 id={titleId} className={styles.title}>
            Simulator
          </h1>
          <span className={styles.liveTag}>Live desk</span>
          <Button variant="ghost" onClick={onClose} aria-label="close simulator">
            Close
          </Button>
        </div>
        <p id={noteId} className={styles.note} role="note">
          Injects into the live desk for pricing. Each generated RFQ/IOI is sent to the{" "}
          <strong>{DESK}</strong> desk and enters the real RFQ/IOI inbox, where the desk prices and
          quotes it.
        </p>
      </header>

      <div className={styles.body}>
        <section className={styles.generator} aria-label="generate a desk request">
          <div className={styles.row}>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>Kind</span>
              <select
                ref={firstRef}
                className={styles.select}
                value={kind}
                aria-label="request kind"
                onChange={(e) => setKind(e.target.value as DeskRequestKind)}
              >
                <option value="RFQ">RFQ</option>
                <option value="IOI">IOI</option>
              </select>
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>Counterparty</span>
              <input
                className={styles.input}
                type="text"
                value={counterparty}
                aria-label="counterparty name"
                onChange={(e) => setCounterparty(e.target.value)}
              />
            </label>
          </div>
          <div className={styles.row}>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>Tenor (y)</span>
              <input
                className={styles.input}
                type="number"
                min={1}
                step={1}
                value={tenorYears}
                aria-label="tenor in years"
                onChange={(e) => setTenorYears(Math.max(1, Math.trunc(Number(e.target.value))))}
              />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>Notional (mm)</span>
              <input
                className={styles.input}
                type="number"
                min={1}
                step={5}
                value={notionalMm}
                aria-label="notional in millions"
                onChange={(e) => setNotionalMm(Math.max(1, Number(e.target.value)))}
              />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>Side</span>
              <select
                className={styles.select}
                value={side}
                aria-label="request side"
                onChange={(e) => setSide(e.target.value as Side)}
              >
                <option value="BUY">Pay</option>
                <option value="SELL">Receive</option>
              </select>
            </label>
          </div>
          <div className={styles.actions}>
            <Button variant="primary" onClick={() => void generate()} disabled={busy}>
              {busy ? "Injecting…" : "Generate"}
            </Button>
          </div>
          {error && (
            <p className={styles.error} role="alert">
              {error}
            </p>
          )}
        </section>

        <section className={styles.list} aria-label="injected requests">
          <div className={styles.listHead}>
            <h2 className={styles.listTitle}>Injected (live desk)</h2>
            <span className={styles.count}>{summary}</span>
          </div>
          {items.length === 0 ? (
            <p className={styles.empty}>
              No requests injected yet — generate an RFQ or IOI above to send one to the desk.
            </p>
          ) : (
            <ul className={styles.itemList}>
              {items.map((it) => (
                <li key={it.requestId} className={styles.item}>
                  <div className={styles.itemTop}>
                    <span className={`${styles.kindBadge} ${styles[`kind${it.kind}`] ?? ""}`}>
                      {it.kind}
                    </span>
                    <span className={styles.cpty}>{it.counterparty}</span>
                    <span className={styles.terms}>
                      {it.tenorYears}y OIS · {fmtMm(it.notional)} · {sideLabel(it.side)}
                    </span>
                  </div>
                  <dl className={styles.quote}>
                    <div className={styles.metric}>
                      <dt className={styles.metricLabel}>Request id</dt>
                      <dd className={styles.metricValueEmphatic}>{it.requestId}</dd>
                    </div>
                    <div className={styles.metric}>
                      <dt className={styles.metricLabel}>State</dt>
                      <dd className={styles.metricValue}>{it.state}</dd>
                    </div>
                  </dl>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
    </section>
  );
}
