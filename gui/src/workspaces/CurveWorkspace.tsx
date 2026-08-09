/**
 * CurveWorkspace — the fixed-income CURVES multi-curve manager (server commit
 * 38bcff9a); the rates lens of the shared, class-parametric `MarketDataWorkspace`.
 * It is now a full manager over the server's persisted {@link CurveDefinition}
 * registry rather than one hardcoded curve:
 *
 *   • **Dashboard** (landing) — every persisted curve in a table (name + primary
 *     badge, currency, index, interpolation, pillar summary); row → edit, "New
 *     curve" → a blank definition.
 *   • a **curve picker** in the header selects the ACTIVE curve (with its per-currency
 *     primary badge); the Definition / Pillars / Query lenses operate on it.
 *   • **Definition** — create / edit the metadata + the now-WIRE-REAL interpolation
 *     scheme, persisted via create / update.
 *   • **Pillars** — the par-OIS pillar ladder of the SELECTED curve, with the live
 *     bootstrap reprice/preview, saved via `update_curve_definition`.
 *   • **By instrument reference** — the server `BuildCurve` calibration tool (a
 *     request-scoped build, unchanged).
 *   • **Query · mark · scenario** — the SurfaceService GetCurve / MarkCurve /
 *     CurveScenario query lens, seeded from the selected curve (ADR-0021).
 *
 * The curve math is REAL and SHARED: every lens samples the SAME in-browser bootstrap
 * the OIS pricer uses (`src/data/ratesPricing.ts`), interpolated log-linear-on-log-DF
 * (the server's shipping default). The definition's chosen interpolation rides the
 * wire and the SERVER bootstraps with it; the in-browser preview is honestly labelled
 * as the log-linear approximation of the pillar shape.
 *
 * Gating: list / view rides `view · fixed_income`; create / update / delete / save
 * ride `refdata · fixed_income` (the server requires Refdata·FixedIncome). A read-only
 * viewer sees the dashboard and every lens but cannot mutate — write controls are
 * hidden or disabled, never faked.
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
  CurveDefinition,
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
  curveInterpolationLabel,
  INSTRUMENT_FAMILY_LABELS,
  oisRatesInstrument,
  pillarTenorLabel,
} from "../data/contract";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import { useApp } from "../app/AppContext";
import { useReferenceData } from "../hooks/useReferenceData";
import { useCurveDefinitions } from "../hooks/useCurveDefinitions";
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
import { CurveDashboard } from "./CurveDashboard";
import { CurveDefinitionEditor } from "./CurveDefinitionEditor";
import styles from "./CurveWorkspace.module.css";

/** The manager lenses, in the order the tab bar offers them. */
type ManagerMode =
  | "dashboard"
  | "definition"
  | "pillars"
  | "instruments"
  | "query";

/** Number of points sampled across the span for the term-structure plots. */
const SAMPLE_COUNT = 96;

/** The default reference (spot-anchor) date for the instrument-reference builder. */
const DEFAULT_REFERENCE_DATE: BrokenDate = DEFAULT_USD_SOFR_CURVE.referenceDate;

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
 * Order a pillar set by true maturity against a reference date — the single
 * year-fraction coordinate `pillarMaturityYears` yields for every arm (years /
 * months / broken date), so a `1M` pillar sorts above `1Y`, and a broken date sorts
 * into its real slot. The bootstrap requires strictly-increasing maturities; holding
 * this as an invariant after every edit keeps the ladder valid.
 */
function sortByMaturity(
  list: readonly EditablePillar[],
  refDate: BrokenDate,
): readonly EditablePillar[] {
  return [...list].sort(
    (a, b) =>
      pillarMaturityYears(a.tenor, refDate) -
      pillarMaturityYears(b.tenor, refDate),
  );
}

/** Lift a curve set's pillar ladder into the editor's (maturity-ordered) percent form. */
function editablePillarsFromSet(set: RatesCurveSet): readonly EditablePillar[] {
  return sortByMaturity(
    set.pillars.map((p) => ({
      id: nextPillarId(),
      tenor: p.tenor,
      parRatePct: p.parRate * 100,
    })),
    set.referenceDate,
  );
}

