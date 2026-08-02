/**
 * Stories — QuotingWorkspace (the dealer-quoting RFQ/IOI desk).
 *
 * The live inbox of inbound dealer requests on the left; a price + respond panel on
 * the right. A trader prices the selected request against its curve (the SAME
 * `priceRates` engine the Rates workspace uses), then RESPONDS — quoting a rate or
 * rejecting — and can ACCEPT (simulating the counterparty lifting the quote) to
 * demonstrate booking a deal + a rates position.
 *
 * One contract, two transports: the workspace talks ONLY to the CelnetTransport desk
 * seam (submitDeskRequest / respondDeskRequest / acceptDeskQuote / listDeskRequests /
 * priceRates), so the SAME lifecycle runs through the deterministic in-app source and
 * the live RfqDeskService edge. The inbox is populated only by REAL inbound requests
 * (the FIX gateway / live counterparties).
 */

import type { Meta, StoryObj } from "@storybook/react";
import { AppProvider } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { QuotingWorkspace } from "./QuotingWorkspace";

const meta = {
  title: "Workspaces/QuotingWorkspace",
  component: QuotingWorkspace,
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
          "The dealer-quoting RFQ/IOI desk. The inbound-request inbox on the left, a " +
          "price + respond panel on the right; a trader prices the selected request via " +
          "the shared priceRates engine, then quotes or rejects (and can accept to " +
          "simulate the counterparty lift + book a position). The inbox refreshes on " +
          "every push Notification and after each action.",
      },
    },
  },
} satisfies Meta<typeof QuotingWorkspace>;

export default meta;

type Story = StoryObj<typeof QuotingWorkspace>;

/**
 * The desk with live traffic. The mock transport seeds a small offline desk on
 * construction, so the inbox lists genuine PENDING requests (derived from the
 * calibrating curve pillars, never baked-in results). Selecting a request row
 * prices it against its curve via `priceRates` and enables the quote / reject
 * response; accepting simulates the counterparty lift and books a rates position.
 * Over the live edge the SAME seam is driven by real inbound FIX/counterparty flow.
 */
export const Default: Story = {};
