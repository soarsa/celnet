/**
 * CurveWorkspace — the rates CURVE inspection surface (FI-ARCHITECTURE §4.2). A
 * trader builds / perturbs the calibrated USD-SOFR curve set (its par-OIS pillars)
 * and inspects the bootstrapped discount curve three ways: the discount factor
 * `DF(t)`, the continuously-compounded zero rate `z(t) = −ln DF(t)/t`, and the
 * instantaneous forward `f(t) = −d ln DF/dt`, across the curve span, alongside the
 * pillar par-rate ladder.
 *
 * The curve math is REAL and SHARED: the workspace samples the SAME in-browser
 * bootstrap the OIS pricer uses (`src/data/ratesPricing.ts`, `bootstrapCurveFromSet`
 * / `sampleCurve`), interpolated log-linear-on-log-DF — the shipping default of the
 * server's `celnet-rates::curve`. Editing a pillar re-bootstraps the curve and
 * re-samples every view; a malformed edit surfaces the real validation error rather
 * than a fabricated curve.
 */

import { useCallback, useMemo, useState } from "react";
import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import { DataGrid } from "../components/DataGrid";
import { Sparkline } from "../components/Sparkline";
import { CurveChart, type CurveSeries } from "../viz/CurveChart";
import type { ColumnDef } from "../lib/grid";
import type { RatesCurveSet } from "../data/contract";
import {
  bootstrapCurveFromSet,
  discountFactorAt,
  instantaneousForwardAt,
  sampleCurve,
  zeroRateAt,
  DEFAULT_USD_SOFR_CURVE,
  type CurveSamplePoint,
} from "../data/ratesPricing";
import styles from "./CurveWorkspace.module.css";

/** Number of points sampled across the span for the term-structure plots. */
const SAMPLE_COUNT = 96;

/** One editable par-OIS pillar in the builder (the par rate held in percent). */
interface EditablePillar {
  readonly tenorYears: number;
  readonly parRatePct: number;
}

/** The DEFAULT pillar ladder, lifted into the editor's percent representation. */
const INITIAL_PILLARS: readonly EditablePillar[] = DEFAULT_USD_SOFR_CURVE.pillars.map(
  (p) => ({ tenorYears: p.tenorYears, parRatePct: p.parRate * 100 }),
);

/** One row of the pillar ladder: the quote and its bootstrapped curve readings. */
interface LadderRow {
  readonly tenorYears: number;
  readonly parRate: number;
  readonly zero: number;
  readonly df: number;
}

/** Format a decimal rate as a percentage with bp precision (0.0405 → "4.0500%"). */
function fmtRatePct(rate: number): string {
  return `${(rate * 100).toFixed(4)}%`;
}

/** Format a discount factor to 6 places (0.81873 → "0.818731"). */
function fmtDf(df: number): string {
  return df.toFixed(6);
}

/** Axis tick: a decimal rate as bare percent points (0.0405 → "4.05"). */
function fmtRateAxis(rate: number): string {
  return (rate * 100).toFixed(2);
}

/** Axis tick: a year-fraction time as a compact tenor (5 → "5y"). */
function fmtTenorAxis(t: number): string {
  return `${t.toFixed(t < 1 ? 1 : 0)}y`;
}

const LADDER_COLUMNS: readonly ColumnDef<LadderRow>[] = [
  {
    key: "pillar",
    header: "Pillar",
    width: 80,
    align: "left",
    accessor: (r) => `${r.tenorYears}y`,
  },
  {
    key: "par",
    header: "Par OIS",
    unit: "%",
    width: 120,
    accessor: (r) => fmtRatePct(r.parRate),
  },
  {
    key: "zero",
    header: "Zero (cc)",
    unit: "%",
    width: 120,
    accessor: (r) => fmtRatePct(r.zero),
  },
  {
    key: "df",
    header: "DF",
    width: 120,
    accessor: (r) => fmtDf(r.df),
  },
];

