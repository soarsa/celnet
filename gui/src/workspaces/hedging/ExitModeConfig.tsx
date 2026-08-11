/**
 * ExitModeConfig — bind each scope to AUTO or SUGGEST.
 *
 * AUTO is the existing behaviour: a breach resolves the policy and trades. SUGGEST changes
 * only the last step — the engine still measures, still resolves the policy and still
 * SIZES the hedge in its vehicle, but then trades nothing and publishes a STANDING row on
 * the risk panel with a "Hedge now" button.
 *
 * The distinction the copy has to carry, because it is the whole design: a suggestion is
 * NOT a confirmation dialog. A modal interrupts whatever the trader was doing, gets
 * dismissed reflexively, and takes the decision with it. A standing row survives being
 * ignored — it waits on the risk panel until someone acts on it.
 *
 * Scoping is the SAME desk/book/instrument axis, and the same most-specific-wins
 * resolution, as {@link LpPanelConfig} and the warehouse thresholds, so a desk learns one
 * scoping model rather than three. Binds to `HedgeConfig.exitModes` via
 * `get_hedge_config` / `set_hedge_config`. Edits gate on the `hedge` capability.
 */
import { useMemo, useState } from "react";

import { HelpButton } from "../../components/HelpButton";
import type { HedgeExitMode, HedgeExitModeBinding, HedgeScopeKind } from "../../data/contract";
import {
  HEDGE_EXIT_MODES,
  exitModeHint,
  exitModeLabel,
  validateExitModeBinding,
} from "../../lib/hedgeVehicle";
import styles from "./HedgingWorkspace.module.css";

interface ExitModeConfigProps {
  bindings: readonly HedgeExitModeBinding[];
  readOnly: boolean;
  busy: boolean;
  /** A server-side rejection to surface, or `null`. */
  saveError: string | null;
  /** Commit the full replacement binding list. */
  onCommit: (bindings: HedgeExitModeBinding[]) => void;
}

type Draft = { index: number | null; binding: HedgeExitModeBinding };

const SCOPE_KINDS: readonly HedgeScopeKind[] = ["desk", "book", "instrument"];

const emptyDraft = (): Draft => ({
  index: null,
  binding: { scopeKind: "book", scopeId: "", mode: "suggest" },
});

/** The mode chip — SUGGEST is the notable state, so only it is tinted. */
function ModeChip({ mode }: { mode: HedgeExitMode }): React.ReactElement {
  return (
    <span
      className={mode === "suggest" ? styles.advisoryBadge : styles.lpChip}
      data-testid={`exit-mode-chip-${mode}`}
    >
      {mode === "suggest" ? "SUGGEST" : "AUTO"}
    </span>
  );
}

