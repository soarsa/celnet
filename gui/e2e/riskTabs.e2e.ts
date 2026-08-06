/**
 * Risk consolidation — OFFLINE e2e against the in-app `?mock` transport (no cargo edge,
 * no server). The four former Fixed-Income "Risk" surfaces — Risk Dashboard (+ its
 * Portfolios tab), Risk Routing, Acceptance and the FI lens of the Risk-scenario grid —
 * are merged into ONE tabbed rail entry, "Risk" (mirroring the Pricing and Transfers→
 * Risk Transfer merges). This drives the production bundle end to end, as the seeded
 * admin (who holds every capability, so all FIVE tabs show):
 *   1. open the consolidated "Risk" host and assert the five tabs render + switch (only
 *      the active tab mounts);
 *   2. assert the standalone "Risk Routing" / "Acceptance" / Risk-scenario rail entries
 *      are gone, while a single "Risk" rail entry remains under Fixed Income;
 *   3. assert the retired deep-links `?view=riskrouting` / `?view=acceptance` land on the
 *      correct folded tab, and `?view=risk` opens the scenario workspace directly (the
 *      cross-asset row is NOT folded — it stays live for FX);
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

test("Risk merges dashboard + portfolios + routing + acceptance + scenario into ONE tabbed rail entry", async ({
  page,
}) => {
  await installFrozenClock(page);
  await page.setViewportSize({ width: 1440, height: 900 });

  // Open the consolidated "Risk" host (FI domain) as the seeded admin; the returned
  // locator is the active (non-inert) pane — scope all queries to it.
  const pane = await gotoMockView(page, "riskdashboard");

  // --- 1 · the five tabs render, Dashboard is the default ---------------------
  const dashboardTab = pane.getByTestId("risk-tab-dashboard");
  const portfoliosTab = pane.getByTestId("risk-tab-portfolios");
  const routingTab = pane.getByTestId("risk-tab-routing");
  const acceptanceTab = pane.getByTestId("risk-tab-acceptance");
  const scenarioTab = pane.getByTestId("risk-tab-scenario");
  for (const [tab, label] of [
    [dashboardTab, "Dashboard"],
    [portfoliosTab, "Portfolios"],
    [routingTab, "Routing"],
    [acceptanceTab, "Acceptance"],
    [scenarioTab, "Scenario"],
  ] as const) {
    await expect(tab).toBeVisible();
    await expect(tab).toHaveText(label);
  }
  // Default lands on Dashboard (its heat-overview table is mounted).
  await expect(dashboardTab).toHaveAttribute("aria-pressed", "true");

  // Screenshot the consolidated tab bar (steady-state Dashboard tab).
  await page.screenshot({ path: "e2e-artifacts/risk-tabs-dashboard.png", fullPage: true });

  // --- 2 · switching tabs mounts only the active panel ------------------------
  await routingTab.click();
  await expect(routingTab).toHaveAttribute("aria-pressed", "true");
  await expect(pane.getByRole("heading", { name: "Risk Routing" })).toBeVisible();

  await acceptanceTab.click();
  await expect(acceptanceTab).toHaveAttribute("aria-pressed", "true");
  await expect(pane.getByRole("heading", { name: "Acceptance" })).toBeVisible();
  await expect(pane.getByRole("heading", { name: "Risk Routing" })).toHaveCount(0); // unmounted

  await scenarioTab.click();
  await expect(scenarioTab).toHaveAttribute("aria-pressed", "true");
  // The FI rates scenario surface mounts (its own "Scenario Risk" lens tab bar).
  await expect(pane.getByRole("button", { name: "Scenario Risk" })).toBeVisible();
  await page.screenshot({ path: "e2e-artifacts/risk-tabs-scenario.png", fullPage: true });

  // --- 4 · axe on the merged surface (still on the scenario tab, before any nav) --
  await expectNoSeriousA11y(page, "consolidated Risk surface (scenario tab)");

  // --- 2b · the Fixed-Income rail carries ONE "Risk" entry; the standalones are gone.
  await page
    .getByRole("tablist", { name: "product domains" })
    .getByRole("tab", { name: "Fixed Income" })
    .click();
  const rail = page.getByRole("complementary", { name: "workspaces" });
  // The consolidated host (subtitle "Dashboard · portfolios · routing · …") is unique.
  await expect(rail.getByRole("button", { name: /Dashboard · portfolios · routing/ })).toHaveCount(1);
  // The retired standalone rows are gone from the FI rail.
  await expect(rail.getByRole("button", { name: /Risk Routing/ })).toHaveCount(0);
  await expect(rail.getByRole("button", { name: /Acceptance/ })).toHaveCount(0);
  // The cross-asset "Risk" Scenario grid (subtitle "Scenario P&L / greeks") is folded
  // into the host's Scenario tab on FI, so its standalone row is off the FI rail too.
  await expect(rail.getByRole("button", { name: /Scenario P&L/ })).toHaveCount(0);
});

test("retired Risk deep-links land on the folded tab", async ({ page }) => {
  await installFrozenClock(page);
  await page.setViewportSize({ width: 1440, height: 900 });

  // `?view=riskrouting` opens the merged host straight on the Routing tab.
  const routingPane = await gotoMockView(page, "riskrouting");
  await expect(routingPane.getByTestId("risk-tab-routing")).toHaveAttribute("aria-pressed", "true");
  await expect(routingPane.getByRole("heading", { name: "Risk Routing" })).toBeVisible();

  // `?view=acceptance` opens the merged host straight on the Acceptance tab.
  const acceptancePane = await gotoMockView(page, "acceptance");
  await expect(acceptancePane.getByTestId("risk-tab-acceptance")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(acceptancePane.getByRole("heading", { name: "Acceptance" })).toBeVisible();

  // `?view=risk` opens the cross-asset Scenario workspace DIRECTLY (it is NOT aliased —
  // it stays a live standalone FX rail row, only withdrawn from the FI rail). So it is
  // the raw scenario surface, NOT the consolidated "Risk" host chrome.
  const scenarioPane = await gotoMockView(page, "risk");
  await expect(scenarioPane.getByTestId("risk-tab-dashboard")).toHaveCount(0); // not the host
});
