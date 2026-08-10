/**
 * The PURE model behind the Exit-Policy "guided rule" wizard (Hedging Rules → Exit
 * Policy). Two complementary, dependency-free capabilities, kept in `lib/` (no React,
 * no transport) so the wizard component, any other client surface, and the vitest
 * suite share ONE implementation:
 *
 *   1. SCENARIO PRESETS — a small set of real hedge intents ("warehouse until a size
 *      limit, then flatten", "hold small / escalate large", "back-to-back a
 *      counterparty", "pure internalisation"). Each generates a COMPLETE, VALID
 *      first-match-wins {@link HedgeRule} list (specific rules + a trailing warehouse
 *      default) via {@link buildScenarioRules} — the SAME rule graphs the manual
 *      "+ Create hedge rule" builder emits, so they round-trip through
 *      `compileRulesToHedgeGraph` and pass `validateHedgeGraph` / the conflict checks
 *      unchanged.
 *
 *   2. FLOW ANALYSIS — {@link summariseBookFlow} reads the data the GUI ALREADY holds
 *      (a book's rolled-up {@link RiskBookRisk} from the risk-dashboard store + the
 *      client-blotter {@link Deal}s) and derives HONEST suggested thresholds for the
 *      selected book. No flow ⇒ it says so ({@link BookFlowSummary.hasFlow} false); it
 *      never fabricates numbers.
 *
 * The DV01-vs-notional distinction is first-class here: a scenario's METRIC is either
 * `net_notional` (FACE, units $) or `net_dv01` (RISK, units $/bp), each carried with
 * its unit so every surface labels the two honestly (the historical bug was a "Net
 * notional" condition silently compared against DV01).
 */
import type { Deal, ExitAction, RiskBookRisk, RouteOp } from "../data/contract";
import { defaultExitAction } from "./hedgeExit";
import {
  newHedgeRuleId,
  type HedgeRule,
  type HedgeRuleCondition,
} from "./hedgeRules";

// --- metrics ---------------------------------------------------------------

/** The two size metrics a scenario can threshold on — the hedge FIELD each maps to. */
export const SCENARIO_METRICS = ["net_notional", "net_dv01"] as const;
export type ScenarioMetric = (typeof SCENARIO_METRICS)[number];

/** How a metric is labelled + its UNIT — so a surface never conflates face with risk. */
export interface MetricDescriptor {
  /** The hedge-field selector the generated condition tests. */
  field: ScenarioMetric;
  /** Human label ("Net notional" / "Net DV01"). */
  label: string;
  /** The metric's unit ("$" face / "$/bp" risk). */
  unit: string;
  /** A one-line reminder of what the metric measures. */
  blurb: string;
}

/** The metric registry — the single source of the label + unit for both metrics. */
export const METRIC_DESCRIPTORS: Record<ScenarioMetric, MetricDescriptor> = {
  net_notional: {
    field: "net_notional",
    label: "Net notional",
    unit: "$",
    blurb: "FACE value — the signed net base-currency notional the book warehouses.",
  },
  net_dv01: {
    field: "net_dv01",
    label: "Net DV01",
    unit: "$/bp",
    blurb: "RATE RISK — the PV change per +1bp (a $/bp risk, NOT a face amount).",
  },
};

// --- scenario catalogue ----------------------------------------------------

/** The scenario presets, in card order. */
export const SCENARIO_IDS = [
  "warehouse-then-flatten",
  "tiered-escalate",
  "b2b-counterparty",
  "pure-internalise",
] as const;
export type ScenarioId = (typeof SCENARIO_IDS)[number];

/** Which input controls a scenario needs (drives the wizard form + validation). */
export interface ScenarioInputs {
  /** Needs a metric + primary size threshold. */
  metric: boolean;
  /** Needs a SECOND (escalation) threshold above the first. */
  escalate: boolean;
  /** Needs a counterparty literal. */
  counterparty: boolean;
}

/** One scenario preset's descriptor — title, plain-English intent, and needed inputs. */
export interface ScenarioSpec {
  id: ScenarioId;
  /** Card title. */
  title: string;
  /** Plain-English "what it does". */
  plain: string;
  /** The inputs the scenario's rules are parameterised by. */
  inputs: ScenarioInputs;
}

