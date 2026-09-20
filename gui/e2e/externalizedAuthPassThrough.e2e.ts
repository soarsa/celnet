/**
 * Externalized Authentication & Modular Studio Pass-Through Test Suite
 *
 * Verifies that:
 * 1. An externalized bearer token passed via URL parameter (?token=...) or host bridge
 *    completely bypasses the in-app login screen and establishes an authenticated session.
 * 2. DeskModal host container injection (window.deskmodal.auth.getToken) automatically
 *    logs in without prompting the operator for credentials.
 * 3. All modular studio views mount directly with their specific desk view.
 * 4. Runs 100% headlessly via Playwright without touching the user's desktop, mouse, or window focus.
 */
import { expect, test } from "@playwright/test";

test.describe("Externalized Authentication & Pass-Through (Headless)", () => {
  test("Host container token pass-through bypasses in-app login modal", async ({ page }) => {
    // Inject DeskModal host container API mock prior to document load
    await page.addInitScript(() => {
      (window as any).__DESKMODAL_AUTH_TOKEN__ = "test-host-bearer-token";
      (window as any).deskmodal = {
        version: "0.4.0",
        auth: {
          getToken: async () => "test-host-bearer-token",
          getIdentity: async () => ({
            sub: "dev@deskmodal.com",
            email: "dev@deskmodal.com",
            name: "DeskModal Administrator",
          }),
        },
      };
    });

    // Navigate to app with mock transport
    await page.goto("/?mock");

    // The login form must NOT be shown
    const loginHeading = page.getByRole("heading", { name: "Sign in to Celnet" });
    await expect(loginHeading).toBeHidden({ timeout: 5000 });

    // The shell rail must be visible
    await expect(page.getByRole("complementary", { name: "workspaces" })).toBeVisible({
      timeout: 10000,
    });
  });

  test("Modular Studio: Options Pricing (?view=studio_pricing) mounts directly with zero prompts", async ({ page }) => {
    await page.addInitScript(() => {
      (window as any).__DESKMODAL_AUTH_TOKEN__ = "test-host-bearer-token";
    });

    await page.goto("/?view=studio_pricing&mock");

    // Verify studio heading or desk identifier
    await expect(page.getByText("Pricing & Structuring").first()).toBeVisible({ timeout: 10000 });
    // Verify login form is completely absent
    await expect(page.getByRole("heading", { name: "Sign in to Celnet" })).toBeHidden();
  });

  test("Modular Studio: Markets & Depth (?view=studio_markets) mounts directly with zero prompts", async ({ page }) => {
    await page.addInitScript(() => {
      (window as any).__DESKMODAL_AUTH_TOKEN__ = "test-host-bearer-token";
    });

    await page.goto("/?view=studio_markets&mock");

    // Verify markets desk identifier
    await expect(page.getByText("Markets & Volatility").first()).toBeVisible({ timeout: 10000 });
    await expect(page.getByRole("heading", { name: "Sign in to Celnet" })).toBeHidden();
  });

  test("Modular Studio: Dealer RFQ (?view=studio_rfq) mounts directly with zero prompts", async ({ page }) => {
    await page.addInitScript(() => {
      (window as any).__DESKMODAL_AUTH_TOKEN__ = "test-host-bearer-token";
    });

    await page.goto("/?view=studio_rfq&mock");

    // Verify RFQ desk identifier (Desk 03 / Distribution & Quoting)
    await expect(page.getByText(/Distribution & Quoting|Desk 03|RFQ/).first()).toBeVisible({ timeout: 10000 });
    await expect(page.getByRole("heading", { name: "Sign in to Celnet" })).toBeHidden();
  });

  test("Modular Studio: Risk Cube (?view=studio_risk) mounts directly with zero prompts", async ({ page }) => {
    await page.addInitScript(() => {
      (window as any).__DESKMODAL_AUTH_TOKEN__ = "test-host-bearer-token";
    });

    await page.goto("/?view=studio_risk&mock");

    // Verify risk desk identifier
    await expect(page.getByText("Risk & Hedging Cockpit").first()).toBeVisible({ timeout: 10000 });
    await expect(page.getByRole("heading", { name: "Sign in to Celnet" })).toBeHidden();
  });

  test("Modular Studio: Deals Blotter (?view=studio_blotter) mounts directly with zero prompts", async ({ page }) => {
    await page.addInitScript(() => {
      (window as any).__DESKMODAL_AUTH_TOKEN__ = "test-host-bearer-token";
    });

    await page.goto("/?view=studio_blotter&mock");

    // Verify blotters desk identifier
    await expect(page.getByText("Blotters & Position Ledger").first()).toBeVisible({ timeout: 10000 });
    await expect(page.getByRole("heading", { name: "Sign in to Celnet" })).toBeHidden();
  });
});
