/**
 * Fixed-income FIX dialects — LIVE e2e against the real demo edge (Enforce).
 *
 * Signs in as the seeded admin, opens the Connections workspace, and verifies the
 * new-connection wizard's Dialect step now offers the two fixed-income cards
 * (Quote/RFQ + Streaming/RFS) ENABLED (the admin holds every capability), then
 * creates an FI-quote venue end-to-end through the live WS mirror and confirms it
 * appears in the connections table with the right dialect label. A desk is created
 * first (every managed connection is desk-owned). Screenshots the Dialect step at
 * 1440. The capability-DISABLED affordance is covered deterministically by the
 * component unit test (`test/fixConnectionWizard.test.tsx`).
 */
import { test, expect } from "@playwright/test";

import { openLive } from "./helpers";

const DESK_NAME = `FI Rates ${Date.now()}`;
const CONN_NAME = `FI Quote Venue ${Date.now()}`;

test("admin sees the FI dialect cards and creates an FI-quote venue", async ({ page }) => {
  await openLive(page);

  // fe-fi-migration re-add: Admin + Connections live under the Administration domain
  // tab — select it first (visible as the seeded admin), then both rail buttons are
  // present. Create a desk to own the connection.
  await page
    .getByRole("tablist", { name: "product domains" })
    .getByRole("tab", { name: "Administration" })
    .click();
  const rail = page.getByRole("complementary", { name: "workspaces" });
  await rail.getByRole("button", { name: "Admin", exact: false }).click();
  await page.getByRole("textbox", { name: "new desk name" }).fill(DESK_NAME);
  await page.getByRole("button", { name: "Add desk" }).click();
  await expect(page.getByText(DESK_NAME)).toBeVisible();

  // → Connections workspace.
  await rail.getByRole("button", { name: "Connections", exact: false }).click();
  await page.getByRole("button", { name: "New connection" }).click();

  // The Dialect step now offers BOTH fixed-income cards, enabled (admin = all caps).
  const quoteCard = page.getByRole("button", { name: /Fixed Income — Quote \(RFQ\)/ });
  const streamCard = page.getByRole("button", { name: /Fixed Income — Streaming \(RFS\)/ });
  await expect(quoteCard).toBeEnabled();
  await expect(streamCard).toBeEnabled();

  // Screenshot the Dialect step at 1440.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({ path: "e2e-artifacts/fix-dialects-1440.png", fullPage: true });

  // Select FI-quote and walk the wizard to a live create.
  await quoteCard.click();
  await page.getByRole("button", { name: "Next" }).click(); // → identity
  await page.getByPlaceholder(/Bank A/).fill(CONN_NAME);
  const deskSelect = page.getByLabel("Owning desk");
  const deskValue = await deskSelect
    .locator("option", { hasText: DESK_NAME })
    .getAttribute("value");
  await deskSelect.selectOption(deskValue);
  await page.getByRole("button", { name: "Next" }).click(); // → compids
  await page.getByRole("button", { name: "Next" }).click(); // → review
  await page.getByRole("button", { name: "Create connection" }).click();

  // The connection appears in the table with the FI-quote dialect label.
  const row = page.getByRole("row", { name: new RegExp(CONN_NAME) });
  await expect(row).toBeVisible();
  await expect(row.getByText("Fixed Income — Quote (RFQ)")).toBeVisible();
});
