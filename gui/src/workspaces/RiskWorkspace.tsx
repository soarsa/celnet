/**
 * RiskWorkspace — the ONE class-parametric RISK workspace (`fe-fi-migration`),
 * under HARD VERTICAL ASSET SEPARATION (W1).
 *
 * There is no FX-vs-FI risk split and no in-screen cross-asset lens toggle any
 * more: the workspace renders EXACTLY ONE risk lens, DERIVED STRICTLY from the
 * active domain — FX/options → the spot×vol scenario grid (`FxScenarioLens`
 * below); fixed-income → the rates netted-risk panel (`RatesRiskPanel`, folded in
 * from the former standalone `RatesRisk` silo). Under Fixed Income the screen
 * shows ONLY FI content; under FX Options ONLY FX content. The derived lens is
 * gated by the entitlement×license the rail uses, so a rates desk risks its book
 * under a fixed-income license and an FX desk never sees a lens it cannot use;
 * when the derived lens's class is not viewable, the honest empty-state is shown.
 *
 * The firm-wide joint options+FI tail cube is NOT a per-domain risk view — it is
 * a separate cross-asset surface (Book/Combined) and no longer appears here. The
 * pure joint-tail request helpers (`seedTailRiskOptionLegs`,
 * `seedTailRiskFiPositions`, `buildJointScenarios`, `buildCombinedTailRequest`)
 * are retained as the shared contract-shaping seam consumed by that surface and
 * by tests.
 *
 * FX scenario lens (`FxScenarioLens`, GUI-DESIGN §4.4): a spot×vol shock grid
 * driven by SurfaceService.Scenario (ShockAxis abs/rel) for the selected
 * structure. Each cell is a P&L under the shock, tinted on the perceptual
 * diverging ramp (reads magnitude honestly, no rainbow); the spot/vol "today"
 * cell is anchored. Vega/gamma ladders are a disclosure, not a separate screen.
 * Pin scenarios for compare. One click from the ticket/blotter (same Instrument),
 * never re-keyed.
 */

