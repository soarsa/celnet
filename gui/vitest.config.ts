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
 */
export default defineConfig({
  plugins: [react()],
  test: {
    include: ["test/**/*.test.{ts,tsx}"],
    environment: "jsdom",
    globals: true,
    setupFiles: ["./test/setup.ts"],
  },
});
