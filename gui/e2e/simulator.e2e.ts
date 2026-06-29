/**
 * Counterparty Simulator (live-desk injector) — LIVE e2e against the real demo
 * edge under Enforce. Proves the rework's two guarantees end-to-end through the
 * production bundle + live WS mirror:
 *
 *   A. The permission-gated Simulator opens a SEPARATE OS WINDOW (so the main desk
 *      stays visible), and generating an RFQ in that popout INJECTS it into the
 *      live desk: back in the MAIN window, Fixed Income ▸ Quoting's RFQ/IOI inbox
 *      now shows the injected request (count > 0). Screenshot the popout + the
 *      main-window inbox at 1440 + an axe pass on the popout (0 serious/critical).
 *
 *   B. The top-bar Simulator button is gated on `simulate·fixed_income`: the admin
 *      DENIES a fresh trader that capability and saves; the admin's OWN button
 *      stays enabled (per-identity), and signing in AS the trader leaves the
 *      Simulator button DISABLED + carrying the denial tooltip — VISIBLE, never
 *      hidden (top-bar affordance discipline). Screenshot the disabled button.
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

test("admin: a popout window injects an RFQ into the live RFQ/IOI inbox", async ({
  page,
  context,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openLive(page);

  // The top-bar Simulator button is enabled for the admin (holds simulate·FI).
  const simBtn = page.getByRole("button", { name: "open counterparty simulator" });
  await expect(simBtn).toBeVisible();
  await expect(simBtn).toBeEnabled();

  // Clicking it opens a SEPARATE window (a new page in this browser context).
  const [popout] = await Promise.all([context.waitForEvent("page"), simBtn.click()]);
  await popout.waitForLoadState("domcontentloaded");

  // The popout is the simulator surface — live-desk note, NOT a sandbox banner.
  const panel = popout.getByRole("region", { name: "Simulator" });
  await expect(panel).toBeVisible();
  await expect(panel.getByText(/injects into the live desk/i)).toBeVisible();

  // Generate an RFQ (default kind) in the popout → real injection into the desk.
  await panel.getByRole("button", { name: "Generate" }).click();

  // The popout confirms the injection with the server-minted request id.
  const injected = panel.getByRole("listitem");
  await expect(injected).toHaveCount(1);
  await expect(injected.filter({ hasText: "RFQ" })).toHaveCount(1);
  await expect(panel.getByText("PENDING")).toBeVisible();

  // Screenshot the popout at 1440 + an axe pass on it.
  await popout.setViewportSize({ width: 1440, height: 900 });
  await popout.screenshot({ path: "e2e-artifacts/simulator-popout-admin-1440.png" });
  await expectNoSeriousA11y(popout, "simulator popout (admin)");

  // Back in the MAIN window: Fixed Income ▸ Quoting now shows the injected request.
  await page.bringToFront();
  await selectDomain(page, "Fixed Income");
  await railClick(page, "Quoting");

  // The inbox is live (it refreshes on the *_RECEIVED push) — count > 0, the
  // PENDING request is listed, and the empty-state is gone.
  await expect(page.getByText("No inbound requests.")).toBeHidden();
  const inbox = page.getByRole("list", { name: "inbound requests" });
  await expect(inbox.getByRole("listitem").first()).toBeVisible();
  await expect(page.getByText(/^0 requests$/)).toBeHidden();

  // Screenshot the main-window inbox showing the injected request at 1440.
  await page.screenshot({ path: "e2e-artifacts/simulator-main-inbox-1440.png" });

  await popout.close();
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
