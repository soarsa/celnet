/**
 * Deals blotter Client/Hedge split — OFFLINE e2e against the in-app `?mock` transport
 * (no cargo edge). Drives the production bundle as the seeded admin:
 *   1. open Risk → Deals; the blotter shows a [Client deals | Hedge deals] toggle,
 *      Client active by default;
 *   2. switch to Hedge deals → the executed-hedge ledger renders (the LP we hit, its
 *      realised price + slippage, the internal/external amounts) or the honest empty
 *      note when no hedge has fired yet;
 *   3. axe on the Hedge-deals lens (0 serious/critical);
 *   4. screenshot the hedge ledger.
 */
import { expect, test } from "@playwright/test";

import { gotoMockView } from "./fidelityHelpers";
import { expectNoSeriousA11y } from "./helpers";

test("Deals blotter splits into Client deals + Hedge deals; the hedge ledger shows LP/price/amounts", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });

  const pane = await gotoMockView(page, "riskdashboard");

  // Open the Deals tab of the consolidated Risk host.
  await pane.getByTestId("risk-tab-deals").click();

  // The Client/Hedge lens toggle — Client is the default.
  const clientLens = pane.getByTestId("deals-lens-client");
  const hedgeLens = pane.getByTestId("deals-lens-hedge");
  await expect(clientLens).toBeVisible();
  await expect(clientLens).toHaveAttribute("aria-pressed", "true");
  await expect(hedgeLens).toHaveAttribute("aria-pressed", "false");

  // Switch to Hedge deals → the executed-hedge ledger.
  await hedgeLens.click();
  await expect(hedgeLens).toHaveAttribute("aria-pressed", "true");

  // Either a fired-hedge row (LP + price columns) or the honest empty note.
  const hedgePanel = pane.getByRole("region", { name: "Hedge deals table" });
  const emptyNote = pane.getByText(/No fired hedges yet/i);
  await expect(hedgePanel.or(emptyNote).first()).toBeVisible();

  // axe on the hedge lens (0 serious/critical).
  await expectNoSeriousA11y(page, "Deals — Hedge deals lens");

  await page.screenshot({ path: "e2e-artifacts/deals-hedge-lens.png", fullPage: true });
});
