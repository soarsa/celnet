/**
 * RiskRoutingWorkspace — the FI risk-routing editor, reworked as a conventional
 * RULES-TABLE + per-rule-editor CRUD flow (docs/FI-RISK-ROUTING-REQUIREMENTS.md
 * §6.1, §8.6). The home view is an ordered TABLE of `IF <ANDed conditions> THEN
 * <risk book>` rules — row order is priority (first-match-wins). "Create risk rule"
 * / "Edit" opens the {@link RuleEditor}; Save adds/updates the rule and returns to
 * the table; "Save routing" persists the whole compiled graph.
 *
 * The wire model ({@link RiskRoutingGraph}) is UNCHANGED — the table compiles to a
 * deterministic first-match-wins graph spine ({@link compileRulesToGraph}) and loads
 * back via {@link decompileGraphToRules}. There is NO yes/no branch wiring and no
 * hand-built graph cycles: the confusing decision-tree canvas is gone. Editing needs
 * the FI risk capability; everyone else sees the table read-only.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import { useApp } from "../../app/AppContext";
import type {
  DeskDesc,
  FixConnection,
  RiskBook,
  RiskRoutingGraph,
} from "../../data/contract";
import {
  compileRulesToGraph,
  decompileGraphToRules,
  detectRuleConflicts,
  newRuleId,
  type RiskRule,
  type RuleConflict,
} from "../../lib/riskRules";
import { validateGraph } from "../../lib/routeTrace";
import { makeBookLabel } from "./nodeLabel";
import { RiskRulesTable } from "./RiskRulesTable";
import { RuleEditor } from "./RuleEditor";
import styles from "./RiskRoutingWorkspace.module.css";

type SaveState =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "ok"; message: string }
  | { kind: "error"; message: string };

type Mode = { kind: "list" } | { kind: "editor"; index: number | null; draft: RiskRule };

const EMPTY_GRAPH: RiskRoutingGraph = { entry: 0, nodes: [] };

/** Move `from`→`to` in a new array (row re-prioritise). */
function reorder<T>(items: readonly T[], from: number, to: number): T[] {
  const next = [...items];
  const moved = next.splice(from, 1)[0];
  if (moved === undefined) return next;
  next.splice(to, 0, moved);
  return next;
}

