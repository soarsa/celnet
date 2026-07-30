/**
 * FI Aggregated Book (ADR-0022) — end-to-end over the production bundle.
 *
 * OFFLINE flow (this spec, `?mock` transport — NO cargo, NO celnet-server): the
 * seeded admin defines an aggregated book with ≥2 LP-SIM members in
 * Administration → Aggregation, then the SAME session opens the FI "Agg Book"
 * price view, selects the new book, and the live composite grid populates (the
 * mock consolidates synthetic LP-SIM two-ways over the SAME `StreamSession` seam
 * the live WS transport implements). This drives the whole GUI half of the
 * contract — the CRUD wsCodec, the admin panel, the subscribe/snapshot/update
 * codec, and the price grid — without a server.
 *
 * LIVE flow (documented below, run by the parent once the cargo lane is free) —
 * the true end-to-end against celnet-server + the celnet-lp-sim fleet:
 *
 *   TODO(live-e2e): with the cargo lane free, stand the real stack up and re-run
 *   the same two user gestures against the WS transport instead of `?mock`:
 *     1. Build + launch the server:
 *          source "$HOME/.cargo/env" && cargo run -p celnet-server
 *        (serves the WS mirror the GUI's WsTransport speaks).
 *     2. Launch the LP-SIM Treasury fleet feeding the LiquidityFeed ingest:
 *          source "$HOME/.cargo/env" && \
 *            cargo run -p celnet-lp-sim --bin lp-sim -- \
 *            --server 127.0.0.1:<grpc-port> --members LP-SIM-01,LP-SIM-02,LP-SIM-03,LP-SIM-04
 *        (see deploy/start-lp-sim.sh / docs/EXCEL-ADDIN-LOCAL-BRINGUP.md for the
 *         canonical invocation + the bundled treasury-universe.json identities).
 *     3. Point the GUI at the live WS mirror (drop `&mock`; the transport resolves
 *        to WsTransport per gui/src/data/transportConfig.ts), sign in as the real
 *        admin, and create a book whose members are LP-SIM-01..LP-SIM-04 with
 *        scope ALL_MEMBERS_QUOTE.
 *     4. Open the Agg Book view, select the book, and assert the composite rows
 *        populate with the server's REAL consolidated best bid/offer + per-LP
 *        contributions (the CUSIP/ISIN identities resolved from the server's
 *        reference-data registry), and that a member which stops quoting flips to
 *        `stale` and drops out of the best price.
 *   The GUI code path is transport-agnostic (the mock and WsTransport implement the
 *   identical `StreamSession` + CRUD seams), so a green offline run here means the
 *   live run exercises the same components against real frames.
 */
import { expect, test } from "@playwright/test";

import { gotoMockView, installFrozenClock } from "./fidelityHelpers";
import { expectNoSeriousA11y, signIn } from "./helpers";

/** Select a top-level product-domain tab (FX Options / Fixed Income / Administration). */
async function selectDomain(page: import("@playwright/test").Page, name: string): Promise<void> {
  await page
    .getByRole("tablist", { name: "product domains" })
    .getByRole("tab", { name })
    .click();
}

/** Open a workspace by its rail button label (title is `"<label> (<chord>)"`). */
async function openWorkspace(page: import("@playwright/test").Page, label: string): Promise<void> {
  await page
    .getByRole("complementary", { name: "workspaces" })
    .locator(`button[title^="${label} ("]`)
    .click();
}

const BOOK_NAME = "E2E Treasuries";

