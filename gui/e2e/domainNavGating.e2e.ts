/**
 * Navigation gating (slice 5c, re-homed by fe-fi-migration #6) — LIVE e2e against
 * the real demo edge under Enforce.
 *
 * Slice 5b gated individual CONTROLS (disable + tooltip). This proves the
 * NAVIGATION half on the SINGLE class-parametric rail (the FX/FI domain-tab split
 * is retired): a signed-in user with no `view` on an asset class does not see the
 * workspaces that serve ONLY that class — they are HIDDEN — while the CROSS-ASSET
 * (class-parametric) rows stay reachable via the OTHER class they serve, and the
 * app never strands the user on a now-hidden workspace (it redirects to a visible
 * one). Admin/ops rows stay hidden for a non-admin, exactly as before.
 *
 * Flow (all through the production bundle + live WS mirror):
 *   1. Sign in as the seeded admin; create a fresh trader.
 *   2. On the Permissions page, deny the trader READ on every FX Options component
 *      (the shared `view·fx_options` capability) and Save.
 *   3. Sign out; sign back in AS the trader (re-deriving their effective set).
 *   4. Confirm (a) there is NO product-domain tab bar, (b) the FX-ONLY rows
 *      (Stream / XVA / Excel) are GONE, (c) the CROSS-ASSET rows (Ticket / Market
 *      Data / Risk / Book) and the FI-only Quoting REMAIN — reachable via
 *      `view·fixed_income`, not stranded on a blank FX view, (d) the admin/ops rows
 *      are absent (the trader is not an admin).
 *   5. A reachable workspace works; screenshot the trader's gated rail at 1440 + an
 *      axe pass (0 serious).
 *   6. Sign back in as admin → the FX-only + admin rows are present again.
 */
import { test, expect, type Page } from "@playwright/test";

import { openLive, signIn, gotoWorkspace, expectNoSeriousA11y } from "./helpers";

const TRADER_EMAIL = `nav-gate-trader-${Date.now()}@celnet.com`;
const TRADER_PW = "longenoughpw1";

/** The FX Options components whose READ toggle is `view·fx_options` (shared cell). */
const FX_READ_SWITCHES = ["Ticket Read", "Stream Read", "Surface Read", "Risk Read"];

/** The workspaces rail (the single class-parametric rail). */
function rail(page: Page) {
  return page.getByRole("complementary", { name: "workspaces" });
}

/** Assert a rail button (by `title="<label> (…)"` prefix) is present / absent. */
async function expectRail(page: Page, label: string, present: boolean): Promise<void> {
  const btn = rail(page).locator(`button[title^="${label} ("]`);
  if (present) await expect(btn).toBeVisible();
  else await expect(btn).toHaveCount(0);
}

/** Click a workspace rail button by its title prefix `"<label> ("` (unique per view). */
async function railClick(page: Page, label: string): Promise<void> {
  await rail(page).locator(`button[title^="${label} ("]`).click();
}

test("single-rail per-workspace gating: FX-only rows hide, cross-asset rows stay reachable via FI", async ({
  page,
}) => {
  // 1) Admin signs in; create a fresh trader via the Admin workspace (single rail —
  // the Admin button is directly present for the signed-in admin, no domain tab).
  await openLive(page);
  await railClick(page, "Admin");

  await page.getByRole("button", { name: "New user" }).click();
  await page.getByPlaceholder("trader@celnet.com").fill(TRADER_EMAIL);
  await page.getByPlaceholder("Jane Trader").fill("Nav Gate Trader");
  await page.getByPlaceholder("at least 12 characters").fill(TRADER_PW);
  await page.getByRole("button", { name: "Create user" }).click();

  // 2) Open the Permissions page, pick the trader, and deny READ on all FX Options
  // components. Every FX component's Read toggle projects over the SAME
  // `view·fx_options` cell, so the first deny flips them all off; click each only
  // while still on (a second click on an already-denied cell would re-grant it).
  await railClick(page, "Permissions");
  const traderPick = page.getByRole("button", { name: new RegExp(TRADER_EMAIL) });
  await expect(traderPick).toBeVisible();
  await traderPick.click();
  await expect(
    page.getByRole("heading", { name: new RegExp(`Access — ${TRADER_EMAIL}`) }),
  ).toBeVisible();

  for (const name of FX_READ_SWITCHES) {
    const sw = page.getByRole("switch", { name, exact: true });
    if ((await sw.getAttribute("aria-checked")) === "true") await sw.click();
  }
  for (const name of FX_READ_SWITCHES) {
    await expect(page.getByRole("switch", { name, exact: true })).toHaveAttribute(
      "aria-checked",
      "false",
    );
  }

  // Save — replaces the overlay wholesale + ends the trader's sessions.
  await page.getByRole("button", { name: "Save access" }).click();
  await expect(
    page.getByText("Saved — the user's active sessions were ended."),
  ).toBeVisible();

  // 3) Sign out; sign back in AS the trader (re-derives their effective set).
  await page.getByRole("button", { name: "Sign out" }).click();
  await signIn(page, TRADER_EMAIL, TRADER_PW);
  await expect(rail(page)).toBeVisible();

  // 4a) There is NO product-domain tab bar — the FX/FI split is retired.
  await expect(page.getByRole("tablist", { name: "product domains" })).toHaveCount(0);

  // 4b) The FX-ONLY rows are GONE (no `view·fx_options`).
  await expectRail(page, "Stream", false);
  await expectRail(page, "XVA", false);
  await expectRail(page, "Excel", false);

  // 4c) The CROSS-ASSET rows remain (reachable via `view·fixed_income`), and the
  // FI-only Quoting remains — the trader is not stranded on a blank FX view.
  for (const label of ["Ticket", "Market Data", "Risk", "Book", "Quoting"]) {
    await expectRail(page, label, true);
  }

  // 4d) The admin/ops rows are absent (the trader is not an admin).
  for (const label of ["Admin", "Permissions", "Connections", "Reference Data"]) {
    await expectRail(page, label, false);
  }

  // 5) A reachable cross-asset workspace works (Book's aggregate-risk lens is
  // class-agnostic), then screenshot the trader's gated rail at 1440 + an axe pass.
  const pane = await gotoWorkspace(page, "book");
  await expect(pane).toBeVisible();
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({
    path: "e2e-artifacts/single-rail-gating-trader-1440.png",
    fullPage: true,
  });
  await expectNoSeriousA11y(page, "trader gated single rail (view·fx_options denied)");

  // 6) Sign back in as the admin → the FX-only + admin rows are present again.
  await page.getByRole("button", { name: "Sign out" }).click();
  await signIn(page);
  for (const label of ["Ticket", "Stream", "Market Data", "Risk", "Book", "Quoting", "XVA", "Excel", "Admin", "Permissions"]) {
    await expectRail(page, label, true);
  }
});
