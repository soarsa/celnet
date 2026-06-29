/**
 * AdminWorkspace — user administration + desk grouping (the `AuthService` admin
 * surface). Lists every user (email, name, role, desk, status) with inline
 * edit / reset-password / delete, and every desk with create / delete; a desk is
 * the group a trader belongs to, and desk membership is what scopes a trader's
 * view of inbound RFQ traffic (the Increment-3 FIX-monitor desk scoping).
 *
 * Administration is admin-only server-side, so the workspace gates on the signed-
 * in identity: anonymous or trader sessions see a sign-in / insufficient-role
 * card instead of the tables (the admin RPCs would be `permission_denied`). All
 * state is server-owned — the hook re-fetches after each mutation.
 */

import { useState } from "react";

import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { CapabilityMatrix } from "../components/CapabilityMatrix";
import { Panel } from "../components/Panel";
import { UserDialog, type UserDialogMode } from "../components/UserDialog";
import { BooksPanel, EntitiesPanel } from "./RegistryPanels";
import type { DeskDesc, UserDesc } from "../data/contract";
import { useAdmin } from "../hooks/useAdmin";
import styles from "./AdminWorkspace.module.css";

/** The role badge for a user row. */
function RoleBadge({ user }: { user: UserDesc }): React.ReactElement {
  if (user.disabled) {
    return <span className={`${styles.badge} ${styles.badgeDisabled}`}>Disabled</span>;
  }
  const isAdmin = user.role === "ADMIN";
  return (
    <span className={`${styles.badge} ${isAdmin ? styles.badgeAdmin : styles.badgeTrader}`}>
      {isAdmin ? "Admin" : "Trader"}
    </span>
  );
}

