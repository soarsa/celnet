/**
 * SimulatorPanel — the standalone, permission-gated counterparty SANDBOX.
 *
 * A pure client-side sandbox for exercising the dealer-quoting shapes (RFQ / IOI
 * / order) WITHOUT touching the live desk: generated items never call
 * `SubmitDeskRequest` (or any server/desk/pricing RPC) and so never enter the
 * real RFQ/IOI inbox or the priced flow. Each generated item is shaped like the
 * desk's display types and is paired with a sample quote computed DETERMINISTICALLY
 * in-browser via {@link priceRatesOffline} — the very same local rates math the
 * offline edge prices against — so the numbers are realistic, not stubbed.
 *
 * Gating: the trigger lives in the app header and is gated on
 * `can('simulate', 'fixed_income')` (disabled + denial tooltip, never hidden).
 *
 * Accessibility: `role="dialog"` + `aria-modal`, labelled title, Esc to close, a
 * Tab focus trap, and initial focus on the first control.
 */

import { useEffect, useId, useMemo, useRef, useState } from "react";

import type { OisInstrument, Side } from "../data/contract";
import { DEFAULT_USD_SOFR_CURVE, priceRatesOffline } from "../data/ratesPricing";
import { fmtPnlAdaptive, fmtRate } from "../lib/format";
import { Button } from "./Button";
import styles from "./SimulatorPanel.module.css";

const MM = 1_000_000;
const CURVE = DEFAULT_USD_SOFR_CURVE;

/** The sandbox item kinds — the desk's RFQ/IOI plus a working order. */
type SimKind = "RFQ" | "IOI" | "ORDER";

/** A locally-generated sandbox item paired with its deterministic sample quote. */
interface SimItem {
  /** Deterministic per-session id (`sim-1`, `sim-2`, …). */
  id: string;
  kind: SimKind;
  counterparty: string;
  tenorYears: number;
  /** Notional in the curve currency (positive; direction carried by `side`). */
  notional: number;
  side: Side;
  /** The sample quoted rate (the fair par rate on the offline curve), a decimal. */
  quoteRate: number;
  /** Sample PV at the par rate (curve ccy). */
  pv: number;
  /** Sample DV01 (curve ccy / bp). */
  dv01: number;
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
  open: boolean;
  onClose: () => void;
}

export function SimulatorPanel({ open, onClose }: SimulatorPanelProps): React.ReactElement | null {
  const titleId = useId();
  const bannerId = useId();
  const panelRef = useRef<HTMLDivElement | null>(null);
  const firstRef = useRef<HTMLSelectElement | null>(null);

  const [kind, setKind] = useState<SimKind>("RFQ");
  const [counterparty, setCounterparty] = useState("Sandbox Counterparty");
  const [tenorYears, setTenorYears] = useState(5);
  const [notionalMm, setNotionalMm] = useState(50);
  const [side, setSide] = useState<Side>("BUY");
  const [items, setItems] = useState<SimItem[]>([]);
  const [seq, setSeq] = useState(0);
  const [error, setError] = useState<string | null>(null);

  // Initial focus on the first control whenever the dialog opens.
  useEffect(() => {
    if (open) firstRef.current?.focus();
  }, [open]);

  // Esc to close + a Tab focus trap that keeps focus inside the panel.
  useEffect(() => {
    if (!open) return;
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
      const active = document.activeElement as HTMLElement | null;
      if (e.shiftKey && active === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const generate = (): void => {
    const name = counterparty.trim() || "Counterparty";
    const notional = notionalMm * MM;
    const instrument: OisInstrument = {
      tenorYears,
      fixedRate: 0.04,
      notional,
      direction: side === "BUY" ? "PAY_FIXED" : "RECEIVE_FIXED",
    };
    try {
      // Deterministic, client-only sample quote — NO transport / server / desk call.
      const priced = priceRatesOffline(CURVE, instrument);
      const nextSeq = seq + 1;
      const item: SimItem = {
        id: `sim-${nextSeq}`,
        kind,
        counterparty: name,
        tenorYears,
        notional,
        side,
        quoteRate: priced.parRate,
        pv: priced.pv,
        dv01: priced.dv01,
      };
      setSeq(nextSeq);
      setItems((cur) => [item, ...cur]);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "could not generate a sample quote");
    }
  };

  const summary = useMemo(
    () => `${items.length} simulated item${items.length === 1 ? "" : "s"}`,
    [items.length],
  );

  if (!open) return null;

  return (
    <div
      className={styles.scrim}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={panelRef}
        className={styles.panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={bannerId}
      >
        <header className={styles.head}>
          <div className={styles.headRow}>
            <h2 id={titleId} className={styles.title}>
              Simulator
            </h2>
            <span className={styles.sandboxTag}>Sandbox</span>
            <Button variant="ghost" onClick={onClose} aria-label="close simulator">
              Close
            </Button>
          </div>
          <p id={bannerId} className={styles.banner} role="note">
            Simulated — not sent to the desk. Generated items stay in this sandbox and never
            enter the live RFQ/IOI inbox or the priced flow.
          </p>
        </header>

        <div className={styles.body}>
          <section className={styles.generator} aria-label="generate a sandbox item">
            <div className={styles.row}>
              <label className={styles.field}>
                <span className={styles.fieldLabel}>Kind</span>
                <select
                  ref={firstRef}
                  className={styles.select}
                  value={kind}
                  aria-label="item kind"
                  onChange={(e) => setKind(e.target.value as SimKind)}
                >
                  <option value="RFQ">RFQ</option>
                  <option value="IOI">IOI</option>
                  <option value="ORDER">Order</option>
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
                  aria-label="item side"
                  onChange={(e) => setSide(e.target.value as Side)}
                >
                  <option value="BUY">Pay</option>
                  <option value="SELL">Receive</option>
                </select>
              </label>
            </div>
            <div className={styles.actions}>
              <Button variant="primary" onClick={generate}>
                Generate
              </Button>
            </div>
            {error && (
              <p className={styles.error} role="alert">
                {error}
              </p>
            )}
          </section>

          <section className={styles.list} aria-label="simulated items">
            <div className={styles.listHead}>
              <h3 className={styles.listTitle}>Generated (sandbox)</h3>
              <span className={styles.count}>{summary}</span>
            </div>
            {items.length === 0 ? (
              <p className={styles.empty}>
                No simulated items yet — generate an RFQ, IOI, or order above.
              </p>
            ) : (
              <ul className={styles.itemList}>
                {items.map((it) => (
                  <li key={it.id} className={styles.item}>
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
                        <dt className={styles.metricLabel}>
                          {it.kind === "ORDER" ? "Working level" : "Sample quote"}
                        </dt>
                        <dd className={styles.metricValueEmphatic}>{fmtRate(it.quoteRate)}</dd>
                      </div>
                      <div className={styles.metric}>
                        <dt className={styles.metricLabel}>PV ({CURVE.currency})</dt>
                        <dd className={styles.metricValue}>{fmtPnlAdaptive(it.pv)}</dd>
                      </div>
                      <div className={styles.metric}>
                        <dt className={styles.metricLabel}>DV01</dt>
                        <dd className={styles.metricValue}>{fmtPnlAdaptive(it.dv01)}/bp</dd>
                      </div>
                    </dl>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}
