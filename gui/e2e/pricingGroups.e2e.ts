/**
 * Pricing Groups pipeline builder — OFFLINE e2e against the in-app `?mock` transport
 * (no cargo edge, no server). Drives the production bundle end to end as the seeded
 * admin:
 *   1. open Administration → Pricing Groups and create a group;
 *   2. enable a custom ESP pipeline, then DRAG MID SHIFT + TIERING + AXE from the
 *      palette onto the canvas (asserting the live preview waterfall grows after each);
 *   3. reorder the cards BOTH by drag AND by the keyboard up/down buttons;
 *   4. configure a feature (MID SHIFT) and confirm the preview OUTBOUND two-way moves;
 *   5. toggle ESP/RFQ + the sharePipeline switch;
 *   6. assign a FIX connection member;
 *   7. Save, reselect the group, and confirm the pipeline round-tripped (persisted);
 *   8. run an axe a11y pass (0 serious/critical) and screenshot the canvas.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";

import { gotoMockView, installFrozenClock } from "./fidelityHelpers";
import { expectNoSeriousA11y } from "./helpers";

/**
 * Open the consolidated FI "Pricing" surface (its default "Pricing Groups" tab) from
 * the mock boot. The surface is `manage_pricing·fixed_income`-gated and now lives under
 * the Fixed Income domain (the Administration→Pricing-Groups rail entry was retired when
 * Pricing Groups + Tiering were consolidated into one FI "Pricing" workspace).
 */
async function openPricingGroups(page: Page): Promise<void> {
  await page
    .getByRole("tablist", { name: "product domains" })
    .getByRole("tab", { name: "Fixed Income" })
    .click();
  await page
    .getByRole("complementary", { name: "workspaces" })
    .getByRole("button", { name: "Pricing", exact: false })
    .click();
  await expect(page.getByRole("heading", { name: "Groups", exact: true })).toBeVisible();
}

/** The ordered feature-kind list currently on the canvas (via each card's data-kind). */
async function cardKinds(page: Page): Promise<string[]> {
  const cards = page.locator('[data-testid^="feature-card-"]');
  return cards.evaluateAll((els) => els.map((e) => e.getAttribute("data-kind") ?? ""));
}

/** The number of preview-waterfall rows (excluding the column header). */
async function previewRowCount(page: Page): Promise<number> {
  return page.locator('[data-testid^="preview-row-"]').count();
}

