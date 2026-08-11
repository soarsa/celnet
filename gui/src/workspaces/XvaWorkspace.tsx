/**
 * XvaWorkspace — the counterparty valuation-adjustment (XVA) surface. A trader
 * assembles a netting set of FX vanillas, sets each party's survival (hazard)
 * curve, LGDs and the funding spread, and prices the set's all-in CVA / DVA / FVA
 * against the single canonical contract (`PricingService.PriceXva`).
 *
 * The figures are REAL and SHARED: the workspace calls the transport's `priceXva`
 * — offline it runs the deterministic in-browser estimator (`src/data/xvaPricing`)
 * that reproduces the server's `celnet_xva::compute_xva` aggregation; over the WS
 * mirror it issues the live `price_xva` RPC. One contract, two transports.
 *
 * HONEST EXPOSURE-PROFILE BOUNDARY: the wire `XvaResult` carries ONLY the four
 * scalar adjustments — the simulated exposure PROFILE (EPE/ENE per bucket) is a
 * server-internal of the estimator and is NOT on the contract. So the counterparty
 * exposure fan (`XvaExposureFan`) is drawn from a DETERMINISTIC, clearly-labelled
 * ILLUSTRATIVE profile seeded from the netting set — never presented as a live
 * valuation. The scalar CVA/DVA/FVA above it ARE the priced result. This is stated
 * on the panel and in the fan's own caption, not hidden.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { Button } from "../components/Button";
import { Panel } from "../components/Panel";
import { XvaExposureFan, type FanBucket } from "../viz/XvaExposureFan";
import type { OptionType, XvaPricingRequest, XvaResult } from "../data/contract";
import { useApp } from "../app/AppContext";
import styles from "./XvaWorkspace.module.css";

import { NumberField as NumberFieldBase } from "../components/NumberField";

/** One editable netting-set trade (vols/notional held in trader-facing units). */
interface EditableTrade {
  readonly optionType: OptionType;
  /** Absolute strike `K`. */
  readonly strike: number;
  /** Time to expiry in years. */
  readonly expiryYears: number;
  /** Annualised vol in PERCENT (13 = 13 vol). */
  readonly volPct: number;
  /** Signed notional in the base currency (negative flips direction). */
  readonly notional: number;
}

/** The credit / funding inputs (hazards + LGDs held in trader-facing units). */
interface CreditInputs {
  /** Counterparty flat hazard in PERCENT (2 = 2%). */
  readonly cptyHazardPct: number;
  /** Own flat hazard in PERCENT. */
  readonly ownHazardPct: number;
  /** Counterparty LGD in PERCENT (60 = 0.60). */
  readonly lgdCptyPct: number;
  /** Own LGD in PERCENT. */
  readonly lgdOwnPct: number;
  /** Funding spread in BASIS POINTS (80 = 0.008). */
  readonly fundingBp: number;
}

/** The exposure-model market (single-factor lognormal spot + carry). */
interface MarketInputs {
  readonly spot0: number;
  /** Exposure-model vol in PERCENT. */
  readonly sigmaPct: number;
  /** Domestic (quote) rate in PERCENT. */
  readonly rDomPct: number;
  /** Foreign (base) rate in PERCENT. */
  readonly rForPct: number;
}

/**
 * The default netting set + market + credit inputs. Mirrors the server's XVA
 * round-trip fixture (a long call + a short put — a two-sided netting set so both
 * CVA and DVA are strictly positive), scaled to a 1mm notional.
 */
const DEFAULT_TRADES: readonly EditableTrade[] = [
  { optionType: "CALL", strike: 1.1, expiryYears: 1.0, volPct: 12, notional: 1_000_000 },
  { optionType: "PUT", strike: 1.05, expiryYears: 1.5, volPct: 14, notional: -1_000_000 },
];

const DEFAULT_MARKET: MarketInputs = {
  spot0: 1.1,
  sigmaPct: 13,
  rDomPct: 3,
  rForPct: 1,
};

