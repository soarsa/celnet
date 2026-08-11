/**
 * The HEDGE-SPECIFIC step surfaces of the Hedging guided-setup wizard — step 3
 * (Internalise & hedge) and step 4 (Review). Steps 1 (Risk portfolios) and 2 (Routing)
 * are the SHARED {@link PortfoliosStep} / {@link RoutingStep} from
 * `workspaces/setupWizard/PortfolioRoutingSteps`, and this module composes the shared
 * {@link Explainer} + {@link ConditionEditor} primitives — no forked copies.
 *
 * Steps are controlled: they render the draft slice and emit a fresh slice up to
 * {@link ../SetupWizard}, which owns all state + the Apply sequence.
 */
import { useMemo } from "react";

import type {
  ExitAction,
  HedgeField,
  HedgeMetric,
  HedgePolicyScopeKind,
  WarehouseThreshold,
} from "../../../data/contract";
import { HelpButton } from "../../../components/HelpButton";
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
import type { ValueOption } from "../../riskrouting/ValueEditor";
import { ExitActionEditor } from "../ExitActionEditor";
import { describeRule } from "../../../lib/riskRules";
import {
  describeHedgeRule,
  newHedgeRuleId,
  type HedgeRule,
  type HedgeRuleCondition,
} from "../../../lib/hedgeRules";
import { defaultExitAction, EXIT_ACTION_KINDS, exitActionHint, exitActionLabel } from "../../../lib/hedgeExit";
import {
  ConditionEditor,
  Explainer,
  type CondDraft,
  type FieldOpt,
} from "../../setupWizard/PortfolioRoutingSteps";
import {
  enabledBookKeys,
  type WizardBook,
  type WizardDraft,
  type WizardPolicyScope,
} from "./wizardModel";
import styles from "../../setupWizard/setupWizard.module.css";

import { NumberField } from "../../../components/NumberField";

// Re-export the shared steps so the wizard host imports all four step surfaces from here.
export { PortfoliosStep, RoutingStep } from "../../setupWizard/PortfolioRoutingSteps";

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
  policyScope,
  readOnly,
  onToggleThreshold,
  onChangeThreshold,
  onChangeHedgeRules,
  onChangePolicyScope,
}: {
  includeThreshold: boolean;
  threshold: WarehouseThreshold;
  hedgeRules: HedgeRule[];
  books: readonly WizardBook[];
  policyScope: WizardPolicyScope;
  readOnly: boolean;
  onToggleThreshold: (on: boolean) => void;
  onChangeThreshold: (next: WarehouseThreshold) => void;
  onChangeHedgeRules: (next: HedgeRule[]) => void;
  onChangePolicyScope: (next: WizardPolicyScope) => void;
}): React.ReactElement {
  const bookTargets = useMemo(() => books.filter((b) => b.name.trim().length > 0), [books]);
  const scopeNoun = policyScope.scopeKind === "bucket" ? "portfolio" : "book";
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

      <div className={styles.subCard} data-testid="wiz-policy-scope-card">
        <h4 className={styles.subHeading}>
          Policy scope
          <HelpButton helpId="concept.hedge-policy-scope" subject="the hedge policy scope" />
        </h4>
        <p className={styles.hint}>
          Save this exit policy firm-wide, or as an override for one <strong>book</strong> or a whole{" "}
          <strong>bucket</strong> (portfolio subtree). A Book/Bucket override falls back to the Firm policy when empty.
        </p>
        <div className={styles.entryGrid}>
          <label className={styles.field}>
            <span className={styles.miniLabel}>Scope</span>
            <select
              className={styles.select}
              value={policyScope.scopeKind}
              disabled={readOnly}
              data-testid="wiz-policy-scope-kind"
              onChange={(e) =>
                onChangePolicyScope({
                  scopeKind: e.target.value as HedgePolicyScopeKind,
                  scopeId: "",
                })
              }
            >
              <option value="firm">Firm (default)</option>
              <option value="book">Book</option>
              <option value="bucket">Bucket (portfolio)</option>
            </select>
          </label>
          {policyScope.scopeKind !== "firm" && (
            <label className={styles.field}>
              <span className={styles.miniLabel}>
                {policyScope.scopeKind === "bucket" ? "Portfolio (subtree root)" : "Risk book"}
              </span>
              <select
                className={styles.select}
                value={policyScope.scopeId}
                disabled={readOnly}
                data-testid="wiz-policy-scope-id"
                onChange={(e) =>
                  onChangePolicyScope({ ...policyScope, scopeId: e.target.value })
                }
              >
                <option value="">(select a {scopeNoun})</option>
                {bookTargets.map((b) => (
                  <option key={b.key} value={b.key}>
                    {b.name}
                  </option>
                ))}
              </select>
            </label>
          )}
        </div>
        <p className={styles.hint} data-testid="wiz-exec-mode-note">
          The rules below decide <em>what to do</em>; how a live hedge externalises (Advisory dry-run,
          LP panel, Composite) is the <strong>execution mode</strong> set on Hedging Rules → Execution mode.
          <HelpButton helpId="concept.hedge-execution-mode" subject="the hedge execution mode" />
        </p>
      </div>

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
              <NumberField
                className={styles.input}
                value={threshold.cap}
                disabled={readOnly}
                data-testid="wiz-threshold-cap"
                onChange={num("cap")}
              />
            </label>
            <label className={styles.field}>
              <span className={styles.miniLabel}>Amber band (0–1)</span>
              <NumberField
                className={styles.input}
                step={0.05}
                value={threshold.amber}
                disabled={readOnly}
                onChange={num("amber")}
              />
            </label>
            <label className={styles.field}>
              <span className={styles.miniLabel}>Red band (0–1)</span>
              <NumberField
                className={styles.input}
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
            <li data-testid="wiz-review-scope">
              Policy scope:{" "}
              {draft.policyScope.scopeKind === "firm"
                ? "Firm-wide (default)"
                : `${draft.policyScope.scopeKind === "bucket" ? "Bucket" : "Book"} · ${bookLabel(draft.policyScope.scopeId)}`}
            </li>
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
