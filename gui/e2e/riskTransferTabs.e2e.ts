/**
 * Risk Transfer consolidation — OFFLINE e2e against the in-app `?mock` transport (no
 * cargo edge, no server). The three former TRANSFERS rail surfaces — Risk Transfer
 * (initiate) · Transfer Inbox (approve) · Transfer Audit (provenance) — are merged into
 * ONE tabbed rail entry, "Risk Transfer" (mirroring the Risk Portfolios→Risk Dashboard
 * and Pricing Groups+Tiering→Pricing merges). This drives the production bundle end to
 * end, as the seeded admin (who holds every capability, so all three tabs show):
 *   1. open the consolidated surface and assert the three tabs render + switch (only the
 *      active tab mounts);
 *   2. assert the standalone "Transfer Inbox" / "Transfer Audit" rail entries are gone,
 *      while the single "Risk Transfer" rail entry remains;
 *   3. assert the retired deep-links `?view=transferinbox` / `?view=transferaudit` land
 *      on the correct folded tab;
 *   4. run axe on the merged surface (0 serious/critical);
 *   5. screenshot the tabs.
 *
 * The Shell mounts a hidden (inert) deep-link pane per consolidated alias, so the tab
 * `data-testid`s repeat across panes — every query is SCOPED to the active (non-inert)
 * pane that {@link gotoMockView} returns.
 */
import { expect, test } from "@playwright/test";

import { gotoMockView, installFrozenClock } from "./fidelityHelpers";
import { expectNoSeriousA11y } from "./helpers";

test("Risk Transfer merges initiate + inbox + audit into ONE tabbed rail entry", async ({
  page,
}) => {
  await installFrozenClock(page);
  await page.setViewportSize({ width: 1440, height: 900 });

  // Open the consolidated Risk Transfer surface (FI domain) as the seeded admin; the
  // returned locator is the active (non-inert) pane — scope all queries to it.
  const pane = await gotoMockView(page, "risktransfer");

  // --- 1 · the three tabs render, ticket is the default -----------------------
  const ticketTab = pane.getByTestId("risk-transfer-tab-ticket");
  const inboxTab = pane.getByTestId("risk-transfer-tab-inbox");
  const auditTab = pane.getByTestId("risk-transfer-tab-audit");
  await expect(ticketTab).toBeVisible();
  await expect(inboxTab).toBeVisible();
  await expect(auditTab).toBeVisible();
  await expect(ticketTab).toHaveText("Risk Transfer");
  await expect(inboxTab).toHaveText("Inbox");
  await expect(auditTab).toHaveText("Audit");
  // Default lands on the initiate ticket (its source-portfolio picker is mounted).
  await expect(ticketTab).toHaveAttribute("aria-pressed", "true");
  await expect(pane.getByTestId("xfer-source")).toBeVisible();

  // Screenshot the consolidated tab bar (steady-state ticket tab).
  await page.screenshot({ path: "e2e-artifacts/risk-transfer-tabs-ticket.png", fullPage: true });

  // --- 2 · switching tabs mounts only the active panel ------------------------
  await inboxTab.click();
  await expect(inboxTab).toHaveAttribute("aria-pressed", "true");
  await expect(pane.getByTestId("xfer-source")).toHaveCount(0); // ticket unmounted
  await expect(pane.getByRole("heading", { name: "Transfer Inbox" })).toBeVisible();

  await auditTab.click();
  await expect(auditTab).toHaveAttribute("aria-pressed", "true");
  await expect(pane.getByTestId("audit-state")).toBeVisible(); // audit blotter mounted
  await expect(pane.getByRole("heading", { name: "Transfer Audit" })).toBeVisible();
  await page.screenshot({ path: "e2e-artifacts/risk-transfer-tabs-audit.png", fullPage: true });

  // --- 4 · axe on the merged surface (still on the audit tab, before any nav) --
  await expectNoSeriousA11y(page, "consolidated Risk Transfer surface (audit tab)");

  // --- 3 · the Fixed-Income rail carries ONE entry; the two standalones are gone.
  // The transfer surfaces are FI-only, so select the Fixed Income product domain, then
  // inspect its rail (the left "workspaces" complementary).
  await page
    .getByRole("tablist", { name: "product domains" })
    .getByRole("tab", { name: "Fixed Income" })
    .click();
  const rail = page.getByRole("complementary", { name: "workspaces" });
  await expect(rail.getByRole("button", { name: /Risk Transfer/ })).toHaveCount(1);
  await expect(rail.getByRole("button", { name: /Transfer Inbox/ })).toHaveCount(0);
  await expect(rail.getByRole("button", { name: /Transfer Audit/ })).toHaveCount(0);
});

test("retired deep-links land on the folded tab", async ({ page }) => {
  await installFrozenClock(page);
  await page.setViewportSize({ width: 1440, height: 900 });

  // `?view=transferinbox` opens the merged host straight on the Inbox tab.
  const inboxPane = await gotoMockView(page, "transferinbox");
  await expect(inboxPane.getByTestId("risk-transfer-tab-inbox")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(inboxPane.getByRole("heading", { name: "Transfer Inbox" })).toBeVisible();
  await expect(inboxPane.getByTestId("xfer-source")).toHaveCount(0);

  // `?view=transferaudit` opens the merged host straight on the Audit tab.
  const auditPane = await gotoMockView(page, "transferaudit");
  await expect(auditPane.getByTestId("risk-transfer-tab-audit")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(auditPane.getByRole("heading", { name: "Transfer Audit" })).toBeVisible();
  await expect(auditPane.getByTestId("audit-state")).toBeVisible();
});
