/**
 * Admin section tabs — OFFLINE e2e against the in-app `?mock` transport (no cargo
 * edge, no server). The Administration screen splits its four sections — Users ·
 * Desks · Legal Entities · Netting Books — into TAB panes (one visible at a time,
 * Users by default) using the app's lens-tab pattern. This drives the production
 * bundle end to end:
 *   1. sign in as the seeded admin (mock) and open the Admin workspace;
 *   2. assert the four section tabs exist and Users is the default pane;
 *   3. assert the Users row lays out cleanly — all four action controls (Edit /
 *      Reset password / Permissions / Delete) are present AND do not overlap the
 *      capability chips (a bounding-box check: the actions sit to the right of the
 *      chips, no intersection);
 *   4. click each of the other three tabs and confirm its section pane renders
 *      (and the Users roster is swapped out).
 */
import { expect, test, type Locator } from "@playwright/test";

import { gotoMockView, installFrozenClock } from "./fidelityHelpers";
import { expectNoSeriousA11y } from "./helpers";

/** True when rectangles a and b do not intersect (adjacent/edge-touching is fine). */
function disjoint(a: { x: number; y: number; width: number; height: number }, b: typeof a): boolean {
  return (
    a.x + a.width <= b.x + 0.5 ||
    b.x + b.width <= a.x + 0.5 ||
    a.y + a.height <= b.y + 0.5 ||
    b.y + b.height <= a.y + 0.5
  );
}

async function box(locator: Locator) {
  const b = await locator.boundingBox();
  if (!b) throw new Error("expected a rendered bounding box");
  return b;
}

