/**
 * AdminWorkspace — user administration + desk grouping (the `AuthService` admin
 * surface). The four sections — Users, Desks, Legal Entities, Netting Books —
 * are TAB panes (one visible at a time) using the app's in-workspace lens-tab
 * pattern (identical to the Book / Market-Data lens tabs), so each renders in
 * its own readable pane instead of one long scroll. Users lists every user
 * (email, name, role, capabilities, desk, status) with inline edit /
 * reset-password / permissions / delete + inline desk assignment; a desk is the
 * group a trader belongs to, and desk membership is what scopes a trader's view
 * of inbound RFQ traffic (the Increment-3 FIX-monitor desk scoping).
 *
 * Administration is admin-only server-side, so the workspace gates on the signed-
 * in identity: anonymous or trader sessions see a sign-in / insufficient-role
 * card instead of the tabs (the admin RPCs would be `permission_denied`). All
 * state is server-owned — the hook re-fetches after each mutation.
 */

import { useState } from "react";

import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { CapabilityMatrix } from "../components/CapabilityMatrix";
import { Panel } from "../components/Panel";
import { UserDialog, type UserDialogMode } from "../components/UserDialog";
import { BooksPanel, EntitiesPanel } from "./RegistryPanels";
import { AggregationPanel } from "./AggregationPanel";
import type { CapabilityAsset, DeskDesc, UserDesc, UserRole } from "../data/contract";
import { ASSET_LABELS, roleBaselineSummary } from "../lib/capabilityMatrix";
import { useAdmin } from "../hooks/useAdmin";
import { useFixConnections } from "../hooks/useFixConnections";
import styles from "./AdminWorkspace.module.css";

/**
 * The four administration sections. Each renders in its own tab pane so a single
 * readable section is visible at a time (previously all four stacked on one long
 * scrolling page). Users is the default.
 */
type AdminTab = "users" | "desks" | "entities" | "books" | "aggregation";
const ADMIN_TABS: readonly { readonly id: AdminTab; readonly label: string }[] = [
  { id: "users", label: "Users" },
  { id: "desks", label: "Desks" },
  { id: "entities", label: "Legal Entities" },
  { id: "books", label: "Netting Books" },
  { id: "aggregation", label: "Aggregation" },
];

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

/**
 * Map a desk-rename failure to a friendly, actionable line. The server rejects a
 * rename three ways — a name that collides with another desk (AlreadyExists), a
 * blank name (InvalidArgument), or an unknown desk (NotFound); each carries a
 * recognisable token in its message. Anything unrecognised falls through to the
 * raw server text (never swallowed).
 */
function friendlyDeskRenameError(message: string): string {
  const m = message.toLowerCase();
  if (m.includes("already") || m.includes("exists") || m.includes("duplicate")) {
    return "That name is already used by another desk — pick a different one.";
  }
  if (m.includes("required") || m.includes("blank") || m.includes("invalid")) {
    return "Enter a desk name.";
  }
  if (m.includes("not_found") || m.includes("no desk")) {
    return "That desk no longer exists — refresh the roster.";
  }
  return message;
}

/** The compact per-asset abbreviation for the capability chips. */
const ASSET_ABBR: Record<CapabilityAsset, string> = {
  fx_options: "FX",
  fixed_income: "FI",
};

/**
 * The role-baseline capability chips for a user row — e.g. "FX 10/10 · FI 10/10"
 * for an admin, "FX 9/10 · FI 9/10" for a trader. This is the honest role
 * baseline; the full overlay-adjusted set is edited via the Permissions button.
 */
function CapabilityChips({ role }: { role: UserRole }): React.ReactElement {
  const summary = roleBaselineSummary(role);
  return (
    <span className={styles.capSummary}>
      {summary.map((s) => (
        <span
          key={s.asset}
          className={styles.capChip}
          title={`${ASSET_LABELS[s.asset]}: ${s.allowed} of ${s.total} actions (role baseline)`}
        >
          {ASSET_ABBR[s.asset]} {s.allowed}/{s.total}
        </span>
      ))}
    </span>
  );
}

/**
 * The Users-table desk-membership cell: an "All desks" toggle over a per-desk
 * checkbox multi-select (membership is MANY-TO-MANY), committing each change
 * OPTIMISTICALLY through `onChange`. Shows membership at a glance — an "All desks"
 * chip, the assigned desk-name chips, or the deskless "receives no quotes" marker
 * (a deskless trader receives no inbound quotes or deals).
 */
