/**
 * AcceptanceWorkspace — the trader-facing INCOMING-QUOTE-ACCEPTANCE surface
 * (`celnet-acceptance`). The THIRD trader-configurable rule engine, mirroring the
 * risk-routing and hedging builders 1:1: an ordered first-match rule table that
 * compiles to a deterministic decision graph ({@link compileRulesToAcceptanceGraph})
 * and loads back via {@link decompileAcceptanceGraphToRules}, with the shared drag-and-
 * drop rule editor (decision leaves) and a live "what would fire" trace.
 *
 * The policy runs AT ACCEPTANCE — after last-look, before booking — on each incoming
 * client lift: ACCEPT books it, REJECT declines it with the reason, HOLD routes it to
 * the desk inbox for manual accept. Authoring gates on the narrow `manage_acceptance`
 * capability × fixed income; everyone else sees the surface read-only.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../app/AppContext";
import type { AcceptanceGraph } from "../../data/contract";
import { defaultAcceptanceAction } from "../../lib/acceptanceAction";
import {
  compileRulesToAcceptanceGraph,
  decompileAcceptanceGraphToRules,
  detectAcceptanceRuleConflicts,
  newAcceptanceRuleId,
  newDefaultAcceptanceRule,
  type AcceptanceRule,
  type AcceptanceRuleConflict,
} from "../../lib/acceptanceRules";
import { validateAcceptanceGraph } from "../../lib/acceptanceTrace";
import { AcceptanceRuleEditor } from "./AcceptanceRuleEditor";
import { AcceptanceRulesTable } from "./AcceptanceRulesTable";
import { AcceptanceTracePanel } from "./AcceptanceTracePanel";
import styles from "./AcceptanceWorkspace.module.css";

type Mode = { kind: "list" } | { kind: "editor"; index: number | null; draft: AcceptanceRule };
type SaveState =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "ok"; message: string }
  | { kind: "error"; message: string };

const EMPTY_GRAPH: AcceptanceGraph = { entry: 0, nodes: [] };

/** Move `from`→`to` in a new array (row re-prioritise). */
function reorder<T>(items: readonly T[], from: number, to: number): T[] {
  const next = [...items];
  const moved = next.splice(from, 1)[0];
  if (moved === undefined) return next;
  next.splice(to, 0, moved);
  return next;
}

