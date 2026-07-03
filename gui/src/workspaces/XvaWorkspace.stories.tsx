/**
 * Stories — XvaWorkspace (the counterparty valuation-adjustment surface).
 *
 * A trader assembles a netting set of FX vanillas, sets each party's survival
 * (hazard) curve, LGDs and the funding spread, and prices the set's all-in CVA /
 * DVA / FVA against the single canonical contract (`PricingService.PriceXva`).
 *
 * The figures are REAL and SHARED: the workspace calls the transport's `priceXva` —
 * offline it runs the deterministic in-browser estimator (`src/data/xvaPricing`)
 * that reproduces the server's `celnet_xva::compute_xva` aggregation; over the WS
 * mirror it issues the live `price_xva` RPC. One contract, two transports.
 *
 * Honest exposure-profile boundary: the wire `XvaResult` carries ONLY the four
 * scalar adjustments — the simulated exposure PROFILE (EPE/ENE per bucket) is a
 * server-internal, so the exposure fan is drawn from a deterministic, clearly-
 * labelled ILLUSTRATIVE profile, never presented as a live valuation.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { AppProvider } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { XvaWorkspace } from "./XvaWorkspace";

const meta = {
  title: "Workspaces/XvaWorkspace",
  component: XvaWorkspace,
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
          "The counterparty valuation-adjustment (XVA) surface. Assemble a netting set " +
          "of FX vanillas, set each party's hazard curve / LGD and the funding spread, " +
          "and price the all-in CVA / DVA / FVA via the canonical PriceXva contract " +
          "(offline: the deterministic in-browser estimator reproducing the server's " +
          "compute_xva aggregation). The scalar adjustments are the priced result; the " +
          "exposure fan is a clearly-labelled illustrative profile.",
      },
    },
  },
} satisfies Meta<typeof XvaWorkspace>;

export default meta;

type Story = StoryObj<typeof XvaWorkspace>;

/**
 * The XVA desk with a seeded netting set. The workspace mounts with an editable set
 * of FX vanillas and default hazard/LGD/funding inputs, and prices CVA / DVA / FVA
 * against the mock `priceXva` on mount. Editing any trade or credit input reprices
 * live; the four scalar adjustments update against the deterministic estimator while
 * the illustrative exposure fan redraws from the current netting set.
 */
export const Default: Story = {};
