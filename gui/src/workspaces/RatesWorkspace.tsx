/**
 * RatesWorkspace — the fixed-income (linear rates) pricing surface. A trader
 * builds an overnight-indexed swap (OIS) against the calibrated USD-SOFR curve
 * and requests a price: the present value, the par (fair fixed) rate, and the
 * first-order curve risk (PV01, DV01, and the key-rate DV01 ladder).
 *
 * One contract, two transports (GUI-DESIGN §6.2): the workspace talks ONLY to the
 * `CelnetTransport.priceRates` seam, so the SAME builder prices through the
 * deterministic in-app source (a genuine in-browser OIS bootstrap + discounting,
 * `src/data/ratesPricing.ts`) and through the live WebSocket `price_rates` mirror
 * to celnet-server — and cannot drift from the wire contract.
 */

import { useCallback, useMemo, useState } from "react";
import { useApp } from "../app/AppContext";
import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import { DataGrid } from "../components/DataGrid";
import type { ColumnDef } from "../lib/grid";
import { fmtPnlAdaptive } from "../lib/format";
import { DEFAULT_USD_SOFR_CURVE } from "../data/ratesPricing";
import type {
  OisDirection,
  OisInstrument,
  RatesPricingResult,
} from "../data/contract";
import styles from "./RatesWorkspace.module.css";

/** The standard quick-pick tenors — the calibrating pillar grid of the curve. */
const QUICK_TENORS: readonly number[] = DEFAULT_USD_SOFR_CURVE.pillars.map(
  (p) => p.tenorYears,
);

/** One row of the key-rate DV01 ladder (a curve pillar's bucketed DV01). */
interface LadderRow {
  readonly tenorYears: number;
  readonly dv01: number;
  /** This bucket's share of the total DV01 (percent); `0` when DV01 is ~0. */
  readonly sharePct: number;
}

const LADDER_COLUMNS: readonly ColumnDef<LadderRow>[] = [
  {
    key: "pillar",
    header: "Pillar",
    width: 96,
    align: "left",
    accessor: (r) => `${r.tenorYears}y`,
  },
  {
    key: "dv01",
    header: "Key-rate DV01",
    unit: "USD/bp",
    width: 160,
    accessor: (r) => fmtPnlAdaptive(r.dv01),
  },
  {
    key: "share",
    header: "% of DV01",
    width: 120,
    accessor: (r) => `${r.sharePct.toFixed(1)}%`,
  },
];

/** Format a decimal rate as a percentage with bp precision (0.0405 → "4.0500%"). */
function fmtRatePct(rate: number): string {
  return `${(rate * 100).toFixed(4)}%`;
}