export function AcceptanceWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;
  // Authoring an acceptance policy gates on the NARROW `manage_acceptance` capability ×
  // FI. The rail already hides the surface without it; this makes every edit affordance
  // read-only for a non-holder who reaches it.
  const canEdit = auth.can("manage_acceptance", "fixed_income");
  const readOnly = !canEdit;

  const [rules, setRules] = useState<AcceptanceRule[]>([]);
  const [baseline, setBaseline] = useState<string>("[]");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>({ kind: "list" });
  const [saveState, setSaveState] = useState<SaveState>({ kind: "idle" });

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const g = await app.transport.getAcceptanceGraph();
        if (cancelled) return;
        const loaded = decompileAcceptanceGraphToRules(g ?? EMPTY_GRAPH);
        // A fresh install with no policy seeds a single accept-all catch-all so the table
        // is never empty / invalid on first open (mirrors the server default).
        const seeded = loaded.length > 0 ? loaded : [newDefaultAcceptanceRule()];
        setRules(seeded);
        setBaseline(JSON.stringify(seeded));
        setLoadError(null);
      } catch (e: unknown) {
        if (!cancelled) {
          setLoadError(e instanceof Error ? e.message : "failed to load the acceptance policy");
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [app.transport]);

  const conflicts = useMemo(() => detectAcceptanceRuleConflicts(rules), [rules]);
  const conflictsByRule = useMemo(() => {
    const m = new Map<string, AcceptanceRuleConflict[]>();
    for (const c of conflicts) {
      const list = m.get(c.ruleId);
      if (list) list.push(c);
      else m.set(c.ruleId, [c]);
    }
    return m;
  }, [conflicts]);
  const errorCount = conflicts.filter((c) => c.severity === "error").length;

  const compiled = useMemo(
    () => compileRulesToAcceptanceGraph(rules.filter((r) => r.enabled)),
    [rules],
  );
  const graphIssues = useMemo(() => validateAcceptanceGraph(compiled), [compiled]);
  const dirty = JSON.stringify(rules) !== baseline;
  const blockSave = errorCount > 0 || graphIssues.length > 0;

  const applyRules = useCallback((next: AcceptanceRule[]): void => {
    setRules(next);
    setSaveState({ kind: "idle" });
  }, []);

  const onCreate = useCallback((): void => {
    setMode({
      kind: "editor",
      index: null,
      // A NEW specific rule most often REJECTS a lift (the catch-all is accept-all), so
      // seed the reject decision — the trader can flip it to hold in the editor.
      draft: { id: newAcceptanceRuleId(), conditions: [], action: defaultAcceptanceAction("reject"), enabled: true },
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
    (rule: AcceptanceRule): void => {
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
      await app.transport.updateAcceptanceGraph(
        compileRulesToAcceptanceGraph(rules.filter((r) => r.enabled)),
      );
      setBaseline(JSON.stringify(rules));
      setSaveState({ kind: "ok", message: "Acceptance policy saved." });
    } catch (e: unknown) {
      setSaveState({ kind: "error", message: e instanceof Error ? e.message : "failed to save the policy" });
    }
  }, [blockSave, rules, app.transport]);

  const resetRules = useCallback((): void => {
    setRules(JSON.parse(baseline) as AcceptanceRule[]);
    setMode({ kind: "list" });
    setSaveState({ kind: "idle" });
  }, [baseline]);

  if (!signedIn) {
    return (
      <div className={styles.wrap}>
        <p className={styles.centerEmpty}>Sign in to view the incoming-quote acceptance policy.</p>
      </div>
    );
  }

  if (mode.kind === "editor") {
    return (
      <div className={styles.wrap}>
        <AcceptanceRuleEditor
          draft={mode.draft}
          isNew={mode.index === null}
          readOnly={readOnly}
          onSave={onEditorSave}
          onCancel={() => setMode({ kind: "list" })}
        />
      </div>
    );
  }

  return (
    <div className={styles.wrap}>
      <header className={styles.head}>
        <div className={styles.headMain}>
          <h1 className={styles.title}>Acceptance</h1>
          <p className={styles.note}>
            Compose the first-match rules that decide each incoming client lift.{" "}
            {readOnly ? "Read-only view." : "manage_acceptance · FI edit."}
          </p>
        </div>
      </header>

      <div className={styles.explainer}>
        <span>
          These rules run <strong>at acceptance</strong> — after last-look, before booking — on
          every incoming lift, in order (the first rule that matches wins).
        </span>
        <span>
          <strong>Accept</strong> books the lift · <strong>Reject</strong> declines it with the
          reason (surfaced to the counterparty) · <strong>Hold for review</strong> routes it to the
          desk inbox for a human to accept manually.
        </span>
      </div>

      <div className={styles.policyTab}>
        <div className={styles.policyActions}>
          {!readOnly && (
            <>
              <button
                type="button"
                className={styles.saveBtn}
                onClick={onCreate}
                data-testid="acceptance-create-rule"
              >
                + Create acceptance rule
              </button>
              <button type="button" className={styles.ghostBtn} onClick={resetRules} disabled={!dirty}>
                Reset
              </button>
              <button
                type="button"
                className={styles.saveBtn}
                onClick={() => void saveGraph()}
                disabled={saveState.kind === "saving" || blockSave || !dirty}
                data-testid="acceptance-save-policy"
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
            <span className={styles.statusBad} data-testid="acceptance-validation-status">
              ⚠ {errorCount + graphIssues.length} issue
              {errorCount + graphIssues.length === 1 ? "" : "s"} to resolve before saving
            </span>
          ) : (
            <span className={styles.statusOk} data-testid="acceptance-validation-status">
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
          <ul className={styles.conflictList} data-testid="acceptance-conflict-details">
            {conflicts.map((c, i) => {
              const idx = rules.findIndex((r) => r.id === c.ruleId);
              return (
                <li
                  key={`c-${i}`}
                  className={c.severity === "error" ? styles.conflictError : styles.conflictWarn}
                >
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

        <AcceptanceRulesTable
          rules={rules}
          conflictsByRule={conflictsByRule}
          readOnly={readOnly}
          onEdit={onEditRow}
          onDelete={(i) => applyRules(rules.filter((_, j) => j !== i))}
          onToggle={(i) => applyRules(rules.map((r, j) => (j === i ? { ...r, enabled: !r.enabled } : r)))}
          onReorder={(from, to) => applyRules(reorder(rules, from, to))}
        />

        <AcceptanceTracePanel graph={compiled} />
      </div>
    </div>
  );
}
