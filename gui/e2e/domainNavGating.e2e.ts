/**
 * Domain navigation gating (slice 5c) — LIVE e2e against the real demo edge under
 * Enforce.
 *
 * Slice 5b gated individual CONTROLS (disable + tooltip). This proves the
 * NAVIGATION half: a signed-in user with no `view` on an asset class does not see
 * that asset class's whole domain TAB or its rail workspaces at all — they are
 * HIDDEN (exactly like the Administration tab is hidden for non-admins), and the
 * app never strands the user on a now-hidden domain (it redirects to a visible
 * one).
 *
 * Flow (all through the production bundle + live WS mirror):
 *   1. Sign in as the seeded admin; create a fresh trader.
 *   2. On the Permissions page, deny the trader READ on every FX Options component
 *      (the shared `view·fx_options` capability) and Save.
 *   3. Sign out; sign back in AS the trader (re-deriving their effective set).
 *   4. Confirm (a) the "FX Options" tab is GONE, (b) the "Fixed Income" tab
 *      remains AND is the active (redirected-to) domain — not stranded on a blank
 *      FX view, (c) a Fixed Income workspace works, (d) the Administration tab is
 *      absent (the trader is not an admin).
 *   5. Screenshot the trader's gated tab bar at 1440 + an axe pass (0 serious).
 *   6. Sign back in as admin → all three tabs are present again.
 */
import { test, expect, type Page } from "@playwright/test";

import { openLive, signIn, expectNoSeriousA11y } from "./helpers";

const TRADER_EMAIL = `nav-gate-trader-${Date.now()}@celnet.com`;
const TRADER_PW = "longenoughpw1";

/** The FX Options components whose READ toggle is `view·fx_options` (shared cell). */
const FX_READ_SWITCHES = ["Ticket Read", "Stream Read", "Surface Read", "Risk Read"];

/** Click a top-level product-domain tab. */
async function selectDomain(page: Page, name: string): Promise<void> {
  await page.getByRole("tab", { name }).click();
}

/** Click a workspace rail button by its title prefix `"<label> ("` (unique per view). */
async function railClick(page: Page, label: string): Promise<void> {
  await page
    .getByRole("complementary", { name: "workspaces" })
    .locator(`button[title^="${label} ("]`)
    .click();
}

test("hiding a domain tab + workspaces for a user with no view on that asset class", async ({
  page,
}) => {
  // 1) Admin signs in; create a fresh trader via the Admin workspace.
  await openLive(page);
  await selectDomain(page, "Administration");
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
  // All FX Read toggles now reflect the denied shared capability.
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
  await expect(page.getByRole("complementary", { name: "workspaces" })).toBeVisible();

  // 4a) The FX Options tab is GONE (no `view·fx_options`).
  await expect(page.getByRole("tab", { name: "FX Options" })).toHaveCount(0);
  // 4d) The Administration tab is gone (the trader is not an admin).
  await expect(page.getByRole("tab", { name: "Administration" })).toHaveCount(0);

  // 4b) The Fixed Income tab remains AND is the active domain — the default FX
  // landing workspace was inaccessible, so the app redirected here (not stranded
  // on a blank/hidden FX view).
  const fiTab = page.getByRole("tab", { name: "Fixed Income" });
  await expect(fiTab).toBeVisible();
  await expect(fiTab).toHaveAttribute("aria-selected", "true");

  // 4c) A Fixed Income workspace works: navigate to Rates and confirm its live
  // pricing affordance renders (the trader retains `view·fixed_income`).
  await railClick(page, "Rates");
  await expect(page.getByRole("button", { name: /Request quote|Pricing…/ })).toBeVisible();

  // 5) Screenshot the trader's gated tab bar at 1440 + an axe pass.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({
    path: "e2e-artifacts/domain-nav-gating-trader-1440.png",
    fullPage: true,
  });
  await expectNoSeriousA11y(page, "trader gated tab bar (view·fx_options denied)");

  // 6) Sign back in as the admin → all three domain tabs are present again.
  await page.getByRole("button", { name: "Sign out" }).click();
  await signIn(page);
  await expect(page.getByRole("tab", { name: "FX Options" })).toBeVisible();
  await expect(page.getByRole("tab", { name: "Fixed Income" })).toBeVisible();
  await expect(page.getByRole("tab", { name: "Administration" })).toBeVisible();
});
