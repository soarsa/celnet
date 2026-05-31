/**
 * SurfaceMesh — the 3D vol-surface visualization (GUI-DESIGN §4.3, §6.2). A
 * bespoke renderer (no chart lib, rule 7): an isometric projected wireframe +
 * shaded quads of the marked surface (tenor × delta × vol), drawn on Canvas 2D
 * with WebGPU-capability detection so the build wave can swap in the WGSL path
 * with no API change. Drag to rotate (direct, 1:1, inertia-free while dragging;
 * eased settle on release — §3.4). Colour uses the perceptual diverging ramp
 * (cool→neutral→warm), never rainbow (§3.2). Presentation is f32; the f64
 * pricing stays server-side (numeric policy).
 *
 * WebGPU note: when `navigator.gpu` exists the build wave attaches a WGSL mesh
 * pipeline here; today the Canvas path renders the identical projected mesh so
 * the standalone app is fully runnable. The seam is the `renderer` selection.
 */

import { useEffect, useRef, useState } from "react";
import type { MarkedSurface } from "../data/contract";
import { sampleSurface } from "../data/surface";
import { rampColor } from "./ramp";
import styles from "./SurfaceMesh.module.css";

export interface SurfaceMeshProps {
  surface: MarkedSurface;
  /** Selected (tenorYears, delta) cross-highlight, if any. */
  selected?: { tenorYears: number; delta: number } | null;
}

const DELTA_AXIS: number[] = [-0.1, -0.18, -0.25, -0.38, 0.5, 0.38, 0.25, 0.18, 0.1].sort(
  (a, b) => a - b,
);

