/**
 * LpPanelConfig — author the STANDING per-scope hedging LP panels
 * (docs/HEDGING-CONFIGURATION-GUIDE §4). Each panel binds a scope (desk / book /
 * instrument) to an include + exclude LP selection that every EXTERNAL exit action
 * inherits; resolution is most-specific-wins (instrument > book > desk). The editor
 * shows the RESOLVED effective LP set live (include-or-all minus exclude) so the trader
 * sees exactly which LPs a hedge will fan to, and mirrors the server write-boundary
 * checks (unknown id / empty effective set) inline. Binds to `HedgeConfig.lpPanels` via
 * `get_hedge_config` / `set_hedge_config`. Edits gate on the `hedge` capability.
 */
import { useMemo, useState } from "react";

import type { HedgeLpPanel, HedgeScopeKind } from "../../data/contract";
import { KNOWN_LPS, effectiveLps, validateLpPanel } from "../../lib/hedgeLpPanel";
import styles from "./HedgingWorkspace.module.css";

interface LpPanelConfigProps {
  panels: readonly HedgeLpPanel[];
  readOnly: boolean;
  busy: boolean;
  /** A server-side rejection (unknown id / empty set) to surface, or `null`. */
  saveError: string | null;
  /** Commit the full replacement panel list (upsert/delete happen client-side). */
  onCommit: (panels: HedgeLpPanel[]) => void;
}

type Draft = { index: number | null; panel: HedgeLpPanel };

const SCOPE_KINDS: readonly HedgeScopeKind[] = ["desk", "book", "instrument"];

const emptyDraft = (): Draft => ({
  index: null,
  panel: { scopeKind: "book", scopeId: "", include: [], exclude: [] },
});

/** A small LP chip strip (excluded ⇒ struck-through amber). */
function LpChips({ ids, excluded }: { ids: readonly string[]; excluded?: boolean }): React.ReactElement {
  if (ids.length === 0) return <span className={styles.lpMuted}>—</span>;
  return (
    <span className={styles.lpChips}>
      {ids.map((id) => (
        <span key={id} className={excluded ? styles.lpChipExcluded : styles.lpChip}>
          {id}
        </span>
      ))}
    </span>
  );
}

