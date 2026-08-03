/**
 * AcceptanceRuleEditor — the per-rule builder for an acceptance policy. The SAME shape
 * as the hedge {@link ../hedging/HedgeRuleEditor} / risk-routing RuleEditor: drag field
 * chips into the conditions area to add `field op value` legs (ANDed), edit each leg
 * with the SHARED typed {@link ../riskrouting/ValueEditor} control, and pick the
 * terminal — but the terminal is an {@link AcceptanceActionEditor} (an accept / reject /
 * hold decision) instead of an exit action or a desk→book picker. A live preview shows
 * the rule in plain English.
 */
import { useMemo, useState } from "react";

import type { AcceptanceAction, AcceptanceField, RouteOp, RouteValue } from "../../data/contract";
import {
  ACCEPTANCE_ASSET_CLASS_VALUES,
  ACCEPTANCE_FIELD_REGISTRY,
  ACCEPTANCE_SIDE_VALUES,
  acceptanceFieldSpec,
} from "../../lib/acceptanceFields";
import {
  defaultOpForAcceptanceField,
  defaultValueForOp,
} from "../../lib/acceptanceGraphOps";
import {
  describeAcceptanceRule,
  type AcceptanceRule,
  type AcceptanceRuleCondition,
} from "../../lib/acceptanceRules";
import { opLabel } from "../../lib/routeOps";
import { ValueEditor, type ValueOption } from "../riskrouting/ValueEditor";
import { decodeAcceptanceDrag, AcceptanceFieldPalette } from "./AcceptanceFieldPalette";
import { AcceptanceActionEditor } from "./AcceptanceActionEditor";
import rr from "../riskrouting/RiskRoutingWorkspace.module.css";

interface AcceptanceRuleEditorProps {
  draft: AcceptanceRule;
  isNew: boolean;
  readOnly: boolean;
  onSave: (rule: AcceptanceRule) => void;
  onCancel: () => void;
}

/** Build the enum value options for an acceptance field (static advisory lists). */
function enumOptionsFor(field: AcceptanceField): ValueOption[] {
  switch (field) {
    case "side":
      return ACCEPTANCE_SIDE_VALUES.map((v) => ({ value: v, label: v }));
    case "asset_class":
      return ACCEPTANCE_ASSET_CLASS_VALUES.map((v) => ({ value: v, label: v }));
    default:
      return [];
  }
}

