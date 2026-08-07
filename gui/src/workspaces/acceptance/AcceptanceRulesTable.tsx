/**
 * AcceptanceRulesTable — the home view of the acceptance policy: every rule as a row of
 * a conventional, ordered table (row order IS priority; first-match-wins). Mirrors the
 * hedge {@link ../hedging/HedgeRulesTable}, but the destination column shows the terminal
 * DECISION (a coloured accept/reject/hold chip) instead of an exit action. Purely
 * presentational.
 */
import { useEffect, useRef, useState } from "react";

import {
  describeAcceptanceRule,
  type AcceptanceRule,
  type AcceptanceRuleConflict,
} from "../../lib/acceptanceRules";
import { acceptanceActionLabel } from "../../lib/acceptanceAction";
import rr from "../riskrouting/RiskRoutingWorkspace.module.css";
import styles from "./AcceptanceWorkspace.module.css";

interface AcceptanceRulesTableProps {
  rules: readonly AcceptanceRule[];
  conflictsByRule: ReadonlyMap<string, AcceptanceRuleConflict[]>;
  readOnly: boolean;
  onEdit: (index: number) => void;
  onDelete: (index: number) => void;
  onToggle: (index: number) => void;
  onReorder: (from: number, to: number) => void;
  /** The id of a just-seeded rule to scroll to + highlight (e.g. from a flow row). */
  highlightRuleId?: string | null;
}

function worst(conflicts: AcceptanceRuleConflict[] | undefined): "error" | "warn" | null {
  if (conflicts === undefined || conflicts.length === 0) return null;
  return conflicts.some((c) => c.severity === "error") ? "error" : "warn";
}

export function AcceptanceRulesTable({
  rules,
  conflictsByRule,
  readOnly,
  onEdit,
  onDelete,
  onToggle,
  onReorder,
  highlightRuleId,
}: AcceptanceRulesTableProps): React.ReactElement {
  const [dragFrom, setDragFrom] = useState<number | null>(null);
  const highlightRef = useRef<HTMLTableRowElement | null>(null);

  // Scroll the just-seeded row into view so the trader immediately sees which rule was
  // added by "Create acceptance rule" from a live-flow row.
  useEffect(() => {
    if (highlightRuleId && highlightRef.current) {
      highlightRef.current.scrollIntoView({ block: "nearest", behavior: "smooth" });
    }
  }, [highlightRuleId]);

  if (rules.length === 0) {
    return (
      <p className={rr.tableEmpty} data-testid="acceptance-rules-empty">
        No acceptance rules yet.{" "}
        {readOnly ? "None are defined." : "Use “Create acceptance rule” to add the first one."}
      </p>
    );
  }

  return (
    <table className={rr.rulesTable} data-testid="acceptance-rules-table">
      <thead>
        <tr>
          <th scope="col" className={rr.colPri}>#</th>
          <th scope="col">Description</th>
          <th scope="col">Decision</th>
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
          const isSeeded = highlightRuleId != null && rule.id === highlightRuleId;
          return (
            <tr
              key={rule.id}
              ref={isSeeded ? highlightRef : undefined}
              data-testid={`acceptance-rule-row-${index}`}
              data-seeded={isSeeded ? "true" : undefined}
              className={[
                rr.ruleTr,
                rule.enabled ? "" : rr.ruleTrDisabled,
                sev === "error" ? rr.ruleTrError : "",
                isSeeded ? styles.ruleTrSeeded : "",
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
              <td className={rr.ruleDesc} data-testid={`acceptance-rule-desc-${index}`}>
                {isDefault && <span className={rr.defaultTag}>DEFAULT</span>}
                {describeAcceptanceRule(rule)}
              </td>
              <td className={rr.ruleRoutes}>
                <span
                  className={`${styles.decisionChip} ${styles[`dec_${rule.action.kind}`]}`}
                  data-testid={`acceptance-rule-decision-${index}`}
                  title={rule.action.reason.length > 0 ? rule.action.reason : undefined}
                >
                  {acceptanceActionLabel(rule.action.kind)}
                </span>
              </td>
              <td className={rr.colEnabled}>
                <label className={rr.toggle}>
                  <input
                    type="checkbox"
                    checked={rule.enabled}
                    disabled={readOnly}
                    data-testid={`acceptance-rule-toggle-${index}`}
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
                    data-testid={`acceptance-rule-conflict-${index}`}
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
                  data-testid={`acceptance-rule-edit-${index}`}
                  onClick={() => onEdit(index)}
                >
                  {readOnly ? "View" : "Edit"}
                </button>
                {!readOnly && (
                  <button
                    type="button"
                    className={rr.dangerBtn}
                    data-testid={`acceptance-rule-delete-${index}`}
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
