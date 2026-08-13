/**
 * SetupWizard — the Hedging guided-setup flow. A single, sequential on-ramp that walks a
 * trader through the whole risk-lifecycle model in one place, instead of navigating three
 * separate surfaces (Risk Portfolios → Risk Routing → Hedging) and having to already
 * understand how they relate:
 *
 *   1. Risk portfolios — create the buckets risk books into
 *   2. Routing — decide which fills land in which portfolio
 *   3. Internalise & hedge — the warehouse cap + a starter exit policy
 *   4. Review & apply — commit everything through the existing RPCs, in order
 *
 * The chrome (stepper, footer, apply-progress, a11y) is the SHARED {@link WizardShell};
 * steps 1–2 are the SHARED {@link PortfoliosStep} / {@link RoutingStep} — this file owns
 * only the hedge-specific step CONTENT + draft state + the Apply sequence
 * ({@link applyWizard}). Capability-gated: steps 1–2 need `risk_manage·FI`, step 3 needs
 * `hedge·FI` — a step the caller can't apply renders read-only with a note rather than
 * failing at Apply. On success the wizard closes and lands the trader on the Risk
 * Dashboard so the new configuration is immediately visible.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../../app/AppContext";
import { useReferenceData } from "../../../hooks/useReferenceData";
import type { DeskDesc, FixConnection, WarehouseThreshold } from "../../../data/contract";
import { newDefaultHedgeRule, type HedgeRule } from "../../../lib/hedgeRules";
import { newRuleId, type RiskRule } from "../../../lib/riskRules";
import { notifyRiskRoutingChanged } from "../../../lib/routingGuard";
import {
  WizardShell,
  type ApplyPhase,
  type StepMeta,
} from "../../setupWizard/WizardShell";
import {
  applyWizard,
  defaultWizardThreshold,
  enabledBookKeys,
  hedgingErrors,
  newBookKey,
  policyScopeErrors,
  portfolioErrors,
  routingErrors,
  thresholdErrors,
  type WizardBook,
  type WizardDraft,
  type WizardPolicyScope,
} from "./wizardModel";
import { HedgingStep, PortfoliosStep, ReviewStep, RoutingStep } from "./WizardSteps";

export interface SetupWizardProps {
  /** Close the wizard (discard). */
  onClose: () => void;
}

const STEP_META: readonly StepMeta[] = [
  { title: "Risk portfolios", short: "Portfolios" },
  { title: "Routing", short: "Routing" },
  { title: "Internalise & hedge", short: "Hedge" },
  { title: "Review & apply", short: "Review" },
];

export function SetupWizard({ onClose }: SetupWizardProps): React.ReactElement {
  const app = useApp();
  // The aggregation instruments a CROSS_INTERNAL exit may target — real ids from the
  // reference-data registry, not a curated list of made-up ones.
  const refData = useReferenceData(app.transport, app.auth.user != null);
  const instrumentOptions = useMemo(
    () =>
      refData.instruments
        .map((d) => d.instrumentId)
        .filter((id) => id.length > 0)
        .sort((a, b) => a.localeCompare(b)),
    [refData.instruments],
  );
  const { auth } = app;
  const canRisk = auth.can("risk_manage", "fixed_income");
  const canHedge = auth.can("hedge", "fixed_income");

  const [step, setStep] = useState(0);
  const [books, setBooks] = useState<WizardBook[]>(() => [
    { key: newBookKey(), name: "", parentKey: null, deskId: null, enabled: true, limits: null, assetClass: "fixed_income" },
  ]);
  const [routingRules, setRoutingRules] = useState<RiskRule[]>(() => [
    { id: newRuleId(), conditions: [], bookId: null, enabled: true },
  ]);
  const [includeThreshold, setIncludeThreshold] = useState(true);
  const [threshold, setThreshold] = useState<WarehouseThreshold>(() => defaultWizardThreshold(""));
  const [hedgeRules, setHedgeRules] = useState<HedgeRule[]>(() => [newDefaultHedgeRule()]);
  const [policyScope, setPolicyScope] = useState<WizardPolicyScope>(() => ({
    scopeKind: "firm",
    scopeId: "",
  }));
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

  const enabledKeys = useMemo(() => enabledBookKeys(books), [books]);

  // Per-step blocking errors — a locked (uncapped) step never blocks.
  const stepErrors = useMemo((): string[][] => {
    return [
      canRisk ? portfolioErrors(books) : [],
      canRisk ? routingErrors(routingRules, enabledKeys) : [],
      canHedge
        ? [
            ...(includeThreshold ? thresholdErrors(threshold) : []),
            ...hedgingErrors(hedgeRules),
            ...policyScopeErrors(policyScope, books),
          ]
        : [],
      [],
    ];
  }, [canRisk, canHedge, books, routingRules, enabledKeys, includeThreshold, threshold, hedgeRules, policyScope]);

  const currentErrors = stepErrors[step] ?? [];
  const stepLocked = (i: number): boolean => (i <= 1 ? !canRisk : i === 2 ? !canHedge : false);
  const stepComplete = (i: number): boolean => i < step && (stepErrors[i]?.length ?? 0) === 0;

  const goNext = useCallback((): void => {
    if (step < STEP_META.length - 1 && currentErrors.length === 0) setStep((s) => s + 1);
  }, [step, currentErrors.length]);
  const goBack = useCallback((): void => setStep((s) => Math.max(0, s - 1)), []);

  const draft: WizardDraft = useMemo(
    () => ({ books, routingRules, includeThreshold, threshold, hedgeRules, policyScope }),
    [books, routingRules, includeThreshold, threshold, hedgeRules, policyScope],
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
      // Let the startup routing guard re-evaluate immediately (a valid default may have
      // just been set), so its warning retires without a reload.
      notifyRiskRoutingChanged();
      app.setWorkspace("riskdashboard");
      onClose();
    } else {
      setPhase({ kind: "failed", steps: result.steps });
    }
  }, [app, draft, canRisk, canHedge, onClose]);

  const capBanner =
    !canRisk || !canHedge ? (
      <>
        {!canRisk && "You lack Manage-Risk — the portfolio & routing steps are read-only. "}
        {!canHedge && "You lack Hedge — the internalise & hedge step is read-only. "}
        Those steps will be skipped on Apply.
      </>
    ) : null;

  return (
    <WizardShell
      title="Guided setup"
      subtitle="Define your risk portfolios, routing and hedging in one flow."
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
        <HedgingStep
          includeThreshold={includeThreshold}
          threshold={threshold}
          hedgeRules={hedgeRules}
          books={books}
          policyScope={policyScope}
          readOnly={!canHedge}
          onToggleThreshold={setIncludeThreshold}
          onChangeThreshold={setThreshold}
          onChangeHedgeRules={setHedgeRules}
          onChangePolicyScope={setPolicyScope}
          instrumentOptions={instrumentOptions}
        />
      )}
      {step === 3 && <ReviewStep draft={draft} canRisk={canRisk} canHedge={canHedge} />}
    </WizardShell>
  );
}
