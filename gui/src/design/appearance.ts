/**
 * Appearance state: the Aurora system supports Dark (default), Light, and an
 * orthogonal Increased-Contrast toggle (GUI-DESIGN §3.2, §7). Appearance is a
 * data attribute on <html> so the token cascade does all the work; no per-color
 * JS. Light and dark are equals (principle 9) — both fully designed.
 */

import { useCallback, useEffect, useState } from "react";

export type Appearance = "dark" | "light";
export type Contrast = "normal" | "high";

const APPEARANCE_KEY = "celnet.appearance";
const CONTRAST_KEY = "celnet.contrast";

function read<T extends string>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return (v as T) ?? fallback;
  } catch {
    return fallback;
  }
}

export function useAppearance(): {
  appearance: Appearance;
  contrast: Contrast;
  toggleAppearance: () => void;
  toggleContrast: () => void;
} {
  const [appearance, setAppearance] = useState<Appearance>(() =>
    read<Appearance>(APPEARANCE_KEY, "dark"),
  );
  const [contrast, setContrast] = useState<Contrast>(() =>
    read<Contrast>(CONTRAST_KEY, "normal"),
  );

  useEffect(() => {
    const root = document.documentElement;
    root.setAttribute("data-appearance", appearance);
    root.setAttribute("data-contrast", contrast === "high" ? "high" : "normal");
    try {
      localStorage.setItem(APPEARANCE_KEY, appearance);
      localStorage.setItem(CONTRAST_KEY, contrast);
    } catch {
      /* storage may be unavailable; appearance still applies in-session */
    }
  }, [appearance, contrast]);

  const toggleAppearance = useCallback(
    () => setAppearance((a) => (a === "dark" ? "light" : "dark")),
    [],
  );
  const toggleContrast = useCallback(
    () => setContrast((c) => (c === "high" ? "normal" : "high")),
    [],
  );

  return { appearance, contrast, toggleAppearance, toggleContrast };
}
