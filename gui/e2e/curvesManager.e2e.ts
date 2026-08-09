/**
 * Curves multi-curve manager — OFFLINE e2e against the in-app `?mock` transport
 * (NO cargo, NO celnet-server), the wire contract of server commit 38bcff9a. Drives
 * the deliverable end to end in a real browser:
 *   1. FX Options domain → Market Data reads "Market Data" and renders the VOL
 *      SURFACE (never "Curves").
 *   2. Fixed Income domain → the same shared row reads "Curves" and opens the
 *      multi-curve dashboard listing the seeded curves (usd-sofr primary +
 *      usd-sofr-street) with the per-currency Primary badge.
 *   3. New curve → the definition editor: metadata + the WIRE-REAL interpolation
 *      radios (log-linear ↔ monotone-convex, both enabled); Create persists it.
 *   4. The active-curve picker + Pillars lens edits + Saves the selected curve.
 *   5. Delete a non-primary curve; deleting the primary surfaces the server's
 *      failed_precondition guidance.
 * Closes each populated surface with an axe a11y pass (0 serious/critical) and
 * screenshots the dashboard + editor for the verification record.
 */
import { expect, test, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

import { signIn } from "./helpers";

function domains(page: Page) {
  return page.getByRole("tablist", { name: "product domains" });
}

function rail(page: Page) {
  return page.getByRole("complementary", { name: "workspaces" });
}

/**
 * axe SCOPED to the Curves manager surface (the deliverable under test), asserting
 * 0 serious/critical. Scoping — the codebase's `help.e2e` idiom — keeps the scan on
 * THIS surface and off transient global chrome (e.g. the header notification toast,
 * whose contrast is a separate, pre-existing component concern). Freezes animations
 * first so axe samples the resting UI.
 */
async function axeCurves(page: Page, context: string): Promise<void> {
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation:none!important;transition:none!important;}",
  });
  const results = await new AxeBuilder({ page })
    .include('[data-testid="curves-manager"]')
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  const serious = results.violations.filter(
    (v) => v.impact === "serious" || v.impact === "critical",
  );
  expect(
    serious,
    `${context}: ${JSON.stringify(serious.map((v) => ({ id: v.id, help: v.help })))}`,
  ).toEqual([]);
}

test("Curves manager: relabel, dashboard, CRUD, interpolation, delete-guard", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/?mock");
  await signIn(page);
  await expect(rail(page)).toBeVisible();

  // 1) FX Options (default domain): the shared Market-Data row reads "Market Data"
  // — NOT "Curves" — and renders the vol surface.
  await expect(rail(page).getByRole("button", { name: /Market Data/ })).toBeVisible();
  await expect(rail(page).getByRole("button", { name: /Curves/ })).toHaveCount(0);
  await rail(page).getByRole("button", { name: /Market Data/ }).click();
  await expect(page.getByRole("group", { name: "surface view" })).toBeVisible();

  // 2) Fixed Income: the SAME row is relabelled "Curves" and opens the dashboard.
  await domains(page).getByRole("tab", { name: "Fixed Income", exact: true }).click();
  const curvesBtn = rail(page).getByRole("button", { name: /Curves/ });
  await expect(curvesBtn).toBeVisible();
  await expect(rail(page).getByRole("button", { name: /^.*Market Data.*$/ })).toHaveCount(0);
  await curvesBtn.click();

  await expect(page.getByRole("heading", { name: "Curve definitions" })).toBeVisible();
  await expect(page.getByRole("button", { name: "USD SOFR", exact: true })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "USD SOFR (street)", exact: true }),
  ).toBeVisible();
  // Exactly one per-currency primary.
  await expect(page.getByText("Primary", { exact: true })).toHaveCount(1);
  // The interpolation of each seeded curve is shown.
  await expect(page.getByText("Log-linear (DF)")).toBeVisible();
  await expect(page.getByText("Monotone convex (forward)")).toBeVisible();
  await page.screenshot({ path: "test-results/curves-dashboard.png", fullPage: true });
  await axeCurves(page, "curves dashboard");

  // 3) New curve → the definition editor with WIRE-REAL interpolation radios.
  await page.getByRole("button", { name: /New curve/ }).click();
  await expect(page.getByRole("heading", { name: "New curve" })).toBeVisible();
  await page.getByLabel("curve display name").fill("USD SOFR mark");
  const monotone = page.getByRole("radio", { name: /Monotone convex/ });
  const logLinear = page.getByRole("radio", { name: /Log-linear \(DF\)/ });
  // Both schemes are enabled — the interpolation now rides the wire (no fake "Target").
  await expect(logLinear).toBeEnabled();
  await expect(monotone).toBeEnabled();
  await monotone.check();
  await expect(monotone).toBeChecked();
  await page.screenshot({ path: "test-results/curves-editor.png", fullPage: true });
  await axeCurves(page, "curve definition editor");
  await page.getByRole("button", { name: /Create curve/ }).click();

  // Back on the dashboard, the new curve is listed with its monotone-convex scheme.
  await expect(
    page.getByRole("button", { name: "USD SOFR mark", exact: true }),
  ).toBeVisible();
  await expect(page.getByText("Monotone convex (forward)")).toHaveCount(2);

  // 4) The picker + Pillars lens edit + Save the selected curve.
  await page.getByRole("tab", { name: "Pillars", exact: true }).click();
  await expect(page.getByRole("combobox", { name: "active curve" })).toBeVisible();
  const firstRate = page.getByLabel(/par rate in percent/).first();
  await firstRate.fill("4.55");
  await page.getByRole("button", { name: /Save pillars/ }).click();
  await expect(page.getByText("Saved", { exact: true })).toBeVisible();

  // 5) Delete a NON-primary curve, then confirm deleting the primary is guarded.
  await page.getByRole("tab", { name: "Dashboard", exact: true }).click();
  await page
    .locator("tr", { hasText: "USD SOFR (street)" })
    .getByRole("button", { name: "Delete" })
    .click();
  await expect(
    page.getByRole("button", { name: "USD SOFR (street)", exact: true }),
  ).toHaveCount(0);

  // Deleting the primary (usd-sofr) while a USD sibling remains → failed_precondition.
  await page
    .locator("tr")
    .filter({ hasText: "Primary" })
    .getByRole("button", { name: "Delete" })
    .click();
  await expect(page.getByText(/primary curve for USD/i)).toBeVisible();

  // Final a11y pass on the mutated dashboard.
  await axeCurves(page, "curves dashboard after CRUD");
});
