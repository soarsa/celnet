/**
 * AcceptanceActionEditor — the acceptance-graph LEAF editor. Where the hedge editor's
 * terminal is an exit-action and the routing editor's is a desk→book picker, the
 * acceptance terminal is a DECISION: an action-kind picker (Accept / Reject / Hold for
 * review) plus a `reason` field for the two that carry one. Every change emits a
 * fully-formed {@link AcceptanceAction} so the compiled graph is always total.
 */
import type { AcceptanceAction, AcceptanceActionKind } from "../../data/contract";
import {
  ACCEPTANCE_ACTION_KINDS,
  acceptanceActionHint,
  acceptanceActionLabel,
  actionUsesReason,
  defaultAcceptanceAction,
} from "../../lib/acceptanceAction";
import rr from "../riskrouting/RiskRoutingWorkspace.module.css";

interface AcceptanceActionEditorProps {
  action: AcceptanceAction;
  readOnly: boolean;
  onChange: (next: AcceptanceAction) => void;
}

export function AcceptanceActionEditor({
  action,
  readOnly,
  onChange,
}: AcceptanceActionEditorProps): React.ReactElement {
  // Swapping the kind resets to that kind's defaults but preserves any typed reason so a
  // re-pick (reject ⇄ hold) is not destructive.
  const setKind = (kind: AcceptanceActionKind): void => {
    onChange({ ...defaultAcceptanceAction(kind), reason: action.reason });
  };

  return (
    <div data-testid="acceptance-action-editor">
      <div className={rr.destRow}>
        <label className={rr.editorField}>
          <span className={rr.fieldLabel}>Decision</span>
          <select
            className={rr.select}
            value={action.kind}
            disabled={readOnly}
            data-testid="acceptance-action-kind"
            onChange={(e) => setKind(e.target.value as AcceptanceActionKind)}
          >
            {ACCEPTANCE_ACTION_KINDS.map((k) => (
              <option key={k} value={k}>
                {acceptanceActionLabel(k)}
              </option>
            ))}
          </select>
        </label>
      </div>
      <p className={rr.ruleEditorHint}>{acceptanceActionHint(action.kind)}</p>

      {actionUsesReason(action.kind) && (
        <label className={rr.editorField}>
          <span className={rr.fieldLabel}>
            {action.kind === "reject" ? "Rejection reason" : "Review reason"}
          </span>
          <input
            className={rr.input}
            type="text"
            value={action.reason}
            disabled={readOnly}
            data-testid="acceptance-action-reason"
            placeholder={
              action.kind === "reject"
                ? "e.g. below edge floor"
                : "e.g. stale quote — desk to review"
            }
            onChange={(e) => onChange({ ...action, reason: e.target.value })}
          />
        </label>
      )}
    </div>
  );
}
