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
import { fmtClock, fmtCompact, fmtPnlAdaptive, fmtSigned } from "../lib/format";
import { oisSpec, irsSpec, fraSpec, bondSpec } from "../products";
import styles from "./FiStreamingWorkspace.module.css";

import { MagnitudeField } from "../components/MagnitudeField";

/** The FI instrument families this hub can stream — the FOUR rates families from
 *  the Ticket product registry (OIS / IRS / FRA / Bond), each an additive
 *  `RatesInstrument` oneof arm. `RatesInstrument["kind"]` sans the arms this hub
 *  builds is exactly this set, so the streamed line prices through the SAME
 *  `priceRates` seam the Ticket uses. */
type RfsKind = Extract<RatesInstrument["kind"], "ois" | "irs" | "fra" | "bond">;

/**
 * The instrument-selector families — REUSED straight from the shared product
 * registry (`gui/src/products`), NOT re-declared here: each row carries its
 * canonical `id` / `label` / `summary` from the same {@link RatesProductSpec}
 * the Ticket gallery renders, so the selector and the Ticket can never drift.
 * The `kind` maps the spec onto the `RatesInstrument` oneof arm this hub streams.
 */
const FI_FAMILIES = [
  { kind: "ois" as const, spec: oisSpec },
  { kind: "irs" as const, spec: irsSpec },
  { kind: "fra" as const, spec: fraSpec },
  { kind: "bond" as const, spec: bondSpec },
] as const;
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
  if (kind === "fra") {
    // The tenor pillar selects the FORWARD-START point; the streamed FRA is the
    // standard 3-month rate starting there (e.g. tenor 2Y → a 24×27 FRA — the 3M
    // rate fixing in 2Y). The pillar par rate seeds a near-ATM fixed strike K.
    const startMonths = tenor * 12;
    return {
      kind: "fra",
      fra: {
        startMonths,
        endMonths: startMonths + 3,
        fixedRate,
        notional,
        direction: oisDirection(swapDir),
        accrualBasis: "ACT_360",
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
  // A FRA carries a forward window, not a single tenor: label it as its 3M window
  // starting at the pillar (e.g. "FRA 2Y×3M pay") — never a bare "FRA 2Y".
  if (kind === "fra") return `FRA ${tenor}Y×3M ${side}`;
  return `${kind.toUpperCase()} ${tenor}Y ${side}`;
}

/** The tenor/maturity token for the blotter's Tenor column, per instrument arm. */
function instrumentTenor(instrument: RatesInstrument): string {
  switch (instrument.kind) {
    case "ois":
      return `${instrument.ois.tenorYears}Y`;
    case "irs":
      return `${instrument.irs.tenorYears}Y`;
    case "fra": {
      // A FRA is a forward window, not a point tenor: show start×end in months.
      const { startMonths, endMonths } = instrument.fra;
      return `${startMonths}×${endMonths}M`;
    }
    case "bond":
      return `${instrument.bond.maturityDate.year} mat`;
  }
}

interface RatesPreset {
  id: string;
  label: string;
  instrument: RatesInstrument;
}

/** The default streamable FI lines — one per family (OIS / IRS / FRA swaps + a cash bond). */
function defaultPresets(curve: typeof DEFAULT_USD_SOFR_CURVE): RatesPreset[] {
  return [
    { id: "ois2y", label: "OIS 2Y rec", instrument: buildRfsInstrument("ois", 2, DEFAULT_NOTIONAL, "RECEIVE", "BUY", curve) },
    { id: "ois10y", label: "OIS 10Y pay", instrument: buildRfsInstrument("ois", 10, DEFAULT_NOTIONAL, "PAY", "BUY", curve) },
    { id: "irs5y", label: "IRS 5Y pay", instrument: buildRfsInstrument("irs", 5, DEFAULT_NOTIONAL, "PAY", "BUY", curve) },
    { id: "fra2y", label: "FRA 2Y×3M pay", instrument: buildRfsInstrument("fra", 2, DEFAULT_NOTIONAL, "PAY", "BUY", curve) },
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
  // DEALER-STYLE RATES BLOTTER ROW — each cell maps to a REAL streamed field of
  // `RatesStreamRow` (from `app.stream.ratesRows`). Column → source field:
  //   Instrument → row.label + row.kind (arm badge)
  //   Tenor      → row.instrument (tenorYears / FRA window / bond maturity)
  //   Mid (rate) → row.result.parRate   (the fair/par rate = the indicative MID)
  //   PV         → row.result.pv
  //   PV01       → row.result.pv01
  //   DV01       → row.result.dv01
  //   Δbp        → row.curveShift (parallel curve shift this tick, ×1e4 → bp)
  //   Trend      → row.pvHistory (PV sparkline)
  //   Updated    → row.epochNanos (last-tick wall-clock time)
  // GAP: the FI rates stream (`RatesPricingResult`) carries only an indicative
  // MID (par rate) — there is NO dealer two-way bid/offer and NO bid/ask SIZE on
  // the one contract. So this blotter streams the honest MID and does NOT
  // fabricate a Bid/Offer spread or a size column; when the desk contract grows a
  // streamed two-way ladder (bid/offer/size), add those columns here off the real
  // fields. (Executable two-way is the desk RFQ path in the sidebar.)
  return (
    <div className={styles.row} role="row">
      <span className={styles.instr} role="cell">
        <span className={styles.arm} aria-label={`${ratesArmBadge(row.kind)} instrument`}>
          {ratesArmBadge(row.kind)}
        </span>
        <span className={styles.instrLabel}>{row.label}</span>
      </span>
      <span className={styles.tenorCell} role="cell" title="Tenor / forward window / maturity">
        {instrumentTenor(row.instrument)}
      </span>
      <span className={`num ${styles.numCell}`} role="cell" title="Indicative MID rate (par / fair fixed rate) — no dealer two-way on the FI stream">
        {fmtParPct(row.result.parRate)}
      </span>
      <span className={`num ${styles.numCell}`} role="cell" title="Present value (curve ccy)">
        {fmtPnlAdaptive(row.result.pv)}
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
      <span className={`num ${styles.updatedCell}`} role="cell" title="Last tick (wall-clock)">
        {fmtClock(row.epochNanos)}
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

  // The primary instrument selector: picking a family (a) makes it the active RFS
  // arm and (b) immediately STREAMS a live line for it at the current tenor / side /
  // notional — the streaming-first "primary FI surface" gesture. It builds the SAME
  // `RatesInstrument` (via buildRfsInstrument) the sidebar Request + presets use and
  // opens the line through the SAME `subscribeRates` path — no side channel.
  const onSelectFamily = (next: RfsKind): void => {
    setKind(next);
    if (!streamable || !notionalValid) return;
    const instrument = buildRfsInstrument(next, tenor, notional, swapDir, bondSide, curve);
    app.stream.subscribeRates(instrument, curve, rfsLabel(next, tenor, swapDir, bondSide));
  };

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
              indicative mid + risk · re-priced each tick vs the USD-SOFR curve ·{" "}
              <abbr title="request for quote / voice desk">RFQ/desk</abbr> for an
              executable two-way
            </span>
            {!streamable && (
              <span className={styles.lock} title={lockTitle}>
                <span aria-hidden="true">🔒</span> {lockTitle}
              </span>
            )}
          </div>

          {streamable && (
            <div className={styles.selector} role="group" aria-label="stream instrument family">
              <span className={styles.controlLabel}>Instrument</span>
              {FI_FAMILIES.map((f) => (
                <button
                  key={f.spec.id}
                  type="button"
                  className={`${styles.segBtn} ${kind === f.kind ? styles.segBtnActive : ""}`}
                  aria-pressed={kind === f.kind}
                  onClick={() => onSelectFamily(f.kind)}
                  title={f.spec.summary}
                >
                  {f.spec.label}
                </button>
              ))}
            </div>
          )}

          {streamable && (
            <div className={styles.presets} role="group" aria-label="stream a preset line">
              <span className={styles.controlLabel}>Quick lines</span>
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
                No streaming lines. Pick an instrument above, or request one on the right,
                to stream its live mid + risk.
              </div>
            ) : (
              <div className={styles.grid} role="table" aria-label="fixed-income streaming lines">
                {/* Dealer-style two-way rates blotter header. The FI stream carries an
                    indicative MID only (no dealer bid/offer or size on the contract),
                    so Mid is the honest streamed price — see the GAP note in RatesLineRow. */}
                <div className={styles.headerRow} role="row">
                  <span className={styles.colLeft} role="columnheader">
                    Instrument
                  </span>
                  <span className={styles.colTenor} role="columnheader">
                    Tenor
                  </span>
                  <span className={`num ${styles.colNum}`} role="columnheader" title="Indicative mid (par) rate — no dealer two-way on the FI stream">
                    Mid
                  </span>
                  <span className={`num ${styles.colNum}`} role="columnheader">
                    PV
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
                  <span className={styles.colUpdated} role="columnheader" title="Last tick (wall-clock)">
                    Updated
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
              {/* The SAME four registry families as the primary selector (shared
                  `kind` state); short arm badges here (the full labels are up top)
                  — the sidebar refines the active family's RFQ request, no auto-stream. */}
              <div className={styles.seg} role="group" aria-label="instrument type">
                {FI_FAMILIES.map((f) => (
                  <button
                    key={f.spec.id}
                    type="button"
                    className={`${styles.segBtn} ${kind === f.kind ? styles.segBtnActive : ""}`}
                    aria-pressed={kind === f.kind}
                    onClick={() => setKind(f.kind)}
                    title={f.spec.label}
                  >
                    {ratesArmBadge(f.kind)}
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
              <MagnitudeField
                id="rfs-notional"
                className={styles.input}
                min={0}
                step={1_000_000}
                allowBlank={false}
                value={Number.isFinite(notional) ? notional : null}
                onCommit={(v) => {
                  if (v !== null) setNotional(v);
                }}
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
