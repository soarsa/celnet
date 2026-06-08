/**
 * App root — wires the appearance system and the app context provider around the
 * single-window Shell. The entire trader workspace lives under one provider so
 * the rail, command palette, and pair switcher re-target one shared state.
 */

import { useEffect } from "react";
import { AppProvider } from "./app/AppContext";
import { Shell } from "./app/Shell";
import { useAppearance } from "./design/appearance";
import { useDensity } from "./design/density";

export function App(): React.ReactElement {
  // Initialize the appearance attributes on first paint (Dark default).
  const { appearance, contrast } = useAppearance();
  // The orthogonal density axis (data-density; comfortable default). Mounting the
  // hook applies the cascade attribute on first paint, just like appearance. This
  // is the ONE allowed JS touch-point for density (the GW0 cascade-disjointness
  // contract): the value + toggle are passed into the provider so components
  // consume `app.density` WITHOUT importing the hook (nothing else reads density
  // in JS — the CSS cascade does the work).
  const { density, toggleDensity } = useDensity();
  useEffect(() => {
    document.documentElement.setAttribute("data-appearance", appearance);
    document.documentElement.setAttribute("data-contrast", contrast === "high" ? "high" : "normal");
  }, [appearance, contrast]);

  return (
    <AppProvider density={density} toggleDensity={toggleDensity}>
      <Shell />
    </AppProvider>
  );
}
