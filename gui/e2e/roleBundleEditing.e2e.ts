/**
 * Role-bundle editing — LIVE e2e against the real demo edge (Enforce).
 *
 * Drives the new "Role bundles" panel on the Permissions page through the
 * production bundle + live WS mirror (`AuthService.{Get,Set}RoleCapabilities`):
 * sign in as the seeded admin, create a trader, open the Permissions page, remove
 * `book·fixed_income` from the **Trader** role bundle, Save, and confirm:
 *   (a) the "every holder's session ended" note shows,
 *   (b) the Administrator role is read-only grant-all (its Save is disabled),
 *   (c) cross-check — signing in AS the (session-revoked) trader, the Rates-Book
 *       "Book position" control is DISABLED with the denial tooltip, still visible.
 *
 * `book·fixed_income` is chosen because the Rates-Book "Book position" button is a
 * single, always-rendered control with a deterministic gate, so the cross-check
 * selector is stable.
 */
import { test, expect, type Page } from "@playwright/test";

import { openLive, signIn } from "./helpers";

const TRADER_EMAIL = `role-bundle-trader-${Date.now()}@celnet.com`;
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
 * to the "Positions & Booking" lens in the book-view group — the rates "Book
 * position" control lives there. Returns the active pane.
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

test("admin narrows the Trader role bundle end-to-end; a re-logged trader loses the affordance", async ({
  page,
}) => {
  // 1) Admin signs in; create a fresh trader via the Admin workspace (a direct
  // button on the single class-parametric rail — no product-domain tab).
  await openLive(page);
  await railClick(page, "Admin");

  await page.getByRole("button", { name: "New user" }).click();
  await page.getByPlaceholder("trader@celnet.com").fill(TRADER_EMAIL);
  await page.getByPlaceholder("Jane Trader").fill("Role Bundle Trader");
  await page.getByPlaceholder("at least 12 characters").fill(TRADER_PW);
  await page.getByRole("button", { name: "Create user" }).click();

  // 2) Open the Permissions page; the "Role bundles" panel edits the role base.
  await railClick(page, "Permissions");
  const rolePanel = page.getByRole("region", { name: "Role bundles" });
  await expect(rolePanel.getByRole("heading", { name: "Role bundles" })).toBeVisible();

  // 3) The Book·Fixed Income cell starts IN the Trader bundle; remove it.
  const bookCell = rolePanel.getByRole("button", { name: /^Book on Fixed Income:/ });
  await expect(bookCell).toHaveAttribute("aria-pressed", "true");
  await expect(bookCell).toHaveAttribute("data-allowed", "true");
  await bookCell.click();
  await expect(bookCell).toHaveAttribute("aria-pressed", "false");
  await expect(bookCell).toHaveAttribute("data-allowed", "false");

  // 4) Save the role bundle — replaces it wholesale + ends every holder's session.
  await rolePanel.getByRole("button", { name: "Save role bundle" }).click();
  await expect(
    page.getByText("Saved — every signed-in user with this role had their session ended."),
  ).toBeVisible();
  // Re-rendered from the server-resolved bundle: the removal survived a round trip.
  await expect(bookCell).toHaveAttribute("aria-pressed", "false");

  // 5) (b) The Administrator role is grant-all and immutable: read-only, Save disabled.
  await rolePanel.getByRole("button", { name: "Administrator" }).click();
  await expect(rolePanel.getByRole("button", { name: "Save role bundle" })).toBeDisabled();
  const adminBookCell = rolePanel.getByRole("button", { name: /^Book on Fixed Income:/ });
  await expect(adminBookCell).toHaveAttribute("aria-pressed", "true");
  await expect(adminBookCell).toBeDisabled();

  // Screenshot the Permissions page (Role bundles panel) at 1440.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({ path: "e2e-artifacts/role-bundles-1440.png", fullPage: true });

  // 6) (c) Cross-check: sign in AS the trader (their session was revoked by the
  // role-bundle change, so this is a fresh login deriving the narrowed base). Their
  // Rates-Book "Book position" control is DISABLED + carries the denial tooltip,
  // still VISIBLE (never hidden).
  await page.getByRole("button", { name: "Sign out" }).click();
  await signIn(page, TRADER_EMAIL, TRADER_PW);

  const ratesBookPane = await gotoRatesBook(page);
  const bookBtn = ratesBookPane.getByRole("button", { name: "Book position" });
  await expect(bookBtn).toBeVisible();
  await expect(bookBtn).toBeDisabled();
  await expect(bookBtn).toHaveAttribute("title", BOOK_DENIED_TITLE);
});