import { useEffect, useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import type {
  CapabilityAsset,
  CcyPair,
  CombinedTailRiskRequest,
  Conventions,
  Instrument,
  JointTailScenario,
  MarketContext,
  RiskBucketRequest,
  ScenarioResult,
  ShockAxis,
  ShockFactor,
  TailRiskCurvePillar,
  TailRiskFiPosition,
  TailRiskOptionLeg,
} from "../data/contract";
import { Panel } from "../components/Panel";
import { Button } from "../components/Button";
import { strategyInstrument } from "../data/seed";
import { Rng } from "../data/rng";
import { defaultRatesRiskRows } from "./RatesRiskWorkspace";
import { ScenarioHeatmap } from "../viz/ScenarioHeatmap";
import { fmtPnlAdaptive, fmtSigned, fmtCompact } from "../lib/format";
import { tenorLabel } from "../lib/trend";
import { configuredLicense, type LicensePredicate } from "../lib/commands";
import { RatesRiskPanel } from "./RatesRiskWorkspace";
import styles from "./RiskWorkspace.module.css";

/**
 * The scenario-axis catalogue (P0-7). The contract's `ShockAxis.factor` already
 * lets a scenario sweep any of these factors on either grid axis; this is the
 * GUI surface for choosing them (the contract is unchanged — purely a request
 * shape). Each factor carries a sensible 5-step preset and its abs/rel mode and a
 * formatter so the headers read honestly for that factor's units.
 */
interface AxisSpec {
  factor: ShockFactor;
  label: string;
  relative: boolean;
  steps: number[];
  /** Render one step's column/row header for this factor. */
  fmtStep: (step: number) => string;
}

const PCT = (s: number): string => (s === 0 ? "0.0%" : `${s > 0 ? "+" : "−"}${Math.abs(s * 100).toFixed(1)}%`);
const VOLPT = (s: number): string => (s === 0 ? "ATM" : `${s > 0 ? "+" : "−"}${Math.abs(s * 100).toFixed(0)}%`);
const BPS = (s: number): string => (s === 0 ? "0bp" : `${s > 0 ? "+" : "−"}${Math.abs(s * 1e4).toFixed(0)}bp`);
const DAYS = (s: number): string => (s === 0 ? "0d" : `+${Math.round(s * 365)}d`);

const AXIS_SPECS: Record<ShockFactor, AxisSpec> = {
  SPOT: { factor: "SPOT", label: "Spot", relative: true, steps: [-0.015, -0.005, 0, 0.005, 0.015], fmtStep: PCT },
  VOL: { factor: "VOL", label: "Vol", relative: false, steps: [-0.02, -0.01, 0, 0.01, 0.02], fmtStep: VOLPT },
  RATE_DOM: { factor: "RATE_DOM", label: "Rate dom", relative: false, steps: [-0.005, -0.0025, 0, 0.0025, 0.005], fmtStep: BPS },
  RATE_FOR: { factor: "RATE_FOR", label: "Rate for", relative: false, steps: [-0.005, -0.0025, 0, 0.0025, 0.005], fmtStep: BPS },
  TIME: { factor: "TIME", label: "Time", relative: false, steps: [0, 1 / 365, 7 / 365, 14 / 365, 30 / 365], fmtStep: DAYS },
};

const FACTOR_ORDER: ShockFactor[] = ["SPOT", "VOL", "RATE_DOM", "RATE_FOR", "TIME"];

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

/** The asset-class lens the shared Risk workspace renders under. Hard vertical
 * separation: exactly one lens per domain — FX/options or fixed-income. There is
 * no cross-asset "combined" lens here (the firm-wide joint tail is a separate
 * Book/Combined surface, not a per-domain risk view). */
export type RiskLens = "fx" | "rates";

export function RiskWorkspace({
  initialLens,
}: {
  initialLens?: RiskLens;
} = {}): React.ReactElement {
  const app = useApp();
  const licensed: LicensePredicate = useMemo(() => configuredLicense(), []);

  // Hard vertical asset separation (W1): the lens is DERIVED STRICTLY from the
  // active domain — fixed_income → the rates netted-risk panel, otherwise → the FX
  // spot×vol scenario grid. There is NO in-screen cross-asset toggle; each domain
  // renders only its own asset. An explicit `initialLens` (stories/tests) may
  // override the domain-derived lens for a fixed render, but never reintroduces a
  // user-facing switch.
  const lens: RiskLens = initialLens ?? (app.activeDomain === "fixed_income" ? "rates" : "fx");
  const asset: CapabilityAsset = lens === "rates" ? "fixed_income" : "fx_options";

  // Honest gate for the DERIVED lens only: render it iff the identity can VIEW its
  // asset class AND the firm is licensed for it (entitlement-first, license-second
  // — the rail's gate). Otherwise show the honest empty-state, never a faked view.
  if (!(app.auth.can("view", asset) && licensed(asset))) {
    return <div className={styles.loading}>No risk lens available for your entitlements.</div>;
  }

  return (
    <div className={styles.classShell}>
      <p className={styles.shellSub}>
        {lens === "rates" ? (
          <>
            Netted <strong>scenario risk</strong> for your rates book — what-if P&amp;L and greeks
            under shocks. Not the routed-risk <em>Risk Dashboard</em>, the <em>Risk Portfolios</em>{" "}
            tree, nor the position-ledger <em>Book</em>.
          </>
        ) : (
          <>
            Spot×vol <strong>scenario</strong> P&amp;L + greeks for the selected structure — an
            analytics what-if grid, not the ledger <em>Book</em> of positions.
          </>
        )}
      </p>
      <div className={styles.lensBody}>
        {lens === "fx" ? <FxScenarioLens /> : <RatesRiskPanel />}
      </div>
    </div>
  );
}

/**
 * The FX/options risk lens — the spot×vol scenario what-if grid + vega ladder
 * (unchanged; see the file header). Mounted only while the FX lens is active, so
 * its Scenario RPC fires only when this lens is shown.
 */
function FxScenarioLens(): React.ReactElement {
  const app = useApp();
  const [metric, setMetric] = useState<Metric>("pnl");
  const [result, setResult] = useState<ScenarioResult | null>(null);
  const [pinned, setPinned] = useState<{ label: string; result: ScenarioResult }[]>([]);
  // Scenario-axis selection (P0-7): which factor sweeps on each grid axis. The
  // contract already supports any ShockAxis.factor; this is GUI-only. Default
  // SPOT (rows) × VOL (cols). The two axes must be distinct factors.
  const [rowFactor, setRowFactor] = useState<ShockFactor>("SPOT");
  const [colFactor, setColFactor] = useState<ShockFactor>("VOL");

  // The structure under analysis (P0-5): the shared selection driven by the
  // Book/Ticket lanes. Falls back to the seeded 25Δ RR on the active pair so the
  // first load (nothing selected yet) still renders a real structure.
  const selected = app.selected;
  const fallbackInstrument: Instrument = useMemo(
    () => strategyInstrument(app.pairCtx.pair, 30 / 365, "RISK_REVERSAL", 10),
    [app.pairCtx.pair],
  );
  const instrument: Instrument = selected?.instrument ?? fallbackInstrument;
  const subjectLabel =
    selected?.label ??
    `${app.pairCtx.pair.base}/${app.pairCtx.pair.quote} 25Δ RR`;

  // The instrument is priced against its OWN pair's base market (a drilled
  // position may be a different pair than the active watch pair), falling back to
  // the active pair's market when the instrument's pair has no marked context.
  const market = useMemo(() => {
    const key = `${instrument.pair.base}/${instrument.pair.quote}`;
    const ctx = app.pairs.find((p) => `${p.pair.base}/${p.pair.quote}` === key);
    return ctx?.market ?? app.pairCtx.market;
  }, [instrument.pair, app.pairs, app.pairCtx.market]);

  const rowSpec = AXIS_SPECS[rowFactor];
  const colSpec = AXIS_SPECS[colFactor];

  const axes: ShockAxis[] = useMemo(
    () => [
      { factor: rowSpec.factor, relative: rowSpec.relative, steps: rowSpec.steps },
      { factor: colSpec.factor, relative: colSpec.relative, steps: colSpec.steps },
    ],
    [rowSpec, colSpec],
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
      .scenario(instrument, market, app.conventions, axes, riskBuckets)
      .then((r) => {
        if (live) setResult(r);
      });
    return () => {
      live = false;
    };
  }, [app.transport, instrument, market, app.conventions, axes, riskBuckets]);

  if (!result) return <div className={styles.loading}>Repricing scenario…</div>;

  const notional = instrument.quantity.notional;
  const basePrice = result.points.find((p) => p.appliedShocks.every((s) => s === 0))?.greeks.price ?? 0;

  // Build the row×col matrix. axes order = [row, col] ⇒ appliedShocks[0] is the
  // row-factor shock, appliedShocks[1] the col-factor shock (P0-7: either axis is
  // user-chosen, the lookup is factor-agnostic).
  const rowSteps = rowSpec.steps;
  const colSteps = colSpec.steps;
  const cellValue = (rowStep: number, colStep: number): number => {
    const pt = result.points.find(
      (p) => Math.abs((p.appliedShocks[0] ?? 0) - rowStep) < 1e-9 && Math.abs((p.appliedShocks[1] ?? 0) - colStep) < 1e-9,
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

  // Map the swept grid onto the ScenarioHeatmap's props (mockup 06): the column
  // factor is the x-axis (spot-shock columns), the row factor the y-axis (vol-shock
  // rows), and pnl[volIndex][spotIndex] the selected metric at that joint shock.
  // Row 0 renders at the bottom, matching the component's category convention.
  const spotLabels = colSteps.map((v) => colSpec.fmtStep(v));
  const volLabels = rowSteps.map((s) => rowSpec.fmtStep(s));
  const heatmapPnl = rowSteps.map((s) => colSteps.map((v) => cellValue(s, v)));
  const heatmapUnit = metric === "pnl" ? instrument.pair.quote : "";

  const fmtCell = (v: number): string => {
    if (metric === "pnl") return fmtPnlAdaptive(v);
    if (metric === "delta") return fmtSigned(v, 3);
    return fmtSigned(v, 4);
  };

  return (
    <div className={styles.grid}>
      <Panel
        glyph="⊞"
        title={
          <span className={styles.subject}>
            {/* P0-5 back affordance: a selection means we drilled in from Book —
                one click returns to the desk-wide cube. Hidden on the seeded
                default (nothing was drilled). */}
            {selected && (
              <button
                className={styles.back}
                onClick={() => {
                  app.setSelected(null);
                  app.setWorkspace("book");
                }}
                title="Back to Book (desk-wide risk)"
              >
                ‹ Book
              </button>
            )}
            <span>Risk · {subjectLabel}</span>
            <span className={styles.subjectMeta}>
              {tenorLabel(instrument.expiryYears)} · {fmtCompact(notional)}
            </span>
          </span>
        }
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
        {/* P0-7 scenario-axis selector — choose which factors the grid sweeps. The
            contract already honours any ShockAxis.factor; this is GUI-only. The two
            axes must be distinct, so picking a factor already on the other axis
            swaps them. */}
        <div className={styles.axisBar}>
          <span className={styles.axisCue}>rows ↓</span>
          <AxisPicker
            value={rowFactor}
            other={colFactor}
            onPick={(f) => {
              if (f === colFactor) setColFactor(rowFactor);
              setRowFactor(f);
            }}
          />
          <span className={styles.axisCueX}>× cols →</span>
          <AxisPicker
            value={colFactor}
            other={rowFactor}
            onPick={(f) => {
              if (f === rowFactor) setRowFactor(colFactor);
              setColFactor(f);
            }}
          />
        </div>

        <div className={styles.gridArea}>
          <ScenarioHeatmap
            pnl={heatmapPnl}
            spotLabels={spotLabels}
            volLabels={volLabels}
            unit={heatmapUnit}
            formatValue={fmtCell}
            ariaLabel={`${rowSpec.label} by ${colSpec.label} ${metric} scenario heatmap, diverging colour centred at zero`}
          />
          <div className={styles.provLine}>
            sweeping <strong>{rowSpec.label}</strong> × <strong>{colSpec.label}</strong> ·
            real reprice via SurfaceService.Scenario at this structure&apos;s pair market
          </div>
        </div>

        <div className={styles.gridFoot}>
          <Button
            variant="secondary"
            onClick={() =>
              setPinned((p) => [
                ...p.slice(-2),
                { label: pinLabel(metric, instrument.pair.base), result },
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
              <span className="num">{tenorLabel(b.tenorYears)}</span>
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
              {tenorLabel(br.rollHorizonsYears[i] ?? 0)}{" "}
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

/**
 * The scenario-axis factor picker (P0-7): a compact segmented control over the
 * five contract `ShockFactor`s. The factor already on the OTHER axis is marked so
 * the user sees it will swap (the two axes must be distinct). Accent (indigo)
 * marks the active factor — selection, never coral.
 */
function AxisPicker({
  value,
  other,
  onPick,
}: {
  value: ShockFactor;
  other: ShockFactor;
  onPick: (f: ShockFactor) => void;
}): React.ReactElement {
  return (
    <div className={styles.axisPick}>
      {FACTOR_ORDER.map((f) => {
        const active = f === value;
        const onOther = f === other;
        return (
          <button
            key={f}
            className={`${styles.axisOpt} ${active ? styles.axisOptActive : ""} ${onOther ? styles.axisOptOther : ""}`}
            onClick={() => onPick(f)}
            title={onOther ? `${AXIS_SPECS[f].label} — picking swaps the axes` : AXIS_SPECS[f].label}
          >
            {AXIS_SPECS[f].label}
          </button>
        );
      })}
    </div>
  );
}

function pillarName(delta: number): string {
  if (Math.abs(delta) >= 0.49) return "ATM";
  return `${delta < 0 ? "−" : "+"}${Math.round(Math.abs(delta) * 100)}Δ`;
}

// ---------------------------------------------------------------------------
// combined options+FI JOINT tail request seam (RiskService.CombinedTailRisk)
// ---------------------------------------------------------------------------
//
// The C2c "true single VaR engine": ONE non-additive tail cube over the book's
// vanilla FX option legs AND its linear-FI (OIS-swap) legs, by full joint
// bump-and-revalue over aligned (options-shock, rate-shock) scenarios — plus the
// FI signed key-rate DV01 ladder and signed parallel DV01. The whole portfolio +
// scenario config travels inline, so BOTH transports satisfy it identically (the
// offline source reprices + reduces in-browser; the live edge issues the RPC).
//
// These are PURE request-shaping helpers, retained as the shared contract seam
// for the firm-wide joint-tail surface (Book/Combined) and consumed by tests.
// They are deliberately NOT a per-domain risk lens — the Risk workspace shows
// exactly one asset's risk (hard vertical separation).

/** One million — FI/option notionals are denominated in millions. */
const CT_MM = 1_000_000;
/** The 3M expiry the seeded option legs are struck at (vol-time, years). */
const CT_OPTION_T = 0.25;
/**
 * The synthetic joint-scenario draw count. 256 correlated draws so the α = 0.99
 * tail (`⌊(1−α)·n⌋ = 2`) is a real ES-over-worst-two, not a single-scenario VaR.
 */
const CT_SCENARIO_COUNT = 256;
/** A fixed RNG seed ⇒ the synthetic scenario set is deterministic across renders. */
const CT_SCENARIO_SEED = 0x9e3779b97f4a7c15n;
/** The default VaR/ES confidence level for the joint tail. */
const CT_ALPHA = 0.99;

/**
 * Seed a small vanilla option book from a pair's live market: a long ATM call and
 * a short 3%-OTM put (a covered-write shape that carries real two-sided tail P&L).
 * GENUINE market inputs — the transport computes every number; nothing is baked.
 */
export function seedTailRiskOptionLegs(
  pair: CcyPair,
  m: MarketContext,
  conv: Conventions,
): TailRiskOptionLeg[] {
  const base = {
    pair,
    spot: m.spot,
    vol: m.vol,
    rDom: m.rDom,
    rFor: m.rFor,
    t: CT_OPTION_T,
    quotedDelta: conv.deltaConvention,
    premiumStyle: conv.premiumStyle,
  };
  return [
    { ...base, optionType: "CALL", notionalBase: 10 * CT_MM, strike: m.spot },
    { ...base, optionType: "PUT", notionalBase: -10 * CT_MM, strike: m.spot * 0.97 },
  ];
}

/**
 * Seed the linear-FI book as OIS swaps from the shared rates-risk seed rows (the
 * SAME editable book the Fixed Income lens rolls up), each an annual fixed leg to
 * its whole-year tenor on the USD-SOFR curve. Genuine inputs, direction-signed.
 */
export function seedTailRiskFiPositions(): TailRiskFiPosition[] {
  return defaultRatesRiskRows().map((row) => ({
    oisSwap: {
      start: 0,
      periods: Array.from({ length: row.tenorYears }, (_, i) => ({
        pay: i + 1,
        accrual: 1,
      })),
      fixedRate: row.fixedRatePct / 100,
      notional: row.notionalMm * CT_MM,
      receiveFixed: row.direction === "RECEIVE_FIXED",
    },
  }));
}

/**
 * A deterministic synthetic joint-scenario set (a Monte-Carlo-style historical
 * proxy, fixed-seed ⇒ reproducible): correlated (spot, vol, rate) draws where a
 * spot sell-off co-moves with a vol spike and a flight-to-quality rate rally, plus
 * a curve-slope component. Each scenario's `rateShifts` is aligned to the base
 * curve's `pillarCount` pillars, and its `discountAbs` moves the options' numeraire
 * rate with the FI parallel move — ONE market state per scenario index.
 */
export function buildJointScenarios(
  pillarCount: number,
  count: number = CT_SCENARIO_COUNT,
): JointTailScenario[] {
  const rng = new Rng(CT_SCENARIO_SEED);
  const n = Math.max(1, pillarCount);
  const out: JointTailScenario[] = [];
  for (let k = 0; k < count; k += 1) {
    const z1 = rng.normal();
    const z2 = rng.normal();
    const z3 = rng.normal();
    const z4 = rng.normal();
    const spotRel = 0.01 * z1;
    // Vol up when spot down (leverage effect); rates rally with risk-off (mild).
    const volAbs = 0.01 * (-0.6 * z1 + 0.8 * z2);
    const rateParallel = 0.0012 * (0.5 * z1 + 0.866 * z3);
    const rateSlope = 0.0008 * z4;
    const rateShifts = Array.from({ length: n }, (_, i) => {
      const frac = n > 1 ? i / (n - 1) : 0.5;
      return rateParallel + rateSlope * (frac - 0.5) * 2;
    });
    out.push({
      spotRel,
      volAbs,
      discountAbs: rateParallel,
      carryAbs: 0.0005 * z2,
      rateShifts,
    });
  }
  return out;
}

/** Assemble the `CombinedTailRiskRequest` from the seed books + sub-book toggles.
 * Pure — the caller just hands it to the transport seam. */
export function buildCombinedTailRequest(opts: {
  includeOptions: boolean;
  includeFi: boolean;
  optionLegs: readonly TailRiskOptionLeg[];
  fiPositions: readonly TailRiskFiPosition[];
  baseCurve: readonly TailRiskCurvePillar[];
  scenarios: readonly JointTailScenario[];
  alpha?: number;
}): CombinedTailRiskRequest {
  return {
    optionLegs: opts.includeOptions ? opts.optionLegs : [],
    fiPositions: opts.includeFi ? opts.fiPositions : [],
    baseCurve: opts.baseCurve,
    scenarios: opts.scenarios,
    alpha: opts.alpha ?? CT_ALPHA,
  };
}
