/**
 * Correlated multi-asset basket / best-of / worst-of — a {@link ProductSpec}
 * (GW2). The trader builds 2–3 correlated currency-pair legs aggregated under a
 * single equicorrelation `ρ`; always Cholesky-correlated Monte-Carlo, so the build
 * surfaces a standard error. Extracted verbatim from the former `TicketWorkspace`
 * monolith so the wire output is byte-identical.
 */
import type { BasketKind, CcyPair, Instrument, OptionType } from "../data/contract";
import { basketInstrument, bookingModelsFor, PAIRS, type BasketTerms } from "../data/seed";
import { pairLabel } from "../lib/universe";
import styles from "../workspaces/TicketWorkspace.module.css";
import { defineProduct, withTenorAndModel, type InputBlockProps } from "./types";

import { NumberField } from "../components/NumberField";

/**
 * The correlated multi-asset basket ticket inputs. The trader builds 2–3 legs
 * (each a currency pair + weight + its own spot / vol / foreign rate), a single
 * off-diagonal correlation `ρ` (the matrix is symmetric with unit diagonal — for
 * 2 legs that one `ρ`; for 3 legs `ρ` populates all three off-diagonals, an
 * equicorrelation matrix), the aggregation kind (BASKET / BEST_OF / WORST_OF),
 * the call/put and the strike `K` on the aggregated underlying (`0` ⇒ default to
 * the inception aggregate level at build). The Monte-Carlo controls (`mcPaths` /
 * `mcReplications` / `mcSteps` / `mcSeed`) tune the server estimator; the basket
 * is always Monte-Carlo, so the build surfaces a standard error.
 */
export interface BasketLegInput {
  pair: CcyPair;
  weight: number;
  spot: number;
  vol: number;
  rFor: number;
}

export interface BasketInputs {
  legs: BasketLegInput[];
  /** The single off-diagonal correlation `ρ` shared by every leg pair. */
  correlation: number;
  optionType: OptionType;
  /** Strike on the aggregated underlying; `0` ⇒ default to the inception aggregate. */
  strike: number;
  kind: BasketKind;
  mcPaths: number;
  mcReplications: number;
  mcSteps: number;
  mcSeed: bigint;
}

/** The minimum and maximum legs the basket builder allows (2–3 legs). */
const BASKET_MIN_LEGS = 2;
const BASKET_MAX_LEGS = 3;

/** A default basket leg seeded from a pair in the universe (spot/vol/rFor off its market). */
function defaultBasketLeg(index: number): BasketLegInput {
  const ctx = PAIRS[index % PAIRS.length]!;
  return {
    pair: ctx.pair,
    weight: 0.5,
    spot: ctx.market.spot,
    vol: ctx.market.vol,
    rFor: ctx.market.rFor,
  };
}

export const DEFAULT_BASKET: BasketInputs = {
  legs: [defaultBasketLeg(0), defaultBasketLeg(1)],
  correlation: 0.4,
  optionType: "CALL",
  strike: 0,
  kind: "WORST_OF",
  mcPaths: 8192,
  mcReplications: 16,
  mcSteps: 1,
  mcSeed: 0xba_5en,
};

/**
 * The inception aggregate level `Σ wₐ Sₐ(0)` (BASKET) / `max` (BEST_OF) /
 * `min` (WORST_OF) of the weighted leg spots — the natural strike default so the
 * basket prices roughly at-the-money on first build.
 */
function basketInceptionAggregate(inputs: BasketInputs): number {
  const weighted = inputs.legs.map((l) => l.weight * l.spot);
  if (weighted.length === 0) return 0;
  if (inputs.kind === "BEST_OF") return Math.max(...weighted);
  if (inputs.kind === "WORST_OF") return Math.min(...weighted);
  return weighted.reduce((a, b) => a + b, 0);
}