const DEFAULT_CREDIT: CreditInputs = {
  cptyHazardPct: 2,
  ownHazardPct: 1.5,
  lgdCptyPct: 60,
  lgdOwnPct: 55,
  fundingBp: 80,
};

/** Monte-Carlo path budget sent to the live edge (the offline path uses quadrature). */
const MC_PATHS = 4096;
/** Exposure time buckets to the netting-set horizon. */
const EXPOSURE_STEPS = 16;
/** Counter-RNG seed for the live estimator (reproducible). */
const MC_SEED = 1;

/** Format a signed money amount as `$1,234.56` with the real minus glyph (U+2212). */
function money(v: number): string {
  const sign = v < 0 ? "−" : "";
  return `${sign}$${Math.abs(v).toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })}`;
}

// ---------------------------------------------------------------------------
// illustrative exposure fan — a deterministic, SEEDED sample (NOT the wire result)
// ---------------------------------------------------------------------------
//
// The exposure PROFILE is not on the contract (only the scalar XVA is), so the fan
// is drawn from a deterministic seeded sample derived from the netting set — the
// same honest posture the component's own story uses. It responds to the inputs
// (scale/horizon) so it reads coherently, but it is explicitly illustrative and
// never presented as a valuation. Fully deterministic in the netting set.

/** Deterministic PRNG (mulberry32) — same seed ⇒ byte-identical sample profile. */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** A stable integer seed derived from the netting set + market (deterministic). */
function seedFrom(trades: readonly EditableTrade[], market: MarketInputs): number {
  let h = 0x811c9dc5;
  const mix = (x: number): void => {
    h = Math.imul(h ^ (Math.round(x * 1000) | 0), 0x01000193) >>> 0;
  };
  for (const t of trades) {
    mix(t.optionType === "CALL" ? 1 : 2);
    mix(t.strike);
    mix(t.expiryYears);
    mix(t.volPct);
    mix(t.notional / 1000);
  }
  mix(market.spot0);
  mix(market.sigmaPct);
  return h >>> 0;
}

/** A compact tenor label for a year-fraction (`0.5` → `6M`, `2` → `2Y`). */
function tenorLabel(t: number): string {
  if (t <= 0) return "0";
  if (t < 1) return `${Math.round(t * 12)}M`;
  return Number.isInteger(t) ? `${t}Y` : `${t.toFixed(1)}Y`;
}

/**
 * Build a deterministic ILLUSTRATIVE exposure fan (EE / quantile bands + mirrored
 * ENE) over the netting set's horizon. The peak magnitude scales with the set's
 * gross notional × spot × vol so it reads at a plausible order; the quantile
 * spread is a fixed multiple of the seeded per-bucket EE (so the natural ordering
 * `pfeLo ≤ q25 ≤ ee ≤ q75 ≤ pfe` always holds). NOT a valuation — see the header.
 */
function buildExposureFan(
  trades: readonly EditableTrade[],
  market: MarketInputs,
): FanBucket[] {
  const horizon = trades.reduce((m, t) => Math.max(m, t.expiryYears), 0) || 1;
  const grossNotional = trades.reduce((s, t) => s + Math.abs(t.notional), 0);
  const sigma = market.sigmaPct / 100;
  // A PFE-scale peak in the netting-set currency (illustrative order of magnitude).
  const peak = Math.max(1, grossNotional * market.spot0 * sigma * 0.4);
  const hump = 0.4 * horizon;
  const rand = mulberry32(seedFrom(trades, market));
  const fractions = [0, 0.04, 0.09, 0.18, 0.32, 0.5, 0.68, 0.84, 1];
  return fractions.map((f) => {
    const t = f * horizon;
    const x = t <= 0 ? 0 : t / hump;
    const shape = t <= 0 ? 0 : Math.pow(x, 0.55) * Math.exp(1 - x);
    const n = peak * shape * (0.9 + 0.2 * rand());
    return {
      t,
      label: tenorLabel(t),
      ee: n,
      q75: n * 1.22,
      q25: n * 0.72,
      pfe: n * 1.85,
      pfeLo: n * 0.42,
      ene: -n * 0.62,
      eneBandHi: -n * 0.3,
      eneBandLo: -n * 1.05,
    };
  });
}

