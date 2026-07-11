/**
 * Playwright e2e + axe a11y config for the Celnet trader GUI.
 *
 * TWO project topologies share ONE `vite preview` webServer (the production bundle):
 *
 *   • `chromium` (default e2e) — drives the REAL product end-to-end against a REAL
 *     `celnet-server` demo edge (booted in `globalSetup`), the live WebSocket mirror
 *     of the single `celnet.wire` contract. Every price/Greek/surface is server-
 *     computed. NEEDS cargo (the edge boot). Specs navigate with `?ws=<edge>`.
 *
 *   • `fidelity` (offline visual-fidelity gate) — the mockup-parity lane
 *     (`gui-visual-regression-gate`). Runs entirely against the in-app `?mock`
 *     transport (src/data/mockSource.ts): NO server, NO cargo, NO demo edge. It
 *     hosts (1) the per-workspace visual-regression baselines
 *     (`e2e/visualRegression.e2e.ts` → `toHaveScreenshot`, the layout/IA DRIFT gate)
 *     and (2) the mockup-vs-live structural fidelity report
 *     (`e2e/mockupFidelity.e2e.ts`). Because it is offline, `globalSetup` SKIPS the
 *     demo-edge boot for a fidelity-only run (see `e2e/fidelityRun.ts` +
 *     `e2e/global-setup.ts`), so `--project=fidelity` spawns no cargo.
 *
 * ─── HOW T2 RUNS THE FIDELITY GATE (no cargo) ───────────────────────────────
 *   npm --prefix gui run fidelity            # gate: build → preview → compare baselines + report
 *   npm --prefix gui run fidelity:update     # regenerate baselines after an intentional UI change
 * Both are just `CELNET_FIDELITY=1 playwright test --project=fidelity [--update-snapshots]`.
 * The `CELNET_FIDELITY=1` env (and/or selecting only `--project=fidelity`) makes
 * `globalSetup` skip the edge, so the fidelity gate is a pure front-end gate that a
 * gui-only T2 window can run without the Rust toolchain. The committed baselines
 * live under `e2e/visualRegression.e2e.ts-snapshots/`; the fidelity report is written
 * to the gitignored `test-results/fidelity/`.
 *
 * Environment note: Playwright's Chromium binary must be present (installed via
 * `npx playwright install chromium`). Where the binary cannot be fetched, this
 * config + the specs are committed and the browser run is environment-gated —
 * they run unchanged wherever Chromium exists. The vitest + jsdom suites cover
 * the component/codec layer regardless.
 */
import { defineConfig, devices } from "@playwright/test";

const PREVIEW_PORT = 4317;

/**
 * The specs that belong to the OFFLINE `fidelity` project (the mock/visual-fidelity
 * gate). Declared once so the `chromium` project can ignore them and the `fidelity`
 * project can match exactly them — the two topologies never run each other's specs.
 */
const FIDELITY_SPECS = [
  /visualRegression\.e2e\.ts/,
  /mockupFidelity\.e2e\.ts/,
  // Admin section-tabs interaction test — offline (mock transport), no cargo edge.
  /adminSectionTabs\.e2e\.ts/,
];

/**
 * A fixed desktop viewport for the fidelity project so the visual baselines are
 * reproducible frame-to-frame (a stable canvas is the precondition for a drift gate).
 */
const FIDELITY_VIEWPORT = { width: 1440, height: 900 };

export default defineConfig({
  testDir: "./e2e",
  testMatch: /.*\.e2e\.ts/,
  globalSetup: "./e2e/global-setup.ts",
  globalTeardown: "./e2e/global-teardown.ts",
  // The edge boot + a real browser RFS round-trip need headroom; keep it bounded.
  timeout: 60_000,
  expect: {
    timeout: 15_000,
    // Visual-regression defaults for the fidelity project: freeze CSS animations and
    // hide the text caret so a screenshot samples the RESTING UI, and tolerate a hair
    // of sub-pixel/AA noise while still catching layout/IA drift (which shifts LARGE
    // regions ≫ this ratio). The mock tape is additionally clock-frozen per spec, so
    // the intended residual here is anti-aliasing only.
    toHaveScreenshot: {
      animations: "disabled",
      caret: "hide",
      maxDiffPixelRatio: 0.01,
    },
  },
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: `http://127.0.0.1:${PREVIEW_PORT}`,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [
    {
      // Live demo-edge e2e (needs cargo). Never runs the offline fidelity specs.
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
      testIgnore: FIDELITY_SPECS,
    },
    {
      // Offline visual-fidelity gate (mock transport; no server, no cargo).
      name: "fidelity",
      testMatch: FIDELITY_SPECS,
      use: { ...devices["Desktop Chrome"], viewport: FIDELITY_VIEWPORT },
    },
  ],
  webServer: {
    // Serve the built app (both projects exercise the production bundle, not dev
    // HMR). This is pure front-end (Vite) — it spawns NO cargo, so the offline
    // fidelity project reuses it as-is.
    command: `npm run build && npx vite preview --host 127.0.0.1 --port ${PREVIEW_PORT} --strictPort`,
    url: `http://127.0.0.1:${PREVIEW_PORT}`,
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});