test("admin screen tabifies the four sections; Users row actions do not overlap the chips", async ({
  page,
}) => {
  await installFrozenClock(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  // Boot the mock + sign in as the seeded admin. Admin lives under the
  // Administration DOMAIN (the top product-domains tab), so select that domain
  // then open the Admin workspace from the rail.
  await gotoMockView(page, "admin");
  await page
    .getByRole("tablist", { name: "product domains" })
    .getByRole("tab", { name: "Administration" })
    .click();
  await page
    .getByRole("complementary", { name: "workspaces" })
    .getByRole("button", { name: "Admin", exact: false })
    .click();

  // 1) The four section tabs exist, and Users is the default selected pane.
  for (const label of ["Users", "Desks", "Legal Entities", "Netting Books"]) {
    await expect(page.getByRole("tab", { name: label })).toBeVisible();
  }
  await expect(page.getByRole("tab", { name: "Users" })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("heading", { name: "Users", exact: true })).toBeVisible();
  // Sibling panes are not mounted while Users is active (one section at a time).
  await expect(page.getByRole("heading", { name: "Desks", exact: true })).toHaveCount(0);

  // 2) The Users roster's first row: every action control is present and queryable.
  const userRow = page
    .getByRole("row")
    .filter({ has: page.getByRole("button", { name: "Permissions" }) })
    .first();
  for (const name of ["Edit", "Reset password", "Permissions", "Delete"]) {
    await expect(userRow.getByRole("button", { name })).toBeVisible();
  }
  // The capability chip cell coexists in the SAME row.
  const chip = userRow.getByText(/FX \d+\/\d+/).first();
  await expect(chip).toBeVisible();

  // 3) No overlap: the row's action group sits clear of the capability chips.
  const chipBox = await box(chip);
  for (const name of ["Edit", "Reset password", "Permissions", "Delete"]) {
    const actionBox = await box(userRow.getByRole("button", { name }));
    expect(
      disjoint(chipBox, actionBox),
      `action "${name}" must not overlap the capability chips`,
    ).toBe(true);
  }

  // 4) Each of the other tabs swaps in its own section pane.
  await page.getByRole("tab", { name: "Desks" }).click();
  await expect(page.getByRole("heading", { name: "Desks", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Add desk" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Users", exact: true })).toHaveCount(0);

  await page.getByRole("tab", { name: "Legal Entities" }).click();
  await expect(page.getByRole("heading", { name: "Legal entities", exact: true })).toBeVisible();

  await page.getByRole("tab", { name: "Netting Books" }).click();
  await expect(page.getByRole("heading", { name: "Netting books", exact: true })).toBeVisible();

  // Back to Users for the artifact screenshot.
  await page.getByRole("tab", { name: "Users" }).click();
  await expect(page.getByRole("heading", { name: "Users", exact: true })).toBeVisible();
  await page.screenshot({ path: "e2e-artifacts/admin-section-tabs-1440.png", fullPage: true });
});

/** Open the Administration domain's Admin workspace from the mock boot state. */
async function openAdmin(page: import("@playwright/test").Page): Promise<void> {
  await page
    .getByRole("tablist", { name: "product domains" })
    .getByRole("tab", { name: "Administration" })
    .click();
  await page
    .getByRole("complementary", { name: "workspaces" })
    .getByRole("button", { name: "Admin", exact: false })
    .click();
}

test("admin sets a user to two desks and to All desks — MANY-TO-MANY membership persists", async ({
  page,
}) => {
  await installFrozenClock(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await gotoMockView(page, "admin");
  await openAdmin(page);

  // 1) Define two desks in the Desks pane.
  await page.getByRole("tab", { name: "Desks" }).click();
  for (const name of ["G10 Options", "EM Rates"]) {
    await page.getByLabel("new desk name").fill(name);
    await page.getByRole("button", { name: "Add desk" }).click();
    await expect(page.getByRole("cell", { name, exact: true })).toBeVisible();
  }

  // 2) Create a deskless trader.
  await page.getByRole("tab", { name: "Users" }).click();
  await page.getByRole("button", { name: "New user" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByPlaceholder("trader@celnet.com").fill("trader@celnet.com");
  await dialog.getByPlaceholder("Jane Trader").fill("Jane Trader");
  await dialog.getByPlaceholder("at least 12 characters").fill("longenoughpw1");
  await dialog.getByRole("button", { name: "Create user" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // The new trader starts deskless (receives no inbound traffic).
  const traderRow = () =>
    page.getByRole("row").filter({ has: page.getByText("trader@celnet.com") });
  await expect(traderRow().getByText("receives no quotes")).toBeVisible();

  // 3) Assign to TWO desks via the inline multi-select checkboxes.
  await traderRow().getByRole("checkbox", { name: "G10 Options" }).check();
  await traderRow().getByRole("checkbox", { name: "EM Rates" }).check();
  await expect(traderRow().getByRole("checkbox", { name: "G10 Options" })).toBeChecked();
  await expect(traderRow().getByRole("checkbox", { name: "EM Rates" })).toBeChecked();
  await expect(traderRow().getByText("receives no quotes")).toHaveCount(0);

  // 4) Persist across a Refresh (re-reads the authoritative roster).
  await page.getByRole("button", { name: "Refresh" }).click();
  await expect(traderRow().getByRole("checkbox", { name: "G10 Options" })).toBeChecked();
  await expect(traderRow().getByRole("checkbox", { name: "EM Rates" })).toBeChecked();

  // 5) Switch to All desks — the per-desk list collapses (All supersedes the set).
  await traderRow().getByRole("checkbox", { name: /All desks/ }).check();
  await expect(traderRow().getByRole("checkbox", { name: "G10 Options" })).toHaveCount(0);

  // 6) All-desks membership persists across a Refresh.
  await page.getByRole("button", { name: "Refresh" }).click();
  await expect(traderRow().getByRole("checkbox", { name: /All desks/ })).toBeChecked();
  await expect(traderRow().getByRole("checkbox", { name: "G10 Options" })).toHaveCount(0);

  await page.screenshot({ path: "e2e-artifacts/admin-multi-desk-1440.png", fullPage: true });

  // Let transient create-success toasts auto-dismiss (the frozen clock holds them
  // open) so the a11y pass covers the steady-state admin surface — the desk
  // membership UI — not an ephemeral toast owned by another component.
  await page.clock.runFor(30_000);
  await expect(page.locator("[class*='toastAction']")).toHaveCount(0);

  // axe on the multi-desk membership UI: 0 serious/critical.
  await expectNoSeriousA11y(page, "admin workspace (multi-desk membership)");
});
