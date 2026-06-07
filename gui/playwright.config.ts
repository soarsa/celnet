/**
 * Playwright e2e + axe a11y config for the Celnet trader GUI.
 *
 * The specs drive the REAL product end-to-end through a real Chromium against a
 * REAL `celnet-server` demo edge (booted in `globalSetup`) — the live WebSocket
 * mirror of the single `celnet.wire` contract, NOT the offline mock. Every price,
 * Greek, surface and scenario the browser renders is server-computed.
 *
 * Topology:
 *   - `globalSetup`  boots the demo edge, captures its `ws://…` URL, writes it to
 *     a temp file (`.e2e-ws-url`) so specs can pin the live transport.
 *   - `webServer`    serves the production `vite build` via `vite preview`.
 *   - each spec navigates with `?ws=<edge>` so the live transport dials the edge.
 *
 * Environment note: Playwright's Chromium binary must be present (installed via
 * `npx playwright install chromium`). Where the binary cannot be fetched, this
 * config + the specs are committed and the browser run is environment-gated —
 * they run unchanged wherever Chromium exists. The vitest + jsdom suites cover
 * the component/codec layer regardless.
 */
import { defineConfig, devices } from "@playwright/test";

const PREVIEW_PORT = 4317;

export default defineConfig({
  testDir: "./e2e",
  testMatch: /.*\.e2e\.ts/,
  globalSetup: "./e2e/global-setup.ts",
  globalTeardown: "./e2e/global-teardown.ts",
  // The edge boot + a real browser RFS round-trip need headroom; keep it bounded.
  timeout: 60_000,
  expect: { timeout: 15_000 },
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
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    // Serve the built app (the e2e exercises the production bundle, not dev HMR).
    command: `npm run build && npx vite preview --host 127.0.0.1 --port ${PREVIEW_PORT} --strictPort`,
    url: `http://127.0.0.1:${PREVIEW_PORT}`,
    reuseExistingServer: !process.env.CI,
    timeout: 180_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});
