/**
 * Own-affordance capability gating (slice 5) — LIVE e2e against the real demo
 * edge under Enforce.
 *
 * This is the trader-facing other half of the slice-4 admin matrix e2e: it proves
 * that the LOGGED-IN user's OWN affordances are gated on their effective
 * capability set (`LoginResponse.capabilities`) — DISABLED with an explanatory
 * tooltip, never hidden — and that the gating narrows EXACTLY the denied
 * capability while leaving siblings (and the admin) untouched.
 *
 * Flow (all through the production bundle + live WS mirror):
 *   1. Sign in as the seeded admin; create a fresh trader.
 *   2. Via the capability matrix, DENY that trader `book·fixed_income` and save
 *      (a real overlay write that revokes the trader's sessions server-side).
 *   3. Confirm the ADMIN's own Rates-Book "Book position" control is ENABLED
 *      (the admin holds book·FI) — gating is per-identity, (c).
 *   4. Sign out; sign back in AS the trader (re-deriving their effective set).
 *   5. Confirm the trader's Rates-Book "Book position" is DISABLED and carries
 *      the denial tooltip, while still VISIBLE (never hidden) — (a).
 *   6. Confirm a sibling FI affordance they DO hold — Rates "Request quote"
 *      (price·FI) — stays ENABLED — (b).
 *   7. Screenshot the trader's gated view at 1440 + an axe pass (0 serious).
 *
 * `book·fixed_income` is the denied capability because the Rates-Book "Book
 * position" button is a single, always-rendered control with no inbound-state
 * prerequisite — a deterministic gate. (The task's `execute·fixed_income` example
 * lives on the QuotingWorkspace accept, which needs a live QUOTED desk request to
 * appear; book is the equivalent FI affordance with a stable selector.)
 */
import { test, expect, type Page } from "@playwright/test";

import { openLive, signIn, expectNoSeriousA11y } from "./helpers";

const TRADER_EMAIL = `gate-trader-${Date.now()}@celnet.com`;
const TRADER_PW = "longenoughpw1";
const BOOK_DENIED_TITLE = "Your permissions don't allow booking fixed-income positions.";

/** Click a top-level product-domain tab (FX Options / Fixed Income / Administration). */
async function selectDomain(page: Page, name: string): Promise<void> {
  await page.getByRole("tab", { name }).click();
}

/**
 * Click a workspace rail button by label within the active domain. The rail
 * button's accessible name embeds the workspace glyph as a prefix, so a string
 * is matched as a substring; pass an END-ANCHORED RegExp (e.g. `/Rates$/`) to
 * disambiguate "Rates" from "Rates Risk"/"Rates Book".
 */
async function railClick(page: Page, label: string | RegExp): Promise<void> {
  await page
    .getByRole("complementary", { name: "workspaces" })
    .getByRole("button", { name: label, exact: false })
    .click();
}

// Hidden panes stay MOUNTED but carry `inert` + `aria-hidden="true"`; Playwright's
// role queries already exclude aria-hidden subtrees, so a page-level `getByRole`
// resolves only the ACTIVE pane's control (and "Book position" / Rates "Request
// quote" are each unique to one workspace anyway).

test("a denied capability disables exactly that affordance for the trader", async ({ page }) => {
  // 1) Admin signs in; create a fresh trader.
  await openLive(page);
  await selectDomain(page, "Administration");
  await railClick(page, "Admin");

  await page.getByRole("button", { name: "New user" }).click();
  await page.getByPlaceholder("trader@celnet.com").fill(TRADER_EMAIL);
  await page.getByPlaceholder("Jane Trader").fill("Gate Trader");
  await page.getByPlaceholder("at least 12 characters").fill(TRADER_PW);
  await page.getByRole("button", { name: "Create user" }).click();

  // 2) Deny book·fixed_income on the trader and save.
  const traderRow = page.getByRole("row", { name: new RegExp(TRADER_EMAIL) });
  await traderRow.getByRole("button", { name: "Permissions" }).click();
  await expect(
    page.getByRole("heading", { name: new RegExp(`Capabilities — ${TRADER_EMAIL}`) }),
  ).toBeVisible();

  const bookFi = page.getByRole("button", { name: /^Book on Fixed Income:/ });
  // Trader role bundle allows book (inherit); inherit → grant → deny.
  await expect(bookFi).toHaveAttribute("aria-label", /allowed, overlay Inherit/);
  await bookFi.click();
  await bookFi.click();
  await expect(bookFi).toHaveAttribute("aria-label", /blocked, overlay Deny/);

  await page.getByRole("button", { name: "Save capabilities" }).click();
  await expect(
    page.getByText("Saved — the user's active sessions were ended."),
  ).toBeVisible();

  // 3) The ADMIN's OWN Rates-Book control is ENABLED (admin holds book·FI).
  await selectDomain(page, "Fixed Income");
  await railClick(page, "Rates Book");
  const adminBookBtn = page.getByRole("button", { name: "Book position" });
  await expect(adminBookBtn).toBeVisible();
  await expect(adminBookBtn).toBeEnabled();

  // 4) Sign out; sign back in AS the trader (re-derives their effective set).
  await page.getByRole("button", { name: "Sign out" }).click();
  await signIn(page, TRADER_EMAIL, TRADER_PW);

  // 5) The trader's Rates-Book "Book position" is DISABLED + tooltip, NOT hidden.
  await selectDomain(page, "Fixed Income");
  await railClick(page, "Rates Book");
  const bookBtn = page.getByRole("button", { name: "Book position" });
  await expect(bookBtn).toBeVisible(); // never hidden
  await expect(bookBtn).toBeDisabled(); // gated off
  await expect(bookBtn).toHaveAttribute("title", BOOK_DENIED_TITLE);

  // 6) A sibling FI affordance they DO hold — Rates "Request quote" (price·FI) —
  // stays ENABLED.
  await railClick(page, /Rates$/);
  const priceBtn = page.getByRole("button", { name: /Request quote|Pricing…/ });
  await expect(priceBtn).toBeVisible();
  await expect(priceBtn).toBeEnabled();

  // 7) Screenshot the trader's gated view at 1440 + an axe pass.
  await selectDomain(page, "Fixed Income");
  await railClick(page, "Rates Book");
  await expect(page.getByRole("button", { name: "Book position" })).toBeDisabled();
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({
    path: "e2e-artifacts/own-affordance-gating-trader-1440.png",
    fullPage: true,
  });
  await expectNoSeriousA11y(page, "trader gated rates-book (book·FI denied)");
});