/** The authored scenario catalogue. */
export const SCENARIOS: readonly ScenarioSpec[] = [
  {
    id: "warehouse-then-flatten",
    title: "Warehouse until a size limit, then flatten to the street",
    plain:
      "Hold (internalise) the book's risk for free while it stays small. Once the chosen size metric crosses your cap, submit a market order to flatten the excess back-to-back with the street.",
    inputs: { metric: true, escalate: false, counterparty: false },
  },
  {
    id: "tiered-escalate",
    title: "Hold small, escalate large (tiered)",
    plain:
      "Two size bands. Below the first threshold, warehouse. Between the two, skew your two-way to attract the offsetting side (no trade). Above the upper threshold, submit a market order to hedge externally.",
    inputs: { metric: true, escalate: true, counterparty: false },
  },
  {
    id: "b2b-counterparty",
    title: "Back-to-back a specific counterparty",
    plain:
      "Any flow originating from the named counterparty is hedged externally immediately (never warehoused). Everything else warehouses. Use it to keep a toxic or size-capped counterparty's risk off the book.",
    inputs: { metric: false, escalate: false, counterparty: true },
  },
  {
    id: "pure-internalise",
    title: "Pure internalisation / wash",
    plain:
      "Warehouse everything — always hold the risk and net client flow against itself. No external hedging. The simplest policy; a good starting point before you add size or counterparty rules.",
    inputs: { metric: false, escalate: false, counterparty: false },
  },
];

/** Look up a scenario spec by id. */
export function scenarioSpec(id: ScenarioId): ScenarioSpec {
  const s = SCENARIOS.find((x) => x.id === id);
  if (!s) throw new Error(`hedgeScenario: unknown scenario \`${id}\``);
  return s;
}

/** The parameters a scenario's rule set is generated from. */
export interface ScenarioParams {
  /** The size metric (Net notional / Net DV01) for the threshold scenarios. */
  metric: ScenarioMetric;
  /** The primary size threshold (the "warehouse up to" cap). */
  threshold: number;
  /** The upper escalation threshold (tiered scenario only) — must exceed {@link threshold}. */
  escalateThreshold: number;
  /** The counterparty literal (back-to-back scenario only). */
  counterparty: string;
}

/** The default scenario parameters (before analysis / user edits). */
export function defaultScenarioParams(): ScenarioParams {
  return { metric: "net_notional", threshold: 0, escalateThreshold: 0, counterparty: "" };
}

// --- rule builders ---------------------------------------------------------

/** One numeric `field op num` condition. */
function numCond(field: ScenarioMetric, op: RouteOp, num: number): HedgeRuleCondition {
  return { field, op, value: { kind: "num", num } };
}

/** The FLATTEN action — a market order sized to fully flatten (not just the overflow). */
function flattenAction(): ExitAction {
  return { ...defaultExitAction("submit_market_order"), size: { kind: "full", fixed: 0 } };
}

/** The EXTERNAL-hedge action — a market order sizing the overflow to the band edge. */
function externalAction(): ExitAction {
  return defaultExitAction("submit_market_order");
}

/** The safe trailing catch-all: warehouse (hold). */
function warehouseDefault(): HedgeRule {
  return { id: newHedgeRuleId(), conditions: [], action: defaultExitAction("warehouse"), enabled: true };
}

/** Wrap conditions + action into a fresh enabled rule. */
function rule(conditions: HedgeRuleCondition[], action: ExitAction): HedgeRule {
  return { id: newHedgeRuleId(), conditions, action, enabled: true };
}

/**
 * Generate the COMPLETE, valid rule list for a scenario. Every scenario ends with a
 * warehouse catch-all so the graph always has exactly one default. Threshold scenarios
 * order the bigger threshold FIRST (first-match-wins), so the tighter upper band is
 * never shadowed by the looser lower one.
 */
export function buildScenarioRules(id: ScenarioId, p: ScenarioParams): HedgeRule[] {
  switch (id) {
    case "warehouse-then-flatten":
      return [rule([numCond(p.metric, "gt", p.threshold)], flattenAction()), warehouseDefault()];
    case "tiered-escalate":
      return [
        rule([numCond(p.metric, "gt", p.escalateThreshold)], externalAction()),
        rule([numCond(p.metric, "gt", p.threshold)], defaultExitAction("skew")),
        warehouseDefault(),
      ];
    case "b2b-counterparty":
      return [
        rule(
          [{ field: "counterparty", op: "eq", value: { kind: "text", text: p.counterparty } }],
          externalAction(),
        ),
        warehouseDefault(),
      ];
    case "pure-internalise":
      return [warehouseDefault()];
  }
}

/**
 * Per-scenario blocking validation of the PARAMETERS (before rules are built). Returns
 * human messages; an empty array means the scenario can generate its rules.
 */
export function scenarioParamErrors(id: ScenarioId, p: ScenarioParams): string[] {
  const errs: string[] = [];
  const spec = scenarioSpec(id);
  if (spec.inputs.metric && !(p.threshold > 0)) {
    errs.push(`Set a positive ${METRIC_DESCRIPTORS[p.metric].label} threshold.`);
  }
  if (spec.inputs.escalate) {
    if (!(p.escalateThreshold > 0)) errs.push("Set a positive upper (escalation) threshold.");
    else if (!(p.escalateThreshold > p.threshold)) {
      errs.push("The upper (escalation) threshold must exceed the first threshold.");
    }
  }
  if (spec.inputs.counterparty && p.counterparty.trim().length === 0) {
    errs.push("Enter the counterparty to back-to-back.");
  }
  return errs;
}

