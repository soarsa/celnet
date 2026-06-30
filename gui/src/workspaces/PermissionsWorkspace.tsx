/**
 * PermissionsWorkspace — the first-class Administration page for controlling each
 * user's component access. A standalone rail workspace (not a panel that only
 * appears after selecting a user in the Admin table — that was the discoverability
 * problem this fixes): a left user picker, and on the right the
 * {@link ComponentAccessGrid} for the selected user (Read/Write toggle widgets per
 * component, with an advanced per-capability disclosure).
 *
 * Admin-only, server-side and client-side: the Administration domain tab is
 * already admin-gated, and this renders the SAME sign-in / insufficient-role card
 * the Admin workspace uses when `!auth.isAdmin` (the underlying capability RPCs
 * would be `permission_denied`). The roster comes from {@link useAdmin}; editing
 * is a per-user overlay on top of the server's role model (we never edit role
 * bundles — there is no server support for that).
 */

import { useEffect, useState } from "react";

import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { ComponentAccessGrid } from "../components/ComponentAccessGrid";
import { RoleCapabilityEditor } from "../components/RoleCapabilityEditor";
import { Panel } from "../components/Panel";
import type { UserDesc } from "../data/contract";
import { useAdmin } from "../hooks/useAdmin";
import styles from "./PermissionsWorkspace.module.css";

/** The role chip for a user in the picker. */
function RoleChip({ user }: { user: UserDesc }): React.ReactElement {
  if (user.disabled) {
    return <span className={`${styles.chip} ${styles.chipDisabled}`}>Disabled</span>;
  }
  const isAdmin = user.role === "ADMIN";
  return (
    <span className={`${styles.chip} ${isAdmin ? styles.chipAdmin : styles.chipTrader}`}>
      {isAdmin ? "Admin" : "Trader"}
    </span>
  );
}

export function PermissionsWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const admin = useAdmin(app.transport, auth.isAdmin);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // Every pane stays mounted (Shell toggles visibility), so this workspace's
  // roster loads once at sign-in — before a user created later in the Admin page
  // exists. Re-fetch whenever the Permissions page becomes the active workspace so
  // the picker always reflects the current server roster without a manual refresh.
  const { refetch } = admin;
  const isActive = app.workspace === "permissions";
  useEffect(() => {
    if (isActive && auth.isAdmin) void refetch();
  }, [isActive, auth.isAdmin, refetch]);

  // --- the sign-in / insufficient-role gate (mirrors AdminWorkspace) --------
  if (!auth.isAdmin) {
    return (
      <div className={styles.root}>
        <div className={styles.gate}>
          <h2 className={styles.gateTitle}>Permissions</h2>
          {auth.user ? (
            <>
              <p className={styles.gateHint}>
                Signed in as <strong>{auth.user.email}</strong> (Trader). Managing user permissions
                requires an administrator account.
              </p>
              <Button variant="secondary" onClick={() => void auth.logout()} disabled={auth.busy}>
                Sign out
              </Button>
            </>
          ) : (
            <>
              <p className={styles.gateHint}>
                Sign in with an administrator account to manage each user&apos;s access to every
                component. The default administrator is <strong>admin@celnet.com</strong>.
              </p>
              <Button variant="primary" onClick={() => app.setSignInOpen(true)}>
                Sign in
              </Button>
            </>
          )}
        </div>
      </div>
    );
  }

  const selected = selectedId ? (admin.users.find((u) => u.id === selectedId) ?? null) : null;

  const pickerActions = (
    <Button variant="ghost" onClick={() => void admin.refetch()} disabled={admin.isLoading}>
      Refresh
    </Button>
  );

  return (
    <div className={styles.root}>
      <div className={styles.layout}>
        <Panel title="Users" glyph="⚷" actions={pickerActions} className={styles.pickerPanel}>
          {admin.error && <p className={styles.banner}>{admin.error}</p>}
          {admin.users.length === 0 ? (
            <p className={styles.empty}>No users.</p>
          ) : (
            <ul className={styles.userList} aria-label="select a user to edit permissions">
              {admin.users.map((u) => {
                const active = u.id === selectedId;
                return (
                  <li key={u.id}>
                    <button
                      type="button"
                      className={`${styles.userBtn} ${active ? styles.userActive : ""}`}
                      aria-pressed={active}
                      onClick={() => setSelectedId(u.id)}
                    >
                      <span className={styles.userMain}>
                        <span className={styles.userEmail}>{u.email}</span>
                        <span className={styles.userName}>{u.displayName}</span>
                      </span>
                      <RoleChip user={u} />
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </Panel>

        {/*
         * Both editor panels live in a single right-hand column so the sticky user
         * picker (column 1) has NOTHING beneath it to overlap. Previously the
         * Role-bundles panel auto-placed into column 1 / row 2, directly under the
         * sticky picker; when the page scrolled, the role grid's cells painted over
         * the picker's user buttons and intercepted clicks on them.
         */}
        <div className={styles.content}>
          <Panel title="Component access" glyph="⚷" className={styles.gridPanel}>
            {selected ? (
              <ComponentAccessGrid
                key={selected.id}
                user={selected}
                transport={app.transport}
                signedInUserId={auth.user?.id}
              />
            ) : (
              <p className={styles.placeholder}>
                Select a user on the left to view and edit their access to each component.
              </p>
            )}
          </Panel>

          <Panel title="Role bundles" glyph="⚷" className={styles.gridPanel}>
            <RoleCapabilityEditor transport={app.transport} />
          </Panel>
        </div>
      </div>
    </div>
  );
}
