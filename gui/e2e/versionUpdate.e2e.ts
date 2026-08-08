/**
 * Deploy auto-refresh — OFFLINE e2e over the production bundle (`?mock` transport:
 * NO cargo, NO celnet-server). Proves the end-to-end story the feature exists for:
 * when the deploy publishes a newer `/version.json` (keyed on `buildTime`, since the
 * packaged `hash` is a constant `v0.0.0`), the running page detects it, CLEARS its
 * client caches, and performs a cache-busting reload onto the fresh bundle — exactly
 * ONCE — and can never spin into a reload loop.
 *
 * The reload here is a REAL browser navigation. We persist a load counter + a
 * cache-delete spy in localStorage (an `addInitScript` re-runs on every load, so the
 * records survive the reload) and route `/version.json` to a far-future build. The
 * loop-guard (a sessionStorage key that survives the reload) is what stops the
 * second, third, … reload even though the served build still reads "newer" than the
 * freshly-loaded bundle.
 *
 * Also asserts the negative: when `/version.json` matches the running build, NO
 * banner and NO reload — and axe (0 serious/critical) on the update banner.
 */
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { signIn } from "./helpers";

const FUTURE_BUILD = "2099-01-01T00:00:00.000Z";
const GUARD_KEY = "celnet:reloaded-for-build";

/**
 * Re-run on EVERY document load: bump a load counter, and wrap `caches.delete` so we
 * can prove the cache-clear step ran before the reload. Both records live in
 * localStorage so they survive the reload the feature triggers.
 */
async function installReloadProbe(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const loads = Number(localStorage.getItem("e2e-loads") ?? "0") + 1;
    localStorage.setItem("e2e-loads", String(loads));
    try {
      // `caches` is a browser global; the e2e tsconfig ships no DOM lib, so reach it
      // through a locally-typed globalThis view (not `any`).
      const cacheApi = (
        globalThis as unknown as { caches: { delete(k: string): Promise<boolean> } }
      ).caches;
      const real = cacheApi.delete.bind(cacheApi);
      cacheApi.delete = async (key: string): Promise<boolean> => {
        const seen = JSON.parse(localStorage.getItem("e2e-cache-deletes") ?? "[]");
        seen.push(key);
        localStorage.setItem("e2e-cache-deletes", JSON.stringify(seen));
        return real(key);
      };
    } catch {
      /* CacheStorage unavailable in this context — the spy is best-effort */
    }
  });
}

/** Route `/version.json` to a strictly-newer build (far-future buildTime). */
async function serveNewerVersion(page: Page): Promise<void> {
  await page.route("**/version.json", (route) =>
    route.fulfill({
      contentType: "application/json",
      headers: { "cache-control": "no-store" },
      body: JSON.stringify({ hash: "v0.0.0", buildTime: FUTURE_BUILD }),
    }),
  );
}

test("a newer deploy triggers a cache-busting reload — exactly once, never looping", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await installReloadProbe(page);
  await serveNewerVersion(page);

  await page.goto("/?mock");
  await signIn(page);

  // Seed a Cache Storage entry so the clear step has something concrete to delete
  // (and thus our spy records a real deletion) when the reload path runs.
  await page.evaluate(() =>
    (globalThis as unknown as { caches: { open(n: string): Promise<unknown> } }).caches.open(
      "e2e-seed-cache",
    ),
  );

  // The update banner announces the freshly-deployed build.
  const banner = page.getByTestId("update-banner");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("Updating to the latest version");

  // axe on the banner surface (freeze animations first): 0 serious/critical.
  await page.addStyleTag({
    content: "*,*::before,*::after{animation:none!important;transition:none!important;}",
  });
  const axe = await new AxeBuilder({ page })
    .include('[data-testid="update-banner"]')
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  const serious = axe.violations.filter(
    (v) => v.impact === "serious" || v.impact === "critical",
  );
  expect(serious, JSON.stringify(serious.map((v) => v.id))).toEqual([]);

  // Trigger the reload deterministically (the 5s countdown would fire it anyway).
  await Promise.all([
    page.waitForFunction(() => localStorage.getItem("e2e-loads") === "2"),
    page.getByRole("button", { name: "Reload now" }).click(),
  ]);

  // The reload actually happened — and cleared caches BEFORE navigating.
  expect(await page.evaluate(() => localStorage.getItem("e2e-loads"))).toBe("2");
  const deletes = await page.evaluate(() =>
    JSON.parse(localStorage.getItem("e2e-cache-deletes") ?? "[]"),
  );
  expect(deletes).toContain("e2e-seed-cache");

  // The loop-guard recorded the target build; it survives the reload.
  expect(await page.evaluate((k) => sessionStorage.getItem(k), GUARD_KEY)).toBe(FUTURE_BUILD);

  // After the reload the served build STILL reads "newer" than the (unchanged) baked
  // bundle, so the banner re-appears — but the guard blocks any further reload. Sign
  // in again (post-reload session) and prove no second navigation occurs even past a
  // full countdown and a manual click.
  await signIn(page);
  const banner2 = page.getByTestId("update-banner");
  await expect(banner2).toBeVisible();
  await banner2.getByRole("button", { name: "Reload now" }).click();
  await page.waitForTimeout(6000); // longer than the 5s auto-reload countdown
  expect(await page.evaluate(() => localStorage.getItem("e2e-loads"))).toBe("2"); // still ONE reload
});

test("a matching deploy version shows no banner and never reloads", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  await installReloadProbe(page);
  // No /version.json route: the real preview server returns the ACTUAL baked build,
  // which equals the running bundle — so isNewerRelease is false.

  await page.goto("/?mock");
  await signIn(page);
  await expect(page.getByRole("complementary", { name: "workspaces" })).toBeVisible();

  // Give the mount + interval polls a beat, then assert: no banner, no reload.
  await page.waitForTimeout(1500);
  await expect(page.getByTestId("update-banner")).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem("e2e-loads"))).toBe("1");
});
