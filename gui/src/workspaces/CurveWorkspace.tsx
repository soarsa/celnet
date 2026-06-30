/**
 * CurveWorkspace — the rates CURVE inspection surface (FI-ARCHITECTURE §4.2). A
 * trader builds / perturbs the calibrated USD-SOFR curve set (its par-OIS pillars)
 * and inspects the bootstrapped discount curve three ways: the discount factor
 * `DF(t)`, the continuously-compounded zero rate `z(t) = −ln DF(t)/t`, and the
 * instantaneous forward `f(t) = −d ln DF/dt`, across the curve span, alongside the
 * pillar par-rate ladder.
 *
 * Pillars are not restricted to the whole-year grid: each pillar's maturity is a
 * `PillarTenor` — a whole-year tenor, a month tenor (sub-/broken-year), or an
 * explicit odd-dated ("broken date") maturity — and the editor lets the trader add,
 * remove, and re-point pillars across all three arms.
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
import type { BrokenDate, PillarTenor, RatesCurveSet } from "../data/contract";
import { pillarTenorLabel } from "../data/contract";
import {
  bootstrapCurveFromSet,
  discountFactorAt,
  instantaneousForwardAt,
  pillarMaturityYears,
  sampleCurve,
  zeroRateAt,
  DEFAULT_USD_SOFR_CURVE,
  type CurveSamplePoint,
} from "../data/ratesPricing";
import styles from "./CurveWorkspace.module.css";

/** Number of points sampled across the span for the term-structure plots. */
const SAMPLE_COUNT = 96;

/** The curve reference (spot-anchor) date all pillar schedules roll from. */
const REFERENCE_DATE: BrokenDate = DEFAULT_USD_SOFR_CURVE.referenceDate;

/** One editable par-OIS pillar in the builder (the par rate held in percent). */
interface EditablePillar {
  readonly tenor: PillarTenor;
  readonly parRatePct: number;
}

/** The DEFAULT pillar ladder, lifted into the editor's percent representation. */
const INITIAL_PILLARS: readonly EditablePillar[] =
  DEFAULT_USD_SOFR_CURVE.pillars.map((p) => ({
    tenor: p.tenor,
    parRatePct: p.parRate * 100,
  }));

/** One row of the pillar ladder: the quote and its bootstrapped curve readings. */
interface LadderRow {
  readonly tenorLabel: string;
  readonly parRate: number;
  readonly zero: number;
  readonly df: number;
}

/** The three pillar-tenor arm kinds, in the order the selector offers them. */
const PILLAR_KINDS: readonly PillarTenor["kind"][] = [
  "years",
  "months",
  "date",
];

const KIND_LABEL: Record<PillarTenor["kind"], string> = {
  years: "Years",
  months: "Months",
  date: "Date",
};

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

/** A `BrokenDate` as the `<input type="date">` value (`"2031-06-30"`). */
function brokenToInput(d: BrokenDate): string {
  return `${d.year}-${String(d.month).padStart(2, "0")}-${String(d.day).padStart(2, "0")}`;
}

/** Parse a `"YYYY-MM-DD"` date-input value to a `BrokenDate`, or `null` if malformed. */
function inputToBroken(value: string): BrokenDate | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!m) return null;
  return { year: Number(m[1]), month: Number(m[2]), day: Number(m[3]) };
}

/** A representative whole-year value for a tenor, for seeding an arm switch. */
function representativeYears(tenor: PillarTenor): number {
  return Math.max(1, Math.round(pillarMaturityYears(tenor, REFERENCE_DATE)));
}

/** Convert a tenor to a different arm, preserving an approximate maturity. */
function switchKind(
  tenor: PillarTenor,
  kind: PillarTenor["kind"],
): PillarTenor {
  if (tenor.kind === kind) return tenor;
  const years = representativeYears(tenor);
  switch (kind) {
    case "years":
      return { kind: "years", years };
    case "months":
      return { kind: "months", months: years * 12 };
    case "date":
      return {
        kind: "date",
        maturityDate: {
          year: REFERENCE_DATE.year + years,
          month: REFERENCE_DATE.month,
          day: REFERENCE_DATE.day,
        },
      };
  }
}

