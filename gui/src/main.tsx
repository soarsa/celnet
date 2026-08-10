/**
 * Browser entry point. Mounts the Celnet trader GUI under React 19's
 * concurrent root and pulls in the Aurora global stylesheet (which imports the
 * design tokens).
 */

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { SettingsProvider } from "./settings/SettingsProvider";
import { installNumberInputAutoSelect } from "./lib/numberInputAutoSelect";
import "./design/global.css";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root element");

// Select a numeric field's contents on focus so the first keystroke replaces
// the value instead of appending after the default `0` (applies app-wide).
installNumberInputAutoSelect();

createRoot(root).render(
  <StrictMode>
    <SettingsProvider>
      <App />
    </SettingsProvider>
  </StrictMode>,
);
