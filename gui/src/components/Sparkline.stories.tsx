/**
 * Sparkline stories — a Canvas 2D mid-history trace (GUI-DESIGN §3.5, §6.2).
 * Direction tint is ONE shared rule (net first→last), exported as
 * `sparklineDirection` so any accompanying glyph/chip agrees. Stories cover:
 * up/down/flat series, explicit vs auto-detected direction, DPR-aware sizes,
 * and a live-tick harness that appends synthetic ticks. Token colors only
 * (--bid / --offer from the Aurora cascade) — no raw hex in story code.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { useEffect, useState } from "react";
import { Sparkline, sparklineDirection } from "./Sparkline";

/** A simple random-walk series of length n seeded from a start value. */
function randomWalk(n: number, start: number, sigma: number): number[] {
  const out: number[] = [start];
  for (let i = 1; i < n; i++) {
    out.push(+(out[i - 1]! + (Math.random() - 0.5) * sigma).toFixed(5));
  }
  return out;
}

const UP_SERIES = [1.085, 1.0852, 1.0855, 1.0858, 1.0861, 1.087, 1.0875, 1.088, 1.0884, 1.0889];
const DOWN_SERIES = [1.0889, 1.0884, 1.088, 1.0875, 1.087, 1.0861, 1.0858, 1.0855, 1.0852, 1.085];
const FLAT_SERIES = [1.0860, 1.0862, 1.0858, 1.0861, 1.0860, 1.0862, 1.0859, 1.0861, 1.0860, 1.0860];

const meta = {
  title: "Components/Sparkline",
  component: Sparkline,
  tags: ["autodocs"],
  argTypes: {
    width: { control: { type: "range", min: 40, max: 240, step: 8 } },
    height: { control: { type: "range", min: 14, max: 64, step: 2 } },
    direction: {
      control: "inline-radio",
      options: [undefined, "up", "down", "flat"],
      description:
        "Force the tint direction (must match the series). Leave undefined to use sparklineDirection(values) automatically.",
    },
    ariaLabel: { control: "text" },
  },
  args: {
    values: UP_SERIES,
    width: 96,
    height: 22,
    ariaLabel: "EUR/USD 10-tick history",
  },
} satisfies Meta<typeof Sparkline>;

export default meta;

type Story = StoryObj<typeof meta>;

/** Auto-direction from an upward series — line tints --bid green. */
export const Default: Story = {};

/** Upward net move — tints --bid (green). */
export const Up: Story = {
  args: { values: UP_SERIES, ariaLabel: "ascending series" },
};

/** Downward net move — tints --offer (red). */
export const Down: Story = {
  args: { values: DOWN_SERIES, ariaLabel: "descending series" },
};

/**
 * Flat window (first ≈ last) — tints neutral (--bid at low alpha). Direction
 * token is "flat"; sparklineDirection returns "flat" for an unchanged window.
 */
export const Flat: Story = {
  args: { values: FLAT_SERIES, direction: "flat", ariaLabel: "flat series" },
};

/** Explicit direction override — forces --offer tint even on an up series. */
export const ExplicitDirection: Story = {
  args: { values: UP_SERIES, direction: "down", ariaLabel: "forced-down tint on up series" },
};

/** Wide format — grid/blotter row sparkline (240×22). */
export const Wide: Story = {
  args: { values: randomWalk(40, 1.0854, 0.0004), width: 240, height: 22 },
};

/** Tall format — inspector strip full-height trace (96×48). */
export const Tall: Story = {
  args: { values: randomWalk(20, 1.0854, 0.0006), width: 96, height: 48 },
};

/**
 * Live-tick harness — appends one synthetic tick every 400 ms to a capped
 * 40-point window. The glyph below reads sparklineDirection from the SAME
 * series — the ONE truth that keeps glyph and line tint in agreement (P0-4).
 */
export const LiveTick: Story = {
  render: () => {
    // eslint-disable-next-line react-hooks/rules-of-hooks
    const [series, setSeries] = useState<number[]>(() => randomWalk(20, 1.0854, 0.0004));
    // eslint-disable-next-line react-hooks/rules-of-hooks
    useEffect(() => {
      const id = setInterval(() => {
        setSeries((prev) => {
          const next = [
            ...prev.slice(-39),
            +(prev[prev.length - 1]! + (Math.random() - 0.5) * 0.0004).toFixed(5),
          ];
          return next;
        });
      }, 400);
      return () => clearInterval(id);
    }, []);

    const dir = sparklineDirection(series);
    const dirColor = dir === "up" ? "var(--bid)" : dir === "down" ? "var(--offer)" : "var(--text-tertiary)";
    const glyph = dir === "up" ? "▲" : dir === "down" ? "▼" : "—";

    return (
      <div style={{ display: "flex", alignItems: "center", gap: "var(--space-4)" }}>
        <Sparkline values={series} width={160} height={30} ariaLabel="EUR/USD live" />
        <span
          style={{
            fontFamily: "var(--font-mono)",
            fontSize: "var(--type-body)",
            color: dirColor,
          }}
          aria-label={`direction ${dir}`}
        >
          {glyph}
        </span>
        <span
          style={{
            fontFamily: "var(--font-mono)",
            fontSize: "var(--type-body)",
            color: "var(--text-primary)",
          }}
        >
          {series[series.length - 1]?.toFixed(5)}
        </span>
      </div>
    );
  },
};

/** A row of sparklines as they would appear in a blotter — compact 80×18 tiles. */
export const BlotterRow: Story = {
  render: () => {
    const rows = [
      { pair: "EUR/USD", series: randomWalk(20, 1.0854, 0.0003) },
      { pair: "USD/JPY", series: randomWalk(20, 149.8, 0.08) },
      { pair: "GBP/USD", series: randomWalk(20, 1.2634, 0.0004) },
      { pair: "EUR/JPY", series: randomWalk(20, 162.4, 0.1) },
    ];
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)" }}>
        {rows.map(({ pair, series: s }) => (
          <div
            key={pair}
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--space-5)",
              padding: "var(--space-2) var(--space-4)",
              background: "var(--bg-raised)",
              borderRadius: "var(--r-sm)",
            }}
          >
            <span
              style={{
                fontSize: "var(--type-body)",
                fontFamily: "var(--font-brand)",
                color: "var(--text-primary)",
                width: 64,
              }}
            >
              {pair}
            </span>
            <Sparkline values={s} width={80} height={18} ariaLabel={`${pair} history`} />
          </div>
        ))}
      </div>
    );
  },
};
