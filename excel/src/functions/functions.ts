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
  formatGreeksSpill,
  formatMarkStatusSpill,
  formatRfqSpill,
  formatSmileSpill,
  parsePair,
  parseTenor,
  shapeVanillaInstrument,
  type SpillMatrix,
} from "./shaping";
import { stageMark } from "./markStaging";
import { getConnection, getRegistry } from "./runtime";
import { smileFromWire, type WireObject } from "../contract/wsCodec";
import type { LiveTick } from "./streamRegistry";

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
 * @customfunction SURFACE
 * @param pair Currency pair, e.g. "EURUSD".
 * @param tenor Tenor, e.g. "1Y".
 * @returns A spill: a delta-pillar header row, a vol row, and a footer.
 */
export async function SURFACE(pair: string, tenor: string): Promise<SpillMatrix> {
  try {
    const ccy = parsePair(pair);
    const { expiryYears } = parseTenor(tenor);
    const reply: WireObject = await getConnection().getSmile(ccy, expiryYears, DEFAULT_CONVENTIONS);
    const smile = smileFromWire(reply);
    const points = smile.points.map((p) => ({ delta: p.delta, vol: p.vol }));
    return formatSmileSpill(
      points,
      smile.arbitrage.butterflyArbitrageFree && smile.arbitrage.calendarArbitrageFree,
      smile.conventions,
      undefined,
      smile.epochNanos,
    );
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
 * @param comment Optional contribution comment.
 * @returns A status spill; the write commits only on task-pane confirmation.
 */
export async function MARK(
  pair: string,
  tenor: string,
  pillar: string,
  vol: number,
  comment?: string,
): Promise<SpillMatrix> {
  try {
    const status = await stageMark(getConnection(), {
      pair,
      tenor,
      pillar,
      vol,
      comment: comment ?? "",
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
  cf.associate("RFQ", RFQ as (...a: never[]) => unknown);
  cf.associate("SUBSCRIBE", SUBSCRIBE as (...a: never[]) => unknown);
  cf.associate("MARK", MARK as (...a: never[]) => unknown);
}

registerAll();
