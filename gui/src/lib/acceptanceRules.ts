/**
 * The trader-facing RULES model for the INCOMING-QUOTE-ACCEPTANCE graph, and the exact
 * bridge to the wire {@link AcceptanceGraph}. The direct analogue of `lib/hedgeRules.ts`:
 * the wire model is a first-match-wins decision graph whose LEAVES ARE DECISIONS
 * (accept / reject / hold), and this module presents it as an ordered RULES TABLE —
 * each rule a plain `IF <ANDed conditions> THEN <decision>` — and compiles that table
 * back into the exact graph the server already accepts.
 *
 *   compileRulesToAcceptanceGraph   — rules (priority order) → a deterministic
 *                                     right-spine graph (first-match-wins, acyclic).
 *   decompileAcceptanceGraphToRules — the structural inverse used to LOAD a graph; a
 *                                     graph produced by the compiler round-trips exactly.
 *   describeAcceptanceRule          — a plain-English one-liner for a table row.
 *   detectAcceptanceRuleConflicts   — the structural-conflict checks the table surfaces.
 *
 * Layering: depends ONLY on the wire contract + the field registry + the decision
 * helpers (`lib/`); never imports a workspace component, so it stays unit-testable.
 */
import type {
  AcceptanceAction,
  AcceptanceField,
  AcceptanceGraph,
  AcceptanceNode,
  RouteOp,
  RouteValue,
} from "../data/contract";
import { acceptanceFieldSpec } from "./acceptanceFields";
import { defaultAcceptanceAction, describeAcceptanceAction } from "./acceptanceAction";
import { opGlyph } from "./routeOps";

/** One ANDed leg of an acceptance rule: `field op value`. `value` is always set. */
export interface AcceptanceRuleCondition {
  field: AcceptanceField;
  op: RouteOp;
  value: RouteValue;
}

/**
 * One trader acceptance rule: `IF <conditions ANDed> THEN <decision>`. The position of
 * a rule in the array is its PRIORITY — earlier rules win (first-match-wins). A rule
 * with an empty {@link conditions} list is the DEFAULT / catch-all (always matches)
 * and must be last.
 */
export interface AcceptanceRule {
  /** Stable client id (table-row identity; not carried on the wire). */
  id: string;
  /** The ANDed conditions; empty ⇒ the default/catch-all rule. */
  conditions: AcceptanceRuleCondition[];
  /** The decision fired when this rule matches. */
  action: AcceptanceAction;
  /** Whether this rule is active. Disabled rules are excluded from the graph. */
  enabled: boolean;
}

/** One detected structural conflict, attributed to a rule row. */
export interface AcceptanceRuleConflict {
  /** The offending rule's {@link AcceptanceRule.id}. */
  ruleId: string;
  /** `error` blocks Save; `warn` is advisory. */
  severity: "error" | "warn";
  /** A trader-readable description. */
  message: string;
}

let RULE_SEQ = 0;
/** Mint a fresh, unique acceptance-rule id (for a new table row). */
export function newAcceptanceRuleId(): string {
  RULE_SEQ += 1;
  return `arule-${Date.now().toString(36)}-${RULE_SEQ}`;
}

/** A fresh empty (catch-all) rule that ACCEPTS — the safe accept-all default leaf. */
export function newDefaultAcceptanceRule(): AcceptanceRule {
  return {
    id: newAcceptanceRuleId(),
    conditions: [],
    action: defaultAcceptanceAction("accept"),
    enabled: true,
  };
}

// --- compile ---------------------------------------------------------------

/**
 * Compile an ordered rule list into an {@link AcceptanceGraph}. The output is a
 * RIGHT-SPINE, first-match-wins decision tree (identical construction to `hedgeRules`):
 * rule i's conditions chain on `on_true` to its decision leaf; every condition's
 * `on_false` points at the entry of rule i+1; the trailing default rule is a bare
 * decision leaf. All edges point at strictly-later ids, so the graph is ALWAYS acyclic.
 */
