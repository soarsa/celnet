/**
 * HedgingWorkspace — the trader-facing AUTO-HEDGE RULES / CONFIG surface
 * (docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md §5, §8). Authoring-only —
 * six tabs, which together answer WHEN / WHAT KIND / WITH WHAT / WHETHER TO FIRE:
 *
 *   • Exit Policy — the drag-and-drop exit-policy graph editor (the SAME risk-routing
 *     decision-graph building blocks, with an {@link ExitActionEditor} leaf instead of
 *     a book target) + a live "what would fire" trace.
 *   • Thresholds — the per-scope warehouse-threshold ("the 100") config.
 *   • Vehicles — the firm's hedge-vehicle registry ({@link HedgeVehicleRegistry}): what
 *     each class of risk is hedged WITH, and the DV01-per-unit that sizes it. A corporate
 *     bond is not hedged with itself; this is where its benchmark future comes from.
 *   • Exit mode — the per-scope AUTO-vs-SUGGEST bindings ({@link ExitModeConfig}). SUGGEST
 *     sizes the hedge and raises a STANDING row on the risk panel instead of trading; it
 *     is deliberately not a confirmation dialog.
 *   • LP Panels — the standing hedge LP-panel roster ({@link LpPanelConfig}).
 *   • Execution mode — the engine kill-switch / advisory-vs-live execution-mode /
 *     rate-guard controls ({@link HedgeConfigControl}).
 *
 * The LIVE hedge MONITOR (advisory intents + fired provenance + per-book RAG) is NOT
 * here — it lives in the Risk surface as the "Hedge flows" tab ({@link HedgeMonitor},
 * self-fetching), so authoring the rules and watching them fire are separated. This
 * is the "Hedging Rules" top-level domain tab.
 *
 * The wire model is the shipped {@link HedgeGraph}: the ordered rules table compiles to
 * a deterministic first-match-wins graph ({@link compileRulesToHedgeGraph}) and loads
 * back via {@link decompileHedgeGraphToRules}. Authoring gates on the `hedge`
 * capability × fixed income; everyone else sees the surface read-only.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useApp } from "../../app/AppContext";
import { useHedgeSeed } from "../../app/HedgeSeedContext";
import {
  hedgeRuleFromPricingGroupSeed,
  hedgeRuleFromSeed,
  hedgeSeedGapNote,
  hedgeSeedHint,
  pricingGroupSeedGapNote,
  pricingGroupSeedHint,
} from "../../lib/hedgeSeed";
import type {
  HedgeConfig,
  HedgeGraph,
  HedgePolicyScopeKind,
  RiskBook,
  WarehouseThreshold,
} from "../../data/contract";
import { HelpButton } from "../../components/HelpButton";
import {
  compileRulesToHedgeGraph,
  decompileHedgeGraphToRules,
  detectHedgeRuleConflicts,
  newDefaultHedgeRule,
  newHedgeRuleId,
  type HedgeRule,
  type HedgeRuleConflict,
} from "../../lib/hedgeRules";
import { defaultExitAction } from "../../lib/hedgeExit";
import { validateHedgeGraph } from "../../lib/hedgeTrace";
import { ExitPolicyWizard } from "./ExitPolicyWizard";
import { HedgeConfigControl } from "./HedgeConfigControl";
import { HedgeConflictPanel } from "./HedgeConflictPanel";
import { HedgeRuleEditor } from "./HedgeRuleEditor";
import { HedgeRulesTable } from "./HedgeRulesTable";
import { HedgeTracePanel } from "./HedgeTracePanel";
import { ExitModeConfig } from "./ExitModeConfig";
import { HedgeVehicleRegistry } from "./HedgeVehicleRegistry";
import { useReferenceData } from "../../hooks/useReferenceData";
import { LpPanelConfig } from "./LpPanelConfig";
import { SetupWizard } from "./SetupWizard/SetupWizard";
import { ThresholdConfig } from "./ThresholdConfig";
import styles from "./HedgingWorkspace.module.css";
import type { HedgeExitModeBinding, HedgeLpPanel, HedgeVehicleRule } from "../../data/contract";

type Tab = "policy" | "thresholds" | "vehicles" | "exit-mode" | "lp-panels" | "execution";
type Mode = { kind: "list" } | { kind: "editor"; index: number | null; draft: HedgeRule };
type SaveState =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "confirm"; message: string }
  | { kind: "ok"; message: string }
  | { kind: "error"; message: string };

const EMPTY_GRAPH: HedgeGraph = { entry: 0, nodes: [] };
/** Advisory aggregation instruments a CROSS_INTERNAL can target. */
/**
 * The aggregation instruments a CROSS_INTERNAL exit may target, drawn from the live
 * reference-data registry rather than a hardcoded list. A curated set of made-up ids
 * looked like configuration but named nothing the platform could cross against.
 */
