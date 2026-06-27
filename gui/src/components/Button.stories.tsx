/**
 * Button stories — the one button primitive (primary / secondary / ghost / bid /
 * offer). All five variants × two sizes, plus the keyboard-hint `kbd` slot and a
 * click-to-trade bid/offer pair rendered side-by-side. The stories consume the
 * real Aurora token cascade (see .storybook/preview.ts) — no raw hex, no
 * inline color overrides.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { Button } from "./Button";

const VARIANTS = ["primary", "secondary", "ghost", "bid", "offer"] as const;
const SIZES = ["md", "lg"] as const;

const meta = {
  title: "Components/Button",
  component: Button,
  tags: ["autodocs"],
  argTypes: {
    variant: {
      control: "inline-radio",
      options: VARIANTS,
      description:
        "Visual register: primary = accent-filled CTA; secondary = outlined default; ghost = text-only; bid/offer = side-tinted click-to-trade.",
    },
    size: {
      control: "inline-radio",
      options: SIZES,
      description: "md (28px) matches --control-h; lg is the display CTA.",
    },
    kbd: {
      control: "text",
      description: "Optional keyboard shortcut hint rendered in a <kbd> chip.",
    },
    disabled: { control: "boolean" },
  },
  args: {
    variant: "secondary",
    size: "md",
    children: "Action",
  },
} satisfies Meta<typeof Button>;

export default meta;

type Story = StoryObj<typeof meta>;

/** Default secondary button — the most common usage. */
export const Default: Story = {};

/** Primary CTA — accent-filled, highest-priority action per screen. */
export const Primary: Story = { args: { variant: "primary", children: "Submit" } };

/** Ghost button — lowest-weight, text-level affordance. */
export const Ghost: Story = { args: { variant: "ghost", children: "Cancel" } };

/** Bid side — green-tinted, click-to-trade. */
export const Bid: Story = { args: { variant: "bid", children: "Buy" } };

/** Offer side — red-tinted, click-to-trade. */
export const Offer: Story = { args: { variant: "offer", children: "Sell" } };

/** Large primary with a keyboard-shortcut hint chip (⌘↵). */
export const WithKbd: Story = {
  args: { variant: "primary", size: "lg", children: "Price", kbd: "⌘↵" },
};

/** Disabled state — consistent across all variants. */
export const Disabled: Story = {
  args: { variant: "primary", children: "Submit", disabled: true },
};

/** All five variants at md size so the palette is visible at a glance. */
export const AllVariants: Story = {
  render: () => (
    <div style={{ display: "flex", gap: "var(--space-4)", flexWrap: "wrap", alignItems: "center" }}>
      {VARIANTS.map((v) => (
        <Button key={v} variant={v}>
          {v.charAt(0).toUpperCase() + v.slice(1)}
        </Button>
      ))}
    </div>
  ),
};

/** Click-to-trade row — bid and offer side by side with kbd hints. */
export const TradeRow: Story = {
  render: () => (
    <div style={{ display: "flex", gap: "var(--space-3)" }}>
      <Button variant="bid" size="lg" kbd="B">
        BUY 1M EUR
      </Button>
      <Button variant="offer" size="lg" kbd="O">
        SELL 1M EUR
      </Button>
    </div>
  ),
};
