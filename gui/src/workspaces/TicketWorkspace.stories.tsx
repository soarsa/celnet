/**
 * Stories — TicketWorkspace (GUI-DESIGN §4.1).
 *
 * THE differentiator: one card that is the analytics surface AND the executable.
 * Build a structure, see a live two-way + full 14-Greek set + conventions on the
 * face, and hit it without changing screens. The story boots the full app context
 * (AppProvider + mock transport) so every rendered story exercises the real
 * component tree against a deterministic in-app datasource, not a fabricated shell.
 *
 * Token contract: no raw colours or pixel literals — every gap/radius/color ref
 * flows from var(--space-*) / var(--radius-*) / semantic token variables set in
 * src/design/tokens.css via the Aurora cascade that preview.ts layers in.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { AppProvider } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { TicketWorkspace } from "./TicketWorkspace";

const meta = {
  title: "Workspaces/TicketWorkspace",
  component: TicketWorkspace,
  // Wrap every story in the real app context against the deterministic mock.
  // Inlined (not a typed `Decorator` const) so it infers against the ticket's own
  // optional props (`initialStructure`) without a StrictArgs conflict.
  decorators: [
    (Story) => (
      <AppProvider transport={createMockTransport()}>
        <Story />
      </AppProvider>
    ),
  ],
  tags: ["autodocs"],
  parameters: {
    // The ticket card fills ~640px of vertical space at rest; give it room.
    layout: "fullscreen",
    docs: {
      description: {
        component:
          "THE differentiator (GUI-DESIGN §4.1). One card that is the analytics " +
          "surface AND the executable: build a structure, see a live two-way + " +
          "the full 14-Greek set + conventions on the face, and hit it without " +
          "changing screens. Strike solve is inline (delta-keyed legs resolve " +
          "server-side); last-look window is a visible depleting ring.",
      },
    },
  },
} satisfies Meta<typeof TicketWorkspace>;

export default meta;

type Story = StoryObj<typeof TicketWorkspace>;

/**
 * Resting state — the ticket at startup, EUR/USD pair, 1M tenor, Risk Reversal.
 * The structure gallery, expiry row, and payoff chart preview are all visible.
 * Click "Request quote" to fire through the mock transport and observe the
 * two-way, 14-Greek strip, and last-look ring.
 */
export const Default: Story = {};

/**
 * The resting state with no Args override — demonstrates that the component
 * self-seeds from AppContext (pair + conventions + market) with zero prop drilling.
 * All token references in the component use var(--…) from the Aurora cascade; the
 * Storybook toolbar lets you flip dark/light and high-contrast to verify every mode.
 */
export const RestingState: Story = {
  name: "Resting state (pre-quote)",
};

/**
 * Full-width layout — the ticket at 100vw to verify the payoff chart and
 * structure gallery lay out correctly at wider viewports (IB second-monitor use).
 */
export const FullWidth: Story = {
  name: "Full-width layout",
  parameters: {
    viewport: { defaultViewport: "tablet" },
  },
};