export function compileRulesToAcceptanceGraph(rules: readonly AcceptanceRule[]): AcceptanceGraph {
  if (rules.length === 0) return { entry: 0, nodes: [] };

  let nextId = 0;
  const condIdsPerRule: number[][] = [];
  const leafPerRule: number[] = [];
  for (const rule of rules) {
    const condIds = rule.conditions.map(() => nextId++);
    condIdsPerRule.push(condIds);
    leafPerRule.push(nextId++);
  }

  const entryOf = (i: number): number => {
    const condIds = condIdsPerRule[i] ?? [];
    return condIds.length > 0 ? (condIds[0] as number) : (leafPerRule[i] as number);
  };

  const nodes: AcceptanceNode[] = [];
  for (let i = 0; i < rules.length; i += 1) {
    const rule = rules[i] as AcceptanceRule;
    const condIds = condIdsPerRule[i] as number[];
    const leaf = leafPerRule[i] as number;
    const onFalseTarget = i + 1 < rules.length ? entryOf(i + 1) : leaf;

    for (let j = 0; j < rule.conditions.length; j += 1) {
      const c = rule.conditions[j] as AcceptanceRuleCondition;
      const isLast = j === rule.conditions.length - 1;
      nodes.push({
        kind: "condition",
        id: condIds[j] as number,
        condition: {
          field: c.field,
          op: c.op,
          value: c.value,
          onTrue: isLast ? leaf : (condIds[j + 1] as number),
          onFalse: onFalseTarget,
        },
      });
    }
    nodes.push({ kind: "decision", id: leaf, action: rule.action });
  }

  return { entry: entryOf(0), nodes };
}

// --- decompile -------------------------------------------------------------

/** Coerce a possibly-null wire value into a concrete {@link RouteValue}. */
function coerceValue(op: RouteOp, value: RouteValue | null): RouteValue {
  if (value !== null) return value;
  switch (op) {
    case "in":
      return { kind: "list", values: [] };
    case "between":
      return { kind: "range", lo: 0, hi: 0 };
    case "gt":
    case "ge":
    case "lt":
    case "le":
      return { kind: "num", num: 0 };
    default:
      return { kind: "text", text: "" };
  }
}

/**
 * Load an {@link AcceptanceGraph} into the ordered rule list. Walks the FALSE spine from
 * the entry: each rule's entry is either a decision leaf (⇒ a default rule with no
 * conditions) or a condition chain (follow `on_true`, collecting each leg, until a
 * decision leaf gives the decision; the FIRST condition's `on_false` is the next rule's
 * entry). A graph produced by {@link compileRulesToAcceptanceGraph} inverts EXACTLY.
 */
export function decompileAcceptanceGraphToRules(graph: AcceptanceGraph): AcceptanceRule[] {
  const byId = new Map<number, AcceptanceNode>(graph.nodes.map((n) => [n.id, n]));
  const rules: AcceptanceRule[] = [];
  const seenEntries = new Set<number>();
  let entry: number | undefined = graph.entry;

  while (entry !== undefined && byId.has(entry) && !seenEntries.has(entry)) {
    seenEntries.add(entry);
    const entryNode = byId.get(entry) as AcceptanceNode;

    if (entryNode.kind === "decision") {
      rules.push({
        id: newAcceptanceRuleId(),
        conditions: [],
        action: entryNode.action,
        enabled: true,
      });
      break;
    }

    const nextEntry = entryNode.condition.onFalse;
    const conditions: AcceptanceRuleCondition[] = [];
    const chainSeen = new Set<number>();
    let cursor: AcceptanceNode | undefined = entryNode;
    while (cursor !== undefined && cursor.kind === "condition" && !chainSeen.has(cursor.id)) {
      chainSeen.add(cursor.id);
      const c = cursor.condition;
      conditions.push({ field: c.field, op: c.op, value: coerceValue(c.op, c.value) });
      cursor = byId.get(c.onTrue);
    }
    const action =
      cursor !== undefined && cursor.kind === "decision"
        ? cursor.action
        : defaultAcceptanceAction("accept");
    rules.push({ id: newAcceptanceRuleId(), conditions, action, enabled: true });
    entry = nextEntry;
  }

  return rules;
}

// --- describe --------------------------------------------------------------

/** A compact number for a rule description (50m, 1.5k, 10). */
function compactNumber(n: number): string {
  if (!Number.isFinite(n)) return "∞";
  return new Intl.NumberFormat("en-US", {
    notation: Math.abs(n) >= 1000 ? "compact" : "standard",
    maximumFractionDigits: 2,
  }).format(n);
}

/** Render a value literal plainly for a rule one-liner. */
function renderValue(v: RouteValue): string {
  switch (v.kind) {
    case "num":
      return compactNumber(v.num);
    case "text":
      return v.text.length > 0 ? v.text : "(empty)";
    case "list":
      return v.values.length > 0 ? `[${v.values.join(", ")}]` : "[]";
    case "range":
      return `${compactNumber(v.lo)}…${compactNumber(v.hi)}`;
  }
}

