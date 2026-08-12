/**
 * ExitPolicyWizard — the GUIDED-RULE on-ramp for Hedging Rules → Exit Policy. It sits
 * next to "+ Create hedge rule" and turns a plain-English hedge INTENT into a complete,
 * valid exit-policy rule set, in two complementary modes woven into one flow:
 *
 *   1. ANALYSE MY FLOW (optional) — reads the data the GUI already holds: the selected
 *      book's rolled-up risk from the risk-dashboard store ({@link RiskBookRisk}) and the
 *      client-blotter {@link Deal}s. It surfaces the book's net DV01 ($/bp) and net
 *      notional ($) — kept HONESTLY DISTINCT — the position/deal counts and peak
 *      utilisation, and suggests a rounded warehouse cap + flatten trigger. No flow ⇒ it
 *      says so; it never fabricates a number ({@link summariseBookFlow} / {@link suggestThresholds}).
 *
 *   2. SCENARIO PRESETS — a card per real intent ("warehouse until a size limit, then
 *      flatten", "hold small / escalate large", "back-to-back a counterparty", "pure
 *      internalisation"). Each shows what it does in plain English + a LIVE preview of the
 *      exact rules it will create, parameterised by a METRIC (Net DV01 vs Net notional,
 *      unit-correct) + threshold(s) that the analysis step can pre-fill.
 *
 * The generated rules are the SAME {@link HedgeRule} graphs the manual builder emits
 * ({@link buildScenarioRules}); on Insert they REPLACE the current scope's draft rules and
 * hand straight to the existing PolicyTab list + validity/conflict panel + Save path —
 * nothing is written until the trader presses Save policy. Chrome is the shared
 * {@link WizardShell}; gated read-only for a non-`hedge·FI` holder.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../app/AppContext";
import type { Deal, RiskBookRisk } from "../../data/contract";
import { HelpButton } from "../../components/HelpButton";
import { fmtCompact } from "../../lib/format";
import {
  compileRulesToHedgeGraph,
  describeHedgeRule,
  detectHedgeRuleConflicts,
  type HedgeRule,
} from "../../lib/hedgeRules";
import { validateHedgeGraph } from "../../lib/hedgeTrace";
import {
  METRIC_DESCRIPTORS,
  SCENARIO_METRICS,
  SCENARIOS,
  buildScenarioRules,
  defaultScenarioParams,
  scenarioParamErrors,
  scenarioSpec,
  summariseBookFlow,
  suggestThresholds,
  type BookFlowSummary,
  type ScenarioId,
  type ScenarioMetric,
  type ScenarioParams,
} from "../../lib/hedgeScenario";
import { WizardShell, type StepMeta } from "../setupWizard/WizardShell";
import { Explainer } from "../setupWizard/PortfolioRoutingSteps";
import styles from "../setupWizard/setupWizard.module.css";

import { MagnitudeField } from "../../components/MagnitudeField";

export interface ExitPolicyWizardProps {
  /** The scope's currently-selected book id (defaults the analysis book), if any. */
  scopeBookId?: string | undefined;
  /** Whether the caller can author (no `hedge·FI` ⇒ read-only, Insert disabled). */
  readOnly: boolean;
  /** Replace the PolicyTab draft rules with the generated set (the trader then Saves). */
  onInsert: (rules: HedgeRule[]) => void;
  /** Close / discard the wizard. */
  onClose: () => void;
}

const STEP_META: readonly StepMeta[] = [
  { title: "Analyse my flow", short: "Analyse" },
  { title: "Choose a scenario", short: "Scenario" },
  { title: "Review & insert", short: "Review" },
];

/** Format a metric magnitude with its unit ("$" prefix / "$/bp" suffix). */
function fmtMetric(value: number, metric: ScenarioMetric): string {
  return metric === "net_notional"
    ? `$${fmtCompact(value)}`
    : `$${fmtCompact(value)}/bp`;
}