export function LpPanelConfig({
  panels,
  readOnly,
  busy,
  saveError,
  onCommit,
}: LpPanelConfigProps): React.ReactElement {
  const [draft, setDraft] = useState<Draft | null>(null);

  const resolved = useMemo(
    () => (draft === null ? [] : effectiveLps(draft.panel, KNOWN_LPS)),
    [draft],
  );
  const draftErrors = useMemo(
    () => (draft === null ? [] : validateLpPanel(draft.panel, KNOWN_LPS)),
    [draft],
  );
  const scopeIdMissing = draft !== null && draft.panel.scopeId.trim() === "";
  const blockSave = draftErrors.length > 0 || scopeIdMissing;

  const setPanel = (p: Partial<HedgeLpPanel>): void =>
    setDraft((d) => (d === null ? d : { ...d, panel: { ...d.panel, ...p } }));

  const toggle = (list: "include" | "exclude", id: string): void =>
    setDraft((d) => {
      if (d === null) return d;
      const cur = d.panel[list];
      const next = cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id];
      return { ...d, panel: { ...d.panel, [list]: next } };
    });

  const onSaveDraft = (): void => {
    if (draft === null || blockSave) return;
    const trimmed: HedgeLpPanel = { ...draft.panel, scopeId: draft.panel.scopeId.trim() };
    const next =
      draft.index === null
        ? [...panels, trimmed]
        : panels.map((p, i) => (i === draft.index ? trimmed : p));
    onCommit(next);
    setDraft(null);
  };

  const onDelete = (index: number): void => {
    onCommit(panels.filter((_, i) => i !== index));
    if (draft?.index === index) setDraft(null);
  };

  return (
    <section
      className={styles.configPanel}
      aria-labelledby="lp-panels-heading"
      data-testid="lp-panels"
    >
      <h3 id="lp-panels-heading" className={styles.panelHeading}>
        LP Panels
      </h3>
      <p className={styles.panelNote}>
        Standing include/exclude LP selection every <strong>external</strong> hedge inherits.
        An empty include starts from all known LPs; excludes are then subtracted. Panels
        resolve <strong>most-specific-wins</strong>: instrument &gt; book &gt; desk. When no
        scope panel matches, the per-rule <code>RFQ_OUT</code> include list still applies
        (back-compat), then the full known panel.
      </p>

      {saveError !== null && (
        <p className={styles.errorText} role="alert" data-testid="lp-panels-save-error">
          {saveError}
        </p>
      )}

      {panels.length === 0 ? (
        <p className={styles.emptyNote} data-testid="lp-panels-empty">
          No standing LP panels — every external hedge fans to the full known panel
          ({KNOWN_LPS.join(", ")}).
        </p>
      ) : (
        <div className={styles.tableScroll}>
          <table className={styles.dataTable} data-testid="lp-panels-table">
            <thead>
              <tr>
                <th scope="col">Scope</th>
                <th scope="col">Include</th>
                <th scope="col">Exclude</th>
                <th scope="col">Resolved LPs</th>
                {!readOnly && <th scope="col">Actions</th>}
              </tr>
            </thead>
            <tbody>
              {panels.map((p, i) => (
                <tr key={`${p.scopeKind}:${p.scopeId}`} data-testid={`lp-panel-row-${p.scopeKind}-${p.scopeId}`}>
                  <td>
                    <span className={styles.scopeTag}>{p.scopeKind}</span>
                    <span className={styles.intentBook}>{p.scopeId}</span>
                  </td>
                  <td>{p.include.length === 0 ? <span className={styles.lpMuted}>all known</span> : <LpChips ids={p.include} />}</td>
                  <td>
                    <LpChips ids={p.exclude} excluded />
                  </td>
                  <td>
                    <span data-testid={`lp-panel-resolved-${p.scopeKind}-${p.scopeId}`}>
                      <LpChips ids={effectiveLps(p, KNOWN_LPS)} />
                    </span>
                  </td>
                  {!readOnly && (
                    <td>
                      <div className={styles.rowActions}>
                        <button
                          type="button"
                          className={styles.ghostBtn}
                          disabled={busy}
                          data-testid={`lp-panel-edit-${p.scopeKind}-${p.scopeId}`}
                          onClick={() => setDraft({ index: i, panel: { ...p, include: [...p.include], exclude: [...p.exclude] } })}
                        >
                          Edit
                        </button>
                        <button
                          type="button"
                          className={styles.dangerBtn}
                          disabled={busy}
                          data-testid={`lp-panel-delete-${p.scopeKind}-${p.scopeId}`}
                          onClick={() => onDelete(i)}
                        >
                          Delete
                        </button>
                      </div>
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {!readOnly && draft === null && (
        <div className={styles.formActions}>
          <button
            type="button"
            className={styles.saveBtn}
            disabled={busy}
            data-testid="lp-panel-add"
            onClick={() => setDraft(emptyDraft())}
          >
            + Add LP panel
          </button>
        </div>
      )}

      {!readOnly && draft !== null && (
        <div className={styles.thresholdForm} data-testid="lp-panel-editor">
          <div className={styles.formGrid}>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Scope</span>
              <select
                className={styles.input}
                value={draft.panel.scopeKind}
                data-testid="lp-panel-scope-kind"
                onChange={(e) => setPanel({ scopeKind: e.target.value as HedgeScopeKind })}
              >
                {SCOPE_KINDS.map((k) => (
                  <option key={k} value={k}>
                    {k}
                  </option>
                ))}
              </select>
            </label>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Scope id</span>
              <input
                className={styles.input}
                type="text"
                value={draft.panel.scopeId}
                placeholder="e.g. fi-rates-emea"
                data-testid="lp-panel-scope-id"
                onChange={(e) => setPanel({ scopeId: e.target.value })}
              />
            </label>
          </div>

          <fieldset className={styles.deskFieldset}>
            <legend className={styles.fieldLabel}>Include (empty ⇒ all known)</legend>
            <div className={styles.deskToggles}>
              {KNOWN_LPS.map((lp) => (
                <label key={lp} className={styles.checkField}>
                  <input
                    type="checkbox"
                    checked={draft.panel.include.includes(lp)}
                    data-testid={`lp-panel-include-${lp}`}
                    onChange={() => toggle("include", lp)}
                  />
                  <span>{lp}</span>
                </label>
              ))}
            </div>
          </fieldset>

          <fieldset className={styles.deskFieldset}>
            <legend className={styles.fieldLabel}>Exclude</legend>
            <div className={styles.deskToggles}>
              {KNOWN_LPS.map((lp) => (
                <label key={lp} className={styles.checkField}>
                  <input
                    type="checkbox"
                    checked={draft.panel.exclude.includes(lp)}
                    data-testid={`lp-panel-exclude-${lp}`}
                    onChange={() => toggle("exclude", lp)}
                  />
                  <span>{lp}</span>
                </label>
              ))}
            </div>
          </fieldset>

          <div className={styles.traceResult} data-testid="lp-panel-resolved">
            <span className={styles.traceResultLabel}>Resolved effective LP set</span>
            <LpChips ids={resolved} />
          </div>

          {draftErrors.length > 0 && (
            <ul className={styles.conflictList} data-testid="lp-panel-errors">
              {draftErrors.map((e, i) => (
                <li key={i} className={styles.conflictError}>
                  {e}
                </li>
              ))}
            </ul>
          )}

          <div className={styles.formActions}>
            <button
              type="button"
              className={styles.saveBtn}
              disabled={busy || blockSave}
              data-testid="lp-panel-save"
              onClick={onSaveDraft}
            >
              {draft.index === null ? "Add panel" : "Save panel"}
            </button>
            <button
              type="button"
              className={styles.ghostBtn}
              disabled={busy}
              data-testid="lp-panel-cancel"
              onClick={() => setDraft(null)}
            >
              Cancel
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
