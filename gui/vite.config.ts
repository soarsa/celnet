import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Minimal ambient declarations for the two Node built-ins we touch here, so the
// config type-checks under the project's node tsconfig WITHOUT pulling in the
// full @types/node dependency (kept lean per the brand-foundation brief).
declare function require(id: string): {
  execSync(cmd: string, opts?: unknown): { toString(): string };
};
declare const process: { env: Record<string, string | undefined> };

// A REAL short build hash + UTC build timestamp, stamped into the status ribbon
// (the Celer login-footer signature detail). The hash is the current git short
// SHA when available; off a git tree (e.g. a packaged artifact) it falls back to
// the package version — never a fabricated value.
function buildHash(): string {
  try {
    const { execSync } = require("node:child_process");
    return execSync("git rev-parse --short HEAD", {
      stdio: ["ignore", "pipe", "ignore"],
    })
      .toString()
      .trim();
  } catch {
    return `v${process.env.npm_package_version ?? "0.0.0"}`;
  }
}

// Celnet GUI build config. The transport seam (src/data/transport) is the only
// place a real gRPC-Web/Connect or WebSocket client is wired; everything else is
// fed by the deterministic in-app mock/replay source so the app runs standalone.
export default defineConfig({
  plugins: [react()],
  define: {
    __CELNET_BUILD_HASH__: JSON.stringify(buildHash() || "unknown"),
    __CELNET_BUILD_TIME__: JSON.stringify(new Date().toISOString()),
  },
  build: {
    target: "es2022",
    sourcemap: true,
  },
});
