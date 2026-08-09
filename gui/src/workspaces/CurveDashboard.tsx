/**
 * CurveDashboard — the Curves manager LANDING view: a table of every persisted
 * {@link CurveDefinition} (server commit 38bcff9a), one row per curve carrying its
 * display name (+ the per-currency PRIMARY badge), currency, index, interpolation
 * scheme and a pillar-ladder summary. A row opens the definition editor; explicit
 * per-row actions edit the pillar ladder or delete the curve. "New curve" starts a
 * blank definition. Every write affordance (New / Delete) is gated on the caller's
 * `canEdit` (Refdata·Fixed-Income) — a read-only viewer sees the full dashboard with
 * the write controls hidden, never a faked capability.
 *
 * Presentational + controlled: it owns no data or async: the manager passes the
 * definitions, the loading/error state, and the action callbacks, and renders the
 * server's authoritative list (including the server-maintained `primary`).
 */

import { Button } from "../components/Button";
import { HelpButton } from "../components/HelpButton";
import type { CurveDefinition } from "../data/contract";
import { curveInterpolationLabel } from "../data/contract";
import { pillarLadderSummary } from "../lib/curveEditing";
import styles from "./CurveWorkspace.module.css";

export interface CurveDashboardProps {
  definitions: readonly CurveDefinition[];
  isLoading: boolean;
  /** The last list-load error, or `null`. */
  error: string | null;
  /** The last create/update/delete failure (surfaces `failed_precondition` etc.), or `null`. */
  actionError: string | null;
  /** True when the identity holds Refdata·Fixed-Income (may create / edit / delete). */
  canEdit: boolean;
  /** A curve id whose action is in flight (its Delete shows a busy state), or `null`. */
  busyCurveId: string | null;
  /** Open the metadata editor for an existing curve. */
  onEdit: (curveId: string) => void;
  /** Open the pillar-ladder editor for a curve. */
  onEditPillars: (curveId: string) => void;
  /** Start a new (blank) curve definition. */
  onNew: () => void;
  /** Delete a curve (the manager runs the RPC and surfaces the outcome). */
  onDelete: (curveId: string) => void;
}

export function CurveDashboard({
  definitions,
  isLoading,
  error,
  actionError,
  canEdit,
  busyCurveId,
  onEdit,
  onEditPillars,
  onNew,
  onDelete,
}: CurveDashboardProps): React.ReactElement {
  return (
    <div className={styles.dashboard}>
      <div className={styles.dashHead}>
        <div className={styles.dashHeadMain}>
          <h2 className={styles.dashTitle}>Curve definitions</h2>
          <HelpButton
            helpId="concept.curve-definitions"
            subject="defining and managing curves"
          />
        </div>
        {canEdit && (
          <Button
            variant="primary"
            onClick={onNew}
            title="create a new curve definition"
          >
            + New curve
          </Button>
        )}
      </div>

      {error && (
        <p className={styles.error} role="alert">
          {error}
        </p>
      )}
      {actionError && (
        <p className={styles.error} role="alert">
          {actionError}
        </p>
      )}

      {definitions.length === 0 ? (
        <p className={styles.empty}>
          {isLoading
            ? "Loading curve definitions…"
            : "No curves defined yet. Create one to price and risk off a named discount curve."}
        </p>
      ) : (
        <div className={styles.dashTableWrap}>
          <table className={styles.dashTable}>
            <caption className={styles.srOnly}>
              Persisted curve definitions — select a row to edit
            </caption>
            <thead>
              <tr>
                <th scope="col">Curve</th>
                <th scope="col">Ccy</th>
                <th scope="col">Index</th>
                <th scope="col">Interpolation</th>
                <th scope="col">Pillars</th>
                <th scope="col" className={styles.dashActionsHead}>
                  Actions
                </th>
              </tr>
            </thead>
            <tbody>
              {definitions.map((def) => {
                const busy = busyCurveId === def.curveId;
                return (
                  <tr key={def.curveId} className={styles.dashRow}>
                    <td>
                      <div className={styles.dashCurveCell}>
                        <button
                          type="button"
                          className={styles.dashCurveName}
                          onClick={() => onEdit(def.curveId)}
                          title={`Edit ${def.displayName}`}
                        >
                          {def.displayName}
                        </button>
                        <span className={styles.dashSlug}>{def.curveId}</span>
                        {def.primary && (
                          <span
                            className={styles.primaryBadge}
                            title={`Primary (default) curve for ${def.pillars.currency}`}
                          >
                            Primary
                          </span>
                        )}
                      </div>
                    </td>
                    <td className={styles.dashMono}>{def.pillars.currency}</td>
                    <td className={styles.dashMono}>{def.indexLabel}</td>
                    <td>{curveInterpolationLabel(def.interpolation)}</td>
                    <td>{pillarLadderSummary(def.pillars)}</td>
                    <td className={styles.dashActions}>
                      <button
                        type="button"
                        className={styles.linkBtn}
                        onClick={(e) => {
                          e.stopPropagation();
                          onEditPillars(def.curveId);
                        }}
                        title="edit this curve's calibrating pillars"
                      >
                        Pillars
                      </button>
                      {canEdit && (
                        <button
                          type="button"
                          className={`${styles.linkBtn} ${styles.linkDanger}`}
                          disabled={busy}
                          onClick={(e) => {
                            e.stopPropagation();
                            onDelete(def.curveId);
                          }}
                          title={
                            def.primary
                              ? "the primary curve cannot be deleted while siblings remain — make another primary first"
                              : "delete this curve"
                          }
                        >
                          {busy ? "Deleting…" : "Delete"}
                        </button>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
