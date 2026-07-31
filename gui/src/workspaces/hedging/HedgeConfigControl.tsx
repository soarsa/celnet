/**
 * HedgeConfigControl — the engine safety controls (docs/AUTO-HEDGING §8.4): the global
 * KILL-SWITCH (halts all auto-hedging → positions warehouse), the ADVISORY-ONLY flag
 * (compute + emit intents but never trade), per-desk toggles, and the rate/size guards.
 * Reads/writes `get_hedge_config` / `set_hedge_config`. Edits gate on the `hedge`
 * capability; everyone else sees the state read-only.
 */
import type { HedgeConfig } from "../../data/contract";
import styles from "./HedgingWorkspace.module.css";

interface HedgeConfigControlProps {
  config: HedgeConfig;
  readOnly: boolean;
  busy: boolean;
  onChange: (next: HedgeConfig) => void;
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

        <label className={`${styles.bigSwitch} ${config.advisoryOnly ? styles.switchAmber : ""}`}>
          <input
            type="checkbox"
            checked={config.advisoryOnly}
            disabled={disabled}
            data-testid="advisory-only"
            onChange={(e) => patch({ advisoryOnly: e.target.checked })}
          />
          <span className={styles.switchMain}>
            <span className={styles.switchLabel}>Advisory only</span>
            <span className={styles.switchHint}>
              {config.advisoryOnly ? "Dry-run — intents emitted, nothing traded" : "Live — hedges trade"}
            </span>
          </span>
        </label>
      </div>

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
          <input
            className={styles.input}
            type="number"
            value={config.maxClip}
            disabled={disabled}
            onChange={(e) => patch({ maxClip: Number(e.target.value) })}
          />
        </label>
        <label className={styles.formField}>
          <span className={styles.fieldLabel}>Max hedges / interval</span>
          <input
            className={styles.input}
            type="number"
            value={config.maxHedgesPerInterval}
            disabled={disabled}
            onChange={(e) => patch({ maxHedgesPerInterval: Math.max(0, Math.trunc(Number(e.target.value))) })}
          />
        </label>
        <label className={styles.formField}>
          <span className={styles.fieldLabel}>Daily external cap</span>
          <input
            className={styles.input}
            type="number"
            value={config.dailyExternalNotionalCap}
            disabled={disabled}
            onChange={(e) => patch({ dailyExternalNotionalCap: Number(e.target.value) })}
          />
        </label>
      </div>
    </section>
  );
}
