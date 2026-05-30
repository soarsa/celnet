import { defineConfig } from "vitest/config";

/**
 * Unit-test config (separate from the Vite app build, which roots at `src/`).
 * Tests run under node — they exercise the pure request-shaping, dynamic-array
 * formatting, streaming dedup/refcount/stale logic, and ticket state machines
 * with NO server and NO mocks of our own functionality.
 */
export default defineConfig({
  test: {
    include: ["test/**/*.test.ts"],
    environment: "node",
  },
});
