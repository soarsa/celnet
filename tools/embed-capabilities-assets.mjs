#!/usr/bin/env node
// Produce a fully self-contained, single-file capabilities document by inlining every
// referenced image (the branded figures + GUI screenshots) as a base64 data URI. The
// output (docs/celnet-capabilities.standalone.html) has NO external asset dependency —
// every visual is embedded — so it is portable and is the source for the professional PDF.
//
// The committed docs/celnet-capabilities.html keeps RELATIVE asset refs (git-friendly,
// 272 KB); this generated standalone (~24 MB) is gitignored and rebuilt on demand:
//   just embed-capabilities        (then `just capabilities-pdf` prints it to PDF)
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const srcHtml = resolve(repo, "docs/celnet-capabilities.html");
const outHtml = resolve(repo, "docs/celnet-capabilities.standalone.html");
const assetDir = resolve(repo, "docs");

let html = readFileSync(srcHtml, "utf8");
const seen = new Set();
let embedded = 0;
let missing = [];

// Match src="assets/celnet-capabilities/<name>.png" (single or double quoted).
html = html.replace(
  /(["'])(assets\/celnet-capabilities\/[^"']+\.png)\1/g,
  (match, quote, relPath) => {
    const abs = resolve(assetDir, relPath);
    if (!existsSync(abs)) {
      missing.push(relPath);
      return match;
    }
    if (!seen.has(relPath)) {
      seen.add(relPath);
      embedded++;
    }
    const b64 = readFileSync(abs).toString("base64");
    return `${quote}data:image/png;base64,${b64}${quote}`;
  },
);

// Sanity: no relative png asset refs may remain in the standalone.
const residual = [...html.matchAll(/["']assets\/celnet-capabilities\/[^"']+\.png["']/g)];

writeFileSync(outHtml, html, "utf8");

const bytes = Buffer.byteLength(html, "utf8");
console.log(`embed-capabilities: ${embedded} unique images embedded into ${outHtml}`);
console.log(`  size: ${(bytes / 1024 / 1024).toFixed(1)} MiB`);
if (missing.length) {
  console.error(`  ERROR: ${missing.length} referenced asset(s) missing on disk:`);
  for (const m of missing) console.error(`    - ${m}`);
  process.exit(1);
}
if (residual.length) {
  console.error(`  ERROR: ${residual.length} relative png ref(s) remain un-embedded.`);
  process.exit(1);
}
console.log("  self-contained: OK — no external image dependency remains.");
