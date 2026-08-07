/**
 * The RISK-SPECIFIC step surfaces of the Risk guided-setup wizard — step 3 (Acceptance)
 * and step 4 (Review). Steps 1 (Risk portfolios) and 2 (Routing) are the SHARED
 * {@link PortfoliosStep} / {@link RoutingStep}; this module composes the shared
 * {@link Explainer} + {@link ConditionEditor} primitives and reuses the standalone
 * {@link AcceptanceActionEditor} decision leaf, so the acceptance builder is the same
 * one the standalone Acceptance workspace uses — no forked copy.
 *
 * Steps are controlled: they render the draft slice and emit a fresh slice up to
 * {@link RiskSetupWizard}, which owns all state + the Apply sequence.
 */
import { useMemo } from "react";

import type { AcceptanceField, DeskDesc, FixConnection } from "../../data/contract";
import {
  ACCEPTANCE_ASSET_CLASS_VALUES,
  ACCEPTANCE_FIELD_REGISTRY,
  ACCEPTANCE_SIDE_VALUES,
  acceptanceFieldSpec,
} from "../../lib/acceptanceFields";
import { defaultOpForAcceptanceField, defaultValueForOp } from "../../lib/acceptanceGraphOps";
import {
  describeAcceptanceRule,
  newAcceptanceRuleId,
  type AcceptanceRule,
  type AcceptanceRuleCondition,
} from "../../lib/acceptanceRules";
import { describeRule } from "../../lib/riskRules";
import type { ValueOption } from "../riskrouting/ValueEditor";
import { AcceptanceActionEditor } from "../acceptance/AcceptanceActionEditor";
import {
  ConditionEditor,
  Explainer,
  type CondDraft,
  type FieldOpt,
} from "../setupWizard/PortfolioRoutingSteps";
import { enabledBookKeys, type WizardBook } from "../setupWizard/wizardModel";
import type { RiskWizardDraft } from "./riskWizardModel";
import { DEFAULT_EDGE_FLOOR_BPS } from "./riskWizardModel";
import styles from "../setupWizard/setupWizard.module.css";

// Re-export the shared steps so the wizard host imports all four step surfaces from here.
export { PortfoliosStep, RoutingStep } from "../setupWizard/PortfolioRoutingSteps";

/** Every acceptance lift-field the wizard exposes (the full registry). */
const ACCEPTANCE_FIELD_OPTS: readonly FieldOpt[] = ACCEPTANCE_FIELD_REGISTRY.map((s) => ({
  value: s.field,
  label: s.label,
}));

