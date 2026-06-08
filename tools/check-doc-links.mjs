#!/usr/bin/env node
/*
 * check-doc-links.mjs — committed, reproducible link/anchor/asset integrity checker
 * for the Celnet capabilities showcase.
 *
 * Why: the capabilities showcase is a multi-file corpus — the flagship
 * docs/CELNET-CAPABILITIES.md, the fourteen part files under
 * docs/celnet-capabilities/*.md, and the standalone single-page
 * docs/celnet-capabilities.html — all cross-referencing each other and a shared
 * figure/screenshot asset set. A single renamed file, moved figure, or stale
 * #anchor silently breaks navigation. This checker proves every relative link,
 * href, figure/img source, and in-document #anchor actually resolves, so the
 * showcase is never shipped with a dead reference.
 *
 * What it scans (the showcase navigation surface only — not the whole repo):
 *   - docs/CELNET-CAPABILITIES.md
 *   - docs/celnet-capabilities/*.md   (every part file)
 *   - docs/celnet-capabilities.html
 *
 * What it extracts and asserts per source file:
 *   - Markdown links     [text](target)            — including ![img](target)
 *   - HTML href/src      href="..."  src="..."
 * For each extracted target:
 *   - http(s):// and mailto: and protocol-relative // links are EXTERNAL → skipped
 *     (network reachability is out of scope; this is a local-integrity gate).
 *   - Pure "#anchor" targets must match an id/anchor defined IN THAT SAME document
 *     (HTML: id="..."/name="..."; Markdown: a GitHub-style slug of a heading).
 *   - "path#anchor" targets: the path (resolved relative to the source file's dir)
 *     must exist; if it points at a Markdown or HTML file we ALSO assert the
 *     #anchor exists inside that target file.
 *   - Plain "path" targets must exist on disk (resolved relative to the source dir).
 *
 * Exit code: 0 if every reference resolves; non-zero otherwise, after printing a
 * grouped, per-source list of every broken reference (target + reason). No silent
 * skips beyond the documented EXTERNAL class; no disabled checks.
 *
 * Usage:
 *     node tools/check-doc-links.mjs        # via `just check-docs`
 */

