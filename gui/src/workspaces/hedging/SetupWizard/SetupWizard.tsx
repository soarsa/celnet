/**
 * SetupWizard — the Hedging guided-setup flow. A single, sequential on-ramp that
 * walks a trader through the whole risk-lifecycle model in one place, instead of
 * navigating three separate surfaces (Risk Portfolios → Risk Routing → Hedging) and
 * having to already understand how they relate:
 *
 *   1. Risk portfolios — create the buckets risk books into
 *   2. Routing — decide which fills land in which portfolio
 *   3. Internalise & hedge — the warehouse cap + a starter exit policy
 *   4. Review & apply — commit everything through the existing RPCs, in order
 *
 * The host owns ALL draft state + the Apply sequence ({@link applyWizard}); the step
 * surfaces ({@link WizardSteps}) are controlled and compose the existing rule-builder
 * primitives. Capability-gated: steps 1–2 need `risk_manage·FI`, step 3 needs
 * `hedge·FI` — a step the caller can't apply renders read-only with a note rather
 * than failing at Apply. On success the wizard closes and lands the trader on the
 * Risk Dashboard so the new configuration is immediately visible.
 *
 * Accessibility: `role="dialog"` + `aria-modal`, a labelled title, a keyboard-
 * navigable stepper, Esc to close, and focus moved to the active step on each change.
 */
import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";

import { useApp } from "../../../app/AppContext";
import type { DeskDesc, FixConnection, WarehouseThreshold } from "../../../data/contract";
import { newDefaultHedgeRule, type HedgeRule } from "../../../lib/hedgeRules";
import { newRuleId, type RiskRule } from "../../../lib/riskRules";
import {
  applyWizard,
  defaultWizardThreshold,
  enabledBookKeys,
  hedgingErrors,
  newBookKey,
  portfolioErrors,
  routingErrors,
  thresholdErrors,
  type ApplyStepState,
  type WizardBook,
  type WizardDraft,
} from "./wizardModel";
import { HedgingStep, PortfoliosStep, ReviewStep, RoutingStep } from "./WizardSteps";
import styles from "./SetupWizard.module.css";

export interface SetupWizardProps {
  /** Close the wizard (discard). */
  onClose: () => void;
}

type ApplyPhase =
  | { kind: "idle" }
  | { kind: "applying"; steps: ApplyStepState[] }
  | { kind: "failed"; steps: ApplyStepState[] };

interface StepMeta {
  title: string;
  short: string;
}
const STEP_META: readonly StepMeta[] = [
  { title: "Risk portfolios", short: "Portfolios" },
  { title: "Routing", short: "Routing" },
  { title: "Internalise & hedge", short: "Hedge" },
  { title: "Review & apply", short: "Review" },
];

