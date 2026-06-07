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
  { id: "surface", name: "Surface" },
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

  test("Cube heatmap view has no serious/critical a11y violations", async ({ page }) => {
    await openLive(page);
    await gotoCube(page);
    // Let the cube assemble its per-pair calibrated cells before scanning.
    await page.waitForTimeout(1_500);
    await expectNoSeriousA11y(page, "Cube heatmap view");
  });

  test("Universe navigator (virtualised listbox) has no serious/critical a11y violations", async ({
    page,
  }) => {
    await openLive(page);
    await openUniverseNavigator(page);
    // The overlay renders synchronously off the in-memory universe; a short
    // settle lets the listbox rows lay out before the scan.
    await page.waitForTimeout(500);
    await expectNoSeriousA11y(page, "Universe navigator");
  });

  test("Keyboard shortcuts overlay has no serious/critical a11y violations", async ({ page }) => {
    await openLive(page);
    // `?` opens the cheatsheet (the keyboard-first discoverability affordance).
    await page.keyboard.press("?");
    await page.getByRole("dialog", { name: "Keyboard shortcuts" }).waitFor();
    await expectNoSeriousA11y(page, "Keyboard shortcuts overlay");
  });
});
