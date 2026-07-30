/**
 * The trader-facing RULES model for FI risk routing, and the exact bridge to the
 * wire {@link RiskRoutingGraph}. The wire model is an unchanged first-match-wins
 * decision graph; this module lets the GUI present it as a conventional ORDERED
 * RULES TABLE — each rule a plain `IF <ANDed conditions> THEN <risk book>` — and
 * compile that table back into the exact graph the server already accepts.
 *
 *   compileRulesToGraph  — rules (priority order) → a deterministic right-spine
 *                          graph (first-match-wins, always acyclic, no user wiring).
 *   decompileGraphToRules — the structural inverse used to LOAD a graph into the
 *                          table. A graph produced by {@link compileRulesToGraph}
 *                          round-trips exactly; a legacy hand-wired graph is
 *                          best-effort (it walks the on_false spine + each rule's
 *                          on_true chain, which recovers any first-match-wins graph
 *                          and degrades gracefully on arbitrary ones).
 *   describeRule         — a plain-English one-liner for a table row.
 *   detectRuleConflicts  — the four logical-conflict checks the table surfaces.
 *
 * Layering: this file depends ONLY on the wire contract + the field registry
 * (`lib/`); it never imports a workspace component, so it stays unit-testable and
 * reusable by any client surface.
 */
import type {
  RiskRoutingGraph,
  RouteField,
  RouteOp,
  RouteValue,
  RoutingNode,
} from "../data/contract";
import { fieldSpec, opGlyph } from "./routeFields";

/** One ANDed leg of a rule: `field op value`. `value` is always set (never null). */
export interface RuleCondition {
  field: RouteField;
  op: RouteOp;
  value: RouteValue;
}

/**
 * One trader routing rule: `IF <conditions ANDed> THEN route risk into <bookId>`.
 * The position of a rule in the array is its PRIORITY — earlier rules win
 * (first-match-wins). A rule with an empty {@link conditions} list is the
 * DEFAULT / catch-all (it always matches) and must be last.
 */
export interface RiskRule {
  /** Stable client id (table-row identity; not carried on the wire). */
  id: string;
  /** The ANDed conditions; empty ⇒ the default/catch-all rule. */
  conditions: RuleCondition[];
  /** The destination risk-book id, or `null` when not yet chosen. */
  bookId: string | null;
  /** Whether this rule is active. Disabled rules are excluded from the graph. */
  enabled: boolean;
}

/** One detected logical conflict, attributed to a rule row. */
export interface RuleConflict {
  /** The offending rule's {@link RiskRule.id}. */
  ruleId: string;
  /** `error` blocks Save; `warn` is advisory. */
  severity: "error" | "warn";
  /** A trader-readable description. */
  message: string;
}

/** A monotonic id source for freshly-created rules. */
let RULE_SEQ = 0;
/** Mint a fresh, unique rule id (for a new table row). */
export function newRuleId(): string {
  RULE_SEQ += 1;
  return `rule-${Date.now().toString(36)}-${RULE_SEQ}`;
}

// --- compile ---------------------------------------------------------------

/**
 * Compile an ordered rule list into a {@link RiskRoutingGraph}. The output is a
 * RIGHT-SPINE, first-match-wins decision tree:
 *
 *   - Rule i's conditions are chained on `on_true`: condition j's `on_true` is
 *     condition j+1, and the LAST condition's `on_true` is rule i's book leaf.
 *   - EVERY condition's `on_false` points to the ENTRY of rule i+1 (any failed leg
 *     abandons this rule and tries the next — the AND / first-match semantics).
 *   - The trailing default rule (`conditions: []`) is a bare book leaf, so every
 *     `on_false` chain terminates there.
 *
 * All edges point at strictly-later-allocated node ids, so the graph is ALWAYS
 * acyclic — there is no user wiring to get wrong. Deterministic: the same rules
 * always produce byte-identical nodes. A rule with no following default has its
 * trailing `on_false` legs pointed at its own book leaf (best-effort; a
 * well-formed table always ends in a default).
 */
