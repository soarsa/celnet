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
import { TableSearch } from "../components/TableSearch";
import { useTableFilter } from "../hooks/useTableFilter";
import { principalForScope } from "../data/riskView";
import { fmtPnlAdaptive, fmtRate, fmtClock, fmtCompact } from "../lib/format";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import { configuredLicense } from "../lib/commands";
import type {
  DeskRequest,
  RatesInstrument,
  RatesPricingResult,
  RatesQuote,
  Side,
} from "../data/contract";
import { oisRatesInstrument } from "../data/contract";
import { DEFAULT_USD_SOFR_CURVE } from "../data/ratesPricing";
import styles from "./QuotingWorkspace.module.css";

const MM = 1_000_000;

/** The taker RFQ instrument arms (the client-reachable `RatesInstrument` oneof). */
type RfqArm = "ois" | "irs" | "fra" | "bond";

const ARM_LABELS: Record<RfqArm, string> = {
  ois: "OIS",
  irs: "IRS",
  fra: "FRA",
  bond: "Bond",
};

/** The taker's directional intent options for the two-way RFQ. */
const RFQ_SIDES: readonly Side[] = ["BUY", "SELL", "TWO_WAY"];

/** A human label for a request's OIS direction (carried on the wire `Side`). */
export function sideLabel(side: Side): string {
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

/** An inbound request's user-visible textual fields, concatenated for search. */
function requestSearchText(r: DeskRequest): string {
  return [
    r.kind,
    r.counterparty,
    r.state,
    `${r.instrument.tenorYears}y OIS`,
    fmtCompact(r.notional),
    sideLabel(r.side),
    fmtClock(r.expiresAtNanos),
  ].join(" ");
}

export function QuotingWorkspace(): React.ReactElement {
  const app = useApp();
  const principal = useMemo(() => principalForScope(app.scope), [app.scope]);
  const trader = app.auth.user?.displayName ?? "desk";

  const [requests, setRequests] = useState<DeskRequest[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The workspace is class-parametric across the FI RFQ lifecycle: RESPOND to
  // inbound requests (the maker desk, `RfqDeskService`) OR REQUEST a firm two-way
  // (the taker RFQ, `QuoteService.RequestRatesQuote`). Default to the maker desk so
  // the live inbox leads.
  const [mode, setMode] = useState<"respond" | "request">("respond");

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

  // Filter the inbound-request list (the desk's working orders) on top of its
  // existing order — selection stays keyed on the full list, so filtering only
  // narrows what is shown, never what is priced.
  const {
    query: reqQuery,
    setQuery: setReqQuery,
    filtered: filteredRequests,
    shown: reqShown,
    total: reqTotal,
  } = useTableFilter(requests, requestSearchText);

  const isOffline = !app.transport.label.startsWith("live");

  return (
    <div className={styles.root}>
      <div className={styles.modeBar} role="tablist" aria-label="quoting mode">
        <button
          type="button"
          role="tab"
          id="quoting-mode-respond"
          aria-selected={mode === "respond"}
          className={`${styles.modeTab} ${mode === "respond" ? styles.modeTabActive : ""}`}
          onClick={() => setMode("respond")}
        >
          Respond to requests
        </button>
        <button
          type="button"
          role="tab"
          id="quoting-mode-request"
          aria-selected={mode === "request"}
          className={`${styles.modeTab} ${mode === "request" ? styles.modeTabActive : ""}`}
          onClick={() => setMode("request")}
        >
          Request two-way (RFQ)
        </button>
      </div>

      {mode === "request" ? (
        <div
          className={styles.takerRegion}
          role="tabpanel"
          aria-labelledby="quoting-mode-request"
        >
          <TakerRfqPanel trader={trader} />
        </div>
      ) : (
        <div className={styles.wrap} role="tabpanel" aria-labelledby="quoting-mode-respond">
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
          <>
            <TableSearch
              query={reqQuery}
              onQueryChange={setReqQuery}
              shown={reqShown}
              total={reqTotal}
              label="Search requests"
              placeholder="Filter requests…"
            />
            {filteredRequests.length === 0 ? (
              <p className={styles.empty}>No requests match “{reqQuery}”.</p>
            ) : (
          <ul className={styles.reqList} aria-label="inbound requests">
            {filteredRequests.map((r) => {
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
                        {r.instrument.tenorYears}y OIS · {fmtCompact(r.notional)} · {sideLabel(r.side)}
                      </span>
                      <span className={styles.deadline}>exp {fmtClock(r.expiresAtNanos)}</span>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
            )}
          </>
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
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// taker RFQ panel — request a firm two-way (QuoteService.RequestRatesQuote)
// ---------------------------------------------------------------------------

/**
 * The taker's fixed-income RFQ: build a `RatesInstrument` (OIS / IRS / FRA / cash
 * bond), price it against the calibrated USD-SOFR curve, and request a firm two-way
 * (`transport.requestRatesQuote`). Renders the two-way (bid | mid | offer) — a RATE
 * market for an OIS/IRS/FRA, a clean-PRICE market for a cash bond — plus the full FI
 * risk. Mirrors the FX RFQ ergonomics (a single request-then-render two-way); there
 * is NO multi-dealer rates path on the contract (the panel wire is FX-`Instrument`
 * only), so no dealer ladder is shown. License-gated on `fixed_income` and the
 * `price` authority (a pure price-discovery request); a11y-safe throughout.
 */
function TakerRfqPanel({ trader }: { trader: string }): React.ReactElement {
  const app = useApp();
  const licensed = useMemo(() => configuredLicense(), []);
  const canRequest = app.auth.can("price", "fixed_income") && licensed("fixed_income");
  const requestDeniedTitle = capabilityDenialTitle("price", "fixed_income");

  const curve = DEFAULT_USD_SOFR_CURVE;

  const [arm, setArm] = useState<RfqArm>("ois");
  const [side, setSide] = useState<Side>("TWO_WAY");
  const [notionalMm, setNotionalMm] = useState<string>("100");
  // OIS / IRS
  const [tenorYears, setTenorYears] = useState<string>("5");
  const [fixedPct, setFixedPct] = useState<string>("4.00");
  // FRA
  const [startMonths, setStartMonths] = useState<string>("3");
  const [endMonths, setEndMonths] = useState<string>("6");
  // Bond
  const [couponPct, setCouponPct] = useState<string>("5.00");
  const [maturityYears, setMaturityYears] = useState<string>("5");

  const [quote, setQuote] = useState<RatesQuote | null>(null);
  const [busy, setBusy] = useState(false);
  const [reqError, setReqError] = useState<string | null>(null);

  const notional = Number.parseFloat(notionalMm) * MM;
  const notionalValid = Number.isFinite(notional) && notional > 0;

  // Build the RatesInstrument oneof from the arm-specific inputs. The arm's own
  // direction/position is set from the taker `side` (BUY = pay fixed / long); the
  // server/offline path re-derives the risk sign from the RFQ envelope side anyway,
  // so this is purely for a faithful, complete wire instrument.
  const buildInstrument = useCallback((): RatesInstrument => {
    const direction = side === "SELL" ? "RECEIVE_FIXED" : "PAY_FIXED";
    const size = notionalValid ? notional : MM;
    switch (arm) {
      case "ois":
        return {
          kind: "ois",
          ois: {
            tenorYears: Math.max(1, Math.trunc(Number(tenorYears) || 1)),
            fixedRate: (Number(fixedPct) || 0) / 100,
            notional: size,
            direction,
          },
        };
      case "irs":
        return {
          kind: "irs",
          irs: {
            tenorYears: Math.max(1, Math.trunc(Number(tenorYears) || 1)),
            fixedRate: (Number(fixedPct) || 0) / 100,
            notional: size,
            direction,
            fixedFrequency: "SEMI_ANNUAL",
            fixedDayCount: "ACT_360",
            floatFrequency: "QUARTERLY",
            floatDayCount: "ACT_360",
          },
        };
      case "fra":
        return {
          kind: "fra",
          fra: {
            startMonths: Math.max(0, Math.trunc(Number(startMonths) || 0)),
            endMonths: Math.max(1, Math.trunc(Number(endMonths) || 1)),
            fixedRate: (Number(fixedPct) || 0) / 100,
            notional: size,
            direction,
            accrualBasis: "ACT_360",
          },
        };
      case "bond":
        return {
          kind: "bond",
          bond: {
            couponRate: (Number(couponPct) || 0) / 100,
            couponFrequency: "SEMI_ANNUAL",
            dayCount: "THIRTY_360_BOND_BASIS",
            maturityDate: {
              year: curve.referenceDate.year + Math.max(1, Math.trunc(Number(maturityYears) || 1)),
              month: curve.referenceDate.month,
              day: curve.referenceDate.day,
            },
            redemption: 100,
            position: side === "SELL" ? "SHORT" : "LONG",
          },
        };
    }
  }, [
    arm,
    side,
    notional,
    notionalValid,
    tenorYears,
    fixedPct,
    startMonths,
    endMonths,
    couponPct,
    maturityYears,
    curve.referenceDate.year,
    curve.referenceDate.month,
    curve.referenceDate.day,
  ]);

  const runRequest = async () => {
    if (!canRequest || !notionalValid) return;
    setBusy(true);
    setReqError(null);
    try {
      const key = `fi-rfq-${arm}-${side}-${Date.now()}`;
      const q = await app.transport.requestRatesQuote(
        curve,
        buildInstrument(),
        notional,
        side,
        key,
      );
      setQuote(q);
    } catch (err) {
      setQuote(null);
      setReqError(err instanceof Error ? err.message : "RFQ failed");
    } finally {
      setBusy(false);
    }
  };

  // A cash bond quotes a clean-PRICE market (per 100 face); the rate arms quote a
  // RATE market. The client knows the arm it requested, so it formats the reply's
  // side-independent two-way correctly (the reply carries no arm discriminator).
  const isPriceMarket = arm === "bond";
  const fmtLevel = (v: number): string => (isPriceMarket ? v.toFixed(3) : fmtRate(v));

  return (
    <Panel className={styles.taker} title="Request a firm two-way (RFQ)">
      <p className={styles.takerHint}>
        Price a fixed-income instrument against the {curve.currency}-SOFR curve and
        request a firm two-way. A single price-discovery two-way — the fixed-income
        contract has no multi-dealer rates panel.
      </p>

      <div className={styles.rfqForm}>
        <fieldset className={styles.armSet}>
          <legend className={styles.fieldLabel}>Instrument</legend>
          <div className={styles.segmented} role="radiogroup" aria-label="instrument arm">
            {(Object.keys(ARM_LABELS) as RfqArm[]).map((a) => (
              <button
                key={a}
                type="button"
                role="radio"
                aria-checked={arm === a}
                className={`${styles.segItem} ${arm === a ? styles.segItemActive : ""}`}
                onClick={() => {
                  setArm(a);
                  setQuote(null);
                }}
              >
                {ARM_LABELS[a]}
              </button>
            ))}
          </div>
        </fieldset>

        <fieldset className={styles.armSet}>
          <legend className={styles.fieldLabel}>Side</legend>
          <div className={styles.segmented} role="radiogroup" aria-label="taker side">
            {RFQ_SIDES.map((s) => (
              <button
                key={s}
                type="button"
                role="radio"
                aria-checked={side === s}
                className={`${styles.segItem} ${side === s ? styles.segItemActive : ""}`}
                onClick={() => setSide(s)}
              >
                {sideLabel(s)}
              </button>
            ))}
          </div>
        </fieldset>

        <div className={styles.rfqFields}>
          {(arm === "ois" || arm === "irs") && (
            <>
              <Field label="Tenor (y)">
                <input
                  className={styles.input}
                  type="number"
                  min={1}
                  step={1}
                  value={tenorYears}
                  aria-label="swap tenor in years"
                  onChange={(e) => setTenorYears(e.target.value)}
                />
              </Field>
              <Field label="Fixed %">
                <input
                  className={styles.input}
                  type="number"
                  step={0.01}
                  value={fixedPct}
                  aria-label="fixed rate in percent"
                  onChange={(e) => setFixedPct(e.target.value)}
                />
              </Field>
            </>
          )}
          {arm === "fra" && (
            <>
              <Field label="Start (m)">
                <input
                  className={styles.input}
                  type="number"
                  min={0}
                  step={1}
                  value={startMonths}
                  aria-label="FRA start in months"
                  onChange={(e) => setStartMonths(e.target.value)}
                />
              </Field>
              <Field label="End (m)">
                <input
                  className={styles.input}
                  type="number"
                  min={1}
                  step={1}
                  value={endMonths}
                  aria-label="FRA end in months"
                  onChange={(e) => setEndMonths(e.target.value)}
                />
              </Field>
              <Field label="Fixed %">
                <input
                  className={styles.input}
                  type="number"
                  step={0.01}
                  value={fixedPct}
                  aria-label="FRA fixed rate in percent"
                  onChange={(e) => setFixedPct(e.target.value)}
                />
              </Field>
            </>
          )}
          {arm === "bond" && (
            <>
              <Field label="Coupon %">
                <input
                  className={styles.input}
                  type="number"
                  step={0.01}
                  value={couponPct}
                  aria-label="bond coupon in percent"
                  onChange={(e) => setCouponPct(e.target.value)}
                />
              </Field>
              <Field label="Maturity (y)">
                <input
                  className={styles.input}
                  type="number"
                  min={1}
                  step={1}
                  value={maturityYears}
                  aria-label="bond maturity in years"
                  onChange={(e) => setMaturityYears(e.target.value)}
                />
              </Field>
            </>
          )}
          <Field label="Notional (mm)">
            <input
              className={styles.input}
              type="number"
              min={1}
              step={5}
              value={notionalMm}
              aria-label="RFQ notional in millions"
              onChange={(e) => setNotionalMm(e.target.value)}
            />
          </Field>
        </div>

        <Button
          variant="primary"
          disabled={busy || !notionalValid || !canRequest}
          onClick={runRequest}
          title={canRequest ? undefined : requestDeniedTitle}
        >
          Request two-way — {trader}
        </Button>
      </div>

      {reqError && (
        <p className={styles.error} role="alert">
          {reqError}
        </p>
      )}

      {quote && (
        <section className={styles.quoteResult} aria-label="two-way quote">
          <h3 className={styles.cardTitle}>
            {ARM_LABELS[arm]} two-way {isPriceMarket ? "(clean price / 100)" : "(rate)"}
          </h3>
          <div className={styles.twoWay}>
            <div className={`${styles.twoWaySide} ${styles.bidSide}`}>
              <span className={styles.twoWayLabel}>BID</span>
              <span className={styles.twoWayValue}>{fmtLevel(quote.price.bid)}</span>
            </div>
            <div className={styles.twoWaySide}>
              <span className={styles.twoWayLabel}>MID</span>
              <span className={styles.twoWayValue}>
                {fmtLevel(0.5 * (quote.price.bid + quote.price.offer))}
              </span>
            </div>
            <div className={`${styles.twoWaySide} ${styles.offerSide}`}>
              <span className={styles.twoWayLabel}>OFFER</span>
              <span className={styles.twoWayValue}>{fmtLevel(quote.price.offer)}</span>
            </div>
          </div>
          <dl className={styles.metrics}>
            <Metric label="Par rate" value={fmtRate(quote.result.parRate)} emphatic />
            <Metric label="PV" value={fmtPnlAdaptive(quote.result.pv)} unit={curve.currency} />
            <Metric label="PV01" value={fmtPnlAdaptive(quote.result.pv01)} unit="/bp" />
            <Metric label="DV01" value={fmtPnlAdaptive(quote.result.dv01)} unit="/bp" />
          </dl>
          <dl className={styles.metrics}>
            <Metric label="Notional" value={fmtCompact(quote.notional)} />
            <Metric label="Key rates" value={`${quote.result.keyRateLadder.length}`} />
            <Metric label="Quote id" value={`${quote.quoteId}`} />
            <Metric label="Good until" value={fmtClock(quote.validUntilNanos)} />
          </dl>
        </section>
      )}
    </Panel>
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
        <Term label="Notional" value={fmtCompact(request.notional)} />
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
            <Metric label="Notional" value={fmtCompact(request.quote.notional)} />
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

/** A labelled form field: a `<label>` wrapping its label text + the control. */
function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}): React.ReactElement {
  return (
    <label className={styles.field}>
      <span className={styles.fieldLabel}>{label}</span>
      {children}
    </label>
  );
}

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