/** One ANDed leg as text, e.g. `Edge (bps) < 0.5`. */
function conditionText(c: AcceptanceRuleCondition): string {
  return `${acceptanceFieldSpec(c.field).label} ${opGlyph(c.op)} ${renderValue(c.value)}`;
}

/**
 * A plain-English one-liner for an acceptance rule, e.g.
 * `Edge (bps) < 0.5 → Reject · "below edge floor"`, or for the default rule
 * `Otherwise → Accept`.
 */
export function describeAcceptanceRule(rule: AcceptanceRule): string {
  const dest = describeAcceptanceAction(rule.action);
  if (rule.conditions.length === 0) return `Otherwise → ${dest}`;
  const guard = rule.conditions.map(conditionText).join(" AND ");
  return `${guard} → ${dest}`;
}

// --- conflicts -------------------------------------------------------------

/** Canonical signature of one condition (order-independent within a value list). */
function conditionSig(c: AcceptanceRuleCondition): string {
  const v = c.value;
  let vs: string;
  switch (v.kind) {
    case "num":
      vs = `n:${v.num}`;
      break;
    case "text":
      vs = `t:${v.text}`;
      break;
    case "list":
      vs = `l:${[...v.values].sort().join(",")}`;
      break;
    case "range":
      vs = `r:${v.lo}-${v.hi}`;
      break;
  }
  return `${c.field}|${c.op}|${vs}`;
}

/** The set of condition signatures for a rule (AND is order-independent). */
function conditionSet(rule: AcceptanceRule): Set<string> {
  return new Set(rule.conditions.map(conditionSig));
}

/** Whether every signature in `a` is also in `b` (a ⊆ b). */
function isSubset(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
  for (const s of a) if (!b.has(s)) return false;
  return true;
}

/**
 * The structural-conflict checks the table surfaces, over the ENABLED rules:
 *   1. Exact duplicate — two rules with an identical condition SET ⇒ ERROR.
 *   2. Shadowed / unreachable — an earlier rule whose conditions are a SUBSET of a
 *      later rule's (the earlier, more-general rule always wins first) ⇒ ERROR.
 *   3. Exactly one default — zero or more-than-one catch-all rule ⇒ ERROR.
 */
export function detectAcceptanceRuleConflicts(
  rules: readonly AcceptanceRule[],
): AcceptanceRuleConflict[] {
  const conflicts: AcceptanceRuleConflict[] = [];
  const active = rules.filter((r) => r.enabled);
  const sets = active.map(conditionSet);

  // 1 — exact duplicates.
  const seenSetSig = new Map<string, string>();
  for (let i = 0; i < active.length; i += 1) {
    const rule = active[i] as AcceptanceRule;
    const sig = [...(sets[i] as Set<string>)].sort().join("&&");
    if (seenSetSig.has(sig)) {
      conflicts.push({
        ruleId: rule.id,
        severity: "error",
        message: "Duplicate rule — identical conditions to an earlier rule.",
      });
    } else {
      seenSetSig.set(sig, rule.id);
    }
  }

  // 2 — shadowed / unreachable.
  for (let j = 0; j < active.length; j += 1) {
    for (let i = 0; i < j; i += 1) {
      if (isSubset(sets[i] as Set<string>, sets[j] as Set<string>)) {
        const byDefault = (active[i] as AcceptanceRule).conditions.length === 0;
        conflicts.push({
          ruleId: (active[j] as AcceptanceRule).id,
          severity: "error",
          message: byDefault
            ? "Unreachable — an earlier default (catch-all) rule matches everything first."
            : "Unreachable — shadowed by an earlier, more general rule.",
        });
        break;
      }
    }
  }

  // 3 — exactly one default.
  const defaults = active.filter((r) => r.conditions.length === 0);
  if (defaults.length === 0) {
    const last = active[active.length - 1];
    conflicts.push({
      ruleId: last !== undefined ? last.id : "",
      severity: "error",
      message: "No default rule — add a catch-all rule (no conditions) as the last row.",
    });
  } else if (defaults.length > 1) {
    for (const d of defaults) {
      conflicts.push({
        ruleId: d.id,
        severity: "error",
        message: "Multiple default rules — only one catch-all is allowed.",
      });
    }
  }

  return conflicts;
}
