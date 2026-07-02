/**
 * VolSurface3D stories — the signature 3D implied-vol surface (mockup 01). Every
 * story uses ONE deterministic, seeded sample surface (7 tenors × 7 delta pillars)
 * built from a fixed ATM / curvature / skew term-structure — clearly SAMPLE data,
 * never invented "live" marks. The stories exercise the three render modes and the
 * arbitrage-flag highlight, and prove the component against the real Aurora token
 * cascade in both appearances (flip Appearance in the toolbar).
 *
 * Appearance note: quantitative colour is the Viridis sequential ramp (`--seq-1..6`),
 * resolved from the live `oklch()` tokens at mount (WebGL can't read CSS vars), so
 * the surface re-tints correctly when you switch Dark ↔ Light / Increased-Contrast.
 * Drag to orbit, scroll to zoom. Under `prefers-reduced-motion` the damped inertia
 * is disabled and frames are drawn on demand.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { VolSurface3D } from "./VolSurface3D";
import type { VolSurfaceMarker } from "./VolSurface3D";

const TENORS = ["1W", "2W", "1M", "3M", "6M", "1Y", "2Y"] as const;
const DELTAS = ["10ΔP", "25ΔP", "35ΔP", "ATM", "35ΔC", "25ΔC", "10ΔC"] as const;

// Seeded smile term-structure (matches the mockup's shared smile generator):
// vol(t, δ) = atm[t] + curv[t]·k² − skew[t]·k, with k the [-1,1] delta coordinate.
const ATM = [7.2, 7.42, 7.85, 8.3, 8.66, 9.02, 9.28] as const;
const CURV = [1.9, 1.4, 1.1, 0.85, 0.68, 0.55, 0.46] as const;
const SKEW = [0.72, 0.55, 0.42, 0.33, 0.27, 0.22, 0.19] as const;

function buildSampleVols(): number[][] {
  const nD = DELTAS.length;
  return TENORS.map((_tenor, ti) => {
    const atm = ATM[ti]!;
    const curv = CURV[ti]!;
    const skew = SKEW[ti]!;
    return DELTAS.map((_delta, di) => {
      const k = -1 + (2 * di) / (nD - 1);
      return Math.round((atm + curv * k * k - skew * k) * 1000) / 1000;
    });
  });
}

const SAMPLE_VOLS = buildSampleVols();

// Ground-truth arb flag from the mockup: the 1W (tenor 0) put wings carry the
// biggest positive residual → butterfly < 0 (negative risk-neutral density).
const ARB_MARKERS: VolSurfaceMarker[] = [
  { tenorIndex: 0, deltaIndex: 0, reason: "1W 10ΔP — butterfly < 0 (implied density negative)" },
  { tenorIndex: 0, deltaIndex: 1, reason: "1W 25ΔP — butterfly < 0 (implied density negative)" },
];

const meta = {
  title: "Viz/VolSurface3D",
  component: VolSurface3D,
  tags: ["autodocs"],
  parameters: {
    layout: "centered",
    docs: {
      description: {
        component:
          "Rotatable WebGL implied-vol surface: X = tenor, Z = delta, Y (height) + Viridis colour = implied vol. " +
          "Sample data — a seeded 7×7 smile term-structure, not live marks.",
      },
    },
  },
  argTypes: {
    mode: {
      control: "inline-radio",
      options: ["shaded", "wireframe", "points"],
      description: "Shaded mesh | wireframe | point cloud.",
    },
    heightGain: {
      control: { type: "number", min: 0.2, max: 3, step: 0.1 },
      description: "Display-only vertical exaggeration of the vol height (colour unaffected).",
    },
    width: { control: { type: "number", min: 280, max: 900, step: 20 } },
    height: { control: { type: "number", min: 220, max: 700, step: 20 } },
  },
  args: {
    vols: SAMPLE_VOLS,
    tenors: [...TENORS],
    deltas: [...DELTAS],
    markers: ARB_MARKERS,
    mode: "shaded",
    width: 560,
    height: 400,
    heightGain: 1,
  },
} satisfies Meta<typeof VolSurface3D>;

export default meta;

type Story = StoryObj<typeof meta>;

/**
 * Shaded — the signature lit mesh with the 1W arb wings flagged in `--danger`.
 * Drag to orbit, scroll to zoom.
 */
export const Shaded: Story = {};

/** Wireframe — the tenor×delta lattice; the shaded mesh dims to a faint ghost underneath. */
export const Wireframe: Story = {
  args: { mode: "wireframe" },
};

/**
 * Point cloud — each marked vertex as a Viridis-coloured point, the surface
 * receding to a whisper. Shown with no arb flags and a taller height gain to read
 * the term-structure relief.
 */
export const PointCloud: Story = {
  args: { mode: "points", markers: [], heightGain: 1.4 },
};
