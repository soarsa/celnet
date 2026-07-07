/**
 * Permissions page — LIVE e2e against the real demo edge (Enforce).
 *
 * Drives the new first-class Permissions rail workspace through the production
 * bundle + live WS mirror: sign in as the seeded admin, create a trader, open the
 * Permissions page, pick the trader, turn the "Rates Book" WRITE toggle OFF
 * (denying `book·fixed_income`), Save, and confirm:
 *   (a) the grid reflects it — the Write switch is off and deny-styled,
 *   (b) the "active sessions ended" note shows,
 *   (c) the advanced disclosure shows `book·fixed_income` denied,
 *   (d) cross-check — signing in AS that trader, the Rates-Book "Book position"
 *       control is DISABLED (slice-5b gating keys on `book·fixed_income`).
 * Then a 1440 screenshot + an axe pass (0 serious/critical).
 *
 * `book·fixed_income` is chosen because the Rates-Book "Book position" button is a
 * single, always-rendered control with a deterministic gate (no inbound-state
 * prerequisite), so the cross-check selector is stable.
 */
import { test, expect, type Page } from "@playwright/test";

import { openLive, signIn, expectNoSeriousA11y } from "./helpers";

const TRADER_EMAIL = `perms-page-trader-${Date.now()}@celnet.com`;
const TRADER_PW = "longenoughpw1";
const BOOK_DENIED_TITLE = "Your permissions don't allow booking fixed-income positions.";

/** Click a workspace rail button by its title prefix `"<label> ("` (unique per view). */
async function railClick(page: Page, label: string): Promise<void> {
  const btn = page
    .getByRole("complementary", { name: "workspaces" })
    .locator(`button[title^="${label} ("]`);
  // fe-fi-migration re-add: the rail is filtered to the active domain. If the row
  // isn't under the current tab, select whichever domain tab surfaces it first.
  if ((await btn.count()) === 0) {
    const tabs = page.getByRole("tablist", { name: "product domains" }).getByRole("tab");
    for (let i = 0; i < (await tabs.count()); i += 1) {
      await tabs.nth(i).click();
      if ((await btn.count()) > 0) break;
    }
  }
  await btn.click();
}

/**
 * Open the rates book. fe-fi-migration #6 collapsed the standalone "Rates Book"
 * rail row into the single class-parametric Book workspace: click Book, then flip
 * to the "Positions & Booking" lens in the book-view group — the rates booking
 * controls live there. Returns the active pane.
 */
async function gotoRatesBook(page: Page) {
  await railClick(page, "Book");
  const pane = page.locator('[aria-hidden="false"]:not([inert])').last();
  await pane
    .getByRole("group", { name: "book view" })
    .getByRole("button", { name: "Positions & Booking" })
    .click();
  return pane;
}

test("admin edits a user's component access on the Permissions page end-to-end", async ({
  page,
}) => {
  // 1) Admin signs in; create a fresh trader via the Admin workspace (a direct
  // button on the single class-parametric rail — no product-domain tab).
  await openLive(page);
  await railClick(page, "Admin");

  await page.getByRole("button", { name: "New user" }).click();
  await page.getByPlaceholder("trader@celnet.com").fill(TRADER_EMAIL);
  await page.getByPlaceholder("Jane Trader").fill("Perms Page Trader");
  await page.getByPlaceholder("at least 12 characters").fill(TRADER_PW);
  await page.getByRole("button", { name: "Create user" }).click();

  // 2) Open the first-class Permissions page and pick the trader.
  await railClick(page, "Permissions");
  await expect(page.getByRole("heading", { name: "Component access" })).toBeVisible();
  await page.getByRole("button", { name: new RegExp(TRADER_EMAIL) }).click();
  await expect(
    page.getByRole("heading", { name: new RegExp(`Access — ${TRADER_EMAIL}`) }),
  ).toBeVisible();

  // 3) The Rates Book WRITE toggle (book·FI). Trader role bundle allows book, so it
  // starts ON; one click turns it OFF (deny — deny-wins).
  const ratesBookWrite = page.getByRole("switch", { name: "Rates Book Write" });
  await expect(ratesBookWrite).toHaveAttribute("aria-checked", "true");
  await expect(ratesBookWrite).toHaveAttribute("data-state", "on");
  await ratesBookWrite.click();
  // (a) The grid reflects the deny: off + deny-styled (not colour alone — data attr).
  await expect(ratesBookWrite).toHaveAttribute("aria-checked", "false");
  await expect(ratesBookWrite).toHaveAttribute("data-state", "off");
  await expect(ratesBookWrite).toHaveAttribute("data-deny", "true");

  // Read stays ON — denying Write must not touch the shared view capability.
  await expect(page.getByRole("switch", { name: "Rates Book Read" })).toHaveAttribute(
    "aria-checked",
    "true",
  );

  // 4) Save — replaces the overlay wholesale + ends the user's sessions.
  await page.getByRole("button", { name: "Save access" }).click();
  // (b) The session-revocation note appears.
  await expect(
    page.getByText("Saved — the user's active sessions were ended."),
  ).toBeVisible();

  // The grid re-rendered from the server-resolved effective set: deny survived a
  // real round trip.
  await expect(ratesBookWrite).toHaveAttribute("aria-checked", "false");
  await expect(ratesBookWrite).toHaveAttribute("data-deny", "true");

  // (c) The advanced view shows book·fixed_income DENIED. Scope to the user's
  // Component-access region ("Access — <email>") — the Role-bundles panel renders
  // its OWN "Book on Fixed Income:" cell on the same page, so a page-wide match is
  // ambiguous (strict-mode violation). The region is the named ComponentAccessGrid
  // <section>, so this targets exactly the per-user advanced cell under test.
  const accessGrid = page.getByRole("region", {
    name: new RegExp(`Access — ${TRADER_EMAIL}`),
  });
  await accessGrid.getByRole("button", { name: "Rates Book", exact: true }).click();
  const bookCell = accessGrid.getByRole("button", { name: /^Book on Fixed Income:/ });
  await expect(bookCell).toHaveAttribute("aria-label", /blocked, overlay Deny/);
  await expect(bookCell).toHaveAttribute("data-overlay", "deny");

  // Screenshot the Permissions page at 1440.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({ path: "e2e-artifacts/permissions-page-1440.png", fullPage: true });

  // axe on the Permissions page: 0 serious/critical.
  await expectNoSeriousA11y(page, "permissions page (component access grid)");

  // 5) (d) Cross-check: sign in AS the trader; their Rates-Book "Book position"
  // control is DISABLED + carries the denial tooltip, still VISIBLE (never hidden).
  await page.getByRole("button", { name: "Sign out" }).click();
  await signIn(page, TRADER_EMAIL, TRADER_PW);

  const ratesBookPane = await gotoRatesBook(page);
  const bookBtn = ratesBookPane.getByRole("button", { name: "Book position" });
  await expect(bookBtn).toBeVisible();
  await expect(bookBtn).toBeDisabled();
  await expect(bookBtn).toHaveAttribute("title", BOOK_DENIED_TITLE);
});
