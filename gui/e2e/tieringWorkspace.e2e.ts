/**
 * TieringWorkspace — the trader-facing FI Tiering surface, SESSION-PIVOTED, end-to-
 * end over the production bundle (`?mock` transport — NO cargo, NO server).
 *
 * Book-level tiering is gone: outbound pricing is composed per client as a pricing-
 * group feature pipeline, and this surface pivots on the INBOUND FIX SESSIONS. The
 * ACCEPTANCE CRITERION: a signed-in FI user opens Fixed Income → Tiering (a first-
 * class rail entry, NOT behind the admin Manage toggle), sees the roster of FIX
 * sessions each badged with the pricing group that prices it (a session bind, a desk
 * default, or "No group"), and the detail pane resolves the selected session's
 * applied group read-only. An ADMIN additionally gets an enabled reassign control +
 * the "Edit in Pricing Groups" deep-link; a NON-ADMIN sees the assign control
 * disabled (read-only).
 *
 * The workspace logic + session→group resolution are covered by the vitest suite
 * (test/tieringWorkspace.test.tsx); this spec drives the real UI end-to-end.
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

import { signIn } from "./helpers";

type Page = import("@playwright/test").Page;

async function selectDomain(page: Page, name: string): Promise<void> {
  await page.getByRole("tablist", { name: "product domains" }).getByRole("tab", { name }).click();
}

async function openWorkspace(page: Page, label: string): Promise<void> {
  await page
    .getByRole("complementary", { name: "workspaces" })
    .locator(`button[title^="${label} ("]`)
    .click();
}

test("a NON-ADMIN trader sees the session roster + its resolved pricing read-only", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });

  // Sign in as the seeded NON-ADMIN trader (role TRADER). It holds
  // `quote_respond·fixed_income` but is NOT admin.
  await page.goto("/?mock");
  await signIn(page, "fi.trader@celnet.com", "password");
  await expect(page.getByRole("complementary", { name: "workspaces" })).toBeVisible();

  // Prove NON-ADMIN: the Administration domain tab is not present for this identity.
  await expect(
    page.getByRole("tablist", { name: "product domains" }).getByRole("tab", { name: "Administration" }),
  ).toHaveCount(0);

  // Tiering is a FIRST-CLASS rail entry under Fixed Income (not admin-gated).
  await selectDomain(page, "Fixed Income");
  await openWorkspace(page, "Tiering");

  // The session roster lists the seeded FIX session, badged with its resolved group
  // ("No group" — it binds no group and its desk carries no default).
  const roster = page.getByRole("region", { name: "select a FIX session" });
  await expect(roster).toBeVisible();
  const sessionBtn = roster.getByRole("button", { name: /Demo bank — Options/ });
  await expect(sessionBtn).toBeVisible();
  await expect(sessionBtn.getByText("No group")).toBeVisible();
  await sessionBtn.click();

  // The detail pane resolves the selected session's applied group (None here).
  const detail = page.getByRole("region", { name: "session pricing" });
  const applied = detail.getByLabel("applied pricing group");
  await expect(applied.getByText("None")).toBeVisible();

  // Non-admin ⇒ the reassign <select> is disabled (read-only) and the admin-only
  // "Edit tiering in Pricing Groups" deep-link is absent.
  await expect(detail.getByLabel("Assign to group")).toBeDisabled();
  await expect(
    detail.getByRole("button", { name: /Edit tiering in Pricing Groups/ }),
  ).toHaveCount(0);

  // Screenshot the delivered surface for the report.
  await page.screenshot({ path: "test-results/tiering-workspace-sessions.png" });

  // A11y pass SCOPED to the session-pricing detail (the surface this task delivers).
  // Freeze animations so axe samples resting colours.
  await page.addStyleTag({
    content: "*,*::before,*::after{animation:none!important;transition:none!important;}",
  });
  const results = await new AxeBuilder({ page })
    .include('[aria-label="session pricing"]')
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  const serious = results.violations.filter(
    (v) => v.impact === "serious" || v.impact === "critical",
  );
  expect(serious, JSON.stringify(serious.map((v) => ({ id: v.id, nodes: v.nodes.length })))).toEqual(
    [],
  );
});

test("the seeded admin sees the Tiering rail under Fixed Income and an enabled reassign control", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/?mock");
  await signIn(page); // default seeded admin
  await expect(page.getByRole("complementary", { name: "workspaces" })).toBeVisible();
  await selectDomain(page, "Fixed Income");
  await openWorkspace(page, "Tiering");

  // The admin gets the enabled reassign control + the Pricing Groups deep-link.
  const detail = page.getByRole("region", { name: "session pricing" });
  await expect(detail.getByLabel("Assign to group")).toBeEnabled();
  await expect(
    detail.getByRole("button", { name: /Edit tiering in Pricing Groups/ }),
  ).toBeVisible();
});
