/**
 * SurfaceWorkspace — the "show me why" (GUI-DESIGN §4.3). Three linked views of
 * one marked surface: the 3D WebGPU-ready mesh, the per-tenor smile overlay, and
 * the broker marking panel (ATM / 25Δ&10Δ RR&BF). Selecting a point anywhere
 * cross-highlights everywhere and opens provenance (calibration inputs + arb
 * report) in the Inspector. Editing ATM/RR/BF reprices live with an arb banner
 * if a butterfly constraint breaks.
 *
 * Comparison note (positioning, not a product identifier): matches the
 * broker-vol marking workflow of incumbent vol terminals, and additionally
 * exposes per-mark calibration provenance — which LP-aggregating front-ends
 * cannot, because we own the calibration engine rather than re-publishing feeds.
 */

import { useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import type { BrokerQuoteSet, SmileModel } from "../data/contract";
import { Panel } from "../components/Panel";
import { Button } from "../components/Button";
import { ArbBanner } from "../components/ArbBanner";
import { SurfaceMesh } from "../viz/SurfaceMesh";
import { SmileChart } from "../viz/SmileChart";
import { calibrateLadder } from "../data/surface";
import { rampGradient } from "../viz/ramp";
import { fmtVol, fmtVolPoint, fmtClock } from "../lib/format";
import { tenorLabel } from "../lib/trend";
import { nowNanos } from "../hooks/useClock";
import { samePair } from "../lib/universe";
import { ASSET_CLASS_LABEL } from "../lib/assetUniverse";
import type { AssetClass } from "../products/types";
import { CubeWorkspace, type CubeDrill } from "./CubeWorkspace";
import styles from "./SurfaceWorkspace.module.css";

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
 * field. The labels are purpose-named (vendor/method-neutral, CLAUDE.md rule 8).
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
 * fabricated (CLAUDE.md rule 2).
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

  // STABLE surface-wide vol band (P0-9): the min/max calibrated vol across EVERY
  // smile point of the working preview. Passed to SmileChart so its y-axis stays
  // fixed across tenor switches and edits — smiles stay visually comparable rather
  // than the axis re-fitting to each curve. Recomputes only when the preview does.
  const volRange = useMemo(() => {
    if (!preview) return undefined;
    let min = Infinity;
    let max = -Infinity;
    for (const s of preview.smiles) {
      for (const p of s.points) {
        if (p.vol < min) min = p.vol;
        if (p.vol > max) max = p.vol;
      }
    }
    return Number.isFinite(min) && Number.isFinite(max) && max > min ? { min, max } : undefined;
  }, [preview]);

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

  if (!surface || !preview || !selectedSmile) {
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
  const previewSurface = { ...surface, smiles: preview.smiles };
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
          <SurfaceMesh surface={previewSurface} selected={{ tenorYears: selTenorYears, delta: selDelta ?? 0.5 }} />
        </div>
        <div className={styles.smileHolder}>
          <SmileChart
            smile={selectedSmile}
            selectedDelta={selDelta}
            onSelect={setSelDelta}
            volRange={volRange}
          />
          <div className={styles.legend}>
            <span className={styles.legendLabel}>low</span>
            <span className={styles.legendBar} style={{ background: rampGradient() }} />
            <span className={styles.legendLabel}>high</span>
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
                      <input
                        key={h}
                        className={`num ${styles.editCell} ${isEdited ? styles.editDirty : ""}`}
                        type="number"
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
