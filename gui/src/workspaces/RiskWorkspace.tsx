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
  RiskBucketRequest,
  ScenarioResult,
  ShockAxis,
} from "../data/contract";
import { Panel } from "../components/Panel";
import { Button } from "../components/Button";
import { strategyInstrument } from "../data/seed";
import { rampColor } from "../viz/ramp";
import { fmtPnlAdaptive, fmtSigned } from "../lib/format";
import styles from "./RiskWorkspace.module.css";

const SPOT_STEPS = [-0.015, -0.005, 0, 0.005, 0.015];
const VOL_STEPS = [-0.02, -0.01, 0, 0.01, 0.02];

/** The delta pillars the vega ladder buckets on (signed convention deltas). */
const VEGA_DELTA_PILLARS = [0.5, 0.25, -0.25, 0.1, -0.1];
/** The theta-roll horizons (years rolled forward): overnight, a week, a month. */
const ROLL_HORIZONS = [1 / 365, 7 / 365, 30 / 365];

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

  // The book-shaped risk decomposition the server computes alongside the grid —
  // present ONLY when we ask for it (the server returns `bucketedRisk: null`
  // otherwise). Vega is bucketed per (tenor, delta) pillar: a single-expiry
  // structure carries vega only at its own expiry tenor, so we request that tenor
  // across the standard delta pillars (other tenors are honestly zero). Cross-gamma
  // covers the desk's coupled second-orders; theta rolls the standard horizons.
  const riskBuckets: RiskBucketRequest = useMemo(
    () => ({
      vegaPillars: VEGA_DELTA_PILLARS.map((delta) => ({
        tenorYears: instrument.expiryYears,
        delta,
      })),
      crossGammaPairs: [
        { factorA: "SPOT", factorB: "VOL" },
        { factorA: "RATE_DOM", factorB: "SPOT" },
        { factorA: "SPOT", factorB: "TIME" },
      ],
      rollHorizonsYears: ROLL_HORIZONS,
    }),
    [instrument.expiryYears],
  );

  useEffect(() => {
    let live = true;
    void app.transport
      .scenario(instrument, app.pairCtx.market, app.conventions, axes, riskBuckets)
      .then((r) => {
        if (live) setResult(r);
      });
    return () => {
      live = false;
    };
  }, [app.transport, instrument, app.pairCtx.market, app.conventions, axes, riskBuckets]);

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
    if (metric === "pnl") return fmtPnlAdaptive(v);
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
        {renderLadder(result, notional, basePrice)}
      </Panel>
    </div>
  );
}

/**
 * The book-shaped risk disclosure: bucketed vega per (tenor, delta) pillar and the
 * off-diagonal cross-gamma. Renders the server's real decomposition when present.
 * When the server returns no decomposition at all (`bucketedRisk === null`, e.g.
 * an edge that did not honor the risk-bucket request), or when every bucket is a
 * structural zero (an honest "no exposure at these pillars" — never fabricated),
 * an explicit empty-state is shown instead of a row of zeros that masquerade as
 * data. Position-level attribution (the desk's actual book vega, not this single
 * structure's) awaits the server's Positions/`AttributePnl` API — surfaced here as
 * a backlog note, not faked.
 */
function renderLadder(
  result: ScenarioResult,
  notional: number,
  basePrice: number,
): React.ReactElement {
  const br = result.bucketedRisk;
  if (!br) {
    return (
      <div className={styles.emptyState}>
        <p className={styles.emptyTitle}>No book-shaped risk returned</p>
        <p className={styles.emptyBody}>
          The edge did not return a risk decomposition for this scenario. Bucketed
          vega and cross-gamma are computed by the server only; nothing is
          fabricated here. Position-level P&amp;L attribution awaits the server&apos;s
          Positions / AttributePnl API.
        </p>
      </div>
    );
  }

  // The vega in the buckets is the structure's value sensitivity to a 1-vol-point
  // move of a pillar (premium-fraction units); scale to the book's notional so the
  // ladder reads in P&L-per-vol-point, consistent with the grid's notional view.
  const vegaPerPoint = (v: number): number => v * notional * 0.01;
  const anyVega = br.vegaBuckets.some((b) => Math.abs(b.vega) > 0);
  const max = Math.max(...br.vegaBuckets.map((x) => Math.abs(x.vega)), 1e-12);

  return (
    <>
      <div className={styles.ladder}>
        <div className={styles.ladderHead}>
          <span>Tenor</span>
          <span>Pillar</span>
          <span>Vega / vol-pt</span>
        </div>
        {anyVega ? (
          br.vegaBuckets.slice(0, 25).map((b, i) => (
            <div key={i} className={styles.ladderRow}>
              <span className="num">{tenorName(b.tenorYears)}</span>
              <span className="num">{pillarName(b.delta)}</span>
              <span className={styles.bar}>
                <span
                  className={styles.barFill}
                  style={{ width: `${(Math.abs(b.vega) / max) * 100}%` }}
                />
                <span className={`num ${styles.barVal}`}>
                  {fmtPnlAdaptive(vegaPerPoint(b.vega))}
                </span>
              </span>
            </div>
          ))
        ) : (
          <div className={styles.emptyInline}>
            No vega at the requested pillars — this single-expiry structure carries
            vega only at its own expiry tenor.
          </div>
        )}
      </div>
      <div className={styles.crossGamma}>
        <span className={styles.provLabel}>cross-gamma</span>
        {br.crossGammas.length > 0 ? (
          br.crossGammas.map((cg, i) => (
            <span key={i} className="num">
              {cg.factorA}×{cg.factorB} {fmtSigned(cg.value, 4)}
            </span>
          ))
        ) : (
          <span className={styles.emptyInline}>none requested</span>
        )}
      </div>
      <div className={styles.crossGamma}>
        <span className={styles.provLabel}>theta roll</span>
        {br.thetaRoll.length > 0 ? (
          br.thetaRoll.map((pv, i) => (
            <span key={i} className="num">
              {tenorName(br.rollHorizonsYears[i] ?? 0)}{" "}
              {fmtPnlAdaptive((pv - basePrice) * notional)}
            </span>
          ))
        ) : (
          <span className={styles.emptyInline}>none requested</span>
        )}
      </div>
    </>
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
