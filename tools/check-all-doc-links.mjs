#!/usr/bin/env node
/*
 * check-all-doc-links.mjs — link/anchor/asset + structure integrity checker over
 * ALL docs (docs/**\/*.md + docs/*.html). Generalizes check-doc-links.mjs.
 *
 * Checks per source file:
 *  - relative links / hrefs / img-src resolve on disk
 *  - path#anchor: path exists AND (if md/html) the anchor exists in the target
 *  - pure #anchor: matches a heading-slug / id in the same doc
 *  - structure: balanced ``` code fences, monotonic-ish heading levels (no jumps
 *    of >1 increasing), well-formed pipe tables (consistent column counts).
 *
 * External (http(s)/mailto/protocol-relative) links are skipped (out of scope).
 */
import { readFile } from 'node:fs/promises';
import { existsSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { glob } from 'node:fs/promises';

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const DOCS = path.join(REPO_ROOT, 'docs');

async function collectSources() {
  const sources = [];
  for await (const e of glob('**/*.md', { cwd: DOCS })) sources.push(path.join(DOCS, e));
  for await (const e of glob('*.html', { cwd: DOCS })) sources.push(path.join(DOCS, e));
  return [...new Set(sources)].sort();
}

function slugify(h) {
  return h.trim().toLowerCase().replace(/[`*_~]/g, '').replace(/[^\w \-]/g, '').replace(/ /g, '-');
}

function extractAnchors(filePath, text) {
  const anchors = new Set();
  if (filePath.endsWith('.html')) {
    for (const m of text.matchAll(/\b(?:id|name)\s*=\s*"([^"]+)"/g)) anchors.add(m[1]);
  } else {
    const seen = new Map();
    let inFence = false;
    for (const line of text.split('\n')) {
      if (/^\s*```/.test(line)) { inFence = !inFence; continue; }
      if (inFence) continue;
      const h = /^#{1,6}\s+(.*?)\s*#*\s*$/.exec(line);
      if (!h) continue;
      const base = slugify(h[1]);
      if (!base) continue;
      const n = seen.get(base) ?? 0;
      anchors.add(n === 0 ? base : `${base}-${n}`);
      seen.set(base, n + 1);
    }
    for (const m of text.matchAll(/<a\b[^>]*\b(?:id|name)\s*=\s*"([^"]+)"/g)) anchors.add(m[1]);
    for (const m of text.matchAll(/\b(?:id|name)\s*=\s*"([^"]+)"/g)) anchors.add(m[1]);
  }
  return anchors;
}

