/**
 * RiskSetupWizard — the Risk guided-setup flow. The risk-management sibling of the
 * Hedging wizard: one sequential on-ramp that walks a trader through the whole
 * incoming-risk model in one place instead of navigating three separate Risk tabs
 * (Portfolios → Routing → Acceptance) and having to already understand how they relate:
 *
 *   1. Risk portfolios — create the buckets risk books into        (shared step)
 *   2. Routing — decide which fills land in which portfolio         (shared step)
 *   3. Acceptance criteria — accept / reject / hold incoming lifts  (risk-specific)
 *   4. Review & apply — commit everything through the existing RPCs, in order
 *
 * The chrome (stepper, footer, apply-progress, a11y) is the SHARED {@link WizardShell};
 * steps 1–2 are the SHARED {@link PortfoliosStep} / {@link RoutingStep} — this file owns
 * only the acceptance step CONTENT + draft state + the Apply sequence
 * ({@link applyRiskWizard}). Capability-gated: steps 1–2 need `risk_manage·FI`, step 3
 * needs `manage_acceptance·FI` — a step the caller can't apply renders read-only with a
 * note rather than failing at Apply. On success the wizard closes and lands the trader on
 * the Risk → Acceptance tab so the new policy is immediately visible.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../app/AppContext";
import type { DeskDesc, FixConnection } from "../../data/contract";
import type { AcceptanceRule } from "../../lib/acceptanceRules";
import { newRuleId, type RiskRule } from "../../lib/riskRules";
import { notifyRiskRoutingChanged } from "../../lib/routingGuard";
import {
  WizardShell,
  type ApplyPhase,
  type StepMeta,
} from "../setupWizard/WizardShell";
import {
  enabledBookKeys,
  newBookKey,
  portfolioErrors,
  routingErrors,
  type WizardBook,
} from "../setupWizard/wizardModel";
import {
  acceptanceErrors,
  applyRiskWizard,
  defaultAcceptanceRules,
  type RiskWizardDraft,
} from "./riskWizardModel";
import { AcceptanceStep, PortfoliosStep, RiskReviewStep, RoutingStep } from "./RiskWizardSteps";

export interface RiskSetupWizardProps {
  /** Close the wizard (discard). */
  onClose: () => void;
}

const STEP_META: readonly StepMeta[] = [
  { title: "Risk portfolios", short: "Portfolios" },
  { title: "Routing", short: "Routing" },
  { title: "Acceptance criteria", short: "Acceptance" },
  { title: "Review & apply", short: "Review" },
];

export function RiskSetupWizard({ onClose }: RiskSetupWizardProps): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const canRisk = auth.can("risk_manage", "fixed_income");
  const canAcceptance = auth.can("manage_acceptance", "fixed_income");

  const [step, setStep] = useState(0);
  const [books, setBooks] = useState<WizardBook[]>(() => [
    { key: newBookKey(), name: "", parentKey: null, deskId: null, enabled: true, limits: null },
  ]);
  const [routingRules, setRoutingRules] = useState<RiskRule[]>(() => [
    { id: newRuleId(), conditions: [], bookId: null, enabled: true },
  ]);
  const [acceptanceRules, setAcceptanceRules] = useState<AcceptanceRule[]>(() => defaultAcceptanceRules());
  const [desks, setDesks] = useState<DeskDesc[]>([]);
  const [connections, setConnections] = useState<FixConnection[]>([]);
  const [phase, setPhase] = useState<ApplyPhase>({ kind: "idle" });

  // Best-effort load of the rosters the routing + acceptance conditions read.
  useEffect(() => {
    let cancelled = false;
    void app.transport
      .listDesks()
      .then((d) => !cancelled && setDesks(d))
      .catch(() => undefined);
    if (canRisk || canAcceptance) {
      void app.transport
        .listFixConnections()
        .then((c) => !cancelled && setConnections(c))
        .catch(() => undefined);
    }
    return () => {
      cancelled = true;
    };
  }, [app.transport, canRisk, canAcceptance]);

  const enabledKeys = useMemo(() => enabledBookKeys(books), [books]);

  // Per-step blocking errors — a locked (uncapped) step never blocks.
  const stepErrors = useMemo((): string[][] => {
    return [
      canRisk ? portfolioErrors(books) : [],
      canRisk ? routingErrors(routingRules, enabledKeys) : [],
      canAcceptance ? acceptanceErrors(acceptanceRules) : [],
      [],
    ];
  }, [canRisk, canAcceptance, books, routingRules, enabledKeys, acceptanceRules]);

  const currentErrors = stepErrors[step] ?? [];
  const stepLocked = (i: number): boolean => (i <= 1 ? !canRisk : i === 2 ? !canAcceptance : false);
  const stepComplete = (i: number): boolean => i < step && (stepErrors[i]?.length ?? 0) === 0;

  const goNext = useCallback((): void => {
    if (step < STEP_META.length - 1 && currentErrors.length === 0) setStep((s) => s + 1);
  }, [step, currentErrors.length]);
  const goBack = useCallback((): void => setStep((s) => Math.max(0, s - 1)), []);

  const draft: RiskWizardDraft = useMemo(
    () => ({ books, routingRules, acceptanceRules }),
    [books, routingRules, acceptanceRules],
  );

  const onApply = useCallback(async (): Promise<void> => {
    setPhase({ kind: "applying", steps: [] });
    const result = await applyRiskWizard(
      app.transport,
      draft,
      { risk: canRisk, acceptance: canAcceptance },
      (steps) => setPhase({ kind: "applying", steps }),
    );
    if (result.ok) {
      // A valid routing default may have just been set — let the startup guard re-check.
      notifyRiskRoutingChanged();
      // Land on the Risk → Acceptance tab so the new policy is immediately visible.
      app.setWorkspace("acceptance");
      onClose();
    } else {
      setPhase({ kind: "failed", steps: result.steps });
    }
  }, [app, draft, canRisk, canAcceptance, onClose]);

  const capBanner =
    !canRisk || !canAcceptance ? (
      <>
        {!canRisk && "You lack Manage-Risk — the portfolio & routing steps are read-only. "}
        {!canAcceptance && "You lack Manage-Acceptance — the acceptance step is read-only. "}
        Those steps will be skipped on Apply.
      </>
    ) : null;

  return (
    <WizardShell
      title="Risk guided setup"
      subtitle="Define your risk portfolios, routing and acceptance in one flow."
      steps={STEP_META}
      step={step}
      onStep={setStep}
      stepLocked={stepLocked}
      stepComplete={stepComplete}
      currentErrors={currentErrors}
      capBanner={capBanner}
      phase={phase}
      onClose={onClose}
      onBack={goBack}
      onNext={goNext}
      onApply={() => void onApply()}
      onEditAgain={() => setPhase({ kind: "idle" })}
      applyLabel="Apply setup"
    >
      {step === 0 && <PortfoliosStep books={books} desks={desks} readOnly={!canRisk} onChange={setBooks} />}
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
        <AcceptanceStep
          rules={acceptanceRules}
          desks={desks}
          connections={connections}
          readOnly={!canAcceptance}
          onChange={setAcceptanceRules}
        />
      )}
      {step === 3 && <RiskReviewStep draft={draft} canRisk={canRisk} canAcceptance={canAcceptance} />}
    </WizardShell>
  );
}
