/**
 * Reference Data workspace e2e (live, server-enforced — CELNET_ACCESS_MODE=enforce).
 *
 * Drives the REAL app against the REAL demo edge: an administrator opens
 * Reference Data (a direct workspace on the single class-parametric rail — the
 * FX/FI domain-tab split is retired), creates an OIS definition AND a bond definition
 * (each with an external identifier), and both appear in the registry table with
 * their external ids. Then a freshly created trader (non-admin) signs in and
 * confirms the per-control admin gating: the list is visible to them, but the
 * create / edit / delete controls are NOT. An axe pass scans the resting panel
 * and a 1440-wide screenshot is captured.
 */
import { expect, test, type Page } from "@playwright/test";

import { openLive, signIn, expectNoSeriousA11y } from "./helpers";

const TRADER_PW = "longenoughpw1";

/** Open a workspace via its product-domain tab + rail button (title prefix). */
async function gotoView(page: Page, domain: string, label: string) {
  await page
    .getByRole("tablist", { name: "product domains" })
    .getByRole("tab", { name: domain, exact: true })
    .click();
  const rail = page.getByRole("complementary", { name: "workspaces" });
  await rail.locator(`button[title^="${label} ("]`).click();
  const pane = page.locator('[aria-hidden="false"]:not([inert])').last();
  await expect(pane).toBeVisible();
  return pane;
}

test("admin manages OIS + bond defs under Administration; trader has no access", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openLive(page);

  const stamp = Date.now().toString().slice(-6);
  // Run-unique trader email — the demo edge persists its identity store across
  // runs, so a fixed email collides on re-run (create-user error → modal stays
  // open → blocks the later Sign out click).
  const traderEmail = `refdata.trader.${stamp}@celnet.com`;
  const oisName = `USD SOFR OIS ${stamp}`;
  const oisTicker = `USOSFR${stamp}`;
  const bondName = `US Treasury ${stamp}`;
  const bondIsin = `US0000${stamp}`;

  const pane = await gotoView(page, "Administration", "Reference Data");

  // --- create an OIS definition (family defaults to OIS) ---------------------
  // All workspaces are persistently mounted, so scope every form locator to the
  // VISIBLE Reference Data pane (a bare page query would match hidden panes).
  await pane.getByRole("button", { name: "New instrument" }).click();
  await pane.getByLabel("Family").selectOption("ois");
  await pane.getByLabel("Name", { exact: true }).fill(oisName);
  await pane.getByLabel("Index", { exact: true }).fill("SOFR");
  await pane.getByLabel("Tenor", { exact: true }).fill("5Y");
  await pane.getByRole("button", { name: "Add identifier" }).click();
  await pane.getByLabel("external id 1 scheme").selectOption("ticker");
  await pane.getByLabel("external id 1 value").fill(oisTicker);
  await pane.getByRole("button", { name: "Create instrument" }).click();

  await expect(pane.getByRole("cell", { name: oisName, exact: true })).toBeVisible();
  await expect(pane.getByRole("cell", { name: new RegExp(oisTicker) })).toBeVisible();

  // --- create a bond definition --------------------------------------------
  await pane.getByRole("button", { name: "New instrument" }).click();
  await pane.getByLabel("Family").selectOption("bond");
  await pane.getByLabel("Name", { exact: true }).fill(bondName);
  await pane.getByLabel("Issuer", { exact: true }).fill("US Treasury");
  await pane.getByRole("button", { name: "Add identifier" }).click();
  await pane.getByLabel("external id 1 scheme").selectOption("isin");
  await pane.getByLabel("external id 1 value").fill(bondIsin);
  await pane.getByRole("button", { name: "Create instrument" }).click();

  await expect(pane.getByRole("cell", { name: bondName, exact: true })).toBeVisible();
  await expect(pane.getByRole("cell", { name: new RegExp(bondIsin) })).toBeVisible();

  // The 1440-wide registry screenshot (required artifact).
  await page.screenshot({ path: "reference-data-1440.png" });
  await expectNoSeriousA11y(page, "Reference Data (instrument registry, admin)");

  // --- a fresh trader: create via Admin, then sign in as them ---------------
  const admin = await gotoView(page, "Administration", "Admin");
  await admin.getByRole("button", { name: "New user" }).click();
  await page.getByPlaceholder("trader@celnet.com").fill(traderEmail);
  await page.getByPlaceholder("Jane Trader").fill("Ref Data Trader");
  await page.getByPlaceholder("at least 12 characters").fill(TRADER_PW);
  await page.getByRole("button", { name: "Create user" }).click();

  await page.getByRole("button", { name: "Sign out" }).click();
  await signIn(page, traderEmail, TRADER_PW);
  await expect(page.getByRole("complementary", { name: "workspaces" })).toBeVisible();

  // The trader is not an admin — Reference Data is admin-managed and now lives
  // under the (hidden-for-non-admins) Administration domain, so its management UI
  // is unreachable for the trader. The registry data itself stays
  // server-resolvable for pricing/curve-building; only the UI is admin-only.
  await expect(page.getByRole("tab", { name: "Administration" })).toHaveCount(0);
  await expect(
    page.getByRole("complementary", { name: "workspaces" }).getByText("Reference Data"),
  ).toHaveCount(0);
});
