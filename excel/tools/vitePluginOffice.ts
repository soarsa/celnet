/**
 * Vite plugin: serve the Office custom-functions artifacts at the exact URLs the
 * add-in manifest references, in BOTH `vite` (dev server) and `vite build` (dist).
 *
 * The Office manifest (excel/manifest.xml) declares three URLs Office fetches to
 * register the `=CELNET.*` worksheet functions:
 *   - `/functions.json`  — the custom-function METADATA (registration).
 *   - `/functions.js`    — the custom-function SCRIPT (shared-runtime: the Page in
 *                          functions.html is authoritative; this endpoint exists so
 *                          the manifest's <Script> SourceLocation resolves).
 *   - `/functions.html`  — the Page Vite already emits as a build input.
 * The metadata lives at `src/functions/functions.json`; without this plugin it is
 * served at `/functions/functions.json` in dev and not emitted to dist at all, so a
 * real sideload would fail to register the functions. This plugin closes that gap
 * with one mechanism for both modes (no second server, no drift).
 *
 * Icons are served by Vite's `publicDir` at the root (`/icon-32.png` …); the
 * manifest references those root paths directly.
 */
import { copyFileSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import type { Plugin } from "vite";

const SRC = resolve(__dirname, "..", "src");
const METADATA = resolve(SRC, "functions", "functions.json");

/** The shared-runtime Script endpoint: re-import the Page's functions module so
 * its `CustomFunctions.associate(...)` side effects register the namespace. In
 * dev this points at the TS source (Vite transforms it); in dist the closeBundle
 * hook rewrites it to the built chunk. */
const DEV_SCRIPT = `import "/functions/functions.ts";\n`;

export function officeAddin(): Plugin {
  return {
    name: "celnet-office-addin",

    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const url = (req.url ?? "").split("?")[0];
        if (url === "/functions.json") {
          res.setHeader("Content-Type", "application/json");
          res.setHeader("Access-Control-Allow-Origin", "*");
          res.end(readFileSync(METADATA));
          return;
        }
        if (url === "/functions.js") {
          res.setHeader("Content-Type", "text/javascript");
          res.setHeader("Access-Control-Allow-Origin", "*");
          res.end(DEV_SCRIPT);
          return;
        }
        next();
      });
    },

    // After the static build, emit the manifest-referenced artifacts into dist/.
    closeBundle() {
      const dist = resolve(__dirname, "..", "dist");
      // 1. Metadata at the manifest path.
      copyFileSync(METADATA, resolve(dist, "functions.json"));
      // 2. Script: a root-level module that imports the built functions chunk, so
      //    `/functions.js` resolves to the real, registering code. Discover the
      //    hashed chunk from the emitted functions.html (Vite hashes asset names).
      try {
        const html = readFileSync(resolve(dist, "functions.html"), "utf8");
        const m = /assets\/functions-[^"']+\.js/.exec(html);
        const chunk = m ? `./${m[0]}` : "./assets/functions.js";
        writeFileSync(resolve(dist, "functions.js"), `import "${chunk}";\n`);
      } catch {
        // functions.html absent (partial build) — leave a no-op stub so the URL
        // still resolves rather than 404ing.
        writeFileSync(resolve(dist, "functions.js"), "/* functions page registers the namespace */\n");
      }
    },
  };
}
