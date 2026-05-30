/**
 * RiskWorkspace — the scenario / what-if grid (GUI-DESIGN §4.4). A spot×vol
 * shock grid driven by SurfaceService.Scenario (ShockAxis abs/rel) for the
 * selected structure. Each cell is a P&L under the shock, tinted on the
 * perceptual diverging ramp (reads magnitude honestly, no rainbow); the
 * spot/vol "today" cell is anchored. Vega/gamma ladders are a disclosure, not a
 * separate screen. Pin scenarios for compare. One click from the ticket/blotter
 * (same Instrument), never re-keyed.
 */

import { useEffect, useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import type {
  Instrument,
  ScenarioResult,
  ShockAxis,
} from "../data/contract";
import { Panel } from "../components/Panel";
import { Button } from "../components/Button";
import { strategyInstrument } from "../data/seed";
import { rampColor } from "../viz/ramp";
import { fmtPnlK, fmtSigned } from "../lib/format";
import styles from "./RiskWorkspace.module.css";

const SPOT_STEPS = [-0.015, -0.005, 0, 0.005, 0.015];
const VOL_STEPS = [-0.02, -0.01, 0, 0.01, 0.02];

const METRICS = [
  { id: "pnl", label: "P&L" },
  { id: "delta", label: "Δ" },
  { id: "vega", label: "ν" },
] as const;
type Metric = (typeof METRICS)[number]["id"];

export function RiskWorkspace(): React.ReactElement {
  const app = useApp();
  const [metric, setMetric] = useState<Metric>("pnl");
  const [result, setResult] = useState<ScenarioResult | null>(null);
  const [pinned, setPinned] = useState<{ label: string; result: ScenarioResult }[]>([]);

  // The structure under analysis: a 25Δ RR on the active pair (promoted from the
  // ticket/blotter in the real flow — same Instrument object).
  const instrument: Instrument = useMemo(
    () => strategyInstrument(app.pairCtx.pair, 30 / 365, "RISK_REVERSAL", 10),
    [app.pairCtx.pair],
  );

  const axes: ShockAxis[] = useMemo(
    () => [
      { factor: "SPOT", relative: true, steps: SPOT_STEPS },
      { factor: "VOL", relative: false, steps: VOL_STEPS },
    ],
    [],
  );

  useEffect(() => {
    let live = true;
    void app.transport
      .scenario(instrument, app.pairCtx.market, app.conventions, axes)
      .then((r) => {
        if (live) setResult(r);
      });
    return () => {
      live = false;
    };
  }, [app.transport, instrument, app.pairCtx.market, app.conventions, axes]);

  if (!result) return <div className={styles.loading}>Repricing scenario…</div>;

  const notional = instrument.quantity.notional;
  const basePrice = result.points.find((p) => p.appliedShocks.every((s) => s === 0))?.greeks.price ?? 0;

  // Build the spot×vol matrix. appliedShocks order = [spot, vol].
  const cellValue = (spotStep: number, volStep: number): number => {
    const pt = result.points.find(
      (p) => Math.abs((p.appliedShocks[0] ?? 0) - spotStep) < 1e-9 && Math.abs((p.appliedShocks[1] ?? 0) - volStep) < 1e-9,
    );
    if (!pt) return 0;
    switch (metric) {
      case "pnl":
        return (pt.greeks.price - basePrice) * notional;
      case "delta":
        return pt.greeks.deltaSpot;
      case "vega":
        return pt.greeks.vega;
    }
  };

  // Magnitude normalization for the diverging tint.
  let maxAbs = 1e-9;
  for (const s of SPOT_STEPS) for (const v of VOL_STEPS) maxAbs = Math.max(maxAbs, Math.abs(cellValue(s, v)));

  const fmtCell = (v: number): string => {
    if (metric === "pnl") return fmtPnlK(v);
    if (metric === "delta") return fmtSigned(v, 3);
    return fmtSigned(v, 4);
  };

  return (
    <div className={styles.grid}>
      <Panel
        glyph="⊞"
        title={`Risk · ${app.pairCtx.pair.base}/${app.pairCtx.pair.quote} 25Δ RR (${notional / 1e6}mm)`}
        actions={
          <div className={styles.metricTabs}>
            {METRICS.map((m) => (
              <button
                key={m.id}
                className={`${styles.tab} ${metric === m.id ? styles.tabActive : ""}`}
                onClick={() => setMetric(m.id)}
              >
                {m.label}
              </button>
            ))}
          </div>
        }
      >
        <div className={styles.gridArea}>
          <div className={styles.axisLabel}>vol →</div>
          <table className={styles.matrix}>
            <thead>
              <tr>
                <th className={styles.corner}>spot ↓</th>
                {VOL_STEPS.map((v) => (
                  <th key={v} className="num">
                    {v === 0 ? "ATM" : `${v > 0 ? "+" : "−"}${Math.abs(v * 100).toFixed(0)}%`}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {SPOT_STEPS.map((s) => (
                <tr key={s}>
                  <th className={`num ${styles.rowHead}`}>
                    {s === 0 ? "0.0%" : `${s > 0 ? "+" : "−"}${Math.abs(s * 100).toFixed(1)}%`}
                  </th>
                  {VOL_STEPS.map((v) => {
                    const val = cellValue(s, v);
                    const anchored = s === 0 && v === 0;
                    const t = 0.5 + (val / maxAbs) * 0.5;
                    return (
                      <td
                        key={v}
                        className={`num ${styles.cell} ${anchored ? styles.anchored : ""}`}
                        style={{ background: rampColor(t, anchored ? 1 : 0.92) }}
                      >
                        {anchored && <span className={styles.nowMark}>▣</span>}
                        {fmtCell(val)}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <div className={styles.gridFoot}>
          <Button
            variant="secondary"
            onClick={() =>
              setPinned((p) => [
                ...p.slice(-2),
                { label: pinLabel(metric, app.pairCtx.pair.base), result },
              ])
            }
          >
            ▶ Pin scenario
          </Button>
          <div className={styles.pins}>
            {pinned.map((p, i) => (
              <span key={i} className={styles.pin}>
                {p.label}
              </span>
            ))}
          </div>
          <span className={styles.legendWrap}>
            <span className={styles.legLabel}>−</span>
            <span
              className={styles.legBar}
              style={{
                background: `linear-gradient(90deg, ${rampColor(0)}, ${rampColor(0.5)}, ${rampColor(1)})`,
              }}
            />
            <span className={styles.legLabel}>+</span>
          </span>
        </div>
      </Panel>

      <Panel glyph="Σ" title="Vega ladder" className={styles.ladderPanel}>
        <div className={styles.ladder}>
          <div className={styles.ladderHead}>
            <span>Tenor</span>
            <span>Pillar</span>
            <span>Vega</span>
          </div>
          {result.bucketedRisk.vegaBuckets.slice(0, 25).map((b, i) => {
            const max = Math.max(...result.bucketedRisk.vegaBuckets.map((x) => Math.abs(x.vega)), 1e-9);
            return (
              <div key={i} className={styles.ladderRow}>
                <span className="num">{tenorName(b.tenorYears)}</span>
                <span className="num">{pillarName(b.delta)}</span>
                <span className={styles.bar}>
                  <span
                    className={styles.barFill}
                    style={{ width: `${(Math.abs(b.vega) / max) * 100}%` }}
                  />
                  <span className={`num ${styles.barVal}`}>{fmtSigned(b.vega, 3)}</span>
                </span>
              </div>
            );
          })}
        </div>
        <div className={styles.crossGamma}>
          <span className={styles.provLabel}>cross-gamma</span>
          {result.bucketedRisk.crossGammas.map((cg, i) => (
            <span key={i} className="num">
              {cg.factorA}×{cg.factorB} {fmtSigned(cg.value, 4)}
            </span>
          ))}
        </div>
      </Panel>
    </div>
  );
}

function pinLabel(metric: Metric, base: string): string {
  return `${base} ${metric}`;
}

function tenorName(years: number): string {
  const days = Math.round(years * 365);
  if (days <= 1) return "ON";
  if (days < 28) return `${Math.round(days / 7)}W`;
  return `${Math.round(days / 30)}M`;
}

function pillarName(delta: number): string {
  if (Math.abs(delta) >= 0.49) return "ATM";
  return `${delta < 0 ? "−" : "+"}${Math.round(Math.abs(delta) * 100)}Δ`;
}
