/**
 * The four step surfaces of the Hedging guided-setup wizard, plus the shared inline
 * condition editor. Each step composes the EXISTING rule-builder primitives (the
 * routing field registry + {@link ValueEditor}, the hedge field registry +
 * {@link ExitActionEditor}) into a linear, plain-language flow — no separate editor
 * modals, no drag canvas. Steps are controlled: they render the draft slice and emit
 * a fresh slice up to {@link SetupWizard}, which owns all state + the Apply sequence.
 */
import { useMemo } from "react";

import type {
  DeskDesc,
  ExitAction,
  FixConnection,
  HedgeField,
  HedgeMetric,
  RouteField,
  RouteOp,
  RouteValue,
  WarehouseThreshold,
} from "../../../data/contract";
import type { FieldKind } from "../../../lib/routeOps";
import { opLabel } from "../../../lib/routeOps";
import { CCY_VALUES, FIELD_REGISTRY, PRODUCT_VALUES, SIDE_VALUES, fieldSpec } from "../../../lib/routeFields";
import { defaultOpForField, defaultValueForOp } from "../../riskrouting/graphOps";
import {
  BREACHED_VALUES,
  HEDGE_CCY_VALUES,
  HEDGE_FIELD_REGISTRY,
  HEDGE_PRODUCT_VALUES,
  hedgeFieldSpec,
} from "../../../lib/hedgeFields";
import {
  defaultOpForHedgeField,
  defaultValueForOp as hedgeDefaultValueForOp,
} from "../../../lib/hedgeGraphOps";
import { ValueEditor, type ValueOption } from "../../riskrouting/ValueEditor";
import { ExitActionEditor } from "../ExitActionEditor";
import { describeRule, newRuleId, type RiskRule, type RuleCondition } from "../../../lib/riskRules";
import {
  describeHedgeRule,
  newHedgeRuleId,
  type HedgeRule,
  type HedgeRuleCondition,
} from "../../../lib/hedgeRules";
import { defaultExitAction, EXIT_ACTION_KINDS, exitActionHint, exitActionLabel } from "../../../lib/hedgeExit";
import {
  enabledBookKeys,
  newBookKey,
  type WizardBook,
  type WizardDraft,
} from "./wizardModel";
import styles from "./SetupWizard.module.css";

/** Advisory aggregation instruments a CROSS_INTERNAL can target (mirrors HedgingWorkspace). */
const INSTRUMENT_OPTIONS: readonly string[] = ["AGG-OIS", "AGG-US10Y", "AGG-EURUSD", "AGG-UK5Y"];
/** Advisory LP ids an RFQ_OUT can fan to. */
const LP_OPTIONS: readonly string[] = ["LP-1", "LP-2", "LP-3", "LP-4"];
/** The hedge risk-state fields the wizard curates (the full registry is on the power surface). */
const WIZARD_HEDGE_FIELDS: readonly HedgeField[] = [
  "ccy",
  "product",
  "breached",
  "utilization",
  "net_dv01",
  "counterparty_toxicity",
];
const METRIC_LABEL: Record<HedgeMetric, string> = {
  dv01: "Net DV01",
  net_notional: "Net notional",
  net_delta: "Net delta",
  net_vega: "Net vega",
};

// ---------------------------------------------------------------------------
// Shared inline condition editor (routing + hedge), over the shared value model.
// ---------------------------------------------------------------------------

interface CondDraft {
  field: string;
  op: RouteOp;
  value: RouteValue;
}
interface FieldOpt {
  value: string;
  label: string;
}

