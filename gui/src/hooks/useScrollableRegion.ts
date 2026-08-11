/**
 * useScrollableRegion — the ONE honest "is this scrollport actually scrollable?"
 * measurement, shared by every bounded `overflow: auto` container in the app.
 *
 * WHY it must be measured rather than assumed (WCAG 2.1.1 / axe
 * `scrollable-region-focusable`): a scroll container whose content genuinely
 * overflows MUST be reachable from the keyboard, because some of them (the
 * surface mesh, the vega ladder, a numbers-only blotter) contain no focusable
 * content at all — without a tab stop a keyboard user simply cannot scroll them.
 * The inverse is equally true: a container that does NOT overflow must add NO
 * tab stop, or every table on the screen becomes a dead stop in the tab order.
 * A hardcoded `tabIndex={0}` gets the second half wrong, which is exactly what
 * this replaces.
 *
 * The measurement has to run on two independent triggers because overflow can
 * start or stop for two unrelated reasons:
 *   - CONTENT changed (rows streamed in, a filter narrowed the set) — that only
 *     ever lands through a React commit, so a per-commit `useLayoutEffect` read
 *     catches it. One `scrollHeight` read on one element is a single cheap
 *     forced layout, and the state setter bails when unchanged (no render loop).
 *   - The CONTAINER resized (window resize, a grid track changing, a sibling
 *     panel collapsing) — that happens with NO React commit at all, so it needs
 *     a `ResizeObserver`.
 *
 * Extracted from `Panel` so the semantic `<DataTable>` binding inherits the same
 * measured behaviour instead of copying it (a second copy would inevitably drift
 * and regress the axe rule on one surface but not the other).
 */

import { useEffect, useLayoutEffect, useRef, useState } from "react";

/** Does this element's content overflow its scrollport (either axis)? */
function overflows(el: HTMLElement): boolean {
  return el.scrollHeight > el.clientHeight || el.scrollWidth > el.clientWidth;
}

/**
 * Measure whether a scrollport overflows.
 *
 * @returns `[ref, scrollable]` — attach `ref` to the `overflow: auto` element;
 *   `scrollable` is true only while its content genuinely overflows.
 */
export function useScrollableRegion<E extends HTMLElement>(): [
  React.RefObject<E | null>,
  boolean,
] {
  const ref = useRef<E>(null);
  const [scrollable, setScrollable] = useState(false);

  // Content changes: every commit re-measures (see the module doc).
  useLayoutEffect(() => {
    const el = ref.current;
    if (el) setScrollable(overflows(el));
  });

  // Container resizes: no commit happens, so observe the box directly.
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => setScrollable(overflows(el)));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  return [ref, scrollable];
}

/**
 * The ARIA/tabindex props a scrollport must carry, derived from a live
 * {@link useScrollableRegion} measurement. Spread onto the scroll container.
 *
 * A NON-overflowing container gets an empty object — deliberately no `tabIndex`,
 * so it adds no tab stop. An overflowing one becomes focusable, and is exposed
 * as a named `region` when the caller has a label to give it (an unnamed region
 * landmark is noise, so the role is omitted without one).
 */
export function scrollableRegionProps(
  scrollable: boolean,
  label?: { readonly labelledBy?: string; readonly label?: string },
): Record<string, unknown> {
  if (!scrollable) return {};
  if (label?.labelledBy !== undefined) {
    return { tabIndex: 0, role: "region", "aria-labelledby": label.labelledBy };
  }
  if (label?.label !== undefined) {
    return { tabIndex: 0, role: "region", "aria-label": label.label };
  }
  return { tabIndex: 0 };
}
