/**
 * ScenarioHeatmap stories — the spot × vol P&L scenario grid (Risk & Scenario).
 *
 * Appearance note: all colour is the diverging `--div-*` dataviz palette (blue = loss,
 * orange = gain), NEVER a brand hue, mapped through a SYMMETRIC ±|max| domain centred
 * on 0 so equal-magnitude gains and losses read as mirror intensities. The mapping is
 * discrete (five bands cut at ±¼|max| and ±½|max|), so it repaints correctly across
 * the Dark / Light / Increased-Contrast appearances and honours prefers-reduced-motion
 * (the reveal animation is dropped). All grids below are DETERMINISTIC sample surfaces
 * synthesised from a simple long/short gamma-vega model — not live risk.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { useState } from "react";
import { ScenarioHeatmap, type ScenarioCell } from "./ScenarioHeatmap";

// Axes: spot shocks left→right, vol shocks bottom→top (ECharts category order).
const SPOT_SHOCKS = [-3, -2, -1, 0, 1, 2, 3] as const;
const VOL_SHOCKS = [-4, -2, 0, 2, 4] as const; // bottom → top

const fmtSpot = (p: number): string => `${p > 0 ? "+" : p < 0 ? "−" : ""}${Math.abs(p)}%`;
const fmtVol = (v: number): string => `${v > 0 ? "+" : v < 0 ? "−" : ""}${Math.abs(v).toFixed(1)}v`;

const SPOT_LABELS = SPOT_SHOCKS.map(fmtSpot);
const VOL_LABELS = VOL_SHOCKS.map(fmtVol);

/**
 * Deterministic sample P&L (in $k) from a toy revaluation:
 *   pnl(s, v) = gamma·s² − vega·v + skew·s + cross·s·v
 * with s the spot shock (%) and v the vol shock (ATM vols). Positive `gamma` is a
 * long-gamma (convex in spot) book; positive `vega` loses as vol rises (short vega).
 */
function book(gamma: number, vega: number, skew: number, cross: number): number[][] {
  return VOL_SHOCKS.map((v) =>
    SPOT_SHOCKS.map((s) => Math.round(gamma * s * s - vega * v + skew * s + cross * s * v)),
  );
}

const LONG_GAMMA_SHORT_VEGA = book(16, 45, 3, 0.8);
const SHORT_GAMMA_LONG_VEGA = book(-14, -40, -2, -0.6);
const CALM_BOOK = book(9, 18, 1, 0.3);

const meta = {
  title: "Viz/ScenarioHeatmap",
  component: ScenarioHeatmap,
  tags: ["autodocs"],
  parameters: {
    layout: "padded",
    docs: {
      description: {
        component:
          "Spot × vol P&L scenario grid. Diverging `--div-*` palette (blue = loss, orange = gain) on a symmetric ±|max| domain centred at 0; click a cell to drill (`onCellClick`). Sample grids are deterministic — not live risk.",
      },
    },
  },
  argTypes: {
    unit: { control: "text", description: "P&L unit shown in tooltip / legend." },
    height: { control: { type: "range", min: 200, max: 520, step: 20 } },
    showValues: { control: "boolean", description: "Draw the P&L number inside each cell." },
    onCellClick: { action: "cellClick" },
    pnl: { control: false },
    spotLabels: { control: false },
    volLabels: { control: false },
  },
  args: {
    pnl: LONG_GAMMA_SHORT_VEGA,
    spotLabels: SPOT_LABELS,
    volLabels: VOL_LABELS,
    unit: "$k",
    height: 300,
    showValues: true,
  },
} satisfies Meta<typeof ScenarioHeatmap>;

export default meta;

type Story = StoryObj<typeof meta>;

/** Long-gamma / short-vega book — gains as vol falls, convex in spot. */
export const LongGammaShortVega: Story = {};

/** Short-gamma / long-vega book — the sign flip, exercising the opposite ramp pole. */
export const ShortGammaLongVega: Story = {
  args: { pnl: SHORT_GAMMA_LONG_VEGA },
};

/** A calmer book with a tighter P&L range — the symmetric domain re-scales to it. */
export const CalmBook: Story = {
  args: { pnl: CALM_BOOK, showValues: true },
};

/**
 * Interactive drill — click any cell; the selected scenario is echoed below to show
 * the `onCellClick` payload the Risk drill panel consumes.
 */
export const InteractiveDrill: Story = {
  args: { pnl: LONG_GAMMA_SHORT_VEGA },
  render: (args) => {
    // eslint-disable-next-line react-hooks/rules-of-hooks
    const [cell, setCell] = useState<ScenarioCell | null>(null);
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)" }}>
        <ScenarioHeatmap {...args} onCellClick={setCell} />
        <div
          style={{
            fontFamily: "var(--font-mono)",
            fontSize: "var(--type-caption)",
            color: cell ? "var(--text-primary)" : "var(--text-tertiary)",
          }}
        >
          {cell
            ? `drill → spot ${cell.spotLabel} · vol ${cell.volLabel} · Δ P&L ${cell.pnl} $k`
            : "click a cell to drill"}
        </div>
      </div>
    );
  },
};

/** Honest empty state — an empty grid renders an em-dash, never a fabricated surface. */
export const EmptyGrid: Story = {
  args: { pnl: [], spotLabels: [], volLabels: [] },
};
