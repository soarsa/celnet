/**
 * virtual.ts tests — the dependency-free fixed-row windowing primitive behind the
 * IB-scale virtualised blotter. We drive the REAL `useVirtualWindow` hook through
 * its public surface in jsdom: render it, attach a scroll container via the
 * returned `ref`, set the element's geometry + fire a `scroll` event (the hook
 * reads `scrollTop`/`clientHeight` off the element), and assert the derived
 * `[start,end)` slice + spacer heights.
 *
 * The windowing model under test (from the module doc):
 *   totalHeight = count*rowHeight
 *   start = max(0, floor(scrollTop/row) - overscan)
 *   end   = min(count, ceil((scrollTop+viewport)/row) + overscan)
 *   padTop = start*row ; padBottom = (count-end)*row
 */
import { act } from "react";
import { describe, expect, it } from "vitest";
import { renderHook } from "@testing-library/react";

import { useVirtualWindow } from "../src/lib/virtual";

/** Attach a real jsdom element to the hook's ref and give it scroll geometry. */
function makeScroller(scrollTop: number, clientHeight: number): HTMLDivElement {
  const el = document.createElement("div");
  // jsdom does not do layout, so scrollTop/clientHeight are settable plain props.
  Object.defineProperty(el, "scrollTop", { value: scrollTop, configurable: true });
  Object.defineProperty(el, "clientHeight", { value: clientHeight, configurable: true });
  return el;
}

describe("useVirtualWindow — pre-measure (no element attached)", () => {
  it("renders just the overscan head so the list paints non-empty", () => {
    const { result } = renderHook(() =>
      useVirtualWindow({ count: 1000, rowHeight: 24, overscan: 6 }),
    );
    expect(result.current.start).toBe(0);
    // viewport=0 ⇒ lastVisible falls back to overscan(6); end = min(count, 6+6).
    expect(result.current.end).toBe(12);
    expect(result.current.totalHeight).toBe(1000 * 24);
    expect(result.current.padTop).toBe(0);
    expect(result.current.padBottom).toBe((1000 - 12) * 24);
  });
});

describe("useVirtualWindow — windowed slice after a scroll", () => {
  it("derives the visible [start,end) slice + spacers from scroll geometry", () => {
    const { result } = renderHook(() =>
      useVirtualWindow({ count: 1000, rowHeight: 20, overscan: 5 }),
    );

    // viewport 400px over 20px rows = 20 rows visible; scroll to row 100.
    const el = makeScroller(2000, 400);
    act(() => {
      result.current.ref(el);
      el.dispatchEvent(new Event("scroll"));
    });

    // firstVisible = floor(2000/20) = 100 ; start = 100 - 5 = 95.
    // lastVisible  = ceil((2000+400)/20) = 120 ; end = 120 + 5 = 125.
    expect(result.current.start).toBe(95);
    expect(result.current.end).toBe(125);
    expect(result.current.padTop).toBe(95 * 20);
    expect(result.current.padBottom).toBe((1000 - 125) * 20);
    expect(result.current.totalHeight).toBe(1000 * 20);
  });

  it("clamps start at 0 near the head and end at count near the tail", () => {
    const { result } = renderHook(() =>
      useVirtualWindow({ count: 50, rowHeight: 20, overscan: 5 }),
    );
    // Scrolled past the end with a tall viewport — must clamp, never overrun.
    const el = makeScroller(10_000, 2_000);
    act(() => {
      result.current.ref(el);
      el.dispatchEvent(new Event("scroll"));
    });
    expect(result.current.start).toBeGreaterThanOrEqual(0);
    expect(result.current.end).toBe(50);
    expect(result.current.padBottom).toBe(0);
  });
});

describe("useVirtualWindow — degenerate inputs", () => {
  it("an empty list yields an empty window with zero heights", () => {
    const { result } = renderHook(() =>
      useVirtualWindow({ count: 0, rowHeight: 24 }),
    );
    expect(result.current.start).toBe(0);
    expect(result.current.end).toBe(0);
    expect(result.current.totalHeight).toBe(0);
    expect(result.current.padTop).toBe(0);
    expect(result.current.padBottom).toBe(0);
  });

  it("a zero rowHeight is treated as 1px so the math never divides by zero", () => {
    const { result } = renderHook(() =>
      useVirtualWindow({ count: 10, rowHeight: 0 }),
    );
    // totalHeight uses the safe row (1) and the offset helper too.
    expect(result.current.totalHeight).toBe(10);
    expect(result.current.offsetOf(3)).toBe(3);
  });

  it("offsetOf maps a row index to its absolute top offset", () => {
    const { result } = renderHook(() =>
      useVirtualWindow({ count: 100, rowHeight: 32 }),
    );
    expect(result.current.offsetOf(0)).toBe(0);
    expect(result.current.offsetOf(10)).toBe(320);
  });
});
