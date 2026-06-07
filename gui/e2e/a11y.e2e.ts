/**
 * Accessibility (axe-core) sweep of the key trader views over the LIVE edge.
 * Each view must have ZERO serious/critical WCAG 2.1 A/AA violations — the bar the
 * W12 task sets. Moderate/minor findings are reported but do not fail the gate.
 */
import { test } from "@playwright/test";

import { expectNoSeriousA11y, gotoWorkspace, openLive } from "./helpers";

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
});
