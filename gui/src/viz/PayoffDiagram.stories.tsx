/**
 * PayoffDiagram stories — the multi-leg payoff-at-expiry diagram (visx).
 *
 * Every story uses SEEDED, deterministic sample legs (a synthetic EUR/USD 1.0850
 * book) — no live/invented numbers. The component auto-derives the strategy name,
 * shades profit (--div-pos1 warm) / loss (--div-neg2 cool) regions, overlays a
 * bold net line, a dotted illustrative "today" curve and light dashed per-leg
 * lines, and marks strikes, spot and break-evens. The last story exercises the
 * honest empty state for a path-dependent (Asian) leg. Token references only — no
 * raw hex — and the mount transition self-disables under prefers-reduced-motion.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { PayoffDiagram } from "./PayoffDiagram";
import type { PayoffLeg } from "./PayoffDiagram";

const SPOT = 1.085;

/** 3-leg seagull: short 1.0700 put + long 1.1000 call − short 1.1250 call (net credit). */
const SEAGULL: PayoffLeg[] = [
  { kind: "put", side: "short", strike: 1.07, premium: 0.009 },
  { kind: "call", side: "long", strike: 1.1, premium: 0.0075 },
  { kind: "call", side: "short", strike: 1.125, premium: 0.003 },
];

/** Long ATM straddle: long 1.0850 call + long 1.0850 put (long vol / long gamma, debit). */
const STRADDLE: PayoffLeg[] = [
  { kind: "call", side: "long", strike: 1.085, premium: 0.011 },
  { kind: "put", side: "long", strike: 1.085, premium: 0.0105 },
];

/** Risk reversal: long 1.1050 call vs short 1.0700 put — directional long-skew. */
const RISK_REVERSAL: PayoffLeg[] = [
  { kind: "put", side: "short", strike: 1.07, premium: 0.0084 },
  { kind: "call", side: "long", strike: 1.105, premium: 0.0079 },
];

/** 4-leg iron condor: long 1.0600 put / short 1.0750 put / short 1.1050 call / long 1.1200 call. */
const IRON_CONDOR: PayoffLeg[] = [
  { kind: "put", side: "long", strike: 1.06, premium: 0.003 },
  { kind: "put", side: "short", strike: 1.075, premium: 0.007 },
  { kind: "call", side: "short", strike: 1.105, premium: 0.0068 },
  { kind: "call", side: "long", strike: 1.12, premium: 0.0028 },
];

/** A path-dependent leg — the diagram cannot draw a terminal-spot payoff for it. */
const ASIAN: PayoffLeg[] = [{ kind: "asian", side: "long", strike: 1.085, premium: 0.009 }];

const meta = {
  title: "Viz/PayoffDiagram",
  component: PayoffDiagram,
  tags: ["autodocs"],
  parameters: {
    docs: {
      description: {
        component:
          "Multi-leg option-strategy payoff at expiry. Bold net line (signed leg sum), " +
          "dotted illustrative today/MTM curve (Gaussian time-value smoothing of the terminal " +
          "payoff — a faithful shape, not a priced mark), light dashed per-leg lines, and " +
          "profit/loss regions shaded with the diverging dataviz palette. Strikes, spot and " +
          "break-evens are marked and the strategy name is auto-derived. All data is seeded " +
          "sample data (synthetic EUR/USD 1.0850); quantitative colour uses --seq/--div tokens " +
          "only, and the enter transition respects prefers-reduced-motion.",
      },
    },
  },
  argTypes: {
    spot: { control: { type: "number", step: 0.001 } },
    height: { control: { type: "number", step: 10 } },
    showLegs: { control: "boolean", description: "Draw the light per-leg payoff lines." },
    showToday: { control: "boolean", description: "Draw the smoothed illustrative today/MTM curve." },
    timeValueVol: {
      control: { type: "range", min: 0, max: 0.1, step: 0.005 },
      description: "ATM vol × √T shaping only the illustrative today curve.",
    },
  },
  args: {
    legs: SEAGULL,
    spot: SPOT,
    height: 320,
    showLegs: true,
    showToday: true,
    timeValueVol: 0.03,
  },
  decorators: [
    (Story) => (
      <div style={{ maxWidth: 760, background: "var(--bg-base)", padding: "var(--space-5)" }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof PayoffDiagram>;

export default meta;

type Story = StoryObj<typeof meta>;

/** Seagull — a recognized 3-leg collar-with-wing (auto-named), net credit. */
export const Seagull: Story = {};

/** Long straddle — a two-sided long-vol profile with unbounded upside and a bounded max loss. */
export const Straddle: Story = {
  args: { legs: STRADDLE },
};

/** Risk reversal — a directional long-skew position (short put financing a long call). */
export const RiskReversal: Story = {
  args: { legs: RISK_REVERSAL },
};

/** Iron condor — a 4-leg range-bound net-credit structure with capped wings. */
export const IronCondor: Story = {
  args: { legs: IRON_CONDOR },
};

/** Net line only — per-leg lines and the today curve toggled off for a clean expiry profile. */
export const NetOnly: Story = {
  args: { legs: IRON_CONDOR, showLegs: false, showToday: false },
};

/**
 * Honest empty state — a path-dependent (Asian) leg cannot be drawn as a clean
 * terminal-spot payoff, so the diagram renders a reason instead of fabricating a curve.
 */
export const PathDependentEmptyState: Story = {
  args: { legs: ASIAN },
};