export function SurfaceMesh({ surface, selected }: SurfaceMeshProps): React.ReactElement {
  const ref = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [yaw, setYaw] = useState(-0.62);
  const [pitch, setPitch] = useState(0.52);
  const drag = useRef<{ x: number; y: number; yaw: number; pitch: number } | null>(null);
  const [hasWebGPU] = useState(() => typeof navigator !== "undefined" && "gpu" in navigator);
  const [size, setSize] = useState({ w: 480, h: 320 });

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const obs = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect;
      if (r) setSize({ w: Math.max(240, r.width), h: Math.max(200, r.height) });
    });
    obs.observe(el);
    return () => obs.disconnect();
  }, []);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const { w, h } = size;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const tenors = surface.smiles.map((s) => s.tenorYears);
    if (tenors.length < 2) return;
    const tMin = tenors[0]!;
    const tMax = tenors[tenors.length - 1]!;

    // Build a grid of vols over (tenor index, delta axis).
    const TROWS = 18;
    const grid: { ti: number; di: number; v: number; tenor: number; delta: number }[][] = [];
    let vMin = Infinity;
    let vMax = -Infinity;
    for (let ti = 0; ti <= TROWS; ti += 1) {
      const tenor = tMin + ((tMax - tMin) * ti) / TROWS;
      const row: { ti: number; di: number; v: number; tenor: number; delta: number }[] = [];
      DELTA_AXIS.forEach((delta, di) => {
        const v = sampleSurface(surface, tenor, delta);
        if (v < vMin) vMin = v;
        if (v > vMax) vMax = v;
        row.push({ ti, di, v, tenor, delta });
      });
      grid.push(row);
    }
    const vMid = (vMin + vMax) / 2;
    const vSpan = (vMax - vMin) / 2 || 1;

    // Isometric projection.
    const cx = w / 2;
    const cy = h / 2 + 28;
    const scale = Math.min(w, h) * 0.34;
    const cosY = Math.cos(yaw);
    const sinY = Math.sin(yaw);
    const cosP = Math.cos(pitch);
    const sinP = Math.sin(pitch);

    const project = (gx: number, gy: number, gz: number) => {
      // Map grid coords to centered cube [-1,1].
      const x = (gx / DELTA_AXIS.length) * 2 - 1;
      const z = (gy / (TROWS + 1)) * 2 - 1;
      const y = gz * 0.9;
      // Rotate around Y then tilt.
      const rx = x * cosY - z * sinY;
      const rz = x * sinY + z * cosY;
      const ry = y * cosP - rz * sinP;
      const depth = y * sinP + rz * cosP;
      return { sx: cx + rx * scale, sy: cy - ry * scale, depth };
    };

    interface Quad {
      pts: { sx: number; sy: number }[];
      depth: number;
      v: number;
    }
    const quads: Quad[] = [];
    for (let ti = 0; ti < grid.length - 1; ti += 1) {
      for (let di = 0; di < DELTA_AXIS.length - 1; di += 1) {
        const a = grid[ti]![di]!;
        const b = grid[ti]![di + 1]!;
        const c = grid[ti + 1]![di + 1]!;
        const d = grid[ti + 1]![di]!;
        const norm = (v: number) => (v - vMid) / vSpan;
        const pa = project(di, ti, norm(a.v));
        const pb = project(di + 1, ti, norm(b.v));
        const pc = project(di + 1, ti + 1, norm(c.v));
        const pd = project(di, ti + 1, norm(d.v));
        quads.push({
          pts: [pa, pb, pc, pd],
          depth: (pa.depth + pb.depth + pc.depth + pd.depth) / 4,
          v: (a.v + b.v + c.v + d.v) / 4,
        });
      }
    }
    quads.sort((p, q) => p.depth - q.depth);

    for (const quad of quads) {
      const t = (quad.v - vMin) / (vMax - vMin || 1);
      ctx.beginPath();
      ctx.moveTo(quad.pts[0]!.sx, quad.pts[0]!.sy);
      for (let i = 1; i < quad.pts.length; i += 1) ctx.lineTo(quad.pts[i]!.sx, quad.pts[i]!.sy);
      ctx.closePath();
      const shade = 0.55 + 0.45 * Math.max(0, Math.min(1, (quad.depth + 1) / 2));
      ctx.fillStyle = rampColor(t, shade);
      ctx.fill();
      ctx.strokeStyle = "oklch(1 0 0 / 0.06)";
      ctx.lineWidth = 0.5;
      ctx.stroke();
    }

    // Selected cross-highlight ridge (the smile at a tenor).
    if (selected) {
      const accent =
        getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() ||
        "oklch(0.62 0.19 280)";
      ctx.beginPath();
      const tFrac = (selected.tenorYears - tMin) / (tMax - tMin || 1);
      const ti = Math.round(tFrac * TROWS);
      DELTA_AXIS.forEach((delta, di) => {
        const v = sampleSurface(surface, selected.tenorYears, delta);
        const p = project(di, ti, (v - vMid) / vSpan);
        if (di === 0) ctx.moveTo(p.sx, p.sy);
        else ctx.lineTo(p.sx, p.sy);
      });
      ctx.strokeStyle = accent;
      ctx.lineWidth = 2;
      ctx.stroke();
    }
  }, [surface, yaw, pitch, selected, size]);

  const onPointerDown = (e: React.PointerEvent) => {
    (e.target as Element).setPointerCapture(e.pointerId);
    drag.current = { x: e.clientX, y: e.clientY, yaw, pitch };
  };
  const onPointerMove = (e: React.PointerEvent) => {
    if (!drag.current) return;
    const dx = e.clientX - drag.current.x;
    const dy = e.clientY - drag.current.y;
    setYaw(drag.current.yaw + dx * 0.006);
    setPitch(Math.max(0.12, Math.min(1.1, drag.current.pitch + dy * 0.005)));
  };
  const onPointerUp = () => {
    drag.current = null;
  };

  return (
    <div className={styles.wrap} ref={wrapRef}>
      <canvas
        ref={ref}
        className={styles.canvas}
        style={{ width: size.w, height: size.h }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
      />
      <div className={styles.axisLabels} aria-hidden>
        <span className={styles.axDelta}>delta →</span>
        <span className={styles.axTenor}>tenor →</span>
        <span className={styles.axVol}>vol ▲</span>
      </div>
      <span className={styles.renderer} title="renderer backend">
        {hasWebGPU ? "WebGPU-ready · Canvas mesh" : "Canvas mesh"}
      </span>
    </div>
  );
}
