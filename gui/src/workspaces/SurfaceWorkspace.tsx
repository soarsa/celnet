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
import { calibrateSmile } from "../data/surface";
import { rampGradient } from "../viz/ramp";
import { fmtVol, fmtVolPoint, fmtClock } from "../lib/format";
import { nowNanos } from "../hooks/useClock";
import styles from "./SurfaceWorkspace.module.css";

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
  const [editAtm, setEditAtm] = useState<number | null>(null);

  const surface = app.surface;

  const selectedSmile = useMemo(() => {
    if (!surface) return null;
    let best = surface.smiles[0];
    let bestD = Infinity;
    for (const s of surface.smiles) {
      const d = Math.abs(s.tenorYears - selTenorYears);
      if (d < bestD) {
        bestD = d;
        best = s;
      }
    }
    if (!best) return null;
    // Apply a live ATM edit (re-calibrate that tenor) without mutating the mark.
    if (editAtm !== null && Math.abs(best.tenorYears - selTenorYears) < 1e-9) {
      const edited: BrokerQuoteSet = { ...best.brokerQuotes, atmVol: editAtm };
      return calibrateSmile(best.pair, edited, best.conventions, nowNanos());
    }
    return best;
  }, [surface, selTenorYears, editAtm]);

  if (!surface || !selectedSmile) {
    return <div className={styles.loading}>Marking surface…</div>;
  }

  const arb = selectedSmile.arbitrage;

  return (
    <div className={styles.grid}>
      <Panel glyph="◷" title={`Surface · ${surface.pair.base}/${surface.pair.quote}`} className={styles.surfacePanel} noPadding>
        <div className={styles.meshHolder}>
          <SurfaceMesh surface={surface} selected={{ tenorYears: selTenorYears, delta: selDelta ?? 0.5 }} />
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
          {surface.smiles.map((s) => {
            const active = Math.abs(s.tenorYears - selTenorYears) < 1e-9;
            return (
              <button
                key={s.tenorYears}
                className={`${styles.markRow} ${active ? styles.markActive : ""}`}
                onClick={() => {
                  setSelTenorYears(s.tenorYears);
                  setEditAtm(null);
                }}
              >
                <span className={styles.tenorCell}>{tenorName(s.tenorYears)}</span>
                {active ? (
                  <input
                    className={`num ${styles.editCell}`}
                    type="number"
                    step={0.05}
                    value={((editAtm ?? s.brokerQuotes.atmVol) * 100).toFixed(2)}
                    onChange={(e) => setEditAtm(Number(e.target.value) / 100)}
                    onClick={(e) => e.stopPropagation()}
                  />
                ) : (
                  <span className="num">{fmtVol(s.brokerQuotes.atmVol)}</span>
                )}
                <span className="num">{fmtVolPoint(s.brokerQuotes.rr25)}</span>
                <span className="num">{fmtVolPoint(s.brokerQuotes.bf25)}</span>
                <span className="num">{fmtVolPoint(s.brokerQuotes.rr10)}</span>
                <span className="num">{fmtVolPoint(s.brokerQuotes.bf10)}</span>
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
        </div>

        <div className={styles.markActions}>
          <Button variant="secondary" onClick={() => { setEditAtm(null); void app.remarkSurface(); }}>
            Re-mark
          </Button>
          <Button variant="primary" onClick={() => void app.remarkSurface()} disabled={!arb.butterflyArbitrageFree}>
            Publish
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
