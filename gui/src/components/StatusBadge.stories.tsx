/**
 * Proof story — StatusBadge (per-row stream health: ◉ healthy / ◐ resyncing /
 * ○ stale). Correctness is visible: glyph + color + a screen-reader label, never
 * color alone. The three states render side-by-side in `AllStates`, and the
 * `health` Control flips a single badge across the contract's StreamHealth union.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { StatusBadge } from "./StatusBadge";
import type { StreamHealth } from "../data/contract";

const HEALTHS: StreamHealth[] = ["HEALTHY", "RESYNCING", "STALE"];

const meta = {
  title: "Components/StatusBadge",
  component: StatusBadge,
  tags: ["autodocs"],
  argTypes: {
    health: {
      control: "inline-radio",
      options: HEALTHS,
      description: "Stream health mapped from the contract's seq/resync state.",
    },
  },
  args: {
    health: "HEALTHY",
  },
} satisfies Meta<typeof StatusBadge>;

export default meta;

type Story = StoryObj<typeof meta>;

/** Single badge; flip the `health` Control to walk the StreamHealth union. */
export const Default: Story = {};

export const Healthy: Story = { args: { health: "HEALTHY" } };
export const Resyncing: Story = { args: { health: "RESYNCING" } };
export const Stale: Story = { args: { health: "STALE" } };

/** All three states together — the at-a-glance legend. */
export const AllStates: Story = {
  render: () => (
    <div style={{ display: "flex", gap: 16, alignItems: "center" }}>
      {HEALTHS.map((health) => (
        <span
          key={health}
          style={{ display: "inline-flex", alignItems: "center", gap: 6 }}
        >
          <StatusBadge health={health} />
          <code style={{ fontSize: 11, opacity: 0.7 }}>{health}</code>
        </span>
      ))}
    </div>
  ),
};
