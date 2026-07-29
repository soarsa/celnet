/**
 * The "Rules" panel — the trader's "show me every rule" view. It enumerates EVERY
 * root-to-leaf path of the current graph ({@link enumeratePaths}) as a
 * human-readable rule, numbered in the exact order the router EVALUATES them (DFS
 * from the entry, `on_true` before `on_false`). Each row shows the ANDed conditions
 * along the path (a `no`-branch leg reads negated) and the landing `DESK / BOOK`.
 * A path that cannot terminate at a known enabled book (dangling / cycle / unknown
 * book) is flagged invalid with its reason. Clicking a rule highlights its path on
 * the canvas via the shared trace-highlight mechanism.
 */
import { useMemo } from "react";

import type { RiskRoutingGraph } from "../../data/contract";
import { fieldSpec, opGlyph } from "../../lib/routeFields";
import { type PathCondition, enumeratePaths } from "../../lib/routeTrace";
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

export function RulesPanel({
  graph,
  knownBookIds,
  bookLabel,
  activePath,
  onPick,
}: RulesPanelProps): React.ReactElement {
  const paths = useMemo(() => enumeratePaths(graph, knownBookIds), [graph, knownBookIds]);

  return (
    <section className={styles.rules} aria-label="Routing rules" data-testid="rules-panel">
      <div className={styles.rulesHead}>
        <h2 className={styles.rulesTitle}>Rules</h2>
        <span className={styles.rulesCount}>
          {paths.length} rule{paths.length === 1 ? "" : "s"}
        </span>
      </div>

      {paths.length === 0 ? (
        <p className={styles.rulesEmpty}>No rules yet — add a condition and a book leaf.</p>
      ) : (
        <ol className={styles.rulesList}>
          {paths.map((p, i) => {
            const active = samePath(activePath, p.nodes);
            const landing = p.bookId !== null ? bookLabel(p.bookId) : null;
            return (
              <li key={i}>
                <button
                  type="button"
                  className={[
                    styles.ruleRow,
                    p.valid ? "" : styles.ruleInvalid,
                    active ? styles.ruleActive : "",
                  ]
                    .filter(Boolean)
                    .join(" ")}
                  onClick={() => onPick(p.nodes)}
                  data-testid={`rule-${i}`}
                  aria-current={active ? "true" : undefined}
                >
                  <span className={styles.ruleNum}>Rule {i + 1}</span>
                  <span className={styles.ruleWhen}>
                    {p.conditions.length === 0 ? (
                      <span className={styles.ruleAlways}>always</span>
                    ) : (
                      p.conditions.map((c, j) => (
                        <span key={j} className={styles.ruleCond}>
                          {j > 0 && <span className={styles.ruleAnd}>and</span>}
                          <span
                            className={c.branch === "onFalse" ? styles.ruleCondNeg : undefined}
                          >
                            {conditionText(c)}
                          </span>
                        </span>
                      ))
                    )}
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
                  {!p.valid && p.issue !== undefined && (
                    <span className={styles.ruleIssue} role="alert">
                      ⚠ {p.issue}
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
