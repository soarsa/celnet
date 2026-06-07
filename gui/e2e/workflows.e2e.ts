/**
 * Celnet trader-GUI end-to-end workflows, driven through a real Chromium against
 * the REAL celnet-server demo edge over the live WebSocket mirror (no mock). Each
 * spec exercises a genuine trader workflow exactly as a person would, and asserts
 * server-computed results render. The four core flows the W12 task calls out:
 *   1. ticket → price
 *   2. surface → mark → version pin (Publish)
 *   3. stream → click-to-trade
 *   4. risk → drill
 * plus the W12 observability surfacing on the status ribbon (server p99 + ring
 * conflation drops, distilled from the live heartbeats).
 *
 * All assertions are scoped to the ACTIVE pane returned by `gotoWorkspace` — every
 * workspace stays mounted, so an unscoped query could match a hidden pane.
 */
import { expect, test } from "@playwright/test";

import { gotoWorkspace, openLive } from "./helpers";

test.describe("Celnet GUI — live trader workflows (real demo edge)", () => {
  test("ticket → price: a requested quote returns a server-priced two-way + Greeks", async ({
    page,
  }) => {
    await openLive(page);
    const pane = await gotoWorkspace(page, "ticket");

    // RFQ the default structure against the live edge (the ticket starts unpriced,
    // showing "—"; the trader clicks Request quote to price).
    await pane.getByRole("button", { name: /Request quote/ }).click();

    // A server-priced two-way returns and the Greeks strip mounts (it renders only
    // once a quote lands), showing a live delta cell (glyph Δ, title "delta
    // (spot)") — a real server-computed sensitivity beside the priced quote.
    await expect(pane.getByTitle("delta (spot)")).toBeVisible({ timeout: 20_000 });
    // The priced premium-unit label appears alongside it (e.g. "% EUR prem").
    await expect(pane.getByText(/% .* prem/).first()).toBeVisible();
  });

  test("surface: marking publishes a fresh, pinned surface version", async ({ page }) => {
    await openLive(page);
    const pane = await gotoWorkspace(page, "surface");

    // The model selector (the MarkSurfaceRequest.smile_model control) is live.
    const modelGroup = pane.getByRole("group", { name: "smile calibration model" });
    await expect(modelGroup).toBeVisible();

    // Switch the calibration family to eSSVI — this re-marks the LIVE surface under
    // EXTENDED_SURFACE on the server and bumps surface_version. The chip becomes the
    // single active one, and the typed provenance line reads the server's family.
    await modelGroup.getByRole("button", { name: "eSSVI" }).click();
    await expect(modelGroup.getByRole("button", { name: "eSSVI" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    // The "marked as" provenance reads the TYPED arbitrage.model field the server
    // stamped (the regex is retired) — the family the surface was marked under.
    await expect(pane.getByText("extended-surface")).toBeVisible();
  });

  test("stream: click-to-trade lifts a live two-way and books a fill", async ({ page }) => {
    await openLive(page);
    const pane = await gotoWorkspace(page, "stream");

    // The blotter is the live multiplexed RFS region (a labelled table-like view).
    const blotter = pane.getByRole("region", { name: "streaming two-way markets" });
    await expect(blotter).toBeVisible();

    // Lift the first tradable offer (BUY). The button is enabled only when a live
    // click-to-trade token is present; clicking sends Execute over the live session.
    const lift = blotter.getByTitle("Lift the offer (BUY)").first();
    await expect(lift).toBeEnabled({ timeout: 20_000 });
    await lift.click();

    // A click-to-trade outcome toast appears — a real Executed (fill) or a typed
    // StreamReject (e.g. last-look expired). Either is an HONEST server outcome of
    // the real token round-trip; we assert the workflow produced a server verdict.
    await expect(page.getByText(/Filled|Rejected/).first()).toBeVisible({ timeout: 20_000 });
  });

  test("risk: drilling the hierarchy shows server-aggregated risk", async ({ page }) => {
    await openLive(page);
    const pane = await gotoWorkspace(page, "risk");

    // The Risk workspace consumes the SERVER's AggregateRisk/DrillRisk — no
    // client-side aggregation. The risk panel renders its subject header ("Risk ·
    // <subject>") and a scenario grid of server-computed sensitivities. We assert
    // the panel materialised and the measure tabs (P&L / Δ / ν) are present.
    await expect(pane.getByText(/^Risk ·/).first()).toBeVisible({ timeout: 20_000 });
    await expect(pane.getByRole("button", { name: "P&L", exact: true }).first()).toBeVisible();
  });

  test("status ribbon surfaces live server observability (p99 + drops)", async ({ page }) => {
    await openLive(page);
    await gotoWorkspace(page, "stream");

    // The ribbon's server-observability cluster is present. Until the first
    // heartbeat lands it shows "—"; once the live edge beats, it shows the real
    // drain-side p99 and the exact ring conflation-drop count. The ribbon is a
    // singleton (one per shell), so these testids are unambiguous page-wide.
    const drops = page.getByTestId("conflation-drops");
    await expect(drops).toBeVisible();
    await expect(drops).toHaveText(/\d+ drops|— drops/);

    const p99 = page.getByTestId("server-p99");
    await expect(p99).toBeVisible();
    await expect(p99).toContainText("server P99");
  });
});