function ConditionEditor({
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
      { key: newBookKey(), name: "", parentKey: null, deskId: null, enabled: true, limits: null },
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
                <div className={styles.entryGrid}>
                  <label className={styles.field}>
                    <span className={styles.miniLabel}>Max net notional</span>
                    <input
                      className={styles.input}
                      inputMode="decimal"
                      disabled={readOnly}
                      value={b.limits?.maxNetNotional ?? ""}
                      onChange={(e) =>
                        patch(b.key, { limits: mergeLimit(b, "maxNetNotional", e.target.value) })
                      }
                    />
                  </label>
                  <label className={styles.field}>
                    <span className={styles.miniLabel}>Max gross notional</span>
                    <input
                      className={styles.input}
                      inputMode="decimal"
                      disabled={readOnly}
                      value={b.limits?.maxGrossNotional ?? ""}
                      onChange={(e) =>
                        patch(b.key, { limits: mergeLimit(b, "maxGrossNotional", e.target.value) })
                      }
                    />
                  </label>
                  <label className={styles.field}>
                    <span className={styles.miniLabel}>Max DV01</span>
                    <input
                      className={styles.input}
                      inputMode="decimal"
                      disabled={readOnly}
                      value={b.limits?.maxDv01 ?? ""}
                      onChange={(e) => patch(b.key, { limits: mergeLimit(b, "maxDv01", e.target.value) })}
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

/** Merge one limit field, returning null when every cap is cleared. */
function mergeLimit(
  b: WizardBook,
  key: "maxNetNotional" | "maxGrossNotional" | "maxDv01",
  raw: string,
): WizardBook["limits"] {
  const t = raw.trim();
  const n = t.length === 0 ? null : Number.isFinite(Number(t)) ? Number(t) : null;
  const base = b.limits ?? { maxNetNotional: null, maxGrossNotional: null, maxDv01: null };
  const next = { ...base, [key]: n };
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

// ---------------------------------------------------------------------------
// Step 3 — Internalisation & hedging
// ---------------------------------------------------------------------------

const HEDGE_FIELD_OPTS: readonly FieldOpt[] = HEDGE_FIELD_REGISTRY.filter((s) =>
  WIZARD_HEDGE_FIELDS.includes(s.field),
).map((s) => ({ value: s.field, label: s.label }));

function hedgeEnum(field: string): ValueOption[] {
  switch (field as HedgeField) {
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

export function HedgingStep({
  includeThreshold,
  threshold,
  hedgeRules,
  books,
  readOnly,
  onToggleThreshold,
  onChangeThreshold,
  onChangeHedgeRules,
}: {
  includeThreshold: boolean;
  threshold: WarehouseThreshold;
  hedgeRules: HedgeRule[];
  books: readonly WizardBook[];
  readOnly: boolean;
  onToggleThreshold: (on: boolean) => void;
  onChangeThreshold: (next: WarehouseThreshold) => void;
  onChangeHedgeRules: (next: HedgeRule[]) => void;
}): React.ReactElement {
  const bookTargets = useMemo(() => books.filter((b) => b.name.trim().length > 0), [books]);
  const patchT = (p: Partial<WarehouseThreshold>): void => onChangeThreshold({ ...threshold, ...p });
  const num = (key: keyof WarehouseThreshold) => (e: React.ChangeEvent<HTMLInputElement>) =>
    patchT({ [key]: Number(e.target.value) } as Partial<WarehouseThreshold>);

  const defaultIdx = hedgeRules.findIndex((r) => r.conditions.length === 0);
  const specifics = hedgeRules.filter((r) => r.conditions.length > 0);
  const defaultRule = defaultIdx >= 0 ? (hedgeRules[defaultIdx] as HedgeRule) : null;
  const replaceRule = (id: string, next: HedgeRule): void =>
    onChangeHedgeRules(hedgeRules.map((r) => (r.id === id ? next : r)));

  const addHedgeRule = (): void => {
    const rule: HedgeRule = {
      id: newHedgeRuleId(),
      conditions: [
        { field: "ccy", op: defaultOpForHedgeField("ccy"), value: hedgeDefaultValueForOp("ccy", defaultOpForHedgeField("ccy")) },
      ],
      action: defaultExitAction("submit_market_order"),
      enabled: true,
    };
    const next = [...hedgeRules];
    const di = next.findIndex((r) => r.conditions.length === 0);
    if (di >= 0) next.splice(di, 0, rule);
    else next.push(rule);
    onChangeHedgeRules(next);
  };
  const removeHedgeRule = (id: string): void => onChangeHedgeRules(hedgeRules.filter((r) => r.id !== id));

  return (
    <div className={styles.stepBody}>
      <Explainer icon="⚖️" title="Internalise, then hedge the overflow">
        You <strong>internalise</strong> (warehouse) risk up to a cap — the &ldquo;100&rdquo; — netting
        client flow against itself for free. Above the cap you <strong>hedge the overflow</strong>{" "}
        back-to-back with the street. Set the cap and warning bands below, then a starter exit policy:
        <strong> hold by default</strong>, with an optional rule to actively hedge specific flow.
      </Explainer>

      <div className={styles.subCard}>
        <label className={styles.checkField}>
          <input
            type="checkbox"
            checked={includeThreshold}
            disabled={readOnly}
            data-testid="wiz-include-threshold"
            onChange={(e) => onToggleThreshold(e.target.checked)}
          />
          <span>
            <strong>Set a warehouse threshold</strong> (the internalise-up-to cap)
          </span>
        </label>
        {includeThreshold && (
          <div className={styles.entryGrid} data-testid="wiz-threshold-form">
            <label className={styles.field}>
              <span className={styles.miniLabel}>Scope portfolio</span>
              <select
                className={styles.select}
                value={threshold.scopeKind === "book" ? threshold.scopeId : ""}
                disabled={readOnly}
                data-testid="wiz-threshold-scope"
                onChange={(e) => patchT({ scopeKind: "book", scopeId: e.target.value })}
              >
                <option value="">(select a portfolio)</option>
                {bookTargets.map((b) => (
                  <option key={b.key} value={b.key}>
                    {b.name}
                  </option>
                ))}
              </select>
            </label>
            <label className={styles.field}>
              <span className={styles.miniLabel}>Metric</span>
              <select
                className={styles.select}
                value={threshold.metric}
                disabled={readOnly}
                onChange={(e) => patchT({ metric: e.target.value as HedgeMetric })}
              >
                {(Object.keys(METRIC_LABEL) as HedgeMetric[]).map((m) => (
                  <option key={m} value={m}>
                    {METRIC_LABEL[m]}
                  </option>
                ))}
              </select>
            </label>
            <label className={styles.field}>
              <span className={styles.miniLabel}>Cap (the &ldquo;100&rdquo;)</span>
              <input
                className={styles.input}
                type="number"
                value={threshold.cap}
                disabled={readOnly}
                data-testid="wiz-threshold-cap"
                onChange={num("cap")}
              />
            </label>
            <label className={styles.field}>
              <span className={styles.miniLabel}>Amber band (0–1)</span>
              <input
                className={styles.input}
                type="number"
                step={0.05}
                value={threshold.amber}
                disabled={readOnly}
                onChange={num("amber")}
              />
            </label>
            <label className={styles.field}>
              <span className={styles.miniLabel}>Red band (0–1)</span>
              <input
                className={styles.input}
                type="number"
                step={0.05}
                value={threshold.red}
                disabled={readOnly}
                onChange={num("red")}
              />
            </label>
          </div>
        )}
      </div>

      <div className={styles.subCard}>
        <h4 className={styles.subHeading}>Exit policy — what to do when risk crosses the band</h4>
        <div className={styles.cardList} data-testid="wiz-hedge-rules">
          {specifics.map((r) => {
            const idx = hedgeRules.indexOf(r);
            return (
              <div className={styles.entryCard} key={r.id} data-testid={`wiz-hedge-rule-${idx}`}>
                <div className={styles.ruleHead}>
                  <span className={styles.ruleBadge}>IF</span>
                  <span className={styles.rulePreview}>{describeHedgeRule(r)}</span>
                  {!readOnly && (
                    <button
                      type="button"
                      className={styles.removeBtn}
                      data-testid={`wiz-hedge-remove-${idx}`}
                      onClick={() => removeHedgeRule(r.id)}
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
                      fieldOpts={HEDGE_FIELD_OPTS}
                      validOps={(f) => hedgeFieldSpec(f as HedgeField).validOps}
                      kindOf={(f) => hedgeFieldSpec(f as HedgeField).kind}
                      enumOptions={hedgeEnum}
                      defaultOp={(f) => defaultOpForHedgeField(f as HedgeField)}
                      defaultValue={(f, op) => hedgeDefaultValueForOp(f as HedgeField, op)}
                      readOnly={readOnly}
                      onChange={(nc) =>
                        replaceRule(r.id, {
                          ...r,
                          conditions: r.conditions.map((x, k) =>
                            k === ci ? (nc as HedgeRuleCondition) : x,
                          ),
                        })
                      }
                      onRemove={() =>
                        replaceRule(r.id, { ...r, conditions: r.conditions.filter((_, k) => k !== ci) })
                      }
                    />
                  ))}
                </div>
                <div className={styles.field}>
                  <span className={styles.miniLabel}>Then</span>
                  <ExitActionEditor
                    action={r.action}
                    readOnly={readOnly}
                    instrumentOptions={INSTRUMENT_OPTIONS}
                    lpOptions={LP_OPTIONS}
                    onChange={(action: ExitAction) => replaceRule(r.id, { ...r, action })}
                  />
                </div>
              </div>
            );
          })}
        </div>

        {!readOnly && (
          <button type="button" className={styles.addBtn} data-testid="wiz-add-hedge-rule" onClick={addHedgeRule}>
            + Add hedge rule
          </button>
        )}

        <div className={styles.entryCard} data-testid="wiz-hedge-default">
          <div className={styles.ruleHead}>
            <span className={`${styles.ruleBadge} ${styles.ruleBadgeDefault}`}>OTHERWISE</span>
            <span className={styles.rulePreview}>{defaultRule ? describeHedgeRule(defaultRule) : "Warehouse (hold)"}</span>
          </div>
          {defaultRule && (
            <div className={styles.field}>
              <span className={styles.miniLabel}>Default action</span>
              <select
                className={styles.select}
                value={defaultRule.action.kind}
                disabled={readOnly}
                aria-label="Default hedge action"
                data-testid="wiz-hedge-default-action"
                onChange={(e) =>
                  replaceRule(defaultRule.id, { ...defaultRule, action: defaultExitAction(e.target.value as ExitAction["kind"]) })
                }
              >
                {EXIT_ACTION_KINDS.map((k) => (
                  <option key={k} value={k}>
                    {exitActionLabel(k)}
                  </option>
                ))}
              </select>
              <span className={styles.hint}>{exitActionHint(defaultRule.action.kind)}</span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Step 4 — Review
// ---------------------------------------------------------------------------

export function ReviewStep({
  draft,
  canRisk,
  canHedge,
}: {
  draft: WizardDraft;
  canRisk: boolean;
  canHedge: boolean;
}): React.ReactElement {
  const enabledKeys = enabledBookKeys(draft.books);
  const bookLabel = (id: string): string =>
    draft.books.find((b) => b.key === id)?.name ?? "(unknown)";
  const routingCount = draft.routingRules.filter((r) => r.enabled).length;

  return (
    <div className={styles.stepBody}>
      <Explainer icon="✅" title="Review & apply">
        Here&apos;s everything the wizard will create, in order. Portfolios are created first (so their
        ids exist), then routing, then the threshold and exit policy. Nothing is written until you press
        <strong> Apply</strong>.
      </Explainer>

      <div className={styles.reviewGrid}>
        <section className={styles.reviewCard} data-testid="wiz-review-portfolios">
          <h4 className={styles.reviewHeading}>
            Risk portfolios <span className={styles.countPill}>{draft.books.length}</span>
          </h4>
          {!canRisk && <p className={styles.skipNote}>Skipped — you lack the Manage-Risk capability.</p>}
          <ul className={styles.reviewList}>
            {draft.books.map((b) => (
              <li key={b.key}>
                <strong>{b.name || "(unnamed)"}</strong>
                {b.parentKey ? ` · under ${bookLabel(b.parentKey)}` : ""}
                {b.enabled ? "" : " · disabled"}
                {b.limits ? " · limited" : ""}
              </li>
            ))}
            {draft.books.length === 0 && <li className={styles.skipNote}>None</li>}
          </ul>
        </section>

        <section className={styles.reviewCard} data-testid="wiz-review-routing">
          <h4 className={styles.reviewHeading}>
            Routing rules <span className={styles.countPill}>{routingCount}</span>
          </h4>
          {!canRisk && <p className={styles.skipNote}>Skipped — you lack the Manage-Risk capability.</p>}
          <ul className={styles.reviewList}>
            {draft.routingRules
              .filter((r) => r.enabled)
              .map((r) => {
                const target = r.bookId !== null && enabledKeys.has(r.bookId);
                return (
                  <li key={r.id} className={target ? undefined : styles.reviewBad}>
                    {describeRule(r, bookLabel)}
                    {target ? "" : " ⚠ target not enabled"}
                  </li>
                );
              })}
          </ul>
        </section>

        <section className={styles.reviewCard} data-testid="wiz-review-hedging">
          <h4 className={styles.reviewHeading}>Internalise &amp; hedge</h4>
          {!canHedge && <p className={styles.skipNote}>Skipped — you lack the Hedge capability.</p>}
          <ul className={styles.reviewList}>
            <li>
              Threshold:{" "}
              {draft.includeThreshold
                ? `${METRIC_LABEL[draft.threshold.metric]} cap ${draft.threshold.cap.toLocaleString()} on ${bookLabel(draft.threshold.scopeId)}`
                : "— none"}
            </li>
            {draft.hedgeRules.map((r) => (
              <li key={r.id}>{describeHedgeRule(r)}</li>
            ))}
          </ul>
        </section>
      </div>
    </div>
  );
}