function useAggregationInstrumentOptions(
  app: ReturnType<typeof useApp>,
): readonly string[] {
  const refData = useReferenceData(app.transport, app.auth.user != null);
  return useMemo(
    () =>
      refData.instruments
        .map((d) => d.instrumentId)
        .filter((id) => id.length > 0)
        .sort((a, b) => a.localeCompare(b)),
    [refData.instruments],
  );
}
/** Advisory LP ids an RFQ_OUT can fan to. */
const LP_OPTIONS: readonly string[] = ["LP-1", "LP-2", "LP-3", "LP-4"];

/** Move `from`→`to` in a new array (row re-prioritise). */
function reorder<T>(items: readonly T[], from: number, to: number): T[] {
  const next = [...items];
  const moved = next.splice(from, 1)[0];
  if (moved === undefined) return next;
  next.splice(to, 0, moved);
  return next;
}

export function HedgingWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;
  // Authoring a hedge policy gates on the NARROW `hedge` capability × FI — separable
  // from RUNNING inside it (booking). The rail already hides the surface without it;
  // this makes every edit affordance read-only for a non-holder who reaches it.
  const canEdit = auth.can("hedge", "fixed_income");
  const readOnly = !canEdit;

  const [tab, setTab] = useState<Tab>("policy");
  const [wizardOpen, setWizardOpen] = useState(false);

  if (!signedIn) {
    return (
      <div className={styles.wrap}>
        <p className={styles.centerEmpty}>Sign in to view the auto-hedge policy.</p>
      </div>
    );
  }

  return (
    <div className={styles.wrap}>
      <header className={styles.head}>
        <div className={styles.headMain}>
          <h1 className={styles.title}>Hedging Rules</h1>
          <p className={styles.note}>
            Internalise warehoused risk up to the threshold, then hedge the overflow — via a
            trader-composed exit policy. Scope rules by <strong>book</strong> or by{" "}
            <strong>counterparty</strong> (e.g. <em>Counterparty = CITADEL → Submit market order</em>{" "}
            back-to-backs all of that counterparty&rsquo;s flow), among the other risk-state fields.{" "}
            {readOnly ? "Read-only view." : "hedge · FI edit."}
          </p>
          <button
            type="button"
            className={styles.guidedSetupBtn}
            data-testid="open-guided-setup"
            onClick={() => setWizardOpen(true)}
          >
            <span aria-hidden="true">🪄</span> Guided setup
            <span className={styles.guidedSetupSub}>portfolios · routing · hedging in one flow</span>
          </button>
        </div>
        <nav className={styles.tabs} aria-label="Hedging views">
          <button
            type="button"
            className={tab === "policy" ? styles.tabActive : styles.tab}
            aria-pressed={tab === "policy"}
            data-testid="tab-policy"
            onClick={() => setTab("policy")}
          >
            Exit Policy
          </button>
          <button
            type="button"
            className={tab === "thresholds" ? styles.tabActive : styles.tab}
            aria-pressed={tab === "thresholds"}
            data-testid="tab-thresholds"
            onClick={() => setTab("thresholds")}
          >
            Thresholds
          </button>
          <button
            type="button"
            className={tab === "vehicles" ? styles.tabActive : styles.tab}
            aria-pressed={tab === "vehicles"}
            data-testid="tab-vehicles"
            onClick={() => setTab("vehicles")}
          >
            Vehicles
          </button>
          <button
            type="button"
            className={tab === "exit-mode" ? styles.tabActive : styles.tab}
            aria-pressed={tab === "exit-mode"}
            data-testid="tab-exit-mode"
            onClick={() => setTab("exit-mode")}
          >
            Exit mode
          </button>
          <button
            type="button"
            className={tab === "lp-panels" ? styles.tabActive : styles.tab}
            aria-pressed={tab === "lp-panels"}
            data-testid="tab-lp-panels"
            onClick={() => setTab("lp-panels")}
          >
            LP Panels
          </button>
          <button
            type="button"
            className={tab === "execution" ? styles.tabActive : styles.tab}
            aria-pressed={tab === "execution"}
            data-testid="tab-execution"
            onClick={() => setTab("execution")}
          >
            Execution mode
          </button>
        </nav>
      </header>

      {tab === "policy" && <PolicyTab app={app} readOnly={readOnly} />}
      {tab === "thresholds" && <ThresholdsTab app={app} readOnly={readOnly} />}
      {tab === "vehicles" && <VehiclesTab app={app} readOnly={readOnly} />}
      {tab === "exit-mode" && <ExitModeTab app={app} readOnly={readOnly} />}
      {tab === "lp-panels" && <LpPanelsTab app={app} readOnly={readOnly} />}
      {tab === "execution" && <ExecutionTab app={app} readOnly={readOnly} />}

      {wizardOpen && <SetupWizard onClose={() => setWizardOpen(false)} />}
    </div>
  );
}

