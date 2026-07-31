/**
 * CorporateActionsWorkspace — the bond corporate-action inbox + effective-schedule
 * viewer (the `CorporateActionsService` client). It lists the golden-source CA
 * events (event type / instrument / key dates / lifecycle status) for any signed-in
 * fixed-income viewer, and — selecting one — shows the affected instrument's
 * effective (post-any-applied-CA) cashflow schedule so a trader sees the effect of
 * applying a CA.
 *
 * The CA-inbox + schedule READS sit on the `view·FI` floor (every FI viewer sees the
 * list). The Confirm / Apply lifecycle WRITES require the dedicated `refdata`
 * capability: without it the controls are DISABLED (never hidden) and carry the
 * standard denial tooltip, so the surface is read-only but discoverable. Applying a
 * full call collapses the shown schedule to its call cashflow — the schedule shortens
 * in place, demonstrating the CA's effect. Server enforces every write; this gates
 * the affordance only.
 */

import { useState } from "react";

import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import {
  CORP_ACTION_STATUS_LABELS,
  CORP_EVENT_TYPE_LABELS,
  CORP_MANDATORY_LABELS,
  type CorporateAction,
  type InstrumentScheduleFlow,
} from "../data/contract";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import { useCorporateActions } from "../hooks/useCorporateActions";
import admin from "./AdminWorkspace.module.css";
import styles from "./CorporateActionsWorkspace.module.css";

/** A dash when a date/value is empty. */
function dash(value: string): React.ReactElement | string {
  return value.length > 0 ? value : "—";
}

/** The lifecycle-status pill (a class per status drives its colour). */
function StatusPill({ status }: { status: CorporateAction["status"] }): React.ReactElement {
  return (
    <span className={`${styles.status} ${styles[`status_${status}`]}`}>
      {CORP_ACTION_STATUS_LABELS[status]}
    </span>
  );
}

/** Format a per-100 cash figure to 3 dp. */
function fmt(n: number): string {
  return n.toFixed(3);
}