export function SetupWizard({ onClose }: SetupWizardProps): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const canRisk = auth.can("risk_manage", "fixed_income");
  const canHedge = auth.can("hedge", "fixed_income");

  const titleId = useId();
  const bodyRef = useRef<HTMLDivElement>(null);

  const [step, setStep] = useState(0);
  const [books, setBooks] = useState<WizardBook[]>(() => [
    { key: newBookKey(), name: "", parentKey: null, deskId: null, enabled: true, limits: null },
  ]);
  const [routingRules, setRoutingRules] = useState<RiskRule[]>(() => [
    { id: newRuleId(), conditions: [], bookId: null, enabled: true },
  ]);
  const [includeThreshold, setIncludeThreshold] = useState(true);
  const [threshold, setThreshold] = useState<WarehouseThreshold>(() => defaultWizardThreshold(""));
  const [hedgeRules, setHedgeRules] = useState<HedgeRule[]>(() => [newDefaultHedgeRule()]);
  const [desks, setDesks] = useState<DeskDesc[]>([]);
  const [connections, setConnections] = useState<FixConnection[]>([]);
  const [phase, setPhase] = useState<ApplyPhase>({ kind: "idle" });

  // Best-effort load of the rosters the routing conditions + portfolio desk picker read.
  useEffect(() => {
    let cancelled = false;
    void app.transport
      .listDesks()
      .then((d) => !cancelled && setDesks(d))
      .catch(() => undefined);
    if (canRisk) {
      void app.transport
        .listFixConnections()
        .then((c) => !cancelled && setConnections(c))
        .catch(() => undefined);
    }
    return () => {
      cancelled = true;
    };
  }, [app.transport, canRisk]);

  // Esc closes (unless mid-apply); move focus to the step body on each step change.
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape" && phase.kind !== "applying") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, phase.kind]);

  useEffect(() => {
    bodyRef.current?.focus();
  }, [step]);

  const enabledKeys = useMemo(() => enabledBookKeys(books), [books]);

  // Per-step blocking errors — a locked (uncapped) step never blocks.
  const stepErrors = useMemo((): string[][] => {
    return [
      canRisk ? portfolioErrors(books) : [],
      canRisk ? routingErrors(routingRules, enabledKeys) : [],
      canHedge ? [...(includeThreshold ? thresholdErrors(threshold) : []), ...hedgingErrors(hedgeRules)] : [],
      [],
    ];
  }, [canRisk, canHedge, books, routingRules, enabledKeys, includeThreshold, threshold, hedgeRules]);

  const currentErrors = stepErrors[step] ?? [];
  const stepLocked = (i: number): boolean => (i <= 1 ? !canRisk : i === 2 ? !canHedge : false);
  const stepComplete = (i: number): boolean => i < step && (stepErrors[i]?.length ?? 0) === 0;

  const goNext = useCallback((): void => {
    if (step < STEP_META.length - 1 && currentErrors.length === 0) setStep((s) => s + 1);
  }, [step, currentErrors.length]);
  const goBack = useCallback((): void => setStep((s) => Math.max(0, s - 1)), []);

  const draft: WizardDraft = useMemo(
    () => ({ books, routingRules, includeThreshold, threshold, hedgeRules }),
    [books, routingRules, includeThreshold, threshold, hedgeRules],
  );

  const onApply = useCallback(async (): Promise<void> => {
    setPhase({ kind: "applying", steps: [] });
    const result = await applyWizard(
      app.transport,
      draft,
      { risk: canRisk, hedge: canHedge },
      (steps) => setPhase({ kind: "applying", steps }),
    );
    if (result.ok) {
      app.setWorkspace("riskdashboard");
      onClose();
    } else {
      setPhase({ kind: "failed", steps: result.steps });
    }
  }, [app, draft, canRisk, canHedge, onClose]);

  const isLast = step === STEP_META.length - 1;
  const applying = phase.kind === "applying";

  return (
    <div
      className={styles.scrim}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && !applying) onClose();
      }}
    >
      <div className={styles.panel} role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <div className={styles.head}>
          <div className={styles.headMain}>
            <span className={styles.wand} aria-hidden="true">
              🪄
            </span>
            <div>
              <h2 id={titleId} className={styles.title}>
                Guided setup
              </h2>
              <p className={styles.subtitle}>
                Define your risk portfolios, routing and hedging in one flow.
              </p>
            </div>
          </div>
          <button
            type="button"
            className={styles.closeBtn}
            data-testid="wiz-close"
            aria-label="Close guided setup"
            disabled={applying}
            onClick={onClose}
          >
            ✕
          </button>
        </div>

        {(!canRisk || !canHedge) && (
          <p className={styles.capBanner} role="note" data-testid="wiz-cap-banner">
            {!canRisk && "You lack Manage-Risk — the portfolio & routing steps are read-only. "}
            {!canHedge && "You lack Hedge — the internalise & hedge step is read-only. "}
            Those steps will be skipped on Apply.
          </p>
        )}

        <div className={styles.layout}>
          <nav className={styles.stepper} aria-label="Setup steps">
            <ol className={styles.stepperList}>
              {STEP_META.map((m, i) => {
                const state = i === step ? "active" : stepComplete(i) ? "complete" : "upcoming";
                return (
                  <li key={m.title} className={styles.stepperItem}>
                    <button
                      type="button"
                      className={`${styles.stepNode} ${styles[`stepNode_${state}` as const] ?? ""}`}
                      aria-current={i === step ? "step" : undefined}
                      data-testid={`wiz-step-${i}`}
                      onClick={() => !applying && setStep(i)}
                      disabled={applying}
                    >
                      <span className={styles.stepDot} aria-hidden="true">
                        {state === "complete" ? "✓" : i + 1}
                      </span>
                      <span className={styles.stepLabels}>
                        <span className={styles.stepTitle}>{m.title}</span>
                        {stepLocked(i) && <span className={styles.stepLocked}>read-only</span>}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ol>
          </nav>

          <div className={styles.content}>
            {phase.kind === "idle" ? (
              <div className={styles.stepScroll} ref={bodyRef} tabIndex={-1} aria-label={STEP_META[step]?.title}>
                {step === 0 && (
                  <PortfoliosStep books={books} desks={desks} readOnly={!canRisk} onChange={setBooks} />
                )}
                {step === 1 && (
                  <RoutingStep
                    rules={routingRules}
                    books={books}
                    desks={desks}
                    connections={connections}
                    readOnly={!canRisk}
                    onChange={setRoutingRules}
                  />
                )}
                {step === 2 && (
                  <HedgingStep
                    includeThreshold={includeThreshold}
                    threshold={threshold}
                    hedgeRules={hedgeRules}
                    books={books}
                    readOnly={!canHedge}
                    onToggleThreshold={setIncludeThreshold}
                    onChangeThreshold={setThreshold}
                    onChangeHedgeRules={setHedgeRules}
                  />
                )}
                {step === 3 && <ReviewStep draft={draft} canRisk={canRisk} canHedge={canHedge} />}
              </div>
            ) : (
              <ApplyPanel steps={phase.steps} failed={phase.kind === "failed"} />
            )}
          </div>
        </div>

        <div className={styles.foot}>
          {phase.kind === "failed" ? (
            <>
              <p className={styles.footError} role="alert" data-testid="wiz-apply-error">
                Apply stopped — nothing further was written. Fix the issue and try again.
              </p>
              <div className={styles.footActions}>
                <button type="button" className={styles.ghostBtn} onClick={() => setPhase({ kind: "idle" })}>
                  Back to edit
                </button>
                <button type="button" className={styles.ghostBtn} onClick={onClose}>
                  Close
                </button>
              </div>
            </>
          ) : (
            <>
              {currentErrors.length > 0 && phase.kind === "idle" && (
                <ul className={styles.footIssues} data-testid="wiz-step-errors">
                  {currentErrors.slice(0, 3).map((msg, i) => (
                    <li key={i}>⚠ {msg}</li>
                  ))}
                </ul>
              )}
              <div className={styles.footActions}>
                <button
                  type="button"
                  className={styles.ghostBtn}
                  onClick={goBack}
                  disabled={step === 0 || applying}
                  data-testid="wiz-back"
                >
                  Back
                </button>
                {isLast ? (
                  <button
                    type="button"
                    className={styles.primaryBtn}
                    onClick={() => void onApply()}
                    disabled={applying}
                    data-testid="wiz-apply"
                  >
                    {applying ? "Applying…" : "Apply setup"}
                  </button>
                ) : (
                  <button
                    type="button"
                    className={styles.primaryBtn}
                    onClick={goNext}
                    disabled={currentErrors.length > 0}
                    data-testid="wiz-next"
                  >
                    Next
                  </button>
                )}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

/** The Apply progress list — one row per dependency-ordered persistence step. */
function ApplyPanel({ steps, failed }: { steps: ApplyStepState[]; failed: boolean }): React.ReactElement {
  return (
    <div className={styles.stepScroll} aria-live="polite">
      <h3 className={styles.applyHeading}>{failed ? "Apply failed" : "Applying your setup…"}</h3>
      <ol className={styles.applyList} data-testid="wiz-apply-progress">
        {steps.map((s) => (
          <li key={s.id} className={`${styles.applyRow} ${styles[`apply_${s.status}` as const] ?? ""}`} data-testid={`wiz-apply-${s.id}`}>
            <span className={styles.applyIcon} aria-hidden="true">
              {s.status === "done"
                ? "✓"
                : s.status === "error"
                  ? "✕"
                  : s.status === "running"
                    ? "…"
                    : s.status === "skipped"
                      ? "–"
                      : "○"}
            </span>
            <span className={styles.applyLabel}>{s.label}</span>
            <span className={styles.applyDetail}>{s.detail}</span>
          </li>
        ))}
      </ol>
    </div>
  );
}
