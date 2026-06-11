#!/usr/bin/env node
/*
 * check-verification-coverage.mjs — the executable proto-arm ⇄ golden-vector ⇄
 * parity-row coverage lint for the Celnet verification contract.
 *
 * Why: `docs/VERIFICATION-CONTRACT.md` makes the per-asset-class gate set written
 * and enforceable instead of tribal. Two of its mandatory gates are machine-
 * checkable for EVERY product family in the one unversioned contract:
 *
 *   (c) a cross-client golden VECTOR exists for the family
 *       (`crates/celnet-golden/vectors/<family>.json`), and
 *   (b)/(a) an INDEPENDENT-oracle parity ROW exists for the family
 *       (a `#[test]` in `crates/celnet-parity/tests/` that drives the family's
 *       pricer against a model-disjoint oracle).
 *
 * This lint is the single source of truth that no product arm can be added to
 * `celnet.proto`'s `Instrument.product` oneof and silently ship without BOTH. It
 * is the gate referenced by MASTER-EVOLUTION-PROGRAM.md §4 (api-first parity gate)
 * / §5 W0 / §6 lens 3 and the [W0] verify/verification-contract-doc backlog item.
 *
 * What it does (REAL — parses the proto and checks disk, no hard-coded arm list):
 *   1. Reads `crates/celnet-proto/proto/celnet.proto`, locates the
 *      `oneof product { ... }` block inside `message Instrument`, and extracts
 *      every arm's snake_case field name (the canonical family key). The golden
 *      corpus's own `FAMILIES` constant and `vectors/*.json` `"family"` tags use
 *      exactly these names, so the proto is the authoritative source.
 *   2. For each family arm, asserts:
 *        (i)  a golden vector file `vectors/<family>.json` exists, is non-empty
 *             JSON, and every record's `"family"` tag matches; and
 *        (ii) a celnet-parity row exists — the curated family→test-file map below
 *             points at the authoritative parity test, and the lint verifies that
 *             file is on disk, contains at least one `#[test]`, and references the
 *             family (so a renamed/emptied test file is caught, not waved through).
 *   3. THIRD pass — the CLIENT axis (the Round-2 cross-asset finding: a family
 *      can carry a vector + a parity row yet ship with NO client able to price
 *      it). For every product arm AND every cross-asset option family, each
 *      client exposure manifest must account for it: the family is either
 *      EXPOSED by that client's conformance suite, or EXPLICITLY declared
 *      not-exposed WITH a written reason. The manifests are the client suites'
 *      own greppable declarations (parsed like the proto — no duplicate list
 *      maintained here):
 *        - GUI:   `gui/test/conformance.test.ts` — `FAMILIES_COVERED` /
 *                 `FAMILIES_NOT_EXPOSED_BY_GUI` (whose doc block names every
 *                 declared family with its concrete reason);
 *        - Excel: `excel/e2e/corpus.ts` — `EXCEL_FAMILIES` / the suite's own
 *                 derived `FAMILIES_NOT_EXPOSED = ALL_FAMILIES \ EXCEL_FAMILIES`
 *                 (re-derived here from the two literals; the rationale comment
 *                 heads each not-exposed run inside `ALL_FAMILIES`).
 *      Honest declarations pass; SILENT absence (a family in neither the exposed
 *      nor the declared-not-exposed list) fails the lint naming the family and
 *      the client. Like the curated parity map, the reason check is a tripwire
 *      against silent gaps, not a semantic proof — the client suites themselves
 *      enforce that exposed families really price/round-trip.
 *   4. Exits non-zero listing every arm missing a vector, a parity row, and/or
 *      an honest client-exposure declaration.
 *
 * The family→parity-file map is CURATED (not a fuzzy keyword search) because a
 * single parity file legitimately covers several arms (e.g. `exotics.rs` gates
 * the four first-generation barrier/digital/touch arms; `structured.rs` gates
 * the quanto/tarf/accumulator/lookback arms) while the word "vanilla" appears in
 * nearly every test. A curated map cannot be fooled by an incidental mention.
 * Adding a new product arm therefore REQUIRES a deliberate one-line map entry
 * pointing at its real parity row — which is exactly the contract this enforces.
 *
 * HARD RULE (no weakening): if an arm genuinely has no parity row, this lint
 * FAILS and names it. It must never be made to pass by deleting the requirement.
 *
 * Usage:
 *     node tools/check-verification-coverage.mjs        # via `just verification-coverage`
 */

import { readFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const PROTO = path.join(REPO_ROOT, 'crates/celnet-proto/proto/celnet.proto');
const VECTORS_DIR = path.join(REPO_ROOT, 'crates/celnet-golden/vectors');
const PARITY_TESTS_DIR = path.join(REPO_ROOT, 'crates/celnet-parity/tests');
/* The per-client exposure manifests (third pass — the CLIENT axis). */
const GUI_CONFORMANCE = 'gui/test/conformance.test.ts';
const EXCEL_CORPUS = 'excel/e2e/corpus.ts';

/*
 * Curated family → authoritative celnet-parity test file map.
 *
 * Each value is the test file in `crates/celnet-parity/tests/` whose `#[test]`
 * rows drive THIS family's pricer against an independent, model-disjoint oracle
 * (per docs/VERIFICATION-CONTRACT.md). Several arms share one file by design —
 * that is fine; the lint asserts the file exists, has tests, and names the family.
 *
 * To add a NEW product arm: add its proto oneof field, its golden vector, its
 * parity row, then ONE entry here pointing at that row. The lint then keeps it
 * honest forever. There is intentionally NO catch-all — an unmapped arm fails.
 */
const FAMILY_TO_PARITY_FILE = {
  vanilla: 'greeks.rs', //  GK price + full 13-Greek FD validation + put-call parity (oracle: QuantLib golden + central FD)
  strategy: 'strategy.rs', //  multi-leg composition vs model-free put-call parity / ATM-fwd symmetry / butterfly convexity + independent GK leg-sum
  american: 'american.rs', //  PSOR free-boundary FD vs no-carry European limit + published Longstaff-Schwartz 2001 Table-1 + premium≥0
  single_barrier: 'exotics.rs', //  all-8 single-barrier flavours vs QuantLib 1.42.1 reference values
  double_barrier: 'exotics.rs', //  double knock-out/knock-in vs QuantLib + reflection-series cross-check
  digital: 'exotics.rs', //  cash-/asset-or-nothing digitals vs QuantLib
  touch: 'exotics.rs', //  one-/no-/double-no-/double-touch vs QuantLib
  variance_swap: 'var_vol_swap.rs', //  log-contract replication vs independent adaptive-Simpson + flat-σ K_var==σ² limit
  volatility_swap: 'var_vol_swap.rs', //  Carr-Lee convexity adjustment, strict K_vol<√K_var
  asian_option: 'asian.rs', //  Turnbull-Wakeman / Curran vs independent MC + closed-form limits
  forward_start: 'forward_start.rs', //  Rubinstein closed form vs two-leg-GBM MC + t1→0→GK limit
  cliquet: 'forward_start.rs', //  plain cliquet == Σ forward-start legs; clamped vs independent clamped MC
  quanto: 'structured.rs', //  quanto vanilla/digital closed form vs independent MC + ρ=0→plain limit
  tarf: 'structured.rs', //  TARF gap-risk decomposition (FullGain > CappedGain, expected overshoot)
  pivot: 'pivot_wire.rs', //  pivot-TRA engine vs code-disjoint splitmix64 indicator-form MC oracle + the P==K→TARF degeneracy law (bitwise engine pair + disjoint golden codings) + two-route gearing monotonicity
  accumulator: 'structured.rs', //  continuous vs discrete knock-out monitoring correctness
  lookback: 'structured.rs', //  floating-/fixed-strike closed form vs MC + lookback-dominates-vanilla
  window_barrier: 'lsv.rs', //  window knock-out under LOCAL_STOCH_VOL (ξ=0→Dupire limit, PDE≈MC)
  basket: 'basket.rs', //  Cholesky multi-asset GBM MC vs Levy-1992 hand-pinned + structural sandwich
  fx_forward: 'linear.rs', //  outright forward PV vs independent two-zero-coupon-bond DCF + fair-fwd⇒PV0 / linearity / netting / t→0 intrinsic + hand-pinned literal
  fx_swap: 'linear.rs', //  swap PV == independent sum of two outright forwards + CIP swap-points identity + equal-dates-net-0 / carry-sign structural gates
  ndf: 'linear.rs', //  NDF PV hand-derived literal + NDF==deliverable-forward-PV identity + independent two-bond DCF + fixing-is-metadata-only honesty gate
  perpetual_option: 'perpetual.rs', //  engine vs independent product-form-bisection+libm::pow oracle + american_fd T→∞ sandwich (FD(50y)<FD(100y)<perpetual, converging) + the b==r call==spot exact law / b>r call typed-refusal (both routes) + perpetual≥European(T) dominance law
  listed_future_option: 'listed_future.rs', //  CPython-recomputed pinned literals (Haug 1.7011 / Hull 1.12 + full precision) + undiscounted parity C−P==F−K (bitwise at ATM anchors) + equity==df·futures-style bitwise + vs independent libm::erf oracle (both marginings)
};

/*
 * Curated cross-asset OPTION family → parity file map.
 *
 * The `vanilla` product arm priced through a NON-FX `Underlying.ref` arm (equity /
 * commodity / digital-asset) is a distinct ASSET-CLASS family of the one contract —
 * a different pricing engine on the cost-of-carry seam (ADR-0008), NOT a new
 * `oneof product` arm. The verification contract still requires BOTH a golden vector
 * and an independent-oracle parity row for each, so this lint enforces them as a
 * SECOND, proto-driven pass keyed off the `Underlying.ref` oneof (below), exactly
 * like the product-arm pass — no weakening, just a second axis the contract demands.
 *
 * The KEYS are the `<assetClass>_option` family names; the proto `Underlying.ref`
 * arm field names they are DERIVED from are listed in CROSS_ASSET_REF_TO_FAMILY so
 * the set stays driven by the proto (a new non-FX underlying arm forces a new entry
 * here, or the lint fails — it can never be silently bypassed).
 */
const CROSS_ASSET_FAMILY_TO_PARITY_FILE = {
  equity_option: 'crossasset.rs', //  generalized-BSM (b=r−q−repo) vs independent libm::erf oracle + put-call parity + q=0 standard-BSM limit + central-FD delta/vega/dividend-rho
  commodity_option: 'crossasset.rs', //  Black-76 (future b=0 / spot b=r−convenience) vs independent libm::erf oracle on the forward + parity + b=0 flat-forward limit + central-FD greeks
  crypto_option: 'crossasset.rs', //  linear funded-BSM + inverse coin-margined 1/S_T closed form vs independent libm::erf oracle + parity + b=0 limit + signed convexity sandwich + central-FD greeks
};

/*
 * Map from the proto `Underlying.ref` oneof arm field name → the `<assetClass>_option`
 * family key. FX (and the metal arm, which projects byte-identically onto a CcyPair
 * and is priced by the SAME FX engine) are excluded: they ARE the FX `vanilla`
 * product family already covered by the product-arm pass. Every OTHER `Underlying.ref`
 * arm is a distinct asset-class option family that MUST carry its own vector + row.
 *
 * To add a NEW cross-asset underlying: add its `Underlying.ref` arm in the proto,
 * its `<asset>_option.json` vector, its parity row, then ONE entry here and ONE in
 * CROSS_ASSET_FAMILY_TO_PARITY_FILE. The lint then keeps it honest forever.
 */
const CROSS_ASSET_REF_TO_FAMILY = {
  equity: 'equity_option',
  commodity: 'commodity_option',
  digital_asset: 'crypto_option',
};

/* The `Underlying.ref` arms that are FX-engine-equivalent (NOT a separate family). */
const FX_EQUIVALENT_REFS = new Set(['fx', 'metal']);

/**
 * Parse the `oneof product { ... }` block inside `message Instrument` and return
 * the ordered list of arm field names (snake_case = family key).
 */
function extractProductArms(protoText) {
  // Locate `message Instrument { ... }` (brace-matched) so we never pick up a
  // oneof from another message (e.g. Owner.seat, Scenario axes).
  const msgIdx = protoText.search(/\bmessage\s+Instrument\s*\{/);
  if (msgIdx < 0) throw new Error('could not find `message Instrument` in celnet.proto');
  let depth = 0;
  let bodyStart = -1;
  let bodyEnd = -1;
  for (let i = protoText.indexOf('{', msgIdx); i < protoText.length; i++) {
    const ch = protoText[i];
    if (ch === '{') {
      if (depth === 0) bodyStart = i + 1;
      depth++;
    } else if (ch === '}') {
      depth--;
      if (depth === 0) {
        bodyEnd = i;
        break;
      }
    }
  }
  if (bodyStart < 0 || bodyEnd < 0) throw new Error('unbalanced braces in `message Instrument`');
  const body = protoText.slice(bodyStart, bodyEnd);

  // Find the `oneof product { ... }` inside the message body (brace-matched).
  const oneofIdx = body.search(/\boneof\s+product\s*\{/);
  if (oneofIdx < 0) throw new Error('could not find `oneof product` inside `message Instrument`');
  let d = 0;
  let start = -1;
  let end = -1;
  for (let i = body.indexOf('{', oneofIdx); i < body.length; i++) {
    const ch = body[i];
    if (ch === '{') {
      if (d === 0) start = i + 1;
      d++;
    } else if (ch === '}') {
      d--;
      if (d === 0) {
        end = i;
        break;
      }
    }
  }
  if (start < 0 || end < 0) throw new Error('unbalanced braces in `oneof product`');
  const oneofBody = body.slice(start, end);

  // Strip comments so a commented-out field is never counted.
  const noBlockComments = oneofBody.replace(/\/\*[\s\S]*?\*\//g, '');
  const arms = [];
  for (const rawLine of noBlockComments.split('\n')) {
    const line = rawLine.replace(/\/\/.*$/, '').trim();
    if (line === '') continue;
    // Field form:  <Type> <field_name> = <number>;
    const m = /^[A-Za-z_][A-Za-z0-9_.]*\s+([a-z][a-z0-9_]*)\s*=\s*\d+\s*;/.exec(line);
    if (m) arms.push(m[1]);
  }
  if (arms.length === 0) throw new Error('parsed zero arms from `oneof product` — parser/proto drift');
  return arms;
}

/**
 * Parse the `oneof ref { ... }` block inside `message Underlying` and return the
 * ordered list of arm field names (the asset-class discriminators). Brace-matched,
 * comment-stripped — the same robust parse `extractProductArms` uses.
 */
function extractUnderlyingRefArms(protoText) {
  const msgIdx = protoText.search(/\bmessage\s+Underlying\s*\{/);
  if (msgIdx < 0) throw new Error('could not find `message Underlying` in celnet.proto');
  let depth = 0;
  let bodyStart = -1;
  let bodyEnd = -1;
  for (let i = protoText.indexOf('{', msgIdx); i < protoText.length; i++) {
    const ch = protoText[i];
    if (ch === '{') {
      if (depth === 0) bodyStart = i + 1;
      depth++;
    } else if (ch === '}') {
      depth--;
      if (depth === 0) {
        bodyEnd = i;
        break;
      }
    }
  }
  if (bodyStart < 0 || bodyEnd < 0) throw new Error('unbalanced braces in `message Underlying`');
  const body = protoText.slice(bodyStart, bodyEnd);

  const oneofIdx = body.search(/\boneof\s+ref\s*\{/);
  if (oneofIdx < 0) throw new Error('could not find `oneof ref` inside `message Underlying`');
  let d = 0;
  let start = -1;
  let end = -1;
  for (let i = body.indexOf('{', oneofIdx); i < body.length; i++) {
    const ch = body[i];
    if (ch === '{') {
      if (d === 0) start = i + 1;
      d++;
    } else if (ch === '}') {
      d--;
      if (d === 0) {
        end = i;
        break;
      }
    }
  }
  if (start < 0 || end < 0) throw new Error('unbalanced braces in `oneof ref`');
  const noBlockComments = body.slice(start, end).replace(/\/\*[\s\S]*?\*\//g, '');
  const arms = [];
  for (const rawLine of noBlockComments.split('\n')) {
    const line = rawLine.replace(/\/\/.*$/, '').trim();
    if (line === '') continue;
    const m = /^[A-Za-z_][A-Za-z0-9_.]*\s+([a-z][a-z0-9_]*)\s*=\s*\d+\s*;/.exec(line);
    if (m) arms.push(m[1]);
  }
  if (arms.length === 0) throw new Error('parsed zero arms from `oneof ref` — parser/proto drift');
  return arms;
}

/**
 * Extract the string items of a `const <name> = [ ... ]` literal from a TS
 * client-suite source. Bracket-matched; comments are stripped before the item
 * parse (a commented-out entry is never counted — same discipline as the proto
 * parse). The RAW literal body (comments intact) is also returned so the
 * caller can verify a not-exposed entry carries an adjacent rationale comment.
 */
function extractStringArray(sourceText, constName, fileLabel) {
  const declRe = new RegExp(`\\bconst\\s+${constName}\\b[^=;]*=\\s*\\[`);
  const m = declRe.exec(sourceText);
  if (!m) throw new Error(`could not find \`const ${constName} = [...]\` in ${fileLabel}`);
  const open = m.index + m[0].length - 1;
  let depth = 0;
  let end = -1;
  for (let i = open; i < sourceText.length; i++) {
    const ch = sourceText[i];
    if (ch === '[') depth++;
    else if (ch === ']') {
      depth--;
      if (depth === 0) {
        end = i;
        break;
      }
    }
  }
  if (end < 0) throw new Error(`unbalanced brackets in \`${constName}\` (${fileLabel})`);
  const rawBody = sourceText.slice(open + 1, end);
  const noComments = rawBody.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/[^\n]*/g, '');
  const items = [...noComments.matchAll(/["']([a-z][a-z0-9_]*)["']/g)].map((mm) => mm[1]);
  if (items.length === 0) {
    throw new Error(`parsed zero entries from \`${constName}\` (${fileLabel}) — manifest/parser drift`);
  }
  return { items, rawBody };
}

/**
 * The contiguous comment block (JSDoc and/or `//` lines) immediately preceding
 * the declaration of `constName` — the manifest's REASON text for an explicit
 * not-exposed list (e.g. the GUI doc block naming every family with its reason).
 */
function leadingCommentOf(sourceText, constName, fileLabel) {
  const declRe = new RegExp(`^[^\\S\\n]*(?:export\\s+)?const\\s+${constName}\\b`, 'm');
  const m = declRe.exec(sourceText);
  if (!m) throw new Error(`could not find \`const ${constName}\` in ${fileLabel}`);
  const lines = sourceText.slice(0, m.index).split('\n');
  const collected = [];
  for (let i = lines.length - 1; i >= 0; i--) {
    const t = lines[i].trim();
    if (t === '') {
      if (collected.length === 0) continue; // blank gap between comment and decl
      break;
    }
    if (t.startsWith('//') || t.startsWith('*') || t.startsWith('/*')) {
      collected.push(lines[i]);
      if (t.startsWith('/*')) break; // reached the head of the block comment
      continue;
    }
    break;
  }
  return collected.reverse().join('\n');
}

/**
 * TRUE when `family`'s entry inside a raw manifest array literal carries an
 * adjacent rationale comment: either a trailing `//` comment on its own line,
 * or — scanning upward, skipping blank lines and other not-exposed entries of
 * the same run — a comment line heading the run (the Excel `ALL_FAMILIES`
 * shape, where one rationale block heads the contiguous not-exposed entries).
 * Hitting an exposed entry or the literal start without a comment ⇒ false.
 */
function notExposedRunHasReasonComment(rawBody, family, notExposedSet) {
  const lines = rawBody.split('\n');
  const entryRe = new RegExp(`["']${family}["']`);
  const idx = lines.findIndex((l) => entryRe.test(l.replace(/\/\/.*$/, '')));
  if (idx < 0) return false;
  if (lines[idx].includes('//')) return true; // trailing same-line reason
  for (let i = idx - 1; i >= 0; i--) {
    const t = lines[i].trim();
    if (t === '') continue;
    if (t.startsWith('//') || t.startsWith('*') || t.startsWith('/*')) return true;
    const sib = /["']([a-z][a-z0-9_]*)["']/.exec(t.replace(/\/\/.*$/, ''));
    if (sib && notExposedSet.has(sib[1])) continue; // same not-exposed run
    return false;
  }
  return false;
}

/**
 * TRUE when the client manifest declares a REASON for this not-exposed family:
 * either the family is NAMED in the comment block over the not-exposed list
 * (the GUI shape), or its entry sits in a comment-headed not-exposed run /
 * carries a trailing comment (the Excel `ALL_FAMILIES` shape).
 */
function reasonDeclared(client, family) {
  if (new RegExp(`\\b${family}\\b`).test(client.reasonText)) return true;
  return notExposedRunHasReasonComment(client.declBody, family, client.notExposed);
}

/**
 * Parse the two client exposure manifests from the client suites' own
 * declarations. Throws (⇒ lint failure) if a manifest constant is missing,
 * empty, or the Excel suite's executable `FAMILIES_NOT_EXPOSED` derivation has
 * been deleted — the manifest must stay enforced IN the client suite, not only
 * mirrored here.
 */
async function loadClientManifests() {
  const manifests = [];

  // --- GUI: explicit covered + explicit not-exposed (reasons in the doc block) ---
  {
    const text = await readFile(path.join(REPO_ROOT, GUI_CONFORMANCE), 'utf8');
    const exposed = extractStringArray(text, 'FAMILIES_COVERED', GUI_CONFORMANCE);
    const notExposed = extractStringArray(text, 'FAMILIES_NOT_EXPOSED_BY_GUI', GUI_CONFORMANCE);
    manifests.push({
      name: 'gui',
      file: GUI_CONFORMANCE,
      exposed: new Set(exposed.items),
      notExposed: new Set(notExposed.items),
      reasonText: leadingCommentOf(text, 'FAMILIES_NOT_EXPOSED_BY_GUI', GUI_CONFORMANCE),
      declBody: notExposed.rawBody,
      exposedConst: 'FAMILIES_COVERED',
      notExposedConst: 'FAMILIES_NOT_EXPOSED_BY_GUI',
    });
  }

  // --- Excel: explicit exposed; not-exposed is the suite's own derived
  //     ALL_FAMILIES \ EXCEL_FAMILIES (re-derived here from the two literals) ---
  {
    const text = await readFile(path.join(REPO_ROOT, EXCEL_CORPUS), 'utf8');
    const all = extractStringArray(text, 'ALL_FAMILIES', EXCEL_CORPUS);
    const exposed = extractStringArray(text, 'EXCEL_FAMILIES', EXCEL_CORPUS);
    if (!/FAMILIES_NOT_EXPOSED\s*=\s*ALL_FAMILIES\s*\.\s*filter/.test(text)) {
      throw new Error(
        `${EXCEL_CORPUS}: the executable \`FAMILIES_NOT_EXPOSED = ALL_FAMILIES.filter(...)\` ` +
          `derivation is gone — the Excel not-exposed manifest must stay enforced in the client suite`,
      );
    }
    const exposedSet = new Set(exposed.items);
    const notExposedSet = new Set(all.items.filter((f) => !exposedSet.has(f)));
    manifests.push({
      name: 'excel',
      file: EXCEL_CORPUS,
      exposed: exposedSet,
      notExposed: notExposedSet,
      reasonText: leadingCommentOf(text, 'EXCEL_FAMILIES', EXCEL_CORPUS),
      declBody: all.rawBody,
      exposedConst: 'EXCEL_FAMILIES',
      notExposedConst: 'FAMILIES_NOT_EXPOSED (derived: ALL_FAMILIES minus EXCEL_FAMILIES)',
    });
  }

  return manifests;
}

/** Check the parity row for a cross-asset option family. Returns null if OK, else a reason. */
async function checkCrossAssetParity(family) {
  const testFile = CROSS_ASSET_FAMILY_TO_PARITY_FILE[family];
  if (!testFile) {
    return `no parity row: cross-asset family is unmapped in CROSS_ASSET_FAMILY_TO_PARITY_FILE (a new non-FX Underlying.ref arm MUST add a curated map entry pointing at its celnet-parity row)`;
  }
  const abs = path.join(PARITY_TESTS_DIR, testFile);
  if (!existsSync(abs)) {
    return `mapped parity test missing on disk: ${path.relative(REPO_ROOT, abs)}`;
  }
  const text = await readFile(abs, 'utf8');
  if (!/#\[test\]/.test(text)) {
    return `mapped parity test ${testFile} contains no #[test] rows`;
  }
  // The file must reference this asset class by its financial stem (e.g.
  // `equity_option` → `equity`), so a gutted/renamed row is caught, not waved through.
  const stem = family.replace(/_option$/, '');
  if (!new RegExp(stem, 'i').test(text)) {
    return `mapped parity test ${testFile} does not reference family "${family}" (stem "${stem}")`;
  }
  return null;
}

/** Check the golden vector for a family. Returns null if OK, else a reason string. */
async function checkVector(family) {
  const file = path.join(VECTORS_DIR, `${family}.json`);
  if (!existsSync(file)) {
    return `no golden vector file: ${path.relative(REPO_ROOT, file)}`;
  }
  let parsed;
  try {
    parsed = JSON.parse(await readFile(file, 'utf8'));
  } catch (e) {
    return `golden vector is not valid JSON (${e.message})`;
  }
  if (!Array.isArray(parsed) || parsed.length === 0) {
    return `golden vector is empty (must contain ≥1 cross-client vector)`;
  }
  const wrong = parsed.find((v) => v && typeof v === 'object' && v.family !== family);
  if (wrong) {
    return `golden vector record has mismatched "family" tag: "${wrong.family}" ≠ "${family}"`;
  }
  return null;
}

/** Check the parity row for a family. Returns null if OK, else a reason string. */
async function checkParity(family) {
  const testFile = FAMILY_TO_PARITY_FILE[family];
  if (!testFile) {
    return `no parity row: family is unmapped in FAMILY_TO_PARITY_FILE (a new arm MUST add a curated map entry pointing at its celnet-parity test row)`;
  }
  const abs = path.join(PARITY_TESTS_DIR, testFile);
  if (!existsSync(abs)) {
    return `mapped parity test missing on disk: ${path.relative(REPO_ROOT, abs)}`;
  }
  const text = await readFile(abs, 'utf8');
  if (!/#\[test\]/.test(text)) {
    return `mapped parity test ${testFile} contains no #[test] rows`;
  }
  // The file must actually reference this family, so a renamed-away or gutted row
  // is caught rather than silently accepted. Parity tests legitimately name the
  // product by its financial stem and the public pricer name rather than the exact
  // proto field tag (e.g. `variance_swap` → `var_swap`/`variance`; `asian_option`
  // → `asian`). We accept any of a small set of DERIVED tokens for the family:
  //   - the full field name (`variance_swap`)
  //   - the name with the trailing `_swap`/`_option` product suffix stripped
  //   - that stem with `_` → `` collapsed and abbreviated (variance→var, …)
  // At least one must appear (case-insensitive) — a meaningful, non-fooled check.
  const stems = new Set([family]);
  const noSuffix = family.replace(/_(swap|option)$/, '');
  stems.add(noSuffix);
  stems.add(noSuffix.replace(/_/g, ''));
  // Common pricer abbreviations used in the codebase for the two-word swaps.
  if (noSuffix === 'variance') stems.add('var_swap');
  if (noSuffix === 'volatility') stems.add('vol_swap');
  const referenced = [...stems].some((s) => {
    const re = new RegExp(s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'i');
    return re.test(text);
  });
  if (!referenced) {
    return `mapped parity test ${testFile} does not reference family "${family}" (tried: ${[...stems].join(', ')})`;
  }
  return null;
}

async function main() {
  let protoText;
  try {
    protoText = await readFile(PROTO, 'utf8');
  } catch (e) {
    console.error(`check-verification-coverage: FAILED — cannot read proto: ${e.message}`);
    process.exit(1);
  }

  let arms;
  try {
    arms = extractProductArms(protoText);
  } catch (e) {
    console.error(`check-verification-coverage: FAILED — proto parse error: ${e.message}`);
    process.exit(1);
  }

  const uncovered = new Map(); // family -> reasons[]
  const fail = (family, reason) => {
    if (!uncovered.has(family)) uncovered.set(family, []);
    uncovered.get(family).push(reason);
  };

  for (const family of arms) {
    const v = await checkVector(family);
    if (v) fail(family, `vector: ${v}`);
    const p = await checkParity(family);
    if (p) fail(family, `parity: ${p}`);
  }

  // SECOND, proto-driven pass: every NON-FX `Underlying.ref` arm is a distinct
  // asset-class OPTION family of the one contract (the `vanilla` product on a non-FX
  // underlying), and the verification contract requires BOTH a golden vector and an
  // independent-oracle parity row for it too. Driven off `message Underlying`'s
  // `oneof ref` so a new underlying arm cannot ship without its coverage.
  let refArms;
  try {
    refArms = extractUnderlyingRefArms(protoText);
  } catch (e) {
    console.error(`check-verification-coverage: FAILED — proto parse error: ${e.message}`);
    process.exit(1);
  }
  const crossAssetFamilies = [];
  for (const ref of refArms) {
    if (FX_EQUIVALENT_REFS.has(ref)) continue; // FX/metal = the FX `vanilla` family, already covered.
    const family = CROSS_ASSET_REF_TO_FAMILY[ref];
    if (!family) {
      fail(
        `${ref} (Underlying.ref)`,
        `cross-asset: non-FX Underlying.ref arm "${ref}" is unmapped in CROSS_ASSET_REF_TO_FAMILY ` +
          `(a new cross-asset underlying MUST add a "<asset>_option" family with a golden vector + parity row)`,
      );
      continue;
    }
    crossAssetFamilies.push(family);
    const v = await checkVector(family);
    if (v) fail(family, `vector: ${v}`);
    const p = await checkCrossAssetParity(family);
    if (p) fail(family, `parity: ${p}`);
  }

  // THIRD pass — the CLIENT axis: every family (product arm or cross-asset
  // option) must be accounted for in EACH client exposure manifest — either
  // exposed by the client suite, or explicitly declared not-exposed WITH a
  // reason. A family in neither list is a SILENT client gap: it would carry a
  // vector + a parity row yet ship with no client able to price it (the
  // Round-2 cross-asset finding). Honest declarations pass; silence fails.
  let clients;
  try {
    clients = await loadClientManifests();
  } catch (e) {
    console.error(`check-verification-coverage: FAILED — client manifest parse error: ${e.message}`);
    process.exit(1);
  }
  const allCheckedFamilies = [...arms, ...crossAssetFamilies];
  for (const client of clients) {
    for (const family of allCheckedFamilies) {
      const isExposed = client.exposed.has(family);
      const isDeclared = client.notExposed.has(family);
      if (isExposed && isDeclared) {
        fail(
          family,
          `client(${client.name}): contradictory manifest — listed in BOTH ${client.exposedConst} ` +
            `and ${client.notExposedConst} (${client.file})`,
        );
      } else if (isExposed || isDeclared) {
        if (isDeclared && !reasonDeclared(client, family)) {
          fail(
            family,
            `client(${client.name}): declared not-exposed but carries NO reason — name it with its ` +
              `rationale in the comment block over ${client.notExposedConst}, or head its run in the ` +
              `manifest with a rationale comment (${client.file})`,
          );
        }
      } else {
        fail(
          family,
          `client(${client.name}): SILENTLY ABSENT from the client exposure manifest (${client.file}) — ` +
            `expose it (add to ${client.exposedConst} with a real client suite row) or declare it in ` +
            `${client.notExposedConst} WITH a reason`,
        );
      }
    }
  }

  const checked = arms.length + crossAssetFamilies.length;
  const covered = checked - uncovered.size;

  if (uncovered.size === 0) {
    console.log(
      `check-verification-coverage: OK — all ${arms.length} product-oneof arms AND all ` +
        `${crossAssetFamilies.length} cross-asset option families (non-FX Underlying.ref arms) have BOTH a ` +
        `golden vector (crates/celnet-golden/vectors/) AND an independent-oracle parity row ` +
        `(crates/celnet-parity/tests/), AND every family is accounted for in every client exposure ` +
        `manifest (exposed, or declared not-exposed with a reason).`,
    );
    console.log(`  product arms: ${arms.join(', ')}`);
    console.log(`  cross-asset families: ${crossAssetFamilies.join(', ')}`);
    for (const c of clients) {
      const exposedN = allCheckedFamilies.filter((f) => c.exposed.has(f)).length;
      const declaredN = allCheckedFamilies.filter((f) => !c.exposed.has(f) && c.notExposed.has(f)).length;
      console.log(
        `  client ${c.name}: ${exposedN} exposed, ${declaredN} declared not-exposed with reason (${c.file})`,
      );
    }
    process.exit(0);
  }

  console.error(
    `check-verification-coverage: FAILED — ${uncovered.size}/${checked} product-oneof arm(s) ` +
      `and/or cross-asset option famil(ies) lack a golden vector, a parity row, and/or an honest ` +
      `client-exposure declaration (see docs/VERIFICATION-CONTRACT.md):\n`,
  );
  for (const [family, reasons] of uncovered) {
    console.error(`  ✗ ${family}`);
    for (const r of reasons) console.error(`      ${r}`);
  }
  console.error(
    `\n  ${covered}/${checked} arms + cross-asset families fully covered. Add the missing golden ` +
      `vector, the independent-oracle parity row (and its map entry), and/or the client-manifest ` +
      `exposure/reasoned-not-exposed declaration — do NOT weaken this lint.`,
  );
  process.exit(1);
}

main();
