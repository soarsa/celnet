/**
 * Stories — MarketDataWorkspace (`fe-fi-migration` #2).
 *
 * The ONE class-parametric MARKET-DATA workspace. There is no FX-vs-FI split any
 * more: a single workspace renders the market-data LENS for the active asset class
 * — FX (options / metals) → the broker vol-surface (mark → arb-gate → publish, the
 * 3D mesh + smile family, the vol-cube drill-in); fixed-income → the rates curve
 * surface (pillar editor + build-by-instrument-reference + the term structure). The
 * lens toggle is offered only for the classes the signed-in identity can `view` AND
 * the firm is licensed for; signed out, `can` is permissive so both lenses show.
 *
 * `initialLens` is the entry-point default: the `surface` rail row opens the FX
 * vol-surface lens, the `curve` rail row opens the fixed-income curve lens. A single
 * available lens renders directly with no toggle.
 */

import type { Meta, StoryObj } from "@storybook/react";
import { AppProvider } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { MarketDataWorkspace } from "./MarketDataWorkspace";

const meta = {
  title: "Workspaces/MarketDataWorkspace",
  component: MarketDataWorkspace,
  // Inlined (untyped) so the meta infers the component's optional-arg type — a
  // top-level `Decorator` const clashes with StrictArgs under
  // exactOptionalPropertyTypes now that MarketDataWorkspace takes `{ initialLens? }`
  // (same fix the BookWorkspace fold used).
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
          "The one class-parametric market-data workspace (fe-fi-migration #2). It " +
          "renders the market-data lens for the active asset class: FX → the broker " +
          "vol-surface (SurfaceWorkspace body), fixed-income → the rates curve surface " +
          "(CurveWorkspace body). The lens toggle appears only for classes the identity " +
          "can view and the firm is licensed for; both lens bodies are composed verbatim.",
      },
    },
  },
} satisfies Meta<typeof MarketDataWorkspace>;

export default meta;

type Story = StoryObj<typeof MarketDataWorkspace>;

/**
 * FX lens (the default entry point). Boots with `initialLens="fx"`, mounting the
 * SurfaceWorkspace body: the mock transport seeds a three-pillar EUR/USD delta-space
 * surface (3D mesh + 1M smile slice + broker marking grid). Signed out, both lens
 * chips are available, so the FX/Fixed-Income toggle is visible at the top.
 */
export const Default: Story = {};

/**
 * Fixed-income lens — the `curve` rail entry point. `initialLens="rates"` opens the
 * CurveWorkspace body directly: the pillar editor + build-by-instrument-reference
 * mode + the YieldCurve term structure, all against the mock transport's seeded
 * USD SOFR curve. The FX/Fixed-Income toggle stays available (permissive `can`).
 */
export const FixedIncomeLens: Story = {
  name: "Fixed-income (rates curve) lens",
  args: { initialLens: "rates" },
  parameters: {
    docs: {
      description: {
        story:
          "Entering via the curve rail row (initialLens='rates') mounts the rates " +
          "curve lens: the pillar editor and instrument-reference build mode over the " +
          "seeded USD SOFR curve. The market-data workflow is now identical across " +
          "asset classes — the FX/FI silo is folded into one workspace.",
      },
    },
  },
};
