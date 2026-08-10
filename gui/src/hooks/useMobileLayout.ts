/**
 * useMobileLayout — decide whether to render the touch-friendly read-only
 * {@link ../app/mobile/MobileStatusApp} in place of the dense desktop `Shell`.
 *
 * Detection is VIEWPORT-driven (responsive, not UA-sniffed) with a coarse-pointer
 * assist: a narrow viewport (≤ 768px) is mobile; a coarse pointer on a not-wide
 * viewport (≤ 1024px, e.g. a large phone in landscape) is also mobile; a fine-pointer
 * desktop stays desktop even with a touchscreen. It re-evaluates on resize /
 * orientation change and on the pointer media-query flipping.
 *
 * An escape hatch — "Use full app" — is persisted in localStorage so a trader can
 * force the desktop Shell on a phone and get back. `showMobile` is the composed
 * decision the app root switches on: mobile viewport AND not forced-desktop.
 */

import { useCallback, useEffect, useState } from "react";

/** localStorage key for the "use the full desktop app on mobile" override. */
export const FORCE_DESKTOP_KEY = "celnet.mobile.forceDesktop";

/** The viewport width (px) at/below which we treat the session as mobile. */
export const MOBILE_MAX_WIDTH = 768;
/** The width ceiling under which a COARSE pointer (touch) also counts as mobile. */
export const COARSE_POINTER_MAX_WIDTH = 1024;

/**
 * The pure detection rule (unit-tested without a DOM): a narrow viewport is mobile,
 * OR a coarse pointer on a not-wide viewport. A fine-pointer wide desktop is never
 * mobile, so a desktop touchscreen at 1920px stays on the full Shell.
 */
export function computeIsMobile(width: number, coarsePointer: boolean): boolean {
  if (width <= MOBILE_MAX_WIDTH) return true;
  return coarsePointer && width <= COARSE_POINTER_MAX_WIDTH;
}

/** Read the current mobile-viewport state from the live window (SSR/test safe). */
function readIsMobile(): boolean {
  if (typeof window === "undefined") return false;
  const width = window.innerWidth || COARSE_POINTER_MAX_WIDTH + 1;
  const coarse =
    typeof window.matchMedia === "function" && window.matchMedia("(pointer: coarse)").matches;
  return computeIsMobile(width, coarse);
}

/** Read the persisted force-desktop override (defaults to false). */
function readForceDesktop(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(FORCE_DESKTOP_KEY) === "1";
  } catch {
    return false;
  }
}

/** The layout decision + the escape-hatch control. */
export interface MobileLayout {
  /** Whether the viewport/pointer currently reads as a mobile device. */
  isMobileViewport: boolean;
  /** Whether the user forced the full desktop app (persisted). */
  forceDesktop: boolean;
  /** The composed decision: render the mobile status board (mobile AND not forced). */
  showMobile: boolean;
  /** Set (and persist) the force-desktop override. */
  setForceDesktop: (value: boolean) => void;
}

export function useMobileLayout(): MobileLayout {
  const [isMobileViewport, setIsMobileViewport] = useState<boolean>(readIsMobile);
  const [forceDesktop, setForceDesktopState] = useState<boolean>(readForceDesktop);

  // Re-evaluate on resize / orientation change and when the pointer capability flips
  // (a media-query change fires on both a viewport crossing and an input-mode change).
  useEffect(() => {
    if (typeof window === "undefined") return;
    const reevaluate = (): void => setIsMobileViewport(readIsMobile());
    window.addEventListener("resize", reevaluate);
    window.addEventListener("orientationchange", reevaluate);
    const coarse =
      typeof window.matchMedia === "function" ? window.matchMedia("(pointer: coarse)") : null;
    coarse?.addEventListener?.("change", reevaluate);
    // Reconcile once in case the initial paint raced the first layout.
    reevaluate();
    return () => {
      window.removeEventListener("resize", reevaluate);
      window.removeEventListener("orientationchange", reevaluate);
      coarse?.removeEventListener?.("change", reevaluate);
    };
  }, []);

  // Cross-tab sync of the escape-hatch preference.
  useEffect(() => {
    if (typeof window === "undefined") return;
    const onStorage = (event: StorageEvent): void => {
      if (event.key === FORCE_DESKTOP_KEY) setForceDesktopState(readForceDesktop());
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);

  const setForceDesktop = useCallback((value: boolean): void => {
    setForceDesktopState(value);
    try {
      if (value) window.localStorage.setItem(FORCE_DESKTOP_KEY, "1");
      else window.localStorage.removeItem(FORCE_DESKTOP_KEY);
    } catch {
      // A private-mode / disabled storage still toggles for this session (state above);
      // it just won't persist across a reload. Honest degradation, never a throw.
    }
  }, []);

  return {
    isMobileViewport,
    forceDesktop,
    showMobile: isMobileViewport && !forceDesktop,
    setForceDesktop,
  };
}
