/**
 * Risk Dashboard — OFFLINE e2e against the in-app `?mock` transport (no cargo edge,
 * no server). Drives the production bundle end to end as the seeded admin to prove
 * the two Dashboard-tab changes live:
 *   1. NUMERIC ALIGNMENT — the roster's numeric columns are right-aligned with
 *      tabular figures, so digits line up in a column (Change 1);
 *   2. TENOR/INSTRUMENT DRILL-DOWN — each portfolio row expands (keyboard-operable
 *      disclosure, aria-expanded) to reveal its "By tenor" + "By instrument"
 *      breakdown, folded client-side from the routed deals (Change 2);
 *   3. axe on the expanded surface (0 serious/critical);
 *   4. screenshot the expanded drill-down.
 */
import { expect, test } from "@playwright/test";

import { gotoMockView } from "./fidelityHelpers";
import { expectNoSeriousA11y } from "./helpers";

test("Risk Dashboard: numeric columns align + a portfolio expands to a tenor/instrument breakdown", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });

  // Open the consolidated Risk host (FI) as the seeded admin; scope to the active pane.
  const pane = await gotoMockView(page, "riskdashboard");
  await expect(pane.getByTestId("risk-tab-dashboard")).toHaveAttribute("aria-pressed", "true");

  // --- 1 · numeric columns are right-aligned + tabular (Change 1) -------------
  const netHeader = pane.getByRole("columnheader", { name: "Net", exact: true });
  await expect(netHeader).toBeVisible();
  const headerStyle = await netHeader.evaluate((el) => {
    const cs = getComputedStyle(el);
    return { textAlign: cs.textAlign, variant: cs.fontVariantNumeric };
  });
  expect(headerStyle.textAlign).toBe("right");
  expect(headerStyle.variant).toContain("tabular-nums");

  // A numeric BODY cell (the first row's Net) is right-aligned + tabular too.
  const firstNumCell = pane.locator('tbody td[class*="num"]').first();
  await expect(firstNumCell).toBeVisible();
  const cellStyle = await firstNumCell.evaluate((el) => {
    const cs = getComputedStyle(el);
    return { textAlign: cs.textAlign, variant: cs.fontVariantNumeric };
  });
  expect(cellStyle.textAlign).toBe("right");
  expect(cellStyle.variant).toContain("tabular-nums");

  // --- 2 · a portfolio row expands to its tenor/instrument breakdown ----------
  const expandBtn = pane.locator('button[data-testid^="risk-expand-"]').first();
  await expect(expandBtn).toBeVisible();
  await expect(expandBtn).toHaveAttribute("aria-expanded", "false");

  // The disclosure is a real, keyboard-operable button — activate it by keyboard.
  await expandBtn.focus();
  await page.keyboard.press("Enter");
  await expect(expandBtn).toHaveAttribute("aria-expanded", "true");

  // The controlled breakdown region reveals; it holds either the two lenses (when
  // the portfolio has routed deals) or the honest empty note (when it has none).
  const controls = await expandBtn.getAttribute("aria-controls");
  expect(controls).toBeTruthy();
  const region = pane.locator(`#${controls}`);
  await expect(region).toBeVisible();
  const regionText = (await region.textContent()) ?? "";
  expect(/By tenor|No routed fills to break down/.test(regionText)).toBe(true);

  // --- 3 · axe on the expanded surface (0 serious/critical) -------------------
  await expectNoSeriousA11y(page, "Risk Dashboard — drill-down expanded");

  // --- 4 · screenshot the expanded drill-down ---------------------------------
  await page.screenshot({ path: "e2e-artifacts/risk-dashboard-drilldown.png", fullPage: true });
});
