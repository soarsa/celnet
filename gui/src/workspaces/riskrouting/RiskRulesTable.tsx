/**
 * RiskRulesTable — the home view of risk routing: every rule as a row of a
 * conventional, ordered table. Row order IS priority (first-match-wins); a row can
 * be dragged to re-weight it. Each row shows its plain-English description, its
 * destination book, an enabled toggle, a conflict badge, and Edit/Delete actions.
 * Purely presentational — the container owns the rules + all mutations.
 */
import { useState } from "react";

import { describeRule, type RiskRule, type RuleConflict } from "../../lib/riskRules";
import styles from "./RiskRoutingWorkspace.module.css";

interface RiskRulesTableProps {
  rules: readonly RiskRule[];
  bookLabel: (id: string) => string;
  /** Conflicts grouped by rule id (worst severity drives the badge). */
  conflictsByRule: ReadonlyMap<string, RuleConflict[]>;
  readOnly: boolean;
  onEdit: (index: number) => void;
  onDelete: (index: number) => void;
  onToggle: (index: number) => void;
  onReorder: (from: number, to: number) => void;
}

/** The worst severity among a rule's conflicts, or null when clean. */
function worst(conflicts: RuleConflict[] | undefined): "error" | "warn" | null {
  if (conflicts === undefined || conflicts.length === 0) return null;
  return conflicts.some((c) => c.severity === "error") ? "error" : "warn";
}

export function RiskRulesTable({
  rules,
  bookLabel,
  conflictsByRule,
  readOnly,
  onEdit,
  onDelete,
  onToggle,
  onReorder,
}: RiskRulesTableProps): React.ReactElement {
  const [dragFrom, setDragFrom] = useState<number | null>(null);

  if (rules.length === 0) {
    return (
      <p className={styles.tableEmpty} data-testid="rules-empty">
        No risk rules yet.{" "}
        {readOnly ? "None are defined." : "Use “Create risk rule” to add the first one."}
      </p>
    );
  }

  return (
    <table className={styles.rulesTable} data-testid="rules-table">
      <thead>
        <tr>
          <th scope="col" className={styles.colPri}>#</th>
          <th scope="col">Description</th>
          <th scope="col">Routes to</th>
          <th scope="col" className={styles.colEnabled}>Enabled</th>
          <th scope="col" className={styles.colConflict}>Conflict</th>
          <th scope="col" className={styles.colActions}>Actions</th>
        </tr>
      </thead>
      <tbody>
        {rules.map((rule, index) => {
          const conflicts = conflictsByRule.get(rule.id);
          const sev = worst(conflicts);
          const routesTo =
            rule.bookId !== null && rule.bookId.length > 0 ? bookLabel(rule.bookId) : "—";
          const isDefault = rule.conditions.length === 0;
          return (
            <tr
              key={rule.id}
              data-testid={`rule-row-${index}`}
              className={[
                styles.ruleTr,
                rule.enabled ? "" : styles.ruleTrDisabled,
                sev === "error" ? styles.ruleTrError : "",
              ]
                .filter(Boolean)
                .join(" ")}
              draggable={!readOnly}
              onDragStart={() => setDragFrom(index)}
              onDragOver={(e) => {
                if (!readOnly && dragFrom !== null) e.preventDefault();
              }}
              onDrop={(e) => {
                e.preventDefault();
                if (readOnly || dragFrom === null || dragFrom === index) {
                  setDragFrom(null);
                  return;
                }
                onReorder(dragFrom, index);
                setDragFrom(null);
              }}
            >
              <td className={styles.colPri}>
                <span className={styles.priNum}>{index + 1}</span>
                {!readOnly && (
                  <span className={styles.dragHandle} aria-hidden title="Drag to re-prioritise">
                    ⠿
                  </span>
                )}
              </td>
              <td className={styles.ruleDesc} data-testid={`rule-desc-${index}`}>
                {isDefault && <span className={styles.defaultTag}>DEFAULT</span>}
                {describeRule(rule, bookLabel)}
              </td>
              <td className={styles.ruleRoutes}>{routesTo}</td>
              <td className={styles.colEnabled}>
                <label className={styles.toggle}>
                  <input
                    type="checkbox"
                    checked={rule.enabled}
                    disabled={readOnly}
                    data-testid={`rule-toggle-${index}`}
                    aria-label={`${rule.enabled ? "Disable" : "Enable"} rule ${index + 1}`}
                    onChange={() => onToggle(index)}
                  />
                  <span>{rule.enabled ? "On" : "Off"}</span>
                </label>
              </td>
              <td className={styles.colConflict}>
                {sev === null ? (
                  <span className={styles.badgeOk} aria-label="No conflicts">
                    ✓
                  </span>
                ) : (
                  <span
                    className={sev === "error" ? styles.badgeError : styles.badgeWarn}
                    data-testid={`rule-conflict-${index}`}
                    title={conflicts?.map((c) => c.message).join("\n")}
                    role="alert"
                  >
                    {sev === "error" ? "⚠ error" : "⚠ warn"}
                  </span>
                )}
              </td>
              <td className={styles.colActions}>
                <button
                  type="button"
                  className={styles.ghostBtn}
                  data-testid={`rule-edit-${index}`}
                  onClick={() => onEdit(index)}
                >
                  {readOnly ? "View" : "Edit"}
                </button>
                {!readOnly && (
                  <button
                    type="button"
                    className={styles.dangerBtn}
                    data-testid={`rule-delete-${index}`}
                    onClick={() => onDelete(index)}
                  >
                    Delete
                  </button>
                )}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