test("admin builds a pricing-group pipeline by drag-and-drop, saves, and it persists", async ({
  page,
}) => {
  await installFrozenClock(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await gotoMockView(page, "admin");
  await openPricingGroups(page);

  // 1) Create a new group.
  await page.getByRole("button", { name: "+ New" }).click();
  await page.locator("#pg-name").fill("E2E-TIER");
  await expect(page.getByRole("heading", { name: "New pricing group" })).toBeVisible();

  // 2) Enable a custom ESP pipeline so the palette + canvas appear.
  await page.locator("#pg-custom").check();
  const canvas: Locator = page.getByTestId("pipeline-canvas");
  await expect(canvas).toBeVisible();
  // Empty pipeline ⇒ the waterfall is just RAW (one row).
  expect(await previewRowCount(page)).toBe(1);

  // 3) DRAG MID SHIFT, then TIERING, then AXE from the palette onto the canvas (which
  //    appends at the end). Assert the live preview waterfall grows after each.
  const canvasDrop = page.getByTestId("pipeline-canvas");
  // Drop into the canvas's empty top-left padding (never over a card child) so the
  // canvas-level append handler fires — a deterministic "add to end".
  const appendPos = { targetPosition: { x: 24, y: 6 } };
  await page.getByTestId("palette-MID_SHIFT").dragTo(canvasDrop, appendPos);
  await expect(page.locator('[data-testid^="feature-card-"]')).toHaveCount(1);
  expect(await previewRowCount(page)).toBe(2); // RAW + MID SHIFT

  await page.getByTestId("palette-TIERING").dragTo(canvasDrop, appendPos);
  await expect(page.locator('[data-testid^="feature-card-"]')).toHaveCount(2);
  expect(await previewRowCount(page)).toBe(3);

  await page.getByTestId("palette-AXE").dragTo(canvasDrop, appendPos);
  await expect(page.locator('[data-testid^="feature-card-"]')).toHaveCount(3);
  expect(await previewRowCount(page)).toBe(4); // RAW + 3 features
  expect(await cardKinds(page)).toEqual(["MID_SHIFT", "TIERING", "AXE"]);

  // 4) Configure MID SHIFT — expand card 0, set its shift to 0.2, and confirm the
  //    preview updates: row 1 (after MID SHIFT) bid becomes mid(99.75) − half(0.05).
  const outBidBefore = await page.getByTestId("preview-row-3").locator("span").nth(1).innerText();
  await page.getByTestId("feature-card-0").getByRole("button", { name: /MID SHIFT feature/ }).click();
  await page.locator("#pg-f0-shift").fill("0.2");
  await expect
    .poll(async () => page.getByTestId("preview-row-1").locator("span").nth(1).innerText())
    .toBe("99.7000");
  const outBidAfter = await page.getByTestId("preview-row-3").locator("span").nth(1).innerText();
  expect(outBidAfter).not.toBe(outBidBefore);

  // 5) Reorder by DRAG: drag AXE (card 2) onto MID SHIFT (card 0) ⇒ AXE moves to front.
  await page.getByTestId("feature-card-2").dragTo(page.getByTestId("feature-card-0"));
  expect(await cardKinds(page)).toEqual(["AXE", "MID_SHIFT", "TIERING"]);

  // 6) Reorder by KEYBOARD: move the first card (AXE) DOWN one via its button.
  await page.getByTestId("feature-card-0").getByRole("button", { name: "Move AXE down" }).click();
  expect(await cardKinds(page)).toEqual(["MID_SHIFT", "AXE", "TIERING"]);

  // 7) ESP/RFQ + share switch: turn sharing ON ⇒ the RFQ tab disables + mirrors ESP.
  await page.locator("#pg-share").check();
  await expect(page.getByRole("tab", { name: /RFQ/ })).toBeDisabled();
  await page.locator("#pg-share").uncheck();
  await expect(page.getByRole("tab", { name: /RFQ/ })).toBeEnabled();

  // 8) Assign a FIX connection member (the mock seeds "Demo bank — Options").
  await page.getByRole("checkbox", { name: /Demo bank/ }).check();
  await expect(page.getByRole("checkbox", { name: /Demo bank/ })).toBeChecked();

  // Screenshot the built canvas for the artifact.
  await page.getByTestId("pipeline-canvas").scrollIntoViewIfNeeded();
  await page.screenshot({ path: "e2e-artifacts/pricing-groups-canvas-1440.png", fullPage: true });

  // 9) Save (Create). A successful create CLOSES the editor popup and the roster then
  //    carries the new group; select it and confirm the pipeline persisted (3 cards
  //    round-tripped through the mock registry).
  await page.getByRole("button", { name: "Create group" }).click();
  await expect(page.getByRole("dialog")).toBeHidden();

  // Reopen E2E-TIER from the roster (the modal closed on save) — a fresh reseed from
  // the store, confirming the pipeline persisted.
  await page.getByRole("button", { name: "E2E-TIER", exact: false }).first().click();
  await expect(page.getByRole("heading", { name: "E2E-TIER" })).toBeVisible();
  await expect(page.locator('[data-testid^="feature-card-"]')).toHaveCount(3);
  expect(await cardKinds(page)).toEqual(["MID_SHIFT", "AXE", "TIERING"]);

  // 10) axe: 0 serious/critical on the pipeline builder.
  await page.clock.runFor(30_000);
  await expectNoSeriousA11y(page, "pricing groups workspace (pipeline builder)");
});

test("admin sets the last-look policy (Async + tolerance + giveback), saves, and it persists", async ({
  page,
}) => {
  await installFrozenClock(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await gotoMockView(page, "admin");
  await openPricingGroups(page);

  // 1) Create a new group.
  await page.getByRole("button", { name: "+ New" }).click();
  await page.locator("#pg-name").fill("E2E-LL");
  await expect(page.getByRole("heading", { name: "New pricing group" })).toBeVisible();

  // 2) Last-look defaults to Sync ⇒ the async-giveback control is hidden.
  const mode = page.locator("#pg-last-look-mode");
  await expect(mode).toHaveValue("0");
  await expect(page.locator("#pg-async-giveback")).toHaveCount(0);

  // 3) Set the adverse-move tolerance (protects the desk in both modes).
  await page.locator("#pg-last-look-tol").fill("2.5");

  // 4) Switch to Async ⇒ the giveback slider appears; set it to 40%.
  await mode.selectOption("1");
  const giveback = page.locator("#pg-async-giveback");
  await expect(giveback).toBeVisible();
  await giveback.fill("40");

  // Screenshot the last-look section for the artifact.
  await page.screenshot({ path: "e2e-artifacts/pricing-groups-last-look-1440.png", fullPage: true });

  // 5) Save (Create) — a successful create CLOSES the editor popup. Then reselect the
  //    group from the roster to force a fresh reseed from the store.
  await page.getByRole("button", { name: "Create group" }).click();
  await expect(page.getByRole("dialog")).toBeHidden();

  // Reopen E2E-LL from the roster (the modal closed on save) — a fresh reseed from the store.
  await page.getByRole("button", { name: "E2E-LL", exact: false }).first().click();
  await expect(page.getByRole("heading", { name: "E2E-LL" })).toBeVisible();

  // 6) The last-look policy round-tripped: mode Async, tolerance 2.5, giveback 40.
  await expect(page.locator("#pg-last-look-mode")).toHaveValue("1");
  await expect(page.locator("#pg-last-look-tol")).toHaveValue("2.5");
  await expect(page.locator("#pg-async-giveback")).toHaveValue("40");

  // 7) Flip back to Sync ⇒ the giveback control disappears (and is dropped from the write).
  await page.locator("#pg-last-look-mode").selectOption("0");
  await expect(page.locator("#pg-async-giveback")).toHaveCount(0);

  // 8) axe: 0 serious/critical with the last-look section rendered.
  await page.clock.runFor(30_000);
  await expectNoSeriousA11y(page, "pricing groups workspace (last-look policy)");
});
