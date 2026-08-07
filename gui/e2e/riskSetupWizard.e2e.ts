/**
 * Risk guided setup — OFFLINE e2e against the in-app `?mock` transport (no cargo edge,
 * no server). Drives the production bundle end to end as the seeded admin (who holds
 * `risk_manage` + `manage_acceptance`, so the whole wizard is editable):
 *   1. open the consolidated "Risk" host, launch "Guided setup";
 *   2. walk all FOUR steps — create a risk portfolio, route the default into it, keep
 *      the seeded acceptance reject rule, review — running axe (0 serious/critical) on
 *      each step and screenshotting the stepper;
 *   3. Apply — then PROVE all three RPCs fired with CONSISTENT ids by observing their
 *      persisted effects on the mock store: the wizard lands on the Acceptance tab
 *      showing the reject rule (updateAcceptanceGraph), the Portfolios tab shows the new
 *      book (createRiskBook), and the Routing tab routes the default into that SAME book
 *      (updateRiskRoutingGraph, re-pointed at the minted id).
 *
 * The books+routing spine + the ordered apply are unit-proven in
 * `test/riskWizardModel.test.ts` / `test/riskSetupWizard.test.tsx`; this is the live gate.
 */
import { expect, test, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

import { gotoMockView, installFrozenClock } from "./fidelityHelpers";

const PORTFOLIO = "EMEA Risk Wizard";

/**
 * Axe over the WIZARD DIALOG only (0 serious/critical). Scoped to `[role="dialog"]` so
 * the scan targets the guided-setup surface under test — the app's global notification
 * toasts render in a portal OUTSIDE the dialog and carry their own pre-existing contrast
 * debt, which is not what this wizard e2e gates.
 */
async function expectNoSeriousA11yInWizard(page: Page, context: string): Promise<void> {
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation:none!important;transition:none!important;animation-duration:0s!important;}",
  });
  const results = await new AxeBuilder({ page })
    .include('[role="dialog"]')
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  const serious = results.violations.filter(
    (v) => v.impact === "serious" || v.impact === "critical",
  );
  if (serious.length > 0) {
    const summary = serious
      .map((v) => `  [${v.impact}] ${v.id}: ${v.help} (${v.nodes.length} node(s))`)
      .join("\n");
    throw new Error(`axe found ${serious.length} serious/critical violation(s) on ${context}:\n${summary}`);
  }
  process.stdout.write(
    `[a11y] ${context}: 0 serious/critical (${results.violations.length} moderate/minor noted)\n`,
  );
}

test("Risk guided setup walks portfolios → routing → acceptance → review, then applies all three", async ({
  page,
}) => {
  await installFrozenClock(page);
  await page.setViewportSize({ width: 1440, height: 900 });

  // Open the consolidated "Risk" host as the seeded admin; scope to the active pane.
  const pane = await gotoMockView(page, "riskdashboard");

  // --- launch the wizard ------------------------------------------------------
  await pane.getByTestId("open-risk-guided-setup").click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("heading", { name: "Risk guided setup" })).toBeVisible();

  // --- step 1 · risk portfolios ----------------------------------------------
  await expect(dialog.getByTestId("wiz-portfolio-0")).toBeVisible();
  await dialog.getByTestId("wiz-portfolio-name-0").fill(PORTFOLIO);
  await page.screenshot({ path: "e2e-artifacts/risk-wizard-step1-stepper.png", fullPage: true });
  await expectNoSeriousA11yInWizard(page, "Risk guided setup — step 1 (portfolios)");
  await dialog.getByTestId("wiz-next").click();

  // --- step 2 · routing (route the Otherwise default into the new portfolio) --
  await expect(dialog.getByTestId("wiz-default-rule")).toBeVisible();
  await dialog.getByTestId("wiz-default-target").selectOption({ label: PORTFOLIO });
  await expectNoSeriousA11yInWizard(page, "Risk guided setup — step 2 (routing)");
  await dialog.getByTestId("wiz-next").click();

  // --- step 3 · acceptance (the seeded reject-below-edge + accept-all) --------
  await expect(dialog.getByTestId("risk-wiz-acceptance")).toBeVisible();
  const seededRule = dialog.getByTestId("risk-wiz-acc-rule-0");
  await expect(seededRule).toBeVisible();
  await expect(seededRule).toContainText("below edge floor");
  await expect(dialog.getByTestId("risk-wiz-acc-default")).toContainText("Accept");
  await expectNoSeriousA11yInWizard(page, "Risk guided setup — step 3 (acceptance)");
  await dialog.getByTestId("wiz-next").click();

  // --- step 4 · review --------------------------------------------------------
  await expect(dialog.getByTestId("risk-wiz-review-acceptance")).toBeVisible();
  await expect(dialog.getByTestId("risk-wiz-review-portfolios")).toContainText(PORTFOLIO);
  await expect(dialog.getByTestId("risk-wiz-review-acceptance")).toContainText("Reject");
  await expectNoSeriousA11yInWizard(page, "Risk guided setup — step 4 (review)");

  // --- apply ------------------------------------------------------------------
  await dialog.getByTestId("wiz-apply").click();

  // On success the wizard closes and lands on the Risk → Acceptance tab.
  await expect(page.getByRole("dialog")).toHaveCount(0);
  const applied = page.locator('[aria-hidden="false"]:not([inert])').last();
  await expect(applied.getByTestId("risk-tab-acceptance")).toHaveAttribute("aria-pressed", "true");

  // The three persisted effects are read by CLICKING each tab, which mounts a FRESH
  // workspace (only the active tab's body mounts) → a fresh RPC read that reflects the
  // just-applied writes (the consolidated host pre-mounts inert alias panes at load, so
  // the initially-revealed tab can be stale — a tab click forces the re-read).

  // (b) createRiskBook — the new portfolio is now in the Portfolios roster.
  await applied.getByTestId("risk-tab-portfolios").click();
  await expect(applied.getByText(PORTFOLIO, { exact: false }).first()).toBeVisible();

  // (c) updateRiskRoutingGraph — the routing graph routes into that SAME minted book,
  //     proving the id threaded consistently from createRiskBook → routing.
  await applied.getByTestId("risk-tab-routing").click();
  await expect(applied.getByRole("heading", { name: "Risk Routing" })).toBeVisible();
  await expect(applied.getByText(PORTFOLIO, { exact: false }).first()).toBeVisible();

  // (a) updateAcceptanceGraph — re-mount the Acceptance tab; the persisted policy loaded
  //     with BOTH seeded rules (the reject-below-edge specific + the accept-all catch-all).
  await applied.getByTestId("risk-tab-acceptance").click();
  await expect(applied.getByRole("heading", { name: "Acceptance" })).toBeVisible();
  await expect(applied.getByTestId("acceptance-validation-status")).toContainText("2 rule");

  await page.screenshot({ path: "e2e-artifacts/risk-wizard-applied.png", fullPage: true });
});
