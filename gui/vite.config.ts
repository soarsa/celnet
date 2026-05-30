import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Celnet GUI build config. The transport seam (src/data/transport) is the only
// place a real gRPC-Web/Connect or WebSocket client is wired; everything else is
// fed by the deterministic in-app mock/replay source so the app runs standalone.
export default defineConfig({
  plugins: [react()],
  build: {
    target: "es2022",
    sourcemap: true,
  },
});
