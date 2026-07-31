/**
 * HedgingWorkspace — the trader-facing AUTO-HEDGE surface
 * (docs/AUTO-HEDGING-AND-INTERNALISATION-REQUIREMENTS.md §5, §8). Three tabs:
 *
 *   • Exit Policy — the drag-and-drop exit-policy graph editor (the SAME risk-routing
 *     decision-graph building blocks, with an {@link ExitActionEditor} leaf instead of
 *     a book target) + a live "what would fire" trace.
 *   • Thresholds — the per-scope warehouse-threshold ("the 100") config.
 *   • Monitor — the live advisory intents + fired provenance + per-book RAG, and the
 *     engine kill-switch / advisory-only / rate-guard controls.
 *
 * The wire model is the shipped {@link HedgeGraph}: the ordered rules table compiles to
 * a deterministic first-match-wins graph ({@link compileRulesToHedgeGraph}) and loads
 * back via {@link decompileHedgeGraphToRules}. Authoring gates on the `hedge`
 * capability × fixed income; everyone else sees the surface read-only.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../app/AppContext";
import type {
  HedgeConfig,
  HedgeGraph,
  HedgeIntent,
  HedgeProvenance,
  WarehouseThreshold,
} from "../../data/contract";
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
import { HedgeConfigControl } from "./HedgeConfigControl";
import { HedgeMonitor } from "./HedgeMonitor";
import { HedgeRuleEditor } from "./HedgeRuleEditor";
import { HedgeRulesTable } from "./HedgeRulesTable";
import { HedgeTracePanel } from "./HedgeTracePanel";
import { ThresholdConfig } from "./ThresholdConfig";
import styles from "./HedgingWorkspace.module.css";

type Tab = "policy" | "thresholds" | "monitor";
type Mode = { kind: "list" } | { kind: "editor"; index: number | null; draft: HedgeRule };
type SaveState =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "ok"; message: string }
  | { kind: "error"; message: string };

const EMPTY_GRAPH: HedgeGraph = { entry: 0, nodes: [] };
/** Advisory aggregation instruments a CROSS_INTERNAL can target. */
const INSTRUMENT_OPTIONS: readonly string[] = ["AGG-OIS", "AGG-US10Y", "AGG-EURUSD", "AGG-UK5Y"];
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
          <h1 className={styles.title}>Hedging</h1>
          <p className={styles.note}>
            Internalise warehoused risk up to the threshold, then hedge the overflow — via a
            trader-composed exit policy. {readOnly ? "Read-only view." : "hedge · FI edit."}
          </p>
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
            className={tab === "monitor" ? styles.tabActive : styles.tab}
            aria-pressed={tab === "monitor"}
            data-testid="tab-monitor"
            onClick={() => setTab("monitor")}
          >
            Monitor
          </button>
        </nav>
      </header>

      {tab === "policy" && <PolicyTab app={app} readOnly={readOnly} />}
      {tab === "thresholds" && <ThresholdsTab app={app} readOnly={readOnly} />}
      {tab === "monitor" && <MonitorTab app={app} readOnly={readOnly} />}
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
  const [rules, setRules] = useState<HedgeRule[]>([]);
  const [baseline, setBaseline] = useState<string>("[]");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>({ kind: "list" });
  const [saveState, setSaveState] = useState<SaveState>({ kind: "idle" });

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const g = await app.transport.getHedgePolicyGraph();
        if (cancelled) return;
        const loaded = decompileHedgeGraphToRules(g ?? EMPTY_GRAPH);
        // A fresh install with no policy seeds a single warehouse catch-all so the
        // table is never empty / invalid on first open.
        const seeded = loaded.length > 0 ? loaded : [newDefaultHedgeRule()];
        setRules(seeded);
        setBaseline(JSON.stringify(seeded));
        setLoadError(null);
      } catch (e: unknown) {
        if (!cancelled) setLoadError(e instanceof Error ? e.message : "failed to load the hedge policy");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [app.transport]);

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
  const blockSave = errorCount > 0 || graphIssues.length > 0;

  const applyRules = useCallback((next: HedgeRule[]): void => {
    setRules(next);
    setSaveState({ kind: "idle" });
  }, []);

  const onCreate = useCallback((): void => {
    setMode({
      kind: "editor",
      index: null,
      draft: { id: newHedgeRuleId(), conditions: [], action: defaultExitAction("warehouse"), enabled: true },
    });
  }, []);

  const onEditRow = useCallback(
    (index: number): void => {
      const draft = rules[index];
      if (draft) setMode({ kind: "editor", index, draft });
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
    if (blockSave) {
      setSaveState({ kind: "error", message: "Resolve the highlighted conflicts before saving." });
      return;
    }
    setSaveState({ kind: "saving" });
    try {
      await app.transport.updateHedgePolicyGraph(compileRulesToHedgeGraph(rules.filter((r) => r.enabled)));
      setBaseline(JSON.stringify(rules));
      setSaveState({ kind: "ok", message: "Hedge policy saved." });
    } catch (e: unknown) {
      setSaveState({ kind: "error", message: e instanceof Error ? e.message : "failed to save the policy" });
    }
  }, [blockSave, rules, app.transport]);

  const resetRules = useCallback((): void => {
    setRules(JSON.parse(baseline) as HedgeRule[]);
    setMode({ kind: "list" });
    setSaveState({ kind: "idle" });
  }, [baseline]);

  if (mode.kind === "editor") {
    return (
      <HedgeRuleEditor
        draft={mode.draft}
        isNew={mode.index === null}
        readOnly={readOnly}
        instrumentOptions={INSTRUMENT_OPTIONS}
        lpOptions={LP_OPTIONS}
        onSave={onEditorSave}
        onCancel={() => setMode({ kind: "list" })}
      />
    );
  }

  return (
    <div className={styles.policyTab}>
      <div className={styles.policyActions}>
        {!readOnly && (
          <>
            <button type="button" className={styles.saveBtn} onClick={onCreate} data-testid="hedge-create-rule">
              + Create hedge rule
            </button>
            <button type="button" className={styles.ghostBtn} onClick={resetRules} disabled={!dirty}>
              Reset
            </button>
            <button
              type="button"
              className={styles.saveBtn}
              onClick={() => void saveGraph()}
              disabled={saveState.kind === "saving" || blockSave || !dirty}
              data-testid="hedge-save-policy"
            >
              {saveState.kind === "saving" ? "Saving…" : "Save policy"}
            </button>
          </>
        )}
      </div>

      {loadError && (
        <p className={styles.errorText} role="alert">
          {loadError}
        </p>
      )}

      <div className={styles.statusRow}>
        {blockSave ? (
          <span className={styles.statusBad} data-testid="hedge-validation-status">
            ⚠ {errorCount + graphIssues.length} issue
            {errorCount + graphIssues.length === 1 ? "" : "s"} to resolve before saving
          </span>
        ) : (
          <span className={styles.statusOk} data-testid="hedge-validation-status">
            ✓ Valid — {rules.length} rule{rules.length === 1 ? "" : "s"}
          </span>
        )}
        {saveState.kind === "ok" && <span className={styles.statusOk}>{saveState.message}</span>}
        {saveState.kind === "error" && (
          <span className={styles.statusBad} role="alert">
            {saveState.message}
          </span>
        )}
      </div>

      {(conflicts.length > 0 || graphIssues.length > 0) && (
        <ul className={styles.conflictList} data-testid="hedge-conflict-details">
          {conflicts.map((c, i) => {
            const idx = rules.findIndex((r) => r.id === c.ruleId);
            return (
              <li key={`c-${i}`} className={c.severity === "error" ? styles.conflictError : styles.conflictWarn}>
                <strong>{c.severity === "error" ? "Error" : "Warning"}</strong>
                {idx >= 0 ? ` · Rule ${idx + 1}` : ""} — {c.message}
              </li>
            );
          })}
          {graphIssues.map((g, i) => (
            <li key={`g-${i}`} className={styles.conflictError}>
              <strong>Error</strong> — {g.message}
            </li>
          ))}
        </ul>
      )}

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

// --- Monitor tab ------------------------------------------------------------

function MonitorTab({
  app,
  readOnly,
}: {
  app: ReturnType<typeof useApp>;
  readOnly: boolean;
}): React.ReactElement {
  const [intents, setIntents] = useState<HedgeIntent[]>([]);
  const [provenance, setProvenance] = useState<HedgeProvenance[]>([]);
  const [config, setConfig] = useState<HedgeConfig | null>(null);
  const [busy, setBusy] = useState(false);

  // Load config once.
  useEffect(() => {
    let cancelled = false;
    void app.transport
      .getHedgeConfig()
      .then((c) => !cancelled && setConfig(c))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [app.transport]);

  // Subscribe to the live advisory-intent stream; refetch provenance on each tick so
  // the audit rows track the fired hedges (the mock appends provenance as it fires).
  useEffect(() => {
    let cancelled = false;
    const refetch = (): void => {
      void app.transport
        .listHedgeProvenance()
        .then((p) => !cancelled && setProvenance(p))
        .catch(() => undefined);
    };
    refetch();
    const dispose = app.transport.streamHedgeIntents((intent) => {
      if (cancelled) return;
      setIntents((cur) => [...cur, intent].slice(-40));
      refetch();
    });
    return () => {
      cancelled = true;
      dispose();
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
    <div className={styles.monitorTab}>
      {config !== null && (
        <HedgeConfigControl config={config} readOnly={readOnly} busy={busy} onChange={onConfigChange} />
      )}
      <HedgeMonitor intents={intents} provenance={provenance} />
    </div>
  );
}
