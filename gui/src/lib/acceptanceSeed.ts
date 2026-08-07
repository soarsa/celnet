/**
 * acceptanceSeed — the PURE bridge from a live-flow row (a deal / shown quote carrying
 * a counterparty) to a seeded {@link AcceptanceRule}. It lets a trader spawn an
 * acceptance rule straight from the Deals / Quotes blotters ("Create acceptance rule"
 * on a row) instead of hand-typing the counterparty name into the builder.
 *
 * Kept in `lib/` (no component imports) so the seed construction + merge are
 * unit-testable and reusable by any client surface, exactly like `lib/acceptanceRules`.
 * The seeded condition literal is built through the SAME `defaultOpForAcceptanceField` /
 * `defaultValueForOp` helpers the rule editor uses for the counterparty (enum) field, so
 * the seeded rule round-trips through compile/decompile and validates identically to one
 * authored by hand.
 */
import type { AcceptanceActionKind, RouteValue } from "../data/contract";
import { defaultAcceptanceAction } from "./acceptanceAction";
import { defaultOpForAcceptanceField, defaultValueForOp } from "./acceptanceGraphOps";
import { newAcceptanceRuleId, type AcceptanceRule } from "./acceptanceRules";

/** The lift-field a flow-seeded rule keys on — the originating counterparty id. */
const COUNTERPARTY_FIELD = "counterparty" as const;

/**
 * Build a fresh acceptance rule pre-populated with `Counterparty = <counterparty>` and a
 * default decision. The decision defaults to REJECT with an EMPTY reason — the
 * least-surprising choice: a specific rule spawned from live flow most often declines a
 * counterparty (the catch-all stays accept-all), and the empty reason is the exact same
 * starting point the "+ Create acceptance rule" button seeds, so the trader immediately
 * edits the reason / flips to hold. Equality (`=`) matches the single named counterparty.
 */
export function counterpartyAcceptanceRule(
  counterparty: string,
  decision: AcceptanceActionKind = "reject",
): AcceptanceRule {
  const op = defaultOpForAcceptanceField(COUNTERPARTY_FIELD); // "eq" (enum equality)
  const base = defaultValueForOp(COUNTERPARTY_FIELD, op); // { kind: "text", text: "" }
  const value: RouteValue = base.kind === "text" ? { kind: "text", text: counterparty } : base;
  return {
    id: newAcceptanceRuleId(),
    conditions: [{ field: COUNTERPARTY_FIELD, op, value }],
    action: defaultAcceptanceAction(decision),
    enabled: true,
  };
}

/**
 * Merge a freshly-seeded rule into the CURRENT policy WITHOUT clobbering the existing
 * acceptance graph — the seeded rule slots ABOVE the trailing catch-all (default) rule so
 * it actually fires (first-match-wins), mirroring the builder's own new-rule placement in
 * `AcceptanceWorkspace.onEditorSave`. With no default present (or a default-less seed) it
 * is appended. Returns a NEW array; never mutates the input.
 */
export function mergeSeedRule(
  rules: readonly AcceptanceRule[],
  seed: AcceptanceRule,
): AcceptanceRule[] {
  const next = [...rules];
  const defaultIdx = next.findIndex((r) => r.conditions.length === 0);
  if (seed.conditions.length > 0 && defaultIdx >= 0) {
    next.splice(defaultIdx, 0, seed);
  } else {
    next.push(seed);
  }
  return next;
}
