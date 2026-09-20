import { defineConfig, type Plugin } from "vite";
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

// The build identity, computed ONCE and shared by both sides of the
// release-detection seam: the `__CELNET_BUILD_*` constants baked into the running
// bundle, AND the `/version.json` manifest emitted beside index.html. Identical
// values on both sides are what make the running page's self-vs-served comparison
// exact (see src/data/versionManifest.ts).
const BUILD_HASH = buildHash() || "unknown";
const BUILD_TIME = new Date().toISOString();

// celnet-version-manifest — emit a tiny, never-cached `/version.json`
// ({ hash, buildTime }) next to index.html, and serve the same payload in dev.
// The running SPA polls it; when the served identity differs from its own
// baked-in constants it knows a newer release was deployed and offers a reload.
// No server API is involved — this is a static deploy artifact, not a versioned
// contract (CLAUDE.md §9).
function versionManifest(hash: string, buildTime: string): Plugin {
  const body = `${JSON.stringify({ hash, buildTime })}\n`;
  return {
    name: "celnet-version-manifest",
    configureServer(server) {
      server.middlewares.use("/version.json", (_req, res) => {
        res.setHeader("Content-Type", "application/json");
        res.setHeader("Cache-Control", "no-store");
        res.end(body);
      });
    },
    generateBundle() {
      this.emitFile({ type: "asset", fileName: "version.json", source: body });
    },
  };
}

// Celnet GUI build config. The transport seam (src/data/transport) is the only
// place a real gRPC-Web/Connect or WebSocket client is wired; everything else is
// fed by the deterministic in-app mock/replay source so the app runs standalone.
export default defineConfig({
  base: "./",
  plugins: [react(), versionManifest(BUILD_HASH, BUILD_TIME)],
  define: {
    __CELNET_BUILD_HASH__: JSON.stringify(BUILD_HASH),
    __CELNET_BUILD_TIME__: JSON.stringify(BUILD_TIME),
  },
  build: {
    target: "es2022",
    sourcemap: true,
  },
});
