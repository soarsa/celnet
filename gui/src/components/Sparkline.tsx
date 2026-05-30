/**
 * Sparkline — a cheap, sharp Canvas 2D mid-history trace (GUI-DESIGN §3.5,
 * §6.2). Hand-rolled (no chart lib, rule 7). Device-pixel-ratio aware for crisp
 * lines; draws an area gradient tinted by net direction. Recomputes only on data
 * change, never per animation frame, to respect the render budget.
 */

import { useEffect, useRef } from "react";
import styles from "./Sparkline.module.css";

export interface SparklineProps {
  values: number[];
  width?: number;
  height?: number;
}

export function Sparkline({
  values,
  width = 96,
  height = 22,
}: SparklineProps): React.ReactElement {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, width, height);

    if (values.length < 2) return;
    let min = Infinity;
    let max = -Infinity;
    for (const v of values) {
      if (v < min) min = v;
      if (v > max) max = v;
    }
    const span = max - min || 1;
    const pad = 2;
    const innerH = height - pad * 2;
    const stepX = width / (values.length - 1);
    const x = (i: number) => i * stepX;
    const y = (v: number) => pad + innerH - ((v - min) / span) * innerH;

    const up = (values[values.length - 1] ?? 0) >= (values[0] ?? 0);
    const root = getComputedStyle(document.documentElement);
    const stroke = root.getPropertyValue(up ? "--bid" : "--offer").trim() || "#5ad1a0";

    // Area fill (subtle, gradient to transparent).
    ctx.beginPath();
    ctx.moveTo(x(0), y(values[0]!));
    for (let i = 1; i < values.length; i += 1) ctx.lineTo(x(i), y(values[i]!));
    ctx.lineTo(x(values.length - 1), height);
    ctx.lineTo(x(0), height);
    ctx.closePath();
    const grad = ctx.createLinearGradient(0, 0, 0, height);
    grad.addColorStop(0, hexWithAlpha(stroke, 0.22));
    grad.addColorStop(1, hexWithAlpha(stroke, 0));
    ctx.fillStyle = grad;
    ctx.fill();

    // Line.
    ctx.beginPath();
    ctx.moveTo(x(0), y(values[0]!));
    for (let i = 1; i < values.length; i += 1) ctx.lineTo(x(i), y(values[i]!));
    ctx.strokeStyle = stroke;
    ctx.lineWidth = 1.25;
    ctx.lineJoin = "round";
    ctx.stroke();

    // Endpoint dot.
    ctx.beginPath();
    ctx.arc(x(values.length - 1), y(values[values.length - 1]!), 1.6, 0, Math.PI * 2);
    ctx.fillStyle = stroke;
    ctx.fill();
  }, [values, width, height]);

  return <canvas ref={ref} className={styles.canvas} style={{ width, height }} />;
}

/** Apply an alpha to an oklch()/hex color string for the area gradient. */
function hexWithAlpha(color: string, alpha: number): string {
  if (color.startsWith("oklch")) {
    return color.replace(/\)$/, ` / ${alpha})`);
  }
  return color;
}
