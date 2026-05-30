/**
 * Browser entry point. Mounts the Celnet trader GUI under React 19's
 * concurrent root and pulls in the Aurora global stylesheet (which imports the
 * design tokens).
 */

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./design/global.css";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root element");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
