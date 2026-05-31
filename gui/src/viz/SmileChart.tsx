/**
 * SmileChart — the per-tenor 2D smile overlay (GUI-DESIGN §4.3). Canvas 2D,
 * sharp and cheap. Plots vol vs the delta axis (10P 25P ATM 25C 10C), marks the
 * selected point, and cross-highlights with the 3D mesh and marking grid. Click
 * a point to select it (provenance opens in the Inspector).
 */

import { useEffect, useRef } from "react";
import type { Smile } from "../data/contract";
import { fmtVol } from "../lib/format";
import { tenorLabel } from "../lib/trend";
import styles from "./SmileChart.module.css";

export interface SmileChartProps {
  smile: Smile;
  selectedDelta?: number | null;
  onSelect?: (delta: number) => void;
  height?: number;
  /**
   * A STABLE vertical vol band the smile is drawn against — supply the
   * surface-wide [min,max] vol so smiles stay visually comparable across tenors
   * and across in-place edits, instead of the axis re-fitting to each smile (which
   * makes a single tenor's curve jump misleadingly on every edit/tenor switch).
   * When omitted, a sane fixed band derived from the smile's own ATM is used so a
   * lone chart still renders honestly.
   */
  volRange?: { min: number; max: number } | undefined;
}

const AXIS_LABELS = ["10P", "25P", "ATM", "25C", "10C"];

/**
 * Snap two deltas onto a stable comparison key. Convention deltas live on a coarse
 * pillar grid (0.10/0.25/0.50…), so rounding to 1e-4 gives an exact integer key —
 * robust selection without brittle absolute float-equality windows that can
 * mis-select a neighbouring pillar after recalibration.
 */
const deltaKey = (d: number): number => Math.round(d * 1e4);

