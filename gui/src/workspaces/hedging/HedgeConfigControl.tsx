/**
 * HedgeConfigControl — the engine safety controls (docs/AUTO-HEDGING §8.4): the global
 * KILL-SWITCH (halts all auto-hedging → positions warehouse), the per-policy EXECUTION
 * MODE (Advisory / LP panel / Composite / LP panel → Composite — replaces the old
 * advisory-only boolean), the composite half-spread, per-desk toggles, and the
 * rate/size guards. Reads/writes `get_hedge_config` / `set_hedge_config`. Edits gate on
 * the `hedge` capability; everyone else sees the state read-only.
 */
import type { HedgeConfig, HedgeExecutionMode } from "../../data/contract";
import { HelpButton } from "../../components/HelpButton";
import styles from "./HedgingWorkspace.module.css";

import { NumberField } from "../../components/NumberField";

interface HedgeConfigControlProps {
  config: HedgeConfig;
  readOnly: boolean;
  busy: boolean;
  onChange: (next: HedgeConfig) => void;
}

/** The four execution modes, in wire-ordinal order, each with a clear label + hint. */
const EXEC_MODES: readonly { mode: HedgeExecutionMode; label: string; hint: string }[] = [
  { mode: "advisory", label: "Advisory", hint: "Dry-run — intents emitted, nothing traded" },
  { mode: "lp_panel", label: "LP panel", hint: "Route external legs to the standing LP panel" },
  { mode: "composite", label: "Composite", hint: "Cross the live consolidated Agg-Book mid" },
  {
    mode: "lp_panel_then_composite",
    label: "LP panel → Composite",
    hint: "Route to LPs, fall back to composite mid",
  },
];

/** Whether a mode crosses the composite (so the composite half-spread applies). */
function modeUsesComposite(mode: HedgeExecutionMode): boolean {
  return mode === "composite" || mode === "lp_panel_then_composite";
}

export function HedgeConfigControl({
  config,
  readOnly,
  busy,
  onChange,
}: HedgeConfigControlProps): React.ReactElement {
  const patch = (p: Partial<HedgeConfig>): void => onChange({ ...config, ...p });
  const setDesk = (desk: string, enabled: boolean): void =>
    patch({ deskEnabled: config.deskEnabled.map((d) => (d.desk === desk ? { ...d, enabled } : d)) });
  const disabled = readOnly || busy;
  const showComposite = modeUsesComposite(config.execution);

  return (
    <section className={styles.configPanel} aria-labelledby="hedge-config-heading" data-testid="hedge-config">
      <h3 id="hedge-config-heading" className={styles.panelHeading}>
        Engine controls
      </h3>

      <div className={styles.switchRow}>
        <label className={`${styles.bigSwitch} ${config.killSwitch ? styles.switchDanger : ""}`}>
          <input
            type="checkbox"
            checked={config.killSwitch}
            disabled={disabled}
            data-testid="kill-switch"
            onChange={(e) => patch({ killSwitch: e.target.checked })}
          />
          <span className={styles.switchMain}>
            <span className={styles.switchLabel}>Kill switch</span>
            <span className={styles.switchHint}>
              {config.killSwitch ? "HALTED — all risk warehouses" : "Engine armed"}
            </span>
          </span>
        </label>
      </div>

      <fieldset
        className={`${styles.deskFieldset} ${config.execution === "advisory" ? styles.switchAmber : ""}`}
        data-testid="hedge-execution-mode"
      >
        <legend className={styles.execLegend}>
          Execution mode
          <HelpButton helpId="concept.hedge-execution-mode" subject="the hedge execution mode" />
        </legend>
        <div className={styles.execModes} role="radiogroup" aria-label="Execution mode">
          {EXEC_MODES.map(({ mode, label, hint }) => {
            const selected = config.execution === mode;
            return (
              <label
                key={mode}
                className={`${styles.execOption} ${selected ? styles.execOptionActive : ""}`}
                data-testid={`exec-mode-${mode}`}
              >
                <input
                  type="radio"
                  name="hedge-execution-mode"
                  value={mode}
                  checked={selected}
                  disabled={disabled}
                  onChange={() => patch({ execution: mode })}
                />
                <span className={styles.switchMain}>
                  <span className={styles.switchLabel}>{label}</span>
                  <span className={styles.switchHint}>{hint}</span>
                </span>
              </label>
            );
          })}
        </div>
      </fieldset>

      {config.deskEnabled.length > 0 && (
        <fieldset className={styles.deskFieldset}>
          <legend className={styles.fieldLabel}>Per-desk enable</legend>
          <div className={styles.deskToggles}>
            {config.deskEnabled.map((d) => (
              <label key={d.desk} className={styles.checkField}>
                <input
                  type="checkbox"
                  checked={d.enabled}
                  disabled={disabled}
                  data-testid={`desk-toggle-${d.desk}`}
                  onChange={(e) => setDesk(d.desk, e.target.checked)}
                />
                <span>{d.desk}</span>
              </label>
            ))}
          </div>
        </fieldset>
      )}

      <div className={styles.formGrid}>
        <label className={styles.formField}>
          <span className={styles.fieldLabel}>Max clip</span>
          <NumberField
            className={styles.input}
            value={config.maxClip}
            disabled={disabled}
            onChange={(e) => patch({ maxClip: Number(e.target.value) })}
          />
        </label>
        <label className={styles.formField}>
          <span className={styles.fieldLabel}>Max hedges / interval</span>
          <NumberField
            className={styles.input}
            value={config.maxHedgesPerInterval}
            disabled={disabled}
            onChange={(e) => patch({ maxHedgesPerInterval: Math.max(0, Math.trunc(Number(e.target.value))) })}
          />
        </label>
        <label className={styles.formField}>
          <span className={styles.fieldLabel}>Daily external cap</span>
          <NumberField
            className={styles.input}
            value={config.dailyExternalNotionalCap}
            disabled={disabled}
            onChange={(e) => patch({ dailyExternalNotionalCap: Number(e.target.value) })}
          />
        </label>
        {showComposite && (
          <label className={styles.formField} data-testid="composite-spread-field">
            <span className={styles.fieldLabel}>Composite spread (bp)</span>
            <NumberField
              className={styles.input}
              step="0.1"
              min="0"
              value={config.compositeSpreadBp}
              disabled={disabled}
              data-testid="composite-spread-bp"
              onChange={(e) => patch({ compositeSpreadBp: Math.max(0, Number(e.target.value)) })}
            />
          </label>
        )}
      </div>
    </section>
  );
}
