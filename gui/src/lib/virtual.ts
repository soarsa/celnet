/**
 * virtual.ts — a tiny, dependency-free windowed-list hook for the virtualised
 * blotter (TRADING-UNIVERSE-SCALE §5: an IB-sized blotter renders only the rows
 * in view). NO new npm dependency: a fixed-row-height windowing primitive that
 * derives the visible index range from a scroll container's scrollTop, padded by
 * an overscan, and gives the caller the spacer heights to position the window.
 *
 * The model is the standard fixed-height virtualiser:
 *   total height   = count * rowHeight
 *   first visible  = floor(scrollTop / rowHeight) - overscan   (clamped ≥ 0)
 *   last  visible  = ceil((scrollTop + viewport) / rowHeight) + overscan
 *   padTop         = start * rowHeight       (height of the unrendered head)
 *   padBottom      = (count - end) * rowHeight (height of the unrendered tail)
 * The caller renders one scroll container of `totalHeight`, a top spacer of
 * `padTop`, the `[start,end)` rows, then a bottom spacer of `padBottom`.
 *
 * It is fully controlled by the caller's scroll element via the returned
 * `onScroll` handler + a `ResizeObserver`-free viewport input the caller wires
 * from the element (we read it lazily on scroll and accept an explicit viewport
 * override), so it has zero external deps and is SSR-safe (no window access).
 */

import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";

/** Parameters for the windowing computation. */
export interface VirtualWindowOptions {
  /** Total number of rows in the list. */
  count: number;
  /** Fixed row height in CSS px (uniform rows). */
  rowHeight: number;
  /** Extra rows rendered above/below the viewport to mask fast scrolls. Default 6. */
  overscan?: number;
}

/** The computed window + the wiring the caller attaches to its scroll element. */
export interface VirtualWindow<E extends HTMLElement = HTMLDivElement> {
  /** Attach to the scrolling container (sets up scroll + resize tracking). */
  ref: (el: E | null) => void;
  /** First row index to render (inclusive). */
  start: number;
  /** One past the last row index to render (exclusive). */
  end: number;
  /** Total scrollable content height in px (count * rowHeight). */
  totalHeight: number;
  /** Height of the top spacer in px (start * rowHeight). */
  padTop: number;
  /** Height of the bottom spacer in px ((count - end) * rowHeight). */
  padBottom: number;
  /**
   * Convenience helper: maps a row index in `[start,end)` to its absolute top
   * offset in px (index * rowHeight) — for absolute-positioned row variants.
   */
  offsetOf: (index: number) => number;
}

/**
 * Windowed-list hook. Tracks the attached element's `scrollTop` + `clientHeight`
 * and returns the visible `[start,end)` slice plus the spacer heights. Re-renders
 * only when the slice actually changes (scrolling within a row is a no-op).
 */
export function useVirtualWindow<E extends HTMLElement = HTMLDivElement>(
  opts: VirtualWindowOptions,
): VirtualWindow<E> {
  const { count, rowHeight, overscan = 6 } = opts;
  const elRef = useRef<E | null>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(0);

  // Read the live geometry from the element; only commit state on real change so
  // a sub-row scroll that doesn't move the window stays cheap (still one setState
  // but React bails the slice memo below). Guard against negative/NaN.
  const measure = useCallback((el: E): void => {
    const top = Math.max(0, el.scrollTop);
    const h = Math.max(0, el.clientHeight);
    setScrollTop((prev) => (prev === top ? prev : top));
    setViewport((prev) => (prev === h ? prev : h));
  }, []);

  // Ref callback: wire/unwire a scroll listener + a ResizeObserver for the
  // viewport. Returns nothing (React ref-callback contract).
  const ro = useRef<ResizeObserver | null>(null);
  const onScroll = useRef<(() => void) | null>(null);

  const ref = useCallback(
    (el: E | null): void => {
      // Tear down any previous wiring.
      if (elRef.current && onScroll.current) {
        elRef.current.removeEventListener("scroll", onScroll.current);
      }
      if (ro.current) {
        ro.current.disconnect();
        ro.current = null;
      }
      elRef.current = el;
      if (!el) return;
      const handler = (): void => measure(el);
      onScroll.current = handler;
      el.addEventListener("scroll", handler, { passive: true });
      if (typeof ResizeObserver !== "undefined") {
        ro.current = new ResizeObserver(() => measure(el));
        ro.current.observe(el);
      }
      // Initial measure.
      measure(el);
    },
    [measure],
  );

  // Recompute the slice whenever the inputs change. Pure arithmetic.
  const win = useMemo(() => {
    const safeRow = rowHeight > 0 ? rowHeight : 1;
    const totalHeight = count * safeRow;
    if (count === 0) {
      return { start: 0, end: 0, totalHeight: 0, padTop: 0, padBottom: 0 };
    }
    // A zero viewport (pre-measure) renders just the overscan head so the list
    // is non-empty on first paint; the post-mount measure fills it in.
    const firstVisible = Math.floor(scrollTop / safeRow);
    const lastVisible = viewport > 0 ? Math.ceil((scrollTop + viewport) / safeRow) : overscan;
    const start = Math.max(0, firstVisible - overscan);
    const end = Math.min(count, lastVisible + overscan);
    return {
      start,
      end,
      totalHeight,
      padTop: start * safeRow,
      padBottom: (count - end) * safeRow,
    };
  }, [count, rowHeight, overscan, scrollTop, viewport]);

  // If the row count shrinks below the current scroll position, the element's
  // scrollTop becomes stale; re-measure after layout so the window re-clamps.
  useLayoutEffect(() => {
    if (elRef.current) measure(elRef.current);
  }, [count, measure]);

  const offsetOf = useCallback(
    (index: number): number => index * (rowHeight > 0 ? rowHeight : 1),
    [rowHeight],
  );

  return { ref, ...win, offsetOf };
}