function DeskMembershipCell({
  user,
  desks,
  onChange,
  error,
}: {
  user: UserDesc;
  desks: readonly DeskDesc[];
  onChange: (userId: string, deskIds: readonly string[], allDesks: boolean) => void;
  error?: string | undefined;
}): React.ReactElement {
  const nameOf = (id: string): string => desks.find((d) => d.id === id)?.name ?? id;
  const toggleDesk = (deskId: string, on: boolean): void => {
    const next = on ? [...user.deskIds, deskId] : user.deskIds.filter((d) => d !== deskId);
    onChange(user.id, next, false);
  };
  return (
    <div className={styles.deskCell}>
      <label className={styles.allDesksToggle}>
        <input
          type="checkbox"
          checked={user.allDesks}
          aria-label={`All desks for ${user.email}`}
          onChange={(e) => onChange(user.id, [], e.target.checked)}
        />
        <span>All desks</span>
      </label>

      {!user.allDesks && desks.length > 0 && (
        <div
          className={styles.deskChecks}
          role="group"
          aria-label={`Desk membership for ${user.email}`}
        >
          {desks.map((d) => (
            <label key={d.id} className={styles.deskCheck}>
              <input
                type="checkbox"
                checked={user.deskIds.includes(d.id)}
                onChange={(e) => toggleDesk(d.id, e.target.checked)}
              />
              <span>{d.name}</span>
            </label>
          ))}
        </div>
      )}

      <div className={styles.deskSummary}>
        {user.allDesks ? (
          <span className={styles.deskChip} title="Belongs to every desk.">
            All desks
          </span>
        ) : user.deskIds.length > 0 ? (
          user.deskIds.map((id) => (
            <span key={id} className={styles.deskChip}>
              {nameOf(id)}
            </span>
          ))
        ) : (
          <span
            className={styles.unassignedFlag}
            title="A deskless trader receives no inbound quotes or deals."
          >
            receives no quotes
          </span>
        )}
      </div>

      {error && (
        <p className={styles.rowError} role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

export function AdminWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const admin = useAdmin(app.transport, auth.isAdmin);
  // The managed FIX-connection registry feeds the Aggregation editor's member
  // candidates (an aggregated book's members are FIX acceptors + LP-feed ids).
  const fix = useFixConnections(app.transport);

  const [tab, setTab] = useState<AdminTab>("users");
  const [dialog, setDialog] = useState<{ mode: UserDialogMode; user?: UserDesc } | null>(null);
  const [deskName, setDeskName] = useState("");
  const [actionError, setActionError] = useState<string | null>(null);
  const [capUserId, setCapUserId] = useState<string | null>(null);
  // Per-row desk-assignment errors, keyed by user id (cleared at each attempt).
  const [deskErrors, setDeskErrors] = useState<Record<string, string>>({});
  // Inline desk-rename state: the desk being edited (its id), its draft label, and
  // per-desk rename errors keyed by desk id. The id is the immutable routing key —
  // only the label changes.
  const [renameDeskId, setRenameDeskId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [renameErrors, setRenameErrors] = useState<Record<string, string>>({});

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

  // Inline desk membership change — optimistic via the hook; a failure surfaces as
  // a per-row inline error and the roster rolls back. Many-to-many: an "All desks"
  // toggle plus a per-desk multi-select; an empty set (not all) ⇒ deskless.
  const changeUserDesks = async (
    userId: string,
    deskIds: readonly string[],
    allDesks: boolean,
  ): Promise<void> => {
    setDeskErrors((prev) => {
      const { [userId]: _cleared, ...rest } = prev;
      return rest;
    });
    try {
      await admin.setUserDesks(userId, deskIds, allDesks);
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : "desk assignment failed";
      setDeskErrors((prev) => ({ ...prev, [userId]: message }));
    }
  };

  // --- inline desk rename --------------------------------------------------
  const beginRename = (d: DeskDesc): void => {
    setRenameDeskId(d.id);
    setRenameDraft(d.name);
    setRenameErrors((prev) => {
      const { [d.id]: _cleared, ...rest } = prev;
      return rest;
    });
  };

  const cancelRename = (): void => {
    setRenameDeskId(null);
    setRenameDraft("");
  };

  // Commit a rename OPTIMISTICALLY via the hook; a failure surfaces as a per-row
  // inline error and the roster rolls back. The id (routing key) is immutable.
  const commitRename = async (id: string): Promise<void> => {
    const next = renameDraft.trim();
    if (next.length === 0) {
      setRenameErrors((prev) => ({ ...prev, [id]: "Enter a desk name." }));
      return;
    }
    setRenameErrors((prev) => {
      const { [id]: _cleared, ...rest } = prev;
      return rest;
    });
    try {
      await admin.updateDesk(id, next);
      setRenameDeskId(null);
      setRenameDraft("");
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : "rename failed";
      setRenameErrors((prev) => ({ ...prev, [id]: friendlyDeskRenameError(message) }));
    }
  };

  const deskName_ = deskName.trim();
  const capUser = capUserId ? (admin.users.find((u) => u.id === capUserId) ?? null) : null;
  // A user is a member if they belong to every desk (`allDesks`) or name this desk.
  const memberCount = (deskId: string): number =>
    admin.users.filter((u) => u.allDesks || u.deskIds.includes(deskId)).length;

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

  // --- Users pane: roster table + inline desk picker + capability matrix ----
  const usersPane = (
    <>
      <Panel title="Users" glyph="⚇" actions={usersActions}>
        {admin.error && <p className={styles.banner}>{admin.error}</p>}
        {actionError && <p className={styles.banner}>{actionError}</p>}
        <p className={styles.hint}>
          A trader receives quotes and executed deals for any desk they belong to. Toggle
          <strong> All desks</strong> or pick one or more desks below to permission what a
          user sees; a deskless trader receives nothing.
        </p>
        {admin.users.length === 0 ? (
          <p className={styles.empty}>No users.</p>
        ) : (
          <table className={`${styles.table} ${styles.usersTable}`}>
            <thead>
              <tr>
                <th>Email</th>
                <th>Name</th>
                <th>Role</th>
                <th>Capabilities (role baseline)</th>
                <th>Desks</th>
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
                  <td>
                    <CapabilityChips role={u.role} />
                  </td>
                  <td>
                    <DeskMembershipCell
                      user={u}
                      desks={admin.desks}
                      onChange={(id, deskIds, allDesks) =>
                        void changeUserDesks(id, deskIds, allDesks)
                      }
                      error={deskErrors[u.id]}
                    />
                  </td>
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
    </>
  );

  // --- Desks pane: roster + create/rename/delete ----------------------------
  const desksPane = (
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
            {admin.desks.map((d: DeskDesc) => {
              const editing = renameDeskId === d.id;
              return (
                <tr key={d.id}>
                  <td className={styles.mono}>{d.id}</td>
                  <td className={styles.nameCell}>
                    {editing ? (
                      <form
                        className={styles.renameForm}
                        onSubmit={(e) => {
                          e.preventDefault();
                          void commitRename(d.id);
                        }}
                      >
                        <input
                          className={styles.deskInput}
                          type="text"
                          value={renameDraft}
                          onChange={(e) => setRenameDraft(e.target.value)}
                          aria-label={`Rename desk ${d.name}`}
                          autoFocus
                        />
                        <Button type="submit" variant="primary" disabled={renameDraft.trim().length === 0}>
                          Save
                        </Button>
                        <Button type="button" variant="ghost" onClick={cancelRename}>
                          Cancel
                        </Button>
                      </form>
                    ) : (
                      d.name
                    )}
                    {renameErrors[d.id] && (
                      <p className={styles.rowError} role="alert">
                        {renameErrors[d.id]}
                      </p>
                    )}
                  </td>
                  <td className={styles.mono}>{memberCount(d.id)}</td>
                  <td className={styles.actionsCol}>
                    <div className={styles.rowActions}>
                      {!editing && (
                        <Button variant="secondary" onClick={() => beginRename(d)}>
                          Rename
                        </Button>
                      )}
                      <Button variant="ghost" onClick={() => void runAction(() => admin.deleteDesk(d.id))}>
                        Delete
                      </Button>
                    </div>
                  </td>
                </tr>
              );
            })}
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
  );

  return (
    <div className={styles.root}>
      <div className={styles.tabBar} role="tablist" aria-label="administration sections">
        {ADMIN_TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            id={`admin-tab-${t.id}`}
            aria-selected={tab === t.id}
            aria-controls={`admin-panel-${t.id}`}
            className={`${styles.tab} ${tab === t.id ? styles.tabActive : ""}`}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>

      <div
        className={styles.tabBody}
        role="tabpanel"
        id={`admin-panel-${tab}`}
        aria-labelledby={`admin-tab-${tab}`}
      >
        {tab === "users" && usersPane}
        {tab === "desks" && desksPane}
        {tab === "entities" && (
          <EntitiesPanel
            entities={admin.entities}
            books={admin.books}
            onCreate={admin.createEntity}
            onUpdate={admin.updateEntity}
            onDelete={admin.deleteEntity}
            run={runAction}
          />
        )}
        {tab === "books" && (
          <BooksPanel
            entities={admin.entities}
            books={admin.books}
            onCreate={admin.createBook}
            onUpdate={admin.updateBook}
            onDelete={admin.deleteBook}
            run={runAction}
          />
        )}
        {tab === "aggregation" && (
          <AggregationPanel
            books={admin.aggregatedBooks}
            connections={fix.connections}
            onCreate={admin.createAggregatedBook}
            onUpdate={admin.updateAggregatedBook}
            onDelete={admin.deleteAggregatedBook}
            run={runAction}
          />
        )}
      </div>

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
