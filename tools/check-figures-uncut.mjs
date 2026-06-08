import pkg from '/Users/adrian/code/celeroption/gui/node_modules/playwright-core/index.js';
const { chromium } = pkg;
import { readFile } from 'node:fs/promises';
import path from 'node:path';

const SRC = '/Users/adrian/code/celeroption/docs/assets/celnet-capabilities/_src';
const meta = JSON.parse(await readFile(path.join(SRC, 'diagram-meta.json'), 'utf8'));

const browser = await chromium.launch();
let anyClipped = false;
for (const m of meta) {
  const file = `file://${path.join(SRC, m.figname + '.html')}`;
  const page = await browser.newPage({ viewport: { width: m.w, height: m.h }, deviceScaleFactor: 1 });
  await page.goto(file, { waitUntil: 'networkidle' });
  const r = await page.evaluate(() => {
    const c = document.querySelector('.canvas') || document.body;
    return {
      clientW: c.clientWidth, clientH: c.clientHeight,
      scrollW: c.scrollWidth, scrollH: c.scrollHeight,
    };
  });
  const overW = r.scrollW - r.clientW;
  const overH = r.scrollH - r.clientH;
  const clipped = overW > 2 || overH > 2;
  if (clipped) anyClipped = true;
  console.log(`${clipped ? 'CLIP' : 'FIT '} ${m.figname}  meta=${m.w}x${m.h} canvas client=${r.clientW}x${r.clientH} scroll=${r.scrollW}x${r.scrollH} over=(${overW},${overH})`);
  await page.close();
}
await browser.close();
console.log(anyClipped ? '\nRESULT: SOME FIGURES CLIPPED' : '\nRESULT: ALL FIGURES FIT');
process.exit(anyClipped ? 1 : 0);
