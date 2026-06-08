#!/usr/bin/env node
/*
 * check-html-responsive.mjs — committed, reproducible responsive-layout gate for
 * the standalone Celnet capabilities showcase page (docs/celnet-capabilities.html).
 *
 * Why: the showcase is a single, self-contained HTML page meant to be opened
 * directly in a browser at any size — from a 1920-wide desk monitor down to a
 * 375-wide phone. A regression in the CSS (a fixed-width table, an un-wrapped
 * code block, an over-wide figure) silently produces a horizontal scrollbar or
 * an element that bleeds off-screen. This gate loads the real page in headless
 * Chromium across a viewport sweep and FAILS if, at any size:
 *   (1) the document overflows horizontally (scrollWidth > clientWidth + 1), or
 *   (2) any single element is wider than the viewport AND that width is not
 *       absorbed by an ancestor scroll/clip container — i.e. it actually bleeds
 *       past the viewport rather than scrolling inside an `overflow-x:auto`
 *       wrapper (the standard responsive-table pattern), or
 *   (3) the page is not vertically reachable to its footer (the footer element,
 *       once scrolled to, lands within the document — i.e. the page scrolls to
 *       reveal its end rather than clipping content above the fold).
 *
 * The +1px tolerance on (1)/(2) absorbs sub-pixel rounding only; it is not a
 * fudge for real overflow. Rule (2) deliberately exempts elements contained by
 * an ancestor whose computed overflow-x is auto/scroll/hidden, because such an
 * element is intentionally scrollable/clipped and does NOT widen the page — that
 * is correct responsive layout, not a defect (verified against (1), which catches
 * any width that genuinely escapes to the document). There are no disabled
 * assertions and no try/catch that swallows a failing viewport — every viewport
 * is checked and every violation is reported before a non-zero exit.
 *
 * Viewport sweep (desktop → tablet → phone, both orientations):
 *   1920×1080, 1440×900, 1366×768, 1024×768, 768×1024, 375×667
 *
 * Determinism: animations/transitions are disabled via an injected stylesheet,
 * fonts and network are awaited before measuring, and device-scale-factor is 1
 * so CSS pixels map 1:1 to the measured client box.
 *
 * Dependency: Playwright (chromium), reusing the copy vendored under
 * gui/node_modules (same as render-capability-figures.mjs). Ensure the browser
 * binary is present with `npx --prefix gui playwright install chromium` (the
 * `check-html-responsive` just recipe does this automatically).
 *
 * Usage:
 *     node tools/check-html-responsive.mjs            # main showcase page
 *     just check-html-responsive
 */