export function AcceptanceRuleEditor({
  draft,
  isNew,
  readOnly,
  onSave,
  onCancel,
}: AcceptanceRuleEditorProps): React.ReactElement {
  const [conditions, setConditions] = useState<AcceptanceRuleCondition[]>(draft.conditions);
  const [action, setAction] = useState<AcceptanceAction>(draft.action);

  const isDefault = conditions.length === 0;

  const addCondition = (field: AcceptanceField): void => {
    const op = defaultOpForAcceptanceField(field);
    setConditions((cs) => [...cs, { field, op, value: defaultValueForOp(field, op) }]);
  };
  const removeCondition = (idx: number): void =>
    setConditions((cs) => cs.filter((_, i) => i !== idx));
  const rebindField = (idx: number, field: AcceptanceField): void =>
    setConditions((cs) =>
      cs.map((c, i) => {
        if (i !== idx) return c;
        const op = defaultOpForAcceptanceField(field);
        return { field, op, value: defaultValueForOp(field, op) };
      }),
    );
  const setOp = (idx: number, op: RouteOp): void =>
    setConditions((cs) =>
      cs.map((c, i) => (i === idx ? { ...c, op, value: defaultValueForOp(c.field, op) } : c)),
    );
  const setVal = (idx: number, value: RouteValue): void =>
    setConditions((cs) => cs.map((c, i) => (i === idx ? { ...c, value } : c)));

  const workingRule: AcceptanceRule = { ...draft, conditions, action };
  const preview = useMemo(() => describeAcceptanceRule(workingRule), [workingRule]);

  const onDropField = (raw: string): void => {
    const field = decodeAcceptanceDrag(raw);
    if (field !== null) addCondition(field);
  };

  const handleSave = (): void => onSave({ ...draft, conditions, action });

  return (
    <div
      className={rr.ruleEditor}
      role="dialog"
      aria-label="Acceptance rule editor"
      data-testid="acceptance-rule-editor"
    >
      <div className={rr.ruleEditorPanel}>
        <header className={rr.ruleEditorHead}>
          <div>
            <h2 className={rr.ruleEditorTitle}>
              {isNew ? "Create acceptance rule" : "Edit acceptance rule"}
            </h2>
            <p className={rr.ruleEditorHint}>
              Drag lift-field chips into the conditions to build{" "}
              <strong>IF &lt;conditions&gt; THEN &lt;decision&gt;</strong>. Remove every condition
              to make this the catch-all (default) rule.
            </p>
          </div>
          <button type="button" className={rr.ghostBtn} onClick={onCancel} aria-label="Close editor">
            ✕
          </button>
        </header>

        <div className={rr.ruleEditorBody}>
          {!readOnly && <AcceptanceFieldPalette readOnly={readOnly} />}

          <div className={rr.ruleEditorMain}>
            <section
              className={rr.condArea}
              data-testid="acceptance-condition-area"
              aria-label="Rule conditions"
              onDragOver={(e) => {
                if (!readOnly) e.preventDefault();
              }}
              onDrop={(e) => {
                e.preventDefault();
                if (readOnly) return;
                onDropField(e.dataTransfer.getData("text/plain"));
              }}
            >
              <h3 className={rr.condAreaTitle}>Conditions (all must hold)</h3>
              {conditions.length === 0 ? (
                <p className={rr.condEmpty}>
                  {readOnly
                    ? "No conditions — this is the catch-all (default) rule."
                    : "Drag a field chip here to add a condition, or leave empty for the catch-all rule."}
                </p>
              ) : (
                <ul className={rr.condList}>
                  {conditions.map((c, idx) => (
                    <AcceptanceConditionRow
                      key={idx}
                      index={idx}
                      cond={c}
                      readOnly={readOnly}
                      onRebind={rebindField}
                      onSetOp={setOp}
                      onSetVal={setVal}
                      onRemove={removeCondition}
                    />
                  ))}
                </ul>
              )}
            </section>

            <section className={rr.destArea} aria-label="Decision">
              <h3 className={rr.condAreaTitle}>Then apply this decision</h3>
              {isDefault && (
                <p className={rr.condEmpty}>
                  Catch-all rules fire for every lift no earlier rule matched.
                </p>
              )}
              <AcceptanceActionEditor action={action} readOnly={readOnly} onChange={setAction} />
            </section>

            <p className={rr.rulePreview} data-testid="acceptance-rule-preview">
              <span className={rr.rulePreviewLabel}>Preview</span>
              {preview}
            </p>
          </div>
        </div>

        <footer className={rr.ruleEditorFoot}>
          <button type="button" className={rr.ghostBtn} onClick={onCancel}>
            Cancel
          </button>
          {!readOnly && (
            <button
              type="button"
              className={rr.saveBtn}
              onClick={handleSave}
              data-testid="acceptance-rule-save"
            >
              Save rule
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}

function AcceptanceConditionRow({
  index,
  cond,
  readOnly,
  onRebind,
  onSetOp,
  onSetVal,
  onRemove,
}: {
  index: number;
  cond: AcceptanceRuleCondition;
  readOnly: boolean;
  onRebind: (idx: number, field: AcceptanceField) => void;
  onSetOp: (idx: number, op: RouteOp) => void;
  onSetVal: (idx: number, value: RouteValue) => void;
  onRemove: (idx: number) => void;
}): React.ReactElement {
  const spec = acceptanceFieldSpec(cond.field);
  const enumOptions = useMemo(() => enumOptionsFor(cond.field), [cond.field]);

  return (
    <li className={rr.condRow} data-testid={`acceptance-cond-row-${index}`}>
      {index > 0 && <span className={rr.condAnd}>AND</span>}
      <div className={rr.condControls}>
        <label className={rr.editorField}>
          <span className={rr.fieldLabel}>Field</span>
          <select
            className={rr.select}
            value={cond.field}
            disabled={readOnly}
            data-testid={`acceptance-cond-field-${index}`}
            onChange={(e) => onRebind(index, e.target.value as AcceptanceField)}
          >
            {ACCEPTANCE_FIELD_REGISTRY.map((s) => (
              <option key={s.field} value={s.field}>
                {s.label}
              </option>
            ))}
          </select>
        </label>

        <label className={rr.editorField}>
          <span className={rr.fieldLabel}>Operator</span>
          <select
            className={rr.select}
            value={cond.op}
            disabled={readOnly}
            data-testid={`acceptance-cond-op-${index}`}
            onChange={(e) => onSetOp(index, e.target.value as RouteOp)}
          >
            {spec.validOps.map((op) => (
              <option key={op} value={op}>
                {opLabel(op)}
              </option>
            ))}
          </select>
        </label>

        <ValueEditor
          op={cond.op}
          value={cond.value}
          kind={spec.kind}
          enumOptions={enumOptions}
          readOnly={readOnly}
          onChange={(v) => onSetVal(index, v)}
        />
      </div>
      {!readOnly && (
        <button
          type="button"
          className={rr.dangerBtn}
          data-testid={`acceptance-cond-remove-${index}`}
          aria-label={`Remove condition ${index + 1}`}
          onClick={() => onRemove(index)}
        >
          Remove
        </button>
      )}
    </li>
  );
}