export function CurveWorkspace(): React.ReactElement {
  const [pillars, setPillars] = useState<readonly EditablePillar[]>(INITIAL_PILLARS);
  const [horizonYears, setHorizonYears] = useState(5);

  const span = pillars[pillars.length - 1]?.tenorYears ?? 0;
  const horizon = Math.min(Math.max(horizonYears, 0), span);

  // The curve set under inspection, assembled from the (editable) pillars over the
  // DEFAULT reference date + currency — the single contract the bootstrap consumes.
  const curveSet = useMemo<RatesCurveSet>(
    () => ({
      currency: DEFAULT_USD_SOFR_CURVE.currency,
      referenceDate: DEFAULT_USD_SOFR_CURVE.referenceDate,
      pillars: pillars.map((p) => ({ tenorYears: p.tenorYears, parRate: p.parRatePct / 100 })),
    }),
    [pillars],
  );

  // Bootstrap once + sample the span. A malformed edit (e.g. a negative par rate the
  // root-find cannot bracket) throws a real RatesPricingError we surface, never a
  // fabricated curve.
  const built = useMemo(() => {
    try {
      const discount = bootstrapCurveFromSet(curveSet);
      const samples = sampleCurve(curveSet, { samples: SAMPLE_COUNT });
      return { discount, samples, error: null as string | null };
    } catch (err) {
      return {
        discount: null,
        samples: [] as CurveSamplePoint[],
        error: err instanceof Error ? err.message : "curve build failed",
      };
    }
  }, [curveSet]);

  const { discount, samples, error } = built;

  // The three term-structure series, sharing the sampled time grid.
  const dfSeries = useMemo<CurveSeries[]>(
    () => [{ label: "DF(t)", tone: "offer", points: samples.map((s) => ({ x: s.t, y: s.df })) }],
    [samples],
  );
  const rateSeries = useMemo<CurveSeries[]>(
    () => [
      { label: "zero z(t)", tone: "accent", points: samples.map((s) => ({ x: s.t, y: s.zero })) },
      { label: "forward f(t)", tone: "bid", points: samples.map((s) => ({ x: s.t, y: s.forward })) },
    ],
    [samples],
  );

  // Compact echoes for the headline cards (reuses the shared Sparkline primitive).
  const dfTrace = useMemo(() => samples.map((s) => s.df), [samples]);
  const zeroTrace = useMemo(() => samples.map((s) => s.zero), [samples]);
  const forwardTrace = useMemo(() => samples.map((s) => s.forward), [samples]);

  // The inspected-horizon readout, read from the very curve the plots sample.
  const horizonReadout = useMemo(() => {
    if (!discount) return null;
    return {
      df: discountFactorAt(discount, horizon),
      zero: zeroRateAt(discount, horizon),
      forward: instantaneousForwardAt(discount, horizon),
    };
  }, [discount, horizon]);

  const ladder = useMemo<LadderRow[]>(() => {
    if (!discount) return [];
    return pillars.map((p) => ({
      tenorYears: p.tenorYears,
      parRate: p.parRatePct / 100,
      zero: zeroRateAt(discount, p.tenorYears),
      df: discountFactorAt(discount, p.tenorYears),
    }));
  }, [discount, pillars]);

  const ladderGroups = useMemo(
    () => [
      {
        key: "",
        label: "",
        rows: ladder.map((r) => ({ key: String(r.tenorYears), datum: r })),
      },
    ],
    [ladder],
  );

  const setPillarRate = useCallback((index: number, pct: number) => {
    setPillars((prev) => prev.map((p, j) => (j === index ? { ...p, parRatePct: pct } : p)));
  }, []);

  const resetPillars = useCallback(() => setPillars(INITIAL_PILLARS), []);

  const isDirty = useMemo(
    () => pillars.some((p, i) => p.parRatePct !== INITIAL_PILLARS[i]?.parRatePct),
    [pillars],
  );

  return (
    <div className={styles.wrap}>
      <Panel material="float" className={styles.builder} title="Curve set">
        <div className={styles.curveRow}>
          <span className={styles.curveLabel}>Curve</span>
          <span className={styles.curveName}>{curveSet.currency}-SOFR</span>
          <span className={styles.curveMeta}>
            {pillars.length} pillars · ref {curveSet.referenceDate.year}-
            {String(curveSet.referenceDate.month).padStart(2, "0")}-
            {String(curveSet.referenceDate.day).padStart(2, "0")} · log-linear-on-log-DF
          </span>
        </div>

        <div className={styles.pillarHead}>
          <span className={styles.fieldLabel}>Par-OIS pillars</span>
          {isDirty && (
            <Button variant="ghost" onClick={resetPillars} title="restore the calibrated pillar rates">
              Reset
            </Button>
          )}
        </div>

        <ul className={styles.pillarList}>
          {pillars.map((p, i) => (
            <li key={p.tenorYears} className={styles.pillarItem}>
              <span className={styles.pillarTenor}>{p.tenorYears}y</span>
              <label className={styles.inlineInput}>
                <input
                  type="number"
                  step={0.01}
                  value={p.parRatePct}
                  aria-label={`${p.tenorYears} year par rate in percent`}
                  onChange={(e) => setPillarRate(i, Number(e.target.value))}
                />
                <span className={styles.inputUnit}>%</span>
              </label>
            </li>
          ))}
        </ul>

        <div className={styles.field}>
          <span className={styles.fieldLabel}>Inspect horizon</span>
          <div className={styles.tenorPicks}>
            {pillars.map((p) => (
              <button
                key={p.tenorYears}
                type="button"
                className={`${styles.tenorPill} ${horizonYears === p.tenorYears ? styles.tenorActive : ""}`}
                onClick={() => setHorizonYears(p.tenorYears)}
                aria-pressed={horizonYears === p.tenorYears}
              >
                {p.tenorYears}y
              </button>
            ))}
            <label className={styles.inlineInput}>
              <input
                type="number"
                min={0}
                max={span}
                step={0.5}
                value={horizonYears}
                aria-label="inspect horizon in years"
                onChange={(e) => setHorizonYears(Number(e.target.value))}
              />
              <span className={styles.inputUnit}>y</span>
            </label>
          </div>
        </div>

        {error && (
          <p className={styles.error} role="alert">
            {error}
          </p>
        )}
      </Panel>

      <Panel className={styles.results} title="Discount curve">
        {discount && horizonReadout ? (
          <>
            <dl className={styles.metrics}>
              <HorizonMetric
                label={`DF(${horizon}y)`}
                value={fmtDf(horizonReadout.df)}
                trace={dfTrace}
                ariaLabel="discount factor term structure"
                emphatic
              />
              <HorizonMetric
                label={`Zero z(${horizon}y)`}
                value={fmtRatePct(horizonReadout.zero)}
                trace={zeroTrace}
                ariaLabel="zero rate term structure"
              />
              <HorizonMetric
                label={`Forward f(${horizon}y)`}
                value={fmtRatePct(horizonReadout.forward)}
                trace={forwardTrace}
                ariaLabel="instantaneous forward term structure"
              />
            </dl>

            <div className={styles.chart}>
              <h3 className={styles.chartTitle}>Discount factor</h3>
              <CurveChart
                series={dfSeries}
                xLabel="tenor (years)"
                formatX={fmtTenorAxis}
                formatY={(v) => v.toFixed(3)}
                markerX={horizon}
              />
            </div>

            <div className={styles.chart}>
              <h3 className={styles.chartTitle}>Zero & instantaneous forward · %</h3>
              <CurveChart
                series={rateSeries}
                xLabel="tenor (years)"
                formatX={fmtTenorAxis}
                formatY={fmtRateAxis}
                markerX={horizon}
              />
            </div>

            <div className={styles.ladder}>
              <h3 className={styles.chartTitle}>Pillar par-rate ladder</h3>
              <DataGrid label="curve pillar ladder" columns={LADDER_COLUMNS} groups={ladderGroups} />
            </div>
          </>
        ) : (
          <p className={styles.empty}>
            {error
              ? "Adjust the pillars to a valid set to bootstrap and inspect the curve."
              : "Bootstrapping the curve…"}
          </p>
        )}
      </Panel>
    </div>
  );
}

/** One headline curve reading at the horizon, with a compact term-structure trace. */
function HorizonMetric({
  label,
  value,
  trace,
  ariaLabel,
  emphatic,
}: {
  label: string;
  value: string;
  trace: number[];
  ariaLabel: string;
  emphatic?: boolean;
}): React.ReactElement {
  return (
    <div className={`${styles.metric} ${emphatic ? styles.metricEmphatic : ""}`}>
      <dt className={styles.metricLabel}>{label}</dt>
      <dd className={styles.metricValue}>{value}</dd>
      <Sparkline values={trace} width={132} height={26} ariaLabel={ariaLabel} />
    </div>
  );
}
