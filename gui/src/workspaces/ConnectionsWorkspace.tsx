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

import { useEffect, useMemo, useState } from "react";

import { useApp } from "../app/AppContext";
import { FixConnectionWizard } from "../components/FixConnectionWizard";
import { FixSessionMonitor } from "../components/FixSessionMonitor";
import { FixSpecModal } from "../components/FixSpecModal";
import { Button } from "../components/Button";
import { DataTable } from "../components/DataTable";
import { Panel } from "../components/Panel";
import { TableSearch } from "../components/TableSearch";
import type { DeskDesc, FixConnection, FixConnectionSpec } from "../data/contract";
import { useGridState } from "../hooks/useGridState";
import { useFixConnections } from "../hooks/useFixConnections";
import type { ColumnDef } from "../lib/grid";
import {
  buildFixClientConfig,
  downloadText,
  fixClientConfigFilename,
  FIX_DICTIONARY_URL,
} from "../lib/fixClientConfig";
import styles from "./ConnectionsWorkspace.module.css";

/** Resolve a routing-desk id to its display name, falling back to the bare id. */
function deskName(desks: readonly DeskDesc[], deskId: string): string {
  return desks.find((d) => d.id === deskId)?.name ?? deskId;
}

/**
 * The editable spec for an existing connection — every field as it stands, so an
 * update carries the whole record and changes only what the caller overrides.
 * `update` replaces the definition, so omitting a field would silently blank it.
 */
function specOf(c: FixConnection, over: Partial<FixConnectionSpec> = {}): FixConnectionSpec {
  return {
    id: c.id,
    name: c.name,
    kind: c.kind,
    bindAddr: c.bindAddr,
    senderCompId: c.senderCompId,
    targetCompId: c.targetCompId,
    enabled: c.enabled,
    desk: c.desk,
    orderEndpoint: c.orderEndpoint,
    ...over,
  };
}

