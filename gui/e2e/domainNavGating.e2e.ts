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
import { test, expect, type Locator, type Page } from "@playwright/test";

import { openLive, openLiveAt, signIn, gotoWorkspace, expectNoSeriousA11y } from "./helpers";

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
  const btn = rail(page).locator(`button[title^="${label} ("]`);
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

/** The product-domain tab bar. */
function domainTabs(page: Page): Locator {
  return page.getByRole("tablist", { name: "product domains" });
}
/** One domain tab by its exact label. */
function domainTab(page: Page, label: string): Locator {
  return domainTabs(page).getByRole("tab", { name: label, exact: true });
}
/** Select a domain tab. */
async function selectDomain(page: Page, label: string): Promise<void> {
  await domainTab(page, label).click();
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

  // 4a) The product-domain TAB BAR gates by `domainAccessible`: the FX Options tab
  // is HIDDEN (no `view·fx_options`), Administration is HIDDEN (not admin), and the
  // Fixed Income tab is present AND selected (the redirect re-homed the active domain
  // to an accessible one so the trader never sits on a dead FX tab).
  await expect(domainTabs(page)).toBeVisible();
  await expect(domainTab(page, "FX Options")).toHaveCount(0);
  await expect(domainTab(page, "Administration")).toHaveCount(0);
  await expect(domainTab(page, "Fixed Income")).toBeVisible();
  await expect(domainTab(page, "Fixed Income")).toHaveAttribute("aria-selected", "true");

  // 4b) Under the FI tab the FX-ONLY rows are absent (not FI-domain rows).
  await expectRail(page, "Stream", false);
  await expectRail(page, "XVA", false);
  await expectRail(page, "Excel", false);

  // 4c) The CROSS-ASSET (shared) rows remain (reachable via `view·fixed_income`),
  // and the FI-only Quoting is present — the trader is not stranded on a blank view.
  for (const label of ["Ticket", "Market Data", "Risk", "Book", "Quoting"]) {
    await expectRail(page, label, true);
  }

  // 4d) The admin/ops rows are absent (the Administration tab is hidden for a
  // non-admin, so nothing surfaces them).
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

  // 6) Sign back in as the admin → all three domain tabs are present, each with its
  // own rail rows (the FX-only + FI-only + admin rows return under their tabs).
  await page.getByRole("button", { name: "Sign out" }).click();
  await signIn(page);
  await expect(domainTab(page, "FX Options")).toBeVisible();
  await expect(domainTab(page, "Fixed Income")).toBeVisible();
  await expect(domainTab(page, "Administration")).toBeVisible();

  // FX tab: the FX-domain rows present; the FI-only Quoting absent. (activeDomain
  // persists across the sign-out/in, so select the FX tab explicitly first.)
  await selectDomain(page, "FX Options");
  await expect(domainTab(page, "FX Options")).toHaveAttribute("aria-selected", "true");
  for (const label of ["Ticket", "Stream", "Market Data", "Risk", "Book", "XVA", "Excel"]) {
    await expectRail(page, label, true);
  }
  await expectRail(page, "Quoting", false);

  // Fixed Income tab: Quoting appears; the FX-only Stream is hidden.
  await selectDomain(page, "Fixed Income");
  await expectRail(page, "Quoting", true);
  await expectRail(page, "Stream", false);

  // Administration tab: the admin/ops rows appear.
  await selectDomain(page, "Administration");
  for (const label of ["Admin", "Permissions", "Connections", "Reference Data"]) {
    await expectRail(page, label, true);
  }

  // 7) Model A — a SHARED screen appears under BOTH trading tabs and pre-selects the
  // tab's lens. Market Data: FX tab ⇒ FX lens; FI tab ⇒ rates lens (screen kept).
  await selectDomain(page, "FX Options");
  await railClick(page, "Market Data");
  const mdLens = page.getByRole("group", { name: "market data asset class" });
  await expect(mdLens.getByRole("button", { name: "FX Options" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await selectDomain(page, "Fixed Income");
  await expect(rail(page).locator('button[title^="Market Data ("]')).toBeVisible(); // kept
  await expect(mdLens.getByRole("button", { name: "Fixed Income" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  // Risk: its lens is a tablist — FX tab ⇒ FX lens tab; FI tab ⇒ rates lens tab.
  await selectDomain(page, "FX Options");
  await railClick(page, "Risk");
  const riskLens = page.getByRole("tablist", { name: "risk asset class lens" });
  await expect(riskLens.getByRole("tab", { name: "FX Options" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await selectDomain(page, "Fixed Income");
  await expect(riskLens.getByRole("tab", { name: "Fixed Income" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
});

test("deep-link ?dom=… drives the active tab AND the shared-screen lens on first paint (no tab click)", async ({
  page,
}) => {
  // A pasted / reloaded link carries the active DOMAIN as `dom`. On load — through
  // the login gate, with NO tab click — the domain tab bar AND the shared screen's
  // FX↔rates lens must both match the URL on the FIRST paint (the regression: the
  // FX tab/lens showed regardless of `dom`). Admin has every class, so no re-home.

  // 1) Risk deep-linked to Fixed Income → the FI tab AND the rates risk lens.
  await openLiveAt(page, "dom=fixed_income&view=risk");
  await expect(domainTab(page, "Fixed Income")).toHaveAttribute("aria-selected", "true");
  await expect(domainTab(page, "FX Options")).toHaveAttribute("aria-selected", "false");
  const riskLens = page.getByRole("tablist", { name: "risk asset class lens" });
  await expect(riskLens.getByRole("tab", { name: "Fixed Income" })).toHaveAttribute(
    "aria-selected",
    "true",
  );

  // 2) Market Data deep-linked to Fixed Income → the rates (curve) lens on load.
  await openLiveAt(page, "dom=fixed_income&view=surface");
  await expect(domainTab(page, "Fixed Income")).toHaveAttribute("aria-selected", "true");
  const mdLens = page.getByRole("group", { name: "market data asset class" });
  await expect(mdLens.getByRole("button", { name: "Fixed Income" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  // 3) An old link with NO `dom` still decodes to the fx_options default (forward-
  // compat): the FX tab is active and Market Data opens the FX vol-surface lens.
  await openLiveAt(page, "view=surface");
  await expect(domainTab(page, "FX Options")).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByRole("group", { name: "market data asset class" }).getByRole("button", {
      name: "FX Options",
    }),
  ).toHaveAttribute("aria-pressed", "true");
});
