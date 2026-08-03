/**
 * AcceptanceTracePanel — the "what would fire" affordance (the analogue of the hedge /
 * risk-routing trace). The trader dials a sample incoming LIFT and the panel walks the
 * CURRENTLY-COMPILED policy graph through {@link traceAcceptanceGraph} (the exact
 * server-parity engine port) to show which decision (accept / reject / hold) resolves,
 * with its reason and the node path — BEFORE arming the policy.
 */
import { useMemo, useState } from "react";

import type { AcceptanceGraph } from "../../data/contract";
import { acceptanceActionLabel, describeAcceptanceAction } from "../../lib/acceptanceAction";
import {
  blankAcceptanceLift,
  traceAcceptanceGraph,
  type AcceptanceSampleLift,
} from "../../lib/acceptanceTrace";
import styles from "./AcceptanceWorkspace.module.css";

interface AcceptanceTracePanelProps {
  /** The compiled policy graph (enabled rules only) — traced live as the trader dials. */
  graph: AcceptanceGraph;
}

const TEXT_FIELDS: readonly { key: keyof AcceptanceSampleLift; label: string }[] = [
  { key: "counterparty", label: "Counterparty" },
  { key: "side", label: "Side" },
  { key: "assetClass", label: "Asset class" },
  { key: "desk", label: "Desk" },
  { key: "instrumentSymbol", label: "Instrument" },
];

const NUMERIC_FIELDS: readonly {
  key: keyof AcceptanceSampleLift;
  label: string;
  step?: number;
}[] = [
  { key: "notionalUsd", label: "Notional (USD)" },
  { key: "tenorYears", label: "Tenor (years)", step: 0.5 },
  { key: "edgeBps", label: "Edge (bps)", step: 0.1 },
  { key: "quoteAgeMs", label: "Quote age (ms)", step: 50 },
];

export function AcceptanceTracePanel({ graph }: AcceptanceTracePanelProps): React.ReactElement {
  const [lift, setLift] = useState<AcceptanceSampleLift>(() => ({
    ...blankAcceptanceLift(),
    counterparty: "Citadel",
    side: "buy",
    assetClass: "fixed_income",
    desk: "emea",
    instrumentSymbol: "US10Y",
    notionalUsd: 25_000_000,
    tenorYears: 10,
    edgeBps: 0.3,
    quoteAgeMs: 400,
  }));

  const trace = useMemo(() => traceAcceptanceGraph(graph, lift), [graph, lift]);
  const setText = (key: keyof AcceptanceSampleLift, value: string): void =>
    setLift((s) => ({ ...s, [key]: value }));
  const setNum = (key: keyof AcceptanceSampleLift, value: number): void =>
    setLift((s) => ({ ...s, [key]: value }));

  const decisionKind = trace.landedAction?.kind ?? "accept";

  return (
    <section
      className={styles.tracePanel}
      aria-labelledby="acceptance-trace-heading"
      data-testid="acceptance-trace"
    >
      <h3 id="acceptance-trace-heading" className={styles.panelHeading}>
        What would fire?
      </h3>
      <p className={styles.panelNote}>
        Dial a sample incoming lift; the current policy is walked live to show the decision it
        resolves to — before you arm it.
      </p>

      <div className={styles.traceGrid}>
        {TEXT_FIELDS.map((f) => (
          <label key={f.key} className={styles.traceField}>
            <span className={styles.traceFieldLabel}>{f.label}</span>
            <input
              className={styles.input}
              type="text"
              value={lift[f.key] as string}
              data-testid={`trace-${f.key}`}
              onChange={(e) => setText(f.key, e.target.value)}
            />
          </label>
        ))}
        {NUMERIC_FIELDS.map((f) => (
          <label key={f.key} className={styles.traceField}>
            <span className={styles.traceFieldLabel}>{f.label}</span>
            <input
              className={styles.input}
              type="number"
              step={f.step ?? 1}
              value={lift[f.key] as number}
              data-testid={`trace-${f.key}`}
              onChange={(e) => setNum(f.key, Number(e.target.value))}
            />
          </label>
        ))}
      </div>

      <div className={styles.traceResult} data-testid="trace-result">
        <span
          className={`${styles.decisionChip} ${styles[`dec_${decisionKind}`]}`}
          data-testid="trace-decision"
        >
          {trace.outcome === "decision" ? acceptanceActionLabel(decisionKind) : "—"}
        </span>
        <span className={styles.traceResultLabel}>Resolves to</span>
        <strong className={styles.traceAction} data-testid="trace-action">
          {trace.outcome === "decision"
            ? describeAcceptanceAction(trace.landedAction)
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
