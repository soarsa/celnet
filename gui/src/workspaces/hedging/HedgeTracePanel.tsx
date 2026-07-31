/**
 * HedgeTracePanel — the "what would fire" affordance (docs/AUTO-HEDGING §5.5, the
 * analogue of the risk-routing trace). The trader dials a sample RISK STATE and the
 * panel walks the CURRENTLY-COMPILED policy graph through {@link traceHedgeGraph} (the
 * exact server-parity engine port) to show which exit action fires, the RAG band, and
 * the node path — BEFORE arming the policy.
 */
import { useMemo, useState } from "react";

import type { HedgeGraph } from "../../data/contract";
import { describeExitAction } from "../../lib/hedgeExit";
import { blankHedgeState, traceHedgeGraph, type HedgeSampleState } from "../../lib/hedgeTrace";
import styles from "./HedgingWorkspace.module.css";

interface HedgeTracePanelProps {
  /** The compiled policy graph (enabled rules only) — traced live as the trader dials. */
  graph: HedgeGraph;
}

/** The band label for a utilisation (green < amber-ish; a breach at ≥ 1). */
function bandFor(utilization: number): string {
  if (utilization >= 1) return "breach";
  if (utilization >= 0.9) return "red";
  if (utilization >= 0.7) return "amber";
  return "green";
}

const NUMERIC_FIELDS: readonly {
  key: keyof HedgeSampleState;
  label: string;
  step?: number;
}[] = [
  { key: "utilization", label: "Utilization", step: 0.05 },
  { key: "overflow", label: "Overflow" },
  { key: "threshold", label: "Threshold" },
  { key: "netDv01", label: "Net DV01" },
  { key: "counterpartyToxicity", label: "Counterparty toxicity", step: 0.05 },
  { key: "internalOffsetAvailable", label: "Internal offset avail." },
  { key: "hedgeCostBp", label: "Hedge cost (bp)", step: 0.1 },
];

export function HedgeTracePanel({ graph }: HedgeTracePanelProps): React.ReactElement {
  const [state, setState] = useState<HedgeSampleState>(() => ({
    ...blankHedgeState(),
    book: "fi-rates-emea",
    instrumentId: "US10Y",
    utilization: 1.15,
    threshold: 250000,
    overflow: 75000,
    breached: true,
    counterpartyToxicity: 0.2,
    internalOffsetAvailable: 60000,
  }));

  const trace = useMemo(() => traceHedgeGraph(graph, state), [graph, state]);
  const band = bandFor(state.utilization);
  const setNum = (key: keyof HedgeSampleState, value: number): void =>
    setState((s) => ({ ...s, [key]: value }));

  return (
    <section className={styles.tracePanel} aria-labelledby="hedge-trace-heading" data-testid="hedge-trace">
      <h3 id="hedge-trace-heading" className={styles.panelHeading}>
        What would fire?
      </h3>
      <p className={styles.panelNote}>
        Dial a sample risk state; the current policy is walked live to show the exit action it
        resolves to — before you arm it.
      </p>

      <div className={styles.traceGrid}>
        <label className={styles.traceField}>
          <span className={styles.traceFieldLabel}>Book</span>
          <input
            className={styles.input}
            type="text"
            value={state.book}
            data-testid="trace-book"
            onChange={(e) => setState((s) => ({ ...s, book: e.target.value }))}
          />
        </label>
        <label className={styles.traceField}>
          <span className={styles.traceFieldLabel}>Instrument</span>
          <input
            className={styles.input}
            type="text"
            value={state.instrumentId}
            onChange={(e) => setState((s) => ({ ...s, instrumentId: e.target.value }))}
          />
        </label>
        <label className={styles.traceField}>
          <span className={styles.traceFieldLabel}>Breached</span>
          <select
            className={styles.input}
            value={state.breached ? "true" : "false"}
            data-testid="trace-breached"
            onChange={(e) => setState((s) => ({ ...s, breached: e.target.value === "true" }))}
          >
            <option value="true">true</option>
            <option value="false">false</option>
          </select>
        </label>
        {NUMERIC_FIELDS.map((f) => (
          <label key={f.key} className={styles.traceField}>
            <span className={styles.traceFieldLabel}>{f.label}</span>
            <input
              className={styles.input}
              type="number"
              step={f.step ?? 1}
              value={state[f.key] as number}
              data-testid={`trace-${f.key}`}
              onChange={(e) => setNum(f.key, Number(e.target.value))}
            />
          </label>
        ))}
      </div>

      <div className={styles.traceResult} data-testid="trace-result">
        <span className={`${styles.ragChip} ${styles[`rag_${band}`]}`} data-testid="trace-band">
          {band.toUpperCase()}
        </span>
        <span className={styles.traceResultLabel}>Fires</span>
        <strong className={styles.traceAction} data-testid="trace-action">
          {trace.outcome === "action"
            ? describeExitAction(trace.landedAction)
            : trace.outcome === "cycle"
              ? "— (cycle: fix the policy)"
              : "— (unreachable node)"}
        </strong>
        <span className={styles.tracePath} data-testid="trace-path">
          path: {trace.path.length > 0 ? trace.path.join(" → ") : "—"}
        </span>
      </div>
    </section>
  );
}
