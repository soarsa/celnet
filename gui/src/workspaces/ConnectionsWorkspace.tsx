/**
 * ConnectionsWorkspace — manage the inbound FIX acceptor connections the edge
 * binds (the `FixAdminService` admin surface).
 *
 * Lists every managed acceptor with its live `running` status and bound address,
 * lets the operator enable/disable or delete one inline, and opens the
 * {@link FixConnectionWizard} to define a new one. All state is server-owned: the
 * hook re-fetches after each mutation so the table reflects the authoritative set
 * (and the persisted definitions auto-load on the next edge restart).
 *
 * v1 manages Options acceptors end-to-end; SPOT FX is a later phase (the wizard
 * shows it disabled), but the table is already kind-agnostic.
 */

import { useState } from "react";

import { useApp } from "../app/AppContext";
import { FixConnectionWizard } from "../components/FixConnectionWizard";
import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import type { FixConnection } from "../data/contract";
import { useFixConnections } from "../hooks/useFixConnections";
import styles from "./ConnectionsWorkspace.module.css";

/** The display label for a connection's dialect. */
function kindLabel(c: FixConnection): string {
  return c.kind === "OPTIONS" ? "Options" : c.kind;
}

export function ConnectionsWorkspace(): React.ReactElement {
  const app = useApp();
  const fix = useFixConnections(app.transport);
  const [wizardOpen, setWizardOpen] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const runAction = async (id: string, action: () => Promise<unknown>): Promise<void> => {
    setBusyId(id);
    setActionError(null);
    try {
      await action();
    } catch (e: unknown) {
      setActionError(e instanceof Error ? e.message : "action failed");
    } finally {
      setBusyId(null);
    }
  };

  const actions = (
    <div className={styles.headActions}>
      <Button variant="ghost" onClick={() => void fix.refetch()} disabled={fix.isLoading}>
        Refresh
      </Button>
      <Button variant="primary" onClick={() => setWizardOpen(true)}>
        New connection
      </Button>
    </div>
  );

  return (
    <div className={styles.root}>
      <Panel title="Inbound FIX connections" glyph="⇄" actions={actions}>
        {fix.error && <p className={styles.banner}>{fix.error}</p>}
        {actionError && <p className={styles.banner}>{actionError}</p>}

        {fix.connections.length === 0 && !fix.isLoading ? (
          <div className={styles.empty}>
            <p className={styles.emptyTitle}>No inbound acceptors defined</p>
            <p className={styles.emptyHint}>
              Define an Options acceptor to receive FIX RFQs. It persists and
              auto-loads on every edge restart.
            </p>
            <Button variant="primary" onClick={() => setWizardOpen(true)}>
              New connection
            </Button>
          </div>
        ) : (
          <table className={styles.table}>
            <thead>
              <tr>
                <th>Status</th>
                <th>Name</th>
                <th>Dialect</th>
                <th>Bind address</th>
                <th>SenderCompID</th>
                <th>TargetCompID</th>
                <th className={styles.actionsCol}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {fix.connections.map((c) => (
                <tr key={c.id}>
                  <td>
                    <span
                      className={[styles.badge, c.running ? styles.badgeOn : styles.badgeOff].join(
                        " ",
                      )}
                      role="img"
                      aria-label={c.running ? "running" : "stopped"}
                      title={c.running ? `running on ${c.boundAddr}` : "stopped"}
                    >
                      {c.running ? "◉ running" : "○ stopped"}
                    </span>
                  </td>
                  <td className={styles.nameCell}>{c.name}</td>
                  <td>{kindLabel(c)}</td>
                  <td className={styles.mono}>{c.running && c.boundAddr ? c.boundAddr : c.bindAddr}</td>
                  <td className={styles.mono}>{c.senderCompId}</td>
                  <td className={styles.mono}>{c.targetCompId}</td>
                  <td className={styles.actionsCol}>
                    <div className={styles.rowActions}>
                      <Button
                        variant="secondary"
                        onClick={() =>
                          void runAction(c.id, () => fix.setEnabled(c.id, !c.enabled))
                        }
                        disabled={busyId === c.id}
                      >
                        {c.enabled ? "Disable" : "Enable"}
                      </Button>
                      <Button
                        variant="ghost"
                        onClick={() => void runAction(c.id, () => fix.remove(c.id))}
                        disabled={busyId === c.id}
                      >
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

      <FixConnectionWizard
        open={wizardOpen}
        onClose={() => setWizardOpen(false)}
        onCreate={fix.create}
        existing={fix.connections}
      />
    </div>
  );
}
