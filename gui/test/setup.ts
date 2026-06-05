/**
 * Global test setup: register `@testing-library/jest-dom`'s custom matchers
 * (e.g. `toBeInTheDocument`) and auto-clean the React render tree between tests
 * so component suites don't leak DOM into one another. Loaded by
 * `vitest.config.ts` `setupFiles`.
 */
import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

afterEach(() => {
  cleanup();
});
