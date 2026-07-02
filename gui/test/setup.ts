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

/**
 * jsdom also does not implement `Element.scrollIntoView` (no layout engine to
 * scroll), so list components that reveal the keyboard-cursor row (the scope
 * switcher's listbox, the structure gallery) would throw inside `act(...)`.
 * Provide the same inert, standards-shaped gap fill: the call exists and does
 * nothing — jsdom has no scroll geometry for it to affect anyway. An environment
 * gap fill, not a mock of any Celnet functionality.
 */
if (typeof Element.prototype.scrollIntoView !== "function") {
  Element.prototype.scrollIntoView = (): void => {};
}

/**
 * jsdom does not implement `window.matchMedia` (no media-query engine), so any
 * component that reads a media query in JS — e.g. the viz charts gating animation
 * on `prefers-reduced-motion` — would throw at mount. Provide a standards-shaped,
 * inert stub reporting no match (motion allowed) with working add/removeEventListener.
 * An environment gap fill, not a mock of any Celnet functionality.
 */
if (typeof window.matchMedia !== "function") {
  window.matchMedia = (query: string): MediaQueryList =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: (): void => {},
      removeEventListener: (): void => {},
      addListener: (): void => {},
      removeListener: (): void => {},
      dispatchEvent: (): boolean => false,
    }) as MediaQueryList;
}

/**
 * jsdom does not implement `HTMLCanvasElement.getContext` (no raster engine — it
 * throws "Not implemented" and returns undefined). Canvas-backed viz (ECharts, the
 * 3D surface's 2D overlays) would then call methods on `undefined` and throw. Provide
 * a standards-shaped, inert **2D** context stub (all ops no-op) and return `null` for
 * `webgl`/`webgl2` (three.js's own try/catch renders its honest WebGL-unavailable
 * fallback). An environment gap fill, not a mock of any Celnet functionality.
 */
if (
  typeof HTMLCanvasElement !== "undefined" &&
  !(HTMLCanvasElement.prototype as unknown as { __ctxStub?: boolean }).__ctxStub
) {
  const noop = (): void => {};
  const make2d = (): unknown => ({
    canvas: undefined,
    save: noop, restore: noop, scale: noop, translate: noop, rotate: noop, transform: noop,
    setTransform: noop, resetTransform: noop, beginPath: noop, closePath: noop, moveTo: noop,
    lineTo: noop, bezierCurveTo: noop, quadraticCurveTo: noop, rect: noop, arc: noop, arcTo: noop,
    ellipse: noop, fill: noop, stroke: noop, clip: noop, fillRect: noop, clearRect: noop,
    strokeRect: noop, fillText: noop, strokeText: noop, setLineDash: noop, getLineDash: () => [],
    measureText: () => ({ width: 0 }),
    createLinearGradient: () => ({ addColorStop: noop }),
    createRadialGradient: () => ({ addColorStop: noop }),
    createPattern: () => null,
    getImageData: () => ({ data: new Uint8ClampedArray(4), width: 1, height: 1 }),
    putImageData: noop, drawImage: noop, createImageData: () => ({ data: new Uint8ClampedArray(4) }),
    fillStyle: "#000", strokeStyle: "#000", lineWidth: 1, lineCap: "butt", lineJoin: "miter",
    font: "10px sans-serif", textAlign: "start", textBaseline: "alphabetic", globalAlpha: 1,
    globalCompositeOperation: "source-over", shadowBlur: 0, shadowColor: "transparent",
  });
  HTMLCanvasElement.prototype.getContext = function (type: string): unknown {
    return type === "2d" ? make2d() : null;
  } as typeof HTMLCanvasElement.prototype.getContext;
  (HTMLCanvasElement.prototype as unknown as { __ctxStub?: boolean }).__ctxStub = true;
}

afterEach(() => {
  cleanup();
});