export function AdminWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const admin = useAdmin(app.transport, auth.isAdmin);

  const [dialog, setDialog] = useState<{ mode: UserDialogMode; user?: UserDesc } | null>(null);
  const [deskName, setDeskName] = useState("");
  const [actionError, setActionError] = useState<string | null>(null);
  const [capUserId, setCapUserId] = useState<string | null>(null);

  // --- the sign-in / insufficient-role gate --------------------------------
  if (!auth.isAdmin) {
    return (
      <div className={styles.root}>
        <div className={styles.gate}>
          <h2 className={styles.gateTitle}>Administration</h2>
          {auth.user ? (
            <>
              <p className={styles.gateHint}>
                Signed in as <strong>{auth.user.email}</strong> (Trader). Managing users and desks
                requires an administrator account.
              </p>
              <Button variant="secondary" onClick={() => void auth.logout()} disabled={auth.busy}>
                Sign out
              </Button>
            </>
          ) : (
            <>
              <p className={styles.gateHint}>
                Sign in with an administrator account to manage users, desks, password resets, and
                FIX-connection ownership. The default administrator is{" "}
                <strong>admin@celnet.com</strong>.
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

  const runAction = async (action: () => Promise<unknown>): Promise<void> => {
    setActionError(null);
    try {
      await action();
    } catch (e: unknown) {
      setActionError(e instanceof Error ? e.message : "action failed");
    }
  };

  const deskName_ = deskName.trim();
  const capUser = capUserId ? (admin.users.find((u) => u.id === capUserId) ?? null) : null;
  const memberCount = (deskId: string): number =>
    admin.users.filter((u) => u.deskId === deskId).length;
  const deskLabel = (deskId: string | undefined): string => {
    if (!deskId) return "—";
    const desk = admin.desks.find((d) => d.id === deskId);
    return desk ? desk.name : deskId;
  };

  const usersActions = (
    <div className={styles.headActions}>
      <Button variant="ghost" onClick={() => void admin.refetch()} disabled={admin.isLoading}>
        Refresh
      </Button>
      <Button variant="primary" onClick={() => setDialog({ mode: "create" })}>
        New user
      </Button>
    </div>
  );

  const submitDesk = (e: React.FormEvent<HTMLFormElement>): void => {
    e.preventDefault();
    if (deskName_.length === 0) return;
    void runAction(async () => {
      await admin.createDesk(deskName_);
      setDeskName("");
    });
  };

  return (
    <div className={styles.root}>
      <Panel title="Users" glyph="⚇" actions={usersActions}>
        {admin.error && <p className={styles.banner}>{admin.error}</p>}
        {actionError && <p className={styles.banner}>{actionError}</p>}
        {admin.users.length === 0 ? (
          <p className={styles.empty}>No users.</p>
        ) : (
          <table className={styles.table}>
            <thead>
              <tr>
                <th>Email</th>
                <th>Name</th>
                <th>Role</th>
                <th>Desk</th>
                <th className={styles.actionsCol}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {admin.users.map((u) => (
                <tr key={u.id}>
                  <td className={styles.mono}>{u.email}</td>
                  <td className={styles.nameCell}>{u.displayName}</td>
                  <td>
                    <RoleBadge user={u} />
                  </td>
                  <td className={styles.mono}>{deskLabel(u.deskId)}</td>
                  <td className={styles.actionsCol}>
                    <div className={styles.rowActions}>
                      <Button variant="secondary" onClick={() => setDialog({ mode: "edit", user: u })}>
                        Edit
                      </Button>
                      <Button variant="secondary" onClick={() => setDialog({ mode: "reset", user: u })}>
                        Reset password
                      </Button>
                      <Button
                        variant="secondary"
                        onClick={() => setCapUserId((id) => (id === u.id ? null : u.id))}
                        aria-pressed={capUserId === u.id}
                      >
                        Permissions
                      </Button>
                      <Button variant="ghost" onClick={() => void runAction(() => admin.deleteUser(u.id))}>
                        Delete
                      </Button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Panel>

      {capUser && (
        <Panel
          title="Permissions"
          glyph="⚿"
          actions={
            <Button variant="ghost" onClick={() => setCapUserId(null)}>
              Close
            </Button>
          }
        >
          <CapabilityMatrix
            key={capUser.id}
            user={capUser}
            transport={app.transport}
            signedInUserId={auth.user?.id}
          />
        </Panel>
      )}

      <Panel title="Desks" glyph="▦">
        {admin.desks.length === 0 ? (
          <p className={styles.empty}>
            No desks yet. A desk groups traders so they see their desk&apos;s inbound RFQs.
          </p>
        ) : (
          <table className={styles.table}>
            <thead>
              <tr>
                <th>Id</th>
                <th>Name</th>
                <th>Members</th>
                <th className={styles.actionsCol}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {admin.desks.map((d: DeskDesc) => (
                <tr key={d.id}>
                  <td className={styles.mono}>{d.id}</td>
                  <td className={styles.nameCell}>{d.name}</td>
                  <td className={styles.mono}>{memberCount(d.id)}</td>
                  <td className={styles.actionsCol}>
                    <div className={styles.rowActions}>
                      <Button variant="ghost" onClick={() => void runAction(() => admin.deleteDesk(d.id))}>
                        Delete
                      </Button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <form className={styles.deskForm} onSubmit={submitDesk}>
          <input
            className={styles.deskInput}
            type="text"
            value={deskName}
            onChange={(e) => setDeskName(e.target.value)}
            placeholder="New desk name, e.g. G10 Options"
            aria-label="new desk name"
          />
          <Button type="submit" variant="secondary" disabled={deskName_.length === 0}>
            Add desk
          </Button>
        </form>
      </Panel>

      <EntitiesPanel
        entities={admin.entities}
        books={admin.books}
        onCreate={admin.createEntity}
        onUpdate={admin.updateEntity}
        onDelete={admin.deleteEntity}
        run={runAction}
      />

      <BooksPanel
        entities={admin.entities}
        books={admin.books}
        onCreate={admin.createBook}
        onUpdate={admin.updateBook}
        onDelete={admin.deleteBook}
        run={runAction}
      />

      <UserDialog
        open={dialog !== null}
        mode={dialog?.mode ?? "create"}
        user={dialog?.user}
        desks={admin.desks}
        onClose={() => setDialog(null)}
        onCreate={admin.createUser}
        onUpdate={admin.updateUser}
        onReset={admin.resetPassword}
      />
    </div>
  );
}