test("admin defines an aggregated book with LP-SIM members; it appears + populates in the price view", async ({
  page,
}) => {
  await installFrozenClock(page);
  await page.setViewportSize({ width: 1440, height: 900 });

  // Boot the mock + sign in as the seeded admin, deep-linked into the Admin view.
  await gotoMockView(page, "admin");
  await selectDomain(page, "Administration");
  await openWorkspace(page, "Admin");

  // 1) Open the Aggregation tab (the 5th Administration section).
  await page.getByRole("tab", { name: "Aggregation" }).click();
  await expect(page.getByRole("tab", { name: "Aggregation" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(page.getByRole("heading", { name: "Define a book" })).toBeVisible();

  // 2) Define a book: a name + two LP-SIM members (the quick-add candidates).
  await page.getByLabel("aggregated book name").fill(BOOK_NAME);
  await page.getByRole("button", { name: "+ LP-SIM-01" }).click();
  await page.getByRole("button", { name: "+ LP-SIM-02" }).click();
  // The two members now show as removable chips (≥2 members satisfies the quorum).
  await expect(page.getByRole("list", { name: "selected members" })).toContainText("LP-SIM-01");
  await expect(page.getByRole("list", { name: "selected members" })).toContainText("LP-SIM-02");

  // Create it — the roster refetches and the new book appears (enabled by default).
  await page.getByRole("button", { name: "Create book" }).click();
  const rosterRow = page
    .getByRole("row")
    .filter({ has: page.getByText(BOOK_NAME, { exact: true }) });
  await expect(rosterRow).toBeVisible();
  await expect(rosterRow.getByText("Enabled")).toBeVisible();

  // 3) Cross to the FI "Agg Book" price view IN THE SAME SESSION (client-side nav,
  //    no reload — so the mock transport's freshly-created book persists).
  await selectDomain(page, "Fixed Income");
  await openWorkspace(page, "Agg Book");
  await expect(page.getByText("Aggregated book · live composite")).toBeVisible();

  // The newly-created book is offered in the selector; select it.
  const bookBtn = page
    .getByRole("group", { name: "select an aggregated book" })
    .getByRole("button", { name: BOOK_NAME });
  await expect(bookBtn).toBeVisible();
  await bookBtn.click();

  // 4) The compacted composite grid populates — one price TILE per security. The
  //    mock emits the baseline snapshot on subscribe; flush the rAF commit + a few
  //    ticks so tiles render.
  await page.clock.runFor(600);
  const tiles = page.getByLabel("aggregated composite tiles");
  await expect(tiles).toBeVisible();
  const firstTile = tiles.getByRole("article").first();
  await expect(firstTile).toBeVisible();

  // The tile FACE is compact: the security name stays, but the static terms + the
  // ISIN/CUSIP line have MOVED off it (now behind the Details affordance).
  await expect(firstTile.getByText(/^ISIN /)).toHaveCount(0);
  await expect(firstTile.getByText("Issuer")).toHaveCount(0);
  await expect(firstTile.getByText("Day count")).toHaveCount(0);
  const detailsBtn = firstTile.getByRole("button", { name: /^Security details for/ });
  await expect(detailsBtn).toBeVisible();

  // 5) The Details popover opens a labelled dialog carrying the full static terms;
  //    Escape closes it.
  await detailsBtn.click();
  const detailsDialog = page.getByRole("dialog");
  await expect(detailsDialog).toBeVisible();
  await expect(detailsDialog.getByText("Issuer")).toBeVisible();
  await expect(detailsDialog.getByText("Maturity")).toBeVisible();
  await expect(detailsDialog.getByText("Day count")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // 6) The per-LP breakdown still opens from the compact footer.
  await firstTile.getByRole("button", { name: "Per-LP" }).click();
  await expect(firstTile.getByText("LP-SIM-01", { exact: false }).first()).toBeVisible();

  // 7) A11y pass over the compact price view WITH the Details popover open (no
  //    serious/critical violations — focus order, contrast, target size, aria).
  await detailsBtn.click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expectNoSeriousA11y(page, "aggregated-book compact view + details popover");
  await page.keyboard.press("Escape");

  // 8) The per-user security-selection preference filters the grid + PERSISTS
  //    across a full reload (a localStorage-backed client preference).
  const tileCountBefore = await tiles.getByRole("article").count();
  await page.getByRole("button", { name: /Securities/ }).click();
  const picker = page.getByRole("dialog", { name: /choose which securities/i });
  await expect(picker).toBeVisible();
  await picker.getByRole("checkbox").first().check();
  await page.keyboard.press("Escape");
  // The grid now shows a strict subset (the ticked security only).
  await expect
    .poll(async () => await tiles.getByRole("article").count())
    .toBeLessThan(tileCountBefore);
  await expect(page.getByRole("button", { name: /Securities/ })).toContainText("selected");

  await page.reload();
  await signIn(page);
  await page.clock.runFor(200);
  // The choice survived the reload.
  await expect(page.getByRole("button", { name: /Securities/ })).toContainText("selected");
});
