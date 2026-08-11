/**
 * ThresholdConfig — the warehouse-threshold ("the 100") editor (docs/AUTO-HEDGING §4).
 * A roster of per-scope soft banded budgets (metric, cap, amber/red bands, target
 * fraction, min/max clip, optional ramp) with an inline add/edit form. Reads/writes
 * via the `list_hedge_thresholds` / `update_hedge_threshold` RPCs (the container owns
 * the transport; this component owns the draft form + roster rendering).
 */
import { useState } from "react";

import type { HedgeMetric, HedgeScopeKind, WarehouseThreshold } from "../../data/contract";
import styles from "./HedgingWorkspace.module.css";

import { NumberField } from "../../components/NumberField";

const SCOPE_KINDS: readonly HedgeScopeKind[] = ["desk", "book", "instrument"];
const METRICS: readonly HedgeMetric[] = ["dv01", "net_notional", "net_delta", "net_vega"];

const METRIC_LABEL: Record<HedgeMetric, string> = {
  dv01: "Net DV01",
  net_notional: "Net notional",
  net_delta: "Net delta",
  net_vega: "Net vega",
};
const SCOPE_LABEL: Record<HedgeScopeKind, string> = {
  desk: "Desk",
  book: "Book",
  instrument: "Instrument",
};

/**
 * A fresh, valid draft threshold. The bands (amber 0.80 / red 0.90) and the
 * target-at-the-amber-edge (0.80) mirror the engine's `WarehouseThreshold::default`
 * (`celnet-hedge-routing` `DEFAULT_AMBER`/`DEFAULT_RED`, `target_fraction = amber`)
 * so the pre-fill matches what the pricer would apply with no explicit bands.
 */
function blankThreshold(): WarehouseThreshold {
  return {
    scopeKind: "book",
    scopeId: "",
    metric: "dv01",
    cap: 100_000,
    amber: 0.8,
    red: 0.9,
    targetFraction: 0.8,
    minClip: 1_000,
    maxClip: 50_000,
    ramped: false,
    rampK: 0,
  };
}

interface ThresholdConfigProps {
  thresholds: readonly WarehouseThreshold[];
  readOnly: boolean;
  busy: boolean;
  /** Upsert a threshold (cap 0 deletes server-side). Resolves when the roster refreshed. */
  onUpsert: (threshold: WarehouseThreshold) => Promise<void>;
  /** Delete a threshold by scope (sends cap 0). */
  onDelete: (threshold: WarehouseThreshold) => Promise<void>;
}

