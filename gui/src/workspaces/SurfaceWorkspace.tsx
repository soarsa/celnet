/**
 * SurfaceWorkspace — the FX (options / metals) vol-surface LENS of the shared,
 * class-parametric `MarketDataWorkspace` (`fe-fi-migration` #2); the `surface`
 * rail row opens the Market Data workspace on this lens. Behaviour is unchanged:
 * it is the "show me why" (GUI-DESIGN §4.3). Three linked views of
 * one marked surface: the rotatable 3D vol surface (the `VolSurface3D` lib chart,
 * three.js — height + Viridis colour both encode vol, arb-flagged tenors carry
 * danger markers), the per-tenor smile family (the `VolSmile` lib chart, visx —
 * the selected tenor is focused with pillar marks + RR/BF read-out, the rest
 * overlay as the ramp-ordered family), and the broker marking panel
 * (ATM / 25Δ&10Δ RR&BF). Selecting a wing chip or a marking row cross-highlights
 * and opens provenance (calibration inputs + arb report) in the Inspector.
 * Editing ATM/RR/BF reprices live with an arb banner if a butterfly constraint
 * breaks.
 *
 * Comparison note (positioning, not a product identifier): matches the
 * broker-vol marking workflow of incumbent vol terminals, and additionally
 * exposes per-mark calibration provenance — which LP-aggregating front-ends
 * cannot, because we own the calibration engine rather than re-publishing feeds.
 */

import { useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import type { BrokerQuoteSet, Smile, SmileModel } from "../data/contract";
import { Panel } from "../components/Panel";
import { Button } from "../components/Button";
import { ArbBanner } from "../components/ArbBanner";
import { ParentSize } from "@visx/responsive";
import { VolSurface3D, type VolSurfaceMarker } from "../viz/VolSurface3D";
import { VolSmile, type SmileTenor } from "../viz/VolSmile";
import { calibrateLadder } from "../data/surface";
import { fmtVol, fmtVolPoint, fmtClock } from "../lib/format";
import { tenorLabel } from "../lib/trend";
import { nowNanos } from "../hooks/useClock";
import { samePair } from "../lib/universe";
import { ASSET_CLASS_LABEL } from "../lib/assetUniverse";
import type { AssetClass } from "../products/types";
import { CubeWorkspace, type CubeDrill } from "./CubeWorkspace";
import styles from "./SurfaceWorkspace.module.css";

import { NumberField } from "../components/NumberField";

/** The Surface workspace's internal view: mark one surface, or scan the cube. */
type SurfaceView = "mark" | "cube";

/** The five editable broker handles, in display order. */
type Handle = "atmVol" | "rr25" | "bf25" | "rr10" | "bf10";
const HANDLES: Handle[] = ["atmVol", "rr25", "bf25", "rr10", "bf10"];

/**
 * The selectable smile-calibration models. These are a REAL control now: the
 * contract's `MarkSurfaceRequest.smile_model` field routes the selection to the
 * server's calibration engine (VV/SABR/SVI/SSVI), which marks under the chosen
 * family and echoes it back in each smile's TYPED `arbitrage.model` provenance
 * field. The labels are purpose-named (vendor/method-neutral, GUIDE.md rule 8).
 */
const SMILE_MODELS: { id: SmileModel; label: string; hint: string }[] = [
  { id: "MARKET_HEDGE", label: "Market hedge", hint: "Desk market-hedge construction (default)" },
  { id: "STOCHASTIC_VOL", label: "Stochastic vol", hint: "Stochastic-vol fit to the broker anchors" },
  { id: "PARAMETRIC", label: "Parametric", hint: "Parametric per-slice fit" },
  { id: "PARAMETRIC_SURFACE", label: "Parametric surface", hint: "Parametric whole-surface fit" },
  {
    id: "EXTENDED_SURFACE",
    label: "eSSVI",
    hint: "Extended whole-surface fit with a maturity-dependent skew",
  },
];

/**
 * The smile-calibration families marked PER ASSET CLASS — the asset-class-aware
 * family switch, as a DATA table: a future non-FX surface family (a crypto
 * delta-space family, an equity strike-space family) is an entry here, never a
 * workspace rewrite. Today ONLY FX carries marked surfaces (the five delta-space
 * calibration families above); every other class is an honest empty list and the
 * workspace renders the typed unavailable state — a non-FX surface is NEVER
 * fabricated (GUIDE.md rule 2).
 */
const CLASS_SMILE_FAMILIES: Record<
  AssetClass,
  readonly { id: SmileModel; label: string; hint: string }[]
> = {
  FX: SMILE_MODELS,
  METAL: [],
  EQUITY: [],
  COMMODITY: [],
  CRYPTO: [],
};

/**
 * The stable, vendor-/method-neutral family label for a TYPED [`SmileModel`] —
 * the calibration family the server actually marked under, read from the typed
 * `arbitrage.model` provenance field (the `model=` note regex is RETIRED). Mirrors
 * the server's `smile_model_label`, so GUI and server agree byte-for-byte.
 */
function modelProvenance(model: SmileModel): string {
  switch (model) {
    case "MARKET_HEDGE":
      return "market-hedge";
    case "STOCHASTIC_VOL":
      return "stochastic-vol";
    case "PARAMETRIC":
      return "parametric";
    case "PARAMETRIC_SURFACE":
      return "parametric-surface";
    case "EXTENDED_SURFACE":
      return "extended-surface";
  }
}
/** Stable per-tenor key — selection compares on this, never an absolute-float window. */
const tkey = (t: number): string => t.toFixed(8);
/** Snap a signed delta to a stable integer key (pillars are coarse: 0.10/0.25/0.50…). */
const deltaKey = (d: number): number => Math.round(d * 1e4);

/**
 * Wing-axis plotting coordinate for a signed convention delta — puts left, ATM
 * centre, calls right: `x = sign(Δ)·(0.5 − |Δ|)`, so 10ΔP → −0.40, 25ΔP → −0.25,
 * ATM (Δ = 0.5) → 0, 25ΔC → +0.25, 10ΔC → +0.40. Monotone left→right in strike,
 * matching the desk's smile reading order (the lib charts' delta axis).
 */
const wingX = (delta: number): number => Math.sign(delta) * (0.5 - Math.abs(delta));

/** Canonical wing label for a signed convention delta ("10ΔP" / "ATM" / "25ΔC"). */
const wingLabel = (delta: number): string =>
  Math.abs(delta) >= 0.49 ? "ATM" : `${Math.round(Math.abs(delta) * 100)}Δ${delta < 0 ? "P" : "C"}`;

// --- publish evidence (implied − model residuals · ATM term slice · arb gate) --

/**
 * One residual cell of the evidence heatmap: `implied − model` in VOL POINTS at a
 * (tenor, wing) pillar — the broker-implied market vol minus the calibrated model
 * vol. `null` ⇒ the broker ladder carries no quote at that pillar (a 3-pt ladder's
 * 10Δ column); rendered as an honest em-dash, never fabricated as 0.
 */
export interface ResidualCell {
  readonly wing: string;
  readonly residual: number | null;
}

/** One evidence row — everything the publish gate knows about ONE tenor. */
export interface EvidenceRow {
  readonly tenorYears: number;
  readonly tenor: string;
  /** The calibrated ATM vol (absolute) — the term-structure slice's y value. */
  readonly atmVol: number;
  readonly butterflyOk: boolean;
  readonly calendarOk: boolean;
  readonly note: string;
  readonly cells: readonly ResidualCell[];
}

/** The derived evidence trail for a calibrated smile ladder (see {@link deriveSurfaceEvidence}). */
export interface SurfaceEvidence {
  /** Residual-heatmap column labels, wing order (10ΔP … ATM … 10ΔC). */
  readonly wings: readonly string[];
  /** Rows in ascending-tenor order. */
  readonly rows: readonly EvidenceRow[];
  /** Largest |residual| (vol points) — the symmetric diverging-domain half-width. */
  readonly maxAbsResidual: number;
  /** The worst non-zero residual cell, or `null` when every quoted pillar fits exactly. */
  readonly worst: { readonly tenor: string; readonly wing: string; readonly residual: number } | null;
}

/**
 * The broker-IMPLIED pillar vol at a signed convention delta — the market side of
 * the residual, from the standard broker decomposition of the quote set:
 * ATM (|Δ| ≥ 0.49) → `atm`; 25Δ → `atm + bf25 ± rr25/2`; 10Δ → `atm + bf10 ± rr10/2`
 * (call +, put −). Returns `null` when the ladder carries no quote at that pillar
 * (the 10Δ columns of a 3-pt ladder, or a non-pillar delta) — never a guessed vol.
 */
export function brokerImpliedPillarVol(q: BrokerQuoteSet, delta: number): number | null {
  if (!Number.isFinite(delta)) return null;
  const ad = Math.abs(delta);
  if (ad >= 0.49) return q.atmVol;
  const half = delta > 0 ? 0.5 : -0.5;
  if (Math.abs(ad - 0.25) <= 0.02) return q.atmVol + q.bf25 + half * q.rr25;
  if (Math.abs(ad - 0.1) <= 0.02) return q.hasTenDelta ? q.atmVol + q.bf10 + half * q.rr10 : null;
  return null;
}

/**
 * Derive the WHOLE publish-evidence trail from the calibrated smiles the workspace
 * already holds — the implied−model residual grid (tenor × wing, vol points), the
 * ATM term-structure slice, and the per-tenor arb-gate rows. Pure and total: no new
 * wire call, every number is a re-read of `points{delta,vol}` + `brokerQuotes` +
 * `arbitrage` (exported so the test can pin it through the public surface).
 */
export function deriveSurfaceEvidence(smiles: readonly Smile[]): SurfaceEvidence {
  const ordered = [...smiles].sort((a, b) => a.tenorYears - b.tenorYears);
  const first = ordered[0];
  if (!first || first.points.length === 0) {
    return { wings: [], rows: [], maxAbsResidual: 0, worst: null };
  }
  const wingDeltas = [...first.points]
    .sort((a, b) => wingX(a.delta) - wingX(b.delta))
    .map((p) => p.delta);
  const wings = wingDeltas.map(wingLabel);

  let maxAbs = 0;
  let worst: { tenor: string; wing: string; residual: number } | null = null;
  const rows: EvidenceRow[] = ordered.map((s) => {
    const tenor = tenorLabel(s.tenorYears);
    const byKey = new Map(s.points.map((p) => [deltaKey(p.delta), p] as const));
    const cells: ResidualCell[] = wingDeltas.map((delta, i) => {
      const wing = wings[i] ?? wingLabel(delta);
      const point = byKey.get(deltaKey(delta));
      const implied = brokerImpliedPillarVol(s.brokerQuotes, delta);
      if (!point || implied === null || !Number.isFinite(point.vol)) return { wing, residual: null };
      const residual = (implied - point.vol) * 100;
      const a = Math.abs(residual);
      if (a > maxAbs) maxAbs = a;
      if (a > 1e-9 && (worst === null || a > Math.abs(worst.residual))) {
        worst = { tenor, wing, residual };
      }
      return { wing, residual };
    });
    const atmPoint = s.points.find((p) => Math.abs(p.delta) >= 0.49);
    return {
      tenorYears: s.tenorYears,
      tenor,
      atmVol: atmPoint ? atmPoint.vol : s.brokerQuotes.atmVol,
      butterflyOk: s.arbitrage.butterflyArbitrageFree,
      calendarOk: s.arbitrage.calendarArbitrageFree,
      note: s.arbitrage.note,
      cells,
    };
  });
  return { wings, rows, maxAbsResidual: maxAbs, worst };
}

/** The five-band diverging cut (same banding as the risk heatmap): symmetric about 0 at ±¼ and ±½ of |max|. */
type ResidualBand = "n2" | "n1" | "mid" | "p1" | "p2";
function residualBand(residual: number, maxAbs: number): ResidualBand {
  const a = Math.abs(residual);
  if (maxAbs <= 0 || a <= maxAbs / 4) return "mid";
  if (a <= maxAbs / 2) return residual < 0 ? "n1" : "p1";
  return residual < 0 ? "n2" : "p2";
}

/** Signed 2-dp residual in vol points, with a REAL minus sign (U+2212). */
function fmtResidual(v: number): string {
  const r = Math.round(v * 100) / 100;
  const mag = Math.abs(r).toFixed(2);
  return r < 0 ? `−${mag}` : r > 0 ? `+${mag}` : mag;
}

/**
 * The ATM term-structure slice — a small inline-SVG line of the calibrated ATM vol
 * across the tenor ladder (categorical, evenly spaced ticks). A calendar-arb tenor's
 * dot renders in the danger status colour: the calendar gate IS a statement about
 * this line (ATM total variance must rise in T), so the violation is marked WHERE it
 * bites. SVG consumes the CSS tokens natively (no canvas token-resolution needed).
 */
function AtmTermSlice({ rows }: { rows: readonly EvidenceRow[] }): React.ReactElement | null {
  const W = 320;
  const H = 104;
  const padL = 36;
  const padR = 10;
  const padT = 8;
  const padB = 16;
  const innerW = W - padL - padR;
  const innerH = H - padT - padB;
  const vols = rows.map((r) => r.atmVol * 100);
  if (vols.length === 0 || vols.some((v) => !Number.isFinite(v))) return null;
  const lo = Math.min(...vols);
  const hi = Math.max(...vols);
  const pad = (hi - lo || Math.abs(hi) || 1) * 0.15;
  const vMin = lo - pad;
  const vMax = hi + pad;
  const x = (i: number): number =>
    padL + (rows.length === 1 ? innerW / 2 : (innerW * i) / (rows.length - 1));
  const y = (v: number): number => padT + innerH - (innerH * (v - vMin)) / (vMax - vMin);
  const path = vols
    .map((v, i) => `${i === 0 ? "M" : "L"}${x(i).toFixed(1)},${y(v).toFixed(1)}`)
    .join(" ");
  const summary = rows.map((r) => `${r.tenor} ${fmtVol(r.atmVol)}`).join(", ");
  return (
    <svg
      className={styles.termSvg}
      viewBox={`0 0 ${W} ${H}`}
      role="img"
      aria-label={`ATM vol term structure: ${summary}`}
    >
      {[vMin + pad, vMax - pad].map((v, i) => (
        // Keyed by rank, not value: a single-tenor ladder collapses lo === hi.
        <g key={i === 0 ? "lo" : "hi"}>
          <line className={styles.termGrid} x1={padL} y1={y(v)} x2={W - padR} y2={y(v)} />
          <text className={styles.termTick} x={padL - 4} y={y(v) + 2.5} textAnchor="end">
            {v.toFixed(2)}
          </text>
        </g>
      ))}
      <path className={styles.termLine} d={path} />
      {rows.map((r, i) => (
        <circle
          key={tkey(r.tenorYears)}
          className={r.calendarOk ? styles.termDot : styles.termDotDanger}
          cx={x(i)}
          cy={y(vols[i] ?? 0)}
          r={2.6}
        >
          <title>{r.calendarOk ? `${r.tenor} ATM ${fmtVol(r.atmVol)}` : `${r.tenor} — ${r.note}`}</title>
        </circle>
      ))}
      {rows.map((r, i) => (
        <text
          key={tkey(r.tenorYears)}
          className={styles.termTick}
          x={x(i)}
          y={H - 4}
          textAnchor="middle"
        >
          {r.tenor}
        </text>
      ))}
    </svg>
  );
}

/** The residual band → CSS-module class map (all colour from `--div-*` tokens). */
const BAND_CLASS: Record<ResidualBand, string> = {
  n2: styles.bandN2 ?? "",
  n1: styles.bandN1 ?? "",
  mid: styles.bandMid ?? "",
  p1: styles.bandP1 ?? "",
  p2: styles.bandP2 ?? "",
};

/**
 * The "show me WHY a publish is blocked" evidence trail (mockup 01's missing
 * product half): a collapsible region carrying (1) the implied−model residual
 * mini-heatmap (tenor × wing, `--div` diverging, centred 0), (2) the ATM
 * term-structure slice, and (3) the per-tenor arb-gate table — so a blocked
 * publish shows WHERE and WHY, not just THAT. Auto-opens while the gate is
 * blocked; the trader's explicit toggle then wins. All content derives from the
 * preview smiles the workspace already holds (no new wire call, nothing faked).
 */
export function SurfaceEvidenceTrail({ smiles }: { smiles: readonly Smile[] }): React.ReactElement {
  const [userOpen, setUserOpen] = useState<boolean | null>(null);
  const evidence = useMemo(() => deriveSurfaceEvidence(smiles), [smiles]);
  const blocked = smiles.some(
    (s) => !s.arbitrage.butterflyArbitrageFree || !s.arbitrage.calendarArbitrageFree,
  );
  const open = userOpen ?? blocked;
  const half = evidence.maxAbsResidual;
  return (
    <div className={styles.evidence}>
      <button
        type="button"
        className={styles.evToggle}
        aria-expanded={open}
        aria-controls="surface-evidence"
        onClick={() => setUserOpen(!open)}
      >
        <span className={styles.evChevron} data-open={open || undefined} aria-hidden="true">
          ▸
        </span>
        Evidence
        <span className={styles.evHint}>
          {blocked ? "publish blocked — where & why" : "residuals · term · arb gate"}
        </span>
      </button>
      {open && (
        <div
          id="surface-evidence"
          role="region"
          aria-label="publish evidence"
          className={styles.evBody}
        >
          {evidence.rows.length === 0 ? (
            <p className={styles.evEmpty}>no calibrated smiles — nothing to evidence</p>
          ) : (
            <>
              <h3 className={styles.evTitle}>Implied − model residuals</h3>
              <table
                className={styles.residTable}
                aria-label="implied minus model residual heatmap, vol points, diverging centred at zero"
              >
                <thead>
                  <tr>
                    <th scope="col" aria-label="tenor" />
                    {evidence.wings.map((w) => (
                      <th key={w} scope="col">
                        {w}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {evidence.rows.map((r) => (
                    <tr key={tkey(r.tenorYears)}>
                      <th scope="row">{r.tenor}</th>
                      {r.cells.map((c) =>
                        c.residual === null ? (
                          <td
                            key={c.wing}
                            className={styles.residNa}
                            title={`${r.tenor} ${c.wing} — no broker quote at this pillar (3-pt ladder)`}
                          >
                            —
                          </td>
                        ) : (
                          <td
                            key={c.wing}
                            className={`num ${styles.residCell} ${BAND_CLASS[residualBand(c.residual, half)]}`}
                            title={`${r.tenor} ${c.wing} · implied − model = ${fmtResidual(c.residual)} vol pts`}
                          >
                            {fmtResidual(c.residual)}
                          </td>
                        ),
                      )}
                    </tr>
                  ))}
                </tbody>
              </table>
              <div className={styles.residLegend}>
                <span className="num">−{half.toFixed(2)}</span>
                <span className={styles.legendRamp} aria-hidden="true" />
                <span className="num">+{half.toFixed(2)}</span>
                <span className={styles.legendCaption}>vol pts · diverging, centred 0</span>
              </div>
              {evidence.worst && (
                <div className={styles.worstLine}>
                  <span className={styles.provLabel}>worst residual</span>
                  <span className="num">
                    {fmtResidual(evidence.worst.residual)} vp · {evidence.worst.tenor}{" "}
                    {evidence.worst.wing}
                  </span>
                </div>
              )}

              <h3 className={styles.evTitle}>ATM term structure</h3>
              <AtmTermSlice rows={evidence.rows} />

              <h3 className={styles.evTitle}>Arb gate · per tenor</h3>
              <table className={styles.gateTable} aria-label="per-tenor arbitrage gate">
                <thead>
                  <tr>
                    <th scope="col">Tenor</th>
                    <th scope="col">Butterfly</th>
                    <th scope="col">Calendar</th>
                    <th scope="col">Note</th>
                  </tr>
                </thead>
                <tbody>
                  {evidence.rows.map((r) => (
                    <tr
                      key={tkey(r.tenorYears)}
                      className={!r.butterflyOk || !r.calendarOk ? styles.gateFailRow : undefined}
                    >
                      <th scope="row">{r.tenor}</th>
                      <td>
                        <span className={r.butterflyOk ? styles.gatePass : styles.gateFail}>
                          {r.butterflyOk ? "✓ pass" : "✕ fail"}
                        </span>
                      </td>
                      <td>
                        <span className={r.calendarOk ? styles.gatePass : styles.gateFail}>
                          {r.calendarOk ? "✓ pass" : "✕ fail"}
                        </span>
                      </td>
                      <td className={styles.gateNote}>{r.note}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}
        </div>
      )}
    </div>
  );
}

export function SurfaceWorkspace(): React.ReactElement {
  const app = useApp();
  // Internal view: the editable marking surface, or the read/scan vol cube. The
  // cube is a sibling pivot of the same marked surfaces (TRADING-UNIVERSE-SCALE
  // §5) — exposed here without touching Shell/AppContext (no new WorkspaceId).
  const [view, setView] = useState<SurfaceView>("mark");
  const [selTenorYears, setSelTenorYears] = useState(30 / 365);
  const [selDelta, setSelDelta] = useState<number | null>(-0.25);

  // Drill from a cube cell into the marking/smile view: re-target the pair if
  // needed, select that (tenor, delta), and flip back to the editable surface.
  const drillFromCube = (d: CubeDrill): void => {
    if (!samePair(d.pair, app.pairCtx.pair)) app.setPair(d.pair);
    setSelTenorYears(d.tenorYears);
    setSelDelta(d.delta);
    setView("mark");
  };

  const viewToggle = (
    <div className={styles.viewToggle} role="group" aria-label="surface view">
      <button
        type="button"
        className={`${styles.viewChip} ${view === "mark" ? styles.viewActive : ""}`}
        aria-pressed={view === "mark"}
        onClick={() => setView("mark")}
        title="Mark and inspect one pair's surface"
      >
        Surface
      </button>
      <button
        type="button"
        className={`${styles.viewChip} ${view === "cube" ? styles.viewActive : ""}`}
        aria-pressed={view === "cube"}
        onClick={() => setView("cube")}
        title="Scan the vol cube (pair × tenor × delta heatmap)"
      >
        Cube
      </button>
    </div>
  );

  // Per-tenor working edits to the broker handles (ATM/RR/BF), keyed by tenor.
  // These are the trader's *unpublished* marks; Publish transmits them through the
  // same MarkSurface API the SDK/Excel use, and Reset discards them.
  const [edits, setEdits] = useState<Record<string, Partial<BrokerQuoteSet>>>({});

  const surface = app.surface;

  // The working ladder = the published broker quotes with the trader's edits laid
  // on top, per tenor. Recalibrated through the SAME model as a real mark
  // (`calibrateLadder` = butterfly per-smile + calendar across tenors) so the live
  // preview AND the publish gate agree with what the server will compute.
  const preview = useMemo(() => {
    if (!surface) return null;
    const ladder: BrokerQuoteSet[] = surface.smiles.map((s) => ({
      ...s.brokerQuotes,
      ...edits[tkey(s.tenorYears)],
    }));
    // Preview under the SAME model the server will mark with (app.surfaceModel),
    // so the live edit preview and the publish-gate arb check agree with the
    // server's calibration. (The live WS transport recalibrates server-side; the
    // mock recalibrates with the identical model locally.)
    const smiles = calibrateLadder(
      surface.pair,
      ladder,
      app.conventions,
      nowNanos(),
      app.surfaceModel,
    );
    return { ladder, smiles };
  }, [surface, edits, app.conventions, app.surfaceModel]);

  const selectedSmile = useMemo(() => {
    if (!preview) return null;
    // Nearest tenor by absolute distance — a robust selector that never depends on
    // an exact-float match (which breaks after recalibration re-derives tenorYears).
    let best = preview.smiles[0];
    let bestD = Infinity;
    for (const s of preview.smiles) {
      const d = Math.abs(s.tenorYears - selTenorYears);
      if (d < bestD) {
        bestD = d;
        best = s;
      }
    }
    return best ?? null;
  }, [preview, selTenorYears]);

  // The (tenor × delta) grid for the 3D surface lib chart: vols in vol POINTS
  // (the contract's vols are absolute, 0.10 = 10 vol), rows in tenor order,
  // columns in wing order (10ΔP … ATM … 10ΔC). An arb-violating tenor surfaces
  // as danger markers across its whole row — the arb report is per-smile, so a
  // single-vertex attribution would be fabricated (GUIDE.md rule 2).
  const surfaceViz = useMemo(() => {
    if (!preview) return null;
    const rows = preview.smiles.map((s) => ({
      smile: s,
      points: [...s.points].sort((a, b) => wingX(a.delta) - wingX(b.delta)),
    }));
    const first = rows[0];
    if (!first || first.points.length === 0) return null;
    const markers: VolSurfaceMarker[] = [];
    rows.forEach((r, tenorIndex) => {
      const arb = r.smile.arbitrage;
      if (arb.butterflyArbitrageFree && arb.calendarArbitrageFree) return;
      const kinds = [
        ...(arb.butterflyArbitrageFree ? [] : ["butterfly"]),
        ...(arb.calendarArbitrageFree ? [] : ["calendar"]),
      ].join(" + ");
      const reason = `${tenorLabel(r.smile.tenorYears)} ${kinds} arbitrage${arb.note ? ` — ${arb.note}` : ""}`;
      r.points.forEach((_, deltaIndex) => markers.push({ tenorIndex, deltaIndex, reason }));
    });
    return {
      vols: rows.map((r) => r.points.map((p) => p.vol * 100)),
      tenors: rows.map((r) => tenorLabel(r.smile.tenorYears)),
      deltas: first.points.map((p) => wingLabel(p.delta)),
      markers,
    };
  }, [preview]);

  // The WHOLE preview family feeds the smile lib chart — VolSmile shares one
  // x/y domain across every tenor it is given, so the y-axis stays fixed across
  // tenor switches (the P0-9 stable-axis behavior volRange used to provide) and
  // the family overlays ramp-ordered. The selected tenor is the focused one
  // (pillar marks + the RR/BF read-out); the calibrated pillar points ARE the
  // fit geometry — nothing is densified or invented between pillars.
  const smileTenors = useMemo<readonly SmileTenor[]>(() => {
    if (!preview || !selectedSmile) return [];
    const focusKey = tkey(selectedSmile.tenorYears);
    return preview.smiles.map((s) => {
      const pillars = [...s.points]
        .sort((a, b) => wingX(a.delta) - wingX(b.delta))
        .map((p) => ({ x: wingX(p.delta), mid: p.vol * 100, label: wingLabel(p.delta) }));
      return {
        tenor: tenorLabel(s.tenorYears),
        pillars,
        fit: pillars.map((p) => ({ x: p.x, y: p.mid })),
        focus: tkey(s.tenorYears) === focusKey,
      };
    });
  }, [preview, selectedSmile]);

  // The asset-class family gate: the families marked for the ACTIVE underlier's
  // class. FX → the five delta-space families (everything below renders exactly
  // as before); a class with no marked family → the typed, honest unavailable
  // state (the marked surfaces AND the cube are FX-keyed today — no fake data).
  const families = CLASS_SMILE_FAMILIES[app.underlier.assetClass];
  if (families.length === 0) {
    const className = ASSET_CLASS_LABEL[app.underlier.assetClass];
    return (
      <div className={styles.cubeShell}>
        <section className={styles.classEmpty} role="status" aria-label="surface unavailable">
          <h2 className={styles.classEmptyTitle}>No marked surface for {className}</h2>
          <p className={styles.classEmptyBody}>
            <span className="num">{app.underlier.label}</span> is a {className} underlier. Marked
            vol surfaces are FX-only today — the calibration families here are FX delta-space
            (market-hedge / stochastic-vol / parametric / parametric-surface / eSSVI), marked off
            FX broker ladders. A {className} surface family will appear as data when its marking
            engine lands; nothing is fabricated in the meantime.
          </p>
          <Button variant="secondary" onClick={() => app.setScopeSwitcherOpen(true)}>
            Switch underlier
          </Button>
        </section>
      </div>
    );
  }

  // The cube pivot is reachable regardless of the marking-surface state (it loads
  // its own per-pair surfaces through `useCube`), so this branch sits AFTER all
  // hooks but BEFORE the mark-view's loading gate.
  if (view === "cube") {
    return (
      <div className={styles.cubeShell}>
        <div className={styles.cubeBar}>{viewToggle}</div>
        <div className={styles.cubeCanvas}>
          <CubeWorkspace onDrill={drillFromCube} />
        </div>
      </div>
    );
  }

  if (!surface || !preview || !selectedSmile || !surfaceViz) {
    return (
      <div className={styles.cubeShell}>
        <div className={styles.cubeBar}>{viewToggle}</div>
        <div className={styles.loading}>Marking surface…</div>
      </div>
    );
  }

  // The selected tenor's stable key — the marking grid marks a row active by
  // comparing this against each row's key, never by an absolute-float window
  // (`< 1e-9`), which could mis-select after the ladder recalibrates.
  const selectedTenorKey = tkey(selectedSmile.tenorYears);
  const dirty = Object.values(edits).some((e) => Object.keys(e).length > 0);
  // Publish gate: the WHOLE surface must be arb-free (every tenor's butterfly AND
  // the cross-tenor calendar check) — not just the selected smile's butterfly.
  const surfaceArbFree = preview.smiles.every(
    (s) => s.arbitrage.butterflyArbitrageFree && s.arbitrage.calendarArbitrageFree,
  );
  const arb = selectedSmile.arbitrage;
  const nextVersion = (surface.surfaceVersion + 1n).toString();

  const setHandle = (tenorYears: number, field: Handle, raw: string): void => {
    const v = Number(raw);
    if (!Number.isFinite(v)) return;
    const k = tkey(tenorYears);
    setEdits((prev) => ({ ...prev, [k]: { ...prev[k], [field]: v / 100 } }));
  };

  return (
    <div className={styles.grid}>
      <Panel glyph="◷" title={`Surface · ${surface.pair.base}/${surface.pair.quote}`} className={styles.surfacePanel} noPadding actions={viewToggle}>
        <div className={styles.meshHolder}>
          {/* The lib 3D surface chart fills the holder; ParentSize (debounced —
              a resize rebuilds the GL scene) supplies the concrete px box the
              WebGL canvas needs. Its Viridis legend + axis cues are built in. */}
          <ParentSize debounceTime={80}>
            {({ width, height }) =>
              width < 40 || height < 40 ? null : (
                <VolSurface3D
                  vols={surfaceViz.vols}
                  tenors={surfaceViz.tenors}
                  deltas={surfaceViz.deltas}
                  markers={surfaceViz.markers}
                  width={Math.floor(width)}
                  height={Math.floor(height)}
                />
              )
            }
          </ParentSize>
        </div>
        <div className={styles.smileHolder}>
          <VolSmile tenors={smileTenors} height={200} />
          {/* Wing selection — the smile slice's cross-highlight control: picking a
              wing drives the Inspector read-out (and survives a cube drill-in),
              keyboard-reachable where the old canvas click was not. */}
          <div className={styles.wingChips} role="group" aria-label="smile wing selection">
            <span className={styles.provLabel}>wing</span>
            {[...selectedSmile.points]
              .sort((a, b) => wingX(a.delta) - wingX(b.delta))
              .map((p) => {
                const active = selDelta !== null && deltaKey(p.delta) === deltaKey(selDelta);
                return (
                  <button
                    key={deltaKey(p.delta)}
                    type="button"
                    className={`${styles.wingChip} ${active ? styles.wingChipActive : ""}`}
                    aria-pressed={active}
                    onClick={() => setSelDelta(p.delta)}
                    title={`Inspect ${wingLabel(p.delta)} — ${fmtVol(p.vol)}`}
                  >
                    {wingLabel(p.delta)}
                  </button>
                );
              })}
          </div>
        </div>
      </Panel>

      <Panel glyph="⌗" title="Marking" className={styles.markPanel}>
        <div className={styles.markGrid}>
          <div className={`${styles.markHead}`}>
            <span>Tenor</span>
            <span>ATM</span>
            <span>25RR</span>
            <span>25BF</span>
            <span>10RR</span>
            <span>10BF</span>
          </div>
          {preview.smiles.map((s) => {
            const active = tkey(s.tenorYears) === selectedTenorKey;
            const q = s.brokerQuotes;
            const edited = edits[tkey(s.tenorYears)] ?? {};
            return (
              // A11y: the row is a plain container, NOT a <button> — the active row
              // holds focusable <input>s, and a button-with-focusable-descendants is
              // a nested-interactive violation. The tenor cell is the row's single
              // select control (a real button); inactive cells are static, active
              // cells are the edit inputs (now siblings of, not nested in, a button).
              // No role="row" here: we don't claim a grid, so a bare div is correct.
              <div
                key={tkey(s.tenorYears)}
                className={`${styles.markRow} ${active ? styles.markActive : ""}`}
              >
                <button
                  type="button"
                  className={styles.tenorCell}
                  onClick={() => setSelTenorYears(s.tenorYears)}
                  aria-pressed={active}
                  title={`Select ${tenorLabel(s.tenorYears)} to edit its marks`}
                >
                  {tenorLabel(s.tenorYears)}
                </button>
                {HANDLES.map((h) => {
                  const val = q[h];
                  const isEdited = edited[h] !== undefined;
                  if (active) {
                    return (
                      <NumberField
                        key={h}
                        className={`num ${styles.editCell} ${isEdited ? styles.editDirty : ""}`}
                        step={h === "atmVol" ? 0.05 : 0.01}
                        value={(val * 100).toFixed(2)}
                        onChange={(e) => setHandle(s.tenorYears, h, e.target.value)}
                        aria-label={`${tenorLabel(s.tenorYears)} ${h}`}
                      />
                    );
                  }
                  return (
                    <span key={h} className={`num ${isEdited ? styles.cellDirty : ""}`}>
                      {h === "atmVol" ? fmtVol(val) : fmtVolPoint(val)}
                    </span>
                  );
                })}
              </div>
            );
          })}
        </div>

        <ArbBanner arb={arb} />

        {/* The publish-evidence trail: the banner says THAT the gate is blocked;
            this region shows WHERE and WHY across the whole surface (residual
            heatmap · ATM term slice · per-tenor arb gate), derived from the SAME
            preview smiles that drive the publish gate — they always agree. */}
        <SurfaceEvidenceTrail smiles={preview.smiles} />

        {/* P0-7 / Phase-1 model SELECTOR — a REAL control. The contract's
            `MarkSurfaceRequest.smile_model` routes the choice to the server's
            calibration engine; selecting a model re-marks the live surface under it
            and bumps `surface_version`. The model used is read back from the marked
            smile's `arbitrage.note` provenance channel (`model=<family>`). */}
        {/* The asset-class-aware family switch: the chips render the ACTIVE
            class's marked families (`CLASS_SMILE_FAMILIES`) — for FX, exactly the
            five delta-space families as before; a future non-FX family joins as
            data and its chips appear here with zero rewrite. */}
        <div className={styles.modelSelect} role="group" aria-label="smile calibration model">
          <span className={styles.provLabel}>model</span>
          {families.map((m) => {
            const active = m.id === app.surfaceModel;
            return (
              <button
                key={m.id}
                type="button"
                className={`${styles.modelChip} ${active ? styles.modelChipActive : ""}`}
                aria-pressed={active}
                onClick={() => {
                  // Selecting a model re-marks the LIVE published surface under it;
                  // any unpublished handle edits are discarded (a model change marks
                  // a fresh version off the live ladder).
                  setEdits({});
                  app.setSurfaceModel(m.id);
                }}
                title={m.hint}
              >
                {m.label}
              </button>
            );
          })}
        </div>
        <div className={styles.provenance}>
          <span className={styles.provLabel}>marked as</span>
          {/* The honest provenance the server reports — the model family it actually
              calibrated under, read from the marked smile's TYPED `arbitrage.model`
              provenance field (the authoritative source; the `model=` note regex is
              retired). */}
          <span title="The calibration family the server marked this surface under (from the smile arb-report's typed provenance field).">
            {modelProvenance(selectedSmile.arbitrage.model)}
          </span>
          <span className={styles.provDot}>·</span>
          <span>{selectedSmile.brokerQuotes.hasTenDelta ? "5-pt handles" : "3-pt handles"}</span>
        </div>
        <div className={styles.provenance}>
          <span className={styles.provLabel}>marked</span>
          <span className="num">{fmtClock(selectedSmile.epochNanos)}</span>
          <span className={styles.provDot}>·</span>
          <span className="num">surf v{surface.surfaceVersion.toString()}</span>
          {dirty && <span className={styles.dirtyBadge}>● unpublished edits</span>}
        </div>

        <div className={styles.markActions}>
          <Button variant="secondary" onClick={() => setEdits({})} disabled={!dirty}>
            Reset to live
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              void app.remarkSurface(preview.ladder);
              setEdits({});
            }}
            disabled={!dirty || !surfaceArbFree}
            title={
              !surfaceArbFree
                ? "Resolve the arbitrage violation before publishing"
                : !dirty
                  ? "No unpublished edits"
                  : `Publish the edited marks as surface v${nextVersion}`
            }
          >
            {dirty ? `Publish v${nextVersion}` : "Published"}
          </Button>
        </div>

        {selDelta !== null && (
          <div className={styles.inspector}>
            <span className={styles.provLabel}>selected</span>
            <span className="num">
              {Math.abs(selDelta) >= 0.49 ? "ATM" : `${Math.round(Math.abs(selDelta) * 100)}Δ ${selDelta < 0 ? "put" : "call"}`}
              {" · "}
              {fmtVol(
                selectedSmile.points.find((p) => deltaKey(p.delta) === deltaKey(selDelta))?.vol ??
                  selectedSmile.brokerQuotes.atmVol,
              )}
            </span>
          </div>
        )}
      </Panel>
    </div>
  );
}
