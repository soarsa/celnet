/**
 * notificationText — a text heuristic for the desk-notification stream. The wire
 * carries NO numeric notional on {@link Notification}; the size lives only inside
 * the human `headline` / `detail` strings, and in DIFFERENT forms depending on the
 * source: the in-app mock emits "75mm" / "5y OIS 75mm", while the live server
 * emits raw "10000000 notional". So we DERIVE the notional from the text.
 *
 * Token grammar (shared by both functions here):
 *   - a number  `\d[\d,]*(\.\d+)?`  (commas are thousands separators, stripped),
 *   - optionally immediately followed by a case-insensitive unit suffix
 *     `mm | bn | m | b | k | t`  scaling by  mm=1e6, m=1e6, k=1e3, b=1e9, bn=1e9,
 *     t=1e12  (longer alternatives — `mm`, `bn` — are tried first);
 *   - a BARE number (no suffix) counts as a notional ONLY if it is `>= 1000` OR is
 *     immediately followed by the word "notional" — so tenors ("5y"), rates
 *     ("3.250%"), and small ids ("deal-1") are NOT mistaken for size;
 *   - a `%` or `y` immediately after the number DISQUALIFIES it (a rate / a tenor).
 *
 * This is an honest, well-bounded heuristic — not a parser of a structured field —
 * because no structured field exists on the contract.
 */

import type { ManualInterventionReason, Notification } from "../data/contract";
import { fmtCompact } from "./format";

/**
 * Human label for a {@link ManualInterventionReason} — the trader-facing "why"
 * rendered on a `MANUAL_INTERVENTION_REQUIRED` notification's toast + desktop
 * growl body. Exhaustive over the four ordinals (1–4) the server exception
 * contract defines (commit 542e547).
 */
export function manualInterventionText(reason: ManualInterventionReason): string {
  switch (reason) {
    case "UNCONFIGURED_TENOR":
      return "Manual pricing needed — unconfigured tenor";
    case "CREDIT_RISK_BREAK":
      return "Manual pricing needed — credit risk break";
    case "UNKNOWN_SECURITY":
      return "Manual pricing needed — unknown security";
    case "PRICING_FAILURE":
      return "Manual pricing needed — pricing failure";
  }
}

/** Unit-suffix → multiplier. Keys are lower-cased at match time. */
const SCALE: Readonly<Record<string, number>> = {
  k: 1e3,
  m: 1e6,
  mm: 1e6,
  b: 1e9,
  bn: 1e9,
  t: 1e12,
};

/** The threshold at or above which a BARE (unsuffixed) number reads as a notional. */
const BARE_NOTIONAL_MIN = 1000;

/**
 * One scan token: a number, an optional unit suffix (group 2, longest-first so
 * `mm`/`bn` win over `m`/`b`), a zero-width guard that the token is NOT the prefix
 * of a longer word (so "75million" / "req12345x" do not match), and an optional
 * trailing " notional" word (captured so `compactNotionals` can preserve it).
 *
 * The `(?![a-z0-9])` after the optional suffix is what rejects a tenor "5y": with
 * no suffix, the next char "y" is `[a-z]`, so the guard fails and the "5" is not
 * matched at all. A following "%" is allowed through the guard (it is not
 * alphanumeric) and handled explicitly in the decision below.
 */
const TOKEN_RE = /(\d[\d,]*(?:\.\d+)?)(mm|bn|m|b|k|t)?(?![a-z0-9])(\s+notional\b)?/gi;

/** The numeric value of a matched (numberString, unitSuffix?) pair. */
function scaledValue(numStr: string, unit: string | undefined): number {
  const n = parseFloat(numStr.replace(/,/g, ""));
  if (!Number.isFinite(n)) return NaN;
  if (unit === undefined) return n;
  return n * (SCALE[unit.toLowerCase()] ?? 1);
}

/**
 * Decide whether a single regex match is a notional token, and its value.
 * `charAfter` is the character immediately following the whole match (or ""),
 * used to reject a large bare number that a "%" turns into a rate ("1500%").
 */
function decideToken(
  numStr: string,
  unit: string | undefined,
  notionalWord: string | undefined,
  charAfter: string,
): { readonly isNotional: boolean; readonly value: number } {
  const value = scaledValue(numStr, unit);
  if (!Number.isFinite(value)) return { isNotional: false, value: NaN };
  // An explicit unit suffix is unambiguous size.
  if (unit !== undefined) return { isNotional: true, value };
  // Bare number: a trailing "%" makes it a rate, not a notional.
  if (charAfter === "%") return { isNotional: false, value };
  // Bare number counts if the word "notional" follows, or it clears the floor.
  if (notionalWord !== undefined) return { isNotional: true, value };
  if (value >= BARE_NOTIONAL_MIN) return { isNotional: true, value };
  return { isNotional: false, value };
}

/**
 * The FIRST notional magnitude mentioned in a notification's `headline` + `detail`,
 * or `undefined` when none is found. Callers treat `undefined` as "size unknown →
 * never suppress" (fail-open), so a headline the heuristic cannot read is still
 * shown.
 */
export function notionalMagnitude(n: Notification): number | undefined {
  const text = `${n.headline} ${n.detail ?? ""}`;
  // A fresh RegExp per call keeps `lastIndex` state local (TOKEN_RE is global).
  const re = new RegExp(TOKEN_RE.source, TOKEN_RE.flags);
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    // Guard against a zero-length match looping forever (defensive).
    if (m[0].length === 0) {
      re.lastIndex += 1;
      continue;
    }
    const charAfter = text.charAt(m.index + m[0].length);
    const { isNotional, value } = decideToken(m[1] ?? "", m[2], m[3], charAfter);
    if (isNotional) return value;
  }
  return undefined;
}

/**
 * Replace every notional token in `text` with its {@link fmtCompact} form,
 * preserving a following " notional" word:
 *   "10000000 notional" → "10m notional",  "75mm" → "75m",  "1,500,000" → "1.5m".
 * Rates / percentages / tenors are left untouched ("3.250%", "5y OIS" unchanged).
 */
export function compactNotionals(text: string): string {
  return text.replace(
    TOKEN_RE,
    (match, numStr: string, unit: string | undefined, notionalWord: string | undefined, offset: number, whole: string): string => {
      const charAfter = whole.charAt(offset + match.length);
      const { isNotional, value } = decideToken(numStr, unit, notionalWord, charAfter);
      if (!isNotional) return match;
      // Preserve the trailing " notional" word (with its original whitespace).
      return `${fmtCompact(value)}${notionalWord ?? ""}`;
    },
  );
}
