/**
 * The SHARED step-1 (Risk portfolios) and step-2 (Routing) surfaces of the guided-setup
 * wizards, plus the shared primitives both wizards' later steps compose — the inline
 * {@link ConditionEditor} (over the shared value model) and the {@link Explainer} card.
 *
 * Both the Hedging guided setup and the Risk guided setup render these two steps
 * VERBATIM: they create the risk portfolios and decide which fills route into which,
 * and only DIVERGE at their final configuration step. Extracting them here means the
 * portfolio + routing UX — and its enabled-target guard — lives in exactly one place.
 *
 * Steps are controlled: they render the draft slice and emit a fresh slice up to the
 * host wizard, which owns all state + the Apply sequence.
 */
import { useMemo } from "react";

import { MAGNITUDE_HELP, MagnitudeField } from "../../components/MagnitudeField";
import type { DeskDesc, FixConnection, RouteField, RouteOp, RouteValue } from "../../data/contract";
import type { FieldKind } from "../../lib/routeOps";
import { opLabel } from "../../lib/routeOps";
import { CCY_VALUES, FIELD_REGISTRY, PRODUCT_VALUES, SIDE_VALUES, fieldSpec } from "../../lib/routeFields";
import { defaultOpForField, defaultValueForOp } from "../riskrouting/graphOps";
import { ValueEditor, type ValueOption } from "../riskrouting/ValueEditor";
import { describeRule, newRuleId, type RiskRule, type RuleCondition } from "../../lib/riskRules";
import { newBookKey, type WizardBook } from "./wizardModel";
import styles from "./setupWizard.module.css";

/** One inline condition being edited — `field op value`, over the shared value model. */
export interface CondDraft {
  field: string;
  op: RouteOp;
  value: RouteValue;
}
/** One `<select>` field option (value + visible label). */
export interface FieldOpt {
  value: string;
  label: string;
}

/**
 * The SHARED inline condition editor (routing + hedge + acceptance), over the shared
 * value model. Field / op / value selects + a typed {@link ValueEditor}, driven by the
 * caller's field registry (routing / hedge / acceptance) so one control serves all three.
 */
export function ConditionEditor({
  cond,
  index,
  fieldOpts,
  validOps,
  kindOf,
  enumOptions,
  defaultOp,
  defaultValue,
  readOnly,
  onChange,
  onRemove,
}: {
  cond: CondDraft;
  index: number;
  fieldOpts: readonly FieldOpt[];
  validOps: (f: string) => readonly RouteOp[];
  kindOf: (f: string) => FieldKind;
  enumOptions: (f: string) => ValueOption[];
  defaultOp: (f: string) => RouteOp;
  defaultValue: (f: string, op: RouteOp) => RouteValue;
  readOnly: boolean;
  onChange: (c: CondDraft) => void;
  onRemove: () => void;
}): React.ReactElement {
  const opts = useMemo(() => enumOptions(cond.field), [enumOptions, cond.field]);
  return (
    <div className={styles.condRow} data-testid={`wiz-cond-${index}`}>
      {index > 0 && <span className={styles.condAnd}>AND</span>}
      <label className={styles.condField}>
        <span className={styles.miniLabel}>Field</span>
        <select
          className={styles.select}
          value={cond.field}
          disabled={readOnly}
          data-testid={`wiz-cond-field-${index}`}
          onChange={(e) => {
            const field = e.target.value;
            const op = defaultOp(field);
            onChange({ field, op, value: defaultValue(field, op) });
          }}
        >
          {fieldOpts.map((f) => (
            <option key={f.value} value={f.value}>
              {f.label}
            </option>
          ))}
        </select>
      </label>
      <label className={styles.condField}>
        <span className={styles.miniLabel}>Op</span>
        <select
          className={styles.select}
          value={cond.op}
          disabled={readOnly}
          data-testid={`wiz-cond-op-${index}`}
          onChange={(e) => {
            const op = e.target.value as RouteOp;
            onChange({ ...cond, op, value: defaultValue(cond.field, op) });
          }}
        >
          {validOps(cond.field).map((op) => (
            <option key={op} value={op}>
              {opLabel(op)}
            </option>
          ))}
        </select>
      </label>
      <div className={styles.condValue}>
        <ValueEditor
          op={cond.op}
          value={cond.value}
          kind={kindOf(cond.field)}
          enumOptions={opts}
          readOnly={readOnly}
          onChange={(v) => onChange({ ...cond, value: v })}
        />
      </div>
      {!readOnly && (
        <button
          type="button"
          className={styles.condRemove}
          data-testid={`wiz-cond-remove-${index}`}
          aria-label={`Remove condition ${index + 1}`}
          onClick={onRemove}
        >
          ✕
        </button>
      )}
    </div>
  );
}

