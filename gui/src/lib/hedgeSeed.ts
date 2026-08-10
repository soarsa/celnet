/**
 * hedgeSeed — the PURE bridge from a received DEAL (the risk a fill just added to the
 * book) to a seeded hedge {@link HedgeRule}, so a trader can right-click a deal on the
 * Deals blotter ("Change hedging strategy") and land in the Hedging Rules → Exit Policy
 * builder with a NEW rule pre-scoped to that flow, instead of hand-composing the
 * conditions. The direct analogue of `lib/acceptanceSeed`.
 *
 * ONE honest asymmetry the code makes explicit: the auto-hedge decision graph runs on
 * POST-AGGREGATION RISK STATE — it is keyed by the identity of the risk it nets (see
 * the `Identity` group of `lib/hedgeFields`: `instrument_id` / `ccy` / `product` /
 * `book` / `desk` / `counterparty`), NOT by per-deal trade attributes. So of a deal's
 * fields only the IDENTITY ones the graph can actually test are seeded as conditions —
 * `ccy`, `product`, `desk`, `counterparty` (+ `instrument_id` when the deal carries a
 * symbol). The `counterparty` field (wire tag 18) keys the graph on the ORIGINATING
 * party-id, so a right-click seed can now scope a rule to exactly that counterparty's
 * flow. The TENOR, the deal NOTIONAL and the SIDE still have NO field in the hedge
 * vocabulary (the graph nets many fills into a book before it ever fires), so they are
 * surfaced in the seed HINT for context but NEVER invented as conditions.
 * {@link HEDGE_SEED_UNREPRESENTED} names them.
 *
 * Kept in `lib/` (no component imports) so the seed construction is unit-testable and
 * reusable by any client surface, exactly like `lib/acceptanceSeed`.
 */
import type {
  Deal,
  HedgeField,
  PricingGroup,
  RatesProductKind,
  RouteValue,
  Side,
} from "../data/contract";
import { fmtCompact } from "./format";
import { defaultExitAction } from "./hedgeExit";
import { defaultOpForHedgeField, defaultValueForOp } from "./hedgeGraphOps";
import { newHedgeRuleId, type HedgeRule, type HedgeRuleCondition } from "./hedgeRules";

/**
 * The deal facts a "Change hedging strategy" request carries to the Hedging builder.
 * A structural subset of {@link Deal} — enough to (a) scope the seeded rule to the
 * flow's identity and (b) render a human hint. Built by {@link hedgeSeedFromDeal}.
 */
export interface HedgeSeedDeal {
  /** The originating deal id — shown in the seed hint (`New rule seeded from deal #…`). */
  dealId: string;
  /** The originating counterparty — seeds `counterparty = …` (wire tag 18). */
  counterparty: string;
  /** The dealt product family (OIS/IRS/FRA/BOND) — seeds `product = …`. */
  productKind: RatesProductKind;
  /** The risk currency — seeds `ccy = …`. */
  currency: string;
  /** The owning desk — seeds `desk = …`. */
  desk: string;
  /** The swap tenor in years — CONTEXT ONLY (no hedge tenor field). */
  tenorYears: number;
  /** The deal notional — CONTEXT ONLY (the graph tests NET book risk, not a fill size). */
  notional: number;
  /** The dealt side — CONTEXT ONLY (the graph tests net inventory sign, not a fill side). */
  side: Side;
  /** The instrument symbol, when the deal carries one — seeds `instrument_id = …`. */
  instrumentSymbol?: string;
}

/** Project a booked {@link Deal} onto the {@link HedgeSeedDeal} the builder consumes. */
export function hedgeSeedFromDeal(deal: Deal): HedgeSeedDeal {
  return {
    dealId: deal.dealId,
    counterparty: deal.counterparty,
    productKind: deal.productKind,
    currency: deal.curveSet.currency,
    desk: deal.desk,
    tenorYears: deal.instrument.tenorYears,
    notional: deal.notional,
    side: deal.side,
  };
}

/**
 * Deal attributes the auto-hedge decision-graph vocabulary CANNOT express as a
 * condition (the graph fires on netted book risk, not per-deal trade attributes). They
 * appear in the seed hint for context but are never fabricated into a rule condition.
 */
export const HEDGE_SEED_UNREPRESENTED: readonly string[] = ["tenor", "notional", "side"];

