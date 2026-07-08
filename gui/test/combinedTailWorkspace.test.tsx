import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";

import { AppProvider } from "../src/app/AppContext";
import { createMockTransport } from "../src/data/mockSource";
import { RiskWorkspace } from "../src/workspaces/RiskWorkspace";

/**
 * Hard vertical asset separation (W1): the shared Risk workspace renders EXACTLY
 * ONE asset's risk, derived from the active domain, with NO in-screen cross-asset
 * toggle. The former joint options+FI "Combined tail" LENS is no longer a
 * per-domain risk view and MUST NOT be reachable from RiskWorkspace under any
 * domain. (The pure joint-tail request seam — `seedTailRiskOptionLegs`,
 * `seedTailRiskFiPositions`, `buildJointScenarios`, `buildCombinedTailRequest` —
 * is retained and exercised by `combinedTailRisk.test.ts`; only the in-screen
 * lens is gone.)
 */

/** Render RiskWorkspace under the real offline transport at a given derived lens. */
function renderRisk(initialLens?: "fx" | "rates"): void {
  render(
    <AppProvider transport={createMockTransport()}>
      <RiskWorkspace initialLens={initialLens} />
    </AppProvider>,
  );
}

/** Assert none of the removed Combined-tail lens artifacts are on screen. */
function expectNoCombinedLens(): void {
  // No cross-asset lens tablist / toggle.
  expect(screen.queryByRole("tablist", { name: "risk asset class lens" })).toBeNull();
  // No "Combined tail" tab label.
  expect(screen.queryByText(/Combined tail/i)).toBeNull();
  // No joint-tail lens body: its panel title, its ES headline, its FI sub-book toggle.
  expect(screen.queryByText(/Joint options \+ FI tail/i)).toBeNull();
  expect(screen.queryByText(/Expected shortfall/i)).toBeNull();
  expect(screen.queryByText(/FI \(USD-SOFR\)/i)).toBeNull();
}

afterEach(() => {
  cleanup();
});

describe("Risk workspace hard vertical asset separation — Combined-tail lens is unreachable", () => {
  it("under the FX domain renders ONLY the FX scenario grid — no combined lens, no asset toggle", async () => {
    renderRisk("fx");
    // The FX single-asset content is present…
    expect(await screen.findByLabelText(/scenario heatmap/i)).toBeInTheDocument();
    // …and none of the combined-lens artifacts are reachable.
    expectNoCombinedLens();
    // The FI rates panel is not rendered under the FX lens.
    expect(screen.queryByText("Netted rates risk")).toBeNull();
  });

  it("under the FI domain renders ONLY the rates panel — no combined lens, no asset toggle", async () => {
    renderRisk("rates");
    // The FI single-asset content is present…
    expect(await screen.findByText("Netted rates risk")).toBeInTheDocument();
    // …and none of the combined-lens artifacts are reachable.
    expectNoCombinedLens();
    // The FX scenario grid is not rendered under the rates lens.
    expect(screen.queryByLabelText(/scenario heatmap/i)).toBeNull();
  });
});