export function SmileChart({
  smile,
  selectedDelta,
  onSelect,
  height = 150,
  volRange,
}: SmileChartProps): React.ReactElement {
  const ref = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const pointsRef = useRef<{ x: number; delta: number }[]>([]);

  useEffect(() => {
    const canvas = ref.current;
    const wrap = wrapRef.current;
    if (!canvas || !wrap) return;
    const w = wrap.clientWidth || 360;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(height * dpr);
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, height);

    // Order points along the delta axis for a smooth smile (puts→ATM→calls).
    const pts = [...smile.points].sort((a, b) => orderKey(a.delta) - orderKey(b.delta));
    if (pts.length < 2) return;

    // STABLE vertical scale (P0-9): the axis does NOT auto-fit to this smile's own
    // min/max — that makes the curve jump on every edit/tenor switch and defeats
    // visual comparison. Use the surface-wide band when provided; otherwise a sane
    // fixed band centred on this smile's ATM. A small symmetric pad keeps the curve
    // off the frame without rescaling per render.
    let loV: number;
    let hiV: number;
    if (volRange && Number.isFinite(volRange.min) && Number.isFinite(volRange.max) && volRange.max > volRange.min) {
      loV = volRange.min;
      hiV = volRange.max;
    } else {
      const atm = smile.brokerQuotes.atmVol;
      loV = Math.max(0, atm - 0.05);
      hiV = atm + 0.05;
    }
    // Pad the fixed band by 8% of its span so the extreme wings sit inside the frame.
    const rawSpan = hiV - loV || 0.001;
    const padV = rawSpan * 0.08;
    const vMin = loV - padV;
    const vMax = hiV + padV;

    const pad = 18;
    const padX = 26;
    const innerW = w - padX * 2;
    const innerH = height - pad * 2;
    const span = vMax - vMin || 0.001;
    const x = (i: number) => padX + (innerW * i) / (pts.length - 1);
    const y = (v: number) => pad + innerH - ((v - vMin) / span) * innerH;

    pointsRef.current = pts.map((p, i) => ({ x: x(i), delta: p.delta }));

    const root = getComputedStyle(document.documentElement);
    const accent = root.getPropertyValue("--accent").trim() || "oklch(0.62 0.19 280)";
    const line = root.getPropertyValue("--grid-line").trim() || "rgba(255,255,255,0.06)";
    const text = root.getPropertyValue("--text-tertiary").trim() || "#888";

    // Horizontal gridlines + a labelled vol axis (P0-9: the axis is now explicit
    // and stable, so the % ticks read the same band across tenors/edits).
    ctx.strokeStyle = line;
    ctx.lineWidth = 0.5;
    ctx.fillStyle = text;
    ctx.font = "9px ui-monospace, monospace";
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    for (let g = 0; g <= 2; g += 1) {
      const gy = pad + (innerH * g) / 2;
      ctx.beginPath();
      ctx.moveTo(padX, gy);
      ctx.lineTo(w - padX, gy);
      ctx.stroke();
      // Tick value at this gridline (top = vMax, bottom = vMin).
      const tickVol = vMax - ((vMax - vMin) * g) / 2;
      ctx.fillText(`${(tickVol * 100).toFixed(1)}`, 2, gy);
    }
    ctx.textBaseline = "alphabetic";

    // Smile curve (Catmull-Rom-ish smoothing via quadratic midpoints).
    ctx.beginPath();
    ctx.moveTo(x(0), y(pts[0]!.vol));
    for (let i = 1; i < pts.length; i += 1) {
      const xm = (x(i - 1) + x(i)) / 2;
      const ym = (y(pts[i - 1]!.vol) + y(pts[i]!.vol)) / 2;
      ctx.quadraticCurveTo(x(i - 1), y(pts[i - 1]!.vol), xm, ym);
    }
    ctx.lineTo(x(pts.length - 1), y(pts[pts.length - 1]!.vol));
    ctx.strokeStyle = accent;
    ctx.lineWidth = 1.75;
    ctx.lineJoin = "round";
    ctx.stroke();

    // Points + selection. Match on the snapped delta key (robust against float
    // drift after recalibration), never a raw absolute-equality window.
    const selKey = selectedDelta !== null && selectedDelta !== undefined ? deltaKey(selectedDelta) : null;
    pts.forEach((p, i) => {
      const selected = selKey !== null && deltaKey(p.delta) === selKey;
      ctx.beginPath();
      ctx.arc(x(i), y(p.vol), selected ? 4.5 : 2.6, 0, Math.PI * 2);
      ctx.fillStyle = selected ? accent : "oklch(0.8 0.02 264)";
      ctx.fill();
      if (selected) {
        ctx.strokeStyle = accent;
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        ctx.arc(x(i), y(p.vol), 8, 0, Math.PI * 2);
        ctx.globalAlpha = 0.4;
        ctx.stroke();
        ctx.globalAlpha = 1;
      }
    });

    // Axis labels (10P 25P ATM 25C 10C) at the canonical positions.
    ctx.fillStyle = text;
    ctx.font = "10px ui-monospace, monospace";
    ctx.textAlign = "center";
    const labelPositions = [0, 1, 2, 3, 4];
    labelPositions.forEach((li, idx) => {
      const px = padX + (innerW * li) / 4;
      ctx.fillText(AXIS_LABELS[idx]!, px, height - 4);
    });
  }, [smile, selectedDelta, height, volRange]);

  const onClick = (e: React.MouseEvent) => {
    if (!onSelect) return;
    const rect = ref.current!.getBoundingClientRect();
    const px = e.clientX - rect.left;
    let best = pointsRef.current[0];
    let bestD = Infinity;
    for (const p of pointsRef.current) {
      const d = Math.abs(p.x - px);
      if (d < bestD) {
        bestD = d;
        best = p;
      }
    }
    if (best) onSelect(best.delta);
  };

  return (
    <div className={styles.wrap} ref={wrapRef}>
      <div className={styles.head}>
        <span className={styles.title}>smile · {tenorLabel(smile.tenorYears)}</span>
        <span className={`${styles.axisUnit}`}>vol %</span>
        <span className={`num ${styles.atm}`}>ATM {fmtVol(smile.brokerQuotes.atmVol)}</span>
      </div>
      <canvas ref={ref} className={styles.canvas} style={{ height }} onClick={onClick} />
    </div>
  );
}

/** Order key so puts (negative delta) sit left, ATM center, calls right. */
function orderKey(delta: number): number {
  if (Math.abs(delta) >= 0.49) return 0; // ATM center
  return delta < 0 ? delta - 1 : delta + 1;
}