/** Build one enum/string equality condition `field = <text>` (skips an empty value). */
export function eqCondition(field: HedgeField, text: string): HedgeRuleCondition | null {
  if (text.length === 0) return null;
  const op = defaultOpForHedgeField(field); // enum/string ⇒ "eq"
  const base = defaultValueForOp(field, op); // { kind: "text", text: "" } for enum/string
  const value: RouteValue = base.kind === "text" ? { kind: "text", text } : base;
  return { field, op, value };
}

/**
 * Build a fresh hedge rule pre-scoped to a deal's FLOW IDENTITY: `ccy = <ccy> AND
 * product = <productKind> AND desk = <desk> AND counterparty = <counterparty>` (+
 * `instrument_id = <symbol>` when the deal carries a symbol). The counterparty leg
 * (wire tag 18) matches the deal's party-id literal exactly as the blotter shows it —
 * no transformation — so the rule keys on that counterparty's netted flow. The exit
 * action defaults to WAREHOUSE (hold) — the same safe leaf the manual "+ Create hedge
 * rule" seeds and the neutral internalise default; the builder's seed hint prompts the
 * trader to pick the real exit action before saving. The rule is a NON-catch-all (it
 * has conditions), so the builder slots it ABOVE the trailing default on save
 * (first-match-wins).
 */
export function hedgeRuleFromSeed(d: HedgeSeedDeal): HedgeRule {
  const conditions = [
    eqCondition("ccy", d.currency),
    eqCondition("product", d.productKind),
    eqCondition("desk", d.desk),
    eqCondition("counterparty", d.counterparty),
    d.instrumentSymbol ? eqCondition("instrument_id", d.instrumentSymbol) : null,
  ].filter((c): c is HedgeRuleCondition => c !== null);
  return {
    id: newHedgeRuleId(),
    conditions,
    action: defaultExitAction("warehouse"),
    enabled: true,
  };
}

/**
 * The one-line hint shown when the Exit Policy builder opens from a deal seed, e.g.
 * `New rule seeded from deal #d-42 (Balyasny, OIS 5y, 10m USD, BUY) — pick an exit
 * action and Save.` Names the flow the rule was scoped from (including the fields that
 * became context-only), and prompts the trader to choose the exit action.
 */
export function hedgeSeedHint(d: HedgeSeedDeal): string {
  return (
    `New rule seeded from deal #${d.dealId} ` +
    `(${d.counterparty}, ${d.productKind} ${d.tenorYears}y, ` +
    `${fmtCompact(d.notional)} ${d.currency}, ${d.side}) — pick an exit action and Save.`
  );
}

/**
 * The gap note that accompanies {@link hedgeSeedHint}: which deal facts became rule
 * CONDITIONS vs which are context-only, so the trader knows the seeded rule scopes on
 * the flow's identity (including its counterparty) — not its size/side (the graph nets
 * book risk before it fires).
 */
export function hedgeSeedGapNote(): string {
  return (
    "Seeded as conditions: currency, product, desk, counterparty. " +
    `The hedge policy fires on netted book risk, so ${HEDGE_SEED_UNREPRESENTED.join(", ")} ` +
    "are shown for context only — not rule conditions."
  );
}

// --- pricing-group seed -----------------------------------------------------
//
// The SECOND seed source: a "Create hedging rule" hand-off from a PRICING GROUP (its
// editor button / roster right-click) into the SAME Hedging Rules → Exit Policy builder. A
// structural clone of the deal path above, but with ONE honest asymmetry that this
// section makes explicit:
//
//   A pricing group's ONLY overlap with the auto-hedge decision-graph vocabulary is
//   `desk` — its `memberDesks`. A group has NO currency / product / counterparty and NO
//   book / bucket / portfolio link, and a hedge rule is a FLAT AND of conditions (it
//   cannot OR several desks in one rule). So:
//     • exactly ONE member desk  ⇒ seed a single `desk = <desk>` condition;
//     • zero or MANY member desks ⇒ seed a NO-condition draft (identical to the manual
//       "+ Create hedge rule"), and let the HINT tell the trader what to add — we NEVER
//       invent a ccy/product/counterparty condition a group cannot substantiate.
//   The scope stays FIRM (a group carries no book/bucket to bind the policy to).