import { readFile } from 'node:fs/promises';
import { existsSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { glob } from 'node:fs/promises';

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const DOCS = path.join(REPO_ROOT, 'docs');

/** Collect the set of source files that make up the showcase navigation surface. */
async function collectSources() {
  const sources = [
    path.join(DOCS, 'CELNET-CAPABILITIES.md'),
    path.join(DOCS, 'celnet-capabilities.html'),
  ];
  for await (const entry of glob('celnet-capabilities/*.md', { cwd: DOCS })) {
    sources.push(path.join(DOCS, entry));
  }
  // Stable, deterministic order.
  return [...new Set(sources)].sort();
}

/** GitHub-style heading slug (lower, strip punctuation, spaces→hyphens). */
function slugify(heading) {
  return heading
    .trim()
    .toLowerCase()
    // strip markdown inline emphasis/code markers
    .replace(/[`*_~]/g, '')
    // drop everything that is not a word char, space, or hyphen
    .replace(/[^\w \-]/g, '')
    .replace(/ /g, '-');
}

/** Extract the set of anchor ids a document defines (for #fragment resolution). */
function extractAnchors(filePath, text) {
  const anchors = new Set();
  if (filePath.endsWith('.html')) {
    for (const m of text.matchAll(/\b(?:id|name)\s*=\s*"([^"]+)"/g)) {
      anchors.add(m[1]);
    }
  } else {
    // Markdown headings → GitHub slugs. Handle duplicate slugs (-1, -2, …).
    const seen = new Map();
    for (const line of text.split('\n')) {
      const h = /^#{1,6}\s+(.*?)\s*#*\s*$/.exec(line);
      if (!h) continue;
      const base = slugify(h[1]);
      if (!base) continue;
      const n = seen.get(base) ?? 0;
      anchors.add(n === 0 ? base : `${base}-${n}`);
      seen.set(base, n + 1);
    }
    // Explicit HTML anchors embedded in markdown (e.g. <a id="x"> / name="x").
    for (const m of text.matchAll(/<a\b[^>]*\b(?:id|name)\s*=\s*"([^"]+)"/g)) {
      anchors.add(m[1]);
    }
  }
  return anchors;
}

/** Extract every reference target from a source file. */
function extractRefs(filePath, text) {
  const refs = [];
  if (filePath.endsWith('.html')) {
    for (const m of text.matchAll(/\b(?:href|src)\s*=\s*"([^"]*)"/g)) {
      refs.push(m[1]);
    }
  } else {
    // Markdown links and images: [text](target "optional title")
    // Image syntax ![..](..) is covered because we match the (target) part.
    for (const m of text.matchAll(/!?\[[^\]]*\]\(\s*([^)\s]+)(?:\s+"[^"]*")?\s*\)/g)) {
      refs.push(m[1]);
    }
    // Inline HTML img/a inside markdown.
    for (const m of text.matchAll(/<(?:img|a)\b[^>]*\b(?:href|src)\s*=\s*"([^"]*)"/g)) {
      refs.push(m[1]);
    }
  }
  return refs;
}

function isExternal(target) {
  return /^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(target);
}

function main() {
  return run();
}

async function run() {
  const sources = await collectSources();

  // Pre-load anchor sets for every doc we may need to resolve fragments against,
  // keyed by absolute path. (Source files plus any md/html they link to.)
  const anchorCache = new Map();
  async function anchorsOf(absPath) {
    if (anchorCache.has(absPath)) return anchorCache.get(absPath);
    let set = null;
    try {
      const t = await readFile(absPath, 'utf8');
      set = extractAnchors(absPath, t);
    } catch {
      set = null; // unreadable / not text — caller treats as "no anchors known"
    }
    anchorCache.set(absPath, set);
    return set;
  }

  const failures = []; // { source, target, reason }

  for (const source of sources) {
    let text;
    try {
      text = await readFile(source, 'utf8');
    } catch (e) {
      failures.push({ source, target: '(file)', reason: `cannot read source: ${e.message}` });
      continue;
    }
    const selfAnchors = extractAnchors(source, text);
    anchorCache.set(source, selfAnchors);
    const dir = path.dirname(source);

    for (const raw of extractRefs(source, text)) {
      const target = raw.trim();
      if (target === '' || isExternal(target)) continue;

      // Pure in-document fragment.
      if (target.startsWith('#')) {
        const anchor = target.slice(1);
        if (anchor === '') continue; // bare "#" — top of page, always valid
        if (!selfAnchors.has(anchor)) {
          failures.push({ source, target, reason: `no matching id/anchor "${anchor}" in this document` });
        }
        continue;
      }

      // Split off any fragment.
      const hashIdx = target.indexOf('#');
      const relPath = hashIdx >= 0 ? target.slice(0, hashIdx) : target;
      const fragment = hashIdx >= 0 ? target.slice(hashIdx + 1) : null;

      const absTarget = path.resolve(dir, decodeURIComponent(relPath));
      if (!existsSync(absTarget)) {
        failures.push({ source, target, reason: `target file does not exist: ${path.relative(REPO_ROOT, absTarget)}` });
        continue;
      }
      // If it's a directory, a bare link to it is fine; a fragment into it is not.
      const st = statSync(absTarget);
      if (st.isDirectory()) {
        if (fragment) {
          failures.push({ source, target, reason: `fragment into a directory: ${path.relative(REPO_ROOT, absTarget)}` });
        }
        continue;
      }

      if (fragment !== null && fragment !== '') {
        if (absTarget.endsWith('.md') || absTarget.endsWith('.html')) {
          const set = await anchorsOf(absTarget);
          if (set && !set.has(fragment)) {
            failures.push({ source, target, reason: `target exists but has no id/anchor "${fragment}"` });
          }
        }
        // Non-text target with a fragment: file exists, fragment unverifiable → accept.
      }
    }
  }

  // Report.
  const totalRefsNote = `scanned ${sources.length} source files`;
  if (failures.length === 0) {
    console.log(`check-doc-links: OK — ${totalRefsNote}; every relative link, asset, and #anchor resolves.`);
    process.exit(0);
  }

  console.error(`check-doc-links: FAILED — ${failures.length} broken reference(s) across ${totalRefsNote}:\n`);
  let lastSource = null;
  for (const f of failures.sort((a, b) => (a.source + a.target).localeCompare(b.source + b.target))) {
    const rel = path.relative(REPO_ROOT, f.source);
    if (rel !== lastSource) {
      console.error(`  ${rel}`);
      lastSource = rel;
    }
    console.error(`    ✗ ${f.target}\n        ${f.reason}`);
  }
  process.exit(1);
}

main();
