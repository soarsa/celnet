/**
 * Capability-overlay editor — LIVE e2e against the real demo edge (Enforce).
 *
 * Drives the real Admin workspace through the production bundle and the live WS
 * mirror: sign in as the seeded admin, create a trader, open the capability
 * matrix, GRANT one capability and DENY another, save, and confirm the matrix
 * re-renders from the server-resolved `effective` set — the deny visibly wins and
 * the "sessions ended" note appears. Then an axe pass on the surface.
 */
import { test, expect } from "@playwright/test";

import { openLive, expectNoSeriousA11y } from "./helpers";

// A unique email per run — the demo edge persists identity across runs, so a
// fixed email would collide and leave the create dialog open over the table.
const TRADER_EMAIL = `perms-trader-${Date.now()}@celnet.com`;

test("admin edits a user's capability overlay end-to-end", async ({ page }) => {
  await openLive(page);

  // fe-fi-migration #6: the single rail — the Admin workspace is a direct rail
  // button (no product-domain tab to select first). It is shown because we signed
  // in as the seeded admin (admin/ops rows are hidden for non-admins).
  await page
    .getByRole("complementary", { name: "workspaces" })
    .getByRole("button", { name: "Admin", exact: false })
    .click();

  // Create a fresh trader to edit (idempotent within a fresh edge).
  await page.getByRole("button", { name: "New user" }).click();
  await page.getByPlaceholder("trader@celnet.com").fill(TRADER_EMAIL);
  await page.getByPlaceholder("Jane Trader").fill("Perms Trader");
  await page.getByPlaceholder("at least 12 characters").fill("longenoughpw1");
  await page.getByRole("button", { name: "Create user" }).click();

  // Open the capability matrix for that trader.
  const traderRow = page.getByRole("row", { name: new RegExp(TRADER_EMAIL) });
  await traderRow.getByRole("button", { name: "Permissions" }).click();
  await expect(
    page.getByRole("heading", { name: new RegExp(`Capabilities — ${TRADER_EMAIL}`) }),
  ).toBeVisible();

  // Cells are addressed by their accessible name "<Action> on <Asset>: …".
  const administerFi = page.getByRole("button", { name: /^Administer on Fixed Income:/ });
  const executeFx = page.getByRole("button", { name: /^Execute \(deal\) on FX Options:/ });

  // Baseline (trader role bundle): administer is blocked, execute is allowed.
  await expect(administerFi).toHaveAttribute("aria-label", /blocked, overlay Inherit/);
  await expect(executeFx).toHaveAttribute("aria-label", /allowed, overlay Inherit/);

  // GRANT administer·FI (one click: inherit → grant).
  await administerFi.click();
  await expect(administerFi).toHaveAttribute("aria-label", /allowed, overlay Grant/);

  // DENY execute·FX (two clicks: inherit → grant → deny). Deny must visibly win.
  await executeFx.click();
  await executeFx.click();
  await expect(executeFx).toHaveAttribute(
    "aria-label",
    /blocked, overlay Deny, deny overrides role default/,
  );
  await expect(executeFx).toHaveAttribute("data-deny-override", "true");
  await expect(page.getByText("Deny overrides role default")).toBeVisible();

  // Save — replaces the overlay wholesale + ends the user's sessions.
  await page.getByRole("button", { name: "Save capabilities" }).click();
  await expect(
    page.getByText("Saved — the user's active sessions were ended."),
  ).toBeVisible();

  // The matrix re-rendered from the server-resolved effective set: the grant and
  // the deny survived a real round trip (read-back === live decision).
  await expect(administerFi).toHaveAttribute("aria-label", /allowed, overlay Grant/);
  await expect(executeFx).toHaveAttribute("aria-label", /blocked, overlay Deny/);

  // Screenshots at the required widths.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({ path: "e2e-artifacts/capabilities-1440.png", fullPage: true });
  await page.setViewportSize({ width: 768, height: 1024 });
  await page.screenshot({ path: "e2e-artifacts/capabilities-768.png", fullPage: true });
  await page.setViewportSize({ width: 1440, height: 900 });

  // a11y: zero serious/critical on the capability surface.
  await expectNoSeriousA11y(page, "admin capability matrix");
});
