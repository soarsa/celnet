/**
 * FiStreamingWorkspace — the Fixed-Income streaming desk (W2). A two-column
 * workspace under the Fixed Income domain ONLY (registered FI-only in `RAIL`, so
 * `workspaceDomains("fistreaming") === ["fixed_income"]` and it never appears under
 * the FX tab):
 *
 *   MAIN (left/centre): the LIVE streaming grid of bond & swap prices over
 *   `app.stream.ratesRows` — each line re-prices every tick against the baseline
 *   USD-SOFR curve shifted by a deterministic ±1bp parallel move, showing the
 *   indicative PV + first-order risk (PV / par / PV01 / DV01 / Δbp) with a PV
 *   sparkline. Modelled on `StreamWorkspace`'s `RatesStreamPanel` / `RatesLineRow`.
 *   Default preset lines (OIS / IRS swaps + a cash bond) the trader can stream
 *   via `app.stream.subscribeRates`.
 *
 *   SIDEBAR (right): the RFS request panel — "request a price from the market" for
 *   FI. A form (instrument type Bond / Swap[OIS·IRS], tenor, size/notional, side)
 *   whose "Request price" builds the matching `RatesInstrument` and calls
 *   `app.stream.subscribeRates(...)` to OPEN a live RFS streaming line; the streamed
 *   indicative price (par rate + PV) for that requested line is shown HONESTLY as an
 *   indicative RFS quote — the FI stream carries an indicative PV + par, NOT a
 *   dealer two-way bid/offer, so no fake bid/offer spread is fabricated (par/PV are
 *   shown as returned and labelled "indicative").
 *
 * EXECUTE — honest FI trade path (CLAUDE.md rule 2, no fabricated execution): there
 * is NO stream-token execute for rates (`app.stream.execute` is the FX click-to-
 * trade path, FX-only). The genuine EXISTING FI execution path is the desk RFQ:
 * `app.transport.submitDeskRequest(...)`. So the Execute action routes the risk
 * trade through the desk RFQ path. The desk RFQ wire's instrument is an
 * `OisInstrument` (the contract's rates desk arm), so Execute is enabled for the
 * OIS swap type and present-but-disabled (with an honest reason) for the IRS/bond
 * types the desk RFQ contract cannot represent — never a faked booking.
 *
 * Entitlement/license: FI streaming is gated on `stream·fixed_income` exactly as
 * `RatesStreamPanel` — present-but-locked when unlicensed/denied, never hidden,
 * never faked. Execute is additionally gated on `execute·fixed_income`.
 */

import { useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import { capabilityDenialTitle } from "../lib/capabilityMatrix";
import { configuredLicense, LICENSE_UPSELL_TITLE, type LicensePredicate } from "../lib/commands";
import { Button } from "../components/Button";
import { Sparkline, sparklineDirection } from "../components/Sparkline";
import { DEFAULT_USD_SOFR_CURVE } from "../data/ratesPricing";
import type {
  OisDirection,
  OisInstrument,
  RatesInstrument,
  Side,
  SubmitDeskRequestRequest,
} from "../data/contract";
import type { RatesStreamRow } from "../hooks/useStreamSession";
import { fmtCompact, fmtPnlAdaptive, fmtSigned } from "../lib/format";
import styles from "./FiStreamingWorkspace.module.css";

/** The RFS instrument families the sidebar can request (Swap · OIS / IRS, or Bond). */
type RfsKind = "ois" | "irs" | "bond";
/** A swap side (Pay/Receive fixed); a bond side (Buy/Sell). */
type SwapDir = "PAY" | "RECEIVE";
type BondSide = "BUY" | "SELL";

/** The tenor pillars the RFS form offers (all present on the default USD-SOFR curve). */
const TENORS: readonly number[] = [2, 5, 10, 30];
/** The default RFS notional (curve currency). */
const DEFAULT_NOTIONAL = 50_000_000;

/** The par (fair fixed) rate at a whole-year curve pillar, or a sane fallback. */
function pillarPar(curve: typeof DEFAULT_USD_SOFR_CURVE, years: number): number {
  const p = curve.pillars.find((q) => q.tenor.kind === "years" && q.tenor.years === years);
  return p ? p.parRate : 0.04;
}

/** A par (fair fixed) rate rendered as a percentage, e.g. 0.0409 → "4.090%". */
function fmtParPct(rate: number): string {
  return `${(rate * 100).toFixed(3)}%`;
}

/** A short arm badge for a fixed-income row ("OIS" / "IRS" / "BOND"). */
function ratesArmBadge(kind: RatesStreamRow["kind"]): string {
  return kind.toUpperCase();
}

/** The `OisDirection` for a swap side. */
function oisDirection(dir: SwapDir): OisDirection {
  return dir === "PAY" ? "PAY_FIXED" : "RECEIVE_FIXED";
}

/**
 * Build the exact `RatesInstrument` for the current RFS form — the SAME shapes
 * `StreamWorkspace`'s presets use, so the requested line prices through the identical
 * `price_rates` mirror the rates unary edge uses. `subscribeRates` opens the live line.
 */
function buildRfsInstrument(
  kind: RfsKind,
  tenor: number,
  notional: number,
  swapDir: SwapDir,
  bondSide: BondSide,
  curve: typeof DEFAULT_USD_SOFR_CURVE,
): RatesInstrument {
  const fixedRate = pillarPar(curve, tenor);
  if (kind === "ois") {
    return {
      kind: "ois",
      ois: { tenorYears: tenor, fixedRate, notional, direction: oisDirection(swapDir) },
    };
  }
  if (kind === "irs") {
    return {
      kind: "irs",
      irs: {
        tenorYears: tenor,
        fixedRate,
        notional,
        direction: oisDirection(swapDir),
        fixedFrequency: "SEMI_ANNUAL",
        fixedDayCount: "ACT_360",
        floatFrequency: "QUARTERLY",
        floatDayCount: "ACT_360",
      },
    };
  }
  const ref = curve.referenceDate;
  return {
    kind: "bond",
    bond: {
      couponRate: fixedRate,
      couponFrequency: "SEMI_ANNUAL",
      dayCount: "THIRTY_360_BOND_BASIS",
      maturityDate: { year: ref.year + tenor, month: ref.month, day: ref.day },
      redemption: 100,
      position: bondSide === "BUY" ? "LONG" : "SHORT",
    },
  };
}

/** The human label a requested/preset line carries in the blotter. */
function rfsLabel(kind: RfsKind, tenor: number, swapDir: SwapDir, bondSide: BondSide): string {
  if (kind === "bond") return `Bond ${tenor}Y ${bondSide === "BUY" ? "long" : "short"}`;
  const side = swapDir === "PAY" ? "pay" : "rec";
  return `${kind.toUpperCase()} ${tenor}Y ${side}`;
}

interface RatesPreset {
  id: string;
  label: string;
  instrument: RatesInstrument;
}

/** The default streamable FI lines (OIS / IRS swaps + a cash bond). */
function defaultPresets(curve: typeof DEFAULT_USD_SOFR_CURVE): RatesPreset[] {
  return [
    { id: "ois2y", label: "OIS 2Y rec", instrument: buildRfsInstrument("ois", 2, DEFAULT_NOTIONAL, "RECEIVE", "BUY", curve) },
    { id: "ois10y", label: "OIS 10Y pay", instrument: buildRfsInstrument("ois", 10, DEFAULT_NOTIONAL, "PAY", "BUY", curve) },
    { id: "irs5y", label: "IRS 5Y pay", instrument: buildRfsInstrument("irs", 5, DEFAULT_NOTIONAL, "PAY", "BUY", curve) },
    { id: "bond10y", label: "Bond 10Y long", instrument: buildRfsInstrument("bond", 10, DEFAULT_NOTIONAL, "PAY", "BUY", curve) },
  ];
}

/** One live fixed-income streaming row (PV / par / PV01 / DV01 / Δbp + PV trend). */
function RatesLineRow({
  row,
  onRemove,
}: {
  row: RatesStreamRow;
  onRemove: () => void;
}): React.ReactElement {
  const pvDir = sparklineDirection(row.pvHistory);
  const hasTrend = row.pvHistory.length >= 2;
  return (
    <div className={styles.row} role="row">
      <span className={styles.instr} role="cell">
        <span className={styles.arm} aria-label={`${ratesArmBadge(row.kind)} instrument`}>
          {ratesArmBadge(row.kind)}
        </span>
        <span className={styles.instrLabel}>{row.label}</span>
      </span>
      <span className={`num ${styles.numCell}`} role="cell" title="Present value (curve ccy)">
        {fmtPnlAdaptive(row.result.pv)}
      </span>
      <span className={`num ${styles.numCell}`} role="cell" title="Par (fair fixed) rate">
        {fmtParPct(row.result.parRate)}
      </span>
      <span className={`num ${styles.numCell}`} role="cell" title="Analytic PV01 (PV per 1bp of the fixed rate)">
        {fmtPnlAdaptive(row.result.pv01)}
      </span>
      <span className={`num ${styles.numCell}`} role="cell" title="DV01 (PV per +1bp parallel curve bump)">
        {fmtPnlAdaptive(row.result.dv01)}
      </span>
      <span className={`num ${styles.numCell}`} role="cell" title="Parallel curve shift this tick (bp)">
        {fmtSigned(row.curveShift * 1e4, 2)}
      </span>
      <span className={styles.trend} role="cell">
        {hasTrend ? (
          <Sparkline values={row.pvHistory} direction={pvDir} ariaLabel={`PV trend, ${pvDir}`} />
        ) : (
          <span className={styles.await} aria-label="awaiting ticks">
            …
          </span>
        )}
      </span>
      <span className={styles.actions} role="cell">
        <button
          type="button"
          className={styles.remove}
          onClick={onRemove}
          aria-label={`Stop streaming ${row.label}`}
          title="Stop streaming this line"
        >
          ✕
        </button>
      </span>
    </div>
  );
}

export function FiStreamingWorkspace(): React.ReactElement {
  const app = useApp();
  const licensed: LicensePredicate = useMemo(() => configuredLicense(), []);
  const canStream = app.auth.can("stream", "fixed_income");
  const isLicensed = licensed("fixed_income");
  const streamable = canStream && isLicensed;
  const lockTitle = !canStream
    ? capabilityDenialTitle("stream", "fixed_income")
    : !isLicensed
      ? LICENSE_UPSELL_TITLE
      : undefined;

  const curve = DEFAULT_USD_SOFR_CURVE;
  const presets = useMemo(() => defaultPresets(curve), [curve]);
  const rows = app.stream.ratesRows;

  // The RFS request form state.
  const [kind, setKind] = useState<RfsKind>("ois");
  const [tenor, setTenor] = useState<number>(5);
  const [notional, setNotional] = useState<number>(DEFAULT_NOTIONAL);
  const [swapDir, setSwapDir] = useState<SwapDir>("PAY");
  const [bondSide, setBondSide] = useState<BondSide>("BUY");
  // The last requested RFS line's subscription id (its indicative price is shown).
  const [requestedId, setRequestedId] = useState<bigint | null>(null);
  const [execStatus, setExecStatus] = useState<string | null>(null);

  const notionalValid = Number.isFinite(notional) && notional > 0;

  // The streamed indicative line for the last RFS request (or undefined until it
  // materializes / after it is torn down). Never fabricated — a pure lookup.
  const requestedRow = useMemo(
    () => (requestedId === null ? undefined : rows.find((r) => r.subscriptionId === requestedId)),
    [rows, requestedId],
  );

  // Execute routes the risk trade through the desk RFQ path. The desk RFQ wire's
  // instrument is an OisInstrument, so it can only represent the OIS swap arm — IRS
  // and bond are honestly unsupported on the desk contract (never a faked booking).
  const canExecute = app.auth.can("execute", "fixed_income");
  const executeSupported = kind === "ois";
  const executeReady = streamable && canExecute && executeSupported && requestedRow !== undefined;
  const executeTitle = !canExecute
    ? capabilityDenialTitle("execute", "fixed_income")
    : !executeSupported
      ? "Desk RFQ books OIS swaps — IRS/bond desk execution is not on the contract"
      : requestedRow === undefined
        ? "Request a price first, then execute the risk trade"
        : "Submit the risk trade to the rates desk (RFQ)";

  const onRequest = (): void => {
    if (!streamable || !notionalValid) return;
    const instrument = buildRfsInstrument(kind, tenor, notional, swapDir, bondSide, curve);
    const label = rfsLabel(kind, tenor, swapDir, bondSide);
    const id = app.stream.subscribeRates(instrument, curve, label);
    setRequestedId(id);
    setExecStatus(null);
  };

  const onExecute = (): void => {
    if (!executeReady) return;
    // The genuine EXISTING FI execution path — the desk RFQ (submitDeskRequest),
    // NOT a fabricated stream execute. Build the OisInstrument the desk contract
    // prices; BUY = pay fixed, SELL = receive fixed (per the wire `Side`).
    const instrument: OisInstrument = {
      tenorYears: tenor,
      fixedRate: pillarPar(curve, tenor),
      notional,
      direction: oisDirection(swapDir),
    };
    const side: Side = swapDir === "PAY" ? "BUY" : "SELL";
    const request: SubmitDeskRequestRequest = {
      kind: "RFQ",
      counterparty: "celnet-gui",
      desk: "rates",
      instrument,
      curveSet: curve,
      side,
      notional,
      ttlMs: 30_000,
    };
    setExecStatus("Submitting to rates desk…");
    void app.transport
      .submitDeskRequest(request)
      .then((res) =>
        setExecStatus(`Sent to rates desk · ${res?.request?.state?.toLowerCase() ?? "pending"}`),
      )
      .catch(() => setExecStatus("Desk RFQ submission failed — retry"));
  };

  const sideControl =
    kind === "bond" ? (
      <div className={styles.seg} role="group" aria-label="bond side">
        {(["BUY", "SELL"] as const).map((s) => (
          <button
            key={s}
            type="button"
            className={`${styles.segBtn} ${bondSide === s ? styles.segBtnActive : ""}`}
            aria-pressed={bondSide === s}
            onClick={() => setBondSide(s)}
          >
            {s === "BUY" ? "Buy" : "Sell"}
          </button>
        ))}
      </div>
    ) : (
      <div className={styles.seg} role="group" aria-label="swap side">
        {(["PAY", "RECEIVE"] as const).map((s) => (
          <button
            key={s}
            type="button"
            className={`${styles.segBtn} ${swapDir === s ? styles.segBtnActive : ""}`}
            aria-pressed={swapDir === s}
            onClick={() => setSwapDir(s)}
          >
            {s === "PAY" ? "Pay" : "Receive"}
          </button>
        ))}
      </div>
    );

  return (
    <div className={styles.wrap}>
      <div className={styles.layout}>
        {/* MAIN — the live streaming grid of bond & swap prices. */}
        <section className={styles.main} aria-label="live fixed-income streaming prices">
          <div className={styles.head}>
            <span className={styles.title}>Fixed income — live streaming</span>
            <span className={styles.note}>
              indicative PV + risk · re-priced each tick vs the USD-SOFR curve ·{" "}
              <abbr title="request for quote / voice desk">RFQ/desk</abbr> to trade
            </span>
            {!streamable && (
              <span className={styles.lock} title={lockTitle}>
                <span aria-hidden="true">🔒</span> {lockTitle}
              </span>
            )}
          </div>

          {streamable && (
            <div className={styles.presets} role="group" aria-label="stream a preset line">
              <span className={styles.controlLabel}>Stream</span>
              {presets.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  className={styles.segBtn}
                  onClick={() => app.stream.subscribeRates(p.instrument, curve, p.label)}
                  title={`Stream ${p.label} against the USD-SOFR curve`}
                >
                  {p.label}
                </button>
              ))}
            </div>
          )}

          {streamable ? (
            rows.length === 0 ? (
              <div className={styles.empty}>
                No streaming lines. Pick a preset above, or request one on the right, to
                stream its live PV + risk.
              </div>
            ) : (
              <div className={styles.grid} role="table" aria-label="fixed-income streaming lines">
                <div className={styles.headerRow} role="row">
                  <span className={styles.colLeft} role="columnheader">
                    Instrument
                  </span>
                  <span className={`num ${styles.colNum}`} role="columnheader">
                    PV
                  </span>
                  <span className={`num ${styles.colNum}`} role="columnheader">
                    Par
                  </span>
                  <span className={`num ${styles.colNum}`} role="columnheader">
                    PV01
                  </span>
                  <span className={`num ${styles.colNum}`} role="columnheader">
                    DV01
                  </span>
                  <span className={`num ${styles.colNum}`} role="columnheader" title="Parallel curve shift (bp)">
                    Δbp
                  </span>
                  <span className={styles.colTrend} role="columnheader">
                    PV trend
                  </span>
                  <span className={styles.colActions} role="columnheader" aria-label="actions" />
                </div>
                {rows.map((row) => (
                  <RatesLineRow
                    key={row.subscriptionId.toString()}
                    row={row}
                    onRemove={() => app.stream.unsubscribeRates(row.subscriptionId)}
                  />
                ))}
              </div>
            )
          ) : (
            <div className={styles.empty}>
              Fixed-income streaming is{" "}
              {canStream ? "not licensed for this firm" : "not enabled for your role"}.
            </div>
          )}
        </section>

        {/* SIDEBAR — the RFS request panel ("request a price from the market"). */}
        <aside className={styles.sidebar} aria-label="RFS — request a price">
          <div className={styles.rfsHead}>
            <span className={styles.rfsTitle}>RFS · request price</span>
            <span className={styles.rfsNote}>Open a live indicative stream to risk-trade.</span>
          </div>

          <fieldset className={styles.form} disabled={!streamable}>
            <legend className={styles.srOnly}>request-for-stream form</legend>

            <div className={styles.field}>
              <span className={styles.fieldLabel}>Instrument</span>
              <div className={styles.seg} role="group" aria-label="instrument type">
                {(
                  [
                    ["ois", "Swap · OIS"],
                    ["irs", "Swap · IRS"],
                    ["bond", "Bond"],
                  ] as const
                ).map(([k, label]) => (
                  <button
                    key={k}
                    type="button"
                    className={`${styles.segBtn} ${kind === k ? styles.segBtnActive : ""}`}
                    aria-pressed={kind === k}
                    onClick={() => setKind(k)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>

            <label className={styles.field} htmlFor="rfs-tenor">
              <span className={styles.fieldLabel}>Tenor</span>
              <select
                id="rfs-tenor"
                className={styles.select}
                value={tenor}
                onChange={(e) => setTenor(Number(e.target.value))}
              >
                {TENORS.map((t) => (
                  <option key={t} value={t}>
                    {t}Y
                  </option>
                ))}
              </select>
            </label>

            <label className={styles.field} htmlFor="rfs-notional">
              <span className={styles.fieldLabel}>
                Size <span className={styles.fieldHint}>{fmtCompact(notional)} notional</span>
              </span>
              <input
                id="rfs-notional"
                className={styles.input}
                type="number"
                min={0}
                step={1_000_000}
                value={Number.isFinite(notional) ? notional : ""}
                onChange={(e) => setNotional(Number(e.target.value))}
                aria-invalid={!notionalValid}
              />
            </label>

            <div className={styles.field}>
              <span className={styles.fieldLabel}>Side</span>
              {sideControl}
            </div>

            <Button
              variant="primary"
              onClick={onRequest}
              disabled={!streamable || !notionalValid}
            >
              Request price
            </Button>
          </fieldset>

          {/* The streamed indicative price for the requested line — shown HONESTLY as
              indicative (par + PV as returned; the FI stream carries no dealer
              bid/offer two-way, so none is fabricated). */}
          <div className={styles.readout} aria-live="polite">
            {requestedRow ? (
              <>
                <div className={styles.readoutHead}>
                  <span className={styles.readoutLabel}>{requestedRow.label}</span>
                  <span className={styles.indicative}>indicative</span>
                </div>
                <dl className={styles.readoutGrid}>
                  <div className={styles.readoutItem}>
                    <dt>Par</dt>
                    <dd className="num">{fmtParPct(requestedRow.result.parRate)}</dd>
                  </div>
                  <div className={styles.readoutItem}>
                    <dt>PV</dt>
                    <dd className="num">{fmtPnlAdaptive(requestedRow.result.pv)}</dd>
                  </div>
                  <div className={styles.readoutItem}>
                    <dt>DV01</dt>
                    <dd className="num">{fmtPnlAdaptive(requestedRow.result.dv01)}</dd>
                  </div>
                </dl>
              </>
            ) : (
              <div className={styles.readoutEmpty}>
                No live request. Fill the form and request a price to see the indicative
                stream.
              </div>
            )}
          </div>

          {/* Execute — the risk trade routes through the EXISTING FI desk RFQ path
              (submitDeskRequest), NOT a fabricated stream execute. */}
          <div className={styles.execBlock}>
            <Button variant="primary" onClick={onExecute} disabled={!executeReady} title={executeTitle}>
              Execute (desk RFQ)
            </Button>
            <p className={styles.execNote} title={executeTitle}>
              FI risk-trade execution routes through the desk{" "}
              <abbr title="request for quote">RFQ</abbr> path — the honest existing FI
              execute. The desk RFQ books OIS swaps.
            </p>
            {execStatus && (
              <p className={styles.execStatus} role="status">
                {execStatus}
              </p>
            )}
          </div>
        </aside>
      </div>
    </div>
  );
}
