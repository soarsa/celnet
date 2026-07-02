/**
 * Stories — RiskWorkspace (GUI-DESIGN §4.4).
 *
 * The scenario / what-if grid. A spot×vol shock grid driven by
 * SurfaceService.Scenario (ShockAxis abs/rel) for the selected structure. Each
 * cell is a P&L under the shock, tinted on the perceptual diverging ramp (reads
 * magnitude honestly, no rainbow); the spot/vol "today" cell is anchored (▣).
 *
 * The scenario-axis selector (P0-7) exposes all five contract `ShockFactor`s on
 * both grid axes: SPOT, VOL, RATE_DOM, RATE_FOR, TIME. Picking a factor already
 * on the other axis swaps them (the two axes must be distinct). The Vega ladder
 * panel renders the server's book-shaped decomposition: (tenor, delta) pillar
 * buckets + cross-gamma + theta roll.
 *
 * Token contract: cell tints come from rampColor() in viz/ramp.ts, which derives
 * from var(--bid)/var(--offer) OKLCH; the legend bar uses the same gradient. The
 * anchor cell uses var(--surface-3). No inline hex anywhere in the component.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { AppProvider } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { RiskWorkspace } from "./RiskWorkspace";

const meta = {
  title: "Workspaces/RiskWorkspace",
  component: RiskWorkspace,
  decorators: [
    (Story) => (
      <AppProvider transport={createMockTransport()}>
        <Story />
      </AppProvider>
    ),
  ],
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    docs: {
      description: {
        component:
          "The scenario / what-if grid (GUI-DESIGN §4.4). A SPOT×VOL shock grid " +
          "for the selected structure; cell P&L tinted on a perceptual diverging ramp. " +
          "The scenario-axis selector exposes all five ShockFactors on both axes " +
          "(swaps when axes would collide). The Vega ladder panel renders the server's " +
          "bucketed decomposition: (tenor, delta) pillars + cross-gamma + theta roll.",
      },
    },
  },
} satisfies Meta<typeof RiskWorkspace>;

export default meta;

type Story = StoryObj<typeof RiskWorkspace>;

/**
 * Default scenario grid — SPOT (rows) × VOL (cols), P&L metric, seeded 25Δ RR
 * on EUR/USD at 1M tenor, 10mm notional. The mock transport fires the Scenario
 * RPC deterministically; the 5×5 matrix renders immediately with real reprice values
 * (not fabricated). The anchored today-cell (▣) is at (0% spot, 0% vol).
 *
 * Use the Metric tabs (P&L / Δ / ν) and the axis pickers to re-slice the grid.
 */
export const Default: Story = {};

/**
 * P&L metric, SPOT × VOL grid — the most common desk view. The diverging tint
 * reads gain/loss direction at a glance; the magnitude is honest (the ramp
 * normalises to the max-absolute value in the current grid). No rainbow — the
 * OKLCH-perceptual ramp from var(--bid)/var(--offer) stays readable in both
 * dark and light appearances.
 */
export const PnlSpotVol: Story = {
  name: "P&L — SPOT × VOL (default axes)",
  parameters: {
    docs: {
      description: {
        story:
          "The default grid (SPOT × VOL) with P&L metric. Cell tints use the " +
          "OKLCH perceptual diverging ramp so magnitude is readable without colour- " +
          "only encoding (the numeric value is always present). The legend bar at " +
          "the foot mirrors the ramp's two-sided gradient.",
      },
    },
  },
};

/**
 * Delta metric — the same SPOT × VOL grid but reading out the delta at each
 * shocked state rather than the P&L. Useful for seeing how the hedge ratio moves
 * as spot and vol shift: a deep OTM option's delta collapses toward zero quickly
 * as spot moves away, a near-ATM option's delta is ~0.5 and moves slowly.
 */
export const DeltaMetric: Story = {
  name: "Delta metric — delta surface across shock states",
  parameters: {
    docs: {
      description: {
        story:
          "Switch to the Δ metric tab to read out spot delta at each (spot, vol) " +
          "shock state. The normalisation is independent per metric so the tint " +
          "range always fills the full diverging scale for that metric's data.",
      },
    },
  },
};

/**
 * Vega ladder panel — the book-shaped risk disclosure alongside the grid. Bucketed
 * vega per (tenor, delta) pillar, cross-gamma pairs (SPOT×VOL, RATE_DOM×SPOT,
 * SPOT×TIME), and theta roll at ON/1W/1M. The server returns `bucketedRisk` in the
 * ScenarioResult; the component renders the honest empty-state when absent.
 */
export const VegaLadder: Story = {
  name: "Vega ladder panel (book-shaped risk)",
  parameters: {
    docs: {
      description: {
        story:
          "The Vega ladder panel (right side) shows the server's book-shaped " +
          "decomposition: (tenor, delta) pillar buckets in notional × 1bp units, " +
          "cross-gamma off-diagonals, and theta roll at three horizons. The bar " +
          "widths are proportional to |vega| / max, normalised locally.",
      },
    },
  },
};