export function ExitPolicyWizard({
  scopeBookId,
  readOnly,
  onInsert,
  onClose,
}: ExitPolicyWizardProps): React.ReactElement {
  const app = useApp();

  const [step, setStep] = useState(0);
  const [risk, setRisk] = useState<RiskBookRisk[]>([]);
  const [deals, setDeals] = useState<Deal[]>([]);
  const [analysisBookId, setAnalysisBookId] = useState<string>(scopeBookId ?? "");
  const [scenarioId, setScenarioId] = useState<ScenarioId>("warehouse-then-flatten");
  const [params, setParams] = useState<ScenarioParams>(() => defaultScenarioParams());

  // Best-effort load of the risk-dashboard store + client-blotter deals the analysis reads.
  useEffect(() => {
    let cancelled = false;
    void app.transport
      .listRiskBookRisk()
      .then((r) => {
        if (cancelled) return;
        setRisk(r);
        // Default the analysis book to the scope's book, else the first with flow.
        setAnalysisBookId((cur) => {
          if (cur && r.some((b) => b.bookId === cur)) return cur;
          const withFlow = r.find((b) => b.positionCount > 0);
          return withFlow?.bookId ?? r[0]?.bookId ?? "";
        });
      })
      .catch(() => undefined);
    void app.transport
      .listDeals({})
      .then((res) => !cancelled && setDeals(res.deals))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [app.transport]);

  const summary = useMemo((): BookFlowSummary | null => {
    const book = risk.find((b) => b.bookId === analysisBookId);
    return book ? summariseBookFlow(book, deals) : null;
  }, [risk, deals, analysisBookId]);

  const rules = useMemo(() => buildScenarioRules(scenarioId, params), [scenarioId, params]);

  const paramErrors = useMemo(() => scenarioParamErrors(scenarioId, params), [scenarioId, params]);
  const graphIssues = useMemo(
    () => validateHedgeGraph(compileRulesToHedgeGraph(rules.filter((r) => r.enabled))),
    [rules],
  );
  const conflictErrors = useMemo(
    () => detectHedgeRuleConflicts(rules).filter((c) => c.severity === "error").length,
    [rules],
  );

  const stepErrors = useMemo(
    (): string[][] => [[], readOnly ? ["You lack the Hedge · Fixed-Income capability — read-only."] : paramErrors, []],
    [paramErrors, readOnly],
  );
  const currentErrors = stepErrors[step] ?? [];

  const goNext = useCallback(
    (): void => setStep((s) => (currentErrors.length === 0 ? Math.min(STEP_META.length - 1, s + 1) : s)),
    [currentErrors.length],
  );
  const goBack = useCallback((): void => setStep((s) => Math.max(0, s - 1)), []);

  const onApply = useCallback((): void => {
    if (readOnly || paramErrors.length > 0) return;
    onInsert(rules);
    onClose();
  }, [readOnly, paramErrors.length, rules, onInsert, onClose]);

  // Apply the analysis suggestion to the scenario params, then advance to the scenario step.
  const applySuggestion = useCallback(
    (metric: ScenarioMetric): void => {
      if (summary === null) return;
      const sug = suggestThresholds(summary, metric);
      if (sug === null) return;
      setParams((p) => ({ ...p, metric, threshold: sug.flattenThreshold, escalateThreshold: sug.cap }));
      setStep(1);
    },
    [summary],
  );

  const capBanner = readOnly ? (
    <>You lack Hedge · Fixed-Income — you can explore the wizard, but Insert is disabled.</>
  ) : null;

  return (
    <WizardShell
      title="New rule wizard"
      subtitle="Analyse your flow, pick a hedge intent, and generate a valid exit policy."
      steps={STEP_META}
      step={step}
      onStep={setStep}
      stepLocked={() => false}
      stepComplete={(i) => i < step && (stepErrors[i]?.length ?? 0) === 0}
      currentErrors={currentErrors}
      capBanner={capBanner}
      phase={{ kind: "idle" }}
      onClose={onClose}
      onBack={goBack}
      onNext={goNext}
      onApply={onApply}
      onEditAgain={() => undefined}
      applyLabel="Insert rules"
    >
      {step === 0 && (
        <AnalyseStep
          risk={risk}
          analysisBookId={analysisBookId}
          summary={summary}
          onPickBook={setAnalysisBookId}
          onUseSuggestion={applySuggestion}
        />
      )}
      {step === 1 && (
        <ScenarioStep
          scenarioId={scenarioId}
          params={params}
          rules={rules}
          readOnly={readOnly}
          onPickScenario={setScenarioId}
          onChangeParams={setParams}
        />
      )}
      {step === 2 && (
        <ReviewStep rules={rules} graphIssues={graphIssues.length} conflictErrors={conflictErrors} scopeBookId={scopeBookId} />
      )}
    </WizardShell>
  );
}

// --- step 0: analyse my flow ------------------------------------------------

function AnalyseStep({
  risk,
  analysisBookId,
  summary,
  onPickBook,
  onUseSuggestion,
}: {
  risk: readonly RiskBookRisk[];
  analysisBookId: string;
  summary: BookFlowSummary | null;
  onPickBook: (id: string) => void;
  onUseSuggestion: (metric: ScenarioMetric) => void;
}): React.ReactElement {
  return (
    <div className={styles.stepBody}>
      <Explainer icon="🔎" title="Analyse the book's flow (optional)">
        Reading the book&rsquo;s rolled-up risk and its recent client deals, the wizard suggests sensible
        thresholds. <strong>Net DV01</strong> is a rate risk ($/bp); <strong>Net notional</strong> is a face
        amount ($) — they are kept distinct. Pick a book, then apply a suggestion, or skip straight to a
        scenario.
      </Explainer>

      <div className={styles.subCard}>
        <label className={styles.field}>
          <span className={styles.miniLabel}>Book to analyse</span>
          <select
            className={styles.select}
            value={analysisBookId}
            data-testid="rw-analysis-book"
            onChange={(e) => onPickBook(e.target.value)}
          >
            <option value="">(select a book)</option>
            {risk.map((b) => (
              <option key={b.bookId} value={b.bookId}>
                {b.name}
              </option>
            ))}
          </select>
        </label>

        {summary === null ? (
          <p className={styles.hint} data-testid="rw-analysis-empty">
            Select a book above to see its flow.
          </p>
        ) : !summary.hasFlow ? (
          <p className={styles.warnNote} data-testid="rw-analysis-noflow">
            {summary.bookName} has no positions or recent deals — not enough flow to suggest a threshold. Pick
            a scenario and set the threshold yourself.
          </p>
        ) : (
          <>
            <div className={styles.statGrid} data-testid="rw-analysis-stats">
              <StatTile
                label="Net DV01"
                value={summary.netDv01 === null ? "—" : `$${fmtCompact(summary.netDv01)}`}
                unit="$/bp risk"
              />
              <StatTile label="Net notional" value={`$${fmtCompact(summary.netNotional)}`} unit="$ face" />
              <StatTile label="Gross notional" value={`$${fmtCompact(summary.grossNotional)}`} unit="$ face" />
              <StatTile label="Positions" value={String(summary.positionCount)} unit="rolled up" />
              <StatTile label="Recent deals" value={String(summary.dealCount)} unit="on the blotter" />
              <StatTile
                label="Peak utilisation"
                value={summary.peakUtilizationPct === null ? "—" : `${summary.peakUtilizationPct.toFixed(0)}%`}
                unit="of cap"
              />
            </div>

            <div className={styles.suggestRow}>
              {SCENARIO_METRICS.map((m) => {
                const sug = suggestThresholds(summary, m);
                const md = METRIC_DESCRIPTORS[m];
                if (sug === null) {
                  return (
                    <span key={m} className={styles.hint}>
                      {md.label}: not evaluable for this book.
                    </span>
                  );
                }
                return (
                  <button
                    key={m}
                    type="button"
                    className={styles.addConditionBtn}
                    data-testid={`rw-use-suggestion-${m}`}
                    onClick={() => onUseSuggestion(m)}
                  >
                    Use {md.label}: flatten &gt; {fmtMetric(sug.flattenThreshold, m)} (cap {fmtMetric(sug.cap, m)})
                  </button>
                );
              })}
            </div>
            <p className={styles.hint}>
              Suggested cap ≈ 1.25× the book&rsquo;s current |exposure| rounded to a round number; the flatten
              trigger is 0.8× the cap, so the policy hedges before the appetite is used up.
            </p>
          </>
        )}
      </div>
    </div>
  );
}

function StatTile({ label, value, unit }: { label: string; value: string; unit: string }): React.ReactElement {
  return (
    <div className={styles.statTile}>
      <span className={styles.statLabel}>{label}</span>
      <span className={styles.statValue}>{value}</span>
      <span className={styles.statUnit}>{unit}</span>
    </div>
  );
}

// --- step 1: choose a scenario ----------------------------------------------

function ScenarioStep({
  scenarioId,
  params,
  rules,
  readOnly,
  onPickScenario,
  onChangeParams,
}: {
  scenarioId: ScenarioId;
  params: ScenarioParams;
  rules: readonly HedgeRule[];
  readOnly: boolean;
  onPickScenario: (id: ScenarioId) => void;
  onChangeParams: (next: ScenarioParams) => void;
}): React.ReactElement {
  const spec = scenarioSpec(scenarioId);
  const md = METRIC_DESCRIPTORS[params.metric];
  const patch = (p: Partial<ScenarioParams>): void => onChangeParams({ ...params, ...p });

  return (
    <div className={styles.stepBody}>
      <Explainer icon="🧭" title="Pick a hedge intent">
        Each card is a real hedge intent that generates a complete, valid exit policy. Choose one, set its
        inputs, and watch the exact rules it will create below.
      </Explainer>

      <div className={styles.scenarioGrid} data-testid="rw-scenario-grid" role="radiogroup" aria-label="Hedge scenario">
        {SCENARIOS.map((s) => (
          <button
            key={s.id}
            type="button"
            role="radio"
            aria-checked={s.id === scenarioId}
            className={`${styles.scenarioCard} ${s.id === scenarioId ? styles.scenarioCardActive : ""}`}
            data-testid={`rw-scenario-${s.id}`}
            onClick={() => onPickScenario(s.id)}
          >
            <span className={styles.scenarioCardTitle}>{s.title}</span>
            <p className={styles.scenarioCardBody}>{s.plain}</p>
          </button>
        ))}
      </div>

      {(spec.inputs.metric || spec.inputs.counterparty) && (
        <div className={styles.subCard} data-testid="rw-scenario-params">
          <h4 className={styles.subHeading}>Scenario inputs</h4>
          <div className={styles.entryGrid}>
            {spec.inputs.metric && (
              <label className={styles.field}>
                <span className={styles.miniLabel}>Size metric</span>
                <select
                  className={styles.select}
                  value={params.metric}
                  disabled={readOnly}
                  data-testid="rw-metric"
                  onChange={(e) => patch({ metric: e.target.value as ScenarioMetric })}
                >
                  {SCENARIO_METRICS.map((m) => (
                    <option key={m} value={m}>
                      {METRIC_DESCRIPTORS[m].label} ({METRIC_DESCRIPTORS[m].unit})
                    </option>
                  ))}
                </select>
                <span className={styles.hint}>{md.blurb}</span>
              </label>
            )}
            {spec.inputs.metric && (
              <label className={styles.field}>
                <span className={styles.miniLabel}>
                  {spec.inputs.escalate ? "First threshold" : "Threshold"} ({md.unit})
                </span>
                <MagnitudeField
                  className={styles.input}
                  allowBlank={false}
                  value={params.threshold}
                  disabled={readOnly}
                  data-testid="rw-threshold"
                  onCommit={(v) => {
                    if (v !== null) patch({ threshold: v });
                  }}
                />
                <span className={styles.hint}>
                  {params.threshold > 0 ? `= ${fmtMetric(params.threshold, params.metric)}` : "set a positive value"}
                </span>
              </label>
            )}
            {spec.inputs.escalate && (
              <label className={styles.field}>
                <span className={styles.miniLabel}>Upper (escalation) threshold ({md.unit})</span>
                <MagnitudeField
                  className={styles.input}
                  allowBlank={false}
                  value={params.escalateThreshold}
                  disabled={readOnly}
                  data-testid="rw-escalate"
                  onCommit={(v) => {
                    if (v !== null) patch({ escalateThreshold: v });
                  }}
                />
                <span className={styles.hint}>
                  {params.escalateThreshold > 0
                    ? `= ${fmtMetric(params.escalateThreshold, params.metric)}`
                    : "must exceed the first threshold"}
                </span>
              </label>
            )}
            {spec.inputs.counterparty && (
              <label className={styles.field}>
                <span className={styles.miniLabel}>Counterparty</span>
                <input
                  className={styles.input}
                  type="text"
                  value={params.counterparty}
                  disabled={readOnly}
                  data-testid="rw-counterparty"
                  placeholder="e.g. CITADEL"
                  onChange={(e) => patch({ counterparty: e.target.value })}
                />
                <span className={styles.hint}>Matches the originating party-id exactly as the blotter shows it.</span>
              </label>
            )}
          </div>
        </div>
      )}

      <RulePreview rules={rules} testid="rw-scenario-preview" heading="Rules this scenario will create" />
    </div>
  );
}

// --- step 2: review & insert ------------------------------------------------

function ReviewStep({
  rules,
  graphIssues,
  conflictErrors,
  scopeBookId,
}: {
  rules: readonly HedgeRule[];
  graphIssues: number;
  conflictErrors: number;
  scopeBookId?: string | undefined;
}): React.ReactElement {
  const ok = graphIssues === 0 && conflictErrors === 0;
  return (
    <div className={styles.stepBody}>
      <Explainer icon="✅" title="Review & insert">
        These rules will <strong>replace</strong> the current exit policy for{" "}
        {scopeBookId ? "the selected scope" : "the Firm scope"} in the editor. Nothing is written until you
        press <strong>Insert rules</strong> here, then <strong>Save policy</strong> on the Exit Policy screen —
        where you can still tweak them and see the validity/conflict panel.
      </Explainer>

      <div className={styles.subCard}>
        {ok ? (
          <p className={styles.hint} data-testid="rw-review-valid">
            ✓ Valid — {rules.length} rule{rules.length === 1 ? "" : "s"}, no conflicts.
          </p>
        ) : (
          <p className={styles.warnNote} data-testid="rw-review-invalid">
            ⚠ {graphIssues} structural issue{graphIssues === 1 ? "" : "s"}, {conflictErrors} conflict
            {conflictErrors === 1 ? "" : "s"} — adjust the scenario inputs.
          </p>
        )}
        <RulePreview rules={rules} testid="rw-review-preview" heading="Generated rules (priority order)" />
      </div>
    </div>
  );
}

// --- shared: rule preview ---------------------------------------------------

function RulePreview({
  rules,
  heading,
  testid,
}: {
  rules: readonly HedgeRule[];
  heading: string;
  testid: string;
}): React.ReactElement {
  return (
    <div className={styles.subCard}>
      <h4 className={styles.subHeading}>
        {heading}
        <HelpButton helpId="concept.hedge-rule-wizard" subject="the exit-policy rule wizard" />
      </h4>
      <div className={styles.cardList} data-testid={testid}>
        {rules.map((r, i) => (
          <div className={styles.entryCard} key={r.id}>
            <div className={styles.ruleHead}>
              <span className={`${styles.ruleBadge} ${r.conditions.length === 0 ? styles.ruleBadgeDefault : ""}`}>
                {r.conditions.length === 0 ? "OTHERWISE" : i === 0 ? "IF" : "ELSE IF"}
              </span>
              <span className={styles.rulePreview}>{describeHedgeRule(r)}</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
