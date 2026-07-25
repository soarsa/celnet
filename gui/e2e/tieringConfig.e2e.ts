/**
 * FI-TIERING phase 3 — per-book outbound-tiering admin config, end-to-end over the
 * production bundle (`?mock` transport — NO cargo, NO celnet-server).
 *
 * The seeded admin opens the FI "Agg Book" workspace, switches to its admin-only
 * "Manage" mode (proving the book + tiering admin is reachable UNDER Fixed Income),
 * defines a book with a Flat 25 price-bps tiering config, saves it, reopens it, and
 * asserts the config round-trips through the mock store. It also adds/removes a
 * second strategy and confirms a bad guardrail (spread_floor = 0) blocks the save.
 *
 * The wire codec (spec→wire→desc) is covered exhaustively by the vitest suite
 * (test/tiering.test.ts); this spec drives the UI + form + round-trip that the unit
 * tests cannot. The GUI code path is transport-agnostic, so a green offline run
 * exercises the same components a live run would against real WS frames.
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

const BOOK_NAME = "E2E Tiering Book";

test("admin adds a Flat 25 price-bps tiering config to a book under Fixed Income; it round-trips", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });

  // Boot the mock (real-time — NO fake clock: the mock's async CRUD resolves on the
  // real microtask queue) + sign in as the seeded admin, then cross to the FI view.
  await page.goto("/?mock");
  await signIn(page);
  await expect(page.getByRole("complementary", { name: "workspaces" })).toBeVisible();
  await selectDomain(page, "Fixed Income");
  await openWorkspace(page, "Agg Book");

  // Switch to the admin-only Manage mode — the book + tiering admin lives here,
  // under Fixed Income (not only under Administration → Aggregation).
  await page.getByRole("group", { name: "aggregated book mode" }).getByRole("button", {
    name: "Manage",
  }).click();
  await expect(page.getByRole("heading", { name: "Define a book" })).toBeVisible();

  // Define a book: a name + two LP-SIM members (≥2 satisfies the default quorum).
  await page.getByLabel("aggregated book name").fill(BOOK_NAME);
  await page.getByRole("button", { name: "+ LP-SIM-01" }).click();
  await page.getByRole("button", { name: "+ LP-SIM-02" }).click();

  // Create it (no tiering yet) — the roster refetches and the new book appears.
  await page.getByRole("button", { name: "Create book" }).click();
  const rosterRow = page
    .getByRole("row")
    .filter({ has: page.getByText(BOOK_NAME, { exact: true }) });
  await expect(rosterRow).toBeVisible();

  // Edit it → add a Flat 25 price-bps tiering config (the default enabled config).
  await rosterRow.getByRole("button", { name: "Edit" }).click();
  await expect(page.getByRole("heading", { name: "Edit book" })).toBeVisible();
  const tiering = page.getByRole("region", { name: "outbound tiering configuration" });
  await tiering.getByLabel("enable outbound tiering").check();
  await expect(tiering.getByLabel("Spread unit")).toHaveValue("PRICE_BPS");
  await expect(tiering.getByText("Flat markup", { exact: true })).toBeVisible();
  await expect(tiering.getByLabel("Half-spread H")).toHaveValue("25");

  // Save it — persists the tiering config on the book.
  await page.getByRole("button", { name: "Save book" }).click();

  // Reopen it — the tiering config round-trips (enabled, Price bps, half-spread 25).
  await rosterRow.getByRole("button", { name: "Edit" }).click();
  await expect(page.getByRole("heading", { name: "Edit book" })).toBeVisible();
  await expect(tiering.getByLabel("enable outbound tiering")).toBeChecked();
  await expect(tiering.getByLabel("Spread unit")).toHaveValue("PRICE_BPS");
  await expect(tiering.getByLabel("Half-spread H")).toHaveValue("25");

  // Add a second strategy (Inventory skew) → its κ/sMax fields appear; remove it.
  await tiering.getByRole("button", { name: "+ Inventory skew" }).click();
  await expect(tiering.getByText("Inventory skew", { exact: true })).toBeVisible();
  await expect(tiering.getByLabel("κ (per unit inventory)")).toBeVisible();
  await tiering.getByRole("button", { name: "Remove Inventory skew strategy" }).click();
  await expect(tiering.getByText("Inventory skew", { exact: true })).toHaveCount(0);

  // A bad guardrail (spread_floor = 0) surfaces an inline error + blocks the save.
  const floor = tiering.getByLabel("spread_floor");
  await floor.fill("0");
  await expect(tiering.getByText("spreadFloor must be a finite value > 0.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Save book" })).toBeDisabled();

  // Restore a valid floor → the save re-enables.
  await floor.fill("0.01");
  await expect(page.getByRole("button", { name: "Save book" })).toBeEnabled();

  // Screenshot the tiering editor (the deliverable surface) for the report.
  await page
    .getByRole("region", { name: "manage aggregated books" })
    .screenshot({ path: "test-results/tiering-editor.png" });

  // A11y pass SCOPED to the tiering editor (the surface this task delivers). Freeze
  // animations so axe samples resting colours. NB: the whole-page scan surfaces the
  // app's PRE-EXISTING dark-theme accent-contrast debt (the AuthMenu role badge +
  // every `.segBtnActive` segmented control fail AA independent of this change), so
  // we validate the editor region itself, which is clean.
  await page.addStyleTag({
    content: "*,*::before,*::after{animation:none!important;transition:none!important;}",
  });
  const results = await new AxeBuilder({ page })
    .include('[aria-label="outbound tiering configuration"]')
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  const serious = results.violations.filter(
    (v) => v.impact === "serious" || v.impact === "critical",
  );
  expect(serious, JSON.stringify(serious.map((v) => ({ id: v.id, nodes: v.nodes.length })))).toEqual(
    [],
  );
});
