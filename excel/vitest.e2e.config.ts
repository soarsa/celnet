import { defineConfig } from "vitest/config";

/**
 * REAL-EDGE conformance config (separate from the unit-test `vitest.config.ts`,
 * which uses an in-process FakeSocket). This config boots the REAL `celnet-server`
 * demo edge once via `e2e/globalSetup.ts`, then runs the conformance spec against
 * it over a REAL WebSocket. Tests run single-threaded over the one shared
 * connection, with generous timeouts covering the heaviest Monte-Carlo family.
 */
export default defineConfig({
  test: {
    include: ["e2e/**/*.e2e.ts"],
    environment: "node",
    globalSetup: ["./e2e/globalSetup.ts"],
    // One shared real connection ⇒ run the spec serially in a single worker.
    fileParallelism: false,
    pool: "forks",
    poolOptions: { forks: { singleFork: true } },
    testTimeout: 120_000,
    hookTimeout: 320_000,
    teardownTimeout: 30_000,
  },
});
