/**
 * TieringEditor — the per-book outbound-tiering config editor (FI-TIERING phase 3).
 *
 * A controlled editor embedded in the aggregated-book definition form: the parent
 * owns the `TieringConfig | null` value (`null` ⇒ tiering disabled) and the derived
 * {@link TieringErrors}; this component renders the enable toggle, the spread-unit
 * select, an add/remove list of strategies (Flat markup / Inventory skew — designed
 * so the remaining four kinds slot in later), the guardrail bounds, and the stale
 * policy, and reports every edit through `onChange` with a fresh immutable config.
 *
 * All numeric inputs mirror the server's `celnet-tiering` invariants and surface
 * inline errors from `validateTiering`; the parent blocks submit on any error.
 */

import type {
  TieringConfig,
  TieringGuardrails,
  TieringSpreadUnit,
  TieringStalePolicy,
  TieringStrategy,
  TieringStrategyKind,
} from "../data/contract";
import {
  defaultTieringConfig,
  defaultTieringStrategy,
  TIERING_SPREAD_UNIT_LABEL,
  TIERING_SPREAD_UNITS,
  TIERING_STALE_POLICIES,
  TIERING_STALE_POLICY_LABEL,
  TIERING_STRATEGY_KIND_HINT,
  TIERING_STRATEGY_KIND_LABEL,
  TIERING_STRATEGY_KINDS,
  type TieringErrors,
} from "../lib/tiering";
import styles from "./TieringEditor.module.css";

interface TieringEditorProps {
  /** The current config, or `null` when tiering is disabled. */
  value: TieringConfig | null;
  /** Emit a fresh config (or `null` to disable). */
  onChange: (next: TieringConfig | null) => void;
  /** The structured validation errors for `value` (empty when valid / disabled). */
  errors: TieringErrors;
  /** A stable id prefix so multiple editors on a page keep unique input ids. */
  idPrefix?: string;
}

