/**
 * Stories — SurfaceWorkspace (GUI-DESIGN §4.3).
 *
 * The "show me why" view. Three linked views of one marked surface: the 3D
 * WebGPU-ready mesh, the per-tenor smile overlay, and the broker marking panel
 * (ATM / 25Δ&10Δ RR&BF). Editing ATM/RR/BF reprices live with an arb banner if
 * a butterfly constraint breaks; publishing flows through the same MarkSurface API
 * the SDK and Excel use.
 *
 * The asset-class-aware family switch gates marked families to the active underlier:
 * FX shows the five delta-space calibration families; a non-FX underlier renders
 * the honest "no marked surface" state — nothing fabricated (GUIDE.md rule 2).
 *
 * Token contract: all color, spacing, and type references come from the Aurora
 * cascade. The ramp gradient (low→high) uses var(--bid)/var(--offer)-derived
 * OKLCH values from viz/ramp.ts — no inline hex.
 */

import type { Decorator, Meta, StoryObj } from "@storybook/react";
import { AppProvider } from "../app/AppContext";
import { createMockTransport } from "../data/mockSource";
import { SurfaceWorkspace } from "./SurfaceWorkspace";

const withAppContext: Decorator = (Story) => (
  <AppProvider transport={createMockTransport()}>
    <Story />
  </AppProvider>
);

const meta = {
  title: "Workspaces/SurfaceWorkspace",
  component: SurfaceWorkspace,
  decorators: [withAppContext],
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    docs: {
      description: {
        component:
          "The 'show me why' workspace (GUI-DESIGN §4.3). Three linked views of " +
          "one marked surface: a 3D mesh, per-tenor smile overlay, and broker " +
          "marking panel (ATM / 25Δ&10Δ RR&BF). Editing handle values reprices live " +
          "with an arb banner on butterfly violation; Publish flows through the same " +
          "MarkSurface API the SDK and Excel use. The vol cube is reachable via the " +
          "Surface/Cube view toggle without touching Shell/AppContext.",
      },
    },
  },
} satisfies Meta<typeof SurfaceWorkspace>;

export default meta;

type Story = StoryObj<typeof meta>;

/**
 * Surface mark view — the default. The mock transport seeds a three-pillar
 * (1M/3M/1Y) EUR/USD delta-space surface; the 3D mesh, the 1M smile slice, and
 * the broker marking grid all render against the deterministic mock. Clicking a
 * tenor row in the marking grid selects it for editing; modifying an ATM/RR/BF
 * input updates the preview live and shows an arb banner when the butterfly
 * constraint breaks.
 */
export const Default: Story = {};

/**
 * Mark view — demonstrates the asset-class gate. The FX class exposes the five
 * delta-space calibration family chips (market-hedge / stochastic-vol /
 * parametric / parametric-surface / eSSVI); selecting one re-marks the live
 * surface server-side and bumps the surface version badge.
 *
 * To reach the non-FX unavailable state, switch the underlier rail to a metal or
 * crypto underlier via the scope switcher (⌘P); the workspace renders the honest
 * "no marked surface for {class}" state without fabricating data.
 */
export const SurfaceMarkView: Story = {
  name: "Surface mark view (FX default)",
  parameters: {
    docs: {
      description: {
        story:
          "Boots with EUR/USD and a three-tenor mock surface. The five calibration " +
          "family chips appear in the Marking panel; the selected smile's provenance " +
          "(model family + handle count + epoch) is visible at the bottom of the panel.",
      },
    },
  },
};

/**
 * Vol cube view — toggle the view to the pair×tenor×delta heatmap. The cube reads
 * surfaces across multiple pairs from the mock transport; each cell is tinted on
 * the same perceptual diverging ramp as the RiskWorkspace scenario grid. Clicking
 * a heatmap cell drills back into the mark view, re-pointing the selected (pair,
 * tenor, delta) without a workspace navigation step.
 *
 * Note: this story renders the same component; the view='cube' state is reached
 * via the Surface/Cube toggle button rendered at the top-right of the panel.
 */
export const VolCubeView: Story = {
  name: "Vol cube view (pair×tenor×delta)",
  parameters: {
    docs: {
      description: {
        story:
          "Click the 'Cube' chip to switch view. The heatmap renders the full " +
          "pair×tenor×delta vol cube from the mock; clicking a cell fires drillFromCube " +
          "and returns to the Surface view re-pointed at that (tenor, delta).",
      },
    },
  },
};
