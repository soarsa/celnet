/**
 * Stories — StreamWorkspace (GUI-DESIGN §4.2).
 *
 * The RFS blotter — the RESTING STATE of the platform. A living table of streaming
 * two-ways multiplexed over one StreamSession. Only changed numbers flash (calm
 * under fire); per-row stream health is honest (◉/◐/○ from real seq/resync state).
 *
 * Scale features on display: row virtualisation (`useVirtualWindow`), GROUP/COLLAPSE
 * by pair or tenor with aggregation headers (ΣΔ / σ̄ / health), sortable columns,
 * and the trend-mode selector (PREMIUM / ATM vol / RR / BF / spot / fwd).
 *
 * Token contract: layout gaps, row heights, and all color references come from
 * var(--row-h) / var(--space-*) / var(--bid) / var(--offer) / var(--accent) etc.
 * defined in src/design/tokens.css — no inline pixel/hex literals.
 */

import type { Decorator, Meta, StoryObj } from "@storybook/react";
import { AppProvider } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { StreamWorkspace } from "./StreamWorkspace";

const withAppContext: Decorator = (Story) => (
  <AppProvider transport={createMockTransport()}>
    <Story />
  </AppProvider>
);

const meta = {
  title: "Workspaces/StreamWorkspace",
  component: StreamWorkspace,
  decorators: [withAppContext],
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    docs: {
      description: {
        component:
          "The RFS blotter — the RESTING STATE (GUI-DESIGN §4.2). A living table " +
          "of streaming two-ways multiplexed over one StreamSession. Virtualised for " +
          "IB-scale books; grouped, collapsed, and sorted; trend-mode selector drives " +
          "the sparkline column across PREMIUM / ATM vol / RR / BF / spot / forward. " +
          "Click a bid/offer cell to trade (Execute, typed Executed/StreamReject toast).",
      },
    },
  },
} satisfies Meta<typeof StreamWorkspace>;

export default meta;

type Story = StoryObj<typeof meta>;

/**
 * Blotter at startup — the mock transport seeds a small set of RFS subscriptions
 * across EUR/USD, GBP/USD, and USD/JPY at canonical tenors. The virtualised list,
 * group header aggregates (ΣΔ / σ̄ / ◉ count), and sparkline trend column are all
 * rendered live against the deterministic mock. Click a bid or offer cell to fire
 * an Execute and see the typed toast (✓ Executed or ✕ StreamReject).
 */
export const Default: Story = {};

/**
 * Grouped by tenor — flips the group-by dimension from pair to tenor so the
 * blotter folds the book by expiry horizon. Demonstrates that the group-by
 * control is stateful: the aggregation-header format changes accordingly and the
 * virtualiser windows both the header and data rows in the new arrangement.
 *
 * The story still boots the full mock transport — the group-by pivot is a
 * client-side state change, not a new RPC, so the streamed numbers stay live.
 */
export const GroupedByTenor: Story = {
  name: "Grouped by tenor",
  render: () => {
    // The component owns its groupBy state; this story shows the blotter as-is
    // (the user can flip it in the Controls toolbar). We document the feature via
    // the story name + description rather than faking an initial state we'd have
    // to maintain through the component's internals.
    return <StreamWorkspace />;
  },
  parameters: {
    docs: {
      description: {
        story:
          "Identical component boot; use the 'Tenor' Group button in the blotter " +
          "controls toolbar to observe the grouping pivot. The aggregation headers " +
          "recalculate over the new groups and the virtualiser adjusts in one frame.",
      },
    },
  },
};

/**
 * Narrow viewport — verifies the column set, overflow behaviour, and the sticky
 * header at mobile/tablet widths. Optional columns (Structure / Δ / σ) can be
 * toggled via the column toolbar to reclaim width.
 */
export const NarrowViewport: Story = {
  name: "Narrow viewport",
  parameters: {
    viewport: { defaultViewport: "mobile2" },
    docs: {
      description: {
        story:
          "At narrow widths the optional columns (Structure / Δ / σ) can be toggled " +
          "off via the column toolbar; the grid adapts track widths from the same " +
          "CSS custom-property template string used for every row.",
      },
    },
  },
};
