/**
 * W2 FI Streaming — OFFLINE mock-transport e2e (the `fidelity`/`?mock` posture, no
 * cargo/edge). Drives the {@link FiStreamingWorkspace} entirely against the in-app
 * mock source: it deep-links straight into the Fixed Income streaming desk, proves
 * the workspace is reachable ONLY under the Fixed Income domain (never under FX),
 * streams a preset line and asserts a LIVE par/PV price renders in the grid, then
 * exercises the RFS request form (instrument / tenor / size / side → "Request
 * price") and asserts the streamed indicative price surfaces for the requested
 * line. Closes with an axe a11y pass (0 serious/critical) on the streaming surface.
 *
 * Auth note: the `AuthGate` gates the Shell behind a mandatory sign-in even under
 * `?mock` (server-enforced sessions), but the MOCK `AuthService` seeds
 * `admin@celnet.com` / `password` and authenticates fully OFFLINE — so we sign in
 * through the mock login (NOT the live `openLive`/`signIn` helpers, which pin the
 * `?ws=` live edge and need the server). As admin the capabilities are permissive
 * and every class is licensed, so FI streaming is `streamable`.
 */
import { test, expect, type Page } from "@playwright/test";

import { expectNoSeriousA11y } from "./helpers";

/** Sign in through the OFFLINE mock login gate (seeded admin), then settle. */
async function signInMock(page: Page): Promise<void> {
  const heading = page.getByRole("heading", { name: "Sign in to Celnet" });
  await expect(heading).toBeVisible();
  await page.getByLabel("Email").fill("admin@celnet.com");
  await page.getByLabel("Password").fill("password");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(heading).toBeHidden();
}

/** The workspaces rail (the single class-parametric rail). */
function rail(page: Page) {
  return page.getByRole("complementary", { name: "workspaces" });
}
/** The FI-only "Streaming" rail button (title is `"Streaming (<chord>)"`). */
function streamingButton(page: Page) {
  return rail(page).locator('button[title^="Streaming ("]');
}
/** One product-domain tab by exact label. */
function domainTab(page: Page, label: string) {
  return page
    .getByRole("tablist", { name: "product domains" })
    .getByRole("tab", { name: label, exact: true });
}

test("FI Streaming: loads under FI, streams a live price, and serves an RFS indicative quote", async ({
  page,
}) => {
  // Deep-link straight into the Fixed Income streaming desk under the offline mock.
  await page.goto("/?mock&dom=fixed_income&view=fistreaming");
  await signInMock(page);

  // 1) The FI Streaming workspace loaded — its two regions are unique to it: the
  // live streaming-prices section + the RFS request panel.
  const streamRegion = page.getByRole("region", {
    name: "live fixed-income streaming prices",
  });
  await expect(streamRegion).toBeVisible();
  await expect(streamRegion.getByText("Fixed income — live streaming")).toBeVisible();
  await expect(
    page.getByRole("complementary", { name: "RFS — request a price" }),
  ).toBeVisible();

  // 2) Reachable ONLY under Fixed Income. The FI-only "Streaming" rail row is present
  // under the FI tab; switching to the FX Options domain HIDES it (while the distinct
  // FX "Stream" row stays), and it returns under Fixed Income.
  await expect(streamingButton(page)).toBeVisible();
  await domainTab(page, "FX Options").click();
  await expect(streamingButton(page)).toHaveCount(0);
  await expect(rail(page).locator('button[title^="Stream ("]')).toBeVisible();
  await domainTab(page, "Fixed Income").click();
  await expect(streamingButton(page)).toBeVisible();
  // The re-home moved off the FI-only view when FX was active; re-open Streaming.
  await streamingButton(page).click();
  await expect(streamRegion).toBeVisible();

  // 3) Stream a preset bond/swap line and assert a LIVE price (par %) renders in the
  // grid. `subscribeRates` emits a baseline snapshot synchronously, so the row and
  // its par/PV materialise on click (then re-price each tick).
  await streamRegion
    .getByRole("group", { name: "stream a preset line" })
    .getByRole("button", { name: "OIS 2Y rec" })
    .click();
  const grid = streamRegion.getByRole("table", { name: "fixed-income streaming lines" });
  await expect(grid).toBeVisible();
  const presetRow = grid.getByRole("row").filter({ hasText: "OIS 2Y rec" });
  await expect(presetRow).toBeVisible();
  // A live par (fair fixed) rate renders as "X.XXX%".
  await expect(presetRow.getByText(/\d+\.\d{3}%/)).toBeVisible();

  // 4) Fill the RFS request form — pick a SWAP instrument (IRS), a tenor, a size, and
  // a side — then "Request price" and assert the streamed indicative price appears
  // for the requested line.
  const rfs = page.getByRole("complementary", { name: "RFS — request a price" });
  await rfs
    .getByRole("group", { name: "instrument type" })
    .getByRole("button", { name: "Swap · IRS" })
    .click();
  await rfs.getByLabel("Tenor").selectOption("10");
  await rfs.getByLabel(/Size/).fill("75000000");
  await rfs
    .getByRole("group", { name: "swap side" })
    .getByRole("button", { name: "Pay" })
    .click();
  await rfs.getByRole("button", { name: "Request price" }).click();

  // The indicative readout (aria-live) surfaces the requested line labelled
  // "indicative", with a live par rate — a real streamed price, never fabricated.
  await expect(rfs.getByText("IRS 10Y pay")).toBeVisible();
  await expect(rfs.getByText("indicative", { exact: true })).toBeVisible();
  await expect(rfs.getByText(/\d+\.\d{3}%/)).toBeVisible();
  // The requested line is also a live row in the streaming grid.
  await expect(grid.getByRole("row").filter({ hasText: "IRS 10Y pay" })).toBeVisible();

  // 5) Accessibility — the populated FI Streaming surface has no serious/critical
  // a11y violations.
  await expectNoSeriousA11y(page, "FI Streaming workspace (mock)");
});
