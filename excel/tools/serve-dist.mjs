// Dev-only: serve the BUILT add-in (excel/dist) over HTTPS:3000 with the trusted
// office-addin-dev-certs cert, dual-stack. The Excel-for-Mac custom-functions
// runtime is a stripped WKWebView that cannot evaluate the Vite *dev* server's raw
// .ts ES-module graph + /@vite/client HMR client — so the registration script never
// runs. Serving the compiled dist (one hashed .js bundle per page) fixes that.
//
//   npm run build && node excel/tools/serve-dist.mjs   (replaces `npm run dev`)
import { createServer } from "node:https";
import { readFileSync, existsSync, statSync } from "node:fs";
import { resolve, join, extname, normalize } from "node:path";
import { homedir } from "node:os";

const DIST = resolve(process.argv[2] ?? resolve(import.meta.dirname, "..", "dist"));
const PORT = Number(process.env.PORT ?? 3000);
const certDir = resolve(homedir(), ".office-addin-dev-certs");
const MIME = {
  ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8", ".json": "application/json; charset=utf-8",
  ".png": "image/png", ".ico": "image/x-icon", ".svg": "image/svg+xml",
  ".map": "application/json", ".woff2": "font/woff2", ".woff": "font/woff",
};

const server = createServer(
  { cert: readFileSync(join(certDir, "localhost.crt")), key: readFileSync(join(certDir, "localhost.key")) },
  (req, res) => {
    res.setHeader("Access-Control-Allow-Origin", "*");
    res.setHeader("Cache-Control", "no-store");
    let p = decodeURIComponent((req.url || "/").split("?")[0]);
    if (p === "/") p = "/taskpane.html";
    const file = normalize(join(DIST, p));
    if (!file.startsWith(DIST) || !existsSync(file) || !statSync(file).isFile()) {
      res.writeHead(404, { "Content-Type": "text/plain" });
      res.end("not found");
      return;
    }
    res.writeHead(200, { "Content-Type": MIME[extname(file)] ?? "application/octet-stream" });
    res.end(readFileSync(file));
  },
);
// no host => dual-stack (reachable via localhost/::1 and 127.0.0.1)
server.listen(PORT, () => console.log(`serving ${DIST} on https://localhost:${PORT} (dual-stack, static)`));
