#!/usr/bin/env node
// Render the self-contained capabilities document to a professionally-styled PDF via
// headless Chromium (Playwright), printing off the document's @media print stylesheet
// (per-section page breaks, figure break-inside:avoid, TOC hidden in print).
//
//   just capabilities-pdf      (embeds assets → standalone → this PDF)
//
// Reuses the Playwright vendored under gui/node_modules (no repo-root install).
import { chromium } from "../gui/node_modules/playwright/index.mjs";
import { statSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const src = resolve(repo, "docs/celnet-capabilities.standalone.html");
const out = resolve(repo, "docs/celnet-capabilities.pdf");

if (!statSync(src, { throwIfNoEntry: false })) {
  console.error(`missing ${src} — run \`just embed-capabilities\` first`);
  process.exit(1);
}

const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  // Print media so the @media print rules drive layout; generous timeout for the
  // ~34 MiB self-contained document (30 embedded images).
  await page.emulateMedia({ media: "print" });
  await page.goto(pathToFileURL(src).href, { waitUntil: "load", timeout: 180_000 });
  // Let any web-fonts settle for crisp typography.
  await page.waitForTimeout(1500);
  await page.pdf({
    path: out,
    format: "A4",
    printBackground: true,
    preferCSSPageSize: true,
    displayHeaderFooter: false,
    margin: { top: "14mm", bottom: "16mm", left: "12mm", right: "12mm" },
  });
} finally {
  await browser.close();
}

const bytes = statSync(out).size;
console.log(`capabilities PDF: ${out}  (${(bytes / 1024 / 1024).toFixed(1)} MiB)`);
