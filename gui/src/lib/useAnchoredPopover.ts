/**
 * useAnchoredPopover — the shared behaviour for a portal-rendered popover anchored
 * to a trigger element. It owns nothing about the popover's CONTENTS; it provides:
 *
 *   - `anchorRef` / `floatingRef` to wire the trigger and the floating panel,
 *   - a `floatingStyle` ({@link React.CSSProperties}) that fixes the panel next to
 *     the anchor and CLAMPS it inside the viewport so it never spills off-screen,
 *   - dismissal: Escape and a pointer-down outside both anchor and panel close it.
 *
 * Positioning uses `position: fixed` + the anchor's viewport rect, recomputed on
 * open and on scroll/resize, so a tile inside an `overflow: auto` grid still gets
 * an on-screen popover once it is portalled to `document.body`.
 *
 * Transitions are the caller's concern and must stay compositor-friendly
 * (transform/opacity) per the web performance rules.
 */

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

/** Which corner of the anchor the panel hangs from. */
export type PopoverPlacement = "bottom-start" | "bottom-end";

/** Gap (px) between the anchor and the floating panel. */
const OFFSET = 6;
/** Minimum viewport margin (px) the clamp keeps around the panel. */
const MARGIN = 8;

export interface AnchoredPopover<
  A extends HTMLElement = HTMLElement,
  F extends HTMLElement = HTMLDivElement,
> {
  /** Whether the popover is currently open. */
  open: boolean;
  /** Open/close the popover (idempotent). */
  setOpen: (open: boolean) => void;
  /** Attach to the trigger element. */
  anchorRef: React.RefObject<A | null>;
  /** Attach to the portalled floating panel. */
  floatingRef: React.RefObject<F | null>;
  /** The `position: fixed` style pinning the panel next to the anchor. */
  floatingStyle: React.CSSProperties;
  /** Recompute the panel position (call after its size changes). */
  reposition: () => void;
}

export function useAnchoredPopover<
  A extends HTMLElement = HTMLElement,
  F extends HTMLElement = HTMLDivElement,
>(placement: PopoverPlacement = "bottom-start"): AnchoredPopover<A, F> {
  const [open, setOpen] = useState(false);
  const [floatingStyle, setFloatingStyle] = useState<React.CSSProperties>({
    position: "fixed",
    top: 0,
    left: 0,
    // Hidden until the first measurement lands, so it never flashes at (0,0).
    visibility: "hidden",
  });
  const anchorRef = useRef<A | null>(null);
  const floatingRef = useRef<F | null>(null);

  const reposition = useCallback((): void => {
    const anchor = anchorRef.current;
    const floating = floatingRef.current;
    if (!anchor || !floating) return;
    const a = anchor.getBoundingClientRect();
    const f = floating.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;

    let left = placement === "bottom-end" ? a.right - f.width : a.left;
    let top = a.bottom + OFFSET;

    // Flip above the anchor when there is no room below but room above.
    if (top + f.height + MARGIN > vh && a.top - f.height - OFFSET >= MARGIN) {
      top = a.top - f.height - OFFSET;
    }
    // Clamp inside the viewport on both axes.
    left = Math.min(Math.max(MARGIN, left), Math.max(MARGIN, vw - f.width - MARGIN));
    top = Math.min(Math.max(MARGIN, top), Math.max(MARGIN, vh - f.height - MARGIN));

    setFloatingStyle({ position: "fixed", top, left, visibility: "visible" });
  }, [placement]);

  // Measure once mounted/open, then keep in sync with scroll + resize.
  useLayoutEffect(() => {
    if (!open) {
      setFloatingStyle((s) => ({ ...s, visibility: "hidden" }));
      return;
    }
    reposition();
    const onScroll = (): void => reposition();
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onScroll);
    return () => {
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onScroll);
    };
  }, [open, reposition]);

  // Dismissal: Escape, and a pointer-down outside the anchor + the panel.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setOpen(false);
        anchorRef.current?.focus();
      }
    };
    const onPointerDown = (e: PointerEvent): void => {
      const target = e.target as Node | null;
      if (
        target &&
        (anchorRef.current?.contains(target) || floatingRef.current?.contains(target))
      ) {
        return;
      }
      setOpen(false);
    };
    document.addEventListener("keydown", onKey, true);
    document.addEventListener("pointerdown", onPointerDown, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      document.removeEventListener("pointerdown", onPointerDown, true);
    };
  }, [open]);

  return { open, setOpen, anchorRef, floatingRef, floatingStyle, reposition };
}
