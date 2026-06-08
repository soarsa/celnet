/**
 * Density axis (GW0): the third orthogonal token axis alongside
 * `data-appearance` (dark|light) and `data-contrast` (normal|high). Density is a
 * data attribute on <html> so the token cascade does ALL the work — row heights,
 * cell padding and gap collapse to the `compact` scale for IB-cardinality grids
 * (thousands of rows on one screen) while `comfortable` (default) keeps the
 * relaxed dense-pro rhythm. NO component reads density in JS; like appearance,
 * every surface opts in purely by consuming the density-scaled tokens in CSS.
 *
 * This mirrors `appearance.ts`'s proven attribute+localStorage pattern exactly so
 * the three axes compose without per-axis JS.
 */

import { useCallback, useEffect, useState } from "react";

export type Density = "comfortable" | "compact";

const DENSITY_KEY = "celnet.density";

/** The single source of truth for the attribute name, shared with the cascade. */
export const DENSITY_ATTR = "data-density";

function read(fallback: Density): Density {
  try {
    const v = localStorage.getItem(DENSITY_KEY);
    return v === "comfortable" || v === "compact" ? v : fallback;
  } catch {
    return fallback;
  }
}

/**
 * Apply the density attribute to <html> (idempotent). Exposed so a non-React
 * boot path or a test can set the cascade without the hook.
 */
export function applyDensity(density: Density): void {
  document.documentElement.setAttribute(DENSITY_ATTR, density);
}

export function useDensity(): {
  density: Density;
  setDensity: (d: Density) => void;
  toggleDensity: () => void;
} {
  const [density, setDensity] = useState<Density>(() => read("comfortable"));

  useEffect(() => {
    applyDensity(density);
    try {
      localStorage.setItem(DENSITY_KEY, density);
    } catch {
      /* storage may be unavailable; density still applies in-session */
    }
  }, [density]);

  const toggleDensity = useCallback(
    () => setDensity((d) => (d === "comfortable" ? "compact" : "comfortable")),
    [],
  );

  return { density, setDensity, toggleDensity };
}
