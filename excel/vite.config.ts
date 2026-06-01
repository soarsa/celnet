import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";
import { defineConfig } from "vite";
import { officeAddin } from "./tools/vitePluginOffice";

/**
 * HTTPS for the dev server: Office requires TLS for a sideloaded add-in. Read the
 * `office-addin-dev-certs` localhost cert (run `office-addin-dev-certs install`
 * once — see README). Falls back to HTTP if the certs are absent so a plain
 * `vite build` / non-Office dev still works.
 */
function devHttps(): { cert: Buffer; key: Buffer } | undefined {
  try {
    const dir = resolve(homedir(), ".office-addin-dev-certs");
    return {
      cert: readFileSync(resolve(dir, "localhost.crt")),
      key: readFileSync(resolve(dir, "localhost.key")),
    };
  } catch {
    return undefined;
  }
}

/**
 * Build the Office.js add-in as two HTML entry points served over HTTPS:
 *  - `functions.html` boots the custom-function runtime (CELNET.* worksheet
 *    functions) — referenced by the manifest `<Page>` for the functions file.
 *  - `taskpane.html` is the ticket task pane SPA (RFQ + mark-surface submit).
 *
 * No framework, no generator: a minimal multi-page Vite build. The dev server
 * runs on HTTPS:3000 (Office requires HTTPS for sideloaded add-ins); generate a
 * dev certificate with `office-addin-dev-certs` or your own mkcert (see README).
 */
const httpsCerts = devHttps();

export default defineConfig({
  plugins: [officeAddin()],
  root: resolve(__dirname, "src"),
  publicDir: resolve(__dirname, "assets"),
  base: "./",
  build: {
    outDir: resolve(__dirname, "dist"),
    emptyOutDir: true,
    target: "es2022",
    rollupOptions: {
      input: {
        functions: resolve(__dirname, "src/functions.html"),
        taskpane: resolve(__dirname, "src/taskpane.html"),
      },
    },
  },
  server: {
    port: 3000,
    strictPort: true,
    host: "localhost",
    // Only set `https` when the dev certs exist (exactOptionalPropertyTypes:
    // the narrowed const keeps `undefined` out of the spread value).
    ...(httpsCerts ? { https: httpsCerts } : {}),
  },
});
