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
import type { BrokerQuoteSet } from "../data/contract";
import { Panel } from "../components/Panel";
import { Button } from "../components/Button";
import { ArbBanner } from "../components/ArbBanner";
import { SurfaceMesh } from "../viz/SurfaceMesh";
import { SmileChart } from "../viz/SmileChart";
import { calibrateLadder } from "../data/surface";
import { rampGradient } from "../viz/ramp";
import { fmtVol, fmtVolPoint, fmtClock } from "../lib/format";
import { nowNanos } from "../hooks/useClock";
import styles from "./SurfaceWorkspace.module.css";

/** The five editable broker handles, in display order. */
type Handle = "atmVol" | "rr25" | "bf25" | "rr10" | "bf10";
const HANDLES: Handle[] = ["atmVol", "rr25", "bf25", "rr10", "bf10"];
const tkey = (t: number): string => t.toFixed(8);

function tenorName(years: number): string {
  const days = Math.round(years * 365);
  if (days <= 1) return "ON";
  if (days < 28) return `${Math.round(days / 7)}W`;
  if (days < 360) return `${Math.round(days / 30)}M`;
  return `${Math.round(days / 365)}Y`;
}

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
    const smiles = calibrateLadder(surface.pair, ladder, app.conventions, nowNanos());
    return { ladder, smiles };
  }, [surface, edits, app.conventions]);

  const selectedSmile = useMemo(() => {
    if (!preview) return null;
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

  if (!surface || !preview || !selectedSmile) {
    return <div className={styles.loading}>Marking surface…</div>;
  }

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
            const active = Math.abs(s.tenorYears - selTenorYears) < 1e-9;
            const q = s.brokerQuotes;
            const edited = edits[tkey(s.tenorYears)] ?? {};
            return (
              <button
                key={tkey(s.tenorYears)}
                className={`${styles.markRow} ${active ? styles.markActive : ""}`}
                onClick={() => setSelTenorYears(s.tenorYears)}
              >
                <span className={styles.tenorCell}>{tenorName(s.tenorYears)}</span>
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
                        aria-label={`${tenorName(s.tenorYears)} ${h}`}
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

        <div className={styles.provenance}>
          <span className={styles.provLabel}>source</span>
          <span>{selectedSmile.brokerQuotes.hasTenDelta ? "5-pt broker" : "3-pt broker"}</span>
          <span className={styles.provDot}>·</span>
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
              {fmtVol(selectedSmile.points.find((p) => Math.abs(p.delta - selDelta) < 1e-6)?.vol ?? selectedSmile.brokerQuotes.atmVol)}
            </span>
          </div>
        )}
      </Panel>
    </div>
  );
}
