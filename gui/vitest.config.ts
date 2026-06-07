import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

/**
 * Unit/component test config for the trader GUI, kept SEPARATE from the Vite app
 * build (`vite.config.ts`) so the production bundle config is untouched.
 *
 * We deliberately reuse the project's own `@vitejs/plugin-react` and pull
 * `defineConfig` from `vitest/config` (NOT `vite`), so vitest resolves a single
 * Vite instance — the project's vite 6 — and there is no "two copies of vite /
 * Plugin type mismatch" at the `plugins` boundary. vitest 2.1.x declares a
 * `vite ^5 || ^6` peer, so it pairs cleanly with the installed vite 6.
 *
 * Tests run under jsdom (a real `document`/`window` for the React-hook + RTL
 * component tests) and exercise the REAL modules through their public surface —
 * no server, no WebGPU, no mocks of our own functionality. GPU/WebSocket-bound
 * modules are intentionally NOT tested here; they belong to the Wave-2 Playwright
 * e2e harness.
 *
 * The two build-time `define` constants Vite injects into the bundle
 * (`__CELNET_BUILD_HASH__` / `__CELNET_BUILD_TIME__`, see vite.config.ts) are
 * mirrored here with deterministic test values so component tests that render the
 * full Shell (whose StatusRibbon reads them) resolve them — they are otherwise
 * undefined under vitest, which only matters once a test mounts the whole shell.
 */
export default defineConfig({
  plugins: [react()],
  define: {
    __CELNET_BUILD_HASH__: JSON.stringify("test"),
    __CELNET_BUILD_TIME__: JSON.stringify("1970-01-01T00:00:00.000Z"),
  },
  test: {
    include: ["test/**/*.test.{ts,tsx}"],
    environment: "jsdom",
    globals: true,
    setupFiles: ["./test/setup.ts"],
  },
});