// ---------------------------------------------------------------------------
// the workspace
// ---------------------------------------------------------------------------

export function XvaWorkspace(): React.ReactElement {
  const app = useApp();

  const [trades, setTrades] = useState<readonly EditableTrade[]>(DEFAULT_TRADES);
  const [market, setMarket] = useState<MarketInputs>(DEFAULT_MARKET);
  const [credit, setCredit] = useState<CreditInputs>(DEFAULT_CREDIT);

  const [result, setResult] = useState<XvaResult | null>(null);
  const [pricing, setPricing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pricedSig, setPricedSig] = useState<string | null>(null);

  // The single canonical request the transport prices — assembled from the
  // trader-facing inputs (percent/bp → decimals) into the wire shape.
  const request = useMemo<XvaPricingRequest>(
    () => ({
      trades: trades.map((t) => ({
        optionType: t.optionType,
        strike: t.strike,
        expiryYears: t.expiryYears,
        vol: t.volPct / 100,
        notional: t.notional,
      })),
      rDom: market.rDomPct / 100,
      rFor: market.rForPct / 100,
      spot0: market.spot0,
      sigma: market.sigmaPct / 100,
      paths: MC_PATHS,
      seed: MC_SEED,
      exposureSteps: EXPOSURE_STEPS,
      counterparty: { pillarTimes: [], hazardRates: [credit.cptyHazardPct / 100] },
      own: { pillarTimes: [], hazardRates: [credit.ownHazardPct / 100] },
      lgdCounterparty: credit.lgdCptyPct / 100,
      lgdOwn: credit.lgdOwnPct / 100,
      fundingSpread: credit.fundingBp / 10_000,
    }),
    [trades, market, credit],
  );

  const requestSig = useMemo(() => JSON.stringify(request), [request]);
  // Inputs changed since the last successful price ⇒ the shown figures are stale.
  const dirty = result !== null && requestSig !== pricedSig;

  const price = useCallback(async (): Promise<void> => {
    setPricing(true);
    setError(null);
    try {
      const priced = await app.transport.priceXva(request);
      setResult(priced);
      setPricedSig(requestSig);
    } catch (e: unknown) {
      setResult(null);
      setError(e instanceof Error ? e.message : "XVA pricing failed");
    } finally {
      setPricing(false);
    }
  }, [app.transport, request, requestSig]);

  // Price once on mount so the surface is populated (the offline estimator is
  // instant + deterministic; a live transport issues one RPC). Re-pricing after an
  // edit is an explicit action (below) so a live socket is never spammed per key.
  useEffect(() => {
    void price();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The illustrative exposure fan — deterministic in the netting set (NOT priced).
  const fanBuckets = useMemo(() => buildExposureFan(trades, market), [trades, market]);

  const addTrade = useCallback(() => {
    setTrades((prev) => [
      ...prev,
      { optionType: "CALL", strike: market.spot0, expiryYears: 1, volPct: 12, notional: 1_000_000 },
    ]);
  }, [market.spot0]);

  const removeTrade = useCallback((index: number) => {
    setTrades((prev) => prev.filter((_, j) => j !== index));
  }, []);

  const patchTrade = useCallback((index: number, patch: Partial<EditableTrade>) => {
    setTrades((prev) => prev.map((t, j) => (j === index ? { ...t, ...patch } : t)));
  }, []);

  const total = result?.totalAdjustment ?? 0;

  return (
    <div className={styles.wrap}>
      <Panel material="float" className={styles.builder} title="Netting set & credit">
        <div className={styles.nettingHead}>
          <span className={styles.fieldLabel}>Trades</span>
          <Button variant="ghost" onClick={addTrade} title="add a trade to the netting set">
            + Trade
          </Button>
        </div>

        <ul className={styles.tradeList}>
          {trades.map((t, i) => (
            <li key={i} className={styles.tradeItem}>
              <select
                className={styles.tradeType}
                value={t.optionType}
                aria-label={`trade ${i + 1} option type`}
                onChange={(e) => patchTrade(i, { optionType: e.target.value as OptionType })}
              >
                <option value="CALL">Call</option>
                <option value="PUT">Put</option>
              </select>
              <label className={styles.inlineInput}>
                <span className={styles.inlineLabel}>K</span>
                <NumberFieldBase
                  step={0.01}
                  value={t.strike}
                  aria-label={`trade ${i + 1} strike`}
                  onChange={(e) => patchTrade(i, { strike: Number(e.target.value) })}
                />
              </label>
              <label className={styles.inlineInput}>
                <NumberFieldBase
                  step={0.25}
                  value={t.expiryYears}
                  aria-label={`trade ${i + 1} expiry in years`}
                  onChange={(e) => patchTrade(i, { expiryYears: Number(e.target.value) })}
                />
                <span className={styles.inputUnit}>y</span>
              </label>
              <label className={styles.inlineInput}>
                <NumberFieldBase
                  step={0.5}
                  value={t.volPct}
                  aria-label={`trade ${i + 1} vol in percent`}
                  onChange={(e) => patchTrade(i, { volPct: Number(e.target.value) })}
                />
                <span className={styles.inputUnit}>%</span>
              </label>
              <label className={styles.inlineInput}>
                <NumberFieldBase
                  step={100_000}
                  value={t.notional}
                  aria-label={`trade ${i + 1} notional`}
                  onChange={(e) => patchTrade(i, { notional: Number(e.target.value) })}
                />
              </label>
              <button
                type="button"
                className={styles.tradeRemove}
                aria-label={`remove trade ${i + 1}`}
                title="remove this trade"
                disabled={trades.length <= 1}
                onClick={() => removeTrade(i)}
              >
                ×
              </button>
            </li>
          ))}
        </ul>

        <fieldset className={styles.group}>
          <legend className={styles.fieldLabel}>Exposure market</legend>
          <div className={styles.grid2}>
            <NumberField
              label="Spot S₀"
              value={market.spot0}
              step={0.01}
              onChange={(v) => setMarket((m) => ({ ...m, spot0: v }))}
            />
            <NumberField
              label="Vol σ"
              unit="%"
              value={market.sigmaPct}
              step={0.5}
              onChange={(v) => setMarket((m) => ({ ...m, sigmaPct: v }))}
            />
            <NumberField
              label="Dom rate"
              unit="%"
              value={market.rDomPct}
              step={0.25}
              onChange={(v) => setMarket((m) => ({ ...m, rDomPct: v }))}
            />
            <NumberField
              label="For rate"
              unit="%"
              value={market.rForPct}
              step={0.25}
              onChange={(v) => setMarket((m) => ({ ...m, rForPct: v }))}
            />
          </div>
        </fieldset>

        <fieldset className={styles.group}>
          <legend className={styles.fieldLabel}>Counterparty & own credit</legend>
          <div className={styles.grid2}>
            <NumberField
              label="Cpty hazard λ"
              unit="%"
              value={credit.cptyHazardPct}
              step={0.25}
              onChange={(v) => setCredit((c) => ({ ...c, cptyHazardPct: v }))}
            />
            <NumberField
              label="Own hazard λ"
              unit="%"
              value={credit.ownHazardPct}
              step={0.25}
              onChange={(v) => setCredit((c) => ({ ...c, ownHazardPct: v }))}
            />
            <NumberField
              label="Cpty LGD"
              unit="%"
              value={credit.lgdCptyPct}
              step={5}
              onChange={(v) => setCredit((c) => ({ ...c, lgdCptyPct: v }))}
            />
            <NumberField
              label="Own LGD"
              unit="%"
              value={credit.lgdOwnPct}
              step={5}
              onChange={(v) => setCredit((c) => ({ ...c, lgdOwnPct: v }))}
            />
            <NumberField
              label="Funding spread"
              unit="bp"
              value={credit.fundingBp}
              step={5}
              onChange={(v) => setCredit((c) => ({ ...c, fundingBp: v }))}
            />
          </div>
        </fieldset>

        <div className={styles.priceRow}>
          <Button
            variant="primary"
            onClick={() => void price()}
            disabled={pricing || trades.length === 0}
            title="price the netting set's XVA"
          >
            {pricing ? "Pricing…" : dirty ? "Recompute XVA" : "Price XVA"}
          </Button>
          {dirty && (
            <span className={styles.staleHint} role="status">
              inputs changed — recompute
            </span>
          )}
        </div>

        <p className={styles.scopeNote}>
          Counterparty hazards are entered as flat curves (constant λ). The exposure
          profile is estimated over {EXPOSURE_STEPS} buckets to the set horizon; the
          live edge uses {MC_PATHS.toLocaleString()} Monte-Carlo paths, the offline
          estimator a deterministic quadrature (they converge, not bit-identical).
        </p>
      </Panel>

      <Panel className={styles.results} title="Valuation adjustments">
        {error ? (
          <p className={styles.error} role="alert">
            {error}
          </p>
        ) : result ? (
          <>
            <dl className={styles.metrics} aria-busy={pricing}>
              <Metric label="Total XVA" value={money(total)} tone="total" emphatic />
              <Metric label="CVA" value={money(result.cva)} tone="cva" />
              <Metric label="DVA" value={money(result.dva)} tone="dva" />
              <Metric label="FVA" value={money(result.fva)} tone="fva" />
            </dl>

            <p className={styles.identityNote}>
              Total adjustment = CVA − DVA + FVA, subtracted from the risk-free value.
              These four scalars are the WHOLE wire result.
            </p>

            <div className={styles.chart}>
              <h3 className={styles.chartTitle}>Counterparty exposure profile</h3>
              <XvaExposureFan buckets={fanBuckets} height={300} unit="" />
              <p className={styles.fanNote}>
                Illustrative, seeded profile — the per-bucket exposure fan is NOT on
                the contract (only the scalar CVA/DVA/FVA above cross the wire), so it
                is a deterministic sample derived from the netting set, never a live
                valuation.
              </p>
            </div>
          </>
        ) : pricing ? (
          <p className={styles.empty}>Pricing the netting set's XVA…</p>
        ) : (
          <p className={styles.empty}>
            Assemble a netting set and price it to compute its CVA / DVA / FVA.
          </p>
        )}
      </Panel>
    </div>
  );
}

/** One labelled numeric input (trader-facing unit shown as a suffix). */
function NumberField({
  label,
  value,
  step,
  unit,
  onChange,
}: {
  label: string;
  value: number;
  step: number;
  unit?: string;
  onChange: (v: number) => void;
}): React.ReactElement {
  return (
    <label className={styles.field}>
      <span className={styles.fieldLabel}>{label}</span>
      <span className={styles.inlineInput}>
        <NumberFieldBase
          step={step}
          value={value}
          aria-label={label}
          onChange={(e) => onChange(Number(e.target.value))}
        />
        {unit && <span className={styles.inputUnit}>{unit}</span>}
      </span>
    </label>
  );
}

/** One headline adjustment figure (JetBrains numerics, token-tinted by leg). */
function Metric({
  label,
  value,
  tone,
  emphatic,
}: {
  label: string;
  value: string;
  tone: "total" | "cva" | "dva" | "fva";
  emphatic?: boolean;
}): React.ReactElement {
  return (
    <div
      className={`${styles.metric} ${emphatic ? styles.metricEmphatic : ""}`}
      data-tone={tone}
    >
      <dt className={styles.metricLabel}>{label}</dt>
      <dd className={styles.metricValue}>{value}</dd>
    </div>
  );
}
