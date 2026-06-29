/**
 * Counterparty Simulator (sandbox) — LIVE e2e against the real demo edge under
 * Enforce. Proves the refactor's two guarantees end-to-end through the production
 * bundle + live WS mirror:
 *
 *   A. The standalone, permission-gated Simulator popup is a PURE CLIENT SANDBOX:
 *      the admin opens it, generates an RFQ and an IOI, each gets a sample quote
 *      in the popup's sandbox list — and the LIVE RFQ/IOI inbox still shows 0
 *      requests (nothing was injected into the priced desk flow). Screenshot the
 *      popup at 1440 + an axe pass (0 serious/critical).
 *
 *   B. The top-bar Simulator button is gated on `simulate·fixed_income`: the admin
 *      DENIES a fresh trader that capability and saves; the admin's OWN button
 *      stays enabled (per-identity), and signing in AS the trader leaves the
 *      Simulator button DISABLED + carrying the denial tooltip — VISIBLE, never
 *      hidden (top-bar affordance discipline). Screenshot the disabled button at
 *      1440.
 */
import { test, expect, type Page } from "@playwright/test";

import { openLive, signIn, expectNoSeriousA11y } from "./helpers";

const TRADER_EMAIL = `sim-trader-${Date.now()}@celnet.com`;
const TRADER_PW = "longenoughpw1";
const SIM_DENIED_TITLE =
  "Your permissions don't allow using the fixed-income counterparty simulator.";

/** Click a top-level product-domain tab (FX Options / Fixed Income / Administration). */
async function selectDomain(page: Page, name: string): Promise<void> {
  await page.getByRole("tab", { name }).click();
}

/** Click a workspace rail button by label within the active domain. */
async function railClick(page: Page, label: string | RegExp): Promise<void> {
  await page
    .getByRole("complementary", { name: "workspaces" })
    .getByRole("button", { name: label, exact: false })
    .click();
}

test("admin sandbox: generate RFQ + IOI without injecting into the live inbox", async ({
  page,
}) => {
  await openLive(page);

  // The top-bar Simulator button is enabled for the admin (holds simulate·FI).
  const simBtn = page.getByRole("button", { name: "open counterparty simulator" });
  await expect(simBtn).toBeVisible();
  await expect(simBtn).toBeEnabled();
  await simBtn.click();

  const dialog = page.getByRole("dialog", { name: "Simulator" });
  await expect(dialog).toBeVisible();
  // The not-live banner is explicit.
  await expect(dialog.getByText(/not sent to the desk/i)).toBeVisible();

  // Generate an RFQ (default kind), then an IOI.
  await dialog.getByRole("button", { name: "Generate" }).click();
  await dialog.getByLabel("item kind").selectOption("IOI");
  await dialog.getByRole("button", { name: "Generate" }).click();

  // Both items appear in the popup's sandbox list, each with a sample quote.
  const items = dialog.getByRole("listitem");
  await expect(items).toHaveCount(2);
  await expect(items.filter({ hasText: "RFQ" })).toHaveCount(1);
  await expect(items.filter({ hasText: "IOI" })).toHaveCount(1);
  await expect(dialog.getByText(/sample quote/i).first()).toBeVisible();

  // Screenshot the popup at 1440 + an axe pass with the dialog open.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({ path: "e2e-artifacts/simulator-popup-admin-1440.png", fullPage: true });
  await expectNoSeriousA11y(page, "simulator popup (admin)");

  // Close the popup and confirm the LIVE RFQ/IOI inbox still shows 0 requests —
  // the sandbox never injected anything into the priced flow.
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await selectDomain(page, "Fixed Income");
  await railClick(page, "Quoting");
  await expect(page.getByText("No inbound requests.")).toBeVisible();
  await expect(page.getByText(/^0 requests$/)).toBeVisible();
});

test("a denied simulate capability disables the trader's Simulator button", async ({ page }) => {
  // 1) Admin signs in; create a fresh trader.
  await openLive(page);
  await selectDomain(page, "Administration");
  await railClick(page, "Admin");

  await page.getByRole("button", { name: "New user" }).click();
  await page.getByPlaceholder("trader@celnet.com").fill(TRADER_EMAIL);
  await page.getByPlaceholder("Jane Trader").fill("Sim Trader");
  await page.getByPlaceholder("at least 12 characters").fill(TRADER_PW);
  await page.getByRole("button", { name: "Create user" }).click();

  // 2) Deny simulate·fixed_income on the trader and save.
  const traderRow = page.getByRole("row", { name: new RegExp(TRADER_EMAIL) });
  await traderRow.getByRole("button", { name: "Permissions" }).click();
  await expect(
    page.getByRole("heading", { name: new RegExp(`Capabilities — ${TRADER_EMAIL}`) }),
  ).toBeVisible();

  const simFi = page.getByRole("button", { name: /^Simulate on Fixed Income:/ });
  // Trader role bundle allows simulate (inherit); inherit → grant → deny.
  await expect(simFi).toHaveAttribute("aria-label", /allowed, overlay Inherit/);
  await simFi.click();
  await simFi.click();
  await expect(simFi).toHaveAttribute("aria-label", /blocked, overlay Deny/);

  await page.getByRole("button", { name: "Save capabilities" }).click();
  await expect(page.getByText("Saved — the user's active sessions were ended.")).toBeVisible();

  // 3) The ADMIN's OWN Simulator button is still ENABLED (gating is per-identity).
  await expect(page.getByRole("button", { name: "open counterparty simulator" })).toBeEnabled();

  // 4) Sign out; sign back in AS the trader (re-derives their effective set).
  await page.getByRole("button", { name: "Sign out" }).click();
  await signIn(page, TRADER_EMAIL, TRADER_PW);

  // 5) The trader's Simulator button is DISABLED + tooltip, NOT hidden.
  const simBtn = page.getByRole("button", { name: "open counterparty simulator" });
  await expect(simBtn).toBeVisible(); // never hidden
  await expect(simBtn).toBeDisabled(); // gated off
  await expect(simBtn).toHaveAttribute("title", SIM_DENIED_TITLE);

  // 6) Screenshot the disabled button at 1440.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({
    path: "e2e-artifacts/simulator-disabled-trader-1440.png",
    fullPage: true,
  });
});