/**
 * Build the row-major N×N correlation matrix from the single off-diagonal `ρ`:
 * unit diagonal, `ρ` on every off-diagonal (an equicorrelation matrix, the
 * symmetric / unit-diagonal shape the server's Cholesky validates as SPD when
 * `−1/(N−1) < ρ < 1`).
 */
function basketCorrelationMatrix(n: number, rho: number): number[] {
  const m = new Array<number>(n * n).fill(0);
  for (let i = 0; i < n; i += 1) {
    for (let j = 0; j < n; j += 1) {
      m[i * n + j] = i === j ? 1 : rho;
    }
  }
  return m;
}

/**
 * True iff the row-major N×N correlation matrix is symmetric-positive-definite (a
 * successful Cholesky with positive pivots) — the exact admissibility the server's
 * `cholesky` enforces. Used to warn in the ticket before a non-SPD request reaches
 * the pricer (which rejects it as `NotPositiveDefinite`). Provenance documented
 * here only, never in an identifier (GUIDE.md rule 8).
 */
function choleskyLowerOk(rowMajor: number[], n: number): boolean {
  const l: number[][] = Array.from({ length: n }, () => new Array<number>(n).fill(0));
  for (let i = 0; i < n; i += 1) {
    for (let j = 0; j <= i; j += 1) {
      if (Math.abs(rowMajor[i * n + j]! - rowMajor[j * n + i]!) > 1e-9) return false;
      let dot = 0;
      for (let k = 0; k < j; k += 1) dot += l[i]![k]! * l[j]![k]!;
      if (i === j) {
        const diag = rowMajor[i * n + i]! - dot;
        if (diag <= 0) return false;
        l[i]![j] = Math.sqrt(diag);
      } else {
        l[i]![j] = (rowMajor[i * n + j]! - dot) / l[j]![j]!;
      }
    }
  }
  return true;
}

/**
 * Build `BasketTerms` from the inputs: the equicorrelation matrix from `ρ` and the
 * strike defaulted to the inception aggregate when left `0`. The MC controls pass
 * through (the server clamps `0` to its defaults).
 */
export function basketTerms(inputs: BasketInputs): BasketTerms {
  const n = inputs.legs.length;
  const strike = inputs.strike > 0 ? inputs.strike : basketInceptionAggregate(inputs);
  return {
    legs: inputs.legs.map((l) => ({
      pair: l.pair,
      weight: l.weight,
      spot: l.spot,
      vol: l.vol,
      rFor: l.rFor,
    })),
    correlations: basketCorrelationMatrix(n, inputs.correlation),
    optionType: inputs.optionType,
    strike,
    kind: inputs.kind,
    mcPaths: Math.max(0, Math.trunc(inputs.mcPaths)),
    mcReplications: Math.max(0, Math.trunc(inputs.mcReplications)),
    mcSteps: Math.max(0, Math.trunc(inputs.mcSteps)),
    mcSeed: inputs.mcSeed,
  };
}

