/**
 * PriceTile stories — the tabular numeric that flashes once on value change and
 * decays (GUI-DESIGN §3.4, the single most important motion in the app). Stories
 * demonstrate: sizes, bid/offer/neutral side tints, the ▲/▼ direction glyph, a
 * live-tick harness that streams synthetic price updates, and the prefers-reduced-
 * motion posture (CSS animation → static tint). Token references only — no raw hex.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { useEffect, useState } from "react";
import { PriceTile } from "./PriceTile";

const SIZES = ["display", "headline", "callout"] as const;
const SIDES = ["bid", "offer", "neutral"] as const;

const fmt2 = (v: number) => v.toFixed(2);
const fmt4 = (v: number) => v.toFixed(4);
const fmt5 = (v: number) => v.toFixed(5);

const meta = {
  title: "Components/PriceTile",
  component: PriceTile,
  tags: ["autodocs"],
  argTypes: {
    value: { control: { type: "number", step: 0.0001 } },
    side: {
      control: "inline-radio",
      options: SIDES,
      description:
        "Resting text tint: bid (green) / offer (red) / neutral. Tick direction is always carried independently by the flash.",
    },
    size: {
      control: "inline-radio",
      options: SIZES,
      description: "display (28px) | headline (20px) | callout (12px).",
    },
    showGlyph: {
      control: "boolean",
      description: "Render the ▲/▼ direction glyph alongside the value.",
    },
    ariaLabel: { control: "text" },
  },
  args: {
    value: 1.08542,
    format: fmt5,
    side: "neutral",
    size: "callout",
    showGlyph: false,
  },
} satisfies Meta<typeof PriceTile>;

export default meta;

type Story = StoryObj<typeof meta>;

/** Single tile; adjust `value` in Controls to trigger a flash. */
export const Default: Story = {};

/** Display size — used for the hero price in the inspector strip. */
export const DisplaySize: Story = {
  args: { value: 1.08542, format: fmt5, size: "display", showGlyph: true, side: "neutral" },
};

/** Bid side tint (green resting text). */
export const BidSide: Story = {
  args: { value: 1.08540, format: fmt5, side: "bid", size: "headline", showGlyph: true },
};

/** Offer side tint (red resting text). */
export const OfferSide: Story = {
  args: { value: 1.08548, format: fmt5, side: "offer", size: "headline", showGlyph: true },
};

/**
 * Live-tick harness — streams a synthetic random walk to prove the flash
 * animation plays on EVERY tick (including consecutive ticks in the same
 * direction, which a CSS-class-toggle approach would miss).
 */
export const LiveTick: Story = {
  render: () => {
    // eslint-disable-next-line react-hooks/rules-of-hooks
    const [price, setPrice] = useState(1.08542);
    // eslint-disable-next-line react-hooks/rules-of-hooks
    useEffect(() => {
      const id = setInterval(() => {
        setPrice((p) => +(p + (Math.random() - 0.5) * 0.0005).toFixed(5));
      }, 600);
      return () => clearInterval(id);
    }, []);
    return (
      <PriceTile
        value={price}
        format={fmt5}
        size="display"
        side="neutral"
        showGlyph
        ariaLabel="EUR/USD live"
      />
    );
  },
};

/** Three sizes stacked — the full typographic hierarchy visible together. */
export const AllSizes: Story = {
  render: () => (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--space-5)",
        alignItems: "flex-start",
      }}
    >
      {SIZES.map((size) => (
        <div
          key={size}
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--space-4)",
          }}
        >
          <span
            style={{
              fontSize: "var(--type-caption)",
              color: "var(--text-tertiary)",
              width: 64,
              textTransform: "uppercase",
              letterSpacing: "0.06em",
            }}
          >
            {size}
          </span>
          <PriceTile value={1.08542} format={fmt5} size={size} showGlyph side="neutral" />
        </div>
      ))}
    </div>
  ),
};

/**
 * Two-way price row — bid left, offer right, as a blotter cell would render.
 * Uses synthetic values close to a real EUR/USD mid; 2-pip spread.
 */
export const TwoWayRow: Story = {
  render: () => (
    <div
      style={{
        display: "flex",
        gap: "var(--space-6)",
        alignItems: "baseline",
        fontFamily: "var(--font-mono)",
      }}
    >
      <PriceTile value={1.08538} format={fmt5} size="headline" side="bid" showGlyph />
      <span style={{ color: "var(--text-tertiary)", fontSize: "var(--type-caption)" }}>
        EUR/USD
      </span>
      <PriceTile value={1.08560} format={fmt5} size="headline" side="offer" showGlyph />
    </div>
  ),
};

/** Compact numeric formats — rate (2dp) and spot (4dp) variants. */
export const NumericFormats: Story = {
  render: () => (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-4)" }}>
      <div style={{ display: "flex", gap: "var(--space-5)", alignItems: "center" }}>
        <span style={{ fontSize: "var(--type-caption)", color: "var(--text-tertiary)", width: 80 }}>
          rate (2dp)
        </span>
        <PriceTile value={5.25} format={fmt2} size="callout" side="neutral" />
      </div>
      <div style={{ display: "flex", gap: "var(--space-5)", alignItems: "center" }}>
        <span style={{ fontSize: "var(--type-caption)", color: "var(--text-tertiary)", width: 80 }}>
          spot (4dp)
        </span>
        <PriceTile value={1.0854} format={fmt4} size="callout" side="neutral" />
      </div>
      <div style={{ display: "flex", gap: "var(--space-5)", alignItems: "center" }}>
        <span style={{ fontSize: "var(--type-caption)", color: "var(--text-tertiary)", width: 80 }}>
          pair (5dp)
        </span>
        <PriceTile value={1.08542} format={fmt5} size="callout" side="neutral" />
      </div>
    </div>
  ),
};