/** Assemble the wire curve set the bootstrap consumes from the editor's pillars. */
function curveSetFromEditable(
  pillars: readonly EditablePillar[],
  refDate: BrokenDate,
  currency: string,
): RatesCurveSet {
  return {
    currency,
    referenceDate: refDate,
    pillars: pillars.map((p) => ({
      tenor: p.tenor,
      parRate: p.parRatePct / 100,
    })),
  };
}

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
function representativeYears(tenor: PillarTenor, refDate: BrokenDate): number {
  return Math.max(1, Math.round(pillarMaturityYears(tenor, refDate)));
}

/** Convert a tenor to a different arm, preserving an approximate maturity. */
function switchKind(
  tenor: PillarTenor,
  kind: PillarTenor["kind"],
  refDate: BrokenDate,
): PillarTenor {
  if (tenor.kind === kind) return tenor;
  const years = representativeYears(tenor, refDate);
  switch (kind) {
    case "years":
      return { kind: "years", years };
    case "months":
      return { kind: "months", months: years * 12 };
    case "date":
      return {
        kind: "date",
        maturityDate: {
          year: refDate.year + years,
          month: refDate.month,
          day: refDate.day,
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
 * The Curves multi-curve manager. Owns the definition registry (via
 * {@link useCurveDefinitions}), the selected curve, and the active lens; each lens is
 * a focused child operating on the selected {@link CurveDefinition}.
 */
export function CurveWorkspace(): React.ReactElement {
  const app = useApp();
  const isAuthed = app.auth.user !== null;
  const curves = useCurveDefinitions(app.transport, isAuthed);
  const canEdit = app.auth.can("refdata", "fixed_income");

  const [mode, setMode] = useState<ManagerMode>("dashboard");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // The definition being edited: null ⇒ create a new curve; a string ⇒ edit that id.
  const [editingId, setEditingId] = useState<string | null>(null);
  const [busyCurveId, setBusyCurveId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  // The active curve is derived (no effect): the explicit selection if it still
  // exists, else the currency primary, else the first — so a fresh load, a delete, or
  // a create always resolves to a live curve.
  const selected = useMemo<CurveDefinition | null>(() => {
    const list = curves.definitions;
    return (
      list.find((d) => d.curveId === selectedId) ??
      list.find((d) => d.primary) ??
      list[0] ??
      null
    );
  }, [curves.definitions, selectedId]);

  const openDashboard = useCallback(() => setMode("dashboard"), []);

  const openEdit = useCallback((curveId: string) => {
    setSelectedId(curveId);
    setEditingId(curveId);
    setMode("definition");
  }, []);

  const openNew = useCallback(() => {
    setEditingId(null);
    setMode("definition");
  }, []);

  const openPillars = useCallback((curveId: string) => {
    setSelectedId(curveId);
    setMode("pillars");
  }, []);

  const onSaved = useCallback((curveId: string) => {
    setSelectedId(curveId);
    setActionError(null);
    setMode("dashboard");
  }, []);

  const handleDelete = useCallback(
    async (curveId: string): Promise<void> => {
      setBusyCurveId(curveId);
      setActionError(null);
      try {
        await curves.deleteCurve(curveId);
      } catch (e: unknown) {
        setActionError(e instanceof Error ? e.message : "delete failed");
      } finally {
        setBusyCurveId(null);
      }
    },
    [curves],
  );

  // The definition open in the editor (null ⇒ create).
  const editing = useMemo<CurveDefinition | null>(
    () =>
      editingId === null
        ? null
        : (curves.definitions.find((d) => d.curveId === editingId) ?? null),
    [editingId, curves.definitions],
  );

  const TABS: readonly { id: ManagerMode; label: string }[] = [
    { id: "dashboard", label: "Dashboard" },
    { id: "definition", label: "Definition" },
    { id: "pillars", label: "Pillars" },
    { id: "instruments", label: "By instrument reference" },
    { id: "query", label: "Query · mark · scenario" },
  ];

  const onTab = (id: ManagerMode): void => {
    if (id === "definition") {
      // The Definition tab edits the SELECTED curve (or a new one if none exists).
      setEditingId(selected?.curveId ?? null);
    }
    setActionError(null);
    setMode(id);
  };

  const showPicker = mode === "pillars" || mode === "query";

  return (
    <div className={styles.page} data-testid="curves-manager">
      <div className={styles.managerBar}>
        <div
          className={styles.modeTabs}
          role="tablist"
          aria-label="curves manager lens"
        >
          {TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={mode === t.id}
              className={`${styles.modeTab} ${mode === t.id ? styles.modeTabActive : ""}`}
              onClick={() => onTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </div>

        {showPicker && (
          <div className={styles.curvePickRow}>
            <label className={styles.pickerLabel} htmlFor="curve-picker">
              Curve
            </label>
            <select
              id="curve-picker"
              className={styles.pickerSelect}
              value={selected?.curveId ?? ""}
              disabled={curves.definitions.length === 0}
              aria-label="active curve"
              onChange={(e) => setSelectedId(e.target.value)}
            >
              {curves.definitions.length === 0 && (
                <option value="">No curves defined</option>
              )}
              {curves.definitions.map((d) => (
                <option key={d.curveId} value={d.curveId}>
                  {d.displayName}
                  {d.primary ? " (primary)" : ""}
                </option>
              ))}
            </select>
            {selected?.primary && (
              <span className={styles.pickerBadge}>Primary</span>
            )}
          </div>
        )}
      </div>

      {mode === "dashboard" && (
        <CurveDashboard
          definitions={curves.definitions}
          isLoading={curves.isLoading}
          error={curves.error}
          actionError={actionError}
          canEdit={canEdit}
          busyCurveId={busyCurveId}
          onEdit={openEdit}
          onEditPillars={openPillars}
          onNew={openNew}
          onDelete={(id) => void handleDelete(id)}
        />
      )}

      {mode === "definition" && (
        <CurveDefinitionEditor
          key={editing?.curveId ?? "new"}
          existing={editing}
          canEdit={canEdit}
          onCreate={curves.createCurve}
          onUpdate={curves.updateCurve}
          onSaved={onSaved}
          onCancel={openDashboard}
        />
      )}

      {mode === "pillars" &&
        (selected ? (
          <PillarEditorMode
            key={selected.curveId}
            definition={selected}
            canEdit={canEdit}
            onSave={async (updated) => {
              await curves.updateCurve(selected.curveId, {
                ...selected,
                pillars: updated,
              });
            }}
          />
        ) : (
          <NoCurveEmpty onNew={openNew} canEdit={canEdit} />
        ))}

      {mode === "instruments" && <InstrumentReferenceMode />}

      {mode === "query" &&
        (selected ? (
          <CurveQueryMode key={selected.curveId} definition={selected} />
        ) : (
          <NoCurveEmpty onNew={openNew} canEdit={canEdit} />
        ))}
    </div>
  );
}

/** The honest empty-state for the per-curve lenses when no curve is defined yet. */
function NoCurveEmpty({
  onNew,
  canEdit,
}: {
  onNew: () => void;
  canEdit: boolean;
}): React.ReactElement {
  return (
    <div className={styles.wrap}>
      <Panel className={styles.builder} title="No curve selected">
        <p className={styles.empty}>
          No curves are defined yet.
          {canEdit
            ? " Create one to edit its pillars and query it."
            : " Ask a reference-data steward to define one."}
        </p>
        {canEdit && (
          <div className={styles.buildRow}>
            <Button variant="primary" onClick={onNew}>
              + New curve
            </Button>
          </div>
        )}
      </Panel>
    </div>
  );
}

/**
 * The per-curve PILLAR editor: edit the SELECTED curve's par-OIS pillars across the
 * years / months / broken-date arms, with the live in-browser bootstrap reprice, and
 * Save the ladder back to the definition (`update_curve_definition`). The curve math
 * is REAL — a malformed edit surfaces the actual `RatesPricingError`, never a
 * fabricated curve. The in-browser preview is log-linear-on-log-DF; the persisted
 * curve is bootstrapped server-side with the definition's own interpolation.
 */
function PillarEditorMode({
  definition,
  canEdit,
  onSave,
}: {
  definition: CurveDefinition;
  canEdit: boolean;
  onSave: (updated: RatesCurveSet) => Promise<void>;
}): React.ReactElement {
  const refDate = definition.pillars.referenceDate;
  const currency = definition.pillars.currency;
  const initialPillars = useMemo(
    () => editablePillarsFromSet(definition.pillars),
    [definition.pillars],
  );

  const [pillars, setPillars] =
    useState<readonly EditablePillar[]>(initialPillars);
  const [horizonYears, setHorizonYears] = useState(5);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const pillarTimes = useMemo(
    () => pillars.map((p) => pillarMaturityYears(p.tenor, refDate)),
    [pillars, refDate],
  );
  const span = pillarTimes.length ? pillarTimes[pillarTimes.length - 1]! : 0;
  const horizon = Math.min(Math.max(horizonYears, 0), span);

  const curveSet = useMemo<RatesCurveSet>(
    () => curveSetFromEditable(pillars, refDate, currency),
    [pillars, refDate, currency],
  );

  // Bootstrap once + sample the span; a malformed edit throws a real
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

  const dfTrace = useMemo(() => samples.map((s) => s.df), [samples]);
  const zeroTrace = useMemo(() => samples.map((s) => s.zero), [samples]);
  const forwardTrace = useMemo(() => samples.map((s) => s.forward), [samples]);

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

  const markDirty = useCallback(() => setSaved(false), []);

  const setPillarRate = useCallback(
    (index: number, pct: number) => {
      setPillars((prev) =>
        prev.map((p, j) => (j === index ? { ...p, parRatePct: pct } : p)),
      );
      markDirty();
    },
    [markDirty],
  );

  const setPillarTenor = useCallback(
    (index: number, tenor: PillarTenor) => {
      setPillars((prev) =>
        sortByMaturity(
          prev.map((p, j) => (j === index ? { ...p, tenor } : p)),
          refDate,
        ),
      );
      markDirty();
    },
    [refDate, markDirty],
  );

  const removePillar = useCallback(
    (index: number) => {
      setPillars((prev) => prev.filter((_, j) => j !== index));
      markDirty();
    },
    [markDirty],
  );

  const addPillar = useCallback(() => {
    setPillars((prev) => {
      const last = prev[prev.length - 1];
      const nextYears = last ? representativeYears(last.tenor, refDate) + 1 : 1;
      const parRatePct = last ? last.parRatePct : 4;
      return sortByMaturity(
        [
          ...prev,
          {
            id: nextPillarId(),
            tenor: { kind: "years", years: nextYears },
            parRatePct,
          },
        ],
        refDate,
      );
    });
    markDirty();
  }, [refDate, markDirty]);

  const resetPillars = useCallback(() => {
    setPillars(initialPillars);
    setSaved(false);
    setSaveError(null);
  }, [initialPillars]);

  const isDirty = useMemo(
    () =>
      pillars.length !== initialPillars.length ||
      pillars.some(
        (p, i) =>
          p.parRatePct !== initialPillars[i]?.parRatePct ||
          pillarTenorLabel(p.tenor) !==
            pillarTenorLabel(initialPillars[i]!.tenor),
      ),
    [pillars, initialPillars],
  );

  const save = useCallback(async (): Promise<void> => {
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      await onSave(curveSet);
      setSaved(true);
    } catch (e: unknown) {
      setSaveError(e instanceof Error ? e.message : "curve save failed");
    } finally {
      setSaving(false);
    }
  }, [onSave, curveSet]);

  const isMonotone = definition.interpolation === "monotone-convex-forward";

  return (
    <div className={styles.wrap}>
      <Panel material="float" className={styles.builder} title="Curve set">
        <div className={styles.curveRow}>
          <span className={styles.curveLabel}>Curve</span>
          <span className={styles.curveName}>{definition.displayName}</span>
          <span className={styles.curveMeta}>
            {pillars.length} pillars · {currency} · {definition.indexLabel} · ref{" "}
            {refDate.year}-{String(refDate.month).padStart(2, "0")}-
            {String(refDate.day).padStart(2, "0")}
          </span>
          {definition.primary && (
            <span className={styles.pickerBadge}>Primary</span>
          )}
        </div>

        <p className={styles.hint}>
          Interpolation: <strong>{curveInterpolationLabel(definition.interpolation)}</strong>
          {isMonotone
            ? " — the persisted curve bootstraps monotone-convex server-side; the preview below is the log-linear-DF approximation of the pillar shape."
            : " — the shipping default; the preview below is the same log-linear-DF bootstrap."}{" "}
          Change it in the Definition tab.
        </p>

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
                title="restore the saved pillar set"
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
                    switchKind(
                      p.tenor,
                      e.target.value as PillarTenor["kind"],
                      refDate,
                    ),
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
        {saveError && (
          <p className={styles.error} role="alert">
            {saveError}
          </p>
        )}

        <div className={styles.buildRow}>
          {saved && (
            <span className={styles.savedNote} role="status">
              Saved
            </span>
          )}
          <Button
            variant="primary"
            onClick={() => void save()}
            disabled={!canEdit || saving || !!error}
            title={
              canEdit
                ? "persist these pillars to the curve (update_curve_definition)"
                : capabilityDenialTitle("refdata", "fixed_income")
            }
          >
            {saving ? "Saving…" : "Save pillars"}
          </Button>
        </div>

        <p className={styles.scopeNote}>
          Saving updates the persisted curve. To read it back under a server version,
          or bump-and-reprice it, use the Query · mark · scenario lens (SurfaceService
          MarkCurve / GetCurve).
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
  const [refInput, setRefInput] = useState(brokenToInput(DEFAULT_REFERENCE_DATE));
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
        dateInput: brokenToInput(DEFAULT_REFERENCE_DATE),
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
// GetCurve / MarkCurve / CurveScenario, ADR-0021), seeded from the SELECTED curve.
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
 * The Query · mark · scenario lens, seeded from the SELECTED curve. A trader edits the
 * curve's par-OIS pillars, then drives the three FI market-data query verbs against
 * it through the one contract: GetCurve (live or pinned), MarkCurve (pin under a
 * version), CurveScenario (parallel + optional key-rate bump-and-reprice with an
 * optional repriced OIS leg). Gated on `fixed_income` (disabled + tooltip, never
 * hidden; the server still enforces).
 */
function CurveQueryMode({
  definition,
}: {
  definition: CurveDefinition;
}): React.ReactElement {
  const app = useApp();

  const canView = app.auth.can("view", "fixed_income");
  const canMark = app.auth.can("price", "fixed_income");
  const canSimulate = app.auth.can("simulate", "fixed_income");

  const refDate = definition.pillars.referenceDate;
  const currency = definition.pillars.currency;

  const [pillars, setPillars] = useState<readonly EditablePillar[]>(() =>
    editablePillarsFromSet(definition.pillars),
  );

  const curveSet = useMemo<RatesCurveSet>(
    () => curveSetFromEditable(pillars, refDate, currency),
    [pillars, refDate, currency],
  );

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
          <span className={styles.curveName}>{definition.displayName}</span>
          <span className={styles.curveMeta}>
            {pillars.length} pillars · {currency} · ref {refDate.year}-
            {String(refDate.month).padStart(2, "0")}-
            {String(refDate.day).padStart(2, "0")}
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
            <li key={p.id} className={styles.pillarItem}>
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
                      unit={`${currency}/bp`}
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
