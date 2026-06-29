/**
 * RoleCapabilityEditor — the per-ROLE capability-bundle editor surfaced inside the
 * Admin Permissions workspace. A sibling to {@link CapabilityMatrix} (which edits a
 * single user's overlay): this edits the *base* a role confers before any per-user
 * overlay. Rows are the nine actions, columns the two asset classes; each cell is a
 * flat on/off membership toggle of the role's bundle.
 *
 * Semantics surfaced (mirrors the server's `IdentityStore::role_bundles`):
 *   - a non-admin role's bundle is editable; saving replaces it wholesale and ends
 *     the live sessions of every user holding the role (shown as a note on success);
 *   - the `ADMIN` role is grant-all and **immutable** — shown read-only (every cell
 *     "In bundle", disabled), with the save affordance disabled and explained.
 *
 * Affordance discipline: controls are disabled with an explanatory tooltip, never
 * hidden. Reuses the {@link CapabilityMatrix} stylesheet so the two editors read as
 * one family.
 */

import { useState } from "react";

import { Button } from "./Button";
import type { UserRole } from "../data/contract";
import { CAPABILITY_ACTIONS, CAPABILITY_ASSETS } from "../data/contract";
import type { CelnetTransport } from "../data/transport";
import { useRoleCapabilityEditor } from "../hooks/useRoleCapabilityEditor";
import { ACTION_LABELS, ASSET_LABELS, capKey } from "../lib/capabilityMatrix";
import styles from "./CapabilityMatrix.module.css";

interface RoleCapabilityEditorProps {
  /** The transport (live WS or offline mock). */
  transport: CelnetTransport;
}

/** The roles a bundle can be edited for, in display order. */
const EDITABLE_ROLES: readonly { role: UserRole; label: string }[] = [
  { role: "TRADER", label: "Trader" },
  { role: "ADMIN", label: "Administrator" },
];

export function RoleCapabilityEditor({
  transport,
}: RoleCapabilityEditorProps): React.ReactElement {
  const [role, setRole] = useState<UserRole>("TRADER");
  const editor = useRoleCapabilityEditor(transport, role);

  const subtitle = editor.isAdminRole
    ? "The Administrator role is grant-all and cannot be narrowed — every capability is always in its bundle."
    : "The base capabilities this role confers. Per-user grants and denies still layer on top. Saving ends every signed-in holder's session.";

  return (
    <section className={styles.root} aria-labelledby="role-matrix-heading">
      <header className={styles.head}>
        <div>
          <h3 id="role-matrix-heading" className={styles.title}>
            Role bundles
          </h3>
          <p className={styles.subtitle}>{subtitle}</p>
          <div
            className={styles.headActions}
            role="group"
            aria-label="select a role to edit"
          >
            {EDITABLE_ROLES.map((r) => (
              <Button
                key={r.role}
                variant={r.role === role ? "primary" : "ghost"}
                onClick={() => setRole(r.role)}
                disabled={editor.isSaving}
                aria-pressed={r.role === role}
                title={`Edit the ${r.label} role bundle`}
              >
                {r.label}
              </Button>
            ))}
          </div>
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
            disabled={
              editor.isAdminRole || !editor.isDirty || editor.isSaving || editor.isLoading
            }
            title={
              editor.isAdminRole
                ? "The Administrator role is grant-all and cannot be narrowed"
                : editor.isSaving
                  ? "Saving…"
                  : editor.isDirty
                    ? "Save the role bundle (ends every holder's live session)"
                    : "No unsaved changes to save"
            }
          >
            {editor.isSaving ? "Saving…" : "Save role bundle"}
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
        <p className={styles.loading}>Loading role bundle…</p>
      ) : (
        <table className={styles.table}>
          <caption className={styles.caption}>
            Each cell is a capability in the role&apos;s base bundle. Activate a cell to add or
            remove it.
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
                  const inBundle = editor.isAdminRole || editor.selected.has(key);
                  const disabled = editor.isAdminRole || editor.isSaving;
                  const tooltip = editor.isAdminRole
                    ? "The Administrator role is grant-all and cannot be narrowed"
                    : editor.isSaving
                      ? "Saving…"
                      : inBundle
                        ? "Remove from the role bundle"
                        : "Add to the role bundle";
                  const ariaLabel =
                    `${ACTION_LABELS[action]} on ${ASSET_LABELS[asset]}: ` +
                    `${inBundle ? "in bundle" : "not in bundle"}` +
                    `${disabled ? "" : ". Activate to toggle."}`;
                  return (
                    <td key={asset} className={styles.cellWrap}>
                      <button
                        type="button"
                        className={styles.cell}
                        data-allowed={inBundle}
                        data-overlay={inBundle ? "grant" : "inherit"}
                        disabled={disabled}
                        title={tooltip}
                        aria-label={ariaLabel}
                        aria-pressed={inBundle}
                        onClick={() => editor.toggle(key)}
                      >
                        <span className={styles.effect}>
                          <span className={styles.effectIcon} aria-hidden="true">
                            {inBundle ? "✓" : "✕"}
                          </span>
                          {inBundle ? "In bundle" : "Excluded"}
                        </span>
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
