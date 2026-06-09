/**
 * The CELNET.* worksheet custom functions (docs/EXCEL-INTEGRATION.md §3).
 *
 * Each function shapes a request in the ONE current contract (src/contract),
 * sends it over the shared WS mirror connection (src/transport), and returns the
 * typed result with the resolved convention shown alongside the value (the §3.4
 * convention-transparency mandate). NO pricing logic lives here — every number is
 * the server's libm-core value, so a cell is bit-identical to the GUI/SDK/CLI.
 *
 * The `@customfunction` JSDoc tags below are the custom-function metadata the
 * Office.js runtime reads. The namespace `CELNET` is declared in the manifest;
 * the function id after `@customfunction` is the worksheet name suffix, so e.g.
 * `@customfunction PRICE` is invoked as `=CELNET.PRICE(...)`.
 *
 * Errors are surfaced as typed `CustomFunctions.Error` values (e.g.
 * `#CELNET_STALE!`), never as a wrong number — a denied/stale/version-mismatch
 * cell is honest about why (docs §5, "no #N/A storm").
 */

import {
  DEFAULT_CONVENTIONS,
  ShapingError,
  americanIsMonteCarlo,
  cliquetIsMonteCarlo,
  formatAsianSpill,
  formatCalibratedSmileSpill,
  formatCliquetSpill,
  formatExoticPremiumSpill,
  formatForwardStartSpill,
  formatGreeksSpill,
  formatLimitsSpill,
  formatMarkStatusSpill,
  formatPathDependentSpill,
  formatPositionsSpill,
  formatQuantoSpill,
  formatRfqSpill,
  formatRiskSpill,
  formatSeriesCell,
  formatServerStatusSpill,
  formatSmileSpill,
  formatVarSwapSpill,
  formatVolSwapSpill,
  lookbackIsMonteCarlo,
  parseObservable,
  parsePair,
  parseRiskDimension,
  parseRiskScope,
  parseSmileModel,
  parseTenor,
  shapeAccumulator,
  shapeAmerican,
  shapeAsianOption,
  shapeBarrier,
  shapeBasket,
  shapeCalibration,
  shapeCliquet,
  shapeDigital,
  shapeForward,
  shapeForwardStart,
  shapeLookback,
  shapeNdf,
  shapeQuanto,
  shapeSwap,
  shapeReportingNumeraire,
  shapeTarf,
  shapeTouch,
  shapeVanillaInstrument,
  shapeWindowBarrier,
  shapeVarianceSwap,
  shapeVolatilitySwap,
  type SpillMatrix,
} from "./shaping";
import {
  aggregateRiskRequest,
  aggregateRiskResponseFromWire,
  limitStatusRequest,
  limitStatusResponseFromWire,
  listPositionsRequest,
  listPositionsResponseFromWire,
  type RiskScope,
} from "../contract/riskCodec";
import { stageMark } from "./markStaging";
import { getConnection, getRegistry, getSeriesRegistry } from "./runtime";
import { brokerQuoteSetToWire, ccyPairToWire, conventionsToWire, smileFromWire, type WireObject } from "../contract/wsCodec";
import { smileModel } from "../contract/enums";
import type { LiveTick } from "./streamRegistry";
import type { SeriesTick } from "./seriesRegistry";
import type { MarketSeriesRequest } from "../transport/connection";

/** Map any error to a custom-function error value with a readable message. */
function toCfError(err: unknown): CustomFunctions.Error {
  const message = err instanceof Error ? err.message : String(err);
  const code = err instanceof ShapingError ? "#CELNET_ARG!" : "#CELNET_ERR!";
  return new CustomFunctions.Error(CustomFunctions.ErrorCode.invalidValue, `${code} ${message}`);
}

/**
 * Price one structure (the premium in the convention's premium-ccy/style).
 * @customfunction PRICE
 * @param pair Currency pair, e.g. "EURUSD".
 * @param tenor Tenor, e.g. "1Y", "3M", "ON".
 * @param strikeOrDelta Absolute strike (1.12) or delta ("25dP", "ATM").
 * @param callPut "C" for call, "P" for put.
 * @param notional Trade notional in the base currency.
 * @returns The premium for the structure.
 */