export function CorporateActionsWorkspace(): React.ReactElement {
  const app = useApp();
  const { auth } = app;
  const isAuthed = auth.user !== null;
  // The Confirm / Apply lifecycle writes gate on the dedicated `refdata` capability
  // (fixed income). Reads (the inbox + schedule) stay on the `view` floor.
  const canRefdata = auth.can("refdata", "fixed_income");
  const denialTitle = capabilityDenialTitle("refdata", "fixed_income");

  const data = useCorporateActions(app.transport, isAuthed);

  const [selectedCaId, setSelectedCaId] = useState<string | null>(null);
  const [flows, setFlows] = useState<InstrumentScheduleFlow[] | null>(null);
  const [poolFactor, setPoolFactor] = useState<number>(1);
  const [scheduleInstrumentId, setScheduleInstrumentId] = useState<string | null>(null);
  const [heldFace, setHeldFace] = useState<string>("1000000");
  const [actionError, setActionError] = useState<string | null>(null);
  const [applyNote, setApplyNote] = useState<string | null>(null);

  const selected = selectedCaId
    ? (data.actions.find((a) => a.caId === selectedCaId) ?? null)
    : null;

  // Resolve a CA's ISIN to the reference-data instrument id (the schedule key).
  const instrumentIdFor = (ca: CorporateAction): string | null =>
    data.instrumentByIsin.get(ca.isin)?.instrumentId ?? null;
  const instrumentNameFor = (ca: CorporateAction): string =>
    data.instrumentByIsin.get(ca.isin)?.name ?? ca.isin;

  const loadScheduleFor = async (ca: CorporateAction): Promise<void> => {
    const instrumentId = instrumentIdFor(ca);
    if (instrumentId === null) {
      setFlows([]);
      setScheduleInstrumentId(null);
      setActionError(`No reference-data instrument found for ISIN ${ca.isin}.`);
      return;
    }
    const res = await data.loadSchedule(instrumentId);
    setFlows(res.flows);
    setPoolFactor(res.poolFactor);
    setScheduleInstrumentId(instrumentId);
  };

  const selectCa = async (ca: CorporateAction): Promise<void> => {
    setSelectedCaId(ca.caId);
    setActionError(null);
    setApplyNote(null);
    await loadScheduleFor(ca);
  };

  const runConfirm = async (ca: CorporateAction): Promise<void> => {
    setActionError(null);
    setApplyNote(null);
    try {
      await data.confirm(ca.caId);
    } catch (e: unknown) {
      setActionError(e instanceof Error ? e.message : "confirm failed");
    }
  };

  const runApply = async (ca: CorporateAction): Promise<void> => {
    setActionError(null);
    setApplyNote(null);
    const face = Number(heldFace);
    if (!Number.isFinite(face) || face <= 0) {
      setActionError("Enter a positive held face to apply the corporate action.");
      return;
    }
    try {
      const res = await data.apply(ca.caId, face);
      setApplyNote(
        `Applied. Face Δ ${res.faceDelta.toLocaleString()} · cash ${res.cash.toLocaleString()} · ${res.remainingFlows} remaining flow(s).`,
      );
      // Re-read the (now post-event) schedule for the affected instrument.
      const fresh = await data.loadSchedule(res.instrumentId);
      setFlows(fresh.flows);
      setPoolFactor(fresh.poolFactor);
      setScheduleInstrumentId(res.instrumentId);
    } catch (e: unknown) {
      setActionError(e instanceof Error ? e.message : "apply failed");
    }
  };

  // --- the sign-in gate (anonymous only; FI viewers see the inbox) ----------
  if (!isAuthed) {
    return (
      <div className={admin.root}>
        <div className={admin.gate}>
          <h2 className={admin.gateTitle}>Corporate Actions</h2>
          <p className={admin.gateHint}>
            Sign in to view the bond corporate-action inbox and instrument schedules. Confirming
            and applying corporate actions requires the reference-data capability.
          </p>
          <Button variant="primary" onClick={() => app.setSignInOpen(true)}>
            Sign in
          </Button>
        </div>
      </div>
    );
  }

  const canConfirm = (ca: CorporateAction): boolean =>
    ca.status === "ANNOUNCED" || ca.status === "ELECTED";
  const canApply = (ca: CorporateAction): boolean => ca.status === "CONFIRMED";

  const headActions = (
    <div className={admin.headActions}>
      <Button variant="ghost" onClick={() => void data.refetch()} disabled={data.isLoading}>
        Refresh
      </Button>
    </div>
  );

  return (
    <div className={styles.root}>
      <Panel title="Corporate action inbox" glyph="❖" actions={headActions}>
        {data.error && <p className={admin.banner}>{data.error}</p>}
        {!canRefdata && (
          <p className={styles.note} data-testid="ca-readonly-note">
            You can view the corporate-action inbox and instrument schedules. Confirming and
            applying corporate actions requires the reference-data capability.
          </p>
        )}
        {data.actions.length === 0 ? (
          <p className={admin.empty}>No corporate actions announced.</p>
        ) : (
          <table className={admin.table}>
            <thead>
              <tr>
                <th scope="col">Event</th>
                <th scope="col">Instrument</th>
                <th scope="col">Ex</th>
                <th scope="col">Payment</th>
                <th scope="col">Cash /100</th>
                <th scope="col">Status</th>
                <th scope="col" className={admin.actionsCol}>
                  Actions
                </th>
              </tr>
            </thead>
            <tbody>
              {data.actions.map((ca) => {
                const isSelected = ca.caId === selectedCaId;
                return (
                  <tr
                    key={ca.caId}
                    data-testid={`ca-row-${ca.caId}`}
                    className={isSelected ? styles.selectedRow : undefined}
                  >
                    <td>
                      <span className={styles.eventName}>{CORP_EVENT_TYPE_LABELS[ca.caev]}</span>
                      <span className={styles.camv}>{CORP_MANDATORY_LABELS[ca.camv]}</span>
                    </td>
                    <td className={admin.nameCell}>
                      <span className={styles.instrName}>{instrumentNameFor(ca)}</span>
                      <span className={admin.mono}>{ca.isin}</span>
                    </td>
                    <td className={admin.mono}>{dash(ca.exDate)}</td>
                    <td className={admin.mono}>{dash(ca.paymentDate)}</td>
                    <td className={admin.mono}>{fmt(ca.cashPer100)}</td>
                    <td>
                      <StatusPill status={ca.status} />
                    </td>
                    <td className={admin.actionsCol}>
                      <div className={admin.rowActions}>
                        <Button
                          variant={isSelected ? "primary" : "secondary"}
                          onClick={() => void selectCa(ca)}
                        >
                          {isSelected ? "Selected" : "Schedule"}
                        </Button>
                        <Button
                          variant="secondary"
                          data-testid={`ca-confirm-${ca.caId}`}
                          disabled={!canRefdata || !canConfirm(ca)}
                          title={!canRefdata ? denialTitle : undefined}
                          onClick={() => void runConfirm(ca)}
                        >
                          Confirm
                        </Button>
                        <Button
                          variant="ghost"
                          data-testid={`ca-apply-${ca.caId}`}
                          disabled={!canRefdata || !canApply(ca)}
                          title={!canRefdata ? denialTitle : undefined}
                          onClick={() => void selectCa(ca).then(() => runApply(ca))}
                        >
                          Apply
                        </Button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </Panel>

      <Panel title="Instrument schedule" glyph="≣">
        {actionError && <p className={admin.banner}>{actionError}</p>}
        {selected === null ? (
          <p className={admin.empty} data-testid="ca-schedule-empty">
            Select a corporate action to see the affected instrument&rsquo;s cashflow schedule and
            the effect of applying it.
          </p>
        ) : (
          <div className={styles.detail}>
            <div className={styles.detailHead}>
              <div>
                <span className={styles.eventName}>{CORP_EVENT_TYPE_LABELS[selected.caev]}</span>{" "}
                <span className={styles.instrName}>{instrumentNameFor(selected)}</span>
                <div className={admin.mono}>{selected.isin}</div>
              </div>
              <StatusPill status={selected.status} />
            </div>

            <dl className={styles.terms}>
              <div>
                <dt>Announced</dt>
                <dd className={admin.mono}>{dash(selected.announcementDate)}</dd>
              </div>
              <div>
                <dt>Record</dt>
                <dd className={admin.mono}>{dash(selected.recordDate)}</dd>
              </div>
              <div>
                <dt>Ex</dt>
                <dd className={admin.mono}>{dash(selected.exDate)}</dd>
              </div>
              {selected.responseDeadline !== undefined && (
                <div>
                  <dt>Election deadline</dt>
                  <dd className={admin.mono}>{selected.responseDeadline}</dd>
                </div>
              )}
              <div>
                <dt>Payment</dt>
                <dd className={admin.mono}>{dash(selected.paymentDate)}</dd>
              </div>
              <div>
                <dt>Cash /100</dt>
                <dd className={admin.mono}>{fmt(selected.cashPer100)}</dd>
              </div>
              <div>
                <dt>Pool factor</dt>
                <dd className={admin.mono} data-testid="ca-pool-factor">
                  {poolFactor.toFixed(4)}
                </dd>
              </div>
            </dl>

            <div className={styles.lifecycle}>
              <label className={styles.faceField}>
                <span>Held face</span>
                <input
                  type="number"
                  min={0}
                  step={100000}
                  value={heldFace}
                  data-testid="ca-heldface"
                  onChange={(e) => setHeldFace(e.target.value)}
                  disabled={!canRefdata}
                />
              </label>
              <Button
                variant="secondary"
                data-testid="ca-detail-confirm"
                disabled={!canRefdata || !canConfirm(selected)}
                title={!canRefdata ? denialTitle : undefined}
                onClick={() => void runConfirm(selected)}
              >
                Confirm
              </Button>
              <Button
                variant="primary"
                data-testid="ca-detail-apply"
                disabled={!canRefdata || !canApply(selected)}
                title={!canRefdata ? denialTitle : undefined}
                onClick={() => void runApply(selected)}
              >
                Apply
              </Button>
            </div>
            {applyNote && (
              <p className={styles.applyNote} data-testid="ca-apply-note">
                {applyNote}
              </p>
            )}

            <h3 className={styles.scheduleHead}>
              Effective schedule
              <span className={styles.flowCount} data-testid="ca-flow-count">
                {flows === null ? "…" : `${flows.length} flow(s)`}
              </span>
            </h3>
            {flows !== null && flows.length === 0 ? (
              <p className={admin.empty}>
                No remaining cashflows{scheduleInstrumentId ? "" : " (no schedule for this instrument)"}.
              </p>
            ) : (
              <table className={admin.table} data-testid="ca-schedule-table">
                <thead>
                  <tr>
                    <th scope="col">Date</th>
                    <th scope="col">Coupon /100</th>
                    <th scope="col">Principal /100</th>
                  </tr>
                </thead>
                <tbody>
                  {(flows ?? []).map((f, i) => (
                    <tr key={`${f.date}-${i}`}>
                      <td className={admin.mono}>{f.date}</td>
                      <td className={admin.mono}>{fmt(f.coupon)}</td>
                      <td className={admin.mono}>{fmt(f.principal)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        )}
      </Panel>
    </div>
  );
}