// --- flow analysis ---------------------------------------------------------

/** The derived flow summary for one book — the honest basis for a suggestion. */
export interface BookFlowSummary {
  bookId: string;
  bookName: string;
  /** Rolled-up net DV01 from the risk store, or `null` when not evaluated at this seam. */
  netDv01: number | null;
  /** Rolled-up signed net base-currency notional from the risk store. */
  netNotional: number;
  /** Rolled-up gross (sum of |notional|) from the risk store. */
  grossNotional: number;
  /** Positions rolled into the book (own + descendants'). */
  positionCount: number;
  /** Client-blotter deals attributed to this book (by `riskBookId`). */
  dealCount: number;
  /** Signed notional summed from those deals (BUY +, SELL −, TWO_WAY 0) — a cross-check. */
  dealSignedNotional: number;
  /** Peak limit utilisation across the book's caps, as a %, or `null` when no cap is set. */
  peakUtilizationPct: number | null;
  /** Whether there is enough flow to derive a suggestion (positions OR deals present). */
  hasFlow: boolean;
}

/** Signed notional of one deal (BUY long +, SELL short −, TWO_WAY 0). */
function dealSign(side: Deal["side"]): number {
  if (side === "BUY") return 1;
  if (side === "SELL") return -1;
  return 0;
}

/**
 * Summarise a book's flow from its risk-store row + the client-blotter deals. Pure:
 * the caller supplies the {@link RiskBookRisk} (from the risk-dashboard store) and the
 * full {@link Deal} list; this filters deals to the book and derives the honest figures.
 */
export function summariseBookFlow(book: RiskBookRisk, deals: readonly Deal[]): BookFlowSummary {
  const mine = deals.filter((d) => d.riskBookId === book.bookId);
  const dealSignedNotional = mine.reduce((acc, d) => acc + dealSign(d.side) * d.notional, 0);
  const peakFraction = book.limits.reduce<number | null>((acc, l) => {
    if (!Number.isFinite(l.fraction)) return acc;
    return acc === null ? l.fraction : Math.max(acc, l.fraction);
  }, null);
  return {
    bookId: book.bookId,
    bookName: book.name,
    netDv01: book.dv01,
    netNotional: book.netNotional,
    grossNotional: book.grossNotional,
    positionCount: book.positionCount,
    dealCount: mine.length,
    dealSignedNotional,
    peakUtilizationPct: peakFraction === null ? null : peakFraction * 100,
    hasFlow: book.positionCount > 0 || mine.length > 0,
  };
}

/**
 * The current magnitude of a metric for a book (|net notional| or |net DV01|), or
 * `null` when the metric is not evaluable (DV01 not yet computed at this seam).
 */
export function metricMagnitude(summary: BookFlowSummary, metric: ScenarioMetric): number | null {
  if (metric === "net_notional") return Math.abs(summary.netNotional);
  return summary.netDv01 === null ? null : Math.abs(summary.netDv01);
}

/**
 * Round a raw magnitude UP to a "nice" 1 / 2 / 5 × 10ⁿ figure — so a suggested cap
 * reads as a round number (e.g. 187m → 200m, 43k → 50k) rather than a noisy exposure.
 */
export function niceRoundUp(value: number): number {
  if (!(value > 0)) return 0;
  const exp = Math.floor(Math.log10(value));
  const base = Math.pow(10, exp);
  const mant = value / base;
  const nice = mant <= 1 ? 1 : mant <= 2 ? 2 : mant <= 5 ? 5 : 10;
  return nice * base;
}

/** A suggested cap + flatten threshold for a metric, or `null` when there is no flow. */
export interface ThresholdSuggestion {
  metric: ScenarioMetric;
  /** The current |metric| exposure the suggestion is derived from. */
  current: number;
  /** A rounded warehouse appetite (cap) with headroom above the current exposure. */
  cap: number;
  /** The flatten/hedge trigger — 0.8 × cap (hedge before the cap is fully used). */
  flattenThreshold: number;
}

/**
 * Derive a suggested cap + flatten threshold for a metric from a book's flow. The cap
 * is a nice round number ≥ 1.25 × the current exposure (headroom); the flatten trigger
 * is 0.8 × cap, so the policy hedges BEFORE the appetite is exhausted. Returns `null`
 * when the book has no flow OR the metric is not evaluable — never a fabricated number.
 */
export function suggestThresholds(
  summary: BookFlowSummary,
  metric: ScenarioMetric,
): ThresholdSuggestion | null {
  if (!summary.hasFlow) return null;
  const current = metricMagnitude(summary, metric);
  if (current === null || !(current > 0)) return null;
  const cap = niceRoundUp(current * 1.25);
  return { metric, current, cap, flattenThreshold: cap * 0.8 };
}