export function RiskRoutingWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const signedIn = auth.user !== undefined && auth.user !== null;
  // Routing rules are FI risk management — a risk manager holding the FI trader
  // capability may edit (not admin-only); book STRUCTURE stays admin (Risk Books pane).
  const canEdit = auth.can("quote_respond", "fixed_income");
  const readOnly = !canEdit;

  const [rules, setRules] = useState<RiskRule[]>([]);
  const [baseline, setBaseline] = useState<string>("[]");
  const [books, setBooks] = useState<RiskBook[]>([]);
  const [desks, setDesks] = useState<DeskDesc[]>([]);
  const [connections, setConnections] = useState<FixConnection[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>({ kind: "list" });
  const [saveState, setSaveState] = useState<SaveState>({ kind: "idle" });

  // --- load -----------------------------------------------------------------
  useEffect(() => {
    if (!signedIn) return;
    let cancelled = false;
    void (async () => {
      try {
        const [g, b] = await Promise.all([
          app.transport.getRiskRoutingGraph(),
          app.transport.listRiskBooks(),
        ]);
        if (cancelled) return;
        const loaded = decompileGraphToRules(g ?? EMPTY_GRAPH);
        setRules(loaded);
        setBaseline(JSON.stringify(loaded));
        setBooks(b);
        setLoadError(null);
      } catch (e: unknown) {
        if (!cancelled) {
          setLoadError(e instanceof Error ? e.message : "failed to load the routing rules");
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [app.transport, signedIn]);

  // Desks power the desk-scoped book labels ("DESK / BOOK") in the table + editor —
  // loaded for everyone (the read-only view reads them too).
  useEffect(() => {
    let cancelled = false;
    void app.transport
      .listDesks()
      .then((d) => !cancelled && setDesks(d))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [app.transport]);

  // FIX counterparties fill the `counterparty` value dropdown — only an editor picks.
  useEffect(() => {
    if (!canEdit) return;
    let cancelled = false;
    void app.transport
      .listFixConnections()
      .then((c) => !cancelled && setConnections(c))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [app.transport, canEdit]);

  // --- derived --------------------------------------------------------------
  const knownBookIds = useMemo(
    () => new Set(books.filter((b) => b.enabled).map((b) => b.id)),
    [books],
  );
  const bookLabel = useMemo(() => makeBookLabel(books, desks), [books, desks]);

  const conflicts = useMemo(
    () => detectRuleConflicts(rules, knownBookIds),
    [rules, knownBookIds],
  );
  const conflictsByRule = useMemo(() => {
    const m = new Map<string, RuleConflict[]>();
    for (const c of conflicts) {
      const list = m.get(c.ruleId);
      if (list) list.push(c);
      else m.set(c.ruleId, [c]);
    }
    return m;
  }, [conflicts]);
  const errorCount = conflicts.filter((c) => c.severity === "error").length;
  const warnCount = conflicts.filter((c) => c.severity === "warn").length;

  // The compiled graph (enabled rules only) + the server-parity graph validation.
  const compiled = useMemo(
    () => compileRulesToGraph(rules.filter((r) => r.enabled)),
    [rules],
  );
  const graphIssues = useMemo(
    () => validateGraph(compiled, knownBookIds),
    [compiled, knownBookIds],
  );

  const dirty = JSON.stringify(rules) !== baseline;
  const blockSave = errorCount > 0 || graphIssues.length > 0;

  // --- rule mutations -------------------------------------------------------
  const applyRules = useCallback((next: RiskRule[]): void => {
    setRules(next);
    setSaveState({ kind: "idle" });
  }, []);

  const onCreate = useCallback((): void => {
    setMode({
      kind: "editor",
      index: null,
      draft: { id: newRuleId(), conditions: [], bookId: null, enabled: true },
    });
  }, []);

  const onEditRow = useCallback(
    (index: number): void => {
      const draft = rules[index];
      if (draft) setMode({ kind: "editor", index, draft });
    },
    [rules],
  );

  const onDeleteRow = useCallback(
    (index: number): void => applyRules(rules.filter((_, i) => i !== index)),
    [rules, applyRules],
  );

  const onToggleRow = useCallback(
    (index: number): void =>
      applyRules(rules.map((r, i) => (i === index ? { ...r, enabled: !r.enabled } : r))),
    [rules, applyRules],
  );

  const onReorderRow = useCallback(
    (from: number, to: number): void => applyRules(reorder(rules, from, to)),
    [rules, applyRules],
  );

  const onEditorSave = useCallback(
    (rule: RiskRule): void => {
      setRules((cur) => {
        if (mode.kind !== "editor") return cur;
        if (mode.index !== null) {
          return cur.map((r, i) => (i === mode.index ? rule : r));
        }
        // A NEW rule: a specific rule slots ABOVE the trailing catch-all so the
        // default stays last (first-match-wins); a new default appends.
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

  const onEditorCancel = useCallback(() => setMode({ kind: "list" }), []);

  // --- persist --------------------------------------------------------------
  const saveRouting = useCallback(async (): Promise<void> => {
    if (blockSave) {
      setSaveState({
        kind: "error",
        message: "Resolve the highlighted rule conflicts before saving.",
      });
      return;
    }
    setSaveState({ kind: "saving" });
    try {
      await app.transport.updateRiskRoutingGraph(compileRulesToGraph(rules.filter((r) => r.enabled)));
      // Keep the local rules (incl. disabled) as the new clean baseline.
      setBaseline(JSON.stringify(rules));
      setSaveState({ kind: "ok", message: "Routing rules saved." });
    } catch (e: unknown) {
      setSaveState({
        kind: "error",
        message: e instanceof Error ? e.message : "failed to save the routing rules",
      });
    }
  }, [blockSave, rules, app.transport]);

  const resetRules = useCallback((): void => {
    setRules(JSON.parse(baseline) as RiskRule[]);
    setMode({ kind: "list" });
    setSaveState({ kind: "idle" });
  }, [baseline]);

  if (!signedIn) {
    return (
      <div className={styles.wrap}>
        <p className={styles.centerEmpty}>Sign in to view the risk-routing rules.</p>
      </div>
    );
  }

  if (mode.kind === "editor") {
    return (
      <div className={styles.wrap}>
        <RuleEditor
          draft={mode.draft}
          isNew={mode.index === null}
          books={books}
          desks={desks}
          connections={connections}
          bookLabel={bookLabel}
          readOnly={readOnly}
          onSave={onEditorSave}
          onCancel={onEditorCancel}
        />
      </div>
    );
  }

  return (
    <div className={styles.wrap}>
      <header className={styles.head}>
        <div className={styles.headMain}>
          <h1 className={styles.title}>Risk Routing</h1>
          <p className={styles.note}>
            Ordered rules route every fill's risk into a desk's book — the first rule that matches
            wins. Reorder rows to re-prioritise. {readOnly ? "Read-only view." : "FI risk edit."}
          </p>
        </div>
        <div className={styles.headActions}>
          {!readOnly && (
            <>
              <button
                type="button"
                className={styles.saveBtn}
                onClick={onCreate}
                data-testid="create-rule"
              >
                + Create risk rule
              </button>
              <button type="button" className={styles.ghostBtn} onClick={resetRules} disabled={!dirty}>
                Reset
              </button>
              <button
                type="button"
                className={styles.saveBtn}
                onClick={() => void saveRouting()}
                disabled={saveState.kind === "saving" || blockSave || !dirty}
                data-testid="save-graph"
              >
                {saveState.kind === "saving" ? "Saving…" : "Save routing"}
              </button>
            </>
          )}
        </div>
      </header>

      {loadError && (
        <p className={styles.error} role="alert">
          {loadError}
        </p>
      )}

      <div className={styles.statusRow}>
        {blockSave ? (
          <span className={styles.statusBad} data-testid="validation-status">
            ⚠ {errorCount + graphIssues.length} issue
            {errorCount + graphIssues.length === 1 ? "" : "s"} to resolve before saving
          </span>
        ) : (
          <span className={styles.statusOk} data-testid="validation-status">
            ✓ Valid — {rules.length} rule{rules.length === 1 ? "" : "s"}
          </span>
        )}
        {warnCount > 0 && (
          <span className={styles.statusWarn}>
            {warnCount} warning{warnCount === 1 ? "" : "s"}
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
        <ul className={styles.conflictList} data-testid="conflict-details">
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

      <RiskRulesTable
        rules={rules}
        bookLabel={bookLabel}
        conflictsByRule={conflictsByRule}
        readOnly={readOnly}
        onEdit={onEditRow}
        onDelete={onDeleteRow}
        onToggle={onToggleRow}
        onReorder={onReorderRow}
      />
    </div>
  );
}
