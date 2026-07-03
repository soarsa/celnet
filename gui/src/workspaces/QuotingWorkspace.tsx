/**
 * QuotingWorkspace — the dealer-quoting RFQ/IOI desk (fixed-income). The live
 * inbox of inbound dealer requests on the left; a price + respond panel on the
 * right. A trader prices the selected request against its curve (the SAME
 * `priceRates` engine the Rates workspace uses), then RESPONDS — quoting a rate
 * or rejecting — and can ACCEPT (simulating the counterparty lifting the quote)
 * to demonstrate booking a deal + a rates position. The inbox is populated only
 * by REAL inbound requests (the FIX gateway / live counterparties); exploratory
 * mock requests live in the standalone, permission-gated Simulator popup
 * (`components/SimulatorPanel`), a pure client-side sandbox that never injects
 * into this priced flow.
 *
 * One contract, two transports (GUI-DESIGN §6.2): the workspace talks ONLY to the
 * `CelnetTransport` desk seam (`submitDeskRequest` / `respondDeskRequest` /
 * `acceptDeskQuote` / `listDeskRequests` / `priceRates`), so the SAME lifecycle
 * runs through the deterministic in-app source and the live `RfqDeskService` edge
 * — and cannot drift from the wire contract. The inbox refreshes on every push
 * `Notification` (and after each action), so a new request appears live.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import { principalForScope } from "../data/riskView";
import { fmtPnlAdaptive, fmtRate, fmtClock } from "../lib/format";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import type { DeskRequest, RatesPricingResult, Side } from "../data/contract";
import { oisRatesInstrument } from "../data/contract";
import styles from "./QuotingWorkspace.module.css";

const MM = 1_000_000;

/** A human label for a request's OIS direction (carried on the wire `Side`). */
function sideLabel(side: Side): string {
  if (side === "BUY") return "Pay fixed";
  if (side === "SELL") return "Receive fixed";
  return "Two-way";
}

/** A short state badge class for a request lifecycle state. */
function stateClass(state: DeskRequest["state"]): string {
  switch (state) {
    case "PENDING":
      return styles.statePending ?? "";
    case "QUOTED":
      return styles.stateQuoted ?? "";
    case "ACCEPTED":
      return styles.stateAccepted ?? "";
    case "REJECTED":
      return styles.stateRejected ?? "";
    default:
      return styles.stateInert ?? "";
  }
}

/** Format a notional (curve ccy) as a compact millions figure. */
function fmtMm(notional: number): string {
  return `${(notional / MM).toLocaleString(undefined, { maximumFractionDigits: 1 })}mm`;
}