export function TieringEditor({
  value,
  onChange,
  errors,
  idPrefix = "tiering",
}: TieringEditorProps): React.ReactElement {
  const enabled = value !== null;

  const toggleEnabled = (on: boolean): void => {
    onChange(on ? defaultTieringConfig() : null);
  };

  // Immutable helpers — each returns a NEW config so React state stays pure.
  const patch = (next: Partial<TieringConfig>): void => {
    if (value === null) return;
    onChange({ ...value, ...next });
  };

  const setUnit = (unit: TieringSpreadUnit): void => patch({ unit });
  const setStalePolicy = (stalePolicy: TieringStalePolicy): void => patch({ stalePolicy });

  const addStrategy = (kind: TieringStrategyKind): void => {
    if (value === null) return;
    patch({ strategies: [...value.strategies, defaultTieringStrategy(kind)] });
  };
  const removeStrategy = (index: number): void => {
    if (value === null) return;
    patch({ strategies: value.strategies.filter((_, i) => i !== index) });
  };
  const patchStrategy = (index: number, next: Partial<TieringStrategy>): void => {
    if (value === null) return;
    patch({
      strategies: value.strategies.map((s, i) => (i === index ? { ...s, ...next } : s)),
    });
  };

  const patchGuardrails = (next: Partial<TieringGuardrails>): void => {
    if (value === null || value.guardrails === null) return;
    patch({ guardrails: { ...value.guardrails, ...next } });
  };

  return (
    <section className={styles.section} aria-label="outbound tiering configuration">
      <div className={styles.head}>
        <span className={styles.title}>Outbound tiering</span>
        <label className={styles.enableRow}>
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => toggleEnabled(e.target.checked)}
            aria-label="enable outbound tiering"
          />
          <span>{enabled ? "Enabled" : "Disabled"}</span>
        </label>
      </div>

      {!enabled ? (
        <p className={styles.hint}>
          Tiering widens around mid and/or skews the book&apos;s composite before it is
          published to clients. Disabled ⇒ the raw consolidated best bid/offer is published
          unchanged.
        </p>
      ) : (
        <div className={styles.body}>
          {/* Spread unit. */}
          <div className={styles.field}>
            <label className={styles.fieldLabel} htmlFor={`${idPrefix}-unit`}>
              Spread unit
            </label>
            <select
              id={`${idPrefix}-unit`}
              className={styles.select}
              value={value.unit}
              onChange={(e) => setUnit(e.target.value as TieringSpreadUnit)}
            >
              {TIERING_SPREAD_UNITS.map((u) => (
                <option key={u} value={u}>
                  {TIERING_SPREAD_UNIT_LABEL[u]}
                </option>
              ))}
            </select>
          </div>

          {/* Strategies. */}
          <div className={styles.field}>
            <span className={styles.fieldLabel}>Strategies</span>
            {errors.form && <p className={styles.error}>{errors.form}</p>}
            <div className={styles.strategies}>
              {value.strategies.map((s, i) => {
                const se = errors.strategies[i] ?? {};
                const isSkew = s.kind === "INVENTORY_SKEW";
                return (
                  <div key={i} className={styles.strategyCard}>
                    <div className={styles.strategyHead}>
                      <span className={styles.strategyKind}>
                        <span className={styles.kindBadge}>
                          {TIERING_STRATEGY_KIND_LABEL[s.kind]}
                        </span>
                      </span>
                      <button
                        type="button"
                        className={styles.removeBtn}
                        onClick={() => removeStrategy(i)}
                        aria-label={`Remove ${TIERING_STRATEGY_KIND_LABEL[s.kind]} strategy`}
                      >
                        Remove
                      </button>
                    </div>
                    <p className={styles.strategyHint}>{TIERING_STRATEGY_KIND_HINT[s.kind]}</p>
                    <div className={styles.grid}>
                      <label className={styles.param} htmlFor={`${idPrefix}-s${i}-half`}>
                        <span className={styles.paramLabel}>Half-spread H</span>
                        <input
                          id={`${idPrefix}-s${i}-half`}
                          className={`${styles.input} ${styles.numInput} ${se.halfSpread ? styles.inputError : ""}`}
                          type="number"
                          step="any"
                          value={s.halfSpread}
                          aria-invalid={se.halfSpread ? true : undefined}
                          onChange={(e) => patchStrategy(i, { halfSpread: Number(e.target.value) })}
                        />
                        {se.halfSpread && <span className={styles.error}>{se.halfSpread}</span>}
                      </label>
                      {isSkew && (
                        <>
                          <label className={styles.param} htmlFor={`${idPrefix}-s${i}-kappa`}>
                            <span className={styles.paramLabel}>κ (per unit inventory)</span>
                            <input
                              id={`${idPrefix}-s${i}-kappa`}
                              className={`${styles.input} ${styles.numInput} ${se.kappa ? styles.inputError : ""}`}
                              type="number"
                              step="any"
                              value={s.kappa}
                              aria-invalid={se.kappa ? true : undefined}
                              onChange={(e) => patchStrategy(i, { kappa: Number(e.target.value) })}
                            />
                            {se.kappa && <span className={styles.error}>{se.kappa}</span>}
                          </label>
                          <label className={styles.param} htmlFor={`${idPrefix}-s${i}-smax`}>
                            <span className={styles.paramLabel}>Strategy sMax</span>
                            <input
                              id={`${idPrefix}-s${i}-smax`}
                              className={`${styles.input} ${styles.numInput} ${se.sMax ? styles.inputError : ""}`}
                              type="number"
                              step="any"
                              value={s.sMax}
                              aria-invalid={se.sMax ? true : undefined}
                              onChange={(e) => patchStrategy(i, { sMax: Number(e.target.value) })}
                            />
                            {se.sMax && <span className={styles.error}>{se.sMax}</span>}
                          </label>
                        </>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
            <div className={styles.addRow} role="group" aria-label="add a strategy">
              {TIERING_STRATEGY_KINDS.map((kind) => (
                <button
                  key={kind}
                  type="button"
                  className={styles.addBtn}
                  onClick={() => addStrategy(kind)}
                >
                  + {TIERING_STRATEGY_KIND_LABEL[kind]}
                </button>
              ))}
            </div>
          </div>

          {/* Guardrails. */}
          <div className={styles.field}>
            <span className={styles.fieldLabel}>Guardrails (price points)</span>
            <div className={styles.grid}>
              <label className={styles.param} htmlFor={`${idPrefix}-hmin`}>
                <span className={styles.paramLabel}>h_min</span>
                <input
                  id={`${idPrefix}-hmin`}
                  className={`${styles.input} ${styles.numInput} ${errors.guardrails.hMin ? styles.inputError : ""}`}
                  type="number"
                  step="any"
                  value={value.guardrails?.hMin ?? 0}
                  aria-invalid={errors.guardrails.hMin ? true : undefined}
                  onChange={(e) => patchGuardrails({ hMin: Number(e.target.value) })}
                />
                {errors.guardrails.hMin && (
                  <span className={styles.error}>{errors.guardrails.hMin}</span>
                )}
              </label>
              <label className={styles.param} htmlFor={`${idPrefix}-hmax`}>
                <span className={styles.paramLabel}>h_max</span>
                <input
                  id={`${idPrefix}-hmax`}
                  className={`${styles.input} ${styles.numInput} ${errors.guardrails.hMax ? styles.inputError : ""}`}
                  type="number"
                  step="any"
                  value={value.guardrails?.hMax ?? 0}
                  aria-invalid={errors.guardrails.hMax ? true : undefined}
                  onChange={(e) => patchGuardrails({ hMax: Number(e.target.value) })}
                />
                {errors.guardrails.hMax && (
                  <span className={styles.error}>{errors.guardrails.hMax}</span>
                )}
              </label>
              <label className={styles.param} htmlFor={`${idPrefix}-smax`}>
                <span className={styles.paramLabel}>s_max</span>
                <input
                  id={`${idPrefix}-smax`}
                  className={`${styles.input} ${styles.numInput} ${errors.guardrails.sMax ? styles.inputError : ""}`}
                  type="number"
                  step="any"
                  value={value.guardrails?.sMax ?? 0}
                  aria-invalid={errors.guardrails.sMax ? true : undefined}
                  onChange={(e) => patchGuardrails({ sMax: Number(e.target.value) })}
                />
                {errors.guardrails.sMax && (
                  <span className={styles.error}>{errors.guardrails.sMax}</span>
                )}
              </label>
              <label className={styles.param} htmlFor={`${idPrefix}-floor`}>
                <span className={styles.paramLabel}>spread_floor</span>
                <input
                  id={`${idPrefix}-floor`}
                  className={`${styles.input} ${styles.numInput} ${errors.guardrails.spreadFloor ? styles.inputError : ""}`}
                  type="number"
                  step="any"
                  value={value.guardrails?.spreadFloor ?? 0}
                  aria-invalid={errors.guardrails.spreadFloor ? true : undefined}
                  onChange={(e) => patchGuardrails({ spreadFloor: Number(e.target.value) })}
                />
                {errors.guardrails.spreadFloor && (
                  <span className={styles.error}>{errors.guardrails.spreadFloor}</span>
                )}
              </label>
            </div>
          </div>

          {/* Stale policy. */}
          <div className={styles.field}>
            <label className={styles.fieldLabel} htmlFor={`${idPrefix}-stale`}>
              Stale policy
            </label>
            <select
              id={`${idPrefix}-stale`}
              className={styles.select}
              value={value.stalePolicy}
              onChange={(e) => setStalePolicy(e.target.value as TieringStalePolicy)}
            >
              {TIERING_STALE_POLICIES.map((p) => (
                <option key={p} value={p}>
                  {TIERING_STALE_POLICY_LABEL[p]}
                </option>
              ))}
            </select>
          </div>
        </div>
      )}
    </section>
  );
}
