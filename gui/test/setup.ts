/**
 * Global test setup: register `@testing-library/jest-dom`'s custom matchers
 * (e.g. `toBeInTheDocument`) and auto-clean the React render tree between tests
 * so component suites don't leak DOM into one another. Loaded by
 * `vitest.config.ts` `setupFiles`.
 */
import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

/**
 * jsdom does not implement `ResizeObserver` (the layout engine that backs it is
 * not present), so any component that measures its container — e.g. the surface
 * 3D mesh (`SurfaceMesh`) — would throw at mount under the test environment.
 * Provide a standards-shaped, no-layout stub: `observe`/`unobserve`/`disconnect`
 * exist and are inert. jsdom reports a zero-size layout, so a measured callback
 * would carry no useful box anyway; the component's own min-size floors cover the
 * headless case. This is a JS-DOM environment gap fill, not a mock of any Celnet
 * functionality.
 */
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class ResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  };
}

afterEach(() => {
  cleanup();
});
