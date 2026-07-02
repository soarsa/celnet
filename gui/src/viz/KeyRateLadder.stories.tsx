/**
 * KeyRateLadder stories — the signed key-rate DV01 pillar ladder (visx bar chart).
 *
 * Appearance note: colour comes ONLY from the diverging `--div` dataviz ramp
 * centred at 0 (cool `--div-neg2/-neg1` for negative rungs, warm `--div-pos1/-pos2`
 * for positive), never the brand coral/indigo — brand hues must not encode a
 * quantitative scale. Numerics render in `--font-mono`, the title in `--font-display`.
 * All figures below are DETERMINISTIC SAMPLE data (a fixed hand-set ladder or a
 * seeded generator), NOT live marks. The Σ row proves additivity: it reconciles the
 * ladder total to the supplied parallel DV01 and, when they disagree, surfaces the
 * residual in `--warn` rather than hiding it. The mount fade honours
 * prefers-reduced-motion (toggle the OS setting to see it drop to a static render).
 */

import type { Meta, StoryObj } from "@storybook/react";
import { KeyRateLadder, type KeyRatePillar } from "./KeyRateLadder";

/**
 * A USD-SOFR 5Y OIS receiver ladder (SAMPLE). Receiving fixed loses as the curve
 * sells off, so the belly pillars carry the large negative key-rate DV01; the wings
 * leak small positive amounts. Σ = −45,000 USD/bp, reconciling to the parallel DV01.
 */
const SAMPLE_5Y: KeyRatePillar[] = [
  { pillar: "1Y", dv01: -2100 },
  { pillar: "2Y", dv01: -5400 },
  { pillar: "3Y", dv01: -9800 },
  { pillar: "5Y", dv01: -29600 },
  { pillar: "7Y", dv01: 1300 },
  { pillar: "10Y", dv01: 600 },
];
const SAMPLE_5Y_PARALLEL = SAMPLE_5Y.reduce((acc, d) => acc + d.dv01, 0);

/** Deterministic PRNG (mulberry32) — a fixed seed makes the sample ladders stable. */
function mulberry32(seed: number): () => number {
  let s = seed >>> 0;
  return () => {
    s = (s + 0x6d2b79f5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Nominal shape of a 10Y receiver's key-rate Jacobian across a full pillar set. */
const NOMINAL_30Y: ReadonlyArray<readonly [string, number]> = [
  ["1Y", -900],
  ["2Y", -1800],
  ["3Y", -3200],
  ["5Y", -7400],
  ["7Y", -12600],
  ["10Y", -41800],
  ["15Y", 2100],
  ["20Y", 1200],
  ["30Y", 500],
];

/** Build a seeded SAMPLE ladder by jittering the nominal shape ±8% deterministically. */
function seededLadder(seed: number): KeyRatePillar[] {
  const rnd = mulberry32(seed);
  return NOMINAL_30Y.map(([pillar, base]) => ({
    pillar,
    dv01: Math.round(base * (0.92 + rnd() * 0.16)),
  }));
}

const FULL_30Y = seededLadder(0x5eed);
const FULL_30Y_PARALLEL = FULL_30Y.reduce((acc, d) => acc + d.dv01, 0);

const meta = {
  title: "Viz/KeyRateLadder",
  component: KeyRateLadder,
  tags: ["autodocs"],
  parameters: {
    docs: {
      description: {
        component:
          "Signed key-rate DV01 pillar ladder — FI risk drawn as a pillar Jacobian, " +
          "not a Greek vector. Bars are coloured by the diverging --div ramp centred " +
          "at 0 (never brand hues); the Σ row reconciles the ladder to the parallel " +
          "DV01 and flags any residual in --warn. All data is deterministic SAMPLE " +
          "data (fixed or seeded), never live marks. Honours prefers-reduced-motion.",
      },
    },
  },
  argTypes: {
    parallelDv01: {
      control: { type: "number" },
      description: "Independently-computed parallel DV01 the ladder Σ reconciles to.",
    },
    unit: { control: "text", description: "Currency-per-bp unit label." },
    width: { control: { type: "number" } },
    height: { control: { type: "number" } },
    data: { control: false },
  },
  args: {
    data: SAMPLE_5Y,
    parallelDv01: SAMPLE_5Y_PARALLEL,
    unit: "USD/bp",
    width: 360,
    height: 200,
  },
} satisfies Meta<typeof KeyRateLadder>;

export default meta;

type Story = StoryObj<typeof meta>;

/**
 * USD-SOFR 5Y OIS receiver (SAMPLE). The belly (5Y) dominates in cool `--div-neg2`;
 * the small positive wings sit in warm `--div-pos1`. Σ reconciles to the parallel DV01.
 */
export const Default: Story = {};

/**
 * A full 1Y→30Y pillar set (SEEDED SAMPLE) for a 10Y receiver — many rungs, the 10Y
 * belly saturating `--div-neg2`. Parallel DV01 is set to the seeded ladder's own
 * total, so the Σ row shows a clean reconciliation across the wider grid.
 */
export const FullCurve30Y: Story = {
  args: {
    data: FULL_30Y,
    parallelDv01: FULL_30Y_PARALLEL,
    unit: "USD/bp",
    width: 440,
    height: 220,
  },
};

/**
 * Honest reconciliation failure — the same 5Y ladder against a parallel DV01 that it
 * does NOT sum to (an unhedged basis / re-bootstrap drift). Σ and the residual render
 * in `--warn` instead of silently claiming the decomposition is additive.
 */
export const UnhedgedResidual: Story = {
  args: {
    data: SAMPLE_5Y,
    parallelDv01: -44200,
    unit: "USD/bp",
  },
};