/**
 * The pricing-group facts a "Create hedging rule" request carries to the Hedging
 * builder. A structural subset of {@link PricingGroup} — enough to (a) seed a
 * desk-scoped rule when the group has exactly one member desk and (b) render an honest
 * hint. Built by {@link hedgeSeedFromPricingGroup}.
 */
export interface HedgeSeedPricingGroup {
  /** The originating pricing-group id — carried for provenance. */
  groupId: string;
  /** The group's display name — shown in the seed hint. */
  name: string;
  /** The group's member desks — the ONLY facts that map into the hedge vocabulary. */
  desks: string[];
  /** The count of member FIX connections — CONTEXT ONLY (no hedge field). */
  connectionCount: number;
  /** The count of member users — CONTEXT ONLY (no hedge field). */
  userCount: number;
}

/** Project a {@link PricingGroup} onto the {@link HedgeSeedPricingGroup} the builder consumes. */
export function hedgeSeedFromPricingGroup(g: PricingGroup): HedgeSeedPricingGroup {
  return {
    groupId: g.id,
    name: g.name,
    desks: g.memberDesks,
    connectionCount: g.memberConnectionIds.length,
    userCount: g.memberUserIds.length,
  };
}

/**
 * Build a fresh hedge rule seeded from a pricing group. When the group has EXACTLY ONE
 * member desk the rule is pre-scoped `desk = <desk>`; with zero or many member desks it
 * has NO conditions (identical to the manual "+ Create hedge rule") because the flat-AND
 * hedge vocabulary cannot express "any of these desks" and a group carries no other
 * testable identity. The exit action defaults to WAREHOUSE (hold) — the same safe leaf
 * the manual create seeds — and the builder's hint prompts the trader to pick the real
 * action + complete the conditions before saving.
 */
export function hedgeRuleFromPricingGroupSeed(g: HedgeSeedPricingGroup): HedgeRule {
  const conditions =
    g.desks.length === 1
      ? [eqCondition("desk", g.desks[0] as string)].filter(
          (c): c is HedgeRuleCondition => c !== null,
        )
      : [];
  return {
    id: newHedgeRuleId(),
    conditions,
    action: defaultExitAction("warehouse"),
    enabled: true,
  };
}

/**
 * The one-line hint shown when the Exit Policy builder opens from a pricing-group seed.
 * Three honest cases keyed on the member-desk count:
 *   • 1 desk  — the rule is pre-scoped to that desk;
 *   • >1 desks — lists them and asks the trader to add ONE desk condition (a flat-AND
 *     rule cannot OR several desks — author one rule per desk);
 *   • 0 desks  — the group is session/user scoped, which the hedge graph cannot test,
 *     so the trader must add the conditions.
 * Every variant ends by prompting the trader to pick an exit action and Save.
 */
export function pricingGroupSeedHint(g: HedgeSeedPricingGroup): string {
  if (g.desks.length === 1) {
    return (
      `New hedge rule seeded from pricing group "${g.name}" (desk ${g.desks[0]}) — ` +
      "pick an exit action and Save."
    );
  }
  if (g.desks.length > 1) {
    return (
      `New hedge rule seeded from pricing group "${g.name}" — it spans ${g.desks.length} desks ` +
      `(${g.desks.join(", ")}); a rule matches ONE desk, so add a desk condition (one rule per desk) — ` +
      "pick an exit action and Save."
    );
  }
  return (
    `New hedge rule seeded from pricing group "${g.name}" — it has no desk members ` +
    "(session/user scoped), which the hedge policy cannot test; add conditions — " +
    "pick an exit action and Save."
  );
}

/**
 * The gap note that accompanies {@link pricingGroupSeedHint}: the honest statement that
 * only a group's DESK membership maps into the hedge vocabulary. A pricing group has no
 * currency / product / counterparty and no book / bucket scope, so the seeded rule is
 * desk-scoped at FIRM level and the trader completes the conditions + exit action.
 */
export function pricingGroupSeedGapNote(): string {
  return (
    "Only a pricing group's desk membership maps into the hedge vocabulary — a group " +
    "has no currency, product or counterparty and no book/bucket scope. The seeded rule " +
    "is therefore desk-scoped at the Firm level; you complete the conditions and pick the exit action."
  );
}
