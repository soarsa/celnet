/**
 * VolSmile stories — the implied-vol smile slice (visx). Every story is driven by
 * DETERMINISTIC, SEEDED sample data (a mulberry32 PRNG), never invented "live"
 * numbers: the market marks are a smooth parabolic smile (atm + curv·u² − skew·u)
 * plus a tiny seeded residual, so the bid/ask band, the mark scatter and the RR/BF
 * read-out are internally consistent and reproducible. Sample data only.
 *
 * Appearance note: all quantitative colour is the Viridis sequential dataviz ramp
 * (--seq-1..6) — a multi-tenor family maps onto the ramp in tenor order and each
 * tenor's marks/fit/band share that ramp colour (distinguished by glyph, not hue).
 * The signed 25Δ/10Δ risk-reversal is tinted by the diverging palette (put-skew →
 * --div-neg2, call-skew → --div-pos2). Chrome (axes/grid/markers) uses the neutral
 * --text/--grid tokens; brand coral/indigo is never used for data. Headings use
 * --font-display (Space Grotesk), every numeric uses --font-mono (JetBrains Mono).
 */

import type { Meta, StoryObj } from "@storybook/react";
import { VolSmile, type SmileTenor } from "./VolSmile";

/* ─────────────────────────── seeded sample data ────────────────────────── */

/** Deterministic PRNG (mulberry32) — same seed ⇒ same smile every render. */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * The canonical FX delta pillars. `x` is the plotting/wing coordinate (puts left,
 * ATM centre, calls right — 10Δ sits furthest out); `u` is the normalised wing in
 * [-1,1] the smile is evaluated at; `hs` is the market half-spread (wider wings).
 */
const PILLARS = [
  { label: "10ΔP", x: -0.4, u: -1.0, hs: 0.17 },
  { label: "25ΔP", x: -0.25, u: -0.625, hs: 0.12 },
  { label: "35ΔP", x: -0.15, u: -0.375, hs: 0.1 },
  { label: "ATM", x: 0.0, u: 0.0, hs: 0.09 },
  { label: "35ΔC", x: 0.15, u: 0.375, hs: 0.1 },
  { label: "25ΔC", x: 0.25, u: 0.625, hs: 0.12 },
  { label: "10ΔC", x: 0.4, u: 1.0, hs: 0.17 },
] as const;

const round3 = (v: number): number => Math.round(v * 1000) / 1000;

/**
 * Build one tenor's smile from (atm, curvature, skew): the fitted curve is the pure
 * parabola; the market marks add a small seeded residual, and the bid/ask is the
 * mid ± the pillar half-spread. Skew > 0 gives a negative (put-skew) risk-reversal.
 */
function buildTenor(
  tenor: string,
  atm: number,
  curv: number,
  skew: number,
  seed: number,
  focus = false,
): SmileTenor {
  const rnd = mulberry32(seed);
  const fit = (x: number): number => {
    const u = x / 0.4;
    return atm + curv * u * u - skew * u;
  };
  const pillars = PILLARS.map((p) => {
    const clean = atm + curv * p.u * p.u - skew * p.u;
    const mid = round3(clean + (rnd() - 0.5) * 0.09);
    return {
      x: p.x,
      mid,
      label: p.label,
      bidask: [round3(mid - p.hs), round3(mid + p.hs)] as [number, number],
    };
  });
  return { tenor, pillars, fit, ...(focus ? { focus: true } : {}) };
}

/** A single 1M EUR/USD-shaped smile (the base IV-vs-delta case). */
const SINGLE: SmileTenor[] = [buildTenor("1M", 7.85, 1.1, 0.42, 0x1a2b, true)];

/** A 5-tenor family (1W → 1Y), focused on 1M — the classic term-of-smile overlay. */
const FAMILY: SmileTenor[] = [
  buildTenor("1W", 7.2, 1.9, 0.72, 0x11),
  buildTenor("2W", 7.42, 1.4, 0.55, 0x22),
  buildTenor("1M", 7.85, 1.1, 0.42, 0x33, true),
  buildTenor("3M", 8.3, 0.85, 0.33, 0x44),
  buildTenor("1Y", 9.02, 0.55, 0.22, 0x55),
];

/** A risk-off crash smile: a heavy put wing and a large negative risk-reversal. */
const CRASH: SmileTenor[] = [buildTenor("3M", 12.4, 2.4, 1.55, 0x7c0d, true)];

/* ─────────────────────────── meta ──────────────────────────────────────── */

const meta = {
  title: "Viz/VolSmile",
  component: VolSmile,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    docs: {
      description: {
        component:
          "Implied-volatility smile slice (visx): market pillar marks + a bid/ask band + " +
          "a fitted curve, ATM & 25Δ/10Δ RR/BF markers, and an optional multi-tenor family " +
          "overlay. Quantitative colour is the Viridis sequential ramp (--seq-1..6); the " +
          "signed risk-reversal uses the diverging palette. Seeded sample data.",
      },
    },
  },
  decorators: [
    (Story) => (
      <div
        style={{
          maxWidth: 760,
          padding: "var(--space-6)",
          background: "var(--bg-base)",
          borderRadius: "var(--r-lg)",
        }}
      >
        <Story />
      </div>
    ),
  ],
  argTypes: {
    height: { control: { type: "range", min: 180, max: 480, step: 20 } },
    showBand: { control: "boolean", description: "Draw the focused tenor's bid/ask band." },
    interactive: { control: "boolean", description: "Hover tooltips on the focused marks." },
    xUnitLabel: { control: "text" },
  },
  args: {
    tenors: SINGLE,
    height: 280,
    showBand: true,
    interactive: true,
    xUnitLabel: "delta (put ◂ · ▸ call)",
  },
} satisfies Meta<typeof VolSmile>;

export default meta;

type Story = StoryObj<typeof meta>;

/* ─────────────────────────── stories ───────────────────────────────────── */

/**
 * Single 1M smile — the base case: market ● with a bid/ask band, the fitted — curve,
 * ATM/25Δ/10Δ reference markers and the derived RR/BF read-out below.
 */
export const SingleTenor: Story = {
  args: { tenors: SINGLE },
};

/**
 * Multi-tenor family (1W → 1Y) — the term of the smile overlaid on shared axes so
 * curves stay comparable. Tenors map onto the Viridis ramp in order; the focused 1M
 * is drawn bold with marks + band, the rest are thin context curves.
 */
export const TenorFamily: Story = {
  args: { tenors: FAMILY, height: 320 },
};

/**
 * Crash / risk-off smile — a heavy put wing and a large negative 25Δ/10Δ risk-
 * reversal, so the diverging RR tint sits at the --div-neg2 pole. Band widened by
 * the fatter wing spreads.
 */
export const CrashSkew: Story = {
  args: { tenors: CRASH },
};
