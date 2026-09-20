/**
 * microAppsMultiInstance.e2e.ts — E2E validation of CelNet Sovereign Micro-Apps
 * within the DeskModal desktop container ecosystem.
 *
 * Validates:
 * 1. Micro-App Mode: Clean canvas with MicroAppHeader; monolithic shell navigation (rail, domain tabs) stripped.
 * 2. Multi-Instance Support: Parameterized instances with distinct symbols and instance IDs.
 * 3. FDC3 Channel Linking: User channel selection (Red, Blue, Green, etc.).
 * 4. Inbound FDC3 Context Sync: Window reacts to external instrument broadcasts without page reload.
 * 5. FDC3 Intent Resolution: ViewInstrument / ViewAnalysis routes to appropriate desks.
 */

import { test, expect } from "@playwright/test";

test.describe("CelNet Sovereign Micro-Apps & Multi-Instance Architecture", () => {
  test("Micro-App Mode strips monolithic shell and mounts info-dense MicroAppHeader", async ({
    page,
  }) => {
    // Inject mock DeskModal bearer token
    await page.addInitScript(() => {
      (window as any).__DESKMODAL_AUTH_TOKEN__ = "test-host-bearer-token";
    });

    await page.goto("/?mode=app&view=studio_pricing&mock");

    // 1. Verify MicroAppHeader is visible
    const header = page.getByRole("banner", { name: "Micro-App Toolbar" });
    await expect(header).toBeVisible({ timeout: 10000 });

    // 2. Verify desk identity and underlier
    await expect(header.getByText("Desk 02")).toBeVisible();
    await expect(header.getByText(/Pricing/i).first()).toBeVisible();
    await expect(header.getByText("EUR/USD")).toBeVisible();

    // 3. Verify monolithic shell chrome is stripped
    await expect(page.locator("aside[aria-label='workspaces']")).toBeHidden();
    await expect(page.locator("div[role='tablist'][aria-label='product domains']")).toBeHidden();
    await expect(page.getByRole("heading", { name: "Sign in to Celnet" })).toBeHidden();
  });

  test("Multi-Instance Isolation: separate instances load with distinct symbols and instance IDs", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      (window as any).__DESKMODAL_AUTH_TOKEN__ = "test-host-bearer-token";
    });

    // Window 1: Options Pricing on EUR/USD, Instance #win-1
    await page.goto("/?mode=app&view=studio_pricing&sym=EURUSD&instanceId=win-1&mock");
    const header1 = page.getByRole("banner", { name: "Micro-App Toolbar" });
    await expect(header1.getByText("#win-1")).toBeVisible({ timeout: 10000 });
    await expect(header1.getByText("EUR/USD")).toBeVisible();

    // Window 2: Markets Depth on USD/JPY, Instance #win-2
    await page.goto("/?mode=app&view=studio_markets&sym=USDJPY&instanceId=win-2&mock");
    const header2 = page.getByRole("banner", { name: "Micro-App Toolbar" });
    await expect(header2.getByText("#win-2")).toBeVisible({ timeout: 10000 });
    await expect(header2.getByText("USD/JPY")).toBeVisible();
    await expect(header2.getByText("Desk 01")).toBeVisible();
  });

  test("FDC3 Channel Selector joins channels and reflects color badge", async ({ page }) => {
    await page.addInitScript(() => {
      (window as any).__DESKMODAL_AUTH_TOKEN__ = "test-host-bearer-token";
    });

    await page.goto("/?mode=app&view=studio_pricing&channel=red&mock");

    const header = page.getByRole("banner", { name: "Micro-App Toolbar" });
    const channelBtn = header.getByRole("button", { name: /FDC3 Channel:/i });
    await expect(channelBtn).toBeVisible({ timeout: 10000 });

    // Open channel dropdown
    await channelBtn.click();
    const dropdown = page.getByRole("menu");
    await expect(dropdown).toBeVisible();

    // Switch to Blue channel
    await page.getByRole("menuitem", { name: "Blue" }).click();
    await expect(channelBtn).toHaveText(/Blue/i);
  });

  test("Inbound FDC3 context broadcast dynamically updates active underlier", async ({ page }) => {
    await page.addInitScript(() => {
      (window as any).__DESKMODAL_AUTH_TOKEN__ = "test-host-bearer-token";
    });

    await page.goto("/?mode=app&view=studio_pricing&mock");

    const header = page.getByRole("banner", { name: "Micro-App Toolbar" });
    await expect(header.getByText("EUR/USD")).toBeVisible({ timeout: 10000 });

    // Simulate inbound FDC3 broadcast from DeskModal Watchlist or Chart
    await page.evaluate(() => {
      const fdc3 = (window as any).fdc3;
      if (fdc3 && fdc3.broadcast) {
        fdc3.broadcast({
          type: "fdc3.instrument",
          name: "USD/JPY Spot",
          id: { ticker: "USD/JPY" },
        });
      }
    });

    // Verify underlier dynamically updated to USD/JPY
    await expect(header.getByText("USD/JPY")).toBeVisible({ timeout: 10000 });
  });

  test("Inbound FDC3 intent routing switches to target desk", async ({ page }) => {
    await page.addInitScript(() => {
      (window as any).__DESKMODAL_AUTH_TOKEN__ = "test-host-bearer-token";
    });

    await page.goto("/?mode=app&view=studio_pricing&mock");

    const header = page.getByRole("banner", { name: "Micro-App Toolbar" });
    await expect(header.getByText("Desk 02")).toBeVisible({ timeout: 10000 });

    // Raise ViewInstrument intent (targeting Market Depth)
    await page.evaluate(() => {
      const fdc3 = (window as any).fdc3;
      if (fdc3 && fdc3.raiseIntent) {
        fdc3.raiseIntent("ViewInstrument", {
          type: "fdc3.instrument",
          id: { ticker: "GBP/USD" },
        });
      }
    });

    // Verify app seamlessly switched to Desk 01 (Markets) with GBP/USD
    await expect(header.getByText("Desk 01")).toBeVisible({ timeout: 10000 });
    await expect(header.getByText("GBP/USD")).toBeVisible({ timeout: 10000 });
  });
});
