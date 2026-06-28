/**
 * CurveChart — a shared-axis term-structure line chart (FI-ARCHITECTURE §4.2).
 * Canvas 2D, sharp and cheap, in the same hand-rolled idiom as `SmileChart`
 * (no chart library, CLAUDE.md rule 7): it plots one or more value-vs-time series
 * on a SHARED time x-axis and a SHARED value y-axis (so overlaid series stay
 * visually comparable), with labelled gridlines, a tone-coded legend, and an
 * optional vertical horizon marker. Device-pixel-ratio aware for crisp lines;
 * recomputes only on data change, never per animation frame.
 *
 * It is deliberately generic over what it plots — the Curve workspace draws the
 * discount factor `DF(t)` in one instance and the zero rate `z(t)` + instantaneous
 * forward `f(t)` overlaid in another — so the axis/scale logic lives once here.
 */

import { useEffect, useRef } from "react";
import styles from "./CurveChart.module.css";

/** A CSS design-token tone a series is drawn in (resolved at paint time). */
export type CurveTone = "accent" | "bid" | "offer" | "neutral";

/** The CSS custom property each tone reads, with a safe literal fallback. */
const TONE_VAR: Record<CurveTone, { token: string; fallback: string }> = {
  accent: { token: "--accent", fallback: "oklch(0.62 0.19 280)" },
  bid: { token: "--bid", fallback: "#5ad1a0" },
  offer: { token: "--offer", fallback: "#e0668a" },
  neutral: { token: "--text-tertiary", fallback: "#888" },
};

/** One plotted series: ascending `(x, y)` points drawn in `tone`. */
export interface CurveSeries {
  readonly label: string;
  readonly tone: CurveTone;
  readonly points: ReadonlyArray<{ x: number; y: number }>;
}

export interface CurveChartProps {
  /** Series sharing ONE x- and y-range (so overlaid curves are comparable). */
  series: readonly CurveSeries[];
  /** Canvas height in CSS px (width fills the container). */
  height?: number;
  /** Caption for the x-axis (e.g. "tenor (years)"). */
  xLabel?: string;
  /** Format a y value for the axis ticks. */
  formatY?: (v: number) => string;
  /** Format an x value for the axis ticks. */
  formatX?: (v: number) => string;
  /** A vertical reference line at this x (the inspected horizon); `null` to omit. */
  markerX?: number | null;
}

const DEFAULT_FORMAT = (v: number): string => v.toFixed(2);

export function CurveChart({
  series,
  height = 180,
  xLabel,
  formatY = DEFAULT_FORMAT,
  formatX = DEFAULT_FORMAT,
  markerX = null,
}: CurveChartProps): React.ReactElement {
  const ref = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);

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

    // Shared data extent across every series (overlaid curves stay comparable).
    let xMin = Infinity;
    let xMax = -Infinity;
    let yMin = Infinity;
    let yMax = -Infinity;
    for (const s of series) {
      for (const p of s.points) {
        if (p.x < xMin) xMin = p.x;
        if (p.x > xMax) xMax = p.x;
        if (p.y < yMin) yMin = p.y;
        if (p.y > yMax) yMax = p.y;
      }
    }
    if (!Number.isFinite(xMin) || !Number.isFinite(yMin) || xMax <= xMin) return;

    // Pad the value band by 8% so the extreme points sit inside the frame.
    const rawSpan = yMax - yMin || Math.abs(yMax) || 1;
    const padY = rawSpan * 0.08;
    const vMin = yMin - padY;
    const vMax = yMax + padY;

    const padTop = 14;
    const padBottom = 26;
    const padLeft = 48;
    const padRight = 14;
    const innerW = w - padLeft - padRight;
    const innerH = height - padTop - padBottom;
    const xSpan = xMax - xMin || 1;
    const ySpan = vMax - vMin || 1;
    const px = (x: number): number => padLeft + (innerW * (x - xMin)) / xSpan;
    const py = (y: number): number => padTop + innerH - (innerH * (y - vMin)) / ySpan;

    const root = getComputedStyle(document.documentElement);
    const tone = (t: CurveTone): string =>
      root.getPropertyValue(TONE_VAR[t].token).trim() || TONE_VAR[t].fallback;
    const gridLine = root.getPropertyValue("--grid-line").trim() || "rgba(255,255,255,0.06)";
    const text = root.getPropertyValue("--text-tertiary").trim() || "#888";

    // Horizontal gridlines + a labelled value axis (top = vMax, bottom = vMin).
    ctx.strokeStyle = gridLine;
    ctx.lineWidth = 0.5;
    ctx.fillStyle = text;
    ctx.font = "9px ui-monospace, monospace";
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    const ticks = 4;
    for (let g = 0; g <= ticks; g += 1) {
      const gy = padTop + (innerH * g) / ticks;
      ctx.beginPath();
      ctx.moveTo(padLeft, gy);
      ctx.lineTo(w - padRight, gy);
      ctx.stroke();
      const tickVal = vMax - ((vMax - vMin) * g) / ticks;
      ctx.fillText(formatY(tickVal), padLeft - 6, gy);
    }

    // x-axis ticks (start / mid / end of the span).
    ctx.textAlign = "center";
    ctx.textBaseline = "alphabetic";
    const xTicks = [xMin, xMin + xSpan / 2, xMax];
    for (const xt of xTicks) {
      ctx.fillText(formatX(xt), px(xt), height - 8);
    }

    // The inspected-horizon marker — a faint vertical reference line.
    if (markerX !== null && markerX >= xMin && markerX <= xMax) {
      ctx.strokeStyle = tone("accent");
      ctx.globalAlpha = 0.35;
      ctx.setLineDash([3, 3]);
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(px(markerX), padTop);
      ctx.lineTo(px(markerX), padTop + innerH);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.globalAlpha = 1;
    }

    // Each series as a polyline in its tone.
    for (const s of series) {
      const pts = [...s.points].sort((a, b) => a.x - b.x);
      if (pts.length < 2) continue;
      ctx.beginPath();
      ctx.moveTo(px(pts[0]!.x), py(pts[0]!.y));
      for (let i = 1; i < pts.length; i += 1) ctx.lineTo(px(pts[i]!.x), py(pts[i]!.y));
      ctx.strokeStyle = tone(s.tone);
      ctx.lineWidth = 1.75;
      ctx.lineJoin = "round";
      ctx.stroke();
    }
  }, [series, height, xLabel, formatY, formatX, markerX]);

  return (
    <div className={styles.wrap} ref={wrapRef}>
      <div className={styles.head}>
        {series.map((s) => (
          <span key={s.label} className={styles.legend}>
            <span className={`${styles.swatch} ${styles[s.tone]}`} aria-hidden />
            {s.label}
          </span>
        ))}
        {xLabel && <span className={styles.xLabel}>{xLabel}</span>}
      </div>
      <canvas ref={ref} className={styles.canvas} style={{ height }} />
    </div>
  );
}
