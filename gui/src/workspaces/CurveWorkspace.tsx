/**
 * CurveWorkspace — the fixed-income (rates curve) LENS of the shared,
 * class-parametric `MarketDataWorkspace` (`fe-fi-migration` #2); the `curve` rail
 * row opens the Market Data workspace on this lens. Behaviour is unchanged: it is
 * the rates CURVE inspection surface (FI-ARCHITECTURE §4.2). A
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
 *
 * The pillar-editor term structure is drawn by the design-system `YieldCurve` chart
 * (mockup 14): the bootstrapped pillar zero rates feed its `CurveNode` contract, and
 * because it interpolates ln(DF) log-linearly from those nodes — the same scheme as
 * the bootstrap — the drawn zero / forward / DF overlays reproduce the workspace's
 * real curve, with an on-chart hover readout off the identical math. The curve model
 * is stated honestly: only log-linear-on-log-DF is wired here; monotone-convex and
 * turn / meeting jumps exist server-engine-side but are not on the wire `CurveSet`,
 * so they render as DISABLED Target affordances, never as fabricated curve math.
 *
 * A third lens — **Query · mark · scenario** — drives the FI market-data query
 * surface (SurfaceService `GetCurve` / `MarkCurve` / `CurveScenario`, ADR-0021: the
 * asset-class-agnostic query seam the FX vol surface already has, generalized so
 * fixed income rides it too). It reads a bootstrapped curve on a tenor axis, pins it
 * under a fresh server-assigned version (a later query reproduces that exact marked
 * curve), and bump-and-reprices it (parallel + optional per-pillar key-rate shift,
 * with an optional repriced leg). Every call rides the ONE contract through
 * `app.transport`, byte-identical live vs. the offline `?mock` bootstrap.
 */

import { useCallback, useMemo, useRef, useState } from "react";
import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import { DataGrid } from "../components/DataGrid";
import { Sparkline } from "../components/Sparkline";
import { CurveChart, type CurveSeries } from "../viz/CurveChart";
import { YieldCurve, type CurveNode } from "../viz/YieldCurve";
import { KeyRateLadder, type KeyRatePillar } from "../viz/KeyRateLadder";
import type { ColumnDef } from "../lib/grid";
import type {
  BrokenDate,
  CalibratedCurve,
  CurvePoint,
  CurveScenarioResult,
  DatePillar,
  GetCurveResult,
  InstrumentDef,
  MarkedCurve,
  OisInstrument,
  PillarTenor,
  RatesCurveSet,
  RatesInstrument,
} from "../data/contract";
import {
  INSTRUMENT_FAMILY_LABELS,
  oisRatesInstrument,
  pillarTenorLabel,
} from "../data/contract";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import { useApp } from "../app/AppContext";
import { useReferenceData } from "../hooks/useReferenceData";
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

/** The curve-authoring / query modes the workspace offers. */
type AuthoringMode = "pillars" | "instruments" | "query";

/** Number of points sampled across the span for the term-structure plots. */
const SAMPLE_COUNT = 96;

/** The curve reference (spot-anchor) date all pillar schedules roll from. */
const REFERENCE_DATE: BrokenDate = DEFAULT_USD_SOFR_CURVE.referenceDate;

/** One editable par-OIS pillar in the builder (the par rate held in percent). */
interface EditablePillar {
  /** Stable identity — the React key, so a row keeps its input/focus when the
   *  ladder re-sorts by maturity after a tenor edit (index keys would swap it). */
  readonly id: string;
  readonly tenor: PillarTenor;
  readonly parRatePct: number;
}

/** Monotonic source of stable pillar ids (client-only; no SSR reuse concern). */
let pillarIdSeq = 0;
function nextPillarId(): string {
  return `pillar-${pillarIdSeq++}`;
}

/**
 * Order a pillar set by true maturity — the single year-fraction coordinate
 * `pillarMaturityYears` yields for every arm (years / months / broken date), so a
 * `1M` pillar sorts above `1Y`, and a broken date sorts into its real slot. The
 * bootstrap requires strictly-increasing maturities; holding this as an invariant
 * after every edit keeps the ladder valid and the horizon picks in tenor order.
 */
function sortByMaturity(
  list: readonly EditablePillar[],
): readonly EditablePillar[] {
  return [...list].sort(
    (a, b) =>
      pillarMaturityYears(a.tenor, REFERENCE_DATE) -
      pillarMaturityYears(b.tenor, REFERENCE_DATE),
  );
}

/** The DEFAULT pillar ladder, lifted into the editor's percent representation. */
const INITIAL_PILLARS: readonly EditablePillar[] = sortByMaturity(
  DEFAULT_USD_SOFR_CURVE.pillars.map((p) => ({
    id: nextPillarId(),
    tenor: p.tenor,
    parRatePct: p.parRate * 100,
  })),
);

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

/**
 * Map the bootstrapped pillar ladder onto the `YieldCurve` pillar-node contract:
 * one dated node per pillar carrying the continuously-compounded zero rate the
 * SAME bootstrap produced there. The chart reconstructs ln DF(t_i) = −z_i·t_i from
 * these nodes, so its curve reproduces the workspace's real discount factors at
 * every pillar, and its log-linear-in-ln(DF) interpolation matches the shipping
 * default between them. Exported for the wiring test.
 */