function BasketInputBlock({ value: basket, onChange }: InputBlockProps<BasketInputs>) {
  const onBasket = onChange;
  const n = basket.legs.length;
  const inceptionAggregate = basketInceptionAggregate(basket);
  const matrix = basketCorrelationMatrix(n, basket.correlation);
  const isSpd = choleskyLowerOk(matrix, n);
  // The equicorrelation SPD bound for N equal-correlated legs: ρ ∈ (−1/(N−1), 1).
  const rhoFloor = -1 / (n - 1);
  const updateLeg = (i: number, patch: Partial<BasketLegInput>): void => {
    const legs = basket.legs.map((l, k) => (k === i ? { ...l, ...patch } : l));
    onBasket({ ...basket, legs });
  };
  const setLegPair = (i: number, key: string): void => {
    const ctx = PAIRS.find((p) => pairLabel(p.pair) === key);
    if (!ctx) return;
    // Seed the leg's market data off the chosen pair (the trader can override).
    updateLeg(i, { pair: ctx.pair, spot: ctx.market.spot, vol: ctx.market.vol, rFor: ctx.market.rFor });
  };
  const addLeg = (): void => {
    if (n >= BASKET_MAX_LEGS) return;
    onBasket({ ...basket, legs: [...basket.legs, defaultBasketLeg(n)] });
  };
  const removeLeg = (i: number): void => {
    if (n <= BASKET_MIN_LEGS) return;
    onBasket({ ...basket, legs: basket.legs.filter((_, k) => k !== i) });
  };
  return (
    <div className={styles.product}>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Aggregation</span>
        <div className={styles.toggleGroup} role="tablist" aria-label="basket kind">
          {(["BASKET", "BEST_OF", "WORST_OF"] as BasketKind[]).map((bk) => (
            <button
              key={bk}
              role="tab"
              aria-selected={basket.kind === bk}
              className={`${styles.modeTab} ${basket.kind === bk ? styles.modeActive : ""}`}
              onClick={() => onBasket({ ...basket, kind: bk })}
            >
              {bk === "BASKET" ? "Basket" : bk === "BEST_OF" ? "Best-of" : "Worst-of"}
            </button>
          ))}
        </div>
        <div className={styles.toggleGroup} role="tablist" aria-label="option type">
          {(["CALL", "PUT"] as OptionType[]).map((ot) => (
            <button
              key={ot}
              role="tab"
              aria-selected={basket.optionType === ot}
              className={`${styles.modeTab} ${basket.optionType === ot ? styles.modeActive : ""}`}
              onClick={() => onBasket({ ...basket, optionType: ot })}
            >
              {ot === "CALL" ? "Call" : "Put"}
            </button>
          ))}
        </div>
      </div>
      {basket.legs.map((leg, i) => (
        <div className={styles.basketLeg} key={i}>
          <span className={styles.basketLegNo}>Leg {i + 1}</span>
          <label className={styles.productField}>
            <span>Pair</span>
            <select
              value={pairLabel(leg.pair)}
              aria-label={`leg ${i + 1} pair`}
              onChange={(ev) => setLegPair(i, ev.target.value)}
            >
              {PAIRS.map((p) => (
                <option key={pairLabel(p.pair)} value={pairLabel(p.pair)}>
                  {pairLabel(p.pair)}
                </option>
              ))}
            </select>
          </label>
          <label className={styles.productField}>
            <span>Weight</span>
            <NumberField
              className="num"
              step={0.05}
              value={leg.weight}
              aria-label={`leg ${i + 1} weight`}
              onChange={(ev) => updateLeg(i, { weight: Number(ev.target.value) })}
            />
          </label>
          <label className={styles.productField}>
            <span>Spot</span>
            <NumberField
              className="num"
              min={0}
              step={0.0001}
              value={leg.spot}
              aria-label={`leg ${i + 1} spot`}
              onChange={(ev) => updateLeg(i, { spot: Math.max(0, Number(ev.target.value)) })}
            />
          </label>
          <label className={styles.productField}>
            <span>Vol</span>
            <NumberField
              className="num"
              min={0}
              step={0.005}
              value={leg.vol}
              aria-label={`leg ${i + 1} vol`}
              onChange={(ev) => updateLeg(i, { vol: Math.max(0, Number(ev.target.value)) })}
            />
          </label>
          <label className={styles.productField}>
            <span>r_for</span>
            <NumberField
              className="num"
              step={0.001}
              value={leg.rFor}
              aria-label={`leg ${i + 1} r_for`}
              onChange={(ev) => updateLeg(i, { rFor: Number(ev.target.value) })}
            />
          </label>
          <button
            className={styles.basketLegRemove}
            aria-label={`remove leg ${i + 1}`}
            disabled={n <= BASKET_MIN_LEGS}
            onClick={() => removeLeg(i)}
          >
            Remove
          </button>
        </div>
      ))}
      <button
        className={styles.basketAddLeg}
        aria-label="add leg"
        disabled={n >= BASKET_MAX_LEGS}
        onClick={addLeg}
      >
        + Add leg
      </button>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Correlation</span>
        <label className={styles.productField}>
          <span>ρ</span>
          <NumberField
            className="num"
            min={-1}
            max={1}
            step={0.05}
            value={basket.correlation}
            aria-label="correlation"
            onChange={(ev) =>
              onBasket({ ...basket, correlation: Math.max(-1, Math.min(1, Number(ev.target.value))) })
            }
          />
          <span>{isSpd ? "" : `not positive-definite — need ρ > ${rhoFloor.toFixed(2)}`}</span>
        </label>
        <label className={styles.productField}>
          <span>Strike</span>
          <NumberField
            className="num"
            min={0}
            step={0.0001}
            value={basket.strike}
            aria-label="strike"
            placeholder={inceptionAggregate.toFixed(4)}
            onChange={(ev) =>
              onBasket({ ...basket, strike: Math.max(0, Number(ev.target.value)) })
            }
          />
          <span>{basket.strike > 0 ? "" : `aggregate ${inceptionAggregate.toFixed(4)}`}</span>
        </label>
      </div>
      <div className={styles.productRow}>
        <span className={styles.productLabel}>Engine</span>
        <label className={styles.productField}>
          <span>MC paths</span>
          <NumberField
            className="num"
            min={0}
            step={1024}
            value={basket.mcPaths}
            aria-label="mc paths"
            onChange={(ev) =>
              onBasket({ ...basket, mcPaths: Math.max(0, Math.trunc(Number(ev.target.value))) })
            }
          />
        </label>
        <label className={styles.productField}>
          <span>Scrambles</span>
          <NumberField
            className="num"
            min={0}
            step={1}
            value={basket.mcReplications}
            aria-label="mc replications"
            onChange={(ev) =>
              onBasket({
                ...basket,
                mcReplications: Math.max(0, Math.trunc(Number(ev.target.value))),
              })
            }
          />
        </label>
        <label className={styles.productField}>
          <span>Steps</span>
          <NumberField
            className="num"
            min={0}
            step={1}
            value={basket.mcSteps}
            aria-label="mc steps"
            onChange={(ev) =>
              onBasket({ ...basket, mcSteps: Math.max(0, Math.trunc(Number(ev.target.value))) })
            }
          />
        </label>
      </div>
      <p className={styles.productNote}>
        {basket.kind === "BASKET"
          ? "Weighted basket"
          : basket.kind === "BEST_OF"
            ? "Best-of (rainbow max)"
            : "Worst-of (rainbow min)"}{" "}
        {basket.optionType === "CALL" ? "call" : "put"} over {n} correlated currency-pair legs.
        Each leg carries its own spot / vol / foreign rate; the enclosing pair is the
        settlement / numeraire pair and the shared domestic rate is the market context&rsquo;s.
        Priced by Cholesky-correlated multi-asset Monte-Carlo (always — there is no closed form),
        so it reports a standard error. Multi-asset Greeks are deferred (the strip is zeroed). The
        offline build prices a genuine antithetic correlated terminal Monte-Carlo; a non
        positive-definite correlation is rejected, never regularised.
      </p>
    </div>
  );
}

/** The correlated multi-asset basket {@link ProductSpec}. */
export const basketSpec = defineProduct<BasketInputs>({
  id: "BASKET",
  label: "Basket / Best-of / Worst-of",
  group: "Structured",
  assetClass: "FX",
  summary:
    "Correlated multi-asset basket / best-of / worst-of — 2–3 currency-pair legs under one equicorrelation.",
  keywords: ["basket", "best-of", "worst-of", "rainbow", "correlated", "multi-asset", "correlation"],
  kind: "basket",
  defaults: DEFAULT_BASKET,
  allowedModels: bookingModelsFor("basket"),
  toInstrument: (inputs: BasketInputs, ctx): Instrument =>
    withTenorAndModel(
      basketInstrument(ctx.pair, ctx.tenorYears, ctx.notionalMm, basketTerms(inputs)),
      ctx,
    ),
  InputBlock: BasketInputBlock,
});
