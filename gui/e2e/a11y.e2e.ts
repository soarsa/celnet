/**
 * Accessibility (axe-core) sweep of the key trader views over the LIVE edge.
 * Each view must have ZERO serious/critical WCAG 2.1 A/AA violations — the bar the
 * W12 task sets. Moderate/minor findings are reported but do not fail the gate.
 *
 * PC-A11Y-WIDEN: the sweep covers the five core rail workspaces PLUS the two
 * data-dense views W12 omitted — the CubeWorkspace (a colour-encoded vol heatmap)
 * and the UniverseNavigator (a virtualised, searchable listbox) — and the
 * keyboard-shortcuts overlay (PC-SHORTCUTS), so the zero-serious claim holds
 * product-wide, not just on the simple workspaces.
 */
import { test } from "@playwright/test";

import {
  expectNoSeriousA11y,
  gotoCube,
  gotoWorkspace,
  openLive,
  openUniverseNavigator,
} from "./helpers";

const VIEWS: { id: "ticket" | "stream" | "surface" | "risk" | "book"; name: string }[] = [
  { id: "ticket", name: "Ticket" },
  { id: "stream", name: "Stream" },
  // fe-fi-migration #6: the "Surface" rail row is now the class-parametric "Market
  // Data" workspace (FX vol surface + FI rates curve lenses).
  { id: "surface", name: "Market Data" },
  { id: "risk", name: "Risk" },
  { id: "book", name: "Book" },
];

test.describe("Celnet GUI — axe a11y (zero serious/critical)", () => {
  for (const view of VIEWS) {
    test(`${view.name} view has no serious/critical a11y violations`, async ({ page }) => {
      await openLive(page);
      await gotoWorkspace(page, view.id);
      // Let the live data settle into the view before scanning.
      await page.waitForTimeout(1_500);
      await expectNoSeriousA11y(page, `${view.name} view`);
    });
  }

  test("Fixed-income OIS ticket (the FI pricing fold) has no serious/critical a11y violations", async ({
    page,
  }) => {
    await openLive(page);
    // fe-fi-migration #6: there is no separate `rates` rail row — the OIS family is
    // reached through the SHARED Ticket's class-parametric structure gallery (a rates
    // ticket under a fixed-income scope/license). Select OIS from the searchable
    // gallery, then price it so axe scans the priced result (PV / par / PV01 / DV01 +
    // the key-rate DV01 ladder) and the direction/tenor selectors, whose active state
    // uses the axe-AA high-contrast pattern (text-primary + accent underline).
    const pane = await gotoWorkspace(page, "ticket");
    await pane.getByRole("combobox", { name: "search structures" }).fill("OIS");
    await pane.getByRole("option", { name: /OIS|Overnight/i }).first().click();
    await pane.getByRole("button", { name: "Price OIS" }).click();
    await pane.getByText("Key-rate DV01 ladder").waitFor();
    await page.waitForTimeout(500);
    await expectNoSeriousA11y(page, "Fixed-income OIS ticket");
  });

  test("Cube heatmap view has no serious/critical a11y violations", async ({ page }) => {
    await openLive(page);
    await gotoCube(page);
    // Let the cube assemble its per-pair calibrated cells before scanning.
    await page.waitForTimeout(1_500);
    await expectNoSeriousA11y(page, "Cube heatmap view");
  });

  test("Scope switcher pair-universe leaf (virtualised listbox) has no serious/critical a11y violations", async ({
    page,
  }) => {
    await openLive(page);
    await openUniverseNavigator(page);
    // The leaf view renders synchronously off the in-memory universe; a short
    // settle lets the listbox rows lay out before the scan.
    await page.waitForTimeout(500);
    await expectNoSeriousA11y(page, "Scope switcher pair leaf");
  });

  test("Keyboard shortcuts overlay has no serious/critical a11y violations", async ({ page }) => {
    await openLive(page);
    // `?` opens the cheatsheet (the keyboard-first discoverability affordance).
    await page.keyboard.press("?");
    await page.getByRole("dialog", { name: "Keyboard shortcuts" }).waitFor();
    await expectNoSeriousA11y(page, "Keyboard shortcuts overlay");
  });
});