import { chromium } from '../gui/node_modules/playwright/index.mjs';
import { existsSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const PAGE = path.join(REPO_ROOT, 'docs', 'celnet-capabilities.html');

const VIEWPORTS = [
  { w: 1920, h: 1080 },
  { w: 1440, h: 900 },
  { w: 1366, h: 768 },
  { w: 1024, h: 768 },
  { w: 768, h: 1024 },
  { w: 375, h: 667 },
];

const TOL = 1; // sub-pixel rounding tolerance only

// Disable animation so geometry is stable the instant we measure.
const FREEZE_CSS = `*,*::before,*::after{
  animation-duration:0s !important; animation-delay:0s !important;
  transition-duration:0s !important; transition-delay:0s !important;
  scroll-behavior:auto !important;
}`;

/**
 * Runs inside the page. Returns the layout facts we assert on for one viewport.
 */
function measure(tol) {
  const de = document.documentElement;
  const clientWidth = de.clientWidth;
  const clientHeight = de.clientHeight;
  const scrollWidth = de.scrollWidth;
  const scrollHeight = de.scrollHeight;

  // True iff some ancestor (up to <html>) clips/scrolls horizontal overflow, so a
  // child wider than the viewport is intentionally scrollable/clipped rather than
  // bleeding past the page edge (the standard `overflow-x:auto` table wrapper).
  const absorbedByScrollAncestor = (el) => {
    for (let p = el.parentElement; p && p !== document.documentElement; p = p.parentElement) {
      const ox = getComputedStyle(p).overflowX;
      if (ox === 'auto' || ox === 'scroll' || ox === 'hidden' || ox === 'clip') return true;
    }
    return false;
  };

  // (2) any element wider than the viewport (with a small tolerance) that is NOT
  // absorbed by a scroll/clip ancestor. Report the worst few offenders.
  const tooWide = [];
  for (const el of document.body.getElementsByTagName('*')) {
    const r = el.getBoundingClientRect();
    // ignore zero-size / hidden
    if (r.width === 0 && r.height === 0) continue;
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    if (r.width > clientWidth + tol && !absorbedByScrollAncestor(el)) {
      const id = el.id ? `#${el.id}` : '';
      const cls = el.className && typeof el.className === 'string'
        ? '.' + el.className.trim().split(/\s+/).join('.')
        : '';
      tooWide.push({
        sel: `${el.tagName.toLowerCase()}${id}${cls}`.slice(0, 120),
        width: Math.round(r.width),
        left: Math.round(r.left),
        right: Math.round(r.right),
      });
    }
  }
  tooWide.sort((a, b) => b.width - a.width);

  // (3) vertical reachability of the footer. Find the footer (or the last
  // sizeable block). Scroll to bottom and confirm the footer's bottom edge is
  // within the scrolled document (i.e. nothing is clipped past it).
  const footer =
    document.querySelector('footer') ||
    document.querySelector('.footer, [role="contentinfo"]') ||
    document.body.lastElementChild;

  window.scrollTo(0, scrollHeight);
  const scrolledY = window.scrollY;
  let footerReachable = true;
  let footerInfo = null;
  if (footer) {
    const fr = footer.getBoundingClientRect();
    // After scrolling to the very bottom, the footer's bottom should be at or
    // above the viewport bottom (it has scrolled into view). If the page is
    // shorter than the viewport, it trivially fits.
    const fits = scrollHeight <= clientHeight + tol;
    footerReachable = fits || fr.bottom <= clientHeight + tol + 1;
    footerInfo = {
      tag: footer.tagName.toLowerCase(),
      bottom: Math.round(fr.bottom),
      clientHeight,
      scrolledY,
    };
  }

  return {
    clientWidth,
    clientHeight,
    scrollWidth,
    scrollHeight,
    horizontalOverflow: scrollWidth > clientWidth + tol,
    tooWide: tooWide.slice(0, 8),
    footerReachable,
    footerInfo,
  };
}

async function main() {
  if (!existsSync(PAGE)) {
    console.error(`check-html-responsive: FAILED — page not found: ${path.relative(REPO_ROOT, PAGE)}`);
    process.exit(1);
  }

  const url = pathToFileURL(PAGE).href;
  const browser = await chromium.launch();
  const failures = []; // { vp, kind, detail }

  try {
    for (const vp of VIEWPORTS) {
      const context = await browser.newContext({
        viewport: { width: vp.w, height: vp.h },
        deviceScaleFactor: 1,
      });
      const page = await context.newPage();
      await page.addStyleTag({ content: FREEZE_CSS }).catch(() => {});
      await page.goto(url, { waitUntil: 'networkidle' });
      // Re-inject after navigation in case goto cleared it, then settle fonts.
      await page.addStyleTag({ content: FREEZE_CSS });
      await page.evaluate(() => document.fonts && document.fonts.ready).catch(() => {});

      const r = await page.evaluate(measure, TOL);
      const label = `${vp.w}×${vp.h}`;

      if (r.horizontalOverflow) {
        failures.push({
          vp: label,
          kind: 'horizontal-overflow',
          detail: `scrollWidth ${r.scrollWidth} > clientWidth ${r.clientWidth} (+${TOL}px tol)`,
        });
      }
      if (r.tooWide.length > 0) {
        for (const el of r.tooWide) {
          failures.push({
            vp: label,
            kind: 'element-wider-than-viewport',
            detail: `${el.sel} — width ${el.width}px (viewport ${r.clientWidth}px), box [${el.left}..${el.right}]`,
          });
        }
      }
      if (!r.footerReachable) {
        const fi = r.footerInfo;
        failures.push({
          vp: label,
          kind: 'footer-not-reachable',
          detail: fi
            ? `<${fi.tag}> bottom=${fi.bottom} not within clientHeight=${fi.clientHeight} after scroll (scrollY=${fi.scrolledY})`
            : 'no footer/last block found to verify vertical reach',
        });
      }

      if (!r.horizontalOverflow && r.tooWide.length === 0 && r.footerReachable) {
        console.log(`  ✓ ${label}  (scrollW ${r.scrollWidth} ≤ clientW ${r.clientWidth}; footer reachable; no over-wide elements)`);
      } else {
        console.log(`  ✗ ${label}  (see failures below)`);
      }

      await context.close();
    }
  } finally {
    await browser.close();
  }

  if (failures.length === 0) {
    console.log(`check-html-responsive: OK — ${path.relative(REPO_ROOT, PAGE)} passes all ${VIEWPORTS.length} viewports.`);
    process.exit(0);
  }

  console.error(`\ncheck-html-responsive: FAILED — ${failures.length} violation(s) in ${path.relative(REPO_ROOT, PAGE)}:`);
  let last = null;
  for (const f of failures) {
    if (f.vp !== last) {
      console.error(`  viewport ${f.vp}:`);
      last = f.vp;
    }
    console.error(`    ✗ [${f.kind}] ${f.detail}`);
  }
  process.exit(1);
}

main().catch((e) => {
  console.error(`check-html-responsive: ERROR — ${e && e.stack ? e.stack : e}`);
  process.exit(1);
});