// --- Exit Policy tab --------------------------------------------------------

function PolicyTab({
  app,
  readOnly,
}: {
  app: ReturnType<typeof useApp>;
  readOnly: boolean;
}): React.ReactElement {
  const instrumentOptions = useAggregationInstrumentOptions(app);
  const [rules, setRules] = useState<HedgeRule[]>([]);
  const [baseline, setBaseline] = useState<string>("[]");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>({ kind: "list" });
  const [saveState, setSaveState] = useState<SaveState>({ kind: "idle" });
  // The policy SCOPE: the Firm-wide singleton (default), or a per-Book / per-Bucket
  // (portfolio subtree-root) override. `scopeId` is empty for Firm; the book/bucket id
  // otherwise. `hasScopedPolicy` tracks whether an override already exists for the scope
  // (so we can offer Remove and explain the fall-back to Firm).
  const [scopeKind, setScopeKind] = useState<HedgePolicyScopeKind>("firm");
  const [scopeId, setScopeId] = useState<string>("");
  const [books, setBooks] = useState<RiskBook[]>([]);
  // The hedge-vehicle registry backs the leaf editor's vehicle picker: a NAMED vehicle
  // must be a registry row (that row carries its DV01-per-unit), so the picker offers
  // exactly these and nothing else rather than free text the server would reject.
  const [vehicles, setVehicles] = useState<HedgeVehicleRule[]>([]);
  const [hasScopedPolicy, setHasScopedPolicy] = useState(false);
  const [ruleWizardOpen, setRuleWizardOpen] = useState(false);
  const scopeReady = scopeKind === "firm" || scopeId !== "";
  // A hedge-rule seed hand-off from a Deals-blotter row ("Change hedging strategy") OR a
  // pricing group ("Create hedging rule"): the SOURCE-AGNOSTIC hint + gap note + the id of
  // the draft rule it seeded, so the editor shows the hint ONLY for that draft (a
  // subsequent hand-built rule has a different id ⇒ no stale hint).
  const { pending: pendingSeed, consumeHedgeSeed } = useHedgeSeed();
  const [seed, setSeed] = useState<{ hint: string; gapNote: string; ruleId: string } | null>(null);
  const appliedSeedNonce = useRef(0);

  // The risk-book roster backs the Book / Bucket scope picker (a Bucket is a
  // subtree-root book whose policy reads the whole portfolio aggregate).
  useEffect(() => {
    let cancelled = false;
    void app.transport
      .listRiskBooks()
      .then((b) => !cancelled && setBooks(b))
      .catch(() => undefined);
    void app.transport
      .getHedgeConfig()
      .then((c) => !cancelled && setVehicles(c.vehicles))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [app.transport]);

  useEffect(() => {
    let cancelled = false;
    if (!scopeReady) {
      // A Book/Bucket scope with no book chosen yet: nothing to load.
      setRules([]);
      setBaseline("[]");
      setHasScopedPolicy(false);
      setMode({ kind: "list" });
      return;
    }
    void (async () => {
      try {
        const g = await app.transport.getHedgePolicyGraph(scopeKind, scopeId);
        if (cancelled) return;
        setHasScopedPolicy(scopeKind !== "firm" && g !== null && g.nodes.length > 0);
        const loaded = decompileHedgeGraphToRules(g ?? EMPTY_GRAPH);
        // A scope with no policy seeds a single warehouse catch-all so the table is
        // never empty / invalid on first open (Save then creates the override).
        const seeded = loaded.length > 0 ? loaded : [newDefaultHedgeRule()];
        setRules(seeded);
        setBaseline(JSON.stringify(seeded));
        setLoadError(null);
        setSaveState({ kind: "idle" });
        setMode({ kind: "list" });
      } catch (e: unknown) {
        if (!cancelled) setLoadError(e instanceof Error ? e.message : "failed to load the hedge policy");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [app.transport, scopeKind, scopeId, scopeReady]);

  // Consume a pending seed exactly once (de-duped by nonce): open a NEW draft rule
  // pre-scoped from the source (a deal's flow, or a pricing group's single desk), and
  // remember the source-agnostic hint + gap note for that draft so the editor shows them.
  // The trader picks the exit action + Saves — nothing auto-saves; the seed is cleared so
  // re-entering the tab does not re-seed.
  useEffect(() => {
    if (!pendingSeed || pendingSeed.nonce === appliedSeedNonce.current) return;
    appliedSeedNonce.current = pendingSeed.nonce;
    const source = pendingSeed.source;
    const draft =
      source.kind === "deal"
        ? hedgeRuleFromSeed(source.deal)
        : hedgeRuleFromPricingGroupSeed(source.group);
    const hint =
      source.kind === "deal" ? hedgeSeedHint(source.deal) : pricingGroupSeedHint(source.group);
    const gapNote = source.kind === "deal" ? hedgeSeedGapNote() : pricingGroupSeedGapNote();
    setSeed({ hint, gapNote, ruleId: draft.id });
    setMode({ kind: "editor", index: null, draft });
    setSaveState({ kind: "idle" });
    consumeHedgeSeed();
  }, [pendingSeed, consumeHedgeSeed]);

  const conflicts = useMemo(() => detectHedgeRuleConflicts(rules), [rules]);
  const conflictsByRule = useMemo(() => {
    const m = new Map<string, HedgeRuleConflict[]>();
    for (const c of conflicts) {
      const list = m.get(c.ruleId);
      if (list) list.push(c);
      else m.set(c.ruleId, [c]);
    }
    return m;
  }, [conflicts]);
  const errorCount = conflicts.filter((c) => c.severity === "error").length;

  const compiled = useMemo(
    () => compileRulesToHedgeGraph(rules.filter((r) => r.enabled)),
    [rules],
  );
  const graphIssues = useMemo(() => validateHedgeGraph(compiled), [compiled]);
  const dirty = JSON.stringify(rules) !== baseline;
  // A genuine graph-validation defect (unset value / dangling target / cycle) CANNOT
  // compile a valid policy — it hard-blocks Save. Rule-table CONFLICTS (duplicate /
  // shadowed / missing-default) are logic warnings a trader may intend as an override,
  // so they don't block: Save requires an explicit confirm rather than being silent.
  const structuralBlock = graphIssues.length > 0;
  const hasConflicts = errorCount > 0;

  const applyRules = useCallback((next: HedgeRule[]): void => {
    setRules(next);
    setSaveState({ kind: "idle" });
  }, []);

  const onCreate = useCallback((): void => {
    setSeed(null); // a hand-built rule carries no seed hint
    setMode({
      kind: "editor",
      index: null,
      draft: { id: newHedgeRuleId(), conditions: [], action: defaultExitAction("warehouse"), enabled: true },
    });
  }, []);

  // Insert a wizard-generated rule set: REPLACE the current scope's draft rules with the
  // complete, valid scenario policy, then drop back to the list so the trader reviews it
  // in the live validity/conflict panel and Saves through the normal path (nothing auto-saves).
  const onInsertGeneratedRules = useCallback((generated: HedgeRule[]): void => {
    setSeed(null);
    setRules(generated);
    setMode({ kind: "list" });
    setSaveState({ kind: "idle" });
  }, []);

  const onEditRow = useCallback(
    (index: number): void => {
      const draft = rules[index];
      if (draft) {
        setSeed(null); // editing an existing rule carries no seed hint
        setMode({ kind: "editor", index, draft });
      }
    },
    [rules],
  );

  const onEditorSave = useCallback(
    (rule: HedgeRule): void => {
      setRules((cur) => {
        if (mode.kind !== "editor") return cur;
        if (mode.index !== null) return cur.map((r, i) => (i === mode.index ? rule : r));
        // A NEW specific rule slots ABOVE the trailing catch-all (first-match-wins).
        const defaultIdx = cur.findIndex((r) => r.conditions.length === 0);
        if (rule.conditions.length > 0 && defaultIdx >= 0) {
          const next = [...cur];
          next.splice(defaultIdx, 0, rule);
          return next;
        }
        return [...cur, rule];
      });
      setSaveState({ kind: "idle" });
      setMode({ kind: "list" });
    },
    [mode],
  );

  const saveGraph = useCallback(async (): Promise<void> => {
    if (structuralBlock) {
      setSaveState({
        kind: "error",
        message: "Resolve the structural graph issue(s) — the policy can't compile until they're fixed.",
      });
      return;
    }
    // First click WITH conflicts arms a confirm rather than saving silently; the second
    // click (saveState already "confirm") lets the trader override intentionally.
    if (hasConflicts && saveState.kind !== "confirm") {
      setSaveState({
        kind: "confirm",
        message: `This policy has ${errorCount} conflict${errorCount === 1 ? "" : "s"} (see the panel above). Save anyway?`,
      });
      return;
    }
    setSaveState({ kind: "saving" });
    try {
      await app.transport.updateHedgePolicyGraph(
        compileRulesToHedgeGraph(rules.filter((r) => r.enabled)),
        scopeKind,
        scopeId,
      );
      setBaseline(JSON.stringify(rules));
      setHasScopedPolicy(scopeKind !== "firm");
      setSaveState({
        kind: "ok",
        message: scopeKind === "firm" ? "Firm hedge policy saved." : "Scoped hedge policy saved.",
      });
    } catch (e: unknown) {
      setSaveState({ kind: "error", message: e instanceof Error ? e.message : "failed to save the policy" });
    }
  }, [structuralBlock, hasConflicts, errorCount, saveState.kind, rules, app.transport, scopeKind, scopeId]);

  // Remove a Book/Bucket override: saving an EMPTY policy for the scope deletes it,
  // and the scope falls back to the Firm policy. Firm has no Remove (it is the
  // singleton). Resets the table to the safe warehouse catch-all afterwards.
  const removeScopedPolicy = useCallback(async (): Promise<void> => {
    if (scopeKind === "firm" || scopeId === "") return;
    setSaveState({ kind: "saving" });
    try {
      await app.transport.updateHedgePolicyGraph(EMPTY_GRAPH, scopeKind, scopeId);
      const seeded = [newDefaultHedgeRule()];
      setRules(seeded);
      setBaseline(JSON.stringify(seeded));
      setHasScopedPolicy(false);
      setMode({ kind: "list" });
      setSaveState({ kind: "ok", message: "Override removed — this scope falls back to the Firm policy." });
    } catch (e: unknown) {
      setSaveState({ kind: "error", message: e instanceof Error ? e.message : "failed to remove the policy" });
    }
  }, [app.transport, scopeKind, scopeId]);

  const resetRules = useCallback((): void => {
    setRules(JSON.parse(baseline) as HedgeRule[]);
    setMode({ kind: "list" });
    setSaveState({ kind: "idle" });
  }, [baseline]);

  if (mode.kind === "editor") {
    // The seed hint shows ONLY for the exact draft the seed created (id match) — a
    // hand-built rule opened afterwards has a different id and no banner.
    const activeSeed = seed !== null && mode.draft.id === seed.ruleId ? seed : null;
    return (
      <HedgeRuleEditor
        draft={mode.draft}
        isNew={mode.index === null}
        readOnly={readOnly}
        instrumentOptions={instrumentOptions}
        lpOptions={LP_OPTIONS}
        vehicles={vehicles}
        onSave={onEditorSave}
        onCancel={() => setMode({ kind: "list" })}
        seedHint={activeSeed?.hint ?? null}
        seedGapNote={activeSeed?.gapNote ?? ""}
      />
    );
  }

  const scopeNoun = scopeKind === "bucket" ? "portfolio" : "book";
  const scopeNote =
    scopeKind === "firm"
      ? "The firm-wide default policy — evaluated for every book that has no override."
      : scopeKind === "bucket"
        ? "A Bucket policy reads the WHOLE portfolio's rolled-up aggregate (net notional / DV01 / …) — enabling e.g. “if PORTFOLIO notional > n → market order / clear risk”. An empty policy falls back to the Firm policy."
        : "A Book policy reads that one book's own net risk. An empty policy falls back to the Firm policy.";

  return (
    <div className={styles.policyTab}>
      <div className={styles.scopeBar} data-testid="hedge-scope-bar">
        <label className={styles.formField}>
          <span className={styles.fieldLabel}>Policy scope</span>
          <select
            className={styles.input}
            value={scopeKind}
            disabled={readOnly}
            data-testid="hedge-scope-kind"
            onChange={(e) => {
              setScopeKind(e.target.value as HedgePolicyScopeKind);
              setScopeId("");
            }}
          >
            <option value="firm">Firm (default)</option>
            <option value="book">Book</option>
            <option value="bucket">Bucket (portfolio)</option>
          </select>
        </label>
        {scopeKind !== "firm" && (
          <label className={styles.formField}>
            <span className={styles.fieldLabel}>
              {scopeKind === "bucket" ? "Portfolio (subtree root)" : "Risk book"}
            </span>
            <select
              className={styles.input}
              value={scopeId}
              disabled={readOnly}
              data-testid="hedge-scope-id"
              onChange={(e) => setScopeId(e.target.value)}
            >
              <option value="">Select a {scopeNoun}…</option>
              {books.map((b) => (
                <option key={b.id} value={b.id}>
                  {b.name}
                </option>
              ))}
            </select>
          </label>
        )}
        <HelpButton helpId="concept.hedge-policy-scope" subject="the hedge policy scope" />
      </div>
      <p className={styles.note} data-testid="hedge-scope-note">
        {scopeNote}
      </p>

      {!scopeReady ? (
        <p className={styles.centerEmpty} data-testid="hedge-scope-prompt">
          Select a {scopeNoun} to view or author its hedge policy.
        </p>
      ) : (
        <>
          <div className={styles.policyActions}>
            {!readOnly && (
              <>
                <button type="button" className={styles.saveBtn} onClick={onCreate} data-testid="hedge-create-rule">
                  + Create hedge rule
                </button>
                <button
                  type="button"
                  className={styles.ghostBtn}
                  onClick={() => setRuleWizardOpen(true)}
                  data-testid="open-rule-wizard"
                >
                  🪄 Rule wizard
                </button>
                <button type="button" className={styles.ghostBtn} onClick={resetRules} disabled={!dirty}>
                  Reset
                </button>
                <button
                  type="button"
                  className={saveState.kind === "confirm" ? styles.dangerBtn : styles.saveBtn}
                  onClick={() => void saveGraph()}
                  disabled={saveState.kind === "saving" || structuralBlock || !dirty}
                  data-testid="hedge-save-policy"
                >
                  {saveState.kind === "saving"
                    ? "Saving…"
                    : saveState.kind === "confirm"
                      ? `Save anyway (${errorCount} conflict${errorCount === 1 ? "" : "s"})`
                      : "Save policy"}
                </button>
                {scopeKind !== "firm" && hasScopedPolicy && (
                  <button
                    type="button"
                    className={styles.ghostBtn}
                    onClick={() => void removeScopedPolicy()}
                    disabled={saveState.kind === "saving"}
                    data-testid="hedge-remove-scoped-policy"
                  >
                    Remove override
                  </button>
                )}
              </>
            )}
          </div>

          {loadError && (
            <p className={styles.errorText} role="alert">
              {loadError}
            </p>
          )}

          <div className={styles.statusRow}>
            {structuralBlock ? (
              <span className={styles.statusBad} data-testid="hedge-validation-status">
                ⚠ {graphIssues.length} structural issue{graphIssues.length === 1 ? "" : "s"} — can&rsquo;t compile a valid policy
              </span>
            ) : hasConflicts ? (
              <span className={styles.statusWarn} data-testid="hedge-validation-status">
                ⚠ {errorCount} conflict{errorCount === 1 ? "" : "s"} — review below (Save asks to confirm)
              </span>
            ) : (
              <span className={styles.statusOk} data-testid="hedge-validation-status">
                ✓ Valid — {rules.length} rule{rules.length === 1 ? "" : "s"}
              </span>
            )}
            {saveState.kind === "confirm" && (
              <span className={styles.statusWarn} role="alert" data-testid="hedge-confirm-note">
                {saveState.message}
              </span>
            )}
            {saveState.kind === "ok" && <span className={styles.statusOk}>{saveState.message}</span>}
            {saveState.kind === "error" && (
              <span className={styles.statusBad} role="alert">
                {saveState.message}
              </span>
            )}
          </div>

          <HedgeConflictPanel rules={rules} conflicts={conflicts} graphIssues={graphIssues} />

          <HedgeRulesTable
            rules={rules}
            conflictsByRule={conflictsByRule}
            readOnly={readOnly}
            onEdit={onEditRow}
            onDelete={(i) => applyRules(rules.filter((_, j) => j !== i))}
            onToggle={(i) => applyRules(rules.map((r, j) => (j === i ? { ...r, enabled: !r.enabled } : r)))}
            onReorder={(from, to) => applyRules(reorder(rules, from, to))}
          />

          <HedgeTracePanel graph={compiled} />

          {ruleWizardOpen && (
            <ExitPolicyWizard
              scopeBookId={scopeKind === "firm" ? undefined : scopeId}
              readOnly={readOnly}
              onInsert={onInsertGeneratedRules}
              onClose={() => setRuleWizardOpen(false)}
            />
          )}
        </>
      )}
    </div>
  );
}

// --- Thresholds tab ---------------------------------------------------------

function ThresholdsTab({
  app,
  readOnly,
}: {
  app: ReturnType<typeof useApp>;
  readOnly: boolean;
}): React.ReactElement {
  const [thresholds, setThresholds] = useState<WarehouseThreshold[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void app.transport
      .listHedgeThresholds()
      .then((t) => !cancelled && setThresholds(t))
      .catch((e: unknown) => !cancelled && setError(e instanceof Error ? e.message : "load failed"));
    return () => {
      cancelled = true;
    };
  }, [app.transport]);

  const onUpsert = useCallback(
    async (threshold: WarehouseThreshold): Promise<void> => {
      setBusy(true);
      try {
        setThresholds(await app.transport.updateHedgeThreshold(threshold));
        setError(null);
      } catch (e: unknown) {
        setError(e instanceof Error ? e.message : "save failed");
      } finally {
        setBusy(false);
      }
    },
    [app.transport],
  );

  const onDelete = useCallback(
    async (threshold: WarehouseThreshold): Promise<void> => {
      await onUpsert({ ...threshold, cap: 0 });
    },
    [onUpsert],
  );

  return (
    <div className={styles.singleTab}>
      {error && (
        <p className={styles.errorText} role="alert">
          {error}
        </p>
      )}
      <ThresholdConfig
        thresholds={thresholds}
        readOnly={readOnly}
        busy={busy}
        onUpsert={onUpsert}
        onDelete={onDelete}
      />
    </div>
  );
}

// --- the shared HedgeConfig-editing tab shell -------------------------------

/**
 * Load the engine {@link HedgeConfig} once and commit patches to it OPTIMISTICALLY: the
 * edit shows immediately, and a server rejection (an unknown LP id, an unregistered hedge
 * vehicle, an empty effective set) reverts the store and surfaces the message — the store
 * is left UNCHANGED on rejection, so the reverted value is the true one.
 *
 * Three tabs (LP Panels, Vehicles, Exit mode) all edit disjoint slices of the SAME config
 * message through the SAME `get_hedge_config` / `set_hedge_config` pair, so they share this
 * shell rather than each re-deriving the load / optimistic-commit / revert dance.
 */
function useHedgeConfigTab(
  app: ReturnType<typeof useApp>,
  rejectionNoun: string,
): {
  config: HedgeConfig | null;
  busy: boolean;
  loadError: string | null;
  saveError: string | null;
  commit: (patch: Partial<HedgeConfig>) => void;
} {
  const [config, setConfig] = useState<HedgeConfig | null>(null);
  const [busy, setBusy] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void app.transport
      .getHedgeConfig()
      .then((c) => !cancelled && setConfig(c))
      .catch((e: unknown) => !cancelled && setLoadError(e instanceof Error ? e.message : "load failed"));
    return () => {
      cancelled = true;
    };
  }, [app.transport]);

  const commit = useCallback(
    (patch: Partial<HedgeConfig>): void => {
      if (config === null) return;
      const prev = config;
      const next: HedgeConfig = { ...config, ...patch };
      setBusy(true);
      setConfig(next);
      void app.transport
        .setHedgeConfig(next)
        .then((c) => {
          setConfig(c);
          setSaveError(null);
        })
        .catch((e: unknown) => {
          setConfig(prev);
          setSaveError(e instanceof Error ? e.message : `the server rejected the ${rejectionNoun}`);
        })
        .finally(() => setBusy(false));
    },
    [app.transport, config, rejectionNoun],
  );

  return { config, busy, loadError, saveError, commit };
}

// --- LP Panels tab ----------------------------------------------------------

function LpPanelsTab({
  app,
  readOnly,
}: {
  app: ReturnType<typeof useApp>;
  readOnly: boolean;
}): React.ReactElement {
  const { config, busy, loadError, saveError, commit } = useHedgeConfigTab(app, "LP panel");

  return (
    <div className={styles.singleTab}>
      {loadError !== null && (
        <p className={styles.errorText} role="alert">
          {loadError}
        </p>
      )}
      {config !== null && (
        <LpPanelConfig
          panels={config.lpPanels}
          readOnly={readOnly}
          busy={busy}
          saveError={saveError}
          onCommit={(lpPanels: HedgeLpPanel[]) => commit({ lpPanels })}
        />
      )}
    </div>
  );
}

// --- Vehicles tab -----------------------------------------------------------

/**
 * VehiclesTab — the firm's hedge-vehicle registry ({@link HedgeVehicleRegistry}): what
 * each class of risk is hedged WITH, and the DV01-per-unit that turns a target DV01 into
 * tradeable units. A rule's named vehicle must be a row here.
 */
function VehiclesTab({
  app,
  readOnly,
}: {
  app: ReturnType<typeof useApp>;
  readOnly: boolean;
}): React.ReactElement {
  const { config, busy, loadError, saveError, commit } = useHedgeConfigTab(app, "hedge vehicle");
  // The instrument registry backing the vehicle pickers, so a trader CHOOSES a real
  // contract (and inherits its published DV01 per contract) instead of typing a code
  // and a number from memory. Loaded here rather than inside the registry component so
  // the component stays a pure view over the data it is handed.
  const refData = useReferenceData(app.transport, app.auth.user != null);

  return (
    <div className={styles.singleTab}>
      {loadError !== null && (
        <p className={styles.errorText} role="alert">
          {loadError}
        </p>
      )}
      {refData.error !== null && (
        <p className={styles.errorText} role="alert">
          Instrument reference data failed to load ({refData.error}) — the vehicle
          pickers will be empty; any already-configured instrument still shows.
        </p>
      )}
      {config !== null && (
        <HedgeVehicleRegistry
          vehicles={config.vehicles}
          instruments={refData.instruments}
          readOnly={readOnly}
          busy={busy}
          saveError={saveError}
          onCommit={(vehicles: HedgeVehicleRule[]) => commit({ vehicles })}
        />
      )}
    </div>
  );
}

// --- Exit mode tab ----------------------------------------------------------

/**
 * ExitModeTab — the per-scope AUTO-vs-SUGGEST bindings ({@link ExitModeConfig}). SUGGEST
 * means the sized hedge lands as a STANDING row on Fixed Income → Book → Hedge flows, never as a popup.
 */
function ExitModeTab({
  app,
  readOnly,
}: {
  app: ReturnType<typeof useApp>;
  readOnly: boolean;
}): React.ReactElement {
  const { config, busy, loadError, saveError, commit } = useHedgeConfigTab(app, "exit-mode binding");

  return (
    <div className={styles.singleTab}>
      {loadError !== null && (
        <p className={styles.errorText} role="alert">
          {loadError}
        </p>
      )}
      {config !== null && (
        <ExitModeConfig
          bindings={config.exitModes}
          readOnly={readOnly}
          busy={busy}
          saveError={saveError}
          onCommit={(exitModes: HedgeExitModeBinding[]) => commit({ exitModes })}
        />
      )}
    </div>
  );
}

// --- Execution mode tab -----------------------------------------------------

/**
 * ExecutionTab — the engine EXECUTION-MODE config (the top half of what used to be the
 * "Monitor" tab): the kill-switch, the Advisory / LP-panel / Composite / LP-panel→
 * Composite execution mode, MAX CLIP / MAX HEDGES / DAILY external CAP / COMPOSITE
 * SPREAD guard-rails ({@link HedgeConfigControl}). The LIVE flow monitor moved to the
 * Risk surface's "Hedge flows" tab, so this tab is pure config — matching the other
 * Hedging Rules tabs.
 */
function ExecutionTab({
  app,
  readOnly,
}: {
  app: ReturnType<typeof useApp>;
  readOnly: boolean;
}): React.ReactElement {
  const [config, setConfig] = useState<HedgeConfig | null>(null);
  const [busy, setBusy] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Load config once.
  useEffect(() => {
    let cancelled = false;
    void app.transport
      .getHedgeConfig()
      .then((c) => !cancelled && setConfig(c))
      .catch((e: unknown) => !cancelled && setLoadError(e instanceof Error ? e.message : "load failed"));
    return () => {
      cancelled = true;
    };
  }, [app.transport]);

  const onConfigChange = useCallback(
    (next: HedgeConfig): void => {
      setConfig(next);
      setBusy(true);
      void app.transport
        .setHedgeConfig(next)
        .then((c) => setConfig(c))
        .catch(() => undefined)
        .finally(() => setBusy(false));
    },
    [app.transport],
  );

  return (
    <div className={styles.singleTab}>
      <div className={styles.scopeBar}>
        <p className={styles.note}>
          How the engine ACTS on the policy: the kill-switch, the execution mode (Advisory dry-run vs
          LP&nbsp;panel / Composite / LP&nbsp;panel&nbsp;→&nbsp;Composite), and the max-clip / max-hedges /
          daily-cap guard-rails. Run Advisory while you calibrate, then arm a live mode. Watch the fired
          hedges under <strong>Fixed Income → Book → Hedge flows</strong>.{" "}
          {readOnly ? "Read-only view." : "hedge · FI edit."}
        </p>
        <HelpButton helpId="concept.hedge-execution-mode" subject="the hedge execution mode" />
      </div>
      {loadError !== null && (
        <p className={styles.errorText} role="alert">
          {loadError}
        </p>
      )}
      {config !== null && (
        <HedgeConfigControl config={config} readOnly={readOnly} busy={busy} onChange={onConfigChange} />
      )}
    </div>
  );
}