const LADDER_COLUMNS: readonly ColumnDef<LadderRow>[] = [
  {
    key: "pillar",
    header: "Pillar",
    width: 96,
    align: "left",
    accessor: (r) => r.tenorLabel,
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
  const [pillars, setPillars] =
    useState<readonly EditablePillar[]>(INITIAL_PILLARS);
  const [horizonYears, setHorizonYears] = useState(5);

  // Each pillar's maturity in year-fraction from spot — the curve-time coordinate
  // the bootstrap places it at, uniform across the three arms.
  const pillarTimes = useMemo(
    () => pillars.map((p) => pillarMaturityYears(p.tenor, REFERENCE_DATE)),
    [pillars],
  );
  const span = pillarTimes.length ? pillarTimes[pillarTimes.length - 1]! : 0;
  const horizon = Math.min(Math.max(horizonYears, 0), span);

  // The curve set under inspection, assembled from the (editable) pillars over the
  // DEFAULT reference date + currency — the single contract the bootstrap consumes.
  const curveSet = useMemo<RatesCurveSet>(
    () => ({
      currency: DEFAULT_USD_SOFR_CURVE.currency,
      referenceDate: REFERENCE_DATE,
      pillars: pillars.map((p) => ({
        tenor: p.tenor,
        parRate: p.parRatePct / 100,
      })),
    }),
    [pillars],
  );

  // Bootstrap once + sample the span. A malformed edit (e.g. a negative par rate the
  // root-find cannot bracket, or out-of-order maturities) throws a real
  // RatesPricingError we surface, never a fabricated curve.
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
    () => [
      {
        label: "DF(t)",
        tone: "offer",
        points: samples.map((s) => ({ x: s.t, y: s.df })),
      },
    ],
    [samples],
  );
  const rateSeries = useMemo<CurveSeries[]>(
    () => [
      {
        label: "zero z(t)",
        tone: "accent",
        points: samples.map((s) => ({ x: s.t, y: s.zero })),
      },
      {
        label: "forward f(t)",
        tone: "bid",
        points: samples.map((s) => ({ x: s.t, y: s.forward })),
      },
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
    return pillars.map((p, i) => ({
      tenorLabel: pillarTenorLabel(p.tenor),
      parRate: p.parRatePct / 100,
      zero: zeroRateAt(discount, pillarTimes[i]!),
      df: discountFactorAt(discount, pillarTimes[i]!),
    }));
  }, [discount, pillars, pillarTimes]);

  const ladderGroups = useMemo(
    () => [
      {
        key: "",
        label: "",
        rows: ladder.map((r, i) => ({ key: `${r.tenorLabel}-${i}`, datum: r })),
      },
    ],
    [ladder],
  );

  const setPillarRate = useCallback((index: number, pct: number) => {
    setPillars((prev) =>
      prev.map((p, j) => (j === index ? { ...p, parRatePct: pct } : p)),
    );
  }, []);

  const setPillarTenor = useCallback((index: number, tenor: PillarTenor) => {
    setPillars((prev) =>
      prev.map((p, j) => (j === index ? { ...p, tenor } : p)),
    );
  }, []);

  const removePillar = useCallback((index: number) => {
    setPillars((prev) => prev.filter((_, j) => j !== index));
  }, []);

  const addPillar = useCallback(() => {
    setPillars((prev) => {
      const last = prev[prev.length - 1];
      const nextYears = last ? representativeYears(last.tenor) + 1 : 1;
      const parRatePct = last ? last.parRatePct : 4;
      return [
        ...prev,
        { tenor: { kind: "years", years: nextYears }, parRatePct },
      ];
    });
  }, []);

  const resetPillars = useCallback(() => setPillars(INITIAL_PILLARS), []);

  const isDirty = useMemo(
    () =>
      pillars.length !== INITIAL_PILLARS.length ||
      pillars.some(
        (p, i) =>
          p.parRatePct !== INITIAL_PILLARS[i]?.parRatePct ||
          pillarTenorLabel(p.tenor) !==
            pillarTenorLabel(INITIAL_PILLARS[i]!.tenor),
      ),
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
            {String(curveSet.referenceDate.day).padStart(2, "0")} ·
            log-linear-on-log-DF
          </span>
        </div>

        <div className={styles.pillarHead}>
          <span className={styles.fieldLabel}>Par-OIS pillars</span>
          <div className={styles.pillarActions}>
            <Button
              variant="ghost"
              onClick={addPillar}
              title="add a calibrating pillar"
            >
              + Pillar
            </Button>
            {isDirty && (
              <Button
                variant="ghost"
                onClick={resetPillars}
                title="restore the calibrated pillar set"
              >
                Reset
              </Button>
            )}
          </div>
        </div>

        <ul className={styles.pillarList}>
          {pillars.map((p, i) => (
            <li key={i} className={styles.pillarItem}>
              <select
                className={styles.pillarKind}
                value={p.tenor.kind}
                aria-label={`pillar ${i + 1} tenor kind`}
                onChange={(e) =>
                  setPillarTenor(
                    i,
                    switchKind(p.tenor, e.target.value as PillarTenor["kind"]),
                  )
                }
              >
                {PILLAR_KINDS.map((k) => (
                  <option key={k} value={k}>
                    {KIND_LABEL[k]}
                  </option>
                ))}
              </select>

              <PillarTenorInput
                tenor={p.tenor}
                index={i}
                onChange={(tenor) => setPillarTenor(i, tenor)}
              />

              <label className={styles.inlineInput}>
                <input
                  type="number"
                  step={0.01}
                  value={p.parRatePct}
                  aria-label={`pillar ${i + 1} par rate in percent`}
                  onChange={(e) => setPillarRate(i, Number(e.target.value))}
                />
                <span className={styles.inputUnit}>%</span>
              </label>

              <button
                type="button"
                className={styles.pillarRemove}
                aria-label={`remove pillar ${i + 1}`}
                title="remove this pillar"
                disabled={pillars.length <= 1}
                onClick={() => removePillar(i)}
              >
                ×
              </button>
            </li>
          ))}
        </ul>

        <div className={styles.field}>
          <span className={styles.fieldLabel}>Inspect horizon</span>
          <div className={styles.tenorPicks}>
            {pillars.map((p, i) => {
              const t = pillarTimes[i]!;
              return (
                <button
                  key={i}
                  type="button"
                  className={`${styles.tenorPill} ${horizonYears === t ? styles.tenorActive : ""}`}
                  onClick={() => setHorizonYears(t)}
                  aria-pressed={horizonYears === t}
                >
                  {pillarTenorLabel(p.tenor)}
                </button>
              );
            })}
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
                label={`DF(${horizon.toFixed(2)}y)`}
                value={fmtDf(horizonReadout.df)}
                trace={dfTrace}
                ariaLabel="discount factor term structure"
                emphatic
              />
              <HorizonMetric
                label={`Zero z(${horizon.toFixed(2)}y)`}
                value={fmtRatePct(horizonReadout.zero)}
                trace={zeroTrace}
                ariaLabel="zero rate term structure"
              />
              <HorizonMetric
                label={`Forward f(${horizon.toFixed(2)}y)`}
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
              <h3 className={styles.chartTitle}>
                Zero & instantaneous forward · %
              </h3>
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
              <DataGrid
                label="curve pillar ladder"
                columns={LADDER_COLUMNS}
                groups={ladderGroups}
              />
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

/**
 * The value editor for one pillar's tenor — a number input for the years / months
 * arms, a native date picker for the broken-date arm.
 */
function PillarTenorInput({
  tenor,
  index,
  onChange,
}: {
  tenor: PillarTenor;
  index: number;
  onChange: (tenor: PillarTenor) => void;
}): React.ReactElement {
  if (tenor.kind === "date") {
    return (
      <label className={styles.inlineInput}>
        <input
          type="date"
          value={brokenToInput(tenor.maturityDate)}
          aria-label={`pillar ${index + 1} maturity date`}
          onChange={(e) => {
            const d = inputToBroken(e.target.value);
            if (d) onChange({ kind: "date", maturityDate: d });
          }}
        />
      </label>
    );
  }
  const value = tenor.kind === "years" ? tenor.years : tenor.months;
  const unit = tenor.kind === "years" ? "y" : "m";
  return (
    <label className={styles.inlineInput}>
      <input
        type="number"
        min={1}
        step={1}
        value={value}
        aria-label={`pillar ${index + 1} tenor in ${tenor.kind}`}
        onChange={(e) => {
          const n = Math.max(1, Math.trunc(Number(e.target.value)));
          onChange(
            tenor.kind === "years"
              ? { kind: "years", years: n }
              : { kind: "months", months: n },
          );
        }}
      />
      <span className={styles.inputUnit}>{unit}</span>
    </label>
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
    <div
      className={`${styles.metric} ${emphatic ? styles.metricEmphatic : ""}`}
    >
      <dt className={styles.metricLabel}>{label}</dt>
      <dd className={styles.metricValue}>{value}</dd>
      <Sparkline values={trace} width={132} height={26} ariaLabel={ariaLabel} />
    </div>
  );
}