export async function PRICE(
  pair: string,
  tenor: string,
  strikeOrDelta: string,
  callPut: string,
  notional: number,
): Promise<number> {
  try {
    const instrument = shapeVanillaInstrument({ pair, tenor, strikeOrDelta, callPut, notional });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `price:${pair}:${tenor}:${strikeOrDelta}:${callPut}:${notional}`,
    );
    // The mid of the two-way is the premium; the directional market is the spill
    // returned by CELNET.RFQ. A single-cell PRICE shows the mid valuation.
    return 0.5 * (quote.price.bid + quote.price.offer);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * The full 13-Greek vector for a structure as a vertical dynamic-array spill
 * `[GreekName, Value]`, with a convention-transparency footer row.
 * @customfunction GREEKS
 * @param pair Currency pair, e.g. "EURUSD".
 * @param tenor Tenor, e.g. "1Y".
 * @param strikeOrDelta Absolute strike or delta ("25dP", "ATM").
 * @param callPut "C" or "P".
 * @param notional Trade notional in the base currency.
 * @returns A 14×2 spill: the 13 Greeks plus a convention footer.
 */
export async function GREEKS(
  pair: string,
  tenor: string,
  strikeOrDelta: string,
  callPut: string,
  notional: number,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeVanillaInstrument({ pair, tenor, strikeOrDelta, callPut, notional });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `greeks:${pair}:${tenor}:${strikeOrDelta}:${callPut}:${notional}`,
    );
    return formatGreeksSpill(quote.greeks, quote.conventions, quote.surfaceVersion, quote.epochNanos);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * The marked smile for a (pair, tenor) as a dynamic-array grid (delta × vol),
 * with an arbitrage-status + convention footer.
 *
 * The optional `model` argument records the smile-calibration family the trader
 * wants surfaced (VV/SABR/SVI/SSVI). The read path (`get_smile`) returns whatever
 * model the surface was last MARKED under; the TYPED `arbitrage.smileModel` the
 * server stamps (proto `ArbReport.smile_model`) is surfaced in the footer as the
 * authoritative provenance, and the requested model is shown alongside so a
 * mismatch (the surface was marked under a different family) is visible — to
 * actually re-calibrate a surface under a chosen model, use `CELNET.MARKSURFACE`
 * (the `mark_surface` `smile_model` path).
 * @customfunction SURFACE
 * @param pair Currency pair, e.g. "EURUSD".
 * @param tenor Tenor, e.g. "1Y".
 * @param model Optional smile model to surface: VV, SABR, SVI or SSVI.
 * @returns A spill: a delta-pillar header row, a vol row, and a footer.
 */
export async function SURFACE(pair: string, tenor: string, model?: string): Promise<SpillMatrix> {
  try {
    const ccy = parsePair(pair);
    const { expiryYears } = parseTenor(tenor);
    const requestedModel = parseSmileModel(model);
    const reply: WireObject = await getConnection().getSmile(ccy, expiryYears, DEFAULT_CONVENTIONS);
    const smile = smileFromWire(reply);
    const points = smile.points.map((p) => ({ delta: p.delta, vol: p.vol }));
    const arbFree =
      smile.arbitrage.butterflyArbitrageFree && smile.arbitrage.calendarArbitrageFree;
    // When no model is requested, keep the existing plain smile spill (unchanged
    // current behaviour). When a model is named, render the model-aware footer so
    // the surfaced family + provenance is explicit.
    if (model === undefined || model.trim() === "") {
      return formatSmileSpill(points, arbFree, smile.conventions, undefined, smile.epochNanos);
    }
    return formatCalibratedSmileSpill({
      points,
      requestedModel,
      actualModel: smile.arbitrage.smileModel,
      arbFree,
      conv: smile.conventions,
      surfaceVersion: undefined,
      epochNanos: smile.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Calibrate and mark a surface under a chosen smile model and return the resulting
 * calibrated smile. This is the model-selection capability over the ONE contract:
 * it issues `mark_surface` with the broker marks and the `smile_model` selector
 * (VV/SABR/SVI/SSVI), depositing a model-tagged surface and returning the
 * calibrated smile grid + the assigned `surface_version` (shown in the footer) so
 * a subsequent `CELNET.RFQ`/`CELNET.PRICE` can pin it.
 *
 * NOTE on side effects: unlike `CELNET.MARK` (two-phase, task-pane-confirmed),
 * this function commits the calibration directly on evaluation — it is the
 * spreadsheet form of the desk "re-mark under model X" action. The server applies
 * its own dedupe, but a recalc with changed broker marks WILL deposit a new
 * surface version; treat it as an explicit action cell, not a live formula.
 * @customfunction MARKSURFACE
 * @param pair Currency pair, e.g. "EURUSD".
 * @param tenor Tenor, e.g. "1Y".
 * @param model Smile model: VV (market hedge), SABR, SVI or SSVI.
 * @param atmVol At-the-money volatility (absolute, e.g. 0.102).
 * @param rr25 25-delta risk reversal (call vol − put vol), e.g. 0.01.
 * @param bf25 25-delta butterfly (broker fly), e.g. 0.003.
 * @param rr10 Optional 10-delta risk reversal (supply with bf10 for a 5-point smile).
 * @param bf10 Optional 10-delta butterfly.
 * @returns A calibrated smile spill: delta pillars, vols, and a model/version footer.
 */
export async function MARKSURFACE(
  pair: string,
  tenor: string,
  model: string,
  atmVol: number,
  rr25: number,
  bf25: number,
  rr10?: number,
  bf10?: number,
): Promise<SpillMatrix> {
  try {
    const shaped = shapeCalibration({ pair, tenor, model, atmVol, rr25, bf25, rr10, bf10 });
    const reply: WireObject = await getConnection().markSurface({
      pair: ccyPairToWire(shaped.pair),
      broker_quotes: [
        brokerQuoteSetToWire({
          tenorYears: shaped.tenorYears,
          atmVol: shaped.atmVol,
          rr25: shaped.rr25,
          bf25: shaped.bf25,
          rr10: shaped.rr10,
          bf10: shaped.bf10,
          hasTenDelta: shaped.hasTenDelta,
        }),
      ],
      conventions: conventionsToWire(DEFAULT_CONVENTIONS),
      smile_model: smileModel.toWire(shaped.model),
    });
    const surfaceVersion =
      typeof reply["surface_version"] === "number"
        ? BigInt(Math.trunc(reply["surface_version"] as number))
        : undefined;
    // The response carries the calibrated smiles; pick the one matching the marked
    // tenor (the request marks a single tenor, so it is the first/only smile).
    const smiles = Array.isArray(reply["smiles"]) ? (reply["smiles"] as WireObject[]) : [];
    const smile = smileFromWire(smiles[0] ?? {});
    const points = smile.points.map((p) => ({ delta: p.delta, vol: p.vol }));
    return formatCalibratedSmileSpill({
      points,
      requestedModel: shaped.model,
      actualModel: smile.arbitrage.smileModel,
      arbFree: smile.arbitrage.butterflyArbitrageFree && smile.arbitrage.calendarArbitrageFree,
      conv: smile.conventions,
      surfaceVersion,
      epochNanos: smile.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * RFQ for a structure: a 1×4 spill `[bid, offer, quoteId, validUntil]` plus a
 * convention footer. The quoteId binds the task-pane Trade ticket (W4/W6).
 * @customfunction RFQ
 * @param pair Currency pair, e.g. "EURUSD".
 * @param tenor Tenor, e.g. "1Y".
 * @param strikeOrDelta Absolute strike or delta ("25dP", "ATM").
 * @param callPut "C" or "P".
 * @param notional Trade notional in the base currency.
 * @returns A spill with the two-way market, quote id, and validity.
 */
export async function RFQ(
  pair: string,
  tenor: string,
  strikeOrDelta: string,
  callPut: string,
  notional: number,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeVanillaInstrument({ pair, tenor, strikeOrDelta, callPut, notional });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `rfq:${pair}:${tenor}:${strikeOrDelta}:${callPut}:${notional}`,
    );
    return formatRfqSpill({
      bid: quote.price.bid,
      offer: quote.price.offer,
      quoteId: quote.quoteId,
      validUntilNanos: quote.validUntilNanos,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a barrier option — single OR double — as a labelled spill of
 * `["premium", PV]`, the 13 risk Greeks, and a convention footer. The premium is
 * the server's exact closed-form / PDE value (the same `celnet-exotics` value the
 * SDK/CLI read); barriers are priced exactly, so there is no standard-error row.
 *
 * ONE function covers both products via params: supplying `upperBarrier` selects a
 * DOUBLE barrier (the `barrier` argument is then the LOWER barrier, `upperBarrier`
 * the upper, and `side` is omitted — a double barrier brackets spot, so it has no
 * single side); omitting `upperBarrier` selects a SINGLE barrier with the given
 * `side` (UP/DOWN). The strike may be an absolute level or a delta ("25dP", "ATM").
 * @customfunction BARRIER
 * @param pair Currency pair, e.g. EURUSD.
 * @param tenor Tenor, e.g. 1Y.
 * @param strikeOrDelta Absolute strike (1.12) or delta (25dP, ATM) of the vanilla payoff.
 * @param callPut C for call, P for put.
 * @param notional Trade notional in the base currency.
 * @param barrier Barrier level (single: the sole barrier; double: the LOWER barrier).
 * @param kind Optional KNOCK_IN (default) or KNOCK_OUT.
 * @param side Optional UP (default) or DOWN — single-barrier only (omit for a double barrier).
 * @param upperBarrier Optional upper barrier; supplying it makes the ticket a DOUBLE barrier.
 * @param rebate Optional rebate paid on the barrier event (omit => 0).
 * @param monitoring Optional CONTINUOUS (default) or DISCRETE crossing.
 * @param model Optional booking model: ANALYTIC (default, closed-form) or LSV (local-stoch-vol ADI PDE; single-barrier only — a double barrier under LSV is rejected).
 * @returns A spill: premium, (std_error if LSV-MC), the 13 Greeks, and a convention footer.
 */
export async function BARRIER(
  pair: string,
  tenor: string,
  strikeOrDelta: string,
  callPut: string,
  notional: number,
  barrier: number,
  kind?: string,
  side?: string,
  upperBarrier?: number,
  rebate?: number,
  monitoring?: string,
  model?: string,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeBarrier({
      pair,
      tenor,
      strikeOrDelta,
      callPut,
      notional,
      barrier,
      kind,
      side,
      upperBarrier,
      rebate,
      monitoring,
      model,
    });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `barrier:${pair}:${tenor}:${strikeOrDelta}:${callPut}:${notional}:${barrier}:${kind ?? ""}:${side ?? ""}:${upperBarrier ?? ""}:${rebate ?? ""}:${monitoring ?? ""}:${model ?? ""}`,
    );
    // The closed-form (ANALYTIC) barrier is exact (no std-error); the LSV
    // Monte-Carlo route may carry one — surface it honestly when present.
    return formatPathDependentSpill({
      premium: quote.greeks.price,
      stdError: quote.priceStdError,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a window (partial-time) barrier as a labelled spill of `["premium", PV]`,
 * then — ONLY for the Monte-Carlo route — an honest `["std_error", σ̄]` row, then
 * the 13 risk Greeks and a convention footer. The premium is the server's
 * `celnet-exotics` value (the same the SDK/CLI read). A window barrier is always a
 * knock-out that is active ONLY inside `[windowStart, windowEnd] ⊆ [0, expiry]`,
 * and has NO closed form — it is priced under the LOCAL_STOCH_VOL booking model
 * (the function selects it; no `model` argument is needed). The window defaults to
 * the full life `[0, expiry]`; supply `windowStart` for a "back" partial barrier,
 * `windowEnd` for a "front" partial. `mcPairs > 0` selects the Monte-Carlo engine
 * (the headline then carries a std-error row); `0` (default) selects the exact
 * ADI-PDE engine (no std-error).
 * @customfunction WINDOWBARRIER
 * @param pair Currency pair, e.g. EURUSD.
 * @param tenor Tenor, e.g. 1Y.
 * @param strikeOrDelta Absolute strike (1.12) or delta (25dP, ATM) of the vanilla payoff.
 * @param callPut C for call, P for put.
 * @param notional Trade notional in the base currency.
 * @param barrier Knock-out barrier level (quote per 1 unit of base), e.g. 1.30.
 * @param side Optional UP (default, up-and-out) or DOWN (down-and-out).
 * @param windowStart Optional active-window start in years (>= 0; default 0 => front partial).
 * @param windowEnd Optional active-window end in years (> windowStart, <= expiry; default expiry => back partial).
 * @param mcPairs Optional Monte-Carlo antithetic pairs (0 => exact ADI-PDE, no std-error).
 * @param mcSteps Optional Monte-Carlo time steps (ignored when mcPairs = 0; 0 => server default).
 * @param mcSeed Optional Monte-Carlo seed (bit-reproducible; ignored when mcPairs = 0).
 * @returns A spill: premium, (std_error if MC), the 13 Greeks, and a convention footer.
 */
export async function WINDOWBARRIER(
  pair: string,
  tenor: string,
  strikeOrDelta: string,
  callPut: string,
  notional: number,
  barrier: number,
  side?: string,
  windowStart?: number,
  windowEnd?: number,
  mcPairs?: number,
  mcSteps?: number,
  mcSeed?: number,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeWindowBarrier({
      pair,
      tenor,
      strikeOrDelta,
      callPut,
      notional,
      barrier,
      side,
      windowStart,
      windowEnd,
      mcPairs,
      mcSteps,
      mcSeed,
    });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `window-barrier:${pair}:${tenor}:${strikeOrDelta}:${callPut}:${notional}:${barrier}:${side ?? ""}:${windowStart ?? ""}:${windowEnd ?? ""}:${mcPairs ?? ""}:${mcSteps ?? ""}:${mcSeed ?? ""}`,
    );
    // The exact ADI-PDE route carries no std-error; the MC route may — surface it
    // honestly when present so an MC premium is never mistaken for an exact one.
    return formatPathDependentSpill({
      premium: quote.greeks.price,
      stdError: quote.priceStdError,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a digital (binary) option as a labelled spill of `["premium", PV]`, the 13
 * risk Greeks, and a convention footer. The premium is the server's exact
 * closed-form value (the same `celnet-exotics` value the SDK/CLI read). The strike
 * must be an absolute level; `callPut` selects the above-strike (call) vs
 * below-strike (put) payoff; `style` cash-or-nothing (default) vs asset-or-nothing;
 * `payout` the fixed cash payout (omit => unit payout).
 * @customfunction DIGITAL
 * @param pair Currency pair, e.g. EURUSD.
 * @param tenor Tenor, e.g. 1Y.
 * @param strike Absolute strike level, e.g. 1.10.
 * @param callPut C for the above-strike payoff, P for the below-strike payoff.
 * @param notional Trade notional in the base currency.
 * @param style Optional CASH_OR_NOTHING (default) or ASSET_OR_NOTHING.
 * @param payout Optional fixed payout amount (omit => 0/unit).
 * @returns A spill: premium, the 13 Greeks, and a convention footer.
 */
export async function DIGITAL(
  pair: string,
  tenor: string,
  strike: number,
  callPut: string,
  notional: number,
  style?: string,
  payout?: number,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeDigital({ pair, tenor, strike, callPut, notional, style, payout });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `digital:${pair}:${tenor}:${strike}:${callPut}:${notional}:${style ?? ""}:${payout ?? ""}`,
    );
    return formatExoticPremiumSpill({
      premium: quote.greeks.price,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a touch structure — one-touch / no-touch / double-no-touch /
 * double-one-touch — as a labelled spill of `["premium", PV]`, the 13 risk Greeks,
 * and a convention footer. The premium is the server's exact closed-form value (the
 * same `celnet-exotics` value the SDK/CLI read). A touch pays the `rebate` on the
 * touch condition and has no vanilla payoff. The single kinds (OT/NT) use `barrier`
 * as the sole level; the double kinds (DNT/DOT) require BOTH `barrier` (the lower)
 * and a strictly-greater `upperBarrier`.
 * @customfunction TOUCH
 * @param pair Currency pair, e.g. EURUSD.
 * @param tenor Tenor, e.g. 1Y.
 * @param kind Touch family: OT (one-touch), NT (no-touch), DNT (double-no-touch), DOT (double-one-touch).
 * @param barrier The (lower / sole) barrier level.
 * @param notional Trade notional in the base currency.
 * @param rebate Optional rebate paid when the touch condition is satisfied (omit => 0).
 * @param upperBarrier Optional upper barrier (required for DNT/DOT; rejected for OT/NT).
 * @param monitoring Optional CONTINUOUS (default) or DISCRETE monitoring.
 * @returns A spill: premium, the 13 Greeks, and a convention footer.
 */
export async function TOUCH(
  pair: string,
  tenor: string,
  kind: string,
  barrier: number,
  notional: number,
  rebate?: number,
  upperBarrier?: number,
  monitoring?: string,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeTouch({
      pair,
      tenor,
      kind,
      barrier,
      notional,
      rebate,
      upperBarrier,
      monitoring,
    });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `touch:${pair}:${tenor}:${kind}:${barrier}:${notional}:${rebate ?? ""}:${upperBarrier ?? ""}:${monitoring ?? ""}`,
    );
    return formatExoticPremiumSpill({
      premium: quote.greeks.price,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a variance swap: a 2×2 spill `["fair_variance", K_var]` /
 * `["fair_vol", √K_var]` plus a convention footer. The fair variance strike is
 * the log-contract static-replication strike the server computes (the same
 * `celnet-exotics` closed form the SDK/CLI read); the fair vol is its √. Supply a
 * non-zero `strikeVol` only to pin a fixed strike — a fresh quote (zero/omitted)
 * reads the fair strike off the response.
 * @customfunction VARSWAP
 * @param pair Currency pair, e.g. "EURUSD".
 * @param tenor Tenor, e.g. "1Y".
 * @param notional Trade notional in the base currency.
 * @param strikeVol Optional fixed strike vol to pin (absolute, e.g. 0.11); omit for the fair strike.
 * @returns A spill: fair_variance, fair_vol, and a convention footer.
 */
export async function VARSWAP(
  pair: string,
  tenor: string,
  notional: number,
  strikeVol?: number,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeVarianceSwap({ pair, tenor, notional, strikeVol });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `varswap:${pair}:${tenor}:${notional}:${strikeVol ?? 0}`,
    );
    // The server returns the fair variance strike on both `resolved_strike` and
    // `greeks.price` (K_var); read the resolved strike (the canonical strike echo).
    return formatVarSwapSpill({
      fairVariance: quote.resolvedStrike,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a volatility swap: a 1×2 spill `["fair_vol", K_vol]` plus a convention
 * footer. `K_vol` is the convexity-adjusted fair volatility strike (strictly below
 * `√K_var` for any non-degenerate smile), the same `celnet-exotics` closed form
 * the SDK/CLI read.
 * @customfunction VOLSWAP
 * @param pair Currency pair, e.g. "EURUSD".
 * @param tenor Tenor, e.g. "1Y".
 * @param notional Trade notional in the base currency.
 * @param strikeVol Optional fixed strike vol to pin (absolute, e.g. 0.11); omit for the fair strike.
 * @returns A spill: fair_vol and a convention footer.
 */
export async function VOLSWAP(
  pair: string,
  tenor: string,
  notional: number,
  strikeVol?: number,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeVolatilitySwap({ pair, tenor, notional, strikeVol });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `volswap:${pair}:${tenor}:${notional}:${strikeVol ?? 0}`,
    );
    // The server returns the fair vol strike on both `resolved_strike` and
    // `greeks.price` (K_vol).
    return formatVolSwapSpill({
      fairVol: quote.resolvedStrike,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price an FX outright forward (deliverable): a labelled spill of `["premium", PV]`,
 * the 13 risk Greeks, and a convention footer. The PV is the server's exact
 * closed-form discounted-cashflow value from the dedicated linear book (the same
 * `celnet-linear` value the SDK/CLI read) — a linear product, so there is no
 * standard-error row. The pair must be DELIVERABLE; a non-deliverable pair is
 * rejected by the server (use `CELNET.NDF`).
 * @customfunction FORWARD
 * @param pair Currency pair, e.g. EURUSD (must be deliverable).
 * @param tenor Tenor, e.g. 1Y (the settlement date).
 * @param rate The contract (delivery) rate K, quote per 1 unit of base.
 * @param notional Trade notional in the base currency.
 * @param side Optional BUY (default, long the base forward) or SELL.
 * @returns A spill: premium, the 13 Greeks, and a convention footer.
 */
export async function FORWARD(
  pair: string,
  tenor: string,
  rate: number,
  notional: number,
  side?: string,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeForward({ pair, tenor, contractRate: rate, notional, side });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `forward:${pair}:${tenor}:${rate}:${notional}:${side ?? ""}`,
    );
    return formatPathDependentSpill({
      premium: quote.greeks.price,
      stdError: quote.priceStdError,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price an FX swap (deliverable): a labelled spill of `["premium", PV]`, the 13
 * net-risk Greeks, and a convention footer. The near leg settles at the spot date
 * and the far leg at the tenor (`rate`/`notional`/`side` drive both legs; the far
 * leg trades the opposite side). The PV is the net of the two legs — the same exact
 * `celnet-linear` value the SDK/CLI read; no standard-error row.
 * @customfunction SWAP
 * @param pair Currency pair, e.g. EURUSD (must be deliverable).
 * @param tenor Tenor of the far leg, e.g. 1Y (the near leg settles at spot).
 * @param rate The near leg's contract rate K (drives both legs).
 * @param notional Trade notional in the base currency.
 * @param nearSide Optional near-leg side BUY (default) or SELL (the far leg is the opposite).
 * @returns A spill: premium, the 13 net Greeks, and a convention footer.
 */
export async function SWAP(
  pair: string,
  tenor: string,
  rate: number,
  notional: number,
  nearSide?: string,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeSwap({ pair, tenor, contractRate: rate, notional, side: nearSide });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `swap:${pair}:${tenor}:${rate}:${notional}:${nearSide ?? ""}`,
    );
    return formatPathDependentSpill({
      premium: quote.greeks.price,
      stdError: quote.priceStdError,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a non-deliverable forward (NDF): a labelled spill of `["premium", PV]`, the
 * 13 risk Greeks, and a convention footer. Cash-settled in the convertible
 * (settlement) currency at the named `fixing`; the risk-neutral PV is identical to a
 * deliverable forward of equal terms (the same exact `celnet-linear` value the
 * SDK/CLI read; no standard-error row). The pair must be NON-DELIVERABLE; a
 * deliverable pair is rejected (use `CELNET.FORWARD`). The `fixing` is convention
 * identity only — the live fixing VALUE is an estate-gated feed, never sourced
 * in-repo.
 * @customfunction NDF
 * @param pair Currency pair, e.g. USDBRL (must be non-deliverable).
 * @param tenor Tenor, e.g. 6M (the settlement date).
 * @param rate The contract (forward) rate K, settlement-ccy per 1 unit of base.
 * @param notional Trade notional in the base currency.
 * @param fixing The published settlement-rate fixing, e.g. BRL.PTAX, COP.TRM, INR.RBIB.
 * @param settlementCcy Optional convertible settlement currency (default USD).
 * @param side Optional BUY (default, long the base forward) or SELL.
 * @returns A spill: premium, the 13 Greeks, and a convention footer.
 */
export async function NDF(
  pair: string,
  tenor: string,
  rate: number,
  notional: number,
  fixing: string,
  settlementCcy?: string,
  side?: string,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeNdf({
      pair,
      tenor,
      contractRate: rate,
      notional,
      fixing,
      settlementCcy,
      side,
    });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `ndf:${pair}:${tenor}:${rate}:${notional}:${fixing}:${settlementCcy ?? ""}:${side ?? ""}`,
    );
    return formatPathDependentSpill({
      premium: quote.greeks.price,
      stdError: quote.priceStdError,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a fixed-strike arithmetic-average-rate Asian option: a spill of
 * `["premium", PV]`, the 13 risk Greeks, and a convention footer. The premium is
 * the discounted option PV the server computes via the chosen analytic estimator
 * (the same `celnet-exotics` closed form the SDK/CLI read). The strike must be an
 * absolute level. DISCRETE averaging needs `observations ≥ 1`; CONTINUOUS ignores
 * it. The seasoning pair (`elapsedAvg`, `elapsedWeight`) prices an in-progress
 * average (e.g. 3 of 12 fixings done ⇒ elapsedWeight 0.25); omit for a fresh one.
 * @customfunction ASIAN
 * @param pair Currency pair, e.g. "EURUSD".
 * @param tenor Tenor, e.g. "1Y".
 * @param strike Absolute strike level, e.g. 1.10.
 * @param callPut "C" for call, "P" for put.
 * @param notional Trade notional in the base currency.
 * @param averaging Optional averaging style: DISCRETE (default) or CONTINUOUS.
 * @param observations Number of equally-spaced fixings (DISCRETE; ≥ 1).
 * @param method Optional analytic estimator: CURRAN (default) or TW.
 * @param elapsedAvg Optional realised running average of the fixed observations (seasoning).
 * @param elapsedWeight Optional fraction ∈ [0,1) of the average already accumulated.
 * @returns A spill: premium, the 13 Greeks, and a convention footer.
 */
export async function ASIAN(
  pair: string,
  tenor: string,
  strike: number,
  callPut: string,
  notional: number,
  averaging?: string,
  observations?: number,
  method?: string,
  elapsedAvg?: number,
  elapsedWeight?: number,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeAsianOption({
      pair,
      tenor,
      strike,
      callPut,
      notional,
      averaging,
      observations,
      method,
      elapsedAvg,
      elapsedWeight,
    });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `asian:${pair}:${tenor}:${strike}:${callPut}:${notional}:${averaging ?? ""}:${observations ?? ""}:${method ?? ""}:${elapsedAvg ?? ""}:${elapsedWeight ?? ""}`,
    );
    return formatAsianSpill({
      premium: quote.greeks.price,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a forward-start vanilla: a spill of `["premium", PV]`, the 13 risk Greeks,
 * and a convention footer. The strike is fixed at the future reset to
 * `moneyness × S(reset)` and the option runs to the tenor; the premium is the
 * server's discounted dual-carry strike-reset closed form (the same
 * `celnet-exotics` value the SDK/CLI read). The reset must lie in `[0, expiry]`.
 * @customfunction FORWARDSTART
 * @param pair Currency pair, e.g. "EURUSD".
 * @param tenor Tenor, e.g. "1Y".
 * @param callPut "C" for call, "P" for put.
 * @param moneyness Proportional strike multiplier on the reset-date spot (e.g. 1.0 = ATM-at-reset).
 * @param reset Reset (strike-fixing) date as a year fraction in [0, expiry], e.g. 0.25.
 * @param notional Trade notional in the base currency.
 * @returns A spill: premium, the 13 Greeks, and a convention footer.
 */
export async function FORWARDSTART(
  pair: string,
  tenor: string,
  callPut: string,
  moneyness: number,
  reset: number,
  notional: number,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeForwardStart({ pair, tenor, callPut, moneyness, reset, notional });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `fwdstart:${pair}:${tenor}:${callPut}:${moneyness}:${reset}:${notional}`,
    );
    return formatForwardStartSpill({
      premium: quote.greeks.price,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a cliquet (ratchet): a spill of `["premium", PV]`, then — ONLY for a
 * clamped cliquet, which is Monte-Carlo-priced — an honest `["std_error", σ̄]` row,
 * then the 13 risk Greeks and a convention footer. A *plain* ratchet (no clamp
 * supplied) is the exact closed-form sum of forward-start legs and carries NO
 * std-error row; supplying any local/global floor or cap makes it MC-priced. The
 * four clamps are presence-tracked — an omitted clamp is unconstrained on that
 * side (never floored/capped at zero). `mcPairs` (0 ⇒ the server default) and
 * `mcSeed` tune the clamped MC and are ignored by a plain ratchet.
 * @customfunction CLIQUET
 * @param pair Currency pair, e.g. "EURUSD".
 * @param tenor Tenor, e.g. "1Y".
 * @param callPut "C" for call, "P" for put.
 * @param moneyness Proportional strike multiplier for each period's leg (e.g. 1.0).
 * @param periods Number of equal ratchet sub-periods (≥ 1).
 * @param notional Trade notional in the base currency.
 * @param localFloor Optional per-period return floor (omit ⇒ unconstrained).
 * @param localCap Optional per-period return cap (omit ⇒ unconstrained).
 * @param globalFloor Optional summed-payoff floor (omit ⇒ unconstrained).
 * @param globalCap Optional summed-payoff cap (omit ⇒ unconstrained).
 * @param mcPairs Optional Monte-Carlo antithetic pairs for the clamped case (0 ⇒ server default).
 * @param mcSeed Optional Monte-Carlo seed for the clamped case (bit-reproducible).
 * @returns A spill: premium, (std_error if MC), the 13 Greeks, and a convention footer.
 */
export async function CLIQUET(
  pair: string,
  tenor: string,
  callPut: string,
  moneyness: number,
  periods: number,
  notional: number,
  localFloor?: number,
  localCap?: number,
  globalFloor?: number,
  globalCap?: number,
  mcPairs?: number,
  mcSeed?: number,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeCliquet({
      pair,
      tenor,
      callPut,
      moneyness,
      periods,
      notional,
      localFloor,
      localCap,
      globalFloor,
      globalCap,
      mcPairs,
      mcSeed,
    });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `cliquet:${pair}:${tenor}:${callPut}:${moneyness}:${periods}:${notional}:${localFloor ?? ""}:${localCap ?? ""}:${globalFloor ?? ""}:${globalCap ?? ""}:${mcPairs ?? ""}:${mcSeed ?? ""}`,
    );
    // The std-error is surfaced ONLY when this cliquet is MC-priced (clamped) AND
    // the server stamped `price_std_error` on the reply. A plain ratchet (closed
    // form) carries none, so the spill honestly omits the row — a cell never reads
    // a precision claim the price doesn't have. `cliquetIsMonteCarlo` gates on the
    // shaped product so we never surface a stray non-MC std-error.
    const isMc =
      instrument.product.kind === "cliquet" && cliquetIsMonteCarlo(instrument.product.cliquet);
    return formatCliquetSpill({
      premium: quote.greeks.price,
      stdError: isMc ? quote.priceStdError : undefined,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a quanto option: a spill of `["premium", PV]`, the 13 risk Greeks, and a
 * convention footer. The payoff is computed on the foreign underlying but settled
 * in the fixed (domestic) currency, with a drift adjustment from the
 * underlying↔settlement-FX correlation; the premium is the server's closed-form
 * quanto-adjusted value (the same `celnet-exotics` value the SDK/CLI read). The
 * strike is an absolute level. `payoff` selects VANILLA (default) or DIGITAL.
 * @customfunction QUANTO
 * @param pair Currency pair, e.g. "EURUSD".
 * @param tenor Tenor, e.g. "1Y".
 * @param callPut "C" for call, "P" for put.
 * @param strike Absolute strike level, e.g. 1.10.
 * @param notional Trade notional in the base currency.
 * @param conversionVol Volatility of the settlement-FX conversion rate (absolute, e.g. 0.09).
 * @param correlation Correlation in [-1, 1] between the underlying and the settlement-FX rate.
 * @param payoff Optional payoff: VANILLA (default) or DIGITAL.
 * @returns A spill: premium, the 13 Greeks, and a convention footer.
 */
export async function QUANTO(
  pair: string,
  tenor: string,
  callPut: string,
  strike: number,
  notional: number,
  conversionVol: number,
  correlation: number,
  payoff?: string,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeQuanto({
      pair,
      tenor,
      callPut,
      strike,
      notional,
      conversionVol,
      correlation,
      payoff,
    });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `quanto:${pair}:${tenor}:${callPut}:${strike}:${notional}:${conversionVol}:${correlation}:${payoff ?? ""}`,
    );
    return formatQuantoSpill({
      premium: quote.greeks.price,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a Target-Redemption Forward (TARF): a spill of `["premium", PV]`, an honest
 * `["std_error", σ̄]` row (a TARF is ALWAYS Monte-Carlo priced — the breaching
 * fixing carries genuine gap risk), the 13 risk Greeks, and a convention footer.
 * The premium is the BANK's present value (the server's `greeks.price`, the same
 * `celnet-exotics` value the SDK/CLI read). The strike is an absolute level;
 * `target > 0` is the cumulative client gain that redeems; `leverage ≥ 0` gears the
 * adverse leg; `fixings ≥ 1` is the (equally-spaced) fixing count.
 * @customfunction TARF
 * @param pair Currency pair, e.g. EURUSD.
 * @param tenor Tenor, e.g. 1Y.
 * @param callPut C for the favourable-up direction, P for the classic exporter (favourable-down) TARF.
 * @param strike Absolute strike level of every fixing, e.g. 1.10.
 * @param target Cumulative client-gain target that redeems the structure (> 0).
 * @param leverage Gearing on the adverse (loss) leg (>= 0).
 * @param fixings Number of equally-spaced fixings (>= 1).
 * @param notional Trade notional in the base currency.
 * @param redemption Optional gap-risk settlement: FULL_GAIN (default) or CAPPED_GAIN.
 * @param fixingNotional Optional notional that accrues at each fixing (omit => 1.0).
 * @param mcPairs Optional Monte-Carlo antithetic pairs (0 => server default).
 * @param mcSeed Optional Monte-Carlo seed (bit-reproducible).
 * @returns A spill: premium, std_error, the 13 Greeks, and a convention footer.
 */
export async function TARF(
  pair: string,
  tenor: string,
  callPut: string,
  strike: string | number,
  target: number,
  leverage: number,
  fixings: number,
  notional: number,
  redemption?: string,
  fixingNotional?: number,
  mcPairs?: number,
  mcSeed?: number,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeTarf({
      pair,
      tenor,
      callPut,
      strike,
      target,
      leverage,
      fixings,
      notional,
      redemption,
      fixingNotional,
      mcPairs,
      mcSeed,
    });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `tarf:${pair}:${tenor}:${callPut}:${strike}:${target}:${leverage}:${fixings}:${notional}:${redemption ?? ""}:${fixingNotional ?? ""}:${mcPairs ?? ""}:${mcSeed ?? ""}`,
    );
    // A TARF is always MC-priced ⇒ the std-error row is surfaced when the server
    // stamped `price_std_error` (it always does for an MC product).
    return formatPathDependentSpill({
      premium: quote.greeks.price,
      stdError: quote.priceStdError,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price an accumulator: a spill of `["premium", PV]`, an honest `["std_error", σ̄]`
 * row (an accumulator is ALWAYS Monte-Carlo priced), the 13 risk Greeks, and a
 * convention footer. The premium is the CLIENT's present value (the server's
 * `greeks.price`, the same `celnet-exotics` value the SDK/CLI read). The pivot and
 * barrier are absolute levels (`barrier > pivot`); `leverage ≥ 0` gears the
 * below-pivot leg; `fixings ≥ 1` is the (equally-spaced) fixing count.
 * @customfunction ACCUMULATOR
 * @param pair Currency pair, e.g. EURUSD.
 * @param tenor Tenor, e.g. 1Y.
 * @param pivot Absolute pivot strike level at which the client accumulates, e.g. 1.10.
 * @param barrier Absolute up-and-out knock-out barrier (> pivot), e.g. 1.15.
 * @param leverage Gearing on the below-pivot (loss) leg (>= 0).
 * @param fixings Number of equally-spaced fixings (>= 1).
 * @param notional Trade notional in the base currency.
 * @param monitoring Optional knock-out monitoring: DISCRETE (default) or CONTINUOUS.
 * @param fixingNotional Optional notional that accrues at each fixing (omit => 1.0).
 * @param mcPairs Optional Monte-Carlo antithetic pairs (0 => server default).
 * @param mcSeed Optional Monte-Carlo seed (bit-reproducible).
 * @returns A spill: premium, std_error, the 13 Greeks, and a convention footer.
 */
export async function ACCUMULATOR(
  pair: string,
  tenor: string,
  pivot: string | number,
  barrier: number,
  leverage: number,
  fixings: number,
  notional: number,
  monitoring?: string,
  fixingNotional?: number,
  mcPairs?: number,
  mcSeed?: number,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeAccumulator({
      pair,
      tenor,
      pivot,
      barrier,
      leverage,
      fixings,
      notional,
      monitoring,
      fixingNotional,
      mcPairs,
      mcSeed,
    });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `accumulator:${pair}:${tenor}:${pivot}:${barrier}:${leverage}:${fixings}:${notional}:${monitoring ?? ""}:${fixingNotional ?? ""}:${mcPairs ?? ""}:${mcSeed ?? ""}`,
    );
    // An accumulator is always MC-priced ⇒ surface the server's std-error.
    return formatPathDependentSpill({
      premium: quote.greeks.price,
      stdError: quote.priceStdError,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a lookback option: a spill of `["premium", PV]`, then — ONLY for the
 * DISCRETE (Monte-Carlo) variant — an honest `["std_error", σ̄]` row, then the 13
 * risk Greeks and a convention footer. The CONTINUOUS variant prices by the exact
 * closed form (no std-error row); the DISCRETE variant prices by Monte-Carlo over
 * `observations` monitoring points. A FLOATING-strike lookback takes no strike (it
 * settles against the path extremum); a FIXED-strike lookback requires an absolute
 * strike. The premium is the server's `celnet-exotics` value (== SDK/CLI).
 * @customfunction LOOKBACK
 * @param pair Currency pair, e.g. EURUSD.
 * @param tenor Tenor, e.g. 1Y.
 * @param callPut C for call, P for put.
 * @param notional Trade notional in the base currency.
 * @param style Optional FLOATING (default, settle vs path extremum) or FIXED.
 * @param monitoring Optional CONTINUOUS (default, exact closed form) or DISCRETE (Monte-Carlo).
 * @param strike Optional absolute strike level (required for FIXED; rejected for FLOATING).
 * @param observations Optional monitoring observations for the DISCRETE variant (0 => server default).
 * @param mcPairs Optional Monte-Carlo antithetic pairs for the DISCRETE variant (0 => server default).
 * @param mcSeed Optional Monte-Carlo seed for the DISCRETE variant (bit-reproducible).
 * @returns A spill: premium, (std_error if DISCRETE/MC), the 13 Greeks, and a footer.
 */
export async function LOOKBACK(
  pair: string,
  tenor: string,
  callPut: string,
  notional: number,
  style?: string,
  monitoring?: string,
  strike?: string | number,
  observations?: number,
  mcPairs?: number,
  mcSeed?: number,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeLookback({
      pair,
      tenor,
      callPut,
      notional,
      style,
      monitoring,
      strike,
      observations,
      mcPairs,
      mcSeed,
    });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `lookback:${pair}:${tenor}:${callPut}:${notional}:${style ?? ""}:${monitoring ?? ""}:${strike ?? ""}:${observations ?? ""}:${mcPairs ?? ""}:${mcSeed ?? ""}`,
    );
    // The std-error is surfaced ONLY for the DISCRETE (MC-priced) variant AND when
    // the server stamped `price_std_error`. The CONTINUOUS variant is exact closed
    // form, so the spill honestly omits the row — a cell never reads a precision
    // claim the price doesn't have. `lookbackIsMonteCarlo` gates on the shaped
    // product so we never surface a stray non-MC std-error.
    const isMc =
      instrument.product.kind === "lookback" && lookbackIsMonteCarlo(instrument.product.lookback);
    return formatPathDependentSpill({
      premium: quote.greeks.price,
      stdError: isMc ? quote.priceStdError : undefined,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price an American / Bermudan early-exercise vanilla: a spill of `["premium", PV]`,
 * then — ONLY for the Longstaff-Schwartz Monte-Carlo engine (`lsmPaths > 0`) — an
 * honest `["std_error", σ̄]` row, then the 13 risk Greeks and a convention footer.
 * Physically-settled FX options trade American-style. The default engine is the
 * exact projected-SOR free-boundary finite difference (no std-error); `lsmPaths > 0`
 * selects the regression Monte-Carlo engine. AMERICAN (default) exercises
 * continuously to expiry; BERMUDAN — selected by `style` or by a positive
 * `bermudanSteps` — exercises only on the `n` equally-spaced dates `k/n · T`. The
 * strike must be an absolute level. The premium is the server's `celnet-exotics`
 * value (== SDK/CLI).
 * @customfunction AMERICAN
 * @param pair Currency pair, e.g. EURUSD.
 * @param tenor Tenor, e.g. 1Y.
 * @param strike Absolute strike level, e.g. 1.10.
 * @param callPut C for call, P for put.
 * @param notional Trade notional in the base currency.
 * @param style Optional AMERICAN (default, continuous) or BERMUDAN (discrete dates).
 * @param bermudanSteps Optional number of equally-spaced Bermudan exercise dates (k/n·T); >0 selects BERMUDAN (0 => continuous AMERICAN).
 * @param lsmPaths Optional Longstaff-Schwartz path count (0 => exact FD, no std-error; >0 => Monte-Carlo).
 * @param lsmExerciseDates Optional LSM AMERICAN exercise resolution (0 => server default; ignored for FD/BERMUDAN).
 * @param lsmSeed Optional LSM scramble seed (bit-reproducible; ignored by FD).
 * @returns A spill: premium, (std_error if LSM/MC), the 13 Greeks, and a convention footer.
 */
export async function AMERICAN(
  pair: string,
  tenor: string,
  strike: string | number,
  callPut: string,
  notional: number,
  style?: string,
  bermudanSteps?: number,
  lsmPaths?: number,
  lsmExerciseDates?: number,
  lsmSeed?: number,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeAmerican({
      pair,
      tenor,
      strike,
      callPut,
      notional,
      style,
      bermudanSteps,
      lsmPaths,
      lsmExerciseDates,
      lsmSeed,
    });
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `american:${pair}:${tenor}:${strike}:${callPut}:${notional}:${style ?? ""}:${bermudanSteps ?? ""}:${lsmPaths ?? ""}:${lsmExerciseDates ?? ""}:${lsmSeed ?? ""}`,
    );
    // The std-error is surfaced ONLY for the Longstaff-Schwartz Monte-Carlo engine
    // (lsmPaths > 0) AND when the server stamped `price_std_error`. The exact FD
    // engine reports none, so the spill honestly omits the row — a cell never reads
    // a precision claim the price doesn't have. `americanIsMonteCarlo` gates on the
    // shaped product so we never surface a stray non-MC std-error.
    const isMc =
      instrument.product.kind === "american" && americanIsMonteCarlo(instrument.product.american);
    return formatPathDependentSpill({
      premium: quote.greeks.price,
      stdError: isMc ? quote.priceStdError : undefined,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a correlated multi-asset basket / best-of / worst-of option: a spill of
 * `["premium", PV]`, an honest `["std_error", σ̄]` row (a basket is ALWAYS
 * Monte-Carlo priced over N correlated FX legs), the 13 risk Greeks, and a
 * convention footer. The premium is the server's `celnet-exotics` multi-asset MC
 * value (== the SDK/CLI), discounted at the shared domestic rate. Each leg row of
 * `legs` is `[pair, weight, spot, vol, rFor]`; `correlations` is the N×N
 * correlation range (encoded ROW-MAJOR on the wire). The settlement / numeraire
 * pair is the top-level `pair`; the underlyings are the per-leg pairs. The server
 * validates the correlation matrix is symmetric / unit-diagonal / positive-definite
 * (a non-PSD matrix is rejected, never regularised).
 *
 * Multi-asset basket Greeks (a per-leg N×{spot,vol} Jacobian + cross-gammas) are a
 * distinct larger increment and are NOT yet computed: the server returns a zeroed
 * Greek strip alongside the priced premium + measured MC std-error, so the spill's
 * Greek rows are honestly `0` (the headline number — premium and its std-error —
 * is real; the sensitivities are deferred, not faked).
 * @customfunction BASKET
 * @param pair Settlement / numeraire currency pair, e.g. EURUSD.
 * @param tenor Tenor, e.g. 1Y.
 * @param callPut C for call, P for put on the aggregated underlying.
 * @param strike Absolute strike level on the aggregated underlying, e.g. 1.18.
 * @param notional Trade notional in the base currency.
 * @param legs The legs as a range, one row per leg: [pair, weight, spot, vol, rFor].
 * @param correlations The N×N correlation matrix as a range (row-major), for the N legs.
 * @param kind Optional aggregation: BASKET (default, weighted sum), BEST_OF (max) or WORST_OF (min).
 * @param mcPaths Optional scrambled-Sobol points per replication (0 => server default).
 * @param mcReplications Optional independent randomized scrambles (0 => server default).
 * @param mcSteps Optional time steps per path (0 => server default).
 * @param mcSeed Optional base scramble seed (bit-reproducible; 0 => default).
 * @returns A spill: premium, std_error, the 13 Greeks (deferred => 0), and a convention footer.
 */
export async function BASKET(
  pair: string,
  tenor: string,
  callPut: string,
  strike: number,
  notional: number,
  legs: (string | number)[][],
  correlations: number[][],
  kind?: string,
  mcPaths?: number,
  mcReplications?: number,
  mcSteps?: number,
  mcSeed?: number,
): Promise<SpillMatrix> {
  try {
    const instrument = shapeBasket({
      pair,
      tenor,
      notional,
      callPut,
      strike,
      kind,
      legs,
      correlations,
      mcPaths,
      mcReplications,
      mcSteps,
      mcSeed,
    });
    const correlationKey = correlations.map((row) => row.join(",")).join(";");
    const legsKey = legs.map((row) => row.join(",")).join(";");
    const quote = await getConnection().requestQuote(
      instrument,
      DEFAULT_CONVENTIONS,
      `basket:${pair}:${tenor}:${callPut}:${strike}:${notional}:${kind ?? ""}:${legsKey}:${correlationKey}:${mcPaths ?? ""}:${mcReplications ?? ""}:${mcSteps ?? ""}:${mcSeed ?? ""}`,
    );
    // A basket is always multi-asset Monte-Carlo priced ⇒ the server genuinely
    // stamps `price_std_error`; surface it honestly. The Greek strip is the
    // server's zeroed price-only strip (multi-asset Greeks are the next increment).
    return formatPathDependentSpill({
      premium: quote.greeks.price,
      stdError: quote.priceStdError,
      greeks: quote.greeks,
      conventions: quote.conventions,
      surfaceVersion: quote.surfaceVersion,
      epochNanos: quote.epochNanos,
    });
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Stream a live two-way for a structure, multiplexed on the single session and
 * coalesced with identical-argument cells. Re-emits on every Update; flips to a
 * stale state on a heartbeat gap rather than freezing as live (docs §5).
 * @customfunction SUBSCRIBE
 * @param pair Currency pair, e.g. "EURUSD".
 * @param tenor Tenor, e.g. "1Y".
 * @param strikeOrDelta Absolute strike or delta ("25dP", "ATM").
 * @param callPut "C" or "P".
 * @param notional Trade notional in the base currency.
 * @param invocation The streaming invocation handle (auto-supplied).
 * @streaming
 */
export function SUBSCRIBE(
  pair: string,
  tenor: string,
  strikeOrDelta: string,
  callPut: string,
  notional: number,
  invocation: CustomFunctions.StreamingInvocation<string>,
): void {
  let instrument;
  try {
    instrument = shapeVanillaInstrument({ pair, tenor, strikeOrDelta, callPut, notional });
  } catch (err) {
    invocation.setResult(toCfError(err) as unknown as string);
    return;
  }
  const registry = getRegistry();
  const label = `${pair} ${tenor} ${strikeOrDelta}${callPut}`;
  const { release } = registry.acquire(instrument, DEFAULT_CONVENTIONS, label, (tick: LiveTick) => {
    invocation.setResult(renderLiveCell(tick));
  });
  // Office.js calls onCanceled when the cell is deleted/recalculated away; tear
  // the subscription down (decrement refcount; unsubscribe at zero — no orphans).
  invocation.onCanceled = () => release();
}

/**
 * Stream a live market observable (a trend series) for a (pair, observable),
 * multiplexed on the single session and coalesced with identical-argument cells.
 * The cell re-emits on every appended point. Observables: ATM (ATM vol), SPOT,
 * RR (risk reversal), BF (butterfly), FWD (forward). ATM/RR/BF/FWD need a `tenor`;
 * RR/BF additionally need a `delta` wing (e.g. 0.25). SPOT is tenor-independent.
 * @customfunction SERIES
 * @param pair Currency pair, e.g. "EURUSD".
 * @param observable Observable: ATM, SPOT, RR, BF or FWD.
 * @param tenor Pillar tenor for ATM/RR/BF/FWD, e.g. "1Y" (omit for SPOT).
 * @param delta Signed/unsigned delta wing for RR/BF, e.g. 0.25 (omit otherwise).
 * @param invocation The streaming invocation handle (auto-supplied).
 * @streaming
 */
export function SERIES(
  pair: string,
  observable: string,
  tenor: string | undefined,
  delta: number | undefined,
  invocation: CustomFunctions.StreamingInvocation<string>,
): void {
  let req: MarketSeriesRequest;
  try {
    req = shapeSeriesRequest(pair, observable, tenor, delta);
  } catch (err) {
    invocation.setResult(toCfError(err) as unknown as string);
    return;
  }
  const registry = getSeriesRegistry();
  const { release } = registry.acquire(req, (tick: SeriesTick) => {
    invocation.setResult(
      formatSeriesCell({
        value: tick.value,
        observable: tick.observable,
        baselined: tick.baselined,
        epochNanos: tick.epochNanos,
      }),
    );
  });
  invocation.onCanceled = () => release();
}

/** Shape a `CELNET.SERIES` cell's arguments into a `MarketSeriesRequest`. */
function shapeSeriesRequest(
  pair: string,
  observable: string,
  tenor: string | undefined,
  delta: number | undefined,
): MarketSeriesRequest {
  const obs = parseObservable(observable);
  const ccy = parsePair(pair);
  const needsTenor = obs !== "SPOT";
  const needsDelta = obs === "RISK_REVERSAL" || obs === "BUTTERFLY";
  const req: { -readonly [K in keyof MarketSeriesRequest]?: MarketSeriesRequest[K] } = {
    pair: ccy,
    observable: obs,
  };
  if (needsTenor) {
    if (tenor === undefined || tenor.trim() === "") {
      throw new ShapingError(`observable ${observable} requires a tenor (e.g. 1Y)`);
    }
    req.tenor = parseTenor(tenor).tenor;
  }
  if (needsDelta) {
    if (delta === undefined || !Number.isFinite(delta)) {
      throw new ShapingError(`observable ${observable} requires a delta wing (e.g. 0.25)`);
    }
    // Accept a fraction (0.25) or a percent-delta (25); normalize to a fraction.
    req.delta = delta >= 1 ? delta / 100 : delta;
  }
  return req as MarketSeriesRequest;
}

/** Render a streamed tick to a single cell string: `bid/offer (health)`. */
function renderLiveCell(tick: LiveTick): string {
  if (tick.health === "STALE") {
    return `… ${fmt(tick.price.bid)}/${fmt(tick.price.offer)} (stale)`;
  }
  const tag = tick.health === "RESYNCING" ? " (resync)" : "";
  return `${fmt(tick.price.bid)}/${fmt(tick.price.offer)}${tag}`;
}

function fmt(x: number): string {
  return Number.isFinite(x) ? x.toFixed(5) : "—";
}

/**
 * Contribute a manual vol/mark back to the surface. Two-phase + idempotent: this
 * function STAGES the contribution (rendering a PENDING status) and the trader
 * confirms it in the task-pane Contribute panel — a recalc never silently writes
 * (docs §3.3). Returns a 1×3 status spill `[status, surfaceVersionAfter, detail]`.
 * @customfunction MARK
 * @param pair Currency pair, e.g. "EURUSD".
 * @param tenor Tenor, e.g. "1Y".
 * @param pillar Delta pillar ("ATM", "25dP", "10dC").
 * @param vol Absolute volatility, e.g. 0.102.
 * @param model Optional smile model the commit calibrates under: VV, SABR, SVI, SSVI.
 * @param comment Optional contribution comment.
 * @returns A status spill; the write commits only on task-pane confirmation.
 */
export async function MARK(
  pair: string,
  tenor: string,
  pillar: string,
  vol: number,
  model?: string,
  comment?: string,
): Promise<SpillMatrix> {
  try {
    const status = await stageMark(getConnection(), {
      pair,
      tenor,
      pillar,
      vol,
      comment: comment ?? "",
      model,
    });
    return formatMarkStatusSpill(status);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Hierarchical risk aggregate over the org cube — the SERVER computes the roll-up
 * (the API-first parity rule: a client never loops positions and sums). Spills the
 * node tree the server returns: a header row plus one row per rolled-up node, then a
 * reporting-numeraire / dimension footer.
 *
 * Aggregation goes through `celnet-risk-normalize` server-side into the chosen
 * reporting numeraire, so every measure is in that numeraire — there is no "native
 * premium units" caveat. Supply the per-currency conversion rates (units of the
 * numeraire per 1 unit of ccy at spot) as the optional `rates` range of `[ccy, rate]`
 * rows; the numeraire's own rate is implicit 1.0. A missing rate the book needs fails
 * the request loudly server-side (never a silent leg drop).
 * @customfunction RISK
 * @param dimension Roll-up dimension: FIRM, TRADER, BOOK, DESK, PAIR, LOCATION or ENTITY.
 * @param numeraire Reporting currency, e.g. USD (3-letter code).
 * @param rates Optional [ccy, rate] rows: numeraire units per 1 unit of ccy at spot.
 * @param scope Optional scope DIM:value (e.g. DESK:99) to narrow before the group-by.
 * @returns A node grid: header, one row per node, and a numeraire/dimension footer.
 */
export async function RISK(
  dimension: string,
  numeraire: string,
  rates?: unknown,
  scope?: string,
): Promise<SpillMatrix> {
  try {
    const dim = parseRiskDimension(dimension);
    const reporting = shapeReportingNumeraire(numeraire, rates);
    const scopeKey = parseRiskScope(typeof scope === "string" ? scope : undefined);
    const body = aggregateRiskRequest({
      dimension: dim,
      numeraire: reporting,
      ...(scopeKey !== undefined ? { scope: scopeKey } : {}),
    });
    const reply = await getConnection().aggregateRisk(body);
    const result = aggregateRiskResponseFromWire(reply);
    return formatRiskSpill(result.dimension, result.numeraire, result.nodes);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * List the entitled open positions the cube aggregates (the desk's book), each with
 * its org placement + attribution. Spills a leaf grid; an honest empty-state row when
 * the principal (default grant-all) sees nothing. The list is the drill-down behind a
 * CELNET.RISK node — same single contract, server-owned book.
 * @customfunction POSITIONS
 * @param scope Optional scope DIM:value (e.g. BOOK:7) to narrow the listing (omit ⇒ all).
 * @returns A position grid: header, one row per position, and a count/empty footer.
 */
export async function POSITIONS(scope?: string): Promise<SpillMatrix> {
  try {
    const scopeKey: RiskScope | undefined = parseRiskScope(
      typeof scope === "string" ? scope : undefined,
    );
    const body = listPositionsRequest(scopeKey !== undefined ? { scope: scopeKey } : {});
    const reply = await getConnection().listPositions(body);
    const result = listPositionsResponseFromWire(reply);
    return formatPositionsSpill(result.positions);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * The limit-tree utilization + RAG for a scope node — the limits configured at the
 * scope, each with cap / exposure / ratio / RAG status / enforcement / headroom,
 * computed server-side against the node's aggregated exposure. Spills a utilization
 * grid plus a worst-RAG / hard-breach footer.
 * @customfunction LIMITS
 * @param scope Scope DIM:value (e.g. DESK:99) or FIRM for the firm apex.
 * @param numeraire Reporting currency the additive exposures are expressed in, e.g. USD.
 * @param rates Optional [ccy, rate] rows: numeraire units per 1 unit of ccy at spot.
 * @returns A limit grid: header, one row per limit, and a worst-RAG / breach footer.
 */
export async function LIMITS(
  scope: string,
  numeraire: string,
  rates?: unknown,
): Promise<SpillMatrix> {
  try {
    const scopeKey = parseRiskScope(scope) ?? { dimension: "FIRM" as const, value: 0n };
    const reporting = shapeReportingNumeraire(numeraire, rates);
    const body = limitStatusRequest({ scope: scopeKey, numeraire: reporting });
    const reply = await getConnection().limitStatus(body);
    const result = limitStatusResponseFromWire(reply);
    const scopeLabel =
      scopeKey.dimension === "FIRM" ? "FIRM" : `${scopeKey.dimension}:${scopeKey.value}`;
    return formatLimitsSpill(scopeLabel, result.limits, result.worst, result.hardBreach);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Live server observability for the desk: the connection state plus the latest
 * server heartbeat's drain-side price latency (p50/p99/p99.9), the
 * `celnet-fanout` ring conflation-drop count, and the surface-version /
 * correlation provenance echo. Every value is the server's own observability
 * stamp surfaced over the ONE contract's `Heartbeat` (HdrHistogram percentiles +
 * the ring's `received + skipped == produced` accounting); nothing is computed
 * locally. This is a passive status cell — it never sends a request, it reads the
 * most recent beat the shared connection has already seen, so it is safe to leave
 * live. Before the first beat it renders an honest waiting state.
 * @customfunction STATUS
 * @returns A 2-row spill: a header and a live server-health value row.
 */
export async function STATUS(): Promise<SpillMatrix> {
  try {
    const conn = getConnection();
    return formatServerStatusSpill(conn.isOpen(), conn.latestHeartbeat());
  } catch (err) {
    throw toCfError(err);
  }
}

// Register the functions with the Office.js custom-function association map when
// running inside the host (the `CustomFunctions` global exists). Under node (no
// host) this is a no-op, so the pure logic stays importable for unit tests.
type AnyCustomFunctions = {
  associate: (id: string, fn: (...args: never[]) => unknown) => void;
};

function registerAll(): void {
  const cf = (globalThis as unknown as { CustomFunctions?: AnyCustomFunctions }).CustomFunctions;
  if (!cf) return;
  cf.associate("PRICE", PRICE as (...a: never[]) => unknown);
  cf.associate("GREEKS", GREEKS as (...a: never[]) => unknown);
  cf.associate("SURFACE", SURFACE as (...a: never[]) => unknown);
  cf.associate("MARKSURFACE", MARKSURFACE as (...a: never[]) => unknown);
  cf.associate("RFQ", RFQ as (...a: never[]) => unknown);
  cf.associate("BARRIER", BARRIER as (...a: never[]) => unknown);
  cf.associate("WINDOWBARRIER", WINDOWBARRIER as (...a: never[]) => unknown);
  cf.associate("DIGITAL", DIGITAL as (...a: never[]) => unknown);
  cf.associate("TOUCH", TOUCH as (...a: never[]) => unknown);
  cf.associate("VARSWAP", VARSWAP as (...a: never[]) => unknown);
  cf.associate("VOLSWAP", VOLSWAP as (...a: never[]) => unknown);
  cf.associate("FORWARD", FORWARD as (...a: never[]) => unknown);
  cf.associate("SWAP", SWAP as (...a: never[]) => unknown);
  cf.associate("NDF", NDF as (...a: never[]) => unknown);
  cf.associate("ASIAN", ASIAN as (...a: never[]) => unknown);
  cf.associate("FORWARDSTART", FORWARDSTART as (...a: never[]) => unknown);
  cf.associate("CLIQUET", CLIQUET as (...a: never[]) => unknown);
  cf.associate("QUANTO", QUANTO as (...a: never[]) => unknown);
  cf.associate("TARF", TARF as (...a: never[]) => unknown);
  cf.associate("ACCUMULATOR", ACCUMULATOR as (...a: never[]) => unknown);
  cf.associate("LOOKBACK", LOOKBACK as (...a: never[]) => unknown);
  cf.associate("AMERICAN", AMERICAN as (...a: never[]) => unknown);
  cf.associate("BASKET", BASKET as (...a: never[]) => unknown);
  cf.associate("SUBSCRIBE", SUBSCRIBE as (...a: never[]) => unknown);
  cf.associate("SERIES", SERIES as (...a: never[]) => unknown);
  cf.associate("MARK", MARK as (...a: never[]) => unknown);
  cf.associate("RISK", RISK as (...a: never[]) => unknown);
  cf.associate("POSITIONS", POSITIONS as (...a: never[]) => unknown);
  cf.associate("LIMITS", LIMITS as (...a: never[]) => unknown);
  cf.associate("STATUS", STATUS as (...a: never[]) => unknown);
}

registerAll();
