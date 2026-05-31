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
  formatCalibratedSmileSpill,
  formatGreeksSpill,
  formatMarkStatusSpill,
  formatRfqSpill,
  formatSeriesCell,
  formatSmileSpill,
  parseObservable,
  parsePair,
  parseSmileModel,
  parseTenor,
  shapeCalibration,
  shapeVanillaInstrument,
  type SpillMatrix,
} from "./shaping";
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
 * model the surface was last MARKED under; the model provenance the server stamps
 * on `arbitrage.note` as `model=<family>` is surfaced in the footer, and the
 * requested model is shown alongside so a mismatch (the surface was marked under a
 * different family) is visible — to actually re-calibrate a surface under a chosen
 * model, use `CELNET.MARKSURFACE` (the `mark_surface` `smile_model` path).
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
      providerNote: smile.arbitrage.note,
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
      providerNote: smile.arbitrage.note,
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
  cf.associate("SUBSCRIBE", SUBSCRIBE as (...a: never[]) => unknown);
  cf.associate("SERIES", SERIES as (...a: never[]) => unknown);
  cf.associate("MARK", MARK as (...a: never[]) => unknown);
}

registerAll();
