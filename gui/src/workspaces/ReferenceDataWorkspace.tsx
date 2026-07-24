/**
 * ReferenceDataWorkspace — the instrument reference-data registry (the
 * `AuthService` instrument surface). Lists every instrument DEFINITION (its
 * family, currency, conventions and external identifiers) for ANY authenticated
 * user; create / edit / delete are administrator-only (server-enforced), so those
 * controls are gated PER-CONTROL — every signed-in user sees the list, only an
 * admin sees the form and the row actions.
 *
 * Unlike the Administration workspace this is NOT hard-gated to admins: a trader
 * reaches it and reads the registry (it populates rates/credit pickers and
 * reference views). State is server-owned — the data hook re-fetches after each
 * mutation.
 */

import { useState } from "react";

import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { InstrumentDialog } from "../components/InstrumentDialog";
import { Panel } from "../components/Panel";
import { INSTRUMENT_FAMILY_LABELS, type InstrumentDef } from "../data/contract";
import { useReferenceData } from "../hooks/useReferenceData";
import admin from "./AdminWorkspace.module.css";
import styles from "./ReferenceDataWorkspace.module.css";

/** The external-identifier chips for a row. */
function ExternalIds({ def }: { def: InstrumentDef }): React.ReactElement {
  if (def.externalIds.length === 0) {
    return <span className={admin.mono}>—</span>;
  }
  return (
    <span className={styles.extIds}>
      {def.externalIds.map((id, i) => (
        <span key={`${id.scheme}-${i}`} className={styles.extIdChip}>
          <span className={styles.extIdScheme}>{id.scheme}</span>
          {id.value}
        </span>
      ))}
    </span>
  );
}

export function ReferenceDataWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const isAuthed = auth.user !== null;
  const isAdmin = auth.isAdmin;
  const data = useReferenceData(app.transport, isAuthed);

  const [editing, setEditing] = useState<InstrumentDef | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const runAction = async (action: () => Promise<unknown>): Promise<void> => {
    setActionError(null);
    try {
      await action();
    } catch (e: unknown) {
      setActionError(e instanceof Error ? e.message : "action failed");
    }
  };

  // --- the sign-in gate (anonymous only; traders + admins see the list) -----
  if (!isAuthed) {
    return (
      <div className={admin.root}>
        <div className={admin.gate}>
          <h2 className={admin.gateTitle}>Reference Data</h2>
          <p className={admin.gateHint}>
            Sign in to browse the instrument reference-data registry. Listing is open to any
            signed-in user; creating and editing definitions requires an administrator account.
          </p>
          <Button variant="primary" onClick={() => app.setSignInOpen(true)}>
            Sign in
          </Button>
        </div>
      </div>
    );
  }

  const openCreate = (): void => {
    setEditing(null);
    setShowCreate(true);
    setActionError(null);
  };
  const openEdit = (def: InstrumentDef): void => {
    setEditing(def);
    setShowCreate(false);
    setActionError(null);
  };
  const closeForm = (): void => {
    setEditing(null);
    setShowCreate(false);
  };

  const headActions = (
    <div className={admin.headActions}>
      <Button variant="ghost" onClick={() => void data.refetch()} disabled={data.isLoading}>
        Refresh
      </Button>
      {isAdmin && (
        <Button variant="primary" onClick={openCreate}>
          New instrument
        </Button>
      )}
    </div>
  );

  const formOpen = isAdmin && (showCreate || editing !== null);

  return (
    <div className={admin.root}>
      <Panel title="Instruments" glyph="❏" actions={headActions}>
        {data.error && <p className={admin.banner}>{data.error}</p>}
        {actionError && <p className={admin.banner}>{actionError}</p>}
        {!isAdmin && (
          <p className={styles.adminNote}>
            You can browse the registry. Creating, editing and deleting instrument definitions
            requires an administrator account.
          </p>
        )}
        {data.instruments.length === 0 ? (
          <p className={admin.empty}>No instrument definitions yet.</p>
        ) : (
          <table className={admin.table}>
            <thead>
              <tr>
                <th scope="col">Id</th>
                <th scope="col">Name</th>
                <th scope="col">Currency</th>
                <th scope="col">Family</th>
                <th scope="col">External ids</th>
                {isAdmin && (
                  <th scope="col" className={admin.actionsCol}>
                    Actions
                  </th>
                )}
              </tr>
            </thead>
            <tbody>
              {data.instruments.map((def) => (
                <tr key={def.instrumentId}>
                  <td className={admin.mono}>{def.instrumentId}</td>
                  <td className={admin.nameCell}>{def.name}</td>
                  <td className={admin.mono}>{def.currency}</td>
                  <td>
                    <span className={styles.familyTag}>{INSTRUMENT_FAMILY_LABELS[def.family]}</span>
                  </td>
                  <td>
                    <ExternalIds def={def} />
                  </td>
                  {isAdmin && (
                    <td className={admin.actionsCol}>
                      <div className={admin.rowActions}>
                        <Button variant="secondary" onClick={() => openEdit(def)}>
                          Edit
                        </Button>
                        <Button
                          variant="ghost"
                          onClick={() => void runAction(() => data.deleteInstrument(def.instrumentId))}
                        >
                          Delete
                        </Button>
                      </div>
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Panel>

      <InstrumentDialog
        open={formOpen}
        editing={editing}
        error={actionError}
        onClose={closeForm}
        onCreate={data.createInstrument}
        onUpdate={data.updateInstrument}
        run={runAction}
      />
    </div>
  );
}