/** Build the acceptance enum-value resolver — static lists + live desk / counterparty rosters. */
function makeAcceptanceEnum(
  desks: readonly DeskDesc[],
  connections: readonly FixConnection[],
): (field: string) => ValueOption[] {
  return (field: string): ValueOption[] => {
    switch (field as AcceptanceField) {
      case "side":
        return ACCEPTANCE_SIDE_VALUES.map((v) => ({ value: v, label: v }));
      case "asset_class":
        return ACCEPTANCE_ASSET_CLASS_VALUES.map((v) => ({ value: v, label: v }));
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
// Step 3 — Acceptance criteria
// ---------------------------------------------------------------------------

export function AcceptanceStep({
  rules,
  desks,
  connections,
  readOnly,
  onChange,
}: {
  rules: AcceptanceRule[];
  desks: readonly DeskDesc[];
  connections: readonly FixConnection[];
  readOnly: boolean;
  onChange: (next: AcceptanceRule[]) => void;
}): React.ReactElement {
  const acceptanceEnum = useMemo(() => makeAcceptanceEnum(desks, connections), [desks, connections]);

  const defaultIdx = rules.findIndex((r) => r.conditions.length === 0);
  const specifics = rules.filter((r) => r.conditions.length > 0);
  const defaultRule = defaultIdx >= 0 ? (rules[defaultIdx] as AcceptanceRule) : null;
  const replace = (id: string, next: AcceptanceRule): void =>
    onChange(rules.map((r) => (r.id === id ? next : r)));

  const addRule = (): void => {
    // A NEW specific rule REJECTS thin-edge flow — seed the edge floor (the trader flips
    // it to hold / edits the guard in place).
    const rule: AcceptanceRule = {
      id: newAcceptanceRuleId(),
      conditions: [{ field: "edge_bps", op: "lt", value: { kind: "num", num: DEFAULT_EDGE_FLOOR_BPS } }],
      action: { kind: "reject", reason: "below edge floor" },
      enabled: true,
    };
    const next = [...rules];
    const di = next.findIndex((r) => r.conditions.length === 0);
    if (di >= 0) next.splice(di, 0, rule);
    else next.push(rule);
    onChange(next);
  };
  const removeRule = (id: string): void => onChange(rules.filter((r) => r.id !== id));

  return (
    <div className={styles.stepBody} data-testid="risk-wiz-acceptance">
      <Explainer icon="🛂" title="Acceptance — which incoming lifts we take">
        Acceptance gates <strong>at the point of a lift</strong> — after last-look, before booking — on
        every incoming client fill, in order (the <strong>first rule that matches</strong> wins).{" "}
        <strong>Accept</strong> books it · <strong>Reject</strong> declines it with a reason surfaced to
        the counterparty · <strong>Hold for review</strong> routes it to the desk inbox for a human. The
        starter policy rejects flow below an edge floor and accepts everything else — remove the
        specific rule to accept every lift.
      </Explainer>

      <div className={styles.subCard}>
        <h4 className={styles.subHeading}>Accept / reject / hold rules</h4>
        <div className={styles.cardList} data-testid="risk-wiz-acc-rules">
          {specifics.map((r) => {
            const idx = rules.indexOf(r);
            return (
              <div className={styles.entryCard} key={r.id} data-testid={`risk-wiz-acc-rule-${idx}`}>
                <div className={styles.ruleHead}>
                  <span className={styles.ruleBadge}>IF</span>
                  <span className={styles.rulePreview}>{describeAcceptanceRule(r)}</span>
                  {!readOnly && (
                    <button
                      type="button"
                      className={styles.removeBtn}
                      data-testid={`risk-wiz-acc-remove-${idx}`}
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
                      fieldOpts={ACCEPTANCE_FIELD_OPTS}
                      validOps={(f) => acceptanceFieldSpec(f as AcceptanceField).validOps}
                      kindOf={(f) => acceptanceFieldSpec(f as AcceptanceField).kind}
                      enumOptions={acceptanceEnum}
                      defaultOp={(f) => defaultOpForAcceptanceField(f as AcceptanceField)}
                      defaultValue={(f, op) => defaultValueForOp(f as AcceptanceField, op)}
                      readOnly={readOnly}
                      onChange={(nc) =>
                        replace(r.id, {
                          ...r,
                          conditions: r.conditions.map((x, k) =>
                            k === ci ? (nc as AcceptanceRuleCondition) : x,
                          ),
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
                      data-testid={`risk-wiz-acc-add-cond-${idx}`}
                      onClick={() =>
                        replace(r.id, {
                          ...r,
                          conditions: [
                            ...r.conditions,
                            {
                              field: "notional_usd",
                              op: defaultOpForAcceptanceField("notional_usd"),
                              value: defaultValueForOp(
                                "notional_usd",
                                defaultOpForAcceptanceField("notional_usd"),
                              ),
                            },
                          ],
                        })
                      }
                    >
                      + AND condition
                    </button>
                  )}
                </div>
                <div className={styles.field}>
                  <span className={styles.miniLabel}>Then</span>
                  <AcceptanceActionEditor
                    action={r.action}
                    readOnly={readOnly}
                    onChange={(action) => replace(r.id, { ...r, action })}
                  />
                </div>
              </div>
            );
          })}
        </div>

        {!readOnly && (
          <button
            type="button"
            className={styles.addBtn}
            data-testid="risk-wiz-add-acc-rule"
            onClick={addRule}
          >
            + Add acceptance rule
          </button>
        )}

        <div className={styles.entryCard} data-testid="risk-wiz-acc-default">
          <div className={styles.ruleHead}>
            <span className={`${styles.ruleBadge} ${styles.ruleBadgeDefault}`}>OTHERWISE</span>
            <span className={styles.rulePreview}>
              {defaultRule ? describeAcceptanceRule(defaultRule) : "Accept (accept-all)"}
            </span>
          </div>
          {defaultRule && (
            <div className={styles.field}>
              <span className={styles.miniLabel}>Default decision</span>
              <AcceptanceActionEditor
                action={defaultRule.action}
                readOnly={readOnly}
                onChange={(action) => replace(defaultRule.id, { ...defaultRule, action })}
              />
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

export function RiskReviewStep({
  draft,
  canRisk,
  canAcceptance,
}: {
  draft: RiskWizardDraft;
  canRisk: boolean;
  canAcceptance: boolean;
}): React.ReactElement {
  const enabledKeys = enabledBookKeys(draft.books);
  const bookLabel = (id: string): string => draft.books.find((b) => b.key === id)?.name ?? "(unknown)";
  const routingCount = draft.routingRules.filter((r) => r.enabled).length;
  const acceptanceCount = draft.acceptanceRules.filter((r) => r.enabled).length;

  return (
    <div className={styles.stepBody}>
      <Explainer icon="✅" title="Review & apply">
        Here&apos;s everything the wizard will create, in order. Portfolios are created first (so their
        ids exist), then routing, then the acceptance policy. Nothing is written until you press
        <strong> Apply</strong>.
      </Explainer>

      <div className={styles.reviewGrid}>
        <section className={styles.reviewCard} data-testid="risk-wiz-review-portfolios">
          <h4 className={styles.reviewHeading}>
            Risk portfolios <span className={styles.countPill}>{draft.books.length}</span>
          </h4>
          {!canRisk && <p className={styles.skipNote}>Skipped — you lack the Manage-Risk capability.</p>}
          <ul className={styles.reviewList}>
            {draft.books.map((b: WizardBook) => (
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

        <section className={styles.reviewCard} data-testid="risk-wiz-review-routing">
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

        <section className={styles.reviewCard} data-testid="risk-wiz-review-acceptance">
          <h4 className={styles.reviewHeading}>
            Acceptance policy <span className={styles.countPill}>{acceptanceCount}</span>
          </h4>
          {!canAcceptance && (
            <p className={styles.skipNote}>Skipped — you lack the Manage-Acceptance capability.</p>
          )}
          <ul className={styles.reviewList}>
            {draft.acceptanceRules
              .filter((r) => r.enabled)
              .map((r) => (
                <li key={r.id}>{describeAcceptanceRule(r)}</li>
              ))}
          </ul>
        </section>
      </div>
    </div>
  );
}