export function ThresholdConfig({
  thresholds,
  readOnly,
  busy,
  onUpsert,
  onDelete,
}: ThresholdConfigProps): React.ReactElement {
  const [draft, setDraft] = useState<WarehouseThreshold>(blankThreshold);
  const [error, setError] = useState<string | null>(null);

  const patch = (p: Partial<WarehouseThreshold>): void => setDraft((d) => ({ ...d, ...p }));
  const numField = (key: keyof WarehouseThreshold) => (e: React.ChangeEvent<HTMLInputElement>) =>
    patch({ [key]: Number(e.target.value) } as Partial<WarehouseThreshold>);

  const editRow = (t: WarehouseThreshold): void => {
    setDraft({ ...t });
    setError(null);
  };

  const save = async (): Promise<void> => {
    if (draft.scopeId.trim().length === 0) {
      setError("A scope id (a desk / book / instrument id) is required.");
      return;
    }
    if (!(draft.cap > 0)) {
      setError("The cap (the “100”) must be greater than zero.");
      return;
    }
    if (!(draft.amber >= 0 && draft.amber <= draft.red && draft.red <= 1)) {
      setError("Bands must satisfy 0 ≤ amber ≤ red ≤ 1.");
      return;
    }
    setError(null);
    await onUpsert({ ...draft, scopeId: draft.scopeId.trim() });
    setDraft(blankThreshold());
  };

  return (
    <section className={styles.thresholdPanel} aria-labelledby="threshold-heading" data-testid="threshold-config">
      <h3 id="threshold-heading" className={styles.panelHeading}>
        Warehouse thresholds — the “100”
      </h3>
      <p className={styles.panelNote}>
        A soft, banded risk budget per scope: warehouse inside it, skew at amber, hedge the overflow
        at red. Most-specific-wins (instrument &gt; book &gt; desk).
      </p>

      {thresholds.length === 0 ? (
        <p className={styles.emptyNote} data-testid="threshold-empty">
          No thresholds configured yet.
        </p>
      ) : (
        <div className={styles.tableScroll}>
          <table className={styles.dataTable} data-testid="threshold-table">
            <thead>
              <tr>
                <th scope="col">Scope</th>
                <th scope="col">Metric</th>
                <th scope="col">Cap</th>
                <th scope="col">Amber</th>
                <th scope="col">Red</th>
                <th scope="col">Target</th>
                <th scope="col">Clip (min…max)</th>
                <th scope="col">Ramp</th>
                {!readOnly && <th scope="col">Actions</th>}
              </tr>
            </thead>
            <tbody>
              {thresholds.map((t) => (
                <tr key={`${t.scopeKind}:${t.scopeId}`} data-testid={`threshold-row-${t.scopeId}`}>
                  <td>
                    <span className={styles.scopeTag}>{SCOPE_LABEL[t.scopeKind]}</span> {t.scopeId}
                  </td>
                  <td>{METRIC_LABEL[t.metric]}</td>
                  <td className={styles.num}>{t.cap.toLocaleString()}</td>
                  <td className={styles.num}>{(t.amber * 100).toFixed(0)}%</td>
                  <td className={styles.num}>{(t.red * 100).toFixed(0)}%</td>
                  <td className={styles.num}>{(t.targetFraction * 100).toFixed(0)}%</td>
                  <td className={styles.num}>
                    {t.minClip.toLocaleString()}…{t.maxClip.toLocaleString()}
                  </td>
                  <td>{t.ramped ? `k=${t.rampK}` : "—"}</td>
                  {!readOnly && (
                    <td className={styles.rowActions}>
                      <button type="button" className={styles.ghostBtn} onClick={() => editRow(t)}>
                        Edit
                      </button>
                      <button
                        type="button"
                        className={styles.dangerBtn}
                        disabled={busy}
                        data-testid={`threshold-delete-${t.scopeId}`}
                        onClick={() => void onDelete(t)}
                      >
                        Delete
                      </button>
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {!readOnly && (
        <div className={styles.thresholdForm} data-testid="threshold-form">
          <h4 className={styles.formHeading}>Add / edit a threshold</h4>
          <div className={styles.formGrid}>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Scope</span>
              <select
                className={styles.input}
                value={draft.scopeKind}
                data-testid="threshold-scope-kind"
                onChange={(e) => patch({ scopeKind: e.target.value as HedgeScopeKind })}
              >
                {SCOPE_KINDS.map((k) => (
                  <option key={k} value={k}>
                    {SCOPE_LABEL[k]}
                  </option>
                ))}
              </select>
            </label>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Scope id</span>
              <input
                className={styles.input}
                type="text"
                value={draft.scopeId}
                data-testid="threshold-scope-id"
                placeholder="e.g. fi-rates-emea"
                onChange={(e) => patch({ scopeId: e.target.value })}
              />
            </label>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Metric</span>
              <select
                className={styles.input}
                value={draft.metric}
                data-testid="threshold-metric"
                onChange={(e) => patch({ metric: e.target.value as HedgeMetric })}
              >
                {METRICS.map((m) => (
                  <option key={m} value={m}>
                    {METRIC_LABEL[m]}
                  </option>
                ))}
              </select>
            </label>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Cap (the “100”)</span>
              <NumberField
                className={styles.input}
                value={draft.cap}
                data-testid="threshold-cap"
                onChange={numField("cap")}
              />
            </label>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Amber (0–1)</span>
              <NumberField className={styles.input} step={0.05} value={draft.amber} onChange={numField("amber")} />
            </label>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Red (0–1)</span>
              <NumberField className={styles.input} step={0.05} value={draft.red} onChange={numField("red")} />
            </label>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Target fraction</span>
              <NumberField
                className={styles.input}
                step={0.05}
                value={draft.targetFraction}
                onChange={numField("targetFraction")}
              />
            </label>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Min clip</span>
              <NumberField className={styles.input} value={draft.minClip} onChange={numField("minClip")} />
            </label>
            <label className={styles.formField}>
              <span className={styles.fieldLabel}>Max clip</span>
              <NumberField className={styles.input} value={draft.maxClip} onChange={numField("maxClip")} />
            </label>
            <label className={styles.checkField}>
              <input
                type="checkbox"
                checked={draft.ramped}
                data-testid="threshold-ramped"
                onChange={(e) => patch({ ramped: e.target.checked })}
              />
              <span>Ramp the hedged fraction with utilisation</span>
            </label>
            {draft.ramped && (
              <label className={styles.formField}>
                <span className={styles.fieldLabel}>Ramp gain k</span>
                <NumberField className={styles.input} step={0.1} value={draft.rampK} onChange={numField("rampK")} />
              </label>
            )}
          </div>

          {error !== null && (
            <p className={styles.errorText} role="alert">
              {error}
            </p>
          )}

          <div className={styles.formActions}>
            <button
              type="button"
              className={styles.saveBtn}
              disabled={busy}
              data-testid="threshold-save"
              onClick={() => void save()}
            >
              {busy ? "Saving…" : "Save threshold"}
            </button>
            <button
              type="button"
              className={styles.ghostBtn}
              onClick={() => {
                setDraft(blankThreshold());
                setError(null);
              }}
            >
              Reset form
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
