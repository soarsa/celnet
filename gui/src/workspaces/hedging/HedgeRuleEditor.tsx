/**
 * HedgeRuleEditor — the per-rule builder for a hedge exit policy. The SAME shape as
 * the risk-routing {@link ../riskrouting/RuleEditor}: drag field chips into the
 * conditions area to add `field op value` legs (ANDed), edit each leg with the SHARED
 * typed {@link ../riskrouting/ValueEditor} control, and pick the terminal — but the
 * terminal is an {@link ExitActionEditor} (an exit-action + its venue/size/style)
 * instead of a desk→book picker. A live preview shows the rule in plain English.
 */
import { useMemo, useState } from "react";

import type {
  ExitAction,
  HedgeField,
  HedgeVehicleRule,
  RouteOp,
  RouteValue,
} from "../../data/contract";
import {
  BREACHED_VALUES,
  HEDGE_CCY_VALUES,
  HEDGE_FIELD_REGISTRY,
  HEDGE_PRODUCT_VALUES,
  hedgeFieldSpec,
} from "../../lib/hedgeFields";
import {
  defaultOpForHedgeField,
  defaultValueForOp,
} from "../../lib/hedgeGraphOps";
import { describeHedgeRule, type HedgeRule, type HedgeRuleCondition } from "../../lib/hedgeRules";
import { opLabel } from "../../lib/routeOps";
import { ValueEditor, type ValueOption } from "../riskrouting/ValueEditor";
import { decodeHedgeDrag, HedgeFieldPalette } from "./HedgeFieldPalette";
import { ExitActionEditor } from "./ExitActionEditor";
import rr from "../riskrouting/RiskRoutingWorkspace.module.css";

interface HedgeRuleEditorProps {
  draft: HedgeRule;
  isNew: boolean;
  readOnly: boolean;
  instrumentOptions: readonly string[];
  lpOptions: readonly string[];
  /**
   * The firm's hedge-vehicle registry — the legal NAMED vehicles for a size-bearing leaf
   * (each row carries the DV01-per-unit its size is computed from). Absent ⇒ empty.
   */
  vehicles?: readonly HedgeVehicleRule[];
  onSave: (rule: HedgeRule) => void;
  onCancel: () => void;
  /**
   * When this editor opened from a SEED (a Deals-blotter "Change hedging strategy" or a
   * pricing group's "Create hedging rule"), the pre-rendered, source-agnostic hint —
   * drives the banner explaining what the draft was scoped from and what to complete.
   * `null`/absent for a hand-built rule (no banner).
   */
  seedHint?: string | null;
  /** The gap note shown beneath {@link seedHint} (empty when there is no seed). */
  seedGapNote?: string;
}

/** Build the enum value options for a hedge field (static advisory lists). */
function enumOptionsFor(field: HedgeField): ValueOption[] {
  switch (field) {
    case "breached":
      return BREACHED_VALUES.map((v) => ({ value: v, label: v }));
    case "ccy":
      return HEDGE_CCY_VALUES.map((v) => ({ value: v, label: v }));
    case "product":
      return HEDGE_PRODUCT_VALUES.map((v) => ({ value: v, label: v }));
    default:
      return [];
  }
}

