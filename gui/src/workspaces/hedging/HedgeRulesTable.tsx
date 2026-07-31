/**
 * HedgeRulesTable — the home view of the hedge exit policy: every rule as a row of a
 * conventional, ordered table (row order IS priority; first-match-wins). Mirrors the
 * risk-routing {@link ../riskrouting/RiskRulesTable}, but the destination column shows
 * the terminal EXIT ACTION instead of a book. Purely presentational.
 */
import { useState } from "react";

import { describeHedgeRule, type HedgeRule, type HedgeRuleConflict } from "../../lib/hedgeRules";
import { describeExitAction } from "../../lib/hedgeExit";
import rr from "../riskrouting/RiskRoutingWorkspace.module.css";

interface HedgeRulesTableProps {
  rules: readonly HedgeRule[];
  conflictsByRule: ReadonlyMap<string, HedgeRuleConflict[]>;
  readOnly: boolean;
  onEdit: (index: number) => void;
  onDelete: (index: number) => void;
  onToggle: (index: number) => void;
  onReorder: (from: number, to: number) => void;
}

function worst(conflicts: HedgeRuleConflict[] | undefined): "error" | "warn" | null {
  if (conflicts === undefined || conflicts.length === 0) return null;
  return conflicts.some((c) => c.severity === "error") ? "error" : "warn";
}

export function HedgeRulesTable({
  rules,
  conflictsByRule,
  readOnly,
  onEdit,
  onDelete,
  onToggle,
  onReorder,
}: HedgeRulesTableProps): React.ReactElement {
  const [dragFrom, setDragFrom] = useState<number | null>(null);

  if (rules.length === 0) {
    return (
      <p className={rr.tableEmpty} data-testid="hedge-rules-empty">
        No hedge rules yet.{" "}
        {readOnly ? "None are defined." : "Use “Create hedge rule” to add the first one."}
      </p>
    );
  }

  return (
    <table className={rr.rulesTable} data-testid="hedge-rules-table">
      <thead>
        <tr>
          <th scope="col" className={rr.colPri}>#</th>
          <th scope="col">Description</th>
          <th scope="col">Exit action</th>
          <th scope="col" className={rr.colEnabled}>Enabled</th>
          <th scope="col" className={rr.colConflict}>Conflict</th>
          <th scope="col" className={rr.colActions}>Actions</th>
        </tr>
      </thead>
      <tbody>
        {rules.map((rule, index) => {
          const conflicts = conflictsByRule.get(rule.id);
          const sev = worst(conflicts);
          const isDefault = rule.conditions.length === 0;
          return (
            <tr
              key={rule.id}
              data-testid={`hedge-rule-row-${index}`}
              className={[
                rr.ruleTr,
                rule.enabled ? "" : rr.ruleTrDisabled,
                sev === "error" ? rr.ruleTrError : "",
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
              <td className={rr.colPri}>
                <span className={rr.priNum}>{index + 1}</span>
                {!readOnly && (
                  <span className={rr.dragHandle} aria-hidden title="Drag to re-prioritise">
                    ⠿
                  </span>
                )}
              </td>
              <td className={rr.ruleDesc} data-testid={`hedge-rule-desc-${index}`}>
                {isDefault && <span className={rr.defaultTag}>DEFAULT</span>}
                {describeHedgeRule(rule)}
              </td>
              <td className={rr.ruleRoutes}>{describeExitAction(rule.action)}</td>
              <td className={rr.colEnabled}>
                <label className={rr.toggle}>
                  <input
                    type="checkbox"
                    checked={rule.enabled}
                    disabled={readOnly}
                    data-testid={`hedge-rule-toggle-${index}`}
                    aria-label={`${rule.enabled ? "Disable" : "Enable"} rule ${index + 1}`}
                    onChange={() => onToggle(index)}
                  />
                  <span>{rule.enabled ? "On" : "Off"}</span>
                </label>
              </td>
              <td className={rr.colConflict}>
                {sev === null ? (
                  <span className={rr.badgeOk} aria-label="No conflicts">
                    ✓
                  </span>
                ) : (
                  <span
                    className={sev === "error" ? rr.badgeError : rr.badgeWarn}
                    data-testid={`hedge-rule-conflict-${index}`}
                    title={conflicts?.map((c) => c.message).join("\n")}
                    role="alert"
                  >
                    {sev === "error" ? "⚠ error" : "⚠ warn"}
                  </span>
                )}
              </td>
              <td className={rr.colActions}>
                <button
                  type="button"
                  className={rr.ghostBtn}
                  data-testid={`hedge-rule-edit-${index}`}
                  onClick={() => onEdit(index)}
                >
                  {readOnly ? "View" : "Edit"}
                </button>
                {!readOnly && (
                  <button
                    type="button"
                    className={rr.dangerBtn}
                    data-testid={`hedge-rule-delete-${index}`}
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