export function QuotingWorkspace(): React.ReactElement {
  const app = useApp();
  const principal = useMemo(() => principalForScope(app.scope), [app.scope]);
  const trader = app.auth.user?.displayName ?? "desk";

  const [requests, setRequests] = useState<DeskRequest[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // The transport seam; a ref keeps the latest refresh callback stable for the
  // notification subscription effect without re-subscribing on every render.
  const refresh = useCallback(() => {
    void app.transport
      .listDeskRequests({ ...(principal ? { principal } : {}) })
      .then((res) => {
        setRequests(res.requests);
        setError(null);
      })
      .catch((err) =>
        setError(err instanceof Error ? err.message : "failed to load desk requests"),
      );
  }, [app.transport, principal]);

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  // Initial load + refresh on every push notification (a new RFQ/IOI, an accept,
  // a reject) so the inbox is always live without a polling churn.
  useEffect(() => {
    refreshRef.current();
    const dispose = app.transport.streamNotifications(undefined, () => refreshRef.current());
    return dispose;
  }, [app.transport]);

  const selected = useMemo(
    () => requests.find((r) => r.requestId === selectedId) ?? null,
    [requests, selectedId],
  );

  // Auto-select the first PENDING request when nothing is selected, so the panel
  // is never empty while there is work to do.
  useEffect(() => {
    if (selectedId !== null && requests.some((r) => r.requestId === selectedId)) return;
    const next = requests.find((r) => r.state === "PENDING") ?? requests[0] ?? null;
    setSelectedId(next ? next.requestId : null);
  }, [requests, selectedId]);

  const isOffline = !app.transport.label.startsWith("live");

  return (
    <div className={styles.wrap}>
      <Panel material="float" className={styles.inbox} title="RFQ / IOI inbox">
        <div className={styles.inboxHead}>
          <span className={styles.engine}>{isOffline ? "in-app desk" : "live desk"}</span>
          <span className={styles.count}>
            {requests.length} request{requests.length === 1 ? "" : "s"}
          </span>
        </div>
        {error && (
          <p className={styles.error} role="alert">
            {error}
          </p>
        )}
        {requests.length === 0 ? (
          <p className={styles.empty}>No inbound requests.</p>
        ) : (
          <ul className={styles.reqList} aria-label="inbound requests">
            {requests.map((r) => {
              const active = r.requestId === selectedId;
              return (
                <li key={r.requestId}>
                  <button
                    type="button"
                    className={`${styles.reqRow} ${active ? styles.reqActive : ""}`}
                    onClick={() => setSelectedId(r.requestId)}
                    aria-current={active}
                  >
                    <span className={styles.reqTop}>
                      <span className={`${styles.kind} ${r.kind === "IOI" ? styles.kindIoi : styles.kindRfq}`}>
                        {r.kind}
                      </span>
                      <span className={styles.cpty}>{r.counterparty}</span>
                      <span className={`${styles.state} ${stateClass(r.state)}`}>{r.state}</span>
                    </span>
                    <span className={styles.reqMeta}>
                      <span>
                        {r.instrument.tenorYears}y OIS · {fmtMm(r.notional)} · {sideLabel(r.side)}
                      </span>
                      <span className={styles.deadline}>exp {fmtClock(r.expiresAtNanos)}</span>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </Panel>

      <Panel className={styles.deal} title="Price & respond">
        {selected ? (
          <PricePanel
            key={selected.requestId}
            request={selected}
            trader={trader}
            onRespondQuote={async (price, notional, validForMs) => {
              await app.transport.respondDeskRequest({
                requestId: selected.requestId,
                response: { kind: "quote", quote: { price, notional, validForMs, trader } },
                ...(principal ? { principal } : {}),
              });
              refresh();
            }}
            onReject={async (reason) => {
              await app.transport.respondDeskRequest({
                requestId: selected.requestId,
                response: { kind: "reject", reject: { reason } },
                ...(principal ? { principal } : {}),
              });
              refresh();
            }}
            onAccept={async () => {
              await app.transport.acceptDeskQuote({
                requestId: selected.requestId,
                ...(principal ? { principal } : {}),
              });
              refresh();
            }}
            priceRequest={(req) =>
              app.transport.priceRates(req.curveSet, oisRatesInstrument(req.instrument))
            }
          />
        ) : (
          <p className={styles.empty}>Select a request to price it.</p>
        )}
      </Panel>
    </div>
  );
}

// ---------------------------------------------------------------------------
// price + respond panel
// ---------------------------------------------------------------------------

function PricePanel({
  request,
  trader,
  onRespondQuote,
  onReject,
  onAccept,
  priceRequest,
}: {
  request: DeskRequest;
  trader: string;
  onRespondQuote: (price: number, notional: number, validForMs: number) => Promise<void>;
  onReject: (reason: string) => Promise<void>;
  onAccept: () => Promise<void>;
  priceRequest: (req: DeskRequest) => Promise<RatesPricingResult>;
}): React.ReactElement {
  const [priced, setPriced] = useState<RatesPricingResult | null>(null);
  const [priceError, setPriceError] = useState<string | null>(null);
  const [ratePct, setRatePct] = useState<string>("");
  const [validSecs, setValidSecs] = useState<number>(30);
  const [rejectReason, setRejectReason] = useState<string>("axe filled");
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  // Price the request against its own curve when it changes (a real PV/risk run).
  useEffect(() => {
    let live = true;
    setPriced(null);
    setPriceError(null);
    void priceRequest(request)
      .then((res) => {
        if (!live) return;
        setPriced(res);
        // Seed the quote rate from the fair par rate when the trader hasn't typed.
        setRatePct((cur) => (cur === "" ? (res.parRate * 100).toFixed(4) : cur));
      })
      .catch((err) => {
        if (!live) return;
        setPriceError(err instanceof Error ? err.message : "pricing failed");
      });
    return () => {
      live = false;
    };
  }, [request, priceRequest]);

  const quotable = request.state === "PENDING";
  const acceptable = request.state === "QUOTED";
  const rateValue = Number.parseFloat(ratePct);
  const rateValid = Number.isFinite(rateValue);

  // Capability gating (slice 5): this is a fixed-income dealer-quoting surface.
  // Responding (quote / reject) is gated on the request-kind's respond capability
  // — an RFQ needs `rfq_respond`, an IOI needs `ioi_respond`; lifting the standing
  // quote (`acceptDeskQuote`) is gated on `execute`. Disabled + tooltip, never
  // hidden; handlers no-op defensively (the server still enforces).
  const app = useApp();
  const respondAction = request.kind === "IOI" ? "ioi_respond" : "rfq_respond";
  const canRespond = app.auth.can(respondAction, "fixed_income");
  const canExecute = app.auth.can("execute", "fixed_income");
  const respondDeniedTitle = capabilityDenialTitle(respondAction, "fixed_income");
  const executeDeniedTitle = capabilityDenialTitle("execute", "fixed_income");

  const runQuote = async () => {
    if (!canRespond) return;
    if (!rateValid) return;
    setBusy(true);
    setActionError(null);
    try {
      await onRespondQuote(rateValue / 100, request.notional, validSecs * 1000);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : "quote failed");
    } finally {
      setBusy(false);
    }
  };

  const runReject = async () => {
    if (!canRespond) return;
    setBusy(true);
    setActionError(null);
    try {
      await onReject(rejectReason.trim() || "declined");
    } catch (err) {
      setActionError(err instanceof Error ? err.message : "reject failed");
    } finally {
      setBusy(false);
    }
  };

  const runAccept = async () => {
    if (!canExecute) return;
    setBusy(true);
    setActionError(null);
    try {
      await onAccept();
    } catch (err) {
      setActionError(err instanceof Error ? err.message : "accept failed");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={styles.panel}>
      <header className={styles.dealHead}>
        <span className={`${styles.kind} ${request.kind === "IOI" ? styles.kindIoi : styles.kindRfq}`}>
          {request.kind}
        </span>
        <span className={styles.dealTitle}>
          {request.counterparty} · {request.instrument.tenorYears}y OIS
        </span>
        <span className={`${styles.state} ${stateClass(request.state)}`}>{request.state}</span>
      </header>

      <dl className={styles.terms}>
        <Term label="Notional" value={fmtMm(request.notional)} />
        <Term label="Side" value={sideLabel(request.side)} />
        <Term label="Curve" value={`${request.curveSet.currency}-SOFR`} />
        <Term label="Desk" value={request.desk} />
      </dl>

      <section className={styles.priceCard} aria-label="priced risk">
        <h3 className={styles.cardTitle}>Priced against curve</h3>
        {priceError ? (
          <p className={styles.error} role="alert">
            {priceError}
          </p>
        ) : priced === null ? (
          <p className={styles.empty}>Pricing…</p>
        ) : (
          <dl className={styles.metrics}>
            <Metric label="Par rate" value={fmtRate(priced.parRate)} emphatic />
            <Metric label="PV" value={fmtPnlAdaptive(priced.pv)} unit={request.curveSet.currency} />
            <Metric label="PV01" value={fmtPnlAdaptive(priced.pv01)} unit="/bp" />
            <Metric label="DV01" value={fmtPnlAdaptive(priced.dv01)} unit="/bp" />
          </dl>
        )}
      </section>

      {request.quote && (
        <section className={styles.quoteCard} aria-label="standing quote">
          <h3 className={styles.cardTitle}>Standing quote</h3>
          <dl className={styles.metrics}>
            <Metric label="Quoted rate" value={fmtRate(request.quote.price)} emphatic />
            <Metric label="Notional" value={fmtMm(request.quote.notional)} />
            <Metric label="Good for" value={`${Math.round(request.quote.validForMs / 1000)}s`} />
            <Metric label="Trader" value={request.quote.trader} />
          </dl>
        </section>
      )}

      {quotable && (
        <section className={styles.respond} aria-label="respond to request">
          <div className={styles.quoteForm}>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>Quote rate %</span>
              <input
                className={styles.input}
                type="number"
                step={0.01}
                value={ratePct}
                aria-label="quote rate in percent"
                onChange={(e) => setRatePct(e.target.value)}
              />
            </label>
            <label className={styles.field}>
              <span className={styles.fieldLabel}>Good for (s)</span>
              <input
                className={styles.input}
                type="number"
                min={1}
                step={5}
                value={validSecs}
                aria-label="quote validity in seconds"
                onChange={(e) => setValidSecs(Math.max(1, Math.trunc(Number(e.target.value))))}
              />
            </label>
            <Button
              variant="primary"
              disabled={busy || !rateValid || !canRespond}
              onClick={runQuote}
              title={canRespond ? undefined : respondDeniedTitle}
            >
              Send quote
            </Button>
          </div>
          <div className={styles.rejectForm}>
            <input
              className={styles.input}
              type="text"
              value={rejectReason}
              aria-label="reject reason"
              placeholder="reject reason"
              onChange={(e) => setRejectReason(e.target.value)}
            />
            <Button
              variant="ghost"
              disabled={busy || !canRespond}
              onClick={runReject}
              title={canRespond ? undefined : respondDeniedTitle}
            >
              Reject
            </Button>
          </div>
        </section>
      )}

      {acceptable && (
        <section className={styles.accept} aria-label="accept quote">
          <p className={styles.acceptHint}>
            Simulate the counterparty lifting the quote — books a deal and a rates
            position.
          </p>
          <Button
            variant="primary"
            disabled={busy || !canExecute}
            onClick={runAccept}
            title={canExecute ? undefined : executeDeniedTitle}
          >
            Accept (counterparty lifts) — {trader}
          </Button>
        </section>
      )}

      {actionError && (
        <p className={styles.error} role="alert">
          {actionError}
        </p>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// small presentational pieces
// ---------------------------------------------------------------------------

function Term({ label, value }: { label: string; value: string }): React.ReactElement {
  return (
    <div className={styles.term}>
      <dt className={styles.termLabel}>{label}</dt>
      <dd className={styles.termValue}>{value}</dd>
    </div>
  );
}

function Metric({
  label,
  value,
  unit,
  emphatic,
}: {
  label: string;
  value: string;
  unit?: string;
  emphatic?: boolean;
}): React.ReactElement {
  return (
    <div className={`${styles.metric} ${emphatic ? styles.metricEmphatic : ""}`}>
      <dt className={styles.metricLabel}>{label}</dt>
      <dd className={styles.metricValue}>
        {value}
        {unit && <span className={styles.metricUnit}>{unit}</span>}
      </dd>
    </div>
  );
}
