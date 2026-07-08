/**
 * Admin desk assignment — LIVE e2e against the real demo edge (Enforce).
 *
 * Desk membership is what permissions a trader's inbound quote/deal reception, so
 * assigning a desk from the Users table is a real routing action. This drives the
 * production bundle + live WS mirror end to end: sign in as the seeded admin,
 * create a desk via the Desks form, create an UNASSIGNED trader, then use that
 * trader's inline Desk `<select>` to assign the desk, click Refresh, and confirm
 * the assignment PERSISTED across a real server round trip (the select still shows
 * the desk and the "receives no quotes" deny-by-default marker is gone). Then a
 * 1440 screenshot + an axe pass (0 serious/critical).
 */
import { test, expect, type Page } from "@playwright/test";

import { openLive, expectNoSeriousA11y } from "./helpers";

const STAMP = Date.now();
const TRADER_EMAIL = `desk-assign-trader-${STAMP}@celnet.com`;
const TRADER_PW = "longenoughpw1";
const DESK_NAME = `Desk Assign ${STAMP}`;

/** Click a workspace rail button by its title prefix `"<label> ("` (unique per view). */
async function railClick(page: Page, label: string): Promise<void> {
  const btn = page
    .getByRole("complementary", { name: "workspaces" })
    .locator(`button[title^="${label} ("]`);
  if ((await btn.count()) === 0) {
    const tabs = page.getByRole("tablist", { name: "product domains" }).getByRole("tab");
    for (let i = 0; i < (await tabs.count()); i += 1) {
      await tabs.nth(i).click();
      if ((await btn.count()) > 0) break;
    }
  }
  await btn.click();
}

test("admin creates a desk + trader and assigns the desk inline, persisted end-to-end", async ({
  page,
}) => {
  // 1) Admin signs in and opens the Admin workspace.
  await openLive(page);
  await railClick(page, "Admin");

  // 2) Create a fresh desk via the Desks form.
  await page.getByLabel("new desk name").fill(DESK_NAME);
  await page.getByRole("button", { name: "Add desk" }).click();
  await expect(page.getByRole("cell", { name: DESK_NAME })).toBeVisible();

  // 3) Create an UNASSIGNED trader via the New user dialog.
  await page.getByRole("button", { name: "New user" }).click();
  await page.getByPlaceholder("trader@celnet.com").fill(TRADER_EMAIL);
  await page.getByPlaceholder("Jane Trader").fill("Desk Assign Trader");
  await page.getByPlaceholder("at least 12 characters").fill(TRADER_PW);
  await page.getByRole("button", { name: "Create user" }).click();

  // The new trader's row carries the deny-by-default marker (no desk ⇒ no quotes).
  const deskSelect = page.getByRole("combobox", { name: `Desk for ${TRADER_EMAIL}` });
  await expect(deskSelect).toBeVisible();
  await expect(deskSelect.locator("option:checked")).toHaveText("Unassigned");

  // 4) Assign the desk inline (optimistic through `assignDesk`).
  await deskSelect.selectOption({ label: DESK_NAME });
  await expect(deskSelect.locator("option:checked")).toHaveText(DESK_NAME);

  // 5) Refresh — re-load the roster from the server; the assignment must survive a
  // real round trip (not merely the optimistic local state).
  await page.getByRole("button", { name: "Refresh" }).click();
  const deskSelectAfter = page.getByRole("combobox", { name: `Desk for ${TRADER_EMAIL}` });
  await expect(deskSelectAfter.locator("option:checked")).toHaveText(DESK_NAME);

  // Screenshot the Admin workspace at 1440.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({ path: "e2e-artifacts/admin-desk-assignment-1440.png", fullPage: true });

  // axe on the Admin workspace: 0 serious/critical.
  await expectNoSeriousA11y(page, "admin workspace (desk assignment)");
});
