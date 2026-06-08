#!/usr/bin/env node
/*
 * render-capability-figures.mjs — committed, reproducible capability-figure renderer.
 *
 * Closes the guardrail-#10 reproducibility gap: previously the capability-doc PNGs
 * were rendered ad-hoc (hand-driven browser screenshots), so there was no committed,
 * one-command way to regenerate them deterministically from their re-authored HTML.
 * This script is that single source of truth.
 *
 * What it does:
 *   1. Reads  docs/assets/celnet-capabilities/_src/diagram-meta.json  (the figure
 *      registry: { figname, w, h, caption, alt } per figure).
 *   2. For each figure, opens  file://.../_src/<figname>.html  in headless Chromium,
 *      sets the viewport to that figure's exact w×h, and screenshots the single
 *      `.canvas` element to  docs/assets/celnet-capabilities/<figname>.png.
 *   3. Also renders  excel-grid-branded.html  (root element `.app`, fixed 1680×1000)
 *      to  shot-10-excel-grid-branded.png  — a known fixed-size figure that is part of
 *      the capability showcase but lives outside diagram-meta.json.
 *
 * Determinism: device-scale-factor is fixed at 2 (crisp 2× raster), animations are
 * disabled, and we wait for fonts + network idle before capturing, so re-runs on the
 * same toolchain produce stable output.
 *
 * Dependency: Playwright (chromium). This script reuses the Playwright already
 * vendored under  gui/node_modules  (gui/package.json devDependency, currently
 * playwright 1.60.0) — no extra install in the repo root. The Chromium browser
 * binary must be present in the Playwright cache; if missing, run:
 *     npx --prefix gui playwright install chromium
 * The `render-figures` just recipe runs that install step automatically.
 *
 * Usage:
 *     node tools/render-capability-figures.mjs            # render all figures
 *     node tools/render-capability-figures.mjs fig-04-... # render one figure by name
 *     just render-figures                                  # via the justfile recipe
 */

import { chromium } from '../gui/node_modules/playwright/index.mjs';
import { readFile } from 'node:fs/promises';
import { statSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, join, resolve } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(__dirname, '..');
const ASSETS_DIR = join(REPO_ROOT, 'docs', 'assets', 'celnet-capabilities');
const SRC_DIR = join(ASSETS_DIR, '_src');
const META_PATH = join(SRC_DIR, 'diagram-meta.json');

// Figures that are part of the showcase but not in diagram-meta.json: { html, out, w, h, selector }.
const EXTRA_FIGURES = [
  {
    html: 'excel-grid-branded.html',
    out: 'shot-10-excel-grid-branded.png',
    w: 1680,
    h: 1000,
    selector: '.app',
  },
];

const DEVICE_SCALE_FACTOR = 2;

async function renderOne(page, { htmlFile, outFile, w, h, selector }) {
  const url = pathToFileURL(join(SRC_DIR, htmlFile)).href;
  await page.setViewportSize({ width: w, height: h });
  await page.goto(url, { waitUntil: 'networkidle' });
  // Ensure web fonts are fully loaded so text metrics match the authored layout.
  await page.evaluate(() => document.fonts && document.fonts.ready);
  const el = await page.$(selector);
  if (!el) {
    throw new Error(`selector "${selector}" not found in ${htmlFile}`);
  }
  const box = await el.boundingBox();
  if (!box || box.width < 1 || box.height < 1) {
    throw new Error(`element "${selector}" in ${htmlFile} has no visible box`);
  }
  const outPath = join(ASSETS_DIR, outFile);
  await el.screenshot({ path: outPath });
  const bytes = statSync(outPath).size;
  if (bytes <= 0) {
    throw new Error(`rendered ${outFile} is empty (0 bytes)`);
  }
  return { outFile, w, h, bytes, boxW: Math.round(box.width), boxH: Math.round(box.height) };
}

async function main() {
  const only = process.argv[2]; // optional figname filter

  const meta = JSON.parse(await readFile(META_PATH, 'utf8'));
  if (!Array.isArray(meta) || meta.length === 0) {
    throw new Error(`diagram-meta.json is empty or not an array: ${META_PATH}`);
  }

  // Build the work list from meta (selector `.canvas`) + the extra fixed-size figures.
  let work = meta.map((m) => ({
    htmlFile: `${m.figname}.html`,
    outFile: `${m.figname}.png`,
    w: m.w,
    h: m.h,
    selector: '.canvas',
  }));
  for (const e of EXTRA_FIGURES) {
    work.push({ htmlFile: e.html, outFile: e.out, w: e.w, h: e.h, selector: e.selector });
  }

  if (only) {
    work = work.filter(
      (x) => x.htmlFile === `${only}.html` || x.htmlFile === only || x.outFile === only,
    );
    if (work.length === 0) {
      throw new Error(`no figure matched filter "${only}"`);
    }
  }

  const browser = await chromium.launch({ args: ['--force-color-profile=srgb'] });
  const context = await browser.newContext({ deviceScaleFactor: DEVICE_SCALE_FACTOR });
  const page = await context.newPage();

  const results = [];
  try {
    for (const job of work) {
      const r = await renderOne(page, job);
      results.push(r);
      console.log(
        `  rendered ${r.outFile.padEnd(44)} ${String(r.w).padStart(4)}×${String(r.h).padEnd(4)}  ` +
          `${(r.bytes / 1024).toFixed(0)} KiB  (canvas box ${r.boxW}×${r.boxH})`,
      );
    }
  } finally {
    await browser.close();
  }

  console.log(`\nrendered ${results.length} figure(s) to ${ASSETS_DIR}`);
}

main().catch((err) => {
  console.error('render-capability-figures FAILED:', err.message);
  process.exit(1);
});
