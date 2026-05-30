/**
 * App root — wires the appearance system and the app context provider around the
 * single-window Shell. The entire trader workspace lives under one provider so
 * the rail, command palette, and pair switcher re-target one shared state.
 */

import { useEffect } from "react";
import { AppProvider } from "./app/AppContext";
import { Shell } from "./app/Shell";
import { useAppearance } from "./design/appearance";

export function App(): React.ReactElement {
  // Initialize the appearance attributes on first paint (Dark default).
  const { appearance, contrast } = useAppearance();
  useEffect(() => {
    document.documentElement.setAttribute("data-appearance", appearance);
    document.documentElement.setAttribute("data-contrast", contrast === "high" ? "high" : "normal");
  }, [appearance, contrast]);

  return (
    <AppProvider>
      <Shell />
    </AppProvider>
  );
}