function extractRefs(filePath, text) {
  const refs = [];
  if (filePath.endsWith('.html')) {
    for (const m of text.matchAll(/\b(?:href|src)\s*=\s*"([^"]*)"/g)) refs.push(m[1]);
  } else {
    // strip fenced code so we don't treat code as links
    let stripped = text.replace(/```[\s\S]*?```/g, '');
    for (const m of stripped.matchAll(/!?\[[^\]]*\]\(\s*([^)\s]+)(?:\s+"[^"]*")?\s*\)/g)) refs.push(m[1]);
    for (const m of stripped.matchAll(/<(?:img|a)\b[^>]*\b(?:href|src)\s*=\s*"([^"]*)"/g)) refs.push(m[1]);
  }
  return refs;
}

const isExternal = (t) => /^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(t);

function structureLint(filePath, text) {
  const issues = [];
  if (filePath.endsWith('.html')) return issues;
  const lines = text.split('\n');
  // fences
  let fenceCount = 0;
  for (const line of lines) if (/^\s*```/.test(line)) fenceCount++;
  if (fenceCount % 2 !== 0) issues.push(`unbalanced code fences (${fenceCount} \`\`\` markers)`);
  // headings + tables, skipping inside fences
  let inFence = false;
  let prevLevel = 0;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (/^\s*```/.test(line)) { inFence = !inFence; continue; }
    if (inFence) continue;
    const hm = /^(#{1,6})\s+\S/.exec(line);
    if (hm) {
      const lvl = hm[1].length;
      if (prevLevel && lvl > prevLevel + 1) {
        issues.push(`heading level jump (h${prevLevel} -> h${lvl}) at line ${i + 1}`);
      }
      prevLevel = lvl;
    }
  }
  // tables: contiguous blocks of lines starting with | — check column consistency
  inFence = false;
  let tbl = [];
  let tblStart = 0;
  const flush = () => {
    if (tbl.length >= 2) {
      const cols = tbl.map((l) => l.split('|').length);
      // header + separator + rows should all share the same pipe count
      const bad = cols.some((c) => c !== cols[0]);
      if (bad) issues.push(`ragged table column counts near line ${tblStart} (${cols.join(',')})`);
    }
    tbl = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (/^\s*```/.test(line)) { inFence = !inFence; flush(); continue; }
    if (inFence) { continue; }
    if (/^\s*\|.*\|\s*$/.test(line)) {
      if (tbl.length === 0) tblStart = i + 1;
      tbl.push(line.trim());
    } else flush();
  }
  flush();
  return issues;
}

async function run() {
  const sources = await collectSources();
  const anchorCache = new Map();
  async function anchorsOf(absPath) {
    if (anchorCache.has(absPath)) return anchorCache.get(absPath);
    let set = null;
    try { set = extractAnchors(absPath, await readFile(absPath, 'utf8')); } catch { set = null; }
    anchorCache.set(absPath, set);
    return set;
  }
  const failures = [];
  const structIssues = [];
  for (const source of sources) {
    let text;
    try { text = await readFile(source, 'utf8'); }
    catch (e) { failures.push({ source, target: '(file)', reason: `cannot read: ${e.message}` }); continue; }
    const selfAnchors = extractAnchors(source, text);
    anchorCache.set(source, selfAnchors);
    const dir = path.dirname(source);
    for (const issue of structureLint(source, text)) structIssues.push({ source, issue });
    for (const raw of extractRefs(source, text)) {
      const target = raw.trim();
      if (target === '' || isExternal(target)) continue;
      // Skip template/placeholder paths (e.g. `<fig>.png`, `{name}.md`) — these
      // are illustrative path templates inside docs, not real links.
      if (/[<>{}]/.test(target)) continue;
      if (target.startsWith('#')) {
        const anchor = target.slice(1);
        if (anchor === '') continue;
        if (!selfAnchors.has(anchor)) failures.push({ source, target, reason: `no anchor "${anchor}" in doc` });
        continue;
      }
      const hashIdx = target.indexOf('#');
      const relPath = hashIdx >= 0 ? target.slice(0, hashIdx) : target;
      const fragment = hashIdx >= 0 ? target.slice(hashIdx + 1) : null;
      if (relPath === '') continue;
      const absTarget = path.resolve(dir, decodeURIComponent(relPath));
      if (!existsSync(absTarget)) {
        failures.push({ source, target, reason: `missing: ${path.relative(REPO_ROOT, absTarget)}` });
        continue;
      }
      const st = statSync(absTarget);
      if (st.isDirectory()) {
        if (fragment) failures.push({ source, target, reason: `fragment into dir` });
        continue;
      }
      if (fragment && (absTarget.endsWith('.md') || absTarget.endsWith('.html'))) {
        const set = await anchorsOf(absTarget);
        if (set && !set.has(fragment)) failures.push({ source, target, reason: `target lacks anchor "${fragment}"` });
      }
    }
  }
  const note = `scanned ${sources.length} doc files`;
  let ok = true;
  if (structIssues.length) {
    ok = false;
    console.error(`STRUCTURE: ${structIssues.length} issue(s):`);
    for (const s of structIssues) console.error(`  ${path.relative(REPO_ROOT, s.source)}: ${s.issue}`);
  } else console.log(`STRUCTURE: OK — balanced fences, well-formed tables, monotonic headings (${note}).`);
  if (failures.length) {
    ok = false;
    console.error(`\nLINKS: FAILED — ${failures.length} broken reference(s):`);
    let last = null;
    for (const f of failures.sort((a, b) => (a.source + a.target).localeCompare(b.source + b.target))) {
      const rel = path.relative(REPO_ROOT, f.source);
      if (rel !== last) { console.error(`  ${rel}`); last = rel; }
      console.error(`    x ${f.target}  (${f.reason})`);
    }
  } else console.log(`LINKS: OK — every relative link, asset, and #anchor resolves (${note}).`);
  process.exit(ok ? 0 : 1);
}
run();
