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

import { useMemo, useState } from "react";

import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { DataTable } from "../components/DataTable";
import { InstrumentDialog } from "../components/InstrumentDialog";
import { Panel } from "../components/Panel";
import { TableSearch } from "../components/TableSearch";
import { INSTRUMENT_FAMILY_LABELS, type InstrumentDef } from "../data/contract";
import { useGridState } from "../hooks/useGridState";
import { useReferenceData } from "../hooks/useReferenceData";
import type { ColumnDef } from "../lib/grid";
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
  // Editing the reference-data registry is DELEGABLE off the coarse admin flag: an admin
  // OR a holder of `refdata·FI` (Action::Refdata — the reference-data/master-data steward)
  // may create/edit/delete definitions. Listing stays open to any signed-in user.
  const canEdit = auth.isAdmin || auth.can("refdata", "fixed_income");
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
            signed-in user; creating and editing definitions requires reference-data permission.
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

  // The registry is the one screen that answers "does this instrument exist, and how is
  // it set up" — which is a lookup, not a browse. Text search over id/name/identifiers
  // plus a family/currency filter is what makes that answerable on a real universe;
  // scrolling a few hundred rows looking for a CUSIP is not.
  const columns = useMemo<ColumnDef<InstrumentDef>[]>(
    () => [
      {
        key: "instrumentId",
        header: "Id",
        width: 190,
        align: "left",
        accessor: (d) => d.instrumentId,
        sortKey: "instrumentId",
        filter: { kind: "text" },
        cell: (d) => <span className={admin.mono}>{d.instrumentId}</span>,
      },
      {
        key: "name",
        header: "Name",
        width: 260,
        align: "left",
        accessor: (d) => d.name,
        sortKey: "name",
        filter: { kind: "text" },
        cell: (d) => <span className={admin.nameCell}>{d.name}</span>,
      },
      {
        key: "currency",
        header: "Currency",
        width: 110,
        align: "left",
        accessor: (d) => d.currency,
        sortKey: "currency",
        filter: { kind: "select" },
        cell: (d) => <span className={admin.mono}>{d.currency}</span>,
      },
      {
        key: "family",
        header: "Family",
        width: 150,
        align: "left",
        accessor: (d) => INSTRUMENT_FAMILY_LABELS[d.family],
        sortKey: "family",
        filter: { kind: "select" },
        cell: (d) => (
          <span className={styles.familyTag}>{INSTRUMENT_FAMILY_LABELS[d.family]}</span>
        ),
      },
      {
        key: "externalIds",
        header: "External ids",
        width: 280,
        align: "left",
        // The accessor flattens every scheme/value pair so a search for an ISIN or a
        // CUSIP finds the row even though the cell renders chips.
        accessor: (d) => d.externalIds.map((x) => `${x.scheme} ${x.value}`).join(" "),
        filter: { kind: "text" },
        cell: (d) => <ExternalIds def={d} />,
      },
      ...(canEdit
        ? ([
            {
              key: "actions",
              header: "Actions",
              width: 170,
              align: "left",
              // Row actions are not data: no accessor text, so they never match a
              // search and never offer a filter.
              accessor: () => "",
              cell: (d) => (
                <div className={admin.rowActions}>
                  <Button variant="secondary" onClick={() => openEdit(d)}>
                    Edit
                  </Button>
                  <Button
                    variant="ghost"
                    onClick={() => void runAction(() => data.deleteInstrument(d.instrumentId))}
                  >
                    Delete
                  </Button>
                </div>
              ),
            },
          ] as ColumnDef<InstrumentDef>[])
        : []),
    ],
    [canEdit, data, openEdit, runAction],
  );

  const [query, setQuery] = useState("");
  const searched = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (needle === "") return data.instruments;
    return data.instruments.filter((d) =>
      columns.some((c) => c.accessor(d).toLowerCase().includes(needle)),
    );
  }, [data.instruments, query, columns]);

  const grid = useGridState<InstrumentDef>({
    tableId: "reference-data-instruments",
    columns,
    rows: searched,
    allRows: data.instruments,
    initialSort: { key: "instrumentId", dir: "asc" },
  });

  const headActions = (
    <div className={admin.headActions}>
      <Button variant="ghost" onClick={() => void data.refetch()} disabled={data.isLoading}>
        Refresh
      </Button>
      {canEdit && (
        <Button variant="primary" onClick={openCreate}>
          New instrument
        </Button>
      )}
    </div>
  );

  const formOpen = canEdit && (showCreate || editing !== null);

  return (
    <div className={admin.root}>
      <Panel title="Instruments" glyph="❏" actions={headActions}>
        {data.error && <p className={admin.banner}>{data.error}</p>}
        {actionError && <p className={admin.banner}>{actionError}</p>}
        {!canEdit && (
          <p className={styles.adminNote}>
            You can browse the registry. Creating, editing and deleting instrument definitions
            requires reference-data permission.
          </p>
        )}
        {data.instruments.length === 0 ? (
          <p className={admin.empty}>No instrument definitions yet.</p>
        ) : (
          <>
            <TableSearch
              query={query}
              onQueryChange={setQuery}
              shown={grid.shown}
              total={grid.total}
              label="Search instruments"
              placeholder="Filter by id, name or identifier…"
            />
            <DataTable
              label="Instruments"
              columns={columns}
              grid={grid}
              rowKey={(d) => d.instrumentId}
              rowProps={(d) => ({ "data-testid": `instrument-row-${d.instrumentId}` })}
              hideRowCount
              emptyState="No instrument matches this search."
            />
          </>
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