export function compileRulesToGraph(rules: readonly RiskRule[]): RiskRoutingGraph {
  if (rules.length === 0) return { entry: 0, nodes: [] };

  // Pass 1 — allocate node ids so forward references (on_false → next rule) resolve.
  let nextId = 0;
  const condIdsPerRule: number[][] = [];
  const bookLeafPerRule: number[] = [];
  for (const rule of rules) {
    const condIds = rule.conditions.map(() => nextId++);
    condIdsPerRule.push(condIds);
    bookLeafPerRule.push(nextId++);
  }

  const entryOf = (i: number): number => {
    const condIds = condIdsPerRule[i] ?? [];
    return condIds.length > 0 ? (condIds[0] as number) : (bookLeafPerRule[i] as number);
  };

  // Pass 2 — emit nodes.
  const nodes: RoutingNode[] = [];
  for (let i = 0; i < rules.length; i += 1) {
    const rule = rules[i] as RiskRule;
    const condIds = condIdsPerRule[i] as number[];
    const bookLeaf = bookLeafPerRule[i] as number;
    // Where a failed leg goes: the next rule's entry, or (no next rule) this rule's
    // own book leaf — a total, acyclic fallback for an ill-formed, default-less list.
    const onFalseTarget = i + 1 < rules.length ? entryOf(i + 1) : bookLeaf;

    for (let j = 0; j < rule.conditions.length; j += 1) {
      const c = rule.conditions[j] as RuleCondition;
      const isLast = j === rule.conditions.length - 1;
      nodes.push({
        kind: "condition",
        id: condIds[j] as number,
        condition: {
          field: c.field,
          op: c.op,
          value: c.value,
          onTrue: isLast ? bookLeaf : (condIds[j + 1] as number),
          onFalse: onFalseTarget,
        },
      });
    }
    nodes.push({ kind: "book", id: bookLeaf, bookId: rule.bookId ?? "" });
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
 * Load a {@link RiskRoutingGraph} into the ordered rule list. Walks the FALSE spine
 * from the entry: each rule's entry is either a book leaf (⇒ a default rule with no
 * conditions) or a condition chain (follow `on_true`, collecting each leg, until a
 * book leaf gives the destination; the FIRST condition's `on_false` is the next
 * rule's entry). A graph produced by {@link compileRulesToGraph} inverts EXACTLY
 * (conditions, order, and destinations preserved); an arbitrary legacy graph is
 * decoded best-effort and bounded (per-rule + per-chain visited sets guard cycles).
 */
export function decompileGraphToRules(graph: RiskRoutingGraph): RiskRule[] {
  const byId = new Map<number, RoutingNode>(graph.nodes.map((n) => [n.id, n]));
  const rules: RiskRule[] = [];
  const seenEntries = new Set<number>();
  let entry: number | undefined = graph.entry;

  while (entry !== undefined && byId.has(entry) && !seenEntries.has(entry)) {
    seenEntries.add(entry);
    const entryNode = byId.get(entry) as RoutingNode;

    // A book leaf as a rule entry is the unconditional catch-all (default) rule.
    if (entryNode.kind === "book") {
      rules.push({
        id: newRuleId(),
        conditions: [],
        bookId: entryNode.bookId.length > 0 ? entryNode.bookId : null,
        enabled: true,
      });
      break;
    }

    // Otherwise collect this rule's ANDed conditions along the on_true chain.
    const nextEntry = entryNode.condition.onFalse;
    const conditions: RuleCondition[] = [];
    const chainSeen = new Set<number>();
    let cursor: RoutingNode | undefined = entryNode;
    while (cursor !== undefined && cursor.kind === "condition" && !chainSeen.has(cursor.id)) {
      chainSeen.add(cursor.id);
      const c = cursor.condition;
      conditions.push({ field: c.field, op: c.op, value: coerceValue(c.op, c.value) });
      cursor = byId.get(c.onTrue);
    }
    const bookId =
      cursor !== undefined && cursor.kind === "book" && cursor.bookId.length > 0
        ? cursor.bookId
        : null;
    rules.push({ id: newRuleId(), conditions, bookId, enabled: true });
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

/** Render a value literal plainly (no decorative quotes) for a rule one-liner. */
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

/** One ANDed leg as text, e.g. `Product = bond`. */
function conditionText(c: RuleCondition): string {
  return `${fieldSpec(c.field).label} ${opGlyph(c.op)} ${renderValue(c.value)}`;
}

/**
 * A plain-English one-liner for a rule, e.g.
 * `Product = bond AND Desk = marex → RATES / GOVIES`, or for the default rule
 * `Otherwise → RATES / GOVIES`. `bookLabel` resolves a book id to its
 * desk-scoped display label; an unset destination reads `(no book)`.
 */
export function describeRule(rule: RiskRule, bookLabel: (id: string) => string): string {
  const dest =
    rule.bookId !== null && rule.bookId.length > 0 ? bookLabel(rule.bookId) : "(no portfolio)";
  if (rule.conditions.length === 0) return `Otherwise → ${dest}`;
  const guard = rule.conditions.map(conditionText).join(" AND ");
  return `${guard} → ${dest}`;
}

// --- conflicts -------------------------------------------------------------

/** Canonical signature of one condition (order-independent within a value list). */
function conditionSig(c: RuleCondition): string {
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
function conditionSet(rule: RiskRule): Set<string> {
  return new Set(rule.conditions.map(conditionSig));
}

/** Whether every signature in `a` is also in `b` (a ⊆ b). */
function isSubset(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
  for (const s of a) if (!b.has(s)) return false;
  return true;
}

/**
 * The four logical-conflict checks the table surfaces, over the ENABLED rules
 * (disabled rules never route, so they cannot conflict):
 *
 *   1. Exact duplicate — two rules with an identical condition SET ⇒ ERROR.
 *   2. Shadowed / unreachable — an earlier rule whose conditions are a SUBSET of a
 *      later rule's (the earlier, more-general rule always wins first, so the later
 *      never fires); the empty set of a default subsumes everything, so a default
 *      that is not last shadows every rule after it ⇒ ERROR on the shadowed rule.
 *   3. Exactly one default — zero or more-than-one catch-all (`conditions: []`)
 *      rule ⇒ ERROR.
 *   4. Duplicate destination — an ENABLED book targeted by ≥2 rules ⇒ WARN.
 *
 * `enabledBookIds` scopes check 4 to real (enabled) destinations — a rule pointing
 * at a disabled/unknown book is a graph-validity error surfaced elsewhere, not a
 * duplication warning.
 */
export function detectRuleConflicts(
  rules: readonly RiskRule[],
  enabledBookIds: ReadonlySet<string>,
): RuleConflict[] {
  const conflicts: RuleConflict[] = [];
  const active = rules.filter((r) => r.enabled);
  const sets = active.map(conditionSet);

  // 1 — exact duplicates (identical condition set as an earlier active rule).
  const seenSetSig = new Map<string, string>(); // canonical-set → first rule id
  for (let i = 0; i < active.length; i += 1) {
    const rule = active[i] as RiskRule;
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

  // 2 — shadowed / unreachable: an earlier rule's conditions ⊆ this rule's.
  for (let j = 0; j < active.length; j += 1) {
    for (let i = 0; i < j; i += 1) {
      if (isSubset(sets[i] as Set<string>, sets[j] as Set<string>)) {
        const byDefault = (active[i] as RiskRule).conditions.length === 0;
        conflicts.push({
          ruleId: (active[j] as RiskRule).id,
          severity: "error",
          message: byDefault
            ? "Unreachable — an earlier default (catch-all) rule matches everything first."
            : "Unreachable — shadowed by an earlier, more general rule.",
        });
        break; // one shadow report per rule is enough
      }
    }
  }

  // 3 — exactly one default (catch-all) rule.
  const defaults = active.filter((r) => r.conditions.length === 0);
  if (defaults.length === 0) {
    const last = active[active.length - 1];
    const anchor = last !== undefined ? last.id : "";
    conflicts.push({
      ruleId: anchor,
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

  // 4 — duplicate enabled destination (advisory).
  const bookCount = new Map<string, number>();
  for (const r of active) {
    if (r.bookId !== null && enabledBookIds.has(r.bookId)) {
      bookCount.set(r.bookId, (bookCount.get(r.bookId) ?? 0) + 1);
    }
  }
  for (const r of active) {
    if (r.bookId !== null && (bookCount.get(r.bookId) ?? 0) >= 2) {
      conflicts.push({
        ruleId: r.id,
        severity: "warn",
        message: "Another rule already routes to this book.",
      });
    }
  }

  return conflicts;
}
