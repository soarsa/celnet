/**
 * LIVE e2e — the FI rates-curve build-by-instrument-reference mode against the
 * REAL demo edge under `CELNET_ACCESS_MODE=enforce`. fe-fi-migration #6 collapsed
 * the standalone Curve rail row into the single class-parametric Market Data
 * workspace: the curve is now the Fixed Income lens of Market Data. Logs in, opens
 * Market Data, flips to its Fixed Income asset-class lens (the curve builder),
 * confirms the slice-A pillar editor still renders, switches to the
 * instrument-reference mode, picks seeded reference-data registry instruments,
 * supplies a calibrating quote each, calls the server's `BuildCurve` over the WS
 * mirror, and asserts the returned calibrated discount curve renders. Closes with
 * an axe a11y pass (0 serious/critical) and a screenshot.
 *
 * The registry is server-seeded (config/reference_data.rs): USD SOFR deposit 3M,
 * OIS 2Y, IRS 5Y — three ascending-maturity, single-currency pillars the server
 * bootstraps into a discount curve.
 */
import { test, expect } from "@playwright/test";

import { openLive, gotoWorkspace, expectNoSeriousA11y } from "./helpers";

/** Seeded USD pillars to calibrate against, with a calibrating quote (percent). */
const PILLARS: readonly { id: string; quotePct: string }[] = [
  { id: "usd-sofr-depo-3m", quotePct: "4.31" },
  { id: "usd-sofr-ois-2y", quotePct: "4.05" },
  { id: "usd-sofr-irs-5y", quotePct: "4.12" },
];

test("Curve workspace builds a discount curve from registry instruments", async ({
  page,
}) => {
  await openLive(page);
  // The curve lives on the single class-parametric Market Data workspace; flip to
  // its Fixed Income asset-class lens to reach the rates-curve builder.
  const pane = await gotoWorkspace(page, "surface");
  await pane
    .getByRole("group", { name: "market data asset class" })
    .getByRole("button", { name: "Fixed Income" })
    .click();

  // Slice-A pillar editor is the default mode and still renders its ladder.
  await expect(
    pane.getByRole("tab", { name: "Pillar editor" }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(
    pane.getByRole("grid", { name: "curve pillar ladder" }),
  ).toBeVisible();

  // Switch to the build-by-instrument-reference mode.
  await pane.getByRole("tab", { name: "By instrument reference" }).click();
  await expect(
    pane.getByRole("tab", { name: "By instrument reference" }),
  ).toHaveAttribute("aria-selected", "true");

  // Pick each seeded registry instrument and enter its calibrating quote.
  const picker = pane.getByLabel("instrument to add");
  for (const pillar of PILLARS) {
    await picker.selectOption(pillar.id);
    await pane.getByRole("button", { name: "+ Add" }).click();
    await pane
      .getByLabel(`${pillar.id} calibrating quote in percent`)
      .fill(pillar.quotePct);
  }

  // Build the curve on the server and confirm the calibrated points render.
  await pane.getByRole("button", { name: "Build curve" }).click();
  const points = pane.getByRole("grid", { name: "calibrated curve points" });
  await expect(points).toBeVisible();
  // The server-bootstrapped points carry each calibrating instrument's name.
  await expect(points.getByText("USD SOFR Deposit 3M")).toBeVisible();
  await expect(points.getByText("USD SOFR IRS 5Y")).toBeVisible();
  await expect(pane.getByText("USD discount")).toBeVisible();
  await expect(pane.getByText(/3 pillars · ref/)).toBeVisible();

  // a11y pass on the populated build-by-instrument surface.
  await expectNoSeriousA11y(page, "curve workspace — build by instrument");

  // Screenshot the calibrated curve for the verification record.
  await page.screenshot({
    path: "test-results/curve-build-by-instrument.png",
    fullPage: true,
  });

  // No regression: switching back to the pillar editor still shows its ladder.
  await pane.getByRole("tab", { name: "Pillar editor" }).click();
  await expect(
    pane.getByRole("grid", { name: "curve pillar ladder" }),
  ).toBeVisible();
});