export function RatesWorkspace(): React.ReactElement {
  const app = useApp();
  const curve = DEFAULT_USD_SOFR_CURVE;

  const [direction, setDirection] = useState<OisDirection>("RECEIVE_FIXED");
  const [tenorYears, setTenorYears] = useState(5);
  const [fixedRatePct, setFixedRatePct] = useState(4.05);
  const [notionalMm, setNotionalMm] = useState(100);
  const [result, setResult] = useState<RatesPricingResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The transport label tells the trader which engine priced this (offline in-app
  // bootstrap vs the live `price_rates` mirror); both compute the SAME real OIS.
  const isOffline = !app.transport.label.startsWith("live");

  const tenorValid = Number.isInteger(tenorYears) && tenorYears >= 1;
  const notionalValid = notionalMm > 0;
  const canPrice = tenorValid && notionalValid && !busy;

  const requestQuote = useCallback(async () => {
    if (!tenorValid || !notionalValid) return;
    setBusy(true);
    setError(null);
    const instrument: OisInstrument = {
      tenorYears,
      fixedRate: fixedRatePct / 100,
      notional: notionalMm * 1_000_000,
      direction,
    };
    try {
      const priced = await app.transport.priceRates(curve, instrument);
      setResult(priced);
    } catch (err) {
      // A pricing failure (server refusal, transport deadline, or an offline
      // validation throw) is a real, surfaced error — never a fabricated price.
      setResult(null);
      setError(err instanceof Error ? err.message : "pricing failed");
    } finally {
      setBusy(false);
    }
  }, [app.transport, curve, direction, fixedRatePct, notionalMm, tenorYears, tenorValid, notionalValid]);

  // Set the fixed rate to the par rate just priced (the zero-PV breakeven coupon).
  const setToPar = useCallback(() => {
    if (result) setFixedRatePct(Number((result.parRate * 100).toFixed(4)));
  }, [result]);

  const ladder = useMemo<LadderRow[]>(() => {
    if (!result) return [];
    const total = Math.abs(result.dv01);
    return curve.pillars.map((p, i) => {
      const dv01 = result.keyRateLadder[i] ?? 0;
      return {
        tenorYears: p.tenorYears,
        dv01,
        sharePct: total > 0 ? (dv01 / result.dv01) * 100 : 0,
      };
    });
  }, [curve.pillars, result]);

  const ladderGroups = useMemo(
    () => [
      {
        key: "",
        label: "",
        rows: ladder.map((r) => ({ key: String(r.tenorYears), datum: r })),
      },
    ],
    [ladder],
  );

  return (
    <div className={styles.wrap}>
      <Panel material="float" className={styles.builder} title="OIS ticket">
        <div className={styles.curveRow}>
          <span className={styles.curveLabel}>Curve</span>
          <span className={styles.curveName}>{curve.currency}-SOFR</span>
          <span className={styles.curveMeta}>
            {curve.pillars.length} pillars · ref {curve.referenceDate.year}-
            {String(curve.referenceDate.month).padStart(2, "0")}-
            {String(curve.referenceDate.day).padStart(2, "0")} · self-discounting
          </span>
        </div>

        <div className={styles.field}>
          <span className={styles.fieldLabel}>Direction</span>
          <div className={styles.toggle} role="tablist" aria-label="swap direction">
            <button
              type="button"
              role="tab"
              aria-selected={direction === "RECEIVE_FIXED"}
              className={`${styles.toggleTab} ${direction === "RECEIVE_FIXED" ? styles.toggleActive : ""}`}
              onClick={() => setDirection("RECEIVE_FIXED")}
            >
              Receive fixed
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={direction === "PAY_FIXED"}
              className={`${styles.toggleTab} ${direction === "PAY_FIXED" ? styles.toggleActive : ""}`}
              onClick={() => setDirection("PAY_FIXED")}
            >
              Pay fixed
            </button>
          </div>
        </div>

        <div className={styles.field}>
          <span className={styles.fieldLabel}>Tenor</span>
          <div className={styles.tenorPicks}>
            {QUICK_TENORS.map((t) => (
              <button
                key={t}
                type="button"
                className={`${styles.tenorPill} ${tenorYears === t ? styles.tenorActive : ""}`}
                onClick={() => setTenorYears(t)}
                aria-pressed={tenorYears === t}
              >
                {t}y
              </button>
            ))}
            <label className={styles.inlineInput}>
              <input
                type="number"
                min={1}
                step={1}
                value={tenorYears}
                aria-label="swap tenor in years"
                onChange={(e) => setTenorYears(Math.trunc(Number(e.target.value)))}
              />
              <span className={styles.inputUnit}>y</span>
            </label>
          </div>
        </div>

        <div className={styles.fieldRow}>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>Fixed rate</span>
            <span className={styles.inlineInput}>
              <input
                type="number"
                step={0.01}
                value={fixedRatePct}
                aria-label="fixed rate in percent"
                onChange={(e) => setFixedRatePct(Number(e.target.value))}
              />
              <span className={styles.inputUnit}>%</span>
            </span>
          </label>
          <label className={styles.field}>
            <span className={styles.fieldLabel}>Notional</span>
            <span className={styles.inlineInput}>
              <input
                type="number"
                min={0}
                step={5}
                value={notionalMm}
                aria-label="notional in millions"
                onChange={(e) => setNotionalMm(Number(e.target.value))}
              />
              <span className={styles.inputUnit}>mm</span>
            </span>
          </label>
        </div>

        <div className={styles.actions}>
          <Button variant="primary" onClick={requestQuote} disabled={!canPrice}>
            {busy ? "Pricing…" : "Request quote"}
          </Button>
          {result && (
            <Button variant="ghost" onClick={setToPar} title="set the fixed rate to the par (breakeven) rate">
              Set to par
            </Button>
          )}
          <span className={styles.engine}>{isOffline ? "in-app pricer" : "live edge"}</span>
        </div>

        {error && (
          <p className={styles.error} role="alert">
            {error}
          </p>
        )}
      </Panel>

      <Panel className={styles.results} title="Price & risk">
        {result ? (
          <>
            <dl className={styles.metrics}>
              <Metric label="PV" value={fmtPnlAdaptive(result.pv)} unit="USD" emphatic />
              <Metric label="Par rate" value={fmtRatePct(result.parRate)} />
              <Metric label="PV01" value={fmtPnlAdaptive(result.pv01)} unit="USD/bp" />
              <Metric label="DV01" value={fmtPnlAdaptive(result.dv01)} unit="USD/bp" />
            </dl>
            <div className={styles.ladder}>
              <h3 className={styles.ladderTitle}>Key-rate DV01 ladder</h3>
              <DataGrid
                label="key-rate DV01 ladder"
                columns={LADDER_COLUMNS}
                groups={ladderGroups}
              />
            </div>
          </>
        ) : (
          <p className={styles.empty}>
            Build an OIS and request a quote to price the swap and its curve risk.
          </p>
        )}
      </Panel>
    </div>
  );
}

/** One headline measure: a labelled term/value pair in the results strip. */
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
