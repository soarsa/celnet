/**
 * CapabilityMatrix — the per-user capability-overlay editor surfaced inside the
 * Admin workspace. Rows are the nine actions, columns the two asset classes; each
 * cell shows the resolved effect (allowed / blocked) and an editable tri-state
 * overlay (inherit / grant / deny) layered on the user's role bundle.
 *
 * Semantics surfaced (mirrors `celnet-entitlements`):
 *   - `effective` = `role bundle ∪ grants ∖ denies`, deny-wins. Shown prominently
 *     per cell (allowed cells highlighted), and as a server-computed set the editor
 *     re-reads after every save.
 *   - a **deny visibly overrides** a would-be-allowed role default — and never by
 *     colour alone (an explicit icon + "Deny overrides" label carry it too).
 *   - **Save replaces the overlay wholesale** and revokes the target's live
 *     sessions; the editor shows that note on success.
 *
 * Affordance discipline: controls are disabled with an explanatory tooltip, never
 * hidden (a save in flight, or the signed-in admin's own `administer` cells —
 * a self-lockout guard).
 */

import { Button } from "./Button";
import type { UserDesc } from "../data/contract";
import { CAPABILITY_ACTIONS, CAPABILITY_ASSETS } from "../data/contract";
import type { CelnetTransport } from "../data/transport";
import { useCapabilityEditor } from "../hooks/useCapabilityEditor";
import {
  ACTION_LABELS,
  ASSET_LABELS,
  capKey,
  overlayStateAt,
  resolveCell,
} from "../lib/capabilityMatrix";
import styles from "./CapabilityMatrix.module.css";

interface CapabilityMatrixProps {
  /** The user whose overlay is being edited. */
  user: UserDesc;
  /** The transport (live WS or offline mock). */
  transport: CelnetTransport;
  /** The signed-in admin's id — guards self-lockout on `administer`. */
  signedInUserId: string | undefined;
}

/** A short human label for an overlay state. */
const OVERLAY_LABEL: Record<"inherit" | "grant" | "deny", string> = {
  inherit: "Inherit",
  grant: "Grant",
  deny: "Deny",
};

export function CapabilityMatrix({
  user,
  transport,
  signedInUserId,
}: CapabilityMatrixProps): React.ReactElement {
  const editor = useCapabilityEditor(transport, user);

  const roleLabel = user.role === "ADMIN" ? "Administrator (grant-all)" : "Trader";
  const isSelf = user.id === signedInUserId;

  return (
    <section className={styles.root} aria-labelledby="cap-matrix-heading">
      <header className={styles.head}>
        <div>
          <h3 id="cap-matrix-heading" className={styles.title}>
            Capabilities — {user.email}
          </h3>
          <p className={styles.subtitle}>
            Role baseline: <strong>{roleLabel}</strong>. Grants widen it, denies narrow it; a
            deny always wins. Effective = role bundle, plus grants, minus denies.
          </p>
        </div>
        <div className={styles.headActions}>
          <Button
            variant="ghost"
            onClick={() => editor.reset()}
            disabled={!editor.isDirty || editor.isSaving}
            title={editor.isDirty ? "Discard unsaved changes" : "No unsaved changes"}
          >
            Reset
          </Button>
          <Button
            variant="primary"
            onClick={() => void editor.save()}
            disabled={!editor.isDirty || editor.isSaving || editor.isLoading}
            title={
              editor.isSaving
                ? "Saving…"
                : editor.isDirty
                  ? "Save the overlay (ends the user's live sessions)"
                  : "No unsaved changes to save"
            }
          >
            {editor.isSaving ? "Saving…" : "Save capabilities"}
          </Button>
        </div>
      </header>

      {editor.error && (
        <p className={styles.error} role="alert">
          {editor.error}
        </p>
      )}
      {editor.savedNote && (
        <p className={styles.savedNote} role="status">
          {editor.savedNote}
        </p>
      )}

      {editor.isLoading ? (
        <p className={styles.loading}>Loading capabilities…</p>
      ) : (
        <table className={styles.table}>
          <caption className={styles.caption}>
            Each cell shows the resolved effect and an editable overlay. Activate a cell to cycle
            its overlay: inherit, then grant, then deny.
          </caption>
          <thead>
            <tr>
              <th scope="col" className={styles.actionHead}>
                Action
              </th>
              {CAPABILITY_ASSETS.map((asset) => (
                <th key={asset} scope="col" className={styles.assetHead}>
                  {ASSET_LABELS[asset]}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {CAPABILITY_ACTIONS.map((action) => (
              <tr key={action}>
                <th scope="row" className={styles.actionCell}>
                  {ACTION_LABELS[action]}
                </th>
                {CAPABILITY_ASSETS.map((asset) => {
                  const key = capKey(action, asset);
                  const overlay = overlayStateAt(editor.overlay, action, asset);
                  const cell = resolveCell(user.role, action, asset, overlay);
                  const selfLock = isSelf && action === "administer";
                  const disabled = editor.isSaving || selfLock;
                  const tooltip = selfLock
                    ? "You cannot change your own administration access."
                    : editor.isSaving
                      ? "Saving…"
                      : "Cycle overlay: inherit → grant → deny";
                  const ariaLabel =
                    `${ACTION_LABELS[action]} on ${ASSET_LABELS[asset]}: ` +
                    `${cell.allowed ? "allowed" : "blocked"}, overlay ${OVERLAY_LABEL[overlay]}` +
                    `${cell.denyOverrides ? ", deny overrides role default" : ""}` +
                    `${disabled ? "" : ". Activate to cycle the overlay."}`;
                  return (
                    <td key={asset} className={styles.cellWrap}>
                      <button
                        type="button"
                        className={styles.cell}
                        data-allowed={cell.allowed}
                        data-overlay={overlay}
                        data-deny-override={cell.denyOverrides}
                        disabled={disabled}
                        title={tooltip}
                        aria-label={ariaLabel}
                        onClick={() => editor.cycle(key)}
                      >
                        <span className={styles.effect}>
                          <span className={styles.effectIcon} aria-hidden="true">
                            {cell.allowed ? "✓" : "✕"}
                          </span>
                          {cell.allowed ? "Allowed" : "Blocked"}
                        </span>
                        <span className={styles.overlayTag} data-overlay={overlay}>
                          {OVERLAY_LABEL[overlay]}
                          {overlay === "inherit" ? " (role default)" : ""}
                        </span>
                        {cell.denyOverrides && (
                          <span className={styles.denyOverride}>
                            <span aria-hidden="true">⛔ </span>
                            Deny overrides role default
                          </span>
                        )}
                      </button>
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