export function curvePillarNodes(
  ladder: readonly { readonly tenorYears: number; readonly zero: number }[],
): CurveNode[] {
  return ladder.map((r) => ({
    label: `${r.tenorYears}y`,
    tenorYears: r.tenorYears,
    zeroRate: r.zero,
  }));
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

/**
 * The rates CURVE surface. Two coherent authoring modes share the same bootstrap
 * and the same discount/zero readout: the slice-A **pillar editor** (author the
 * par-OIS pillars directly across the years / months / broken-date arms) and the
 * **build-by-instrument-reference** mode (pick reference-data registry instruments
 * and supply a calibrating quote each — the SERVER resolves every id, bootstraps,
 * and returns the calibrated points). A tab switches between them.
 */
export function CurveWorkspace(): React.ReactElement {
  const [mode, setMode] = useState<AuthoringMode>("pillars");

  return (
    <div className={styles.page}>
      <div
        className={styles.modeTabs}
        role="tablist"
        aria-label="curve authoring mode"
      >
        <button
          type="button"
          role="tab"
          aria-selected={mode === "pillars"}
          className={`${styles.modeTab} ${mode === "pillars" ? styles.modeTabActive : ""}`}
          onClick={() => setMode("pillars")}
        >
          Pillar editor
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={mode === "instruments"}
          className={`${styles.modeTab} ${mode === "instruments" ? styles.modeTabActive : ""}`}
          onClick={() => setMode("instruments")}
        >
          By instrument reference
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={mode === "query"}
          className={`${styles.modeTab} ${mode === "query" ? styles.modeTabActive : ""}`}
          onClick={() => setMode("query")}
        >
          Query · mark · scenario
        </button>
      </div>
      {mode === "pillars" && <PillarEditorMode />}
      {mode === "instruments" && <InstrumentReferenceMode />}
      {mode === "query" && <CurveQueryMode />}
    </div>
  );
}

function PillarEditorMode(): React.ReactElement {
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

  // The YieldCurve pillar nodes, mapped off the SAME bootstrapped ladder: each
  // pillar's curve-time (year-fraction from spot) carries the bootstrapped zero
  // rate, so the drawn zero / forward / DF overlays reproduce the workspace's real
  // curve under the identical log-linear-in-ln(DF) scheme.
  const curveNodes = useMemo<CurveNode[]>(
    () =>
      curvePillarNodes(
        ladder.map((r, i) => ({ tenorYears: pillarTimes[i]!, zero: r.zero })),
      ),
    [ladder, pillarTimes],
  );

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
    // Re-sort after the tenor changes so the ladder stays in maturity order
    // (a `1M` edit floats to the top); the stable `id` key keeps the edited
    // row's input attached to it as it moves.
    setPillars((prev) =>
      sortByMaturity(prev.map((p, j) => (j === index ? { ...p, tenor } : p))),
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
      return sortByMaturity([
        ...prev,
        {
          id: nextPillarId(),
          tenor: { kind: "years", years: nextYears },
          parRatePct,
        },
      ]);
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

        {/*
         * Curve model — the ONLY interpolation this workspace's in-browser
         * bootstrap implements is log-linear-on-log-DF (`src/data/ratesPricing.ts`),
         * the server's shipping default. Monotone-convex and turn/meeting jumps are
         * real server-engine capabilities not yet reachable from here, so they
         * render disabled + Target-tagged — honest, not faked.
         */}
        <fieldset className={styles.modelField}>
          <legend className={styles.fieldLabel}>Curve model</legend>
          <div className={styles.modelChoices}>
            <label className={styles.modelChoice}>
              <input type="radio" name="curve-interpolation" defaultChecked />
              <span>Log-linear DF</span>
              <span className={styles.tagLive}>Live</span>
            </label>
            <label className={`${styles.modelChoice} ${styles.modelOff}`}>
              <input
                type="radio"
                name="curve-interpolation"
                disabled
                aria-describedby="curve-model-note"
              />
              <span>Monotone convex</span>
              <span className={styles.tagTarget}>Target</span>
            </label>
            <label className={`${styles.modelChoice} ${styles.modelOff}`}>
              <input type="checkbox" disabled aria-describedby="curve-model-note" />
              <span>Turn / meeting jumps</span>
              <span className={styles.tagTarget}>Target</span>
            </label>
          </div>
          <p id="curve-model-note" className={styles.modelNote}>
            Monotone-convex interpolation and turn / meeting-date jumps exist in the
            server engine (celnet-rates) but are not yet on the wire CurveSet or in
            this in-browser bootstrap — shown disabled, never approximated.
          </p>
        </fieldset>

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
            <li key={p.id} className={styles.pillarItem}>
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
                  key={p.id}
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

        {/*
         * Scope honesty: this editor's curve set is a request-scoped payload
         * (PriceRates / AggregateRatesRisk). Server-side curve publish + versioning
         * DOES now exist (SurfaceService MarkCurve / GetCurve, ADR-0021) — it lives in
         * the Query · mark · scenario lens; this editor stays request-scoped.
         */}
        <p className={styles.scopeNote}>
          Request-scoped curve set: pillar edits reprice this workspace and ride each
          pricing request. To pin a curve under a server version and read it back, use
          the Query · mark · scenario lens (SurfaceService MarkCurve / GetCurve).
        </p>
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
              <h3 className={styles.chartTitle}>
                Term structure · zero &amp; forward (%) · discount factor
              </h3>
              <YieldCurve
                nodes={curveNodes}
                interpolation="log-linear"
                height={300}
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

/** One added calibrating pillar in the instrument-reference builder. */
interface InstrumentPick {
  readonly instrumentId: string;
  readonly quotePct: number;
}

/**
 * One added standalone date-anchored pillar: a maturity date (`<input type="date">`
 * value) + its simple rate in percent, with a stable React list key.
 */
interface DatePick {
  readonly key: number;
  readonly dateInput: string;
  readonly quotePct: number;
}

/** One row of the calibrated-curve readout (a returned bootstrapped point). */
interface PointRow {
  readonly label: string;
  readonly timeYears: number;
  readonly df: number;
  readonly zero: number;
}

const POINT_COLUMNS: readonly ColumnDef<PointRow>[] = [
  {
    key: "instrument",
    header: "Instrument",
    width: 168,
    align: "left",
    accessor: (r) => r.label,
  },
  {
    key: "t",
    header: "Maturity",
    unit: "y",
    width: 96,
    accessor: (r) => r.timeYears.toFixed(2),
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

/**
 * Build-by-instrument-reference mode: pick reference-data registry instruments,
 * enter a calibrating quote per pillar, and call the server's `BuildCurve` — the
 * returned calibrated curve (discount-factor / zero-rate points) renders here. The
 * curve math is the SERVER's (no in-browser bootstrap on this path); the GUI only
 * assembles the request and renders the response.
 */
function InstrumentReferenceMode(): React.ReactElement {
  const app = useApp();
  const isAuthed = app.auth.user !== null;
  const refData = useReferenceData(app.transport, isAuthed);

  const [currency, setCurrency] = useState("USD");
  const [refInput, setRefInput] = useState(brokenToInput(REFERENCE_DATE));
  const [picks, setPicks] = useState<readonly InstrumentPick[]>([]);
  const [addId, setAddId] = useState("");
  const [datePicks, setDatePicks] = useState<readonly DatePick[]>([]);
  const dateKey = useRef(0);
  const [result, setResult] = useState<CalibratedCurve | null>(null);
  const [building, setBuilding] = useState(false);
  const [buildError, setBuildError] = useState<string | null>(null);

  const byId = useMemo(() => {
    const m = new Map<string, InstrumentDef>();
    for (const d of refData.instruments) m.set(d.instrumentId, d);
    return m;
  }, [refData.instruments]);

  // Only instruments in the chosen curve currency can calibrate it.
  const addable = useMemo(
    () =>
      refData.instruments.filter(
        (d) =>
          d.currency === currency &&
          !picks.some((p) => p.instrumentId === d.instrumentId),
      ),
    [refData.instruments, currency, picks],
  );

  const instrumentLabel = useCallback(
    (id: string): string => {
      const def = byId.get(id);
      if (!def) return id;
      return `${def.name} · ${INSTRUMENT_FAMILY_LABELS[def.family]}`;
    },
    [byId],
  );

  const addPick = useCallback(() => {
    if (!addId) return;
    setPicks((prev) =>
      prev.some((p) => p.instrumentId === addId)
        ? prev
        : [...prev, { instrumentId: addId, quotePct: 4 }],
    );
    setAddId("");
    setResult(null);
  }, [addId]);

  const removePick = useCallback((id: string) => {
    setPicks((prev) => prev.filter((p) => p.instrumentId !== id));
    setResult(null);
  }, []);

  const setQuote = useCallback((id: string, pct: number) => {
    setPicks((prev) =>
      prev.map((p) => (p.instrumentId === id ? { ...p, quotePct: pct } : p)),
    );
  }, []);

  const addDatePick = useCallback(() => {
    setDatePicks((prev) => [
      ...prev,
      {
        key: (dateKey.current += 1),
        dateInput: brokenToInput(REFERENCE_DATE),
        quotePct: 4,
      },
    ]);
    setResult(null);
  }, []);

  const removeDatePick = useCallback((key: number) => {
    setDatePicks((prev) => prev.filter((p) => p.key !== key));
    setResult(null);
  }, []);

  const setDatePickDate = useCallback((key: number, dateInput: string) => {
    setDatePicks((prev) =>
      prev.map((p) => (p.key === key ? { ...p, dateInput } : p)),
    );
    setResult(null);
  }, []);

  const setDatePickQuote = useCallback((key: number, pct: number) => {
    setDatePicks((prev) =>
      prev.map((p) => (p.key === key ? { ...p, quotePct: pct } : p)),
    );
  }, []);

  const build = useCallback(async (): Promise<void> => {
    const referenceDate = inputToBroken(refInput);
    if (!referenceDate) {
      setBuildError("reference date must be a valid YYYY-MM-DD date");
      return;
    }
    if (picks.length === 0 && datePicks.length === 0) {
      setBuildError("add at least one calibrating instrument or date pillar");
      return;
    }
    const datePillars: DatePillar[] = [];
    for (const dp of datePicks) {
      const maturityDate = inputToBroken(dp.dateInput);
      if (!maturityDate) {
        setBuildError("each date pillar needs a valid YYYY-MM-DD maturity date");
        return;
      }
      datePillars.push({ maturityDate, quote: dp.quotePct / 100 });
    }
    setBuilding(true);
    setBuildError(null);
    try {
      const curve = await app.transport.buildCurve({
        requestId: `curve-${Date.now()}`,
        currency,
        referenceDate,
        pillars: picks.map((p) => ({
          instrumentId: p.instrumentId,
          quote: p.quotePct / 100,
        })),
        datePillars,
      });
      setResult(curve);
    } catch (e: unknown) {
      setResult(null);
      setBuildError(e instanceof Error ? e.message : "curve build failed");
    } finally {
      setBuilding(false);
    }
  }, [app.transport, currency, refInput, picks, datePicks]);

  const pointRows = useMemo<PointRow[]>(() => {
    if (!result) return [];
    // Instrument pillars carry an id the registry resolves to a name; date-anchored
    // pillars carry an empty id and a server-supplied `Date YYYY-MM-DD` label.
    return result.points.map((p) => ({
      label: p.instrumentId ? instrumentLabel(p.instrumentId) : p.label,
      timeYears: p.timeYears,
      df: p.discountFactor,
      zero: p.zeroRate,
    }));
  }, [result, instrumentLabel]);

  const pointGroups = useMemo(
    () => [
      {
        key: "",
        label: "",
        rows: pointRows.map((r, i) => ({ key: `${r.label}-${i}`, datum: r })),
      },
    ],
    [pointRows],
  );

  const dfSeries = useMemo<CurveSeries[]>(() => {
    if (!result) return [];
    return [
      {
        label: "DF(t)",
        tone: "offer",
        points: result.points.map((p) => ({ x: p.timeYears, y: p.discountFactor })),
      },
    ];
  }, [result]);

  const zeroSeries = useMemo<CurveSeries[]>(() => {
    if (!result) return [];
    return [
      {
        label: "zero z(t)",
        tone: "accent",
        points: result.points.map((p) => ({ x: p.timeYears, y: p.zeroRate })),
      },
    ];
  }, [result]);

  return (
    <div className={styles.wrap}>
      <Panel
        material="float"
        className={styles.builder}
        title="Reference instruments"
      >
        <p className={styles.hint}>
          Pick calibrating instruments from the reference-data registry and enter
          each observed quote. The server resolves every id, bootstraps, and
          returns the calibrated discount curve.
        </p>

        <div className={styles.curveRow}>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>Currency</span>
            <input
              className={styles.ccyInput}
              type="text"
              value={currency}
              aria-label="curve currency"
              maxLength={3}
              onChange={(e) => {
                setCurrency(e.target.value.toUpperCase().slice(0, 3));
                setPicks([]);
                setResult(null);
              }}
            />
          </label>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>Reference date</span>
            <input
              type="date"
              value={refInput}
              aria-label="curve reference date"
              onChange={(e) => {
                setRefInput(e.target.value);
                setResult(null);
              }}
            />
          </label>
        </div>

        {!isAuthed && (
          <p className={styles.notice} role="status">
            Sign in to load the instrument registry and build curves.
          </p>
        )}

        <div className={styles.pillarHead}>
          <span className={styles.fieldLabel}>Calibrating instruments</span>
        </div>

        <div className={styles.picker}>
          <select
            className={styles.pickerSelect}
            value={addId}
            aria-label="instrument to add"
            disabled={addable.length === 0}
            onChange={(e) => setAddId(e.target.value)}
          >
            <option value="">
              {addable.length === 0
                ? `No more ${currency} instruments`
                : `Select a ${currency} instrument…`}
            </option>
            {addable.map((d) => (
              <option key={d.instrumentId} value={d.instrumentId}>
                {d.name} · {INSTRUMENT_FAMILY_LABELS[d.family]}
              </option>
            ))}
          </select>
          <Button
            variant="ghost"
            onClick={addPick}
            disabled={!addId}
            title="add this instrument as a calibrating pillar"
          >
            + Add
          </Button>
        </div>

        {picks.length > 0 && (
          <ul className={styles.pillarList}>
            {picks.map((p) => (
              <li key={p.instrumentId} className={styles.pickItem}>
                <span className={styles.pickLabel}>
                  {instrumentLabel(p.instrumentId)}
                </span>
                <label className={styles.inlineInput}>
                  <input
                    type="number"
                    step={0.01}
                    value={p.quotePct}
                    aria-label={`${p.instrumentId} calibrating quote in percent`}
                    onChange={(e) =>
                      setQuote(p.instrumentId, Number(e.target.value))
                    }
                  />
                  <span className={styles.inputUnit}>%</span>
                </label>
                <button
                  type="button"
                  className={styles.pillarRemove}
                  aria-label={`remove ${p.instrumentId}`}
                  title="remove this pillar"
                  onClick={() => removePick(p.instrumentId)}
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
        )}

        <div className={styles.pillarHead}>
          <span className={styles.fieldLabel}>Date pillars</span>
          <Button
            variant="ghost"
            onClick={addDatePick}
            title="pin the curve to an explicit maturity date"
          >
            + Date pillar
          </Button>
        </div>

        <p className={styles.hint}>
          Pin the curve to an explicit date (a turn, an IMM, a meeting) with its simple
          rate. The server calibrates a synthetic cash deposit to that date.
        </p>

        {datePicks.length > 0 && (
          <ul className={styles.pillarList}>
            {datePicks.map((dp) => (
              <li key={dp.key} className={styles.pickItem}>
                <label className={styles.inlineInput}>
                  <input
                    type="date"
                    value={dp.dateInput}
                    aria-label={`date pillar ${dp.key} maturity date`}
                    onChange={(e) => setDatePickDate(dp.key, e.target.value)}
                  />
                </label>
                <label className={styles.inlineInput}>
                  <input
                    type="number"
                    step={0.01}
                    value={dp.quotePct}
                    aria-label={`date pillar ${dp.key} rate in percent`}
                    onChange={(e) =>
                      setDatePickQuote(dp.key, Number(e.target.value))
                    }
                  />
                  <span className={styles.inputUnit}>%</span>
                </label>
                <button
                  type="button"
                  className={styles.pillarRemove}
                  aria-label={`remove date pillar ${dp.key}`}
                  title="remove this date pillar"
                  onClick={() => removeDatePick(dp.key)}
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
        )}

        <div className={styles.buildRow}>
          <Button
            variant="primary"
            onClick={() => void build()}
            disabled={
              building ||
              (picks.length === 0 && datePicks.length === 0) ||
              !isAuthed
            }
            title="bootstrap the curve from the selected pillars"
          >
            {building ? "Building…" : "Build curve"}
          </Button>
        </div>

        {refData.error && (
          <p className={styles.error} role="alert">
            {refData.error}
          </p>
        )}
        {buildError && (
          <p className={styles.error} role="alert">
            {buildError}
          </p>
        )}
      </Panel>

      <Panel className={styles.results} title="Calibrated curve">
        {result && result.points.length > 0 ? (
          <>
            <div className={styles.curveRow}>
              <span className={styles.curveLabel}>Curve</span>
              <span className={styles.curveName}>
                {result.currency} discount
              </span>
              <span className={styles.curveMeta}>
                {result.points.length} pillars · ref{" "}
                {result.referenceDate.year}-
                {String(result.referenceDate.month).padStart(2, "0")}-
                {String(result.referenceDate.day).padStart(2, "0")} · server
                bootstrap
              </span>
            </div>

            <div className={styles.chart}>
              <h3 className={styles.chartTitle}>Discount factor</h3>
              <CurveChart
                series={dfSeries}
                xLabel="tenor (years)"
                formatX={fmtTenorAxis}
                formatY={(v) => v.toFixed(3)}
              />
            </div>

            <div className={styles.chart}>
              <h3 className={styles.chartTitle}>Zero rate · %</h3>
              <CurveChart
                series={zeroSeries}
                xLabel="tenor (years)"
                formatX={fmtTenorAxis}
                formatY={fmtRateAxis}
              />
            </div>

            <div className={styles.ladder}>
              <h3 className={styles.chartTitle}>Calibrated points</h3>
              <DataGrid
                label="calibrated curve points"
                columns={POINT_COLUMNS}
                groups={pointGroups}
              />
            </div>
          </>
        ) : (
          <p className={styles.empty}>
            {isAuthed
              ? "Add calibrating instruments and build to bootstrap the discount curve."
              : "Sign in to build a curve from registry instruments."}
          </p>
        )}
      </Panel>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Query · mark · scenario — the FI market-data query lens (SurfaceService
// GetCurve / MarkCurve / CurveScenario, ADR-0021). The discount-curve analogue of
// the FX vol surface's GetSmile / MarkSurface / Scenario, driven through the ONE
// contract (`app.transport`) so live and offline `?mock` are byte-identical.
// ---------------------------------------------------------------------------

/** The standard query tenor grid (year fractions), clamped to the curve span. */
const QUERY_TENOR_GRID: readonly number[] = [
  0.25, 0.5, 1, 2, 3, 5, 7, 10, 15, 20, 30,
];

/** One row of the queried-curve readout (a returned {@link CurvePoint}). */
interface QueryPointRow {
  readonly tenorLabel: string;
  readonly zero: number;
  readonly df: number;
}

const QUERY_POINT_COLUMNS: readonly ColumnDef<QueryPointRow>[] = [
  {
    key: "tenor",
    header: "Tenor",
    width: 96,
    align: "left",
    accessor: (r) => r.tenorLabel,
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

/** Map returned curve points onto the `YieldCurve` pillar-node contract. */
function pointsToNodes(points: readonly CurvePoint[]): CurveNode[] {
  return points.map((p) => ({
    label: fmtTenorAxis(p.tenorYears),
    tenorYears: p.tenorYears,
    zeroRate: p.zeroRate,
  }));
}

/** Format a signed currency amount (PV / DV01) with thousands separators. */
function fmtCcy(v: number): string {
  return v.toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
}

/**
 * The Query · mark · scenario lens. A trader edits a par-OIS curve set, then drives
 * the three FI market-data query verbs against it through the one contract:
 *   • GetCurve  — read the bootstrapped curve on a tenor axis (live, or pinned to a
 *                 marked version so a later read reproduces the exact marked curve);
 *   • MarkCurve — bootstrap + pin the curve under a fresh server-assigned version;
 *   • CurveScenario — parallel (+ optional per-pillar key-rate) bump-and-reprice,
 *                 with an optional repriced OIS leg (PV impact + base-curve DV01).
 * License-gated on `fixed_income` (disabled + tooltip, never hidden; the server
 * still enforces). The queried / shifted curves reuse the `YieldCurve` chart and the
 * repriced leg's key-rate DV01 reuses the `KeyRateLadder`.
 */
function CurveQueryMode(): React.ReactElement {
  const app = useApp();

  // Capability gating (never hidden — disabled + denial tooltip, server-enforced;
  // anonymous ⇒ permissive, exactly as the rates booking / risk lenses gate).
  const canView = app.auth.can("view", "fixed_income");
  const canMark = app.auth.can("price", "fixed_income");
  const canSimulate = app.auth.can("simulate", "fixed_income");

  // The editable base curve set (par rates in percent), seeded from the default
  // USD-SOFR ladder — the single contract the query / mark / scenario verbs consume.
  const [pillars, setPillars] =
    useState<readonly EditablePillar[]>(INITIAL_PILLARS);

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

  // A local bootstrap probe: a malformed edit disables the verbs + surfaces the real
  // RatesPricingError, never a fabricated curve (mirrors the pillar editor).
  const curveError = useMemo<string | null>(() => {
    try {
      bootstrapCurveFromSet(curveSet);
      return null;
    } catch (err) {
      return err instanceof Error ? err.message : "curve build failed";
    }
  }, [curveSet]);

  const span = useMemo(() => {
    const times = curveSet.pillars.map((p) =>
      pillarMaturityYears(p.tenor, curveSet.referenceDate),
    );
    return times.length ? Math.max(...times) : 0;
  }, [curveSet]);

  const queryTenors = useMemo(
    () => QUERY_TENOR_GRID.filter((t) => t <= span + 1e-9),
    [span],
  );

  // Query / mark / scenario async state.
  const [queryResult, setQueryResult] = useState<GetCurveResult | null>(null);
  const [queryError, setQueryError] = useState<string | null>(null);
  const [querying, setQuerying] = useState(false);

  const [marked, setMarked] = useState<MarkedCurve | null>(null);
  const [markError, setMarkError] = useState<string | null>(null);
  const [marking, setMarking] = useState(false);

  const [parallelBp, setParallelBp] = useState(25);
  const [keyRateMode, setKeyRateMode] = useState(false);
  const [keyRateBp, setKeyRateBp] = useState<readonly number[]>([]);
  const [repriceLeg, setRepriceLeg] = useState(false);
  const [legTenorY, setLegTenorY] = useState(5);
  const [legFixedPct, setLegFixedPct] = useState(4);
  const [legNotional, setLegNotional] = useState(10_000_000);
  const [legDirection, setLegDirection] =
    useState<OisInstrument["direction"]>("PAY_FIXED");
  const [scenario, setScenario] = useState<CurveScenarioResult | null>(null);
  const [legLadder, setLegLadder] = useState<readonly KeyRatePillar[] | null>(
    null,
  );
  const [scenarioError, setScenarioError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);

  const setPillarRate = useCallback((index: number, pct: number) => {
    setPillars((prev) =>
      prev.map((p, j) => (j === index ? { ...p, parRatePct: pct } : p)),
    );
    // A curve edit invalidates the pinned/queried/scenario reads off the old curve.
    setQueryResult(null);
    setScenario(null);
  }, []);

  const setKeyRate = useCallback((index: number, bp: number) => {
    setKeyRateBp((prev) => {
      const next = [...prev];
      while (next.length < index + 1) next.push(0);
      next[index] = bp;
      return next;
    });
  }, []);

  const legInstrument = useCallback((): RatesInstrument | undefined => {
    if (!repriceLeg) return undefined;
    const ois: OisInstrument = {
      tenorYears: legTenorY,
      fixedRate: legFixedPct / 100,
      notional: legNotional,
      direction: legDirection,
    };
    return oisRatesInstrument(ois);
  }, [repriceLeg, legTenorY, legFixedPct, legNotional, legDirection]);

  const runQuery = useCallback(
    async (pinnedVersion?: bigint): Promise<void> => {
      setQuerying(true);
      setQueryError(null);
      try {
        const result = await app.transport.getCurve(
          pinnedVersion === undefined ? curveSet : null,
          queryTenors,
          pinnedVersion,
        );
        setQueryResult(result);
      } catch (e: unknown) {
        setQueryResult(null);
        setQueryError(e instanceof Error ? e.message : "curve query failed");
      } finally {
        setQuerying(false);
      }
    },
    [app.transport, curveSet, queryTenors],
  );

  const runMark = useCallback(async (): Promise<void> => {
    if (!canMark) return;
    setMarking(true);
    setMarkError(null);
    try {
      const result = await app.transport.markCurve(curveSet);
      setMarked(result);
    } catch (e: unknown) {
      setMarked(null);
      setMarkError(e instanceof Error ? e.message : "curve mark failed");
    } finally {
      setMarking(false);
    }
  }, [app.transport, curveSet, canMark]);

  const runScenario = useCallback(async (): Promise<void> => {
    if (!canSimulate) return;
    setRunning(true);
    setScenarioError(null);
    try {
      const keyVec = keyRateMode
        ? pillars.map((_, i) => keyRateBp[i] ?? 0)
        : [];
      const instrument = legInstrument();
      const result = await app.transport.curveScenario(
        curveSet,
        parallelBp,
        keyVec,
        queryTenors,
        instrument,
      );
      setScenario(result);
      // The repriced leg's key-rate DV01 ladder (base curve) — a second read through
      // the SAME `price_rates` seam, reconciling Σ key-rate DV01 ≈ scenario DV01.
      if (instrument) {
        const priced = await app.transport.priceRates(curveSet, instrument);
        setLegLadder(
          pillars.map((p, i) => ({
            pillar: pillarTenorLabel(p.tenor),
            dv01: priced.keyRateLadder[i] ?? 0,
          })),
        );
      } else {
        setLegLadder(null);
      }
    } catch (e: unknown) {
      setScenario(null);
      setLegLadder(null);
      setScenarioError(
        e instanceof Error ? e.message : "curve scenario failed",
      );
    } finally {
      setRunning(false);
    }
  }, [
    app.transport,
    curveSet,
    parallelBp,
    keyRateMode,
    keyRateBp,
    pillars,
    queryTenors,
    legInstrument,
    canSimulate,
  ]);

  const queryNodes = useMemo<CurveNode[]>(
    () => (queryResult ? pointsToNodes(queryResult.points) : []),
    [queryResult],
  );
  const scenarioNodes = useMemo<CurveNode[]>(
    () => (scenario ? pointsToNodes(scenario.points) : []),
    [scenario],
  );

  const queryPointGroups = useMemo(
    () => [
      {
        key: "",
        label: "",
        rows: (queryResult?.points ?? []).map((p, i) => ({
          key: `${p.tenorYears}-${i}`,
          datum: {
            tenorLabel: fmtTenorAxis(p.tenorYears),
            zero: p.zeroRate,
            df: p.discountFactor,
          } as QueryPointRow,
        })),
      },
    ],
    [queryResult],
  );

  const disabled = !!curveError || querying;
  const pinnedVersion = marked?.curveVersion ?? null;

  return (
    <div className={styles.wrap}>
      <Panel material="float" className={styles.builder} title="Curve set">
        <p className={styles.hint}>
          Edit the calibrating par-OIS pillars, then read / pin / shift the
          bootstrapped curve through the one contract (SurfaceService GetCurve /
          MarkCurve / CurveScenario, ADR-0021).
        </p>

        <div className={styles.curveRow}>
          <span className={styles.curveLabel}>Curve</span>
          <span className={styles.curveName}>{curveSet.currency}-SOFR</span>
          <span className={styles.curveMeta}>
            {pillars.length} pillars · ref {curveSet.referenceDate.year}-
            {String(curveSet.referenceDate.month).padStart(2, "0")}-
            {String(curveSet.referenceDate.day).padStart(2, "0")}
          </span>
        </div>

        {!canView && (
          <p className={styles.notice} role="status">
            Your entitlements do not include the fixed-income license — curve
            queries are shown read-only and the server will refuse them.
          </p>
        )}

        <div className={styles.pillarHead}>
          <span className={styles.fieldLabel}>Par-OIS pillars</span>
          <label className={styles.inlineInput}>
            <input
              type="checkbox"
              checked={keyRateMode}
              aria-label="enable per-pillar key-rate shift inputs"
              onChange={(e) => setKeyRateMode(e.target.checked)}
            />
            <span>Key-rate shift</span>
          </label>
        </div>

        <ul className={styles.pillarList}>
          {pillars.map((p, i) => (
            <li key={i} className={styles.pillarItem}>
              <span className={styles.pickLabel}>{pillarTenorLabel(p.tenor)}</span>
              <label className={styles.inlineInput}>
                <input
                  type="number"
                  step={0.01}
                  value={p.parRatePct}
                  aria-label={`${pillarTenorLabel(p.tenor)} par rate in percent`}
                  onChange={(e) => setPillarRate(i, Number(e.target.value))}
                />
                <span className={styles.inputUnit}>%</span>
              </label>
              {keyRateMode && (
                <label className={styles.inlineInput}>
                  <input
                    type="number"
                    step={1}
                    value={keyRateBp[i] ?? 0}
                    aria-label={`${pillarTenorLabel(p.tenor)} key-rate shift in basis points`}
                    onChange={(e) => setKeyRate(i, Number(e.target.value))}
                  />
                  <span className={styles.inputUnit}>bp</span>
                </label>
              )}
            </li>
          ))}
        </ul>

        <div className={styles.field}>
          <span className={styles.fieldLabel}>Parallel shift</span>
          <label className={styles.inlineInput}>
            <input
              type="number"
              step={1}
              value={parallelBp}
              aria-label="scenario parallel shift in basis points"
              onChange={(e) => setParallelBp(Number(e.target.value))}
            />
            <span className={styles.inputUnit}>bp</span>
          </label>
        </div>

        <fieldset className={styles.modelField}>
          <legend className={styles.fieldLabel}>Reprice a leg</legend>
          <label className={styles.inlineInput}>
            <input
              type="checkbox"
              checked={repriceLeg}
              aria-label="reprice an OIS leg on the base and shifted curves"
              onChange={(e) => setRepriceLeg(e.target.checked)}
            />
            <span>Reprice an OIS leg on the shift</span>
          </label>
          {repriceLeg && (
            <div className={styles.tenorPicks}>
              <label className={styles.inlineInput}>
                <input
                  type="number"
                  min={1}
                  step={1}
                  value={legTenorY}
                  aria-label="repriced leg tenor in years"
                  onChange={(e) =>
                    setLegTenorY(Math.max(1, Math.trunc(Number(e.target.value))))
                  }
                />
                <span className={styles.inputUnit}>y</span>
              </label>
              <label className={styles.inlineInput}>
                <input
                  type="number"
                  step={0.01}
                  value={legFixedPct}
                  aria-label="repriced leg fixed rate in percent"
                  onChange={(e) => setLegFixedPct(Number(e.target.value))}
                />
                <span className={styles.inputUnit}>%</span>
              </label>
              <label className={styles.inlineInput}>
                <input
                  type="number"
                  step={1_000_000}
                  value={legNotional}
                  aria-label="repriced leg notional"
                  onChange={(e) => setLegNotional(Number(e.target.value))}
                />
              </label>
              <select
                className={styles.pillarKind}
                value={legDirection}
                aria-label="repriced leg direction"
                onChange={(e) =>
                  setLegDirection(
                    e.target.value as OisInstrument["direction"],
                  )
                }
              >
                <option value="PAY_FIXED">Pay fixed</option>
                <option value="RECEIVE_FIXED">Receive fixed</option>
              </select>
            </div>
          )}
        </fieldset>

        <div className={styles.buildRow}>
          <Button
            variant="primary"
            onClick={() => void runQuery()}
            disabled={disabled}
            title="read the bootstrapped curve on the query axis"
          >
            {querying ? "Querying…" : "Query live"}
          </Button>
          <Button
            variant="ghost"
            onClick={() => pinnedVersion !== null && void runQuery(pinnedVersion)}
            disabled={disabled || pinnedVersion === null}
            title={
              pinnedVersion === null
                ? "mark a curve first to read a pinned version"
                : `read the pinned marked version ${pinnedVersion}`
            }
          >
            Query pinned
          </Button>
          <Button
            variant="ghost"
            onClick={() => void runMark()}
            disabled={!!curveError || marking || !canMark}
            title={
              canMark
                ? "pin this curve under a fresh server version"
                : capabilityDenialTitle("price", "fixed_income")
            }
          >
            {marking ? "Marking…" : "Mark curve"}
          </Button>
          <Button
            variant="ghost"
            onClick={() => void runScenario()}
            disabled={!!curveError || running || !canSimulate}
            title={
              canSimulate
                ? "bump-and-reprice the curve"
                : capabilityDenialTitle("simulate", "fixed_income")
            }
          >
            {running ? "Running…" : "Run scenario"}
          </Button>
        </div>

        {marked && (
          <p className={styles.notice} role="status">
            Pinned as version {String(marked.curveVersion)} ·{" "}
            {marked.parPillars.length} par pillars — a Query pinned read reproduces
            this exact curve.
          </p>
        )}
        {curveError && (
          <p className={styles.error} role="alert">
            {curveError}
          </p>
        )}
        {queryError && (
          <p className={styles.error} role="alert">
            {queryError}
          </p>
        )}
        {markError && (
          <p className={styles.error} role="alert">
            {markError}
          </p>
        )}
        {scenarioError && (
          <p className={styles.error} role="alert">
            {scenarioError}
          </p>
        )}
      </Panel>

      <Panel className={styles.results} title="Queried curve">
        {queryResult && queryResult.points.length > 0 ? (
          <>
            <div className={styles.curveRow}>
              <span className={styles.curveLabel}>Read</span>
              <span className={styles.curveName}>
                {queryResult.currency} discount
              </span>
              <span className={styles.curveMeta}>
                {queryResult.curveVersion === undefined
                  ? "live bootstrap"
                  : `marked v${String(queryResult.curveVersion)}`}{" "}
                · {queryResult.points.length} points · ref{" "}
                {queryResult.referenceDate.year}-
                {String(queryResult.referenceDate.month).padStart(2, "0")}-
                {String(queryResult.referenceDate.day).padStart(2, "0")}
              </span>
            </div>

            <div className={styles.chart}>
              <h3 className={styles.chartTitle}>
                Term structure · zero &amp; forward (%) · discount factor
              </h3>
              <YieldCurve
                nodes={queryNodes}
                interpolation="log-linear"
                height={280}
              />
            </div>

            <div className={styles.ladder}>
              <h3 className={styles.chartTitle}>Queried points</h3>
              <DataGrid
                label="queried curve points"
                columns={QUERY_POINT_COLUMNS}
                groups={queryPointGroups}
              />
            </div>
          </>
        ) : (
          <p className={styles.empty}>
            {canView
              ? "Query the curve to read its zero rates and discount factors on the tenor axis."
              : "Sign in with a fixed-income license to query curves."}
          </p>
        )}

        {scenario && scenario.points.length > 0 && (
          <>
            <div className={styles.curveRow}>
              <span className={styles.curveLabel}>Scenario</span>
              <span className={styles.curveName}>
                {parallelBp >= 0 ? "+" : ""}
                {parallelBp}bp{keyRateMode ? " + key-rate" : ""}
              </span>
              <span className={styles.curveMeta}>shifted curve</span>
            </div>
            <div className={styles.chart}>
              <h3 className={styles.chartTitle}>Shifted term structure</h3>
              <YieldCurve
                nodes={scenarioNodes}
                interpolation="log-linear"
                height={240}
              />
            </div>

            {scenario.reprice && (
              <>
                <dl className={styles.metrics}>
                  <div className={styles.metric}>
                    <dt className={styles.metricLabel}>Base PV</dt>
                    <dd className={styles.metricValue}>
                      {fmtCcy(scenario.reprice.basePv)}
                    </dd>
                  </div>
                  <div className={styles.metric}>
                    <dt className={styles.metricLabel}>Shifted PV</dt>
                    <dd className={styles.metricValue}>
                      {fmtCcy(scenario.reprice.shiftedPv)}
                    </dd>
                  </div>
                  <div className={`${styles.metric} ${styles.metricEmphatic}`}>
                    <dt className={styles.metricLabel}>Δ PV</dt>
                    <dd className={styles.metricValue}>
                      {fmtCcy(scenario.reprice.pvChange)}
                    </dd>
                  </div>
                  <div className={styles.metric}>
                    <dt className={styles.metricLabel}>DV01 (base)</dt>
                    <dd className={styles.metricValue}>
                      {fmtCcy(scenario.reprice.dv01)}
                    </dd>
                  </div>
                </dl>
                <p className={styles.scopeNote}>
                  First-order check: DV01 · parallel shift ={" "}
                  {fmtCcy(scenario.reprice.dv01 * parallelBp)} vs. actual Δ PV{" "}
                  {fmtCcy(scenario.reprice.pvChange)} (the residual is curve
                  convexity + any key-rate shift).
                </p>
                {legLadder && legLadder.length > 0 && (
                  <div className={styles.chart}>
                    <h3 className={styles.chartTitle}>
                      Repriced leg · key-rate DV01
                    </h3>
                    <KeyRateLadder
                      data={legLadder}
                      parallelDv01={scenario.reprice.dv01}
                      unit={`${curveSet.currency}/bp`}
                    />
                  </div>
                )}
              </>
            )}
          </>
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
