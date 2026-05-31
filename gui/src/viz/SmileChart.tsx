/**
 * SmileChart — the per-tenor 2D smile overlay (GUI-DESIGN §4.3). Canvas 2D,
 * sharp and cheap. Plots vol vs the delta axis (10P 25P ATM 25C 10C), marks the
 * selected point, and cross-highlights with the 3D mesh and marking grid. Click
 * a point to select it (provenance opens in the Inspector).
 */

import { useEffect, useRef } from "react";
import type { Smile } from "../data/contract";
import { fmtVol } from "../lib/format";
import styles from "./SmileChart.module.css";

export interface SmileChartProps {
  smile: Smile;
  selectedDelta?: number | null;
  onSelect?: (delta: number) => void;
  height?: number;
}

const AXIS_LABELS = ["10P", "25P", "ATM", "25C", "10C"];

export function SmileChart({
  smile,
  selectedDelta,
  onSelect,
  height = 150,
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
    let vMin = Infinity;
    let vMax = -Infinity;
    for (const p of pts) {
      if (p.vol < vMin) vMin = p.vol;
      if (p.vol > vMax) vMax = p.vol;
    }
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

    // Horizontal gridlines.
    ctx.strokeStyle = line;
    ctx.lineWidth = 0.5;
    for (let g = 0; g <= 2; g += 1) {
      const gy = pad + (innerH * g) / 2;
      ctx.beginPath();
      ctx.moveTo(padX, gy);
      ctx.lineTo(w - padX, gy);
      ctx.stroke();
    }

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

    // Points + selection.
    pts.forEach((p, i) => {
      const selected = selectedDelta !== null && selectedDelta !== undefined && Math.abs(p.delta - selectedDelta) < 1e-6;
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
  }, [smile, selectedDelta, height]);

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
        <span className={styles.title}>smile · {tenorName(smile.tenorYears)}</span>
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

function tenorName(years: number): string {
  const days = Math.round(years * 365);
  if (days <= 1) return "ON";
  if (days < 28) return `${Math.round(days / 7)}W`;
  if (days < 360) return `${Math.round(days / 30)}M`;
  return `${Math.round(days / 365)}Y`;
}