export function ExitModeConfig({
  bindings,
  readOnly,
  busy,
  saveError,
  onCommit,
}: ExitModeConfigProps): React.ReactElement {
  const [draft, setDraft] = useState<Draft | null>(null);

  const draftErrors = useMemo(() => {
    if (draft === null) return [];
    const others = bindings.filter((_, i) => i !== draft.index);
    return validateExitModeBinding(draft.binding, others);
  }, [draft, bindings]);

  const setBinding = (p: Partial<HedgeExitModeBinding>): void =>
    setDraft((d) => (d === null ? d : { ...d, binding: { ...d.binding, ...p } }));

  const onSaveDraft = (): void => {
    if (draft === null || draftErrors.length > 0) return;
    const trimmed: HedgeExitModeBinding = { ...draft.binding, scopeId: draft.binding.scopeId.trim() };
    onCommit(
      draft.index === null
        ? [...bindings, trimmed]
        : bindings.map((b, i) => (i === draft.index ? trimmed : b)),
    );
    setDraft(null);
  };

  return (
    <section className={styles.configPanel} aria-labelledby="exit-modes-heading" data-testid="exit-modes">
      <div className={styles.scopeBar}>
        <h3 id="exit-modes-heading" className={styles.panelHeading}>
          Exit mode
        </h3>
        <HelpButton helpId="concept.hedge-exit-mode" subject="the hedge exit mode" />
      </div>
      <p className={styles.panelNote}>
        Whether a resolved hedge <strong>fires by itself</strong>. <strong>Auto</strong> is the
        existing behaviour — a breach trades. <strong>Suggest</strong> changes only the last
        step: the engine still measures, resolves the policy and sizes the hedge in its
        vehicle, then trades nothing and puts a <strong>standing row</strong> on{" "}
        <strong>Risk → Hedge flows</strong> with a “Hedge now” button. It is deliberately{" "}
        <strong>not a confirmation dialog</strong> — nothing pops up, and the row waits there
        until you act on it. Scopes resolve most-specific-wins: instrument &gt; book &gt; desk;
        an unbound scope is Auto.
      </p>

      {saveError !== null && (
        <p className={styles.errorText} role="alert" data-testid="exit-modes-save-error">
          {saveError}
        </p>
      )}

      {bindings.length === 0 ? (
        <p className={styles.emptyNote} data-testid="exit-modes-empty">
          No scope is bound — every book hedges on <strong>Auto</strong>.
        </p>
      ) : (
        <div className={styles.tableScroll}>
          <table className={styles.dataTable} data-testid="exit-modes-table">
            <thead>
              <tr>
                <th scope="col">Scope</th>
                <th scope="col">Mode</th>
                <th scope="col">What happens on a breach</th>
                {!readOnly && <th scope="col">Actions</th>}
              </tr>
            </thead>
            <tbody>
              {bindings.map((b, i) => (
                <tr
                  key={`${b.scopeKind}:${b.scopeId}`}
                  data-testid={`exit-mode-row-${b.scopeKind}-${b.scopeId}`}
                >
                  <td>
                    <span className={styles.scopeTag}>{b.scopeKind}</span>
                    <span>{b.scopeId}</span>
                  </td>
                  <td>
                    <ModeChip mode={b.mode} />
                  </td>
                  <td>
                    {b.mode === "suggest"
                      ? "Sized, then a standing row on Risk → Hedge flows. Nothing trades."
                      : "Trades immediately."}
                  </td>
                  {!readOnly && (
                    <td>
                      <div className={styles.rowActions}>
                        <button
                          type="button"
                          className={styles.ghostBtn}
                          disabled={busy}
                          data-testid={`exit-mode-edit-${b.scopeKind}-${b.scopeId}`}
                          onClick={() => setDraft({ index: i, binding: { ...b } })}
                        >
                          Edit
                        </button>
                        <button
                          type="button"
                          className={styles.dangerBtn}
                          disabled={busy}
                          data-testid={`exit-mode-delete-${b.scopeKind}-${b.scopeId}`}
                          onClick={() => {
                            onCommit(bindings.filter((_, j) => j !== i));
                            setDraft(null);
                          }}
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
            data-testid="exit-mode-add"
            onClick={() => setDraft(emptyDraft())}
          >
            + Bind a scope
          </button>
        </div>
      )}

      {!readOnly && draft !== null && (
        <div className={styles.thresholdForm} data-testid="exit-mode-editor">
          <div className={styles.formGrid}>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Scope</span>
              <select
                className={styles.input}
                value={draft.binding.scopeKind}
                data-testid="exit-mode-scope-kind"
                onChange={(e) => setBinding({ scopeKind: e.target.value as HedgeScopeKind })}
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
                value={draft.binding.scopeId}
                placeholder="e.g. fi-credit-emea"
                data-testid="exit-mode-scope-id"
                onChange={(e) => setBinding({ scopeId: e.target.value })}
              />
            </label>
          </div>

          <fieldset className={styles.deskFieldset}>
            <legend className={styles.fieldLabel}>Mode</legend>
            <div className={styles.execModes} role="radiogroup" aria-label="Exit mode">
              {HEDGE_EXIT_MODES.map((mode) => (
                <label
                  key={mode}
                  className={`${styles.execOption} ${draft.binding.mode === mode ? styles.execOptionActive : ""}`}
                  data-testid={`exit-mode-option-${mode}`}
                >
                  <input
                    type="radio"
                    name="hedge-exit-mode"
                    value={mode}
                    checked={draft.binding.mode === mode}
                    onChange={() => setBinding({ mode })}
                  />
                  <span className={styles.switchMain}>
                    <span className={styles.switchLabel}>{exitModeLabel(mode)}</span>
                    <span className={styles.switchHint}>{exitModeHint(mode)}</span>
                  </span>
                </label>
              ))}
            </div>
          </fieldset>

          {draftErrors.length > 0 && (
            <ul className={styles.conflictList} data-testid="exit-mode-errors">
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
              disabled={busy || draftErrors.length > 0}
              data-testid="exit-mode-save"
              onClick={onSaveDraft}
            >
              {draft.index === null ? "Bind scope" : "Save binding"}
            </button>
            <button
              type="button"
              className={styles.ghostBtn}
              disabled={busy}
              data-testid="exit-mode-cancel"
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
