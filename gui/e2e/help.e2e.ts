/**
 * Trader Help & guided-tutorials system — OFFLINE e2e over the production bundle
 * (`?mock` transport: NO cargo, NO celnet-server). Drives the deliverable end to
 * end as the seeded admin:
 *   1. Fixed Income → Tiering, enable a book's tiering (Flat 25 price bps) and
 *      confirm the LIVE preview turns 99.50 / 99.60 into 99.30 / 99.80.
 *   2. Open a strategy's "?" → the rich in-app HelpPanel with the worked example.
 *   3. "Walk me through it" → the guided tutorial spotlights the strategy, the bps
 *      input and the preview; step Next / Back / Skip.
 *   4. The header Help center: search "bid offer tiering" → the entry + a tour launch.
 *   5. axe (0 serious/critical) SCOPED to the delivered surfaces, + screenshots.
 *
 * The whole-page axe scan surfaces the app's PRE-EXISTING dark-theme accent
 * contrast debt (AuthMenu badge, segmented controls — see tieringConfig.e2e.ts), so
 * the a11y gate is scoped to the help/tour surfaces this task delivers.
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { signIn } from "./helpers";

async function selectDomain(page: Page, name: string): Promise<void> {
  await page.getByRole("tablist", { name: "product domains" }).getByRole("tab", { name }).click();
}

async function openWorkspace(page: Page, label: string): Promise<void> {
  await page
    .getByRole("complementary", { name: "workspaces" })
    .locator(`button[title^="${label} ("]`)
    .click();
}

/** axe SCOPED to a selector; asserts 0 serious/critical (freezes animations first). */
async function axeScoped(page: Page, selector: string, context: string): Promise<void> {
  await page.addStyleTag({
    content: "*,*::before,*::after{animation:none!important;transition:none!important;}",
  });
  const results = await new AxeBuilder({ page })
    .include(selector)
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  const serious = results.violations.filter(
    (v) => v.impact === "serious" || v.impact === "critical",
  );
  expect(
    serious,
    `${context}: ${JSON.stringify(serious.map((v) => ({ id: v.id, nodes: v.nodes.length })))}`,
  ).toEqual([]);
}

test("help panels, the Help center, and a guided tutorial work end to end", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/?mock");
  await signIn(page);
  await expect(page.getByRole("complementary", { name: "workspaces" })).toBeVisible();

  // 1) Fixed Income → Tiering; enable the seeded book's tiering (default Flat 25 bps).
  await selectDomain(page, "Fixed Income");
  await openWorkspace(page, "Tiering");
  const editor = page.getByRole("region", { name: "outbound tiering configuration" });
  await expect(editor).toBeVisible();
  await editor.getByLabel("enable outbound tiering").check();

  // The LIVE worked-example preview: LP 99.50 / 99.60 → you stream 99.30 / 99.80.
  const preview = page.getByLabel("sample outbound two-way");
  await expect(preview).toContainText("99.30");
  await expect(preview).toContainText("99.80");

  // 2) A pricing feature's "?" opens the rich in-app help panel with the example.
  await editor.getByRole("button", { name: "Help for the Flat markup strategy" }).click();
  const panel = page.getByTestId("help-panel");
  await expect(panel).toBeVisible();
  await expect(panel.getByRole("heading", { name: "Flat markup" })).toBeVisible();
  await expect(panel.getByRole("heading", { name: "Worked example" })).toBeVisible();
  await expect(panel.getByRole("heading", { name: "How to configure" })).toBeVisible();
  await expect(panel.getByText(/99\.30/).first()).toBeVisible();
  await panel.screenshot({ path: "test-results/help-panel.png" });
  await axeScoped(page, '[data-testid="help-panel"]', "help panel");
  await panel.getByRole("button", { name: "Done" }).click();
  await expect(panel).toBeHidden();

  // 3) The header Help center: search "bid offer tiering" → the ranked entry + a tour.
  await page.getByRole("button", { name: "open help and tutorials" }).click();
  const center = page.getByTestId("help-center");
  await expect(center).toBeVisible();
  await center.getByLabel("search help topics").fill("bid offer tiering");
  await expect(center.getByText("Bid / offer tiering")).toBeVisible();
  await center.screenshot({ path: "test-results/help-center.png" });
  await axeScoped(page, '[data-testid="help-center"]', "help center");

  // 4) "Walk me through it" on the top result launches "How bid / offer tiering works".
  await center.getByRole("button", { name: /Walk me through it/ }).first().click();
  const tip = page.getByTestId("tour-tooltip");
  await expect(tip).toBeVisible();
  await expect(tip).toContainText("How bid / offer tiering works");
  await expect(tip).toContainText("1 / 4");

  // Step through: center → the strategy → the bps input → the live preview.
  await tip.getByRole("button", { name: "Next" }).click();
  await expect(tip).toContainText("The strategy");
  await tip.getByRole("button", { name: "Next" }).click();
  await expect(tip).toContainText("The half-spread input");
  await tip.getByRole("button", { name: "Next" }).click();
  await expect(tip).toContainText("The live preview");
  await page.screenshot({ path: "test-results/tour-step.png" });
  await axeScoped(page, '[data-testid="tour-tooltip"]', "tour tooltip");

  // Back reverses; Skip closes.
  await tip.getByRole("button", { name: "Back" }).click();
  await expect(tip).toContainText("The half-spread input");
  await tip.getByRole("button", { name: "Skip" }).click();
  await expect(tip).toBeHidden();
});