/** The display label for a connection's dialect. */
function kindLabel(c: FixConnection): string {
  switch (c.kind) {
    case "OPTIONS":
      return "Options";
    case "FIXED_INCOME_QUOTE":
      return "Fixed Income — Quote (RFQ)";
    case "FIXED_INCOME_STREAM":
      return "Fixed Income — Request for Stream (RFS)";
    case "FIXED_INCOME_ESP":
      return "Fixed Income — Executable Streaming Price (ESP)";
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
  // The connection whose OUTBOUND order route is being edited, and the draft value.
  // Editing is inline rather than in a modal because it is a one-field change an
  // operator makes while reading the row's live status.
  const [routeEditId, setRouteEditId] = useState<string | null>(null);
  const [routeDraft, setRouteDraft] = useState("");
  // The desks a new connection may be routed to (routing is OPTIONAL — a blank
  // desk is a valid unrouted connection). Loaded for the wizard's routing-desk
  // picker AND to resolve each row's routing-desk id to its display name. Desk
  // admin is admin-gated, so a non-admin simply sees an empty roster. Refreshed
  // when the wizard opens so a desk just created in the Admin workspace is
  // selectable.
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

  // A FIX roster is a lookup surface — "is CELER_RATES_ESP up, and where does it bind" —
  // so it carries the same search + per-column filters as every other roster. Status,
  // dialect and desk are select filters because they are small closed vocabularies; the
  // identifiers are free text.
  const columns = useMemo<ColumnDef<FixConnection>[]>(
    () => [
      {
        key: "status",
        header: "Status",
        width: 120,
        align: "left",
        accessor: (c) => (c.running ? "running" : "stopped"),
        sortKey: "status",
        filter: { kind: "select" },
        cell: (c) => (
          <span
            className={[styles.badge, c.running ? styles.badgeOn : styles.badgeOff].join(" ")}
            role="img"
            aria-label={c.running ? "running" : "stopped"}
            title={c.running ? `running on ${c.boundAddr}` : "stopped"}
          >
            {c.running ? "◉ running" : "○ stopped"}
          </span>
        ),
      },
      {
        key: "name",
        header: "Name",
        width: 200,
        align: "left",
        accessor: (c) => c.name,
        sortKey: "name",
        filter: { kind: "text" },
        cell: (c) => <span className={styles.nameCell}>{c.name}</span>,
      },
      {
        key: "dialect",
        header: "Dialect",
        width: 150,
        align: "left",
        accessor: (c) => kindLabel(c),
        sortKey: "dialect",
        filter: { kind: "select" },
      },
      {
        key: "bindAddr",
        header: "Bind address",
        width: 170,
        align: "left",
        accessor: (c) => (c.running && c.boundAddr ? c.boundAddr : c.bindAddr),
        sortKey: "bindAddr",
        filter: { kind: "text" },
        cell: (c) => (
          <span className={styles.mono}>{c.running && c.boundAddr ? c.boundAddr : c.bindAddr}</span>
        ),
      },
      {
        key: "desk",
        header: "Desk",
        width: 160,
        align: "left",
        // An unrouted connection reads as "unrouted" rather than empty, so the select
        // filter offers it as a first-class value — finding them is the point.
        accessor: (c) => (c.desk ? deskName(desks, c.desk) : "unrouted"),
        sortKey: "desk",
        filter: { kind: "select" },
        cell: (c) =>
          c.desk ? (
            <span className={styles.nameCell}>{deskName(desks, c.desk)}</span>
          ) : (
            <span
              className={styles.unrouted}
              title="Unrouted — no desk's users receive this connection's RFQs or deals."
            >
              ⚠ unrouted
            </span>
          ),
      },
      {
        key: "senderCompId",
        header: "SenderCompID",
        width: 180,
        align: "left",
        accessor: (c) => c.senderCompId,
        sortKey: "senderCompId",
        filter: { kind: "text" },
        cell: (c) => <span className={styles.mono}>{c.senderCompId}</span>,
      },
      {
        key: "orderEndpoint",
        header: "Order route",
        width: 260,
        align: "left",
        description:
          "Where WE dial to send this counterparty a hedge order (NewOrderSingle). Without it the counterparty can quote us but we cannot trade with it.",
        // Same reasoning as the desk column: a missing route is the thing an operator
        // hunts for, so it is a filterable value rather than a blank.
        accessor: (c) => c.orderEndpoint || "no order route",
        sortKey: "orderEndpoint",
        filter: { kind: "text" },
        cell: (c) =>
          routeEditId === c.id ? (
            <form
              className={styles.routeEdit}
              onSubmit={(e) => {
                e.preventDefault();
                const next = routeDraft.trim();
                void runAction(c.id, async () => {
                  await fix.update(c.id, specOf(c, { orderEndpoint: next }));
                  setRouteEditId(null);
                });
              }}
            >
              <input
                className={styles.routeInput}
                value={routeDraft}
                autoFocus
                placeholder="host:port"
                aria-label={`Order route for ${c.name}`}
                onChange={(e) => setRouteDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setRouteEditId(null);
                }}
              />
              <Button variant="primary" type="submit" disabled={busyId === c.id}>
                Save
              </Button>
              <Button variant="ghost" onClick={() => setRouteEditId(null)}>
                Cancel
              </Button>
            </form>
          ) : (
            <button
              type="button"
              className={styles.routeCell}
              aria-label={`Edit order route for ${c.name}`}
              title={
                c.orderEndpoint
                  ? `Hedge orders are sent to ${c.orderEndpoint}`
                  : "No order route — this counterparty can quote us, but a hedge order to it is answered no_order_endpoint and is never filled from its quote."
              }
              onClick={() => {
                setRouteEditId(c.id);
                setRouteDraft(c.orderEndpoint);
              }}
            >
              {c.orderEndpoint || <span className={styles.unrouted}>⚠ no order route</span>}
            </button>
          ),
      },
      {
        key: "actions",
        header: "Actions",
        width: 330,
        align: "left",
        accessor: () => "",
        cell: (c) => (
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
              onClick={() => void runAction(c.id, () => fix.setEnabled(c.id, !c.enabled))}
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
        ),
      },
    ],
    [desks, routeEditId, routeDraft, busyId, monitorId, fix, runAction],
  );

  const [query, setQuery] = useState("");
  const searched = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (needle === "") return fix.connections;
    return fix.connections.filter((c) =>
      columns.some((col) => col.accessor(c).toLowerCase().includes(needle)),
    );
  }, [fix.connections, query, columns]);

  const grid = useGridState<FixConnection>({
    tableId: "fix-connections",
    columns,
    rows: searched,
    allRows: fix.connections,
    initialSort: { key: "name", dir: "asc" },
  });

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
          <>
            <TableSearch
              query={query}
              onQueryChange={setQuery}
              shown={grid.shown}
              total={grid.total}
              label="Search connections"
              placeholder="Filter by name, CompID, desk or address…"
            />
            <DataTable
              label="FIX connections"
              columns={columns}
              grid={grid}
              rowKey={(c) => c.id}
              rowProps={(c) => ({ "data-testid": `fix-connection-row-${c.id}` })}
              hideRowCount
              emptyState="No connection matches this search."
            />
          </>
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
