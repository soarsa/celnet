/**
 * XvaExposureFan stories — the counterparty exposure-profile fan (EE / EPE /
 * PFE(95%) + mirrored ENE), GUI mockup 07. Every story feeds a DETERMINISTIC
 * seeded sample profile (a mulberry32 PRNG over a fixed tenor ladder), never
 * invented "live" numbers: the live wire is blocked on the deferred D-xva
 * activation (`celnet-xva::compute_xva` has zero non-test callers), so these are
 * illustrative figures only. Token references throughout — no raw hex; quantitative
 * colour uses the Viridis sequential ramp (--seq-*) and the diverging cool pole
 * (--div-neg*), never brand hues.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { XvaExposureFan, type FanBucket } from "./XvaExposureFan";

/** Deterministic PRNG (mulberry32) — same seed ⇒ byte-identical sample profile. */
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

/** The tenor ladder to the 2Y horizon (years + display label). */
const TENORS: ReadonlyArray<{ t: number; label: string }> = [
  { t: 0, label: "0" },
  { t: 1 / 52, label: "1W" },
  { t: 1 / 12, label: "1M" },
  { t: 0.25, label: "3M" },
  { t: 0.5, label: "6M" },
  { t: 0.75, label: "9M" },
  { t: 1.0, label: "1Y" },
  { t: 1.5, label: "18M" },
  { t: 2.0, label: "2Y·H" },
];

interface ProfileOpts {
  /** EE peak magnitude (in $mm) at the hump tenor. */
  readonly peak: number;
  /** Hump tenor in years (where EE peaks). */
  readonly hump?: number;
  /** Tail-width multiplier — how wide the quantile fan spreads around EE. */
  readonly tail?: number;
}

/**
 * A hump-shaped netting-set exposure profile — the canonical FX-options shape:
 * exposure builds as optionality accrues, peaks near the hump tenor, then decays
 * to the horizon. Quantiles are fixed multiples of a per-bucket seeded EE (so the
 * `pfeLo <= q25 <= ee <= q75 <= pfe` ordering always holds), the negative side is
 * a mirrored fraction. Fully deterministic in `seed`.
 */
function makeProfile(seed: number, opts: ProfileOpts): FanBucket[] {
  const rand = mulberry32(seed);
  const hump = opts.hump ?? 0.5;
  const tail = opts.tail ?? 1;
  return TENORS.map(({ t, label }) => {
    const x = t <= 0 ? 0 : t / hump;
    const shape = t <= 0 ? 0 : Math.pow(x, 0.55) * Math.exp(1 - x);
    const n = opts.peak * shape * (0.9 + 0.2 * rand());
    return {
      t,
      label,
      ee: n,
      q75: n * (1 + 0.22 * tail),
      q25: n * (1 - 0.28 * tail),
      pfe: n * (1 + 0.85 * tail),
      pfeLo: n * (1 - 0.58 * tail),
      ene: -n * 0.62,
      eneBandHi: -n * (0.62 - 0.32 * tail),
      eneBandLo: -n * (0.62 + 0.43 * tail),
    };
  });
}

/** Uncollateralised G10-A netting set — the full hump with wide tails. */
const UNCOLLATERALISED = makeProfile(0x51ce, { peak: 6.0, hump: 0.5, tail: 1 });
/** Daily-margined CSA — collateral compresses the profile and its tails. */
const COLLATERALISED = makeProfile(0xc07a, { peak: 2.4, hump: 0.4, tail: 0.45 });
/** Wrong-way stressed uncollateralised — later hump, fat PFE tail. */
const STRESSED = makeProfile(0x57e5, { peak: 8.6, hump: 0.75, tail: 1.6 });

const meta = {
  title: "Viz/XvaExposureFan",
  component: XvaExposureFan,
  tags: ["autodocs"],
  parameters: {
    docs: {
      description: {
        component:
          "Counterparty exposure-profile fan for the XVA workspace: an outer PFE 5–95% " +
          "quantile band, an inner 25–75% band, the expected-exposure (EE) line, the " +
          "PFE(95%) envelope, the time-average EPE reference, and a mirrored ENE band + " +
          "line below zero with its EEN reference. All colour is design-token CSS vars — " +
          "the Viridis sequential ramp (--seq-*) for positive exposure and the diverging " +
          "cool pole (--div-neg*) for the mirrored negative exposure, never brand hues. " +
          "Appearance/contrast/density all compose via the token cascade; the chart is " +
          "static SVG so the only motion is a mount fade that respects prefers-reduced-" +
          "motion. DATA IS SEEDED SAMPLE ONLY — the live wire is blocked on the deferred " +
          "D-xva activation (celnet-xva has zero non-test callers).",
      },
    },
  },
  argTypes: {
    measure: {
      control: "inline-radio",
      options: ["ee", "epe", "pfe"],
      description: "Emphasised measure — thickens the matching line (mirrors the segment control).",
    },
    height: { control: { type: "range", min: 220, max: 420, step: 20 } },
    unit: { control: "text" },
    title: { control: "text" },
  },
  args: {
    buckets: UNCOLLATERALISED,
    measure: "ee",
    height: 300,
    unit: "mm",
    title: "Exposure profile",
  },
} satisfies Meta<typeof XvaExposureFan>;

export default meta;

type Story = StoryObj<typeof meta>;

/**
 * The default uncollateralised G10-A netting set, EE emphasised. The hump peaks
 * near 6M and the mirrored ENE band sits below the zero baseline.
 */
export const Default: Story = {};

/**
 * PFE(95%) emphasised — the segment-control "PFE 95%" posture: the outer band
 * darkens and the 95% envelope becomes a solid, heavier line for capital / limit
 * reading.
 */
export const PfeEmphasis: Story = {
  args: { measure: "pfe" },
};

/**
 * Daily-margined CSA counterparty — collateral compresses both the exposure level
 * and the quantile tails, so the fan is tighter and lower. EPE emphasised.
 */
export const Collateralised: Story = {
  args: { buckets: COLLATERALISED, measure: "epe", title: "Exposure profile · CSA (daily)" },
};

/**
 * Wrong-way-risk stress — a later hump and a fat PFE(95%) tail on an
 * uncollateralised name; PFE emphasised for the peak-exposure read.
 */
export const WrongWayStress: Story = {
  args: {
    buckets: STRESSED,
    measure: "pfe",
    height: 340,
    title: "Exposure profile · WWR stress",
  },
};

/**
 * Honest empty state — a single-bucket (degenerate) profile cannot draw a fan, so
 * the component renders an em-dash empty state with a reason rather than fabricating
 * a curve.
 */
export const EmptyState: Story = {
  args: { buckets: [UNCOLLATERALISED[4]!] },
};