/** An explainer card — the plain-language "what & why" atop every step. */
export function Explainer({
  icon,
  title,
  children,
}: {
  icon: string;
  title: string;
  children: React.ReactNode;
}): React.ReactElement {
  return (
    <div className={styles.explainer}>
      <span className={styles.explainerIcon} aria-hidden="true">
        {icon}
      </span>
      <div>
        <h3 className={styles.explainerTitle}>{title}</h3>
        <p className={styles.explainerBody}>{children}</p>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Step 1 — Risk portfolios
// ---------------------------------------------------------------------------

export function PortfoliosStep({
  books,
  desks,
  readOnly,
  onChange,
}: {
  books: WizardBook[];
  desks: readonly DeskDesc[];
  readOnly: boolean;
  onChange: (next: WizardBook[]) => void;
}): React.ReactElement {
  const patch = (key: string, p: Partial<WizardBook>): void =>
    onChange(books.map((b) => (b.key === key ? { ...b, ...p } : b)));
  const addBook = (): void =>
    onChange([
      ...books,
      { key: newBookKey(), name: "", parentKey: null, deskId: null, enabled: true, limits: null, assetClass: "fixed_income" },
    ]);
  const removeBook = (key: string): void =>
    onChange(books.filter((b) => b.key !== key).map((b) => (b.parentKey === key ? { ...b, parentKey: null } : b)));

  return (
    <div className={styles.stepBody}>
      <Explainer icon="🗂️" title="Risk portfolios — where your risk is bucketed">
        A <strong>risk portfolio</strong> (book) is a bucket that your risk, limits and P&amp;L roll up
        into. Create one per desk or strategy — nest sub-portfolios under a parent for a hierarchy.
        Only <strong>enabled</strong> portfolios are valid routing targets in the next step. The server
        mints each portfolio&apos;s id from its name.
      </Explainer>

      <div className={styles.cardList} data-testid="wiz-portfolio-list">
        {books.length === 0 && <p className={styles.emptyNote}>No portfolios yet — add your first below.</p>}
        {books.map((b, i) => (
          <div className={styles.entryCard} key={b.key} data-testid={`wiz-portfolio-${i}`}>
            <div className={styles.entryGrid}>
              <label className={styles.field}>
                <span className={styles.miniLabel}>Name</span>
                <input
                  className={styles.input}
                  value={b.name}
                  disabled={readOnly}
                  placeholder="e.g. FX EMEA Vanilla"
                  data-testid={`wiz-portfolio-name-${i}`}
                  onChange={(e) => patch(b.key, { name: e.target.value })}
                />
              </label>
              <label className={styles.field}>
                <span className={styles.miniLabel}>Parent (optional)</span>
                <select
                  className={styles.select}
                  value={b.parentKey ?? ""}
                  disabled={readOnly}
                  onChange={(e) => patch(b.key, { parentKey: e.target.value === "" ? null : e.target.value })}
                >
                  <option value="">(top-level)</option>
                  {books
                    .filter((o) => o.key !== b.key && o.name.trim().length > 0)
                    .map((o) => (
                      <option key={o.key} value={o.key}>
                        {o.name}
                      </option>
                    ))}
                </select>
              </label>
              <label className={styles.field}>
                <span className={styles.miniLabel}>Owning desk (optional)</span>
                <select
                  className={styles.select}
                  value={b.deskId ?? ""}
                  disabled={readOnly}
                  onChange={(e) => patch(b.key, { deskId: e.target.value === "" ? null : e.target.value })}
                >
                  <option value="">(unowned)</option>
                  {desks.map((d) => (
                    <option key={d.id} value={d.id}>
                      {d.name}
                    </option>
                  ))}
                </select>
              </label>
            </div>
            <div className={styles.entryFoot}>
              <label className={styles.checkField}>
                <input
                  type="checkbox"
                  checked={b.enabled}
                  disabled={readOnly}
                  data-testid={`wiz-portfolio-enabled-${i}`}
                  onChange={(e) => patch(b.key, { enabled: e.target.checked })}
                />
                <span>Enabled — a valid routing target</span>
              </label>
              <details className={styles.limitsDisclosure}>
                <summary>Pre-trade limits (optional)</summary>
                <p className={styles.miniLabel}>{MAGNITUDE_HELP}</p>
                <div className={styles.entryGrid}>
                  <label className={styles.field}>
                    <span className={styles.miniLabel}>Max net notional</span>
                    <MagnitudeField
                      className={styles.input}
                      disabled={readOnly}
                      value={b.limits?.maxNetNotional ?? null}
                      onCommit={(v) => patch(b.key, { limits: mergeLimit(b, "maxNetNotional", v) })}
                    />
                  </label>
                  <label className={styles.field}>
                    <span className={styles.miniLabel}>Max gross notional</span>
                    <MagnitudeField
                      className={styles.input}
                      disabled={readOnly}
                      value={b.limits?.maxGrossNotional ?? null}
                      onCommit={(v) => patch(b.key, { limits: mergeLimit(b, "maxGrossNotional", v) })}
                    />
                  </label>
                  <label className={styles.field}>
                    <span className={styles.miniLabel}>Max DV01</span>
                    <MagnitudeField
                      className={styles.input}
                      disabled={readOnly}
                      value={b.limits?.maxDv01 ?? null}
                      onCommit={(v) => patch(b.key, { limits: mergeLimit(b, "maxDv01", v) })}
                    />
                  </label>
                </div>
              </details>
              {!readOnly && (
                <button
                  type="button"
                  className={styles.removeBtn}
                  data-testid={`wiz-portfolio-remove-${i}`}
                  onClick={() => removeBook(b.key)}
                >
                  Remove
                </button>
              )}
            </div>
          </div>
        ))}
      </div>

      {!readOnly && (
        <button type="button" className={styles.addBtn} data-testid="wiz-add-portfolio" onClick={addBook}>
          + Add portfolio
        </button>
      )}
    </div>
  );
}

/**
 * Merge one committed limit, returning null when every cap is cleared.
 *
 * `value` arrives already parsed and validated by {@link MagnitudeField}: an
 * entry that failed to parse is never committed, so this function can no longer
 * be reached with a typo — which previously landed here as `null`, silently
 * turning a mistyped cap into an UNCAPPED one.
 */
function mergeLimit(
  b: WizardBook,
  key: "maxNetNotional" | "maxGrossNotional" | "maxDv01",
  value: number | null,
): WizardBook["limits"] {
  const base = b.limits ?? { maxNetNotional: null, maxGrossNotional: null, maxDv01: null };
  const next = { ...base, [key]: value };
  return next.maxNetNotional === null && next.maxGrossNotional === null && next.maxDv01 === null
    ? null
    : next;
}

// ---------------------------------------------------------------------------
// Step 2 — Routing
// ---------------------------------------------------------------------------

const ROUTE_FIELD_OPTS: readonly FieldOpt[] = FIELD_REGISTRY.map((s) => ({ value: s.field, label: s.label }));

export function RoutingStep({
  rules,
  books,
  desks,
  connections,
  readOnly,
  onChange,
}: {
  rules: RiskRule[];
  books: readonly WizardBook[];
  desks: readonly DeskDesc[];
  connections: readonly FixConnection[];
  readOnly: boolean;
  onChange: (next: RiskRule[]) => void;
}): React.ReactElement {
  const targets = useMemo(() => books.filter((b) => b.enabled && b.name.trim().length > 0), [books]);
  const bookLabel = useMemo(
    () => (id: string) => targets.find((b) => b.key === id)?.name ?? "(unknown portfolio)",
    [targets],
  );
  const routeEnum = useMemo(() => makeRouteEnum(desks, connections), [desks, connections]);

  const defaultIdx = rules.findIndex((r) => r.conditions.length === 0);
  const specifics = rules.filter((r) => r.conditions.length > 0);
  const defaultRule = defaultIdx >= 0 ? (rules[defaultIdx] as RiskRule) : null;

  const replace = (id: string, next: RiskRule): void => onChange(rules.map((r) => (r.id === id ? next : r)));

  const addRule = (): void => {
    const rule: RiskRule = {
      id: newRuleId(),
      conditions: [{ field: "counterparty", op: defaultOpForField("counterparty"), value: defaultValueForOp("counterparty", defaultOpForField("counterparty")) }],
      bookId: targets[0]?.key ?? null,
      enabled: true,
    };
    // Insert the specific rule ABOVE the trailing default (first-match-wins).
    const next = [...rules];
    const di = next.findIndex((r) => r.conditions.length === 0);
    if (di >= 0) next.splice(di, 0, rule);
    else next.push(rule);
    onChange(next);
  };
  const removeRule = (id: string): void => onChange(rules.filter((r) => r.id !== id));

  return (
    <div className={styles.stepBody}>
      <Explainer icon="🔀" title="Routing — which fills land in which portfolio">
        When a fill comes in, the <strong>first rule that matches</strong> decides which portfolio its
        risk books into. Add specific rules (e.g. <em>counterparty = X → EMEA book</em>); the{" "}
        <strong>Otherwise</strong> rule at the bottom catches everything else. Every rule must point at
        an <strong>enabled</strong> portfolio you created in step 1.
      </Explainer>

      {targets.length === 0 && (
        <p className={styles.warnNote} role="note" data-testid="wiz-routing-no-targets">
          ⚠ You have no enabled portfolios yet — go back to step 1 and enable at least one before routing.
        </p>
      )}

      <div className={styles.cardList} data-testid="wiz-routing-list">
        {specifics.map((r) => {
          const idx = rules.indexOf(r);
          return (
            <div className={styles.entryCard} key={r.id} data-testid={`wiz-rule-${idx}`}>
              <div className={styles.ruleHead}>
                <span className={styles.ruleBadge}>IF</span>
                <span className={styles.rulePreview} data-testid={`wiz-rule-preview-${idx}`}>
                  {describeRule(r, bookLabel)}
                </span>
                {!readOnly && (
                  <button
                    type="button"
                    className={styles.removeBtn}
                    data-testid={`wiz-rule-remove-${idx}`}
                    onClick={() => removeRule(r.id)}
                  >
                    Remove
                  </button>
                )}
              </div>
              <div className={styles.condStack}>
                {r.conditions.map((c, ci) => (
                  <ConditionEditor
                    key={ci}
                    cond={c as CondDraft}
                    index={ci}
                    fieldOpts={ROUTE_FIELD_OPTS}
                    validOps={(f) => fieldSpec(f as RouteField).validOps}
                    kindOf={(f) => fieldSpec(f as RouteField).kind}
                    enumOptions={routeEnum}
                    defaultOp={(f) => defaultOpForField(f as RouteField)}
                    defaultValue={(f, op) => defaultValueForOp(f as RouteField, op)}
                    readOnly={readOnly}
                    onChange={(nc) =>
                      replace(r.id, {
                        ...r,
                        conditions: r.conditions.map((x, k) => (k === ci ? (nc as RuleCondition) : x)),
                      })
                    }
                    onRemove={() =>
                      replace(r.id, { ...r, conditions: r.conditions.filter((_, k) => k !== ci) })
                    }
                  />
                ))}
                {!readOnly && (
                  <button
                    type="button"
                    className={styles.addConditionBtn}
                    data-testid={`wiz-rule-add-cond-${idx}`}
                    onClick={() =>
                      replace(r.id, {
                        ...r,
                        conditions: [
                          ...r.conditions,
                          {
                            field: "ccy",
                            op: defaultOpForField("ccy"),
                            value: defaultValueForOp("ccy", defaultOpForField("ccy")),
                          },
                        ],
                      })
                    }
                  >
                    + AND condition
                  </button>
                )}
              </div>
              <BookPicker
                label="Route into"
                value={r.bookId}
                targets={targets}
                readOnly={readOnly}
                testid={`wiz-rule-target-${idx}`}
                onChange={(bookId) => replace(r.id, { ...r, bookId })}
              />
            </div>
          );
        })}
      </div>

      {!readOnly && (
        <button
          type="button"
          className={styles.addBtn}
          data-testid="wiz-add-rule"
          disabled={targets.length === 0}
          onClick={addRule}
        >
          + Add routing rule
        </button>
      )}

      <div className={styles.entryCard} data-testid="wiz-default-rule">
        <div className={styles.ruleHead}>
          <span className={`${styles.ruleBadge} ${styles.ruleBadgeDefault}`}>OTHERWISE</span>
          <span className={styles.rulePreview}>Every unmatched fill routes here</span>
        </div>
        <BookPicker
          label="Default portfolio"
          value={defaultRule?.bookId ?? null}
          targets={targets}
          readOnly={readOnly}
          testid="wiz-default-target"
          onChange={(bookId) => {
            if (defaultRule) replace(defaultRule.id, { ...defaultRule, bookId });
          }}
        />
      </div>
    </div>
  );
}

function BookPicker({
  label,
  value,
  targets,
  readOnly,
  testid,
  onChange,
}: {
  label: string;
  value: string | null;
  targets: readonly WizardBook[];
  readOnly: boolean;
  testid: string;
  onChange: (bookId: string | null) => void;
}): React.ReactElement {
  return (
    <label className={styles.field}>
      <span className={styles.miniLabel}>{label}</span>
      <select
        className={styles.select}
        value={value ?? ""}
        disabled={readOnly}
        data-testid={testid}
        onChange={(e) => onChange(e.target.value === "" ? null : e.target.value)}
      >
        <option value="">(select a portfolio)</option>
        {targets.map((b) => (
          <option key={b.key} value={b.key}>
            {b.name}
          </option>
        ))}
      </select>
    </label>
  );
}

/** Build the enum-value option resolver for a routing field (static lists or live rosters). */
function makeRouteEnum(
  desks: readonly DeskDesc[],
  connections: readonly FixConnection[],
): (field: string) => ValueOption[] {
  return (field: string): ValueOption[] => {
    const spec = fieldSpec(field as RouteField);
    switch (spec.enumSource) {
      case "side":
        return SIDE_VALUES.map((v) => ({ value: v, label: v }));
      case "product":
        return PRODUCT_VALUES.map((v) => ({ value: v, label: v }));
      case "ccy":
        return CCY_VALUES.map((v) => ({ value: v, label: v }));
      case "desk":
        return desks.map((d) => ({ value: d.id, label: d.name }));
      case "counterparty":
        return connections.map((c) => ({ value: c.id, label: c.name || c.id }));
      default:
        return [];
    }
  };
}
