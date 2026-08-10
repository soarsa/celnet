/**
 * HedgeConflictPanel — the prominent conflict SUMMARY shown above the exit-policy rules
 * table. It makes rule conflicts impossible to miss: each structural conflict is listed
 * with the offending rule NUMBER(s), a severity chip, and a plain human explanation
 * ("Rule 3 is unreachable — shadowed by Rule 1"); genuine graph-validation defects are
 * listed alongside. When the policy is clean it renders a small "✓ No conflicts"
 * affirmation so the trader always knows the check ran. Purely presentational — the
 * container resolves rule numbers from the ordered {@link HedgeRule} list.
 */
import type { HedgeRule, HedgeRuleConflict } from "../../lib/hedgeRules";
import type { HedgeValidationIssue } from "../../lib/hedgeTrace";
import styles from "./HedgingWorkspace.module.css";

interface HedgeConflictPanelProps {
  rules: readonly HedgeRule[];
  conflicts: readonly HedgeRuleConflict[];
  graphIssues: readonly HedgeValidationIssue[];
}

/** Resolve a rule id to its 1-based position in the ordered table (0 ⇒ not found). */
function ruleNumber(rules: readonly HedgeRule[], id: string): number {
  return rules.findIndex((r) => r.id === id) + 1;
}

/** "Rule 3" / "Rules 3 & 5" for a set of related ids (dropping any not in the table). */
function relatedText(rules: readonly HedgeRule[], ids: readonly string[] | undefined): string {
  if (ids === undefined || ids.length === 0) return "";
  const ns = ids.map((id) => ruleNumber(rules, id)).filter((n) => n > 0).sort((a, b) => a - b);
  if (ns.length === 0) return "";
  const label = ns.length === 1 ? "Rule" : "Rules";
  return ` (see ${label} ${ns.join(" & ")})`;
}

export function HedgeConflictPanel({
  rules,
  conflicts,
  graphIssues,
}: HedgeConflictPanelProps): React.ReactElement {
  const total = conflicts.length + graphIssues.length;

  if (total === 0) {
    return (
      <div className={styles.conflictAffirm} data-testid="hedge-conflict-affirm" role="status">
        <span aria-hidden="true">✓</span> No conflicts — every rule is reachable and there is exactly one
        default.
      </div>
    );
  }

  const errorCount = conflicts.filter((c) => c.severity === "error").length + graphIssues.length;

  return (
    <section
      className={styles.conflictPanel}
      data-testid="hedge-conflict-panel"
      aria-labelledby="hedge-conflict-heading"
      role="alert"
    >
      <h4 id="hedge-conflict-heading" className={styles.conflictPanelHead}>
        <span aria-hidden="true">⚠</span> {total} conflict{total === 1 ? "" : "s"} to review
        {errorCount > 0 && <span className={styles.conflictPanelSub}>{errorCount} would change how the policy fires</span>}
      </h4>
      <ul className={styles.conflictItems}>
        {conflicts.map((c, i) => {
          const n = ruleNumber(rules, c.ruleId);
          return (
            <li
              key={`c-${i}`}
              className={styles.conflictItem}
              data-testid={`hedge-conflict-item-${n > 0 ? n : "x"}`}
            >
              <span className={c.severity === "error" ? styles.conflictSevError : styles.conflictSevWarn}>
                {c.severity === "error" ? "Error" : "Warning"}
              </span>
              <span className={styles.conflictItemBody}>
                {n > 0 && <strong>Rule {n}</strong>} — {c.message}
                {relatedText(rules, c.relatedRuleIds)}
              </span>
            </li>
          );
        })}
        {graphIssues.map((g, i) => (
          <li key={`g-${i}`} className={styles.conflictItem} data-testid="hedge-graph-issue">
            <span className={styles.conflictSevError}>Error</span>
            <span className={styles.conflictItemBody}>
              {g.node !== null && <strong>Node #{g.node}</strong>} {g.node !== null ? "— " : ""}
              {g.message}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}
