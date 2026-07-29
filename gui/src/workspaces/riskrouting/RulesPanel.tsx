/**
 * The "Rules" panel — the trader's "show me every rule" view. A rule is ONE ROUTE
 * TO A DESTINATION BOOK, not one row per branch permutation: the book-terminating
 * paths of the current graph ({@link enumerateRules}) are grouped by destination
 * book, so a book reached by several branches is a single rule with alternative
 * guards (`guard_a` OR `guard_b`). Rules are numbered in the order the router
 * REACHES each book (the book hit first is Rule 1). Each guard shows the ANDed
 * conditions to reach the book (a `no`-branch leg reads negated); the pure
 * fall-through leg (all no-branches) reads `otherwise`. A rule whose destination is
 * unset or unknown/disabled is flagged invalid with its reason. Paths that loop or
 * dangle are NOT rules — they never reach a book and are surfaced as graph defects
 * by the validator, not here; a graph with no complete route shows "No complete
 * rules yet". Clicking a rule highlights (the first of) its paths on the canvas.
 */
import { useMemo } from "react";

import type { RiskRoutingGraph } from "../../data/contract";
import { fieldSpec, opGlyph } from "../../lib/routeFields";
import { type PathCondition, enumerateRules } from "../../lib/routeTrace";
import { valueLabel } from "./nodeLabel";
import styles from "./RiskRoutingWorkspace.module.css";

interface RulesPanelProps {
  graph: RiskRoutingGraph;
  knownBookIds: ReadonlySet<string>;
  /** Desk-scoped book label resolver (`DESK / BOOK`). */
  bookLabel: (id: string) => string;
  /** The node ids of the currently highlighted rule (for the active row style). */
  activePath: number[] | null;
  /** Highlight a rule's path on the canvas (toggles when the same rule is re-picked). */
  onPick: (nodes: number[]) => void;
}

/** A single condition leg as readable text, e.g. `Currency = "EUR"` or `not Notional > 50m`. */
function conditionText(c: PathCondition): string {
  const body = `${fieldSpec(c.field).label} ${opGlyph(c.op)} ${valueLabel(c.value)}`;
  return c.branch === "onFalse" ? `not ${body}` : body;
}

/** Whether two node-id paths are the same sequence (for the active-row test). */
function samePath(a: number[] | null, b: number[]): boolean {
  return a !== null && a.length === b.length && a.every((v, i) => v === b[i]);
}

/**
 * A route reached by taking ONLY no-branches is the pure fall-through — the trader
 * reads it as "otherwise", not as a stack of negations. An empty guard (the entry is
 * itself the book) reads "always".
 */
function isFallThrough(guard: PathCondition[]): boolean {
  return guard.length > 0 && guard.every((c) => c.branch === "onFalse");
}

/** One guard (a single route to the book) rendered as its ANDed legs. */
function GuardText({ guard }: { guard: PathCondition[] }): React.ReactElement {
  if (guard.length === 0) return <span className={styles.ruleAlways}>always</span>;
  if (isFallThrough(guard)) return <span className={styles.ruleAlways}>otherwise</span>;
  return (
    <>
      {guard.map((c, j) => (
        <span key={j} className={styles.ruleCond}>
          {j > 0 && <span className={styles.ruleAnd}>and</span>}
          <span className={c.branch === "onFalse" ? styles.ruleCondNeg : undefined}>
            {conditionText(c)}
          </span>
        </span>
      ))}
    </>
  );
}

export function RulesPanel({
  graph,
  knownBookIds,
  bookLabel,
  activePath,
  onPick,
}: RulesPanelProps): React.ReactElement {
  const rules = useMemo(() => enumerateRules(graph, knownBookIds), [graph, knownBookIds]);

  return (
    <section className={styles.rules} aria-label="Routing rules" data-testid="rules-panel">
      <div className={styles.rulesHead}>
        <h2 className={styles.rulesTitle}>Rules</h2>
        <span className={styles.rulesCount}>
          {rules.length} rule{rules.length === 1 ? "" : "s"}
        </span>
      </div>

      {rules.length === 0 ? (
        <p className={styles.rulesEmpty}>
          No complete rules yet — every route loops or dangles. Wire a Book leaf as a destination.
        </p>
      ) : (
        <ol className={styles.rulesList}>
          {rules.map((r, i) => {
            const active = r.paths.some((p) => samePath(activePath, p));
            const landing = r.bookId !== null ? bookLabel(r.bookId) : null;
            return (
              <li key={i}>
                <button
                  type="button"
                  className={[
                    styles.ruleRow,
                    r.valid ? "" : styles.ruleInvalid,
                    active ? styles.ruleActive : "",
                  ]
                    .filter(Boolean)
                    .join(" ")}
                  onClick={() => onPick(r.paths[0] ?? [])}
                  data-testid={`rule-${i}`}
                  aria-current={active ? "true" : undefined}
                >
                  <span className={styles.ruleNum}>Rule {i + 1}</span>
                  <span className={styles.ruleWhen}>
                    {r.guards.map((guard, gi) => (
                      <span key={gi} className={styles.ruleGuard}>
                        {gi > 0 && <span className={styles.ruleOr}>or</span>}
                        <GuardText guard={guard} />
                      </span>
                    ))}
                  </span>
                  <span className={styles.ruleThen}>
                    <span className={styles.ruleArrow} aria-hidden>
                      →
                    </span>
                    {landing !== null ? (
                      <span className={styles.ruleBook}>{landing}</span>
                    ) : (
                      <span className={styles.ruleNoBook}>no book</span>
                    )}
                  </span>
                  {!r.valid && r.issue !== undefined && (
                    <span className={styles.ruleIssue} role="alert">
                      ⚠ {r.issue}
                    </span>
                  )}
                </button>
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}
