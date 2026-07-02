/**
 * YieldCurve stories — the FI term-structure workbench chart (mockup 14). Stories
 * demonstrate: the log-linear piecewise-flat forward staircase, the smooth
 * monotone-convex forward, an upward-sloping curve for shape contrast, and the
 * honest empty state. Every dataset is DETERMINISTIC synthetic sample data (a
 * SOFR-shaped inverted-front curve and a textbook upward curve) — clearly labelled
 * sample, never live marks. Colour comes only from the --seq-* dataviz tokens, so
 * flipping the Storybook appearance/contrast theme recolours the chart with no
 * code change.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { YieldCurve, type CurveNode } from "./YieldCurve";

/**
 * SAMPLE — a SOFR-shaped USD OIS curve as of a mid-2026-like date: an inverted
 * front (o/n ~5.31%) easing through a ~3.8% belly and rising into the long end.
 * Continuously-compounded zeros as fractions. Synthetic; not a live mark.
 */
const SAMPLE_SOFR: readonly CurveNode[] = [
  { label: "ON", tenorYears: 1 / 365, zeroRate: 0.0531 },
  { label: "1W", tenorYears: 7 / 365, zeroRate: 0.0531 },
  { label: "1M", tenorYears: 1 / 12, zeroRate: 0.053 },
  { label: "3M", tenorYears: 0.25, zeroRate: 0.051 },
  { label: "6M", tenorYears: 0.5, zeroRate: 0.0485 },
  { label: "1Y", tenorYears: 1, zeroRate: 0.045 },
  { label: "2Y", tenorYears: 2, zeroRate: 0.0402 },
  { label: "5Y", tenorYears: 5, zeroRate: 0.0378 },
  { label: "10Y", tenorYears: 10, zeroRate: 0.0392 },
  { label: "30Y", tenorYears: 30, zeroRate: 0.0405 },
];

/** SAMPLE — a textbook upward-sloping curve (2.50% → 5.45%). Synthetic. */
const SAMPLE_UPWARD: readonly CurveNode[] = [
  { label: "ON", tenorYears: 1 / 365, zeroRate: 0.025 },
  { label: "1W", tenorYears: 7 / 365, zeroRate: 0.0255 },
  { label: "1M", tenorYears: 1 / 12, zeroRate: 0.027 },
  { label: "3M", tenorYears: 0.25, zeroRate: 0.0295 },
  { label: "6M", tenorYears: 0.5, zeroRate: 0.0325 },
  { label: "1Y", tenorYears: 1, zeroRate: 0.036 },
  { label: "2Y", tenorYears: 2, zeroRate: 0.04 },
  { label: "5Y", tenorYears: 5, zeroRate: 0.0455 },
  { label: "10Y", tenorYears: 10, zeroRate: 0.05 },
  { label: "30Y", tenorYears: 30, zeroRate: 0.0545 },
];

const meta = {
  title: "Viz/YieldCurve",
  component: YieldCurve,
  tags: ["autodocs"],
  parameters: {
    docs: {
      description: {
        component:
          "Overlaid zero / instantaneous-forward / discount-factor term structure on a log-time tenor axis (ON → 30Y), all derived from ONE ln(DF) bootstrap of the pillar nodes. The forward is drawn as the honest piecewise-flat staircase that log-linear-in-ln(DF) interpolation actually produces (switch interpolation to monotone-convex for the smooth forward). Legend toggles each overlay; hover for a z/f/DF readout. Appearance: sequential --seq-* dataviz palette only (never brand coral/indigo), Space Grotesk labels, JetBrains Mono numerics — theme-driven via CSS custom properties, so it tracks the dark/light/high-contrast appearance switch automatically.",
      },
    },
  },
  argTypes: {
    interpolation: {
      control: "inline-radio",
      options: ["log-linear", "monotone-convex"],
      description:
        "ln(DF) interpolation: log-linear gives a piecewise-flat forward; monotone-convex gives a smooth, arbitrage-monotone forward.",
    },
    height: { control: { type: "number", min: 200, max: 520, step: 20 } },
    nodes: { control: false },
  },
  args: {
    nodes: SAMPLE_SOFR,
    interpolation: "log-linear",
    height: 340,
  },
} satisfies Meta<typeof YieldCurve>;

export default meta;

type Story = StoryObj<typeof meta>;

/**
 * Log-linear bootstrap of the sample SOFR curve — the instantaneous forward is the
 * piecewise-flat staircase (each pillar interval carries one constant forward).
 */
export const Default: Story = {};

/**
 * Monotone-convex bootstrap of the same sample curve — the forward is now a smooth
 * piecewise-quadratic that stays arbitrage-monotone. Compare the forward line with
 * the Default staircase.
 */
export const MonotoneConvex: Story = {
  args: { interpolation: "monotone-convex" },
};

/**
 * An upward-sloping sample curve for shape contrast — zero and forward both rise
 * into the long end and the discount factor decays faster on the right axis.
 */
export const UpwardSloping: Story = {
  args: { nodes: SAMPLE_UPWARD },
};

/**
 * Honest empty state — a single pillar cannot define a curve, so the chart renders
 * an em-dash placeholder with a reason instead of fabricating an interpolation.
 */
export const NotEnoughPillars: Story = {
  args: { nodes: [{ label: "1Y", tenorYears: 1, zeroRate: 0.045 }] },
};
