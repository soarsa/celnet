/**
 * Cross-asset STREAMED-edge e2e (item A — carry seam to the streamed edge,
 * `docs/plan/CARRY-SEAM-TO-EDGE.md`, phase P3) driven through a real Chromium
 * against the REAL `celnet-server` demo edge under the PRODUCTION Enforce posture
 * (`gui/e2e/demoEdge.ts` boots it with `CELNET_ACCESS_MODE=enforce`).
 *
 * This is the binding live proof that the carry seam reaches the STREAMED edge of
 * the GUI: a non-FX (equity) line is streamed end-to-end and renders the
 * asset-class-correct CARRY rho — for an equity the dividend-yield rho — sourced
 * from the server-computed, carry-tagged streamed `Greeks` (the streamed `Update`
 * carries `RateSensitivities::Carry`; the GUI's flat `rho_for` projection is the
 * exact carry rho, which `GreeksStrip` relabels by the row's asset class).
 *
 * The flow is a REAL trader workflow, not a transport poke:
 *   1. structure the gallery's "Cross-asset vanilla" card (an equity AAPL/USD
 *      vanilla — the registry default; its strike defaults to the ATM forward so
 *      the streamed line is non-degenerate at the demo edge's market scale),
 *   2. click "Stream this ≋" to promote that exact `Instrument` into the blotter,
 *   3. open the streamed AAPL/USD line's full Greeks strip in the blotter,
 *   4. assert the equity carry-rho cell ("rho (dividend yield)") renders a finite,
 *      non-zero value — a real server-computed sensitivity, never a fabricated 0.
 *
 * Honesty (GUIDE.md rule 2): a finite, non-zero value is required. A 0 would mean
 * the streamed line priced as worthless (a market-scale/seed failure) and fails the
 * gate — exactly as the variance-swap e2e refuses a 0.000000 fair strike.
 */
import { expect, test, type Locator } from "@playwright/test";

import { expectNoSeriousA11y, gotoWorkspace, openLive } from "./helpers";

/** The gallery option whose label span is exactly `label` (the cross-asset card). */
function galleryCard(pane: Locator, label: string): Locator {
  return pane.getByRole("option").filter({ hasText: label });
}

/** Parse the GreeksStrip cell value text (a signed decimal) into a finite number. */
function cellNumber(text: string): number {
  // The cell renders "<glyph> <signed-number>"; the trailing token is the value.
  const m = text.trim().match(/-?\d+(?:\.\d+)?$/);
  expect(m, `Greeks cell value parses from "${text}"`).not.toBeNull();
  return Number(m![0]);
}

test.describe("Celnet GUI — cross-asset streamed carry rho (live demo edge, Enforce)", () => {
  test("equity line streams end-to-end and renders the dividend-yield carry rho", async ({
    page,
  }) => {
    await openLive(page);
    const ticket = await gotoWorkspace(page, "ticket");

    // Structure the cross-asset (equity) vanilla via the live gallery. The card's
    // registry default is an AAPL/USD equity CALL; its strike defaults to the ATM
    // forward, so the promoted line prices to a real two-way on the demo edge.
    const card = galleryCard(ticket, "Cross-asset vanilla");
    await card.click();
    await expect(card).toHaveAttribute("aria-selected", "true");

    // Promote the EXACT structured instrument into the streaming blotter. "Stream
    // this" carries the full `Instrument` (its `underlying` = the equity arm) over
    // the live StreamSession and jumps to the Stream workspace.
    await ticket.getByRole("button", { name: /Stream this/ }).click();
    const stream = await gotoWorkspace(page, "stream");

    // The streamed equity line lands in the blotter as an AAPL/USD row (the FX
    // pair projection of the equity underlying). It groups under an "AAPL/USD"
    // group header; the data row's pair cell reads AAPL/USD.
    const blotter = stream.getByRole("region", { name: "streaming two-way markets" });
    await expect(blotter).toBeVisible();
    // Wait for the server baseline snapshot to materialise the line (the pair cell).
    await expect(blotter.getByText("AAPL/USD").first()).toBeVisible({ timeout: 20_000 });

    // Open the line's full, asset-class-correct Greeks strip: click the row's
    // structure cell ("Cross-asset vanilla — show full Greeks"), then expand the
    // strip to reveal the rate-rho (secondary) Greeks.
    await blotter.getByRole("button", { name: /Cross-asset vanilla/ }).first().click();
    const detail = stream.getByRole("region", { name: /full Greeks for/ });
    await expect(detail).toBeVisible();
    await detail.getByRole("button", { name: "toggle full Greeks" }).click();

    // The equity carry-rho cell: GreeksStrip relabels the rate-rho Greeks by class,
    // so an equity line shows "rho (dividend yield)" (NOT "rho foreign"). Its value
    // is the server-computed carry rho — assert it is finite AND non-zero.
    const divRho = detail.getByTitle("rho (dividend yield)");
    await expect(divRho).toBeVisible();
    const value = cellNumber((await divRho.textContent()) ?? "");
    expect(Number.isFinite(value), "dividend-yield rho is finite").toBe(true);
    expect(Math.abs(value), "dividend-yield rho is a real non-zero sensitivity").toBeGreaterThan(0);

    // The FX two-rate label must NOT appear for an equity line (the carry seam, not
    // the FX arm): no "rho foreign" cell is rendered in this strip.
    await expect(detail.getByTitle("rho foreign")).toHaveCount(0);

    // a11y: the streamed cross-asset detail surface is clean (serious/critical = 0).
    await expectNoSeriousA11y(page, "stream cross-asset Greeks detail");
  });
});
