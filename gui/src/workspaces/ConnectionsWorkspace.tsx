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

import { useEffect, useState } from "react";

import { useApp } from "../app/AppContext";
import { FixConnectionWizard } from "../components/FixConnectionWizard";
import { FixSessionMonitor } from "../components/FixSessionMonitor";
import { FixSpecModal } from "../components/FixSpecModal";
import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import type { DeskDesc, FixConnection } from "../data/contract";
import { useFixConnections } from "../hooks/useFixConnections";
import {
  buildFixClientConfig,
  downloadText,
  fixClientConfigFilename,
  FIX_DICTIONARY_URL,
} from "../lib/fixClientConfig";
import styles from "./ConnectionsWorkspace.module.css";

/** The display label for a connection's dialect. */
function kindLabel(c: FixConnection): string {
  switch (c.kind) {
    case "OPTIONS":
      return "Options";
    case "FIXED_INCOME_QUOTE":
      return "Fixed Income — Quote (RFQ)";
    case "FIXED_INCOME_STREAM":
      return "Fixed Income — Streaming (RFS)";
  }
}

export function ConnectionsWorkspace(): React.ReactElement {
  const app = useApp();
  const fix = useFixConnections(app.transport);
  const [wizardOpen, setWizardOpen] = useState(false);
  const [specOpen, setSpecOpen] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [monitorId, setMonitorId] = useState<string | null>(null);
  // The desks a new connection may be assigned to (every connection belongs to a
  // desk). Loaded for the wizard's desk picker; desk admin is admin-gated, so a
  // non-admin simply sees an empty roster (and the server rejects a deskless
  // create anyway). Refreshed when the wizard opens so a desk just created in the
  // Admin workspace is selectable.
  const [desks, setDesks] = useState<DeskDesc[]>([]);
  useEffect(() => {
    let cancelled = false;
    void app.transport
      .listDesks()
      .then((d) => {
        if (!cancelled) setDesks(d);
      })
      .catch(() => {
        if (!cancelled) setDesks([]);
      });
    return () => {
      cancelled = true;
    };
  }, [app.transport, wizardOpen]);

  // The monitored connection, resolved live so it reflects status/rename changes
  // and clears itself if the connection is deleted out from under the monitor.
  const monitored = monitorId === null ? null : (fix.connections.find((c) => c.id === monitorId) ?? null);

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

  // When the acceptor list is empty (and settled), the empty-state card below owns
  // the primary "New connection" call-to-action, so the header omits its own to
  // keep EXACTLY ONE "New connection" affordance on screen (a second identically
  // named button is both an a11y ambiguity and a strict-locator hazard).
  const showEmpty = fix.connections.length === 0 && !fix.isLoading;

  const actions = (
    <div className={styles.headActions}>
      <a
        className={styles.specLink}
        href={FIX_DICTIONARY_URL}
        download
        title="Download the QuickFIX FIX 4.4 data dictionary for building a client"
      >
        ⤓ FIX dictionary
      </a>
      <button
        type="button"
        className={styles.specLink}
        onClick={() => setSpecOpen(true)}
        title="Open the FIX API guide (message flow + how to connect)"
      >
        ⤓ API guide
      </button>
      <Button variant="ghost" onClick={() => void fix.refetch()} disabled={fix.isLoading}>
        Refresh
      </Button>
      {!showEmpty && (
        <Button variant="primary" onClick={() => setWizardOpen(true)}>
          New connection
        </Button>
      )}
    </div>
  );

  /** Download a ready-to-use QuickFIX client config for one acceptor. */
  const downloadClientConfig = (c: FixConnection): void => {
    downloadText(fixClientConfigFilename(c), buildFixClientConfig(c), "text/plain");
  };

  return (
    <div className={styles.root}>
      <Panel title="Inbound FIX connections" glyph="⇄" actions={actions}>
        {fix.error && <p className={styles.banner}>{fix.error}</p>}
        {actionError && <p className={styles.banner}>{actionError}</p>}

        {showEmpty ? (
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
                <th>Desk</th>
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
                  <td className={styles.mono}>{c.desk || "—"}</td>
                  <td className={styles.mono}>{c.senderCompId}</td>
                  <td className={styles.mono}>{c.targetCompId}</td>
                  <td className={styles.actionsCol}>
                    <div className={styles.rowActions}>
                      <Button
                        variant={monitorId === c.id ? "primary" : "secondary"}
                        onClick={() => setMonitorId((id) => (id === c.id ? null : c.id))}
                        aria-pressed={monitorId === c.id}
                      >
                        Monitor
                      </Button>
                      <Button
                        variant="secondary"
                        onClick={() => downloadClientConfig(c)}
                        title="Download a QuickFIX client config to connect to this acceptor"
                      >
                        Client config
                      </Button>
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

      {monitored && (
        <FixSessionMonitor
          transport={app.transport}
          connection={monitored}
          onClose={() => setMonitorId(null)}
        />
      )}

      <FixConnectionWizard
        open={wizardOpen}
        onClose={() => setWizardOpen(false)}
        onCreate={fix.create}
        existing={fix.connections}
        desks={desks}
        can={app.auth.can}
      />

      <FixSpecModal open={specOpen} onClose={() => setSpecOpen(false)} />
    </div>
  );
}
