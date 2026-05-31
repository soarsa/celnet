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
import styles from "./SurfaceWorkspace.module.css";

/** The five editable broker handles, in display order. */
type Handle = "atmVol" | "rr25" | "bf25" | "rr10" | "bf10";
const HANDLES: Handle[] = ["atmVol", "rr25", "bf25", "rr10", "bf10"];

/**
 * The selectable smile-calibration models. These are a REAL control now: the
 * contract's `MarkSurfaceRequest.smile_model` field routes the selection to the
 * server's calibration engine (VV/SABR/SVI/SSVI), which marks under the chosen
 * family and echoes it in each smile's `arbitrage.note` as `model=<family>`. The
 * labels are purpose-named (vendor/method-neutral, CLAUDE.md rule 8).
 */
const SMILE_MODELS: { id: SmileModel; label: string; hint: string }[] = [
  { id: "MARKET_HEDGE", label: "Market hedge", hint: "Desk market-hedge construction (default)" },
  { id: "STOCHASTIC_VOL", label: "Stochastic vol", hint: "Stochastic-vol fit to the broker anchors" },
  { id: "PARAMETRIC", label: "Parametric", hint: "Parametric per-slice fit" },
  { id: "PARAMETRIC_SURFACE", label: "Parametric surface", hint: "Parametric whole-surface fit" },
];

/** Read the `model=<family>` provenance the server stamps into a smile's arb note. */
function modelProvenance(note: string): string | null {
  const m = note.match(/model=([\w-]+)/);
  return m ? m[1]! : null;
}
/** Stable per-tenor key — selection compares on this, never an absolute-float window. */
const tkey = (t: number): string => t.toFixed(8);
/** Snap a signed delta to a stable integer key (pillars are coarse: 0.10/0.25/0.50…). */
const deltaKey = (d: number): number => Math.round(d * 1e4);

export function SurfaceWorkspace(): React.ReactElement {
  const app = useApp();
  const [selTenorYears, setSelTenorYears] = useState(30 / 365);
  const [selDelta, setSelDelta] = useState<number | null>(-0.25);
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

  if (!surface || !preview || !selectedSmile) {
    return <div className={styles.loading}>Marking surface…</div>;
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
      <Panel glyph="◷" title={`Surface · ${surface.pair.base}/${surface.pair.quote}`} className={styles.surfacePanel} noPadding>
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
              <button
                key={tkey(s.tenorYears)}
                className={`${styles.markRow} ${active ? styles.markActive : ""}`}
                onClick={() => setSelTenorYears(s.tenorYears)}
              >
                <span className={styles.tenorCell}>{tenorLabel(s.tenorYears)}</span>
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
                        onClick={(e) => e.stopPropagation()}
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
              </button>
            );
          })}
        </div>

        <ArbBanner arb={arb} />

        {/* P0-7 / Phase-1 model SELECTOR — a REAL control. The contract's
            `MarkSurfaceRequest.smile_model` routes the choice to the server's
            calibration engine; selecting a model re-marks the live surface under it
            and bumps `surface_version`. The model used is read back from the marked
            smile's `arbitrage.note` provenance channel (`model=<family>`). */}
        <div className={styles.modelSelect} role="group" aria-label="smile calibration model">
          <span className={styles.provLabel}>model</span>
          {SMILE_MODELS.map((m) => {
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
              calibrated under, parsed from the marked smile's arb note. Falls back to
              the selected model's label if the note carries no tag. */}
          <span title="The calibration family the server marked this surface under (from the smile arb-report provenance).">
            {modelProvenance(selectedSmile.arbitrage.note) ??
              SMILE_MODELS.find((m) => m.id === app.surfaceModel)?.label.toLowerCase() ??
              "market-hedge"}
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
