/**
 * TieringWorkspace — the trader-facing FI Tiering surface (server commit 8404bc9),
 * end-to-end over the production bundle (`?mock` transport — NO cargo, NO server).
 *
 * The ACCEPTANCE CRITERION: a NON-ADMIN trader (seeded `fi.trader@celnet.com`,
 * role TRADER — holds `quote_respond·fixed_income`, NOT admin) signs in, opens
 * Fixed Income → Tiering (a first-class rail entry, NOT behind the admin Manage
 * toggle), picks a book, enables a Flat 25 price-bps config, APPLIES it, and the
 * book round-trips to "Tiering on". It then adds a Scaled-Smoothed-Spread strategy
 * and re-applies. A second test proves the seeded ADMIN also sees the Tiering rail.
 *
 * The wire codec + workspace logic are covered by the vitest suites
 * (test/updateBookTiering.test.ts, test/tieringWorkspace.test.tsx); this spec drives
 * the real UI end-to-end as the trader — the thing the unit tests cannot.
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

import { signIn } from "./helpers";

type Page = import("@playwright/test").Page;

async function selectDomain(page: Page, name: string): Promise<void> {
  await page.getByRole("tablist", { name: "product domains" }).getByRole("tab", { name }).click();
}

async function openWorkspace(page: Page, label: string): Promise<void> {
  await page
    .getByRole("complementary", { name: "workspaces" })
    .locator(`button[title^="${label} ("]`)
    .click();
}

test("a NON-ADMIN trader retunes a book's tiering under Fixed Income → Tiering; it round-trips", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });

  // Sign in as the seeded NON-ADMIN trader (role TRADER). It holds
  // `quote_respond·fixed_income` but is NOT admin.
  await page.goto("/?mock");
  await signIn(page, "fi.trader@celnet.com", "password");
  await expect(page.getByRole("complementary", { name: "workspaces" })).toBeVisible();

  // Prove NON-ADMIN: the Administration domain tab is not present for this identity.
  await expect(
    page.getByRole("tablist", { name: "product domains" }).getByRole("tab", { name: "Administration" }),
  ).toHaveCount(0);

  // Tiering is a FIRST-CLASS rail entry under Fixed Income (not admin-gated).
  await selectDomain(page, "Fixed Income");
  await openWorkspace(page, "Tiering");
  await expect(page.getByText("Widen and/or skew a book's consolidated composite")).toBeVisible();

  // The seeded `US Treasuries` book lists with its current tiering state = OFF.
  const bookBtn = page.getByRole("button", { name: /US Treasuries/ });
  await expect(bookBtn).toBeVisible();
  await expect(bookBtn.getByText("Tiering off")).toBeVisible();
  await bookBtn.click();

  // Enable a Flat 25 price-bps config in the shared TieringEditor.
  const tiering = page.getByRole("region", { name: "outbound tiering configuration" });
  await tiering.getByLabel("enable outbound tiering").check();
  await expect(tiering.getByLabel("Spread unit")).toHaveValue("PRICE_BPS");
  await expect(tiering.getByText("Flat markup", { exact: true })).toBeVisible();
  await expect(tiering.getByLabel("Half-spread H")).toHaveValue("25");

  // APPLY — the trader-accessible UpdateBookTiering RPC (mock) fires and the book
  // round-trips: the success badge shows and the roster badge flips to "Tiering on".
  const applyBtn = page.getByRole("button", { name: "Apply tiering" });
  await expect(applyBtn).toBeEnabled();
  await applyBtn.click();
  await expect(page.getByText("✓ Applied")).toBeVisible();
  await expect(bookBtn.getByText("Tiering on")).toBeVisible();

  // Add a Scaled-Smoothed-Spread strategy (its distinctive params appear) + re-apply.
  await tiering.getByRole("button", { name: "+ Scaled Smoothed Spread" }).click();
  await expect(tiering.getByText("Scaled Smoothed Spread", { exact: true })).toBeVisible();
  await expect(tiering.getByLabel("Smoothing weight w (0–1]")).toBeVisible();
  await expect(applyBtn).toBeEnabled();
  await applyBtn.click();
  await expect(page.getByText("✓ Applied")).toBeVisible();

  // Screenshot the delivered surface for the report.
  await page.screenshot({ path: "test-results/tiering-workspace-trader.png" });

  // A11y pass SCOPED to the tiering editor (the surface this task delivers). Freeze
  // animations so axe samples resting colours. The whole-page scan surfaces the
  // app's PRE-EXISTING dark-theme accent-contrast debt (AuthMenu role badge +
  // `.segBtnActive` controls) independent of this change, so we validate the editor.
  await page.addStyleTag({
    content: "*,*::before,*::after{animation:none!important;transition:none!important;}",
  });
  const results = await new AxeBuilder({ page })
    .include('[aria-label="outbound tiering configuration"]')
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  const serious = results.violations.filter(
    (v) => v.impact === "serious" || v.impact === "critical",
  );
  expect(serious, JSON.stringify(serious.map((v) => ({ id: v.id, nodes: v.nodes.length })))).toEqual(
    [],
  );
});

test("the seeded admin also sees the Tiering rail under Fixed Income", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/?mock");
  await signIn(page); // default seeded admin
  await expect(page.getByRole("complementary", { name: "workspaces" })).toBeVisible();
  await selectDomain(page, "Fixed Income");
  await expect(
    page.getByRole("complementary", { name: "workspaces" }).locator('button[title^="Tiering ("]'),
  ).toBeVisible();
});
