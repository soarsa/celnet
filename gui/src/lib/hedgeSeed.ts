/**
 * hedgeSeed — the PURE bridge from a received DEAL (the risk a fill just added to the
 * book) to a seeded hedge {@link HedgeRule}, so a trader can right-click a deal on the
 * Deals blotter ("Change hedging strategy") and land in the Hedging → Exit Policy
 * builder with a NEW rule pre-scoped to that flow, instead of hand-composing the
 * conditions. The direct analogue of `lib/acceptanceSeed`.
 *
 * ONE honest asymmetry the code makes explicit: the auto-hedge decision graph runs on
 * POST-AGGREGATION RISK STATE — it is keyed by the identity of the risk it nets (see
 * the `Identity` group of `lib/hedgeFields`: `instrument_id` / `ccy` / `product` /
 * `book` / `desk`), NOT by per-deal trade attributes. So of a deal's fields only the
 * IDENTITY ones the graph can actually test are seeded as conditions — `ccy`,
 * `product`, `desk` (+ `instrument_id` when the deal carries a symbol). The originating
 * COUNTERPARTY, the TENOR, the deal NOTIONAL and the SIDE have NO field in the hedge
 * vocabulary (the graph nets many fills into a book before it ever fires), so they are
 * surfaced in the seed HINT for context but NEVER invented as conditions.
 * {@link HEDGE_SEED_UNREPRESENTED} names them.
 *
 * Kept in `lib/` (no component imports) so the seed construction is unit-testable and
 * reusable by any client surface, exactly like `lib/acceptanceSeed`.
 */
import type { Deal, HedgeField, RatesProductKind, RouteValue, Side } from "../data/contract";
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
  /** The originating counterparty — CONTEXT ONLY (no hedge field keys on it). */
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
export const HEDGE_SEED_UNREPRESENTED: readonly string[] = [
  "counterparty",
  "tenor",
  "notional",
  "side",
];

/** Build one enum/string equality condition `field = <text>` (skips an empty value). */
function eqCondition(field: HedgeField, text: string): HedgeRuleCondition | null {
  if (text.length === 0) return null;
  const op = defaultOpForHedgeField(field); // enum/string ⇒ "eq"
  const base = defaultValueForOp(field, op); // { kind: "text", text: "" } for enum/string
  const value: RouteValue = base.kind === "text" ? { kind: "text", text } : base;
  return { field, op, value };
}

/**
 * Build a fresh hedge rule pre-scoped to a deal's FLOW IDENTITY: `ccy = <ccy> AND
 * product = <productKind> AND desk = <desk>` (+ `instrument_id = <symbol>` when the
 * deal carries a symbol). The exit action defaults to WAREHOUSE (hold) — the same safe
 * leaf the manual "+ Create hedge rule" seeds and the neutral internalise default; the
 * builder's seed hint prompts the trader to pick the real exit action before saving.
 * The rule is a NON-catch-all (it has conditions), so the builder slots it ABOVE the
 * trailing default on save (first-match-wins).
 */
export function hedgeRuleFromSeed(d: HedgeSeedDeal): HedgeRule {
  const conditions = [
    eqCondition("ccy", d.currency),
    eqCondition("product", d.productKind),
    eqCondition("desk", d.desk),
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
 * the flow's identity — not its counterparty/size/side (the graph nets book risk).
 */
export function hedgeSeedGapNote(): string {
  return (
    "Seeded as conditions: currency, product, desk. " +
    `The hedge policy fires on netted book risk, so ${HEDGE_SEED_UNREPRESENTED.join(", ")} ` +
    "are shown for context only — not rule conditions."
  );
}
