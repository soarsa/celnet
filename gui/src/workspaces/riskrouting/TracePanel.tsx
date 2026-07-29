/**
 * The "trace a sample fill" panel. A trader composes a sample fill (or picks a
 * preset); on every change the graph is walked in TS ({@link traceGraph}) and the
 * landing book is shown — while the canvas lights up the traversed path. Because
 * {@link traceGraph} is a faithful port of the Rust router, the book shown here is
 * the book the server would route the fill to.
 */
import type { RiskBook } from "../../data/contract";
import { type SampleFill, type TraceResult, blankFill } from "../../lib/routeTrace";
import styles from "./RiskRoutingWorkspace.module.css";

interface Preset {
  label: string;
  fill: SampleFill;
}

/** A few worked sample fills matching the canonical rules — instant demonstrations. */
const PRESETS: readonly Preset[] = [
  { label: "EUR 60m", fill: { ...blankFill(), ccy: "EUR", notional: 60_000_000, product: "vanilla" } },
  { label: "10Y swap", fill: { ...blankFill(), product: "swap", tenor: 10, ccy: "USD" } },
  { label: "HF-1 flow", fill: { ...blankFill(), counterparty: "HF-1", ccy: "GBP", notional: 5_000_000 } },
  { label: "Small USD", fill: { ...blankFill(), ccy: "USD", notional: 250_000, product: "forward" } },
];

interface TracePanelProps {
  fill: SampleFill;
  trace: TraceResult | null;
  books: readonly RiskBook[];
  onChange: (fill: SampleFill) => void;
}

export function TracePanel({ fill, trace, books, onChange }: TracePanelProps): React.ReactElement {
  const patch = (p: Partial<SampleFill>): void => onChange({ ...fill, ...p });
  const bookName = (id: string): string => books.find((b) => b.id === id)?.name ?? id;

  const outcome = ((): React.ReactElement => {
    if (!trace) return <span className={styles.traceMiss}>—</span>;
    if (trace.outcome === "book" && trace.landedBook !== null) {
      return (
        <span className={styles.traceBook} data-testid="trace-landing">
          {bookName(trace.landedBook)}
        </span>
      );
    }
    if (trace.outcome === "cycle") return <span className={styles.traceMiss}>cycle — fix the graph</span>;
    return <span className={styles.traceMiss}>no landing (broken edge)</span>;
  })();

  return (
    <section className={styles.trace} aria-label="Trace a sample fill">
      <h2 className={styles.traceTitle}>Trace a sample fill</h2>

      <div className={styles.presetRow}>
        {PRESETS.map((p) => (
          <button
            key={p.label}
            type="button"
            className={styles.presetBtn}
            onClick={() => onChange(p.fill)}
            data-testid={`preset-${p.label}`}
          >
            {p.label}
          </button>
        ))}
      </div>

      <div className={styles.traceGrid}>
        <label className={styles.traceField}>
          <span>Ccy</span>
          <input className={styles.input} value={fill.ccy} onChange={(e) => patch({ ccy: e.target.value })} data-testid="trace-ccy" />
        </label>
        <label className={styles.traceField}>
          <span>Notional</span>
          <input
            className={styles.input}
            type="number"
            value={fill.notional}
            onChange={(e) => patch({ notional: Number(e.target.value) })}
            data-testid="trace-notional"
          />
        </label>
        <label className={styles.traceField}>
          <span>Product</span>
          <input className={styles.input} value={fill.product} onChange={(e) => patch({ product: e.target.value })} data-testid="trace-product" />
        </label>
        <label className={styles.traceField}>
          <span>Side</span>
          <input className={styles.input} value={fill.side} onChange={(e) => patch({ side: e.target.value })} />
        </label>
        <label className={styles.traceField}>
          <span>Tenor (y)</span>
          <input
            className={styles.input}
            type="number"
            value={fill.tenor}
            onChange={(e) => patch({ tenor: Number(e.target.value) })}
            data-testid="trace-tenor"
          />
        </label>
        <label className={styles.traceField}>
          <span>Counterparty</span>
          <input
            className={styles.input}
            value={fill.counterparty}
            onChange={(e) => patch({ counterparty: e.target.value })}
            data-testid="trace-counterparty"
          />
        </label>
        <label className={styles.traceField}>
          <span>Desk</span>
          <input className={styles.input} value={fill.desk} onChange={(e) => patch({ desk: e.target.value })} />
        </label>
        <label className={styles.traceField}>
          <span>Strike</span>
          <input className={styles.input} type="number" value={fill.strike} onChange={(e) => patch({ strike: Number(e.target.value) })} />
        </label>
      </div>

      <div className={styles.traceResult}>
        <span className={styles.traceResultLabel}>Routes to</span>
        {outcome}
      </div>
    </section>
  );
}