export function HedgeRuleEditor({
  draft,
  isNew,
  readOnly,
  instrumentOptions,
  lpOptions,
  vehicles = [],
  onSave,
  onCancel,
  seedHint = null,
  seedGapNote = "",
}: HedgeRuleEditorProps): React.ReactElement {
  const [conditions, setConditions] = useState<HedgeRuleCondition[]>(draft.conditions);
  const [action, setAction] = useState<ExitAction>(draft.action);

  const isDefault = conditions.length === 0;

  const addCondition = (field: HedgeField): void => {
    const op = defaultOpForHedgeField(field);
    setConditions((cs) => [...cs, { field, op, value: defaultValueForOp(field, op) }]);
  };
  const removeCondition = (idx: number): void =>
    setConditions((cs) => cs.filter((_, i) => i !== idx));
  const rebindField = (idx: number, field: HedgeField): void =>
    setConditions((cs) =>
      cs.map((c, i) => {
        if (i !== idx) return c;
        const op = defaultOpForHedgeField(field);
        return { field, op, value: defaultValueForOp(field, op) };
      }),
    );
  const setOp = (idx: number, op: RouteOp): void =>
    setConditions((cs) =>
      cs.map((c, i) => (i === idx ? { ...c, op, value: defaultValueForOp(c.field, op) } : c)),
    );
  const setVal = (idx: number, value: RouteValue): void =>
    setConditions((cs) => cs.map((c, i) => (i === idx ? { ...c, value } : c)));

  const workingRule: HedgeRule = { ...draft, conditions, action };
  const preview = useMemo(() => describeHedgeRule(workingRule), [workingRule]);

  const onDropField = (raw: string): void => {
    const field = decodeHedgeDrag(raw);
    if (field !== null) addCondition(field);
  };

  const handleSave = (): void => onSave({ ...draft, conditions, action });

  return (
    <div className={rr.ruleEditor} role="dialog" aria-label="Hedge rule editor" data-testid="hedge-rule-editor">
      <div className={rr.ruleEditorPanel}>
        <header className={rr.ruleEditorHead}>
          <div>
            <h2 className={rr.ruleEditorTitle}>{isNew ? "Create hedge rule" : "Edit hedge rule"}</h2>
            <p className={rr.ruleEditorHint}>
              Drag risk-state chips into the conditions to build{" "}
              <strong>IF &lt;conditions&gt; THEN &lt;exit action&gt;</strong>. Remove every condition to
              make this the catch-all (default) rule.
            </p>
          </div>
          <button type="button" className={rr.ghostBtn} onClick={onCancel} aria-label="Close editor">
            ✕
          </button>
        </header>

        {seedHint !== null && (
          <div className={rr.seedHint} role="status" data-testid="hedge-seed-hint">
            <strong className={rr.seedHintMain}>{seedHint}</strong>
            <span className={rr.seedHintNote}>{seedGapNote}</span>
          </div>
        )}

        <div className={rr.ruleEditorBody}>
          {!readOnly && <HedgeFieldPalette readOnly={readOnly} />}

          <div className={rr.ruleEditorMain}>
            <section
              className={rr.condArea}
              data-testid="hedge-condition-area"
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
                    <HedgeConditionRow
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

            <section className={rr.destArea} aria-label="Exit action">
              <h3 className={rr.condAreaTitle}>Then fire this exit action</h3>
              {isDefault && (
                <p className={rr.condEmpty}>
                  Catch-all rules fire for every risk state no earlier rule matched.
                </p>
              )}
              <ExitActionEditor
                action={action}
                readOnly={readOnly}
                instrumentOptions={instrumentOptions}
                lpOptions={lpOptions}
                vehicles={vehicles}
                onChange={setAction}
              />
            </section>

            <p className={rr.rulePreview} data-testid="hedge-rule-preview">
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
            <button type="button" className={rr.saveBtn} onClick={handleSave} data-testid="hedge-rule-save">
              Save rule
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}

function HedgeConditionRow({
  index,
  cond,
  readOnly,
  onRebind,
  onSetOp,
  onSetVal,
  onRemove,
}: {
  index: number;
  cond: HedgeRuleCondition;
  readOnly: boolean;
  onRebind: (idx: number, field: HedgeField) => void;
  onSetOp: (idx: number, op: RouteOp) => void;
  onSetVal: (idx: number, value: RouteValue) => void;
  onRemove: (idx: number) => void;
}): React.ReactElement {
  const spec = hedgeFieldSpec(cond.field);
  const enumOptions = useMemo(() => enumOptionsFor(cond.field), [cond.field]);

  return (
    <li className={rr.condRow} data-testid={`hedge-cond-row-${index}`}>
      {index > 0 && <span className={rr.condAnd}>AND</span>}
      <div className={rr.condControls}>
        <label className={rr.editorField}>
          <span className={rr.fieldLabel}>Field</span>
          <select
            className={rr.select}
            value={cond.field}
            disabled={readOnly}
            data-testid={`hedge-cond-field-${index}`}
            onChange={(e) => onRebind(index, e.target.value as HedgeField)}
          >
            {HEDGE_FIELD_REGISTRY.map((s) => (
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
            data-testid={`hedge-cond-op-${index}`}
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
          data-testid={`hedge-cond-remove-${index}`}
          aria-label={`Remove condition ${index + 1}`}
          onClick={() => onRemove(index)}
        >
          Remove
        </button>
      )}
    </li>
  );
}
