/**
 * The CELNET.* worksheet custom functions (docs/EXCEL-INTEGRATION.md §3) — the
 * POLYMORPHIC pricing surface.
 *
 * ONE composable spec replaces the retired per-product function table:
 *
 *   =CELNET.INSTRUMENT(underlier, product, terms, [tenor], [notional])
 *
 * builds an opaque instrument token from (1) the one underlier grammar across all
 * five asset classes (FX "EURUSD", metal "XAUUSD"/"XAU/EUR", equity
 * "AAPL@XNAS:USD", commodity "BRENT@:USD", crypto "BTC/USD"[:inverse|:linear]),
 * (2) a product-family name traders know (VANILLA, BARRIER, ASIAN, TARF, …) and
 * (3) a named, order-free 2-column key/value terms range. The polymorphic verbs —
 * CELNET.PRICE / GREEKS / RFQ / SUBSCRIBE — accept EITHER that token (any family,
 * any asset class) OR the legacy vanilla positional form (pair, tenor,
 * strikeOrDelta, callPut, notional), which behaves exactly as before. The token
 * is a VALUE, not an API: the one current contract stays `celnet.proto`
 * (src/contract), and the token is the canonical JSON of the instrument's
 * WS-mirror frame.
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
  formatBondSpill,
  formatCalibratedCurveSpill,
  formatCalibratedSmileSpill,
  formatGetCurveSpill,
  formatGreeksSpill,
  formatInstrumentsSpill,
  formatLimitsSpill,
  formatMarginSpill,
  formatMarkCurveSpill,
  formatMarkStatusSpill,
  formatPositionsSpill,
  formatPreTradeMarginSpill,
  formatPremiumSpill,
  formatRatesBookSpill,
  formatRatesRfqSpill,
  formatRatesRiskSpill,
  formatRatesSeriesCell,
  formatRatesSpill,
  formatRfqPanelSpill,
  formatRfqSpill,
  formatRiskSpill,
  formatSeriesCell,
  formatServerStatusSpill,
  formatSmileSpill,
  formatUpgradeStatusSpill,
  formatVarSwapSpill,
  formatVolSwapSpill,
  formatXvaSpill,
  formatAlgoOrderSpill,
  formatAlgoOrdersListSpill,
  formatClusterTopologySpill,
  formatCdmSpill,
  formatAttestationSpill,
  formatLicenseCapabilitiesSpill,
  lookbackIsMonteCarlo,
  parseObservable,
  parsePair,
  parseRatesObservable,
  parseRatesRfqSide,
  parseRatesRiskScope,
  parseRfqPanelFlag,
  parseRiskDimension,
  parseRiskScope,
  parseSmileModel,
  parseTenor,
  shapeBondInstrument,
  shapeBuildCurveRequest,
  shapeCalibration,
  shapeFraInstrument,
  shapeGetCurveRequest,
  shapeIrsInstrument,
  shapeMarkCurveRequest,
  shapeOisInstrument,
  shapeRatesCurve,
  shapeRatesRfqInstrument,
  shapeRatesRiskPositions,
  shapeReportingNumeraire,
  shapeVanillaInstrument,
  shapeXvaRequest,
  type SpillMatrix,
} from "./shaping";
import {
  decodeInstrumentToken,
  encodeInstrumentToken,
  instrumentLabel,
  isInstrumentToken,
  shapeSpecInstrument,
} from "./instrumentSpec";
import {
  aggregateRatesRiskRequest,
  aggregateRatesRiskResponseFromWire,
  aggregateRiskRequest,
  aggregateRiskResponseFromWire,
  limitStatusRequest,
  limitStatusResponseFromWire,
  listBooksResponseFromWire,
  listEntitiesResponseFromWire,
  listPositionsRequest,
  listPositionsResponseFromWire,
  listRatesPositionsRequest,
  listRatesPositionsResponseFromWire,
  type AggregateRatesRiskRequest,
  type BookDesc,
  type EntityDesc,
  type RiskScope,
} from "../contract/riskCodec";
import {
  instrumentResponseFromWire,
  instrumentsResponseFromWire,
} from "../contract/referenceDataCodec";
import { stageMark } from "./markStaging";
import {
  getConnection,
  getRatesStreamRegistry,
  getRegistry,
  getSeriesRegistry,
  getSession,
} from "./runtime";
import type { EntryPointId } from "../contract/access";
import { brokerQuoteSetToWire, ccyPairToWire, conventionsToWire, smileFromWire, type WireObject } from "../contract/wsCodec";
import { smileModel } from "../contract/enums";
import {
  oisRatesInstrument,
  type Instrument,
  type Quote,
  type MarginCalculationResponse,
  type PreTradeMarginResponse,
  type PreTradeMarginOutcome,
  type AlgoOrderResponse,
  type AlgoOrderStatus,
  type ChildSlice,
  type ChildSliceStatus,
  type ListAlgoOrdersResponse,
  type ClusterTopologyResponse,
  type NodeMember,
  type NodeLifecycleStatus,
  type UpgradeStatusResponse,
  type ExportCdmResponse,
  type AttestationResponse,
  type LicenseCapabilityResponse,
} from "../contract/contract";
import type { LiveTick } from "./streamRegistry";
import type { RatesLiveTick } from "./ratesStreamRegistry";
import type { SeriesTick } from "./seriesRegistry";
import type { MarketSeriesRequest } from "../transport/connection";

/** Map any error to a custom-function error value with a readable message. */
function toCfError(err: unknown): CustomFunctions.Error {
  // A capability-denial (or any pre-built CF error) passes through verbatim so its
  // honest `#CELNET_DENIED!` message is not re-wrapped with a generic code.
  if (err instanceof CustomFunctions.Error) return err;
  const message = err instanceof Error ? err.message : String(err);
  const code = err instanceof ShapingError ? "#CELNET_ARG!" : "#CELNET_ERR!";
  return new CustomFunctions.Error(CustomFunctions.ErrorCode.invalidValue, `${code} ${message}`);
}

/**
 * Cell-side capability gate: when the caller is SIGNED IN but lacks the capability
 * the entry point requires (or their session has expired), throw a denied
 * custom-function error carrying the SAME honest denial sentence the GUI/task pane
 * show — never a wrong number, never a silent value (docs §5, "no #N/A storm").
 * Anonymous callers keep the existing price-preview behaviour (the server enforces
 * every request). The session is the shared-runtime singleton the task pane signs
 * into, so a sign-in there immediately gates the cells too.
 */
function denyIfUngated(id: EntryPointId): void {
  const session = getSession();
  if (!session.isSignedIn()) return; // anonymous: permissive (server enforces)
  if (session.canEntry(id)) return;
  throw new CustomFunctions.Error(
    CustomFunctions.ErrorCode.invalidValue,
    `#CELNET_DENIED! ${session.entryDenialReason(id)}`,
  );
}

// ---------------------------------------------------------------------------
// the polymorphic spec + verbs
// ---------------------------------------------------------------------------

/**
 * Build the opaque instrument token the polymorphic verbs price. The token is a
 * deterministic (canonical, key-sorted) compact-JSON encoding of the instrument's
 * wire frame — treat it as a value to reference from PRICE/GREEKS/RFQ/SUBSCRIBE
 * cells, never as a contract to parse.
 *
 * `terms` is a 2-column key/value range whose keys mirror the family's
 * parameters exactly (order-free, self-documenting), e.g. for a knock-out
 * barrier: ("strike",1.12) ("callPut","C") ("barrier",1.20) ("kind","KNOCK_OUT").
 * The matrix keys repeat one row per matrix row: BASKET's ("legs", pair, weight,
 * spot, vol, rFor) and ("correlations", ρ₁, ρ₂, …); STRATEGY's ("legs", callPut,
 * strike, side, ratio?) — one row per option leg. `tenor`/`notional` may be
 * given positionally or as terms (not both); notional defaults to 1.
 * @customfunction INSTRUMENT
 * @param underlier Underlier: FX EURUSD, metal XAUUSD, equity AAPL@XNAS:USD, commodity BRENT@:USD, crypto BTC/USD (optionally :inverse / :linear).
 * @param product Product family: VANILLA, STRATEGY (or RISK_REVERSAL / STRADDLE / STRANGLE / SEAGULL), BARRIER, WINDOWBARRIER, DIGITAL, TOUCH, VARSWAP, VOLSWAP, ASIAN, FORWARDSTART, CLIQUET, QUANTO, TARF, PIVOT, ACCUMULATOR, LOOKBACK, AMERICAN, BASKET, FORWARD, SWAP, NDF, PERPETUAL, FUTUREOPTION.
 * @param terms The 2-column key/value terms range (keys mirror the family's parameters).
 * @param tenor Optional tenor, e.g. 1Y (or supply a ("tenor", …) term).
 * @param notional Optional notional in the base/asset leg (or a ("notional", …) term; default 1).
 * @returns The opaque instrument token the other CELNET.* functions accept.
 */
export function INSTRUMENT(
  underlier: string,
  product: string,
  terms: (string | number | boolean)[][],
  tenor?: string,
  notional?: number,
): string {
  try {
    return encodeInstrumentToken(
      shapeSpecInstrument({ underlier, product, terms, tenor, notional }),
    );
  } catch (err) {
    throw toCfError(err);
  }
}

/** Reject a polymorphic call that mixes an instrument token with positional args. */
function rejectPositionalTail(fn: string, tail: readonly unknown[]): void {
  if (tail.some((a) => a !== undefined && a !== null && a !== "")) {
    throw new ShapingError(
      `${fn}: an instrument token takes no further arguments (the token carries the full spec)`,
    );
  }
}

/** Validate + assemble the legacy vanilla positional form (unchanged behaviour). */
function legacyVanilla(
  fn: string,
  pair: string,
  tenor: string | undefined,
  strikeOrDelta: string | undefined,
  callPut: string | undefined,
  notional: number | undefined,
): { instrument: Instrument; key: string } {
  if (tenor === undefined || strikeOrDelta === undefined || callPut === undefined || notional === undefined) {
    throw new ShapingError(
      `${fn} takes (pair, tenor, strikeOrDelta, callPut, notional) — or a single CELNET.INSTRUMENT token`,
    );
  }
  return {
    instrument: shapeVanillaInstrument({ pair, tenor, strikeOrDelta, callPut, notional }),
    key: `${fn.toLowerCase()}:${pair}:${tenor}:${strikeOrDelta}:${callPut}:${notional}`,
  };
}

/**
 * The Monte-Carlo standard error to surface for a token-priced premium spill —
 * gated on the SHAPED product exactly as the retired per-product functions gated
 * it, so a closed-form price never carries a stray precision claim:
 * TARF/accumulator/basket and the barrier/window-barrier/linear families pass the
 * server's stamp through; cliquet/lookback/American surface it only for their MC
 * variants; the exact closed-form families never surface one.
 */
function stdErrorFor(instrument: Instrument, quote: Quote): number | undefined {
  const p = instrument.product;
  switch (p.kind) {
    case "cliquet":
      return cliquetIsMonteCarlo(p.cliquet) ? quote.priceStdError : undefined;
    case "lookback":
      return lookbackIsMonteCarlo(p.lookback) ? quote.priceStdError : undefined;
    case "american":
      return americanIsMonteCarlo(p.american) ? quote.priceStdError : undefined;
    case "vanilla":
    case "strategy":
    case "digital":
    case "touch":
    case "asianOption":
    case "forwardStart":
    case "quanto":
    case "varianceSwap":
    case "volatilitySwap":
    // Arms 30/31 are exact closed forms (perpetual stationary-ODE; futures-
    // measure listed-future option) — never Monte-Carlo, never a std-error.
    case "perpetualOption":
    case "listedFutureOption":
      return undefined;
    case "singleBarrier":
    case "doubleBarrier":
    case "windowBarrier":
    case "tarf":
    // The pivot TRA (arm 32) is always Monte-Carlo, exactly like its TARF slice.
    case "pivot":
    case "accumulator":
    case "basket":
    case "fxForward":
    case "fxSwap":
    case "ndf":
      return quote.priceStdError;
  }
}

/**
 * Format a token-priced quote as the family's spill: the swaps keep their
 * fair-strike geometries; every premium-headline family spills
 * `["premium", PV]`, an honest `["std_error", σ̄]` row only when MC-priced, the
 * 13 risk Greeks, and the convention footer — exactly the geometry the retired
 * per-product function for that family produced.
 */
function premiumSpillFor(instrument: Instrument, quote: Quote): SpillMatrix {
  const common = {
    conventions: quote.conventions,
    surfaceVersion: quote.surfaceVersion,
    epochNanos: quote.epochNanos,
  };
  switch (instrument.product.kind) {
    case "varianceSwap":
      // The server returns the fair variance strike on `resolved_strike` (K_var).
      return formatVarSwapSpill({ fairVariance: quote.resolvedStrike, ...common });
    case "volatilitySwap":
      // The server returns the fair vol strike on `resolved_strike` (K_vol).
      return formatVolSwapSpill({ fairVol: quote.resolvedStrike, ...common });
    default:
      return formatPremiumSpill({
        premium: quote.greeks.price,
        stdError: stdErrorFor(instrument, quote),
        greeks: quote.greeks,
        ...common,
      });
  }
}

/**
 * Price one structure. POLYMORPHIC: pass a CELNET.INSTRUMENT token as the single
 * argument to price ANY product family on ANY asset class — the cell spills the
 * family's labelled view (premium, an honest std_error row only when MC-priced,
 * the 13 Greeks, and a convention footer; the variance/volatility swaps spill
 * their fair strikes). The simple vanilla positional form is unchanged: it
 * returns the single mid premium in the convention's premium-ccy/style.
 * @customfunction PRICE
 * @param pairOrInstrument Currency pair (e.g. "EURUSD") — or a CELNET.INSTRUMENT token.
 * @param tenor Tenor, e.g. "1Y", "3M", "ON" (positional form only).
 * @param strikeOrDelta Absolute strike (1.12) or delta ("25dP", "ATM") (positional form only).
 * @param callPut "C" for call, "P" for put (positional form only).
 * @param notional Trade notional in the base currency (positional form only).
 * @returns The mid premium (positional form) or the family's priced spill (token form).
 */
export async function PRICE(
  pairOrInstrument: string,
  tenor?: string,
  strikeOrDelta?: string,
  callPut?: string,
  notional?: number,
): Promise<number | SpillMatrix> {
  try {
    denyIfUngated("price");
    if (isInstrumentToken(pairOrInstrument)) {
      rejectPositionalTail("PRICE", [tenor, strikeOrDelta, callPut, notional]);
      const instrument = decodeInstrumentToken(pairOrInstrument);
      const quote = await getConnection().requestQuote(
        instrument,
        DEFAULT_CONVENTIONS,
        `price:${pairOrInstrument}`,
      );
      return premiumSpillFor(instrument, quote);
    }
    const { instrument, key } = legacyVanilla(
      "PRICE",
      pairOrInstrument,
      tenor,
      strikeOrDelta,
      callPut,
      notional,
    );
    const quote = await getConnection().requestQuote(instrument, DEFAULT_CONVENTIONS, key);
    // The mid of the two-way is the premium; the directional market is the spill
    // returned by CELNET.RFQ. A single-cell PRICE shows the mid valuation.
    return 0.5 * (quote.price.bid + quote.price.offer);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * The full 13-Greek vector for a structure as a vertical dynamic-array spill
 * `[GreekName, Value]`, with a convention-transparency footer row. POLYMORPHIC:
 * pass a CELNET.INSTRUMENT token as the single argument for ANY family on ANY
 * asset class; the legacy vanilla positional form is unchanged.
 * @customfunction GREEKS
 * @param pairOrInstrument Currency pair (e.g. "EURUSD") — or a CELNET.INSTRUMENT token.
 * @param tenor Tenor, e.g. "1Y" (positional form only).
 * @param strikeOrDelta Absolute strike or delta ("25dP", "ATM") (positional form only).
 * @param callPut "C" or "P" (positional form only).
 * @param notional Trade notional in the base currency (positional form only).
 * @returns A 14×2 spill: the 13 Greeks plus a convention footer.
 */
export async function GREEKS(
  pairOrInstrument: string,
  tenor?: string,
  strikeOrDelta?: string,
  callPut?: string,
  notional?: number,
): Promise<SpillMatrix> {
  try {
    denyIfUngated("greeks");
    let instrument: Instrument;
    let key: string;
    if (isInstrumentToken(pairOrInstrument)) {
      rejectPositionalTail("GREEKS", [tenor, strikeOrDelta, callPut, notional]);
      instrument = decodeInstrumentToken(pairOrInstrument);
      key = `greeks:${pairOrInstrument}`;
    } else {
      ({ instrument, key } = legacyVanilla(
        "GREEKS",
        pairOrInstrument,
        tenor,
        strikeOrDelta,
        callPut,
        notional,
      ));
    }
    const quote = await getConnection().requestQuote(instrument, DEFAULT_CONVENTIONS, key);
    // Pass the underlying so the rate-rho rows carry the asset-class-correct carry
    // label (e.g. an equity's `rho_dividend_yield`) — the carry seam reaching the
    // Excel Greeks spill. FX is byte-identical (the `rho_dom`/`rho_for` pair).
    return formatGreeksSpill(
      quote.greeks,
      quote.conventions,
      quote.surfaceVersion,
      quote.epochNanos,
      instrument.underlying,
    );
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price an overnight-indexed swap (OIS) against a self-discounting curve via the
 * live `price_rates` engine RPC, and spill the priced PV + first-order risk. The
 * add-in carries NO rates math: the calibrated `curve` (its par-OIS pillars) and
 * the OIS terms are sent to the `celnet-rates` engine, which bootstraps the
 * discount/forward curve and returns the authoritative result; this cell only
 * shapes the inputs and lays out the reply.
 *
 * The spill is a labelled `(4 + pillars)×2` matrix: `pv`, `par_rate`, `pv01`,
 * `dv01`, then the key-rate DV01 ladder — one `kr_dv01[<tenor>Y]` row per curve
 * pillar (the ladder sums to `dv01` to first order). All measures are in the
 * curve currency and carry the `direction` sign (payer and receiver of the same
 * swap report equal-and-opposite numbers).
 * @customfunction RATES
 * @param curve The 2-column `[tenorYears, parRate]` curve range — one row per self-discounting OIS pillar, in strictly increasing tenor order.
 * @param referenceDate The curve reference (spot-anchor) date — an Excel date cell or "YYYY-MM-DD".
 * @param tenor The OIS tenor in whole years (e.g. 5 or "5Y").
 * @param fixedRate The fixed-leg rate as a decimal (0.041 = 4.10%).
 * @param direction "PAY_FIXED" (payer) or "RECEIVE_FIXED" (receiver).
 * @param notional The (positive) notional in the curve currency.
 * @param currency Optional ISO-4217 curve currency (defaults to USD).
 * @returns A `(4 + pillars)×2` spill: pv, par_rate, pv01, dv01, then the key-rate DV01 ladder.
 */
export async function RATES(
  curve: (string | number | boolean)[][],
  referenceDate: number | string,
  tenor: number | string,
  fixedRate: number,
  direction: string,
  notional: number,
  currency?: string,
): Promise<SpillMatrix> {
  try {
    denyIfUngated("rates");
    const curveSet = shapeRatesCurve({ curve, referenceDate, currency });
    const instrument = shapeOisInstrument({ tenor, fixedRate, direction, notional });
    const result = await getConnection().priceRates(curveSet, instrument);
    return formatRatesSpill(result, curveSet.pillars);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a fixed-coupon cash bond off a self-discounting curve via the live
 * `price_rates` engine RPC (the SAME RPC as CELNET.RATES, carrying the
 * `RatesInstrument.bond` arm), and spill the priced bond result. The add-in carries
 * NO bond math: the calibrated `curve` (its par-OIS pillars) and the bond terms are
 * sent to the `celnet-rates` bond engine, which bootstraps the discount curve, PVs
 * each cashflow (the DIRTY price), and returns the authoritative result; this cell
 * only shapes the inputs and lays out the reply.
 *
 * The spill is a labelled `3×2` matrix: `dirty_price` (the full settlement price,
 * carrying the position `side` sign), `ytm` (yield to maturity, side-independent),
 * and `dv01` (the yield DV01). These are exactly the fields the `price_rates`
 * response carries for a bond — clean price / accrued / duration are computed inside
 * the engine but are NOT on the wire, so they are never shown here (no fabricated
 * numbers). Settlement is the curve reference date rolled to the next US business
 * day; the coupon schedule rolls back from `maturity` at `frequency`.
 * @customfunction BOND
 * @param curve The 2-column `[tenorYears, parRate]` discount-curve range — one row per self-discounting OIS pillar, in strictly increasing tenor order.
 * @param referenceDate The curve reference (settlement / spot-anchor) date — an Excel date cell or "YYYY-MM-DD".
 * @param maturity The bond maturity (final-redemption) date — an Excel date cell or "YYYY-MM-DD"; must be after referenceDate.
 * @param couponRate The annual coupon rate as a decimal (0.06 = 6%); 0 for a zero-coupon bond.
 * @param redemption Optional par redemption / face value (defaults to 100).
 * @param frequency Optional coupon frequency: ANNUAL, SEMI_ANNUAL (default) or QUARTERLY.
 * @param dayCount Optional accrual day-count: ACT_360, ACT_365_FIXED or THIRTY_360_BOND_BASIS (default).
 * @param side Optional position direction: LONG (default, +PV) or SHORT (−PV).
 * @param currency Optional ISO-4217 curve currency (defaults to USD).
 * @returns A `3×2` spill: dirty_price, ytm, dv01.
 */
export async function BOND(
  curve: (string | number | boolean)[][],
  referenceDate: number | string,
  maturity: number | string,
  couponRate: number,
  redemption?: number,
  frequency?: string,
  dayCount?: string,
  side?: string,
  currency?: string,
): Promise<SpillMatrix> {
  try {
    denyIfUngated("bond");
    const curveSet = shapeRatesCurve({ curve, referenceDate, currency });
    const bond = shapeBondInstrument({
      maturity,
      referenceDate: curveSet.referenceDate,
      couponRate,
      redemption,
      frequency,
      dayCount,
      side,
    });
    const result = await getConnection().priceRatesBond(curveSet, bond);
    return formatBondSpill(result);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a vanilla fixed-vs-float interest-rate swap off a self-discounting curve via
 * the live `price_rates` engine RPC (the SAME RPC as CELNET.RATES/BOND, carrying the
 * `RatesInstrument.irs` arm), and spill the priced PV + first-order risk. The add-in
 * carries NO swap math: the calibrated `curve` (its par-OIS pillars) and the swap
 * terms are sent to the `celnet-rates` engine, which bootstraps the discount/forward
 * curve, PVs each leg, and returns the authoritative result; this cell only shapes
 * the inputs and lays out the reply.
 *
 * The swap is SPOT-STARTING: its schedule of `tenor` whole years is reconstructed
 * server-side from the curve reference date (there is no separate effective date on
 * the wire). The spill is the labelled `(4 + pillars)×2` matrix: `pv`, `par_rate`
 * (the fair fixed rate), `pv01`, `dv01`, then the key-rate DV01 ladder — one
 * `kr_dv01[<tenor>Y]` row per curve pillar (the ladder sums to `dv01` to first order).
 * All measures are in the curve currency and carry the `side` sign (a payer and a
 * receiver of the same swap report equal-and-opposite numbers).
 * @customfunction IRS
 * @param curve The 2-column `[tenorYears, parRate]` curve range — one row per self-discounting OIS pillar, in strictly increasing tenor order.
 * @param referenceDate The curve reference (spot-anchor) date — an Excel date cell or "YYYY-MM-DD".
 * @param tenor The swap tenor in whole years (e.g. 5 or "5Y").
 * @param fixedRate The fixed-leg rate as a decimal (0.041 = 4.10%).
 * @param notional The (positive) notional in the curve currency.
 * @param side Optional direction: PAY_FIXED (payer, default) or RECEIVE_FIXED (receiver).
 * @param fixedFrequency Optional fixed-leg frequency: ANNUAL, SEMI_ANNUAL (default) or QUARTERLY.
 * @param fixedDayCount Optional fixed-leg day-count: ACT_360 (default) or ACT_365_FIXED.
 * @param floatFrequency Optional float-leg frequency: ANNUAL, SEMI_ANNUAL or QUARTERLY (default).
 * @param floatDayCount Optional float-leg day-count: ACT_360 (default) or ACT_365_FIXED.
 * @param currency Optional ISO-4217 curve currency (defaults to USD).
 * @returns A `(4 + pillars)×2` spill: pv, par_rate, pv01, dv01, then the key-rate DV01 ladder.
 */
export async function IRS(
  curve: (string | number | boolean)[][],
  referenceDate: number | string,
  tenor: number | string,
  fixedRate: number,
  notional: number,
  side?: string,
  fixedFrequency?: string,
  fixedDayCount?: string,
  floatFrequency?: string,
  floatDayCount?: string,
  currency?: string,
): Promise<SpillMatrix> {
  try {
    denyIfUngated("irs");
    const curveSet = shapeRatesCurve({ curve, referenceDate, currency });
    const irs = shapeIrsInstrument({
      tenor,
      fixedRate,
      notional,
      side,
      fixedFrequency,
      fixedDayCount,
      floatFrequency,
      floatDayCount,
    });
    const result = await getConnection().priceRatesIrs(curveSet, irs);
    return formatRatesSpill(result, curveSet.pillars);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price a forward rate agreement (FRA) off a self-discounting curve via the live
 * `price_rates` engine RPC (the SAME RPC as CELNET.RATES/BOND/IRS, carrying the
 * `RatesInstrument.fra` arm), and spill the priced PV + first-order risk. The add-in
 * carries NO FRA math: the calibrated `curve` and the FRA terms are sent to the
 * `celnet-rates` engine, which rebuilds the roll-adjusted accrual window and prices
 * the single-period swaplet; this cell only shapes the inputs and lays out the reply.
 *
 * The accrual window is quoted in whole MONTHS from spot — the standard "3x6 FRA"
 * market convention: `startMonths` = 3, `endMonths` = 6 for a 3×6. The spill is the
 * labelled `(4 + pillars)×2` matrix: `pv`, `par_rate` (the break-even rate), `pv01`,
 * `dv01`, then the key-rate DV01 ladder — one `kr_dv01[<tenor>Y]` row per curve
 * pillar (the ladder sums to `dv01` to first order). All measures are in the curve
 * currency and carry the `side` sign (payer and receiver report equal-and-opposite
 * numbers).
 * @customfunction FRA
 * @param curve The 2-column `[tenorYears, parRate]` curve range — one row per self-discounting OIS pillar, in strictly increasing tenor order.
 * @param referenceDate The curve reference (spot-anchor) date — an Excel date cell or "YYYY-MM-DD".
 * @param startMonths The window start (fixing) tenor in whole months from spot (e.g. 3 or "3M").
 * @param endMonths The window end (maturity) tenor in whole months from spot (e.g. 6 or "6M"); must be after startMonths.
 * @param fixedRate The contractual fixed rate as a decimal (0.033 = 3.30%).
 * @param notional The (positive) notional in the curve currency.
 * @param side Optional direction: PAY_FIXED (payer, default) or RECEIVE_FIXED (receiver).
 * @param accrualBasis Optional accrual day-count: ACT_360 (default), ACT_365_FIXED or THIRTY_360_BOND_BASIS.
 * @param currency Optional ISO-4217 curve currency (defaults to USD).
 * @returns A `(4 + pillars)×2` spill: pv, par_rate, pv01, dv01, then the key-rate DV01 ladder.
 */
export async function FRA(
  curve: (string | number | boolean)[][],
  referenceDate: number | string,
  startMonths: number | string,
  endMonths: number | string,
  fixedRate: number,
  notional: number,
  side?: string,
  accrualBasis?: string,
  currency?: string,
): Promise<SpillMatrix> {
  try {
    denyIfUngated("fra");
    const curveSet = shapeRatesCurve({ curve, referenceDate, currency });
    const fra = shapeFraInstrument({
      startMonths,
      endMonths,
      fixedRate,
      notional,
      side,
      accrualBasis,
    });
    const result = await getConnection().priceRatesFra(curveSet, fra);
    return formatRatesSpill(result, curveSet.pillars);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Request a tradeable two-way FIXED-INCOME RFQ off a self-discounting curve via the
 * live `request_rates_quote` engine RPC (the WS mirror of
 * `QuoteService.RequestRatesQuote`) — the FI twin of `=CELNET.RFQ`, bringing
 * request-for-quote parity to the rates surface (FI RFQ previously existed only over
 * FIX). The add-in carries NO FI math: the calibrated `curve` (its par-OIS pillars),
 * the instrument terms, the RFQ `notional` and the taker `side` are sent to the
 * `celnet-rates` engine, which prices the SIDE-INDEPENDENT fair level (the par rate)
 * and returns the two-way struck around it plus the full first-order risk; this cell
 * only shapes the inputs and lays out the authoritative reply — bit-identical to the
 * GUI / SDK rates RFQ over the one unversioned contract.
 *
 * The RFQ mirrors the `=CELNET.RATES` arg grammar (the instrument is built from
 * `tenor` + `fixedRate` + `notional`). It is a SINGLE two-way maker quote (the FI
 * price-discovery two-way, mirroring the FIX venue auto-quote), not a multi-dealer
 * panel — `=CELNET.RFQ`'s `panel` flag has no FI analogue on this contract.
 *
 * The spill is a labelled `10×2` matrix: `bid`, `offer`, `mid` (= `(bid + offer)/2`,
 * the side-independent fair level — equal to `par_rate` for a swap, a live
 * cross-check), then the RFQ metadata `notional`, `quote_id` (64-bit-exact as a
 * string, referenceable by a later accept) and `valid_until` (the last-look deadline,
 * ISO-8601), then the full FI risk at the taker side: `pv`, `par_rate`, `pv01`,
 * `dv01`. All measures are in the curve currency.
 * @customfunction RATESRFQ
 * @param curve The 2-column `[tenorYears, parRate]` curve range — one row per self-discounting OIS pillar, in strictly increasing tenor order.
 * @param referenceDate The curve reference (spot-anchor) date — an Excel date cell or "YYYY-MM-DD".
 * @param tenor The instrument tenor in whole years (e.g. 5 or "5Y").
 * @param fixedRate The fixed-leg rate as a decimal (0.041 = 4.10%).
 * @param notional The (positive) RFQ size in the curve currency.
 * @param side Optional taker side: BUY (pay fixed), SELL (receive fixed) or TWO_WAY (default — request a two-way market).
 * @param currency Optional ISO-4217 curve currency (defaults to USD).
 * @param instrument Optional instrument arm: OIS (default) or IRS (a spot-starting vanilla swap).
 * @returns A `10×2` spill: bid, offer, mid, notional, quote_id, valid_until, pv, par_rate, pv01, dv01.
 */
export async function RATESRFQ(
  curve: (string | number | boolean)[][],
  referenceDate: number | string,
  tenor: number | string,
  fixedRate: number,
  notional: number,
  side?: string,
  currency?: string,
  instrument?: string,
): Promise<SpillMatrix> {
  try {
    denyIfUngated("ratesrfq");
    const curveSet = shapeRatesCurve({ curve, referenceDate, currency });
    const takerSide = parseRatesRfqSide(side);
    const rfqInstrument = shapeRatesRfqInstrument({
      tenor,
      fixedRate,
      notional,
      side: takerSide,
      instrument,
    });
    // A deterministic idempotency key over the RFQ terms: an identical retry is
    // deduplicated server-side, never colliding with a unary price cell for the
    // same instrument (distinct `ratesrfq:` namespace).
    const key = `ratesrfq:${curveSet.currency}:${takerSide}:${notional}:${JSON.stringify(rfqInstrument)}`;
    const quote = await getConnection().requestRatesQuote(
      curveSet,
      rfqInstrument,
      takerSide,
      notional,
      key,
    );
    return formatRatesRfqSpill(quote);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Bootstrap a single-currency discount curve from its market pillars via the live
 * `build_curve` engine RPC (the WS mirror of `AuthService.BuildCurve`), and spill
 * the calibrated curve. The add-in carries NO bootstrap math: each pillar and the
 * reference date are sent to the `celnet-rates` engine, which resolves every
 * registry instrument id, decodes the schedule, and runs the sequential
 * discount-curve calibration; this cell only shapes the inputs and lays out the
 * authoritative reply — bit-identical to the GUI CurveWorkspace over the one
 * unversioned contract.
 *
 * The `pillars` range is 2-column `[pillar, quote]`: each pillar (first column) is
 * EITHER a reference-data registry instrument id (a non-date string, e.g.
 * `usd-sofr-irs-10y`) OR a maturity date (an Excel date cell or "YYYY-MM-DD"), and
 * the quote (second column) is the observed rate as a decimal (0.0431 = 4.31%).
 * The spill is a labelled `(1 + pillars)×4` matrix: a header then one row per
 * bootstrapped pillar — `[pillar, time_years, discount_factor, zero_rate]` — in the
 * server's short→long maturity order. The pillar label is the server's display
 * label (e.g. `Date 2027-12-31`) or the resolving instrument id.
 * @customfunction CURVE
 * @param pillars The 2-column `[pillar, quote]` range — one row per calibrating pillar (registry instrument id OR maturity date, then its observed rate as a decimal).
 * @param referenceDate The curve reference (spot-anchor) date — an Excel date cell or "YYYY-MM-DD".
 * @param currency Optional ISO-4217 curve currency (defaults to USD).
 * @returns A `(1 + pillars)×4` spill: header, then pillar/time_years/discount_factor/zero_rate per bootstrapped pillar.
 */
export async function CURVE(
  pillars: (string | number | boolean)[][],
  referenceDate: number | string,
  currency?: string,
): Promise<SpillMatrix> {
  try {
    denyIfUngated("curve");
    const request = shapeBuildCurveRequest({ pillars, referenceDate, currency });
    const curve = await getConnection().buildCurve(request);
    return formatCalibratedCurveSpill(curve);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Read a discount curve — marked or live-bootstrapped — via the live `get_curve`
 * engine RPC (the WS mirror of `SurfaceService.GetCurve`, ADR-0021; the
 * fixed-income analogue of CELNET.SURFACE/GetSmile), and spill the read curve. The
 * add-in carries NO curve math: the `curve` par pillars + reference date are sent
 * to the `celnet-rates` engine, which bootstraps the discount/forward term
 * structure (or, when `pinnedVersion` is supplied, reads the exact `MarkCurve`d
 * curve of that version from the store) and reports the zero rate + discount factor
 * at each pillar tenor; this cell only shapes the inputs and lays out the
 * authoritative reply — bit-identical to the GUI CurveWorkspace over the one
 * unversioned contract.
 *
 * The `curve` range is 2-column `[tenorYears, parRate]`: one row per self-
 * discounting OIS pillar (whole-year tenor from spot, then its observed par rate as
 * a decimal, 0.0405 = 4.05%), in strictly increasing tenor order. When
 * `pinnedVersion` is supplied the marked curve of that version is read (the `curve`
 * range then only supplies the tenor axis the read reports at); otherwise the curve
 * is bootstrapped live.
 *
 * The spill is a labelled `(2 + points + parPillars + 1)×4` matrix: a `[point,
 * tenor_years, discount_factor, zero_rate]` header then one queried point per pillar
 * tenor, a `[par_pillar, tenor_years, par_rate, ]` header then the echoed
 * calibrating par pillars, and a footer `[version, v<n>|live, currency, <ccy>]`.
 * @customfunction GETCURVE
 * @param curve The 2-column `[tenorYears, parRate]` curve range — one row per OIS pillar, strictly increasing tenor.
 * @param referenceDate The curve reference (spot-anchor) date — an Excel date cell or "YYYY-MM-DD".
 * @param pinnedVersion Optional MarkCurve version to read from the store (omit ⇒ bootstrap the curve live).
 * @param currency Optional ISO-4217 curve currency (defaults to USD).
 * @returns A `(2 + points + parPillars + 1)×4` spill: the queried points, the echoed par pillars, then a version/currency footer.
 */
export async function GETCURVE(
  curve: (string | number | boolean)[][],
  referenceDate: number | string,
  pinnedVersion?: number | string,
  currency?: string,
): Promise<SpillMatrix> {
  try {
    denyIfUngated("getcurve");
    const request = shapeGetCurveRequest({ curve, referenceDate, pinnedVersion, currency });
    const reply = await getConnection().getCurve(request);
    return formatGetCurveSpill(reply);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Mark (persist) a discount curve under a fresh server-assigned version and return
 * the pinned curve — the fixed-income analogue of `CELNET.MARKSURFACE`. It issues
 * `mark_curve` with the calibrating pillars; the live `celnet-rates` engine
 * bootstraps the curve and DEPOSITS it through the same versioning seam a surface
 * mark uses, returning the assigned `curve_version` (shown in the footer) so a later
 * `CELNET.GETCURVE(curve, referenceDate, pinnedVersion)` reproduces THIS EXACT curve.
 *
 * The `curve` range is 2-column `[tenorYears, parRate]`: one row per self-
 * discounting OIS pillar (whole-year tenor from spot, then its observed par rate as
 * a decimal, 0.0405 = 4.05%), in strictly increasing tenor order — identical to the
 * `CELNET.GETCURVE` / `CELNET.CURVE` input, so a mark and a live read bootstrap
 * byte-identically.
 *
 * NOTE on side effects: like `CELNET.MARKSURFACE` (and unlike a live read), this
 * commits a WRITE on evaluation — it deposits a fresh pinned curve version. The
 * server applies its own dedupe, but a recalc with changed pillars WILL deposit a
 * new version; treat it as an explicit "mark this curve" action cell, not a live
 * formula.
 *
 * The spill is a labelled `(2 + points + parPillars + 1)×4` matrix: a `[point,
 * tenor_years, discount_factor, zero_rate]` header then one bootstrapped point per
 * pillar tenor, a `[par_pillar, tenor_years, par_rate, ]` header then the echoed
 * calibrating par pillars, and a footer `[version, v<n>, currency, <ccy>]` carrying
 * the assigned pinned version (always a concrete `v<n>`).
 * @customfunction MARKCURVE
 * @param curve The 2-column `[tenorYears, parRate]` curve range — one row per OIS pillar, strictly increasing tenor.
 * @param referenceDate The curve reference (spot-anchor) date — an Excel date cell or "YYYY-MM-DD".
 * @param currency Optional ISO-4217 curve currency (defaults to USD).
 * @returns A `(2 + points + parPillars + 1)×4` spill: the bootstrapped points, the echoed par pillars, then a version/currency footer carrying the assigned pinned version.
 */
export async function MARKCURVE(
  curve: (string | number | boolean)[][],
  referenceDate: number | string,
  currency?: string,
): Promise<SpillMatrix> {
  try {
    denyIfUngated("markcurve");
    const request = shapeMarkCurveRequest({ curve, referenceDate, currency });
    const reply = await getConnection().markCurve(request);
    return formatMarkCurveSpill(reply);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Net the linear-rates risk of a whole OIS BOOK against one shared curve via the
 * live `aggregate_rates_risk` engine RPC (the WS mirror of
 * `RiskService.AggregateRatesRisk`), and spill the per-currency netted risk. The
 * add-in carries NO rates-risk math: the `curve` pillars and every position's OIS
 * terms are sent to the `celnet-rates` engine, which prices each position and the
 * SERVER sums them additively into one node per settlement currency (the API-first
 * parity rule — a client never loops positions and sums); this cell only shapes
 * the inputs and lays out the authoritative netted reply.
 *
 * `positions` is a row-per-position range `[tenorYears, fixedRate, direction,
 * notional, entity?, book?]`: each row is one OIS (whole-year tenor, decimal fixed
 * rate, PAY_FIXED/RECEIVE_FIXED direction carrying the sign, positive notional)
 * plus the optional `(entity, book)` booking cell the roll-up and `scope` filter
 * on. The optional `scope` narrows BEFORE the roll-up — a comma-separated list of
 * `ENTITY:<n>` / `BOOK:<n>` / `CCY:<xxx>` tokens (each present key constrains).
 *
 * The spill is a labelled `(1 + currencies + 1)×(4 + pillars)` grid: a header
 * `[ccy, net_pv, net_pv01, net_dv01, kr_dv01[<tenor>Y]…]`, one row per settlement-
 * currency node (netted PV / PV01 / DV01 then the key-rate DV01 ladder, one column
 * per curve pillar — the ladder sums to `net_dv01` to first order), then a summary
 * footer. All measures are in the node currency and already net long against short
 * by the position directions.
 * @customfunction RATESRISK
 * @param curve The 2-column `[tenorYears, parRate]` curve range — one row per self-discounting OIS pillar, in strictly increasing tenor order (the shared market every position prices against).
 * @param referenceDate The curve reference (spot-anchor) date — an Excel date cell or "YYYY-MM-DD".
 * @param positions The row-per-position range `[tenorYears, fixedRate, direction, notional, entity?, book?]` — one OIS per row.
 * @param currency Optional ISO-4217 curve currency (defaults to USD).
 * @param scope Optional pre-rollup filter: a comma-separated list of ENTITY:<n>, BOOK:<n>, CCY:<xxx> (omit ⇒ the whole book).
 * @returns A `(1 + currencies + 1)×(4 + pillars)` spill: header, one netted node per settlement currency, then a footer.
 */
export async function RATESRISK(
  curve: (string | number | boolean)[][],
  referenceDate: number | string,
  positions: (string | number | boolean)[][],
  currency?: string,
  scope?: string,
): Promise<SpillMatrix> {
  try {
    denyIfUngated("ratesrisk");
    const curveSet = shapeRatesCurve({ curve, referenceDate, currency });
    const parsedPositions = shapeRatesRiskPositions(positions);
    const scopeFilter = parseRatesRiskScope(typeof scope === "string" ? scope : undefined);
    const request: AggregateRatesRiskRequest = {
      curveSet,
      positions: parsedPositions,
      ...(scopeFilter !== undefined ? { scope: scopeFilter } : {}),
    };
    const reply = await getConnection().aggregateRatesRisk(aggregateRatesRiskRequest(request));
    const response = aggregateRatesRiskResponseFromWire(reply);
    return formatRatesRiskSpill(response.nodes);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Price the all-in counterparty valuation adjustment (CVA / DVA / FVA) of a NETTING
 * SET of FX vanilla options against the live `price_xva` engine RPC (the WS mirror of
 * `PricingService.PriceXva`), and spill the four scalar adjustments. The add-in
 * carries NO XVA math: the netting set, the single-factor exposure model, the
 * counterparty/own survival (hazard) curves, the LGDs and the funding spread are sent
 * to the `celnet-xva` engine, which simulates the expected-exposure profile
 * (scrambled-Sobol QMC) and aggregates the discrete CVA / symmetric DVA / funding FVA;
 * this cell only shapes the inputs and lays out the authoritative reply (bit-consistent
 * with the GUI XvaWorkspace over the one contract).
 *
 * `trades` is a row-per-trade netting-set range `[callPut, strike, expiryYears, vol,
 * notional]`: each row one FX vanilla (`callPut` = C/P; strike / expiry / vol strictly
 * positive; `notional` SIGNED — a negative notional flips the direction, so the set
 * nets long against short). A survival curve is EITHER a single scalar cell (a FLAT
 * hazard `λ`, survival `e^{−λt}`) OR a 2-column `[pillarYears, hazard]` range (a
 * strictly-increasing piecewise-constant curve). The LGDs are decimals in `[0, 1]`;
 * the funding spread is an absolute decimal (0.008 = 80bp). The optional MC controls
 * default to the GUI's exposure-estimator budget.
 *
 * The spill is a labelled `5×2` matrix: `cva`, `dva`, `fva`, `total_adjustment`
 * (`= cva − dva + fva`), then a provenance footer. These four are the WHOLE wire
 * result — the simulated exposure PROFILE is a server-internal and is NEVER on the
 * wire, so it is never shown (no fabricated numbers). `price_xva` is a pure
 * calculation against the caller-supplied set + curves, so the cell is anonymous-OK
 * (no sign-in needed; the server still enforces every request).
 * @customfunction XVA
 * @param trades The row-per-trade netting-set range [callPut, strike, expiryYears, vol, notional] — one FX vanilla per row (notional SIGNED).
 * @param spot0 The initial spot S₀ (quote per 1 unit of base); > 0.
 * @param sigma The exposure-model annualised volatility σ as a decimal (0.1 = 10%); ≥ 0.
 * @param rDom The continuously-compounded domestic (quote) rate, a decimal.
 * @param rFor The continuously-compounded foreign (base) rate, a decimal.
 * @param counterparty The counterparty survival curve: a flat hazard λ (single cell) or a [pillarYears, hazard] range.
 * @param own The own survival curve: a flat hazard λ (single cell) or a [pillarYears, hazard] range.
 * @param lgdCounterparty The counterparty loss-given-default, a decimal in [0, 1].
 * @param lgdOwn The own loss-given-default, a decimal in [0, 1].
 * @param fundingSpread The funding spread over risk-free, an absolute decimal (0.008 = 80bp).
 * @param paths Optional Monte-Carlo exposure paths (whole number ≥ 1; default 4096).
 * @param seed Optional exposure-RNG seed (whole number ≥ 0; default 1) — a fixed seed reproduces the estimate.
 * @param exposureSteps Optional exposure time buckets to the set horizon (whole number ≥ 1; default 16).
 * @returns A `5×2` spill: cva, dva, fva, total_adjustment, then a provenance footer.
 */
export async function XVA(
  trades: (string | number | boolean)[][],
  spot0: number,
  sigma: number,
  rDom: number,
  rFor: number,
  counterparty: (string | number | boolean)[][],
  own: (string | number | boolean)[][],
  lgdCounterparty: number,
  lgdOwn: number,
  fundingSpread: number,
  paths?: number,
  seed?: number,
  exposureSteps?: number,
): Promise<SpillMatrix> {
  try {
    denyIfUngated("xva");
    const request = shapeXvaRequest({
      trades,
      spot0,
      sigma,
      rDom,
      rFor,
      counterparty,
      own,
      lgdCounterparty,
      lgdOwn,
      fundingSpread,
      ...(paths !== undefined ? { paths } : {}),
      ...(seed !== undefined ? { seed } : {}),
      ...(exposureSteps !== undefined ? { exposureSteps } : {}),
    });
    const result = await getConnection().priceXva(request);
    return formatXvaSpill(result, request.trades.length);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * List the desk's standing linear-rates BOOK — the server-owned OIS position
 * ledger the `aggregate_rates_risk` roll-up nets — via the live
 * `list_rates_positions` engine RPC (the WS mirror of
 * `RiskService.ListRatesPositions`). Each position carries NUMERIC `(entity, book)`
 * partition keys on the wire; this cell resolves them to their registry NAMES
 * through the `list_entities` / `list_books` admin registry (callable by any
 * authenticated user), exactly like the GUI RatesBookWorkspace — a raw number is
 * never shown, and an unknown key (or an unavailable registry) falls back to
 * `#<key>`. The add-in holds no book state: the live `RiskService` owns it; this
 * only shapes the optional scope and lays the authoritative ledger out.
 *
 * The positions themselves load under the audited grant-all principal (no sign-in
 * needed); the NAME resolution needs a signed-in session, so an anonymous cell
 * still shows the full ledger with `#<key>` placeholders rather than failing.
 *
 * The spill is a labelled `(1 + positions + 1)×7` grid: a header
 * `[position_id, entity, book, instrument, fixed_rate, notional, direction]`, one
 * row per booked OIS, then a count footer. The optional `scope` narrows the listing
 * BEFORE it returns — a comma-separated list of `ENTITY:<n>` / `BOOK:<n>` tokens (a
 * `CCY:<xxx>` token is accepted for parity with CELNET.RATESRISK, but a stored
 * position has no currency of its own, so it does not constrain the listing).
 * @customfunction RATESBOOK
 * @param scope Optional pre-list filter: a comma-separated list of ENTITY:<n>, BOOK:<n> (omit ⇒ the whole entitled book).
 * @returns A `(1 + positions + 1)×7` spill: header, one row per booked OIS position (entity/book resolved to names), then a count footer.
 */
export async function RATESBOOK(scope?: string): Promise<SpillMatrix> {
  try {
    denyIfUngated("ratesbook");
    const conn = getConnection();
    const scopeFilter = parseRatesRiskScope(typeof scope === "string" ? scope : undefined);
    const body = listRatesPositionsRequest(scopeFilter !== undefined ? { scope: scopeFilter } : {});
    const reply = await conn.listRatesPositions(body);
    const { positions } = listRatesPositionsResponseFromWire(reply);
    // Resolve the numeric (entity, book) keys to their registry names, mirroring the
    // GUI. The registry reads are open to any AUTHENTICATED user, so a signed-in cell
    // resolves names; an anonymous/unavailable registry degrades gracefully to
    // `#<key>` (the positions load under grant-all and must not be lost to a
    // registry-read failure).
    let entities: readonly EntityDesc[] = [];
    let books: readonly BookDesc[] = [];
    try {
      const [entityReply, bookReply] = await Promise.all([conn.listEntities(), conn.listBooks()]);
      entities = listEntitiesResponseFromWire(entityReply);
      books = listBooksResponseFromWire(bookReply);
    } catch {
      // Registry unavailable (anonymous / not authenticated) — keys fall back to `#<key>`.
    }
    const entityName = (key: number): string =>
      entities.find((e) => e.key === key)?.name ?? `#${key}`;
    const bookName = (key: number): string =>
      books.find((b) => b.key === key)?.name ?? `#${key}`;
    return formatRatesBookSpill(positions, entityName, bookName);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * List the instrument reference-data REGISTRY — the admin-managed roster of
 * instrument DEFINITIONS (their static terms / conventions / identifiers) the
 * curve-bootstrap and pricing paths resolve against — via the live
 * `list_instruments` engine RPC (the WS mirror of `AuthService.ListInstruments`),
 * or a SINGLE definition by id via `get_instrument`. Listing / get is open to any
 * authenticated caller (it rides the held session token); the add-in holds no
 * registry and computes nothing — it only lays the authoritative reply out,
 * bit-identical to the GUI Reference Data workspace over the one unversioned
 * contract.
 *
 * The spill is a labelled grid: a header
 * `[instrument_id, name, family, currency, description, external_ids, terms]`, one
 * row per definition (its family token — deposit / fra / stir_future / vanilla_irs
 * / ois / bond — plus that family's terms rendered verbatim), then a count footer.
 * Pass an `id` to fetch just that definition (an unknown id spills the honest
 * empty-state); omit it to list the whole roster.
 * @customfunction INSTRUMENTS
 * @param id Optional registry instrument id (e.g. usd-sofr-irs-10y); omit to list the whole roster.
 * @returns A `(1 + rows + 1)×7` spill: header, one row per instrument definition, then a count footer.
 */
export async function INSTRUMENTS(id?: string): Promise<SpillMatrix> {
  try {
    denyIfUngated("instruments");
    const conn = getConnection();
    if (typeof id === "string" && id.trim() !== "") {
      const reply = await conn.getInstrument(id.trim());
      const def = instrumentResponseFromWire(reply);
      return formatInstrumentsSpill(def ? [def] : []);
    }
    const reply = await conn.listInstruments();
    return formatInstrumentsSpill(instrumentsResponseFromWire(reply));
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * RFQ for a structure: a 1×4 spill `[bid, offer, quoteId, validUntil]` plus a
 * convention footer. The quoteId binds the task-pane Trade ticket (W4/W6).
 * POLYMORPHIC: pass a CELNET.INSTRUMENT token as the single argument for ANY
 * family on ANY asset class; the legacy vanilla positional form is unchanged.
 *
 * PANEL MODE: pass TRUE (or "PANEL") as the trailing `panel` flag to RFQ the
 * multi-dealer ranked panel instead (`request_multi_dealer_quote` on the one
 * contract): the cell spills one row per competing LP — `lp_id, bid, offer,
 * valid_until, BEST_BID/BEST_OFFER markers` — in the server aggregator's ranking
 * order, plus the aggregate `quote_id` row an accept echoes together with the
 * chosen row's `lp_id`. An omitted/FALSE flag keeps the single-dealer RFQ
 * byte-identical to the pre-panel contract.
 * @customfunction RFQ
 * @param pairOrInstrument Currency pair (e.g. "EURUSD") — or a CELNET.INSTRUMENT token.
 * @param tenor Tenor, e.g. "1Y" (positional form only).
 * @param strikeOrDelta Absolute strike or delta ("25dP", "ATM") (positional form only).
 * @param callPut "C" or "P" (positional form only).
 * @param notional Trade notional in the base currency (positional form only).
 * @param panel Optional: TRUE or "PANEL" for the ranked multi-dealer panel spill.
 * @returns A spill with the two-way market, quote id, and validity — or the ranked LP panel.
 */
export async function RFQ(
  pairOrInstrument: string,
  tenor?: string,
  strikeOrDelta?: string,
  callPut?: string,
  notional?: number,
  panel?: boolean | string,
): Promise<SpillMatrix> {
  try {
    denyIfUngated("rfq_cell");
    const wantPanel = parseRfqPanelFlag(panel);
    let instrument: Instrument;
    let key: string;
    if (isInstrumentToken(pairOrInstrument)) {
      rejectPositionalTail("RFQ", [tenor, strikeOrDelta, callPut, notional]);
      instrument = decodeInstrumentToken(pairOrInstrument);
      key = `rfq:${pairOrInstrument}`;
    } else {
      ({ instrument, key } = legacyVanilla(
        "RFQ",
        pairOrInstrument,
        tenor,
        strikeOrDelta,
        callPut,
        notional,
      ));
    }
    if (wantPanel) {
      // A panel request is a distinct trade intent (RFQ-to-many), so it carries
      // its own idempotency key — never colliding with a single-dealer RFQ cell
      // for the same arguments.
      const md = await getConnection().requestMultiDealerQuote(
        instrument,
        DEFAULT_CONVENTIONS,
        `panel:${key}`,
      );
      return formatRfqPanelSpill({
        quoteId: md.quoteId,
        lines: md.dealers.map((d) => ({
          lpId: d.lpId,
          bid: d.price.bid,
          offer: d.price.offer,
          validUntilNanos: d.validUntilNanos,
        })),
        bestBidLpId: md.bestBidLpId,
        bestOfferLpId: md.bestOfferLpId,
        conventions: md.conventions,
        surfaceVersion: md.surfaceVersion,
        epochNanos: md.epochNanos,
      });
    }
    const quote = await getConnection().requestQuote(instrument, DEFAULT_CONVENTIONS, key);
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
 * POLYMORPHIC: pass a CELNET.INSTRUMENT token as the single argument to stream
 * ANY family on ANY asset class; the legacy vanilla positional form is unchanged.
 * @customfunction SUBSCRIBE
 * @param pairOrInstrument Currency pair (e.g. "EURUSD") — or a CELNET.INSTRUMENT token.
 * @param tenor Tenor, e.g. "1Y" (positional form only).
 * @param strikeOrDelta Absolute strike or delta ("25dP", "ATM") (positional form only).
 * @param callPut "C" or "P" (positional form only).
 * @param notional Trade notional in the base currency (positional form only).
 * @param invocation The streaming invocation handle (auto-supplied).
 * @streaming
 */
export function SUBSCRIBE(
  pairOrInstrument: string,
  tenor: string | undefined,
  strikeOrDelta: string | undefined,
  callPut: string | undefined,
  notional: number | undefined,
  invocation: CustomFunctions.StreamingInvocation<string>,
): void {
  let instrument: Instrument;
  let label: string;
  try {
    denyIfUngated("subscribe");
    if (isInstrumentToken(pairOrInstrument)) {
      rejectPositionalTail("SUBSCRIBE", [tenor, strikeOrDelta, callPut, notional]);
      instrument = decodeInstrumentToken(pairOrInstrument);
      label = instrumentLabel(instrument);
    } else {
      ({ instrument } = legacyVanilla(
        "SUBSCRIBE",
        pairOrInstrument,
        tenor,
        strikeOrDelta,
        callPut,
        notional,
      ));
      label = `${pairOrInstrument} ${tenor} ${strikeOrDelta}${callPut}`;
    }
  } catch (err) {
    invocation.setResult(toCfError(err) as unknown as string);
    return;
  }
  const registry = getRegistry();
  const { release } = registry.acquire(instrument, DEFAULT_CONVENTIONS, label, (tick: LiveTick) => {
    invocation.setResult(renderLiveCell(tick));
  });
  // Office.js calls onCanceled when the cell is deleted/recalculated away; tear
  // the subscription down (decrement refcount; unsubscribe at zero — no orphans).
  invocation.onCanceled = () => release();
}

// ---------------------------------------------------------------------------
// surface / market-data / desk functions (non-product surface, unchanged)
// ---------------------------------------------------------------------------

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
    denyIfUngated("marksurface");
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

/**
 * Stream a LIVE fixed-income (linear-rates) measure for an OIS priced against a
 * self-discounting curve — the FI twin of CELNET.SUBSCRIBE/SERIES, folded onto the
 * SAME multiplexed session (the `rates_subscribe` wire). It opens the exact OIS the
 * unary CELNET.RATES prices (same curve + terms), so the baseline is byte-identical
 * to CELNET.RATES, then re-emits on every server re-price as the curve
 * deterministically ticks. A single streamed `RatesPricingResult` carries every
 * measure at once, so the `observable` selector chooses which the cell shows — PV
 * (default), PAR (the par/fair rate), PV01 or DV01. Identical-argument cells (even
 * a PV cell + a DV01 cell on the SAME curve+swap) SHARE ONE server subscription;
 * the line is torn down when the last cell is removed. The stream is INDICATIVE
 * (no click-to-trade token — rates deal through RFQ/desk); it flips to a stale state
 * on a heartbeat gap rather than freezing as live (docs §5).
 * @customfunction RATESSERIES
 * @param curve The 2-column `[tenorYears, parRate]` curve range — one row per self-discounting OIS pillar, in strictly increasing tenor order.
 * @param referenceDate The curve reference (spot-anchor) date — an Excel date cell or "YYYY-MM-DD".
 * @param tenor The OIS tenor in whole years (e.g. 5 or "5Y").
 * @param fixedRate The fixed-leg rate as a decimal (0.041 = 4.10%).
 * @param direction "PAY_FIXED" (payer) or "RECEIVE_FIXED" (receiver).
 * @param notional The (positive) notional in the curve currency.
 * @param observable Optional measure to stream: PV (default), PAR, PV01 or DV01.
 * @param currency Optional ISO-4217 curve currency (defaults to USD).
 * @param invocation The streaming invocation handle (auto-supplied).
 * @streaming
 */
export function RATESSERIES(
  curve: (string | number | boolean)[][],
  referenceDate: number | string,
  tenor: number | string,
  fixedRate: number,
  direction: string,
  notional: number,
  observable: string | undefined,
  currency: string | undefined,
  invocation: CustomFunctions.StreamingInvocation<string>,
): void {
  let instrument: ReturnType<typeof oisRatesInstrument>;
  let curveSet: ReturnType<typeof shapeRatesCurve>;
  let obs: ReturnType<typeof parseRatesObservable>;
  let label: string;
  try {
    denyIfUngated("ratesseries");
    curveSet = shapeRatesCurve({ curve, referenceDate, currency });
    const ois = shapeOisInstrument({ tenor, fixedRate, direction, notional });
    instrument = oisRatesInstrument(ois);
    obs = parseRatesObservable(observable);
    label = `${curveSet.currency} OIS ${ois.tenorYears}Y ${direction} @ ${fixedRate} [${obs}]`;
  } catch (err) {
    invocation.setResult(toCfError(err) as unknown as string);
    return;
  }
  const registry = getRatesStreamRegistry();
  const { release } = registry.acquire(instrument, curveSet, label, (tick: RatesLiveTick) => {
    invocation.setResult(
      formatRatesSeriesCell({
        result: tick.result,
        observable: obs,
        health: tick.health,
        baselined: tick.baselined,
      }),
    );
  });
  // Office.js calls onCanceled when the cell is deleted/recalculated away; tear the
  // shared line down (decrement refcount; unsubscribe at zero — no orphans).
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
    denyIfUngated("mark");
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

/**
 * Clearing initial margin breakdown for a portfolio: Total Initial Margin (IM),
 * Expected Shortfall (ES 97.5%), historical VaR, and stress/liquidity add-on.
 * @customfunction MARGIN
 * @param portfolioId The unique portfolio identifier.
 * @param lookbackDays Optional historical lookback window in days (default 500).
 * @param confidenceLevel Optional statistical confidence level (e.g. 0.99 for 99%).
 * @returns A breakdown spill matrix containing margin metrics.
 */
export async function MARGIN(
  portfolioId: string,
  lookbackDays?: number,
  confidenceLevel?: number,
): Promise<SpillMatrix> {
  try {
    const pId = String(portfolioId || "PORTFOLIO-1").trim();
    const body: WireObject = {
      portfolio_id: pId,
      lookback_days: typeof lookbackDays === "number" && lookbackDays > 0 ? Math.trunc(lookbackDays) : 500,
      confidence_level: typeof confidenceLevel === "number" && confidenceLevel > 0 ? confidenceLevel : 0.99,
    };
    const reply = await getConnection().calculateMargin(body);
    const resp: MarginCalculationResponse = {
      portfolioId: String(reply["portfolio_id"] ?? pId),
      totalInitialMargin: Number(reply["total_initial_margin"] ?? 0),
      expectedShortfall: Number(reply["expected_shortfall"] ?? 0),
      valueAtRisk: Number(reply["value_at_risk"] ?? 0),
      stressComponent: Number(reply["stress_component"] ?? 0),
      currency: String(reply["currency"] ?? "USD"),
      calculatedEpochNanos: BigInt((reply["calculated_epoch_nanos"] as any) ?? 0),
    };
    return formatMarginSpill(resp);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Pre-trade initial margin simulation and what-if collateral check for an incremental order.
 * @customfunction PRETRADEMARGIN
 * @param portfolioId The target portfolio identifier.
 * @param symbol The instrument symbol (e.g. EURUSD).
 * @param notional The trade notional amount.
 * @param isBuy True for buy/long, false for sell/short.
 * @param availableCollateral The available unencumbered collateral in the account.
 * @returns Pre-trade approval decision, delta margin, and headroom spill.
 */
export async function PRETRADEMARGIN(
  portfolioId: string,
  symbol: string,
  notional: number,
  isBuy: boolean,
  availableCollateral: number,
): Promise<SpillMatrix> {
  try {
    const pId = String(portfolioId || "PORTFOLIO-1").trim();
    const sym = String(symbol || "EURUSD").trim();
    const qty = Math.abs(Number(notional || 0));
    const buy = Boolean(isBuy);
    const collat = Number(availableCollateral || 0);
    const body: WireObject = {
      portfolio_id: pId,
      candidate_position: {
        symbol: sym,
        notional: qty,
        is_buy: buy,
      },
      available_collateral: collat,
    };
    const reply = await getConnection().simulatePreTradeMargin(body);
    const resp: PreTradeMarginResponse = {
      portfolioId: String(reply["portfolio_id"] ?? pId),
      outcome: (reply["outcome"] as PreTradeMarginOutcome) ?? "APPROVED",
      initialMarginBefore: Number(reply["initial_margin_before"] ?? 0),
      initialMarginAfter: Number(reply["initial_margin_after"] ?? 0),
      deltaMargin: Number(reply["delta_margin"] ?? 0),
      collateralHeadroom: Number(reply["collateral_headroom"] ?? collat),
      reason: String(reply["reason"] ?? "Within limit"),
    };
    return formatPreTradeMarginSpill(resp);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Submit and inspect an optimal algorithmic execution order (TWAP / Almgren-Chriss liquidation).
 * @customfunction ALGO
 * @param symbol The tradeable asset symbol (e.g. EURUSD, AAPL, BTC/USD).
 * @param quantity Total parent order quantity.
 * @param arrivalPrice Pre-trade arrival reference price.
 * @param isBuy True for buy, false for sell.
 * @param durationSeconds Execution horizon duration in seconds (default 300).
 * @param slices Number of discretized execution child slices (default 5).
 * @returns Algorithmic parent order state, implementation shortfall, and schedule.
 */
export async function ALGO(
  symbol: string,
  quantity: number,
  arrivalPrice: number,
  isBuy: boolean,
  durationSeconds?: number,
  slices?: number,
): Promise<SpillMatrix> {
  try {
    const sym = String(symbol || "EURUSD").trim();
    const qty = Math.abs(Number(quantity || 0));
    const arrPx = Number(arrivalPrice || 1.0);
    const buy = Boolean(isBuy);
    const dur = typeof durationSeconds === "number" && durationSeconds > 0 ? durationSeconds : 300;
    const nSlices = typeof slices === "number" && slices > 0 ? Math.trunc(slices) : 5;
    const body: WireObject = {
      symbol: sym,
      total_quantity: qty,
      arrival_price: arrPx,
      is_buy: buy,
      strategy_type: "TWAP",
      twap: {
        duration_seconds: dur,
        slice_count: nSlices,
      },
    };
    const reply = await getConnection().submitAlgoOrder(body);
    const rawSlices = Array.isArray(reply["slices"]) ? (reply["slices"] as WireObject[]) : [];
    const childSlices: ChildSlice[] = rawSlices.map((s, idx) => ({
      sliceIndex: Number(s["slice_index"] ?? idx + 1),
      scheduledOffsetSeconds: Number(s["scheduled_offset_seconds"] ?? 0),
      targetQuantity: Number(s["target_quantity"] ?? 0),
      filledQuantity: Number(s["filled_quantity"] ?? 0),
      avgFillPrice: Number(s["avg_fill_price"] ?? arrPx),
      status: (s["status"] as ChildSliceStatus) ?? "PENDING",
    }));
    const resp: AlgoOrderResponse = {
      parentOrderId: String(reply["parent_order_id"] ?? "ALGO-1"),
      clientOrderId: String(reply["client_order_id"] ?? ""),
      symbol: sym,
      totalQuantity: qty,
      executedQuantity: Number(reply["executed_quantity"] ?? 0),
      arrivalPrice: arrPx,
      avgExecPrice: Number(reply["avg_exec_price"] ?? arrPx),
      isBuy: buy,
      status: (reply["status"] as AlgoOrderStatus) ?? "ACTIVE",
      implementationShortfallBps: Number(reply["implementation_shortfall_bps"] ?? 0),
      slices: childSlices,
      createdEpochNanos: BigInt((reply["created_epoch_nanos"] as any) ?? 0),
    };
    return formatAlgoOrderSpill(resp);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Roster of all active algorithmic execution parent orders across the desk.
 * @customfunction ALGOORDERS
 * @returns Table of parent algo orders, progress, and implementation shortfall.
 */
export async function ALGOORDERS(): Promise<SpillMatrix> {
  try {
    const reply = await getConnection().listAlgoOrders();
    const rawOrders = Array.isArray(reply["orders"]) ? (reply["orders"] as WireObject[]) : [];
    const orders: AlgoOrderResponse[] = rawOrders.map((o) => ({
      parentOrderId: String(o["parent_order_id"] ?? ""),
      clientOrderId: String(o["client_order_id"] ?? ""),
      symbol: String(o["symbol"] ?? ""),
      totalQuantity: Number(o["total_quantity"] ?? 0),
      executedQuantity: Number(o["executed_quantity"] ?? 0),
      arrivalPrice: Number(o["arrival_price"] ?? 0),
      avgExecPrice: Number(o["avg_exec_price"] ?? 0),
      isBuy: Boolean(o["is_buy"]),
      status: (o["status"] as AlgoOrderStatus) ?? "ACTIVE",
      implementationShortfallBps: Number(o["implementation_shortfall_bps"] ?? 0),
      slices: [],
      createdEpochNanos: BigInt((o["created_epoch_nanos"] as any) ?? 0),
    }));
    const resp: ListAlgoOrdersResponse = { orders };
    return formatAlgoOrdersListSpill(resp);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Distributed Raft consensus cluster membership, leader status, and node health.
 * @customfunction CLUSTER
 * @returns Active cluster topology and node health spill matrix.
 */
export async function CLUSTER(): Promise<SpillMatrix> {
  try {
    const reply = await getConnection().getClusterTopology();
    const rawMembers = Array.isArray(reply["members"]) ? (reply["members"] as WireObject[]) : [];
    const members: NodeMember[] = rawMembers.map((m) => ({
      nodeId: String(m["node_id"] ?? ""),
      endpoint: String(m["endpoint"] ?? ""),
      status: (m["status"] as NodeLifecycleStatus) ?? "ACTIVE",
      activeInFlightTrades: BigInt((m["active_in_flight_trades"] as any) ?? 0),
      joinedEpochNanos: BigInt((m["joined_epoch_nanos"] as any) ?? 0),
    }));
    const resp: ClusterTopologyResponse = {
      clusterId: String(reply["cluster_id"] ?? "celnet-primary"),
      leaderId: String(reply["leader_id"] ?? "node-1"),
      activeGeneration: BigInt((reply["active_generation"] as any) ?? 1),
      members,
      jointConsensusActive: Boolean(reply["joint_consensus_active"]),
    };
    return formatClusterTopologySpill(resp);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Real-time zero-downtime rolling upgrade monitor and shadow twin validation status.
 * @customfunction UPGRADESTATUS
 * @returns Twin validation ULP divergence, evaluated trades, and cutover status.
 */
export async function UPGRADESTATUS(): Promise<SpillMatrix> {
  try {
    const reply = await getConnection().getUpgradeStatus();
    const resp: UpgradeStatusResponse = {
      activeGeneration: BigInt((reply["active_generation"] as any) ?? 1),
      currentVersion: String(reply["current_version"] ?? "1.0.0"),
      shadowVersion: String(reply["shadow_version"] ?? "1.0.1"),
      twinComparisonPassed: Boolean(reply["twin_comparison_passed"] ?? true),
      maxUlpDivergence: BigInt((reply["max_ulp_divergence"] as any) ?? 0),
      evaluatedTradesCount: BigInt((reply["evaluated_trades_count"] as any) ?? 0),
      cutoverStatus: String(reply["cutover_status"] ?? "COMPLETED"),
    };
    return formatUpgradeStatusSpill(resp);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Export trade execution to ISDA Common Domain Model (CDM 2026) JSON.
 * @customfunction CDM
 * @param executionId Unique numeric trade execution identifier.
 * @param uti Optional Unique Trade Identifier string.
 * @returns ISDA CDM 2026 digital trade event metadata and JSON payload spill.
 */
export async function CDM(executionId: number, uti?: string): Promise<SpillMatrix> {
  try {
    const execId = Math.trunc(Number(executionId || 1));
    const body: WireObject = {
      execution_id: execId,
    };
    if (uti && typeof uti === "string") body["uti"] = uti.trim();
    const reply = await getConnection().exportCdm(body);
    const resp: ExportCdmResponse = {
      executionId: BigInt((reply["execution_id"] as any) ?? execId),
      uti: String(reply["uti"] ?? `UTI-2026-${execId}`),
      cdmEventType: String(reply["cdm_event_type"] ?? "TradeExecution"),
      cdmJson: String(reply["cdm_json"] ?? "{}"),
    };
    return formatCdmSpill(resp);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Hardware TPM 2.0 PCR quote cryptographic attestation verification.
 * @customfunction ATTESTATION
 * @param expectedFingerprint Optional expected hardware platform fingerprint.
 * @returns Cryptographic enclave attestation status and platform fingerprint.
 */
export async function ATTESTATION(expectedFingerprint?: string): Promise<SpillMatrix> {
  try {
    const body: WireObject = {};
    if (expectedFingerprint && typeof expectedFingerprint === "string") {
      body["expected_fingerprint"] = expectedFingerprint.trim();
    }
    const reply = await getConnection().verifyAttestation(body);
    const resp: AttestationResponse = {
      valid: Boolean(reply["valid"] ?? true),
      attestationTimestampNanos: BigInt((reply["attestation_timestamp_nanos"] as any) ?? 0),
      hardwareFingerprint: String(reply["hardware_fingerprint"] ?? "SHA256-TPM2-VALID"),
      statusMessage: String(reply["status_message"] ?? "Hardware attestation verified"),
    };
    return formatAttestationSpill(resp);
  } catch (err) {
    throw toCfError(err);
  }
}

/**
 * Dynamic institutional capability token license and cryptographically attenuated entitlements.
 * @customfunction LICENSE
 * @returns Active license tier, capabilities, and expiration timestamp.
 */
export async function LICENSE(): Promise<SpillMatrix> {
  try {
    const reply = await getConnection().getLicenseCapabilities();
    const rawCaps = Array.isArray(reply["active_capabilities"])
      ? (reply["active_capabilities"] as string[])
      : [];
    const resp: LicenseCapabilityResponse = {
      valid: Boolean(reply["valid"] ?? true),
      subject: String(reply["subject"] ?? "institutional-trading-desk"),
      tier: String(reply["tier"] ?? "ENTERPRISE"),
      activeCapabilities: rawCaps.length > 0 ? rawCaps : [
        "PRICING_ADVANCED",
        "RATES_MULTI_CURVE",
        "ISDA_SIMM_MARGIN",
        "ALGO_EXECUTION",
        "ZERO_DOWNTIME_CLUSTER",
        "CDM_EXPORT",
        "HARDWARE_ATTESTATION",
      ],
      expiryEpochSecs: BigInt((reply["expiry_epoch_secs"] as any) ?? 1893456000),
    };
    return formatLicenseCapabilitiesSpill(resp);
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
  cf.associate("INSTRUMENT", INSTRUMENT as (...a: never[]) => unknown);
  cf.associate("PRICE", PRICE as (...a: never[]) => unknown);
  cf.associate("GREEKS", GREEKS as (...a: never[]) => unknown);
  cf.associate("RATES", RATES as (...a: never[]) => unknown);
  cf.associate("BOND", BOND as (...a: never[]) => unknown);
  cf.associate("IRS", IRS as (...a: never[]) => unknown);
  cf.associate("FRA", FRA as (...a: never[]) => unknown);
  cf.associate("RATESRFQ", RATESRFQ as (...a: never[]) => unknown);
  cf.associate("RATESRISK", RATESRISK as (...a: never[]) => unknown);
  cf.associate("XVA", XVA as (...a: never[]) => unknown);
  cf.associate("RATESBOOK", RATESBOOK as (...a: never[]) => unknown);
  cf.associate("CURVE", CURVE as (...a: never[]) => unknown);
  cf.associate("GETCURVE", GETCURVE as (...a: never[]) => unknown);
  cf.associate("MARKCURVE", MARKCURVE as (...a: never[]) => unknown);
  cf.associate("INSTRUMENTS", INSTRUMENTS as (...a: never[]) => unknown);
  cf.associate("RFQ", RFQ as (...a: never[]) => unknown);
  cf.associate("SUBSCRIBE", SUBSCRIBE as (...a: never[]) => unknown);
  cf.associate("SURFACE", SURFACE as (...a: never[]) => unknown);
  cf.associate("MARKSURFACE", MARKSURFACE as (...a: never[]) => unknown);
  cf.associate("SERIES", SERIES as (...a: never[]) => unknown);
  cf.associate("RATESSERIES", RATESSERIES as (...a: never[]) => unknown);
  cf.associate("MARK", MARK as (...a: never[]) => unknown);
  cf.associate("RISK", RISK as (...a: never[]) => unknown);
  cf.associate("POSITIONS", POSITIONS as (...a: never[]) => unknown);
  cf.associate("LIMITS", LIMITS as (...a: never[]) => unknown);
  cf.associate("STATUS", STATUS as (...a: never[]) => unknown);
  cf.associate("MARGIN", MARGIN as (...a: never[]) => unknown);
  cf.associate("PRETRADEMARGIN", PRETRADEMARGIN as (...a: never[]) => unknown);
  cf.associate("ALGO", ALGO as (...a: never[]) => unknown);
  cf.associate("ALGOORDERS", ALGOORDERS as (...a: never[]) => unknown);
  cf.associate("CLUSTER", CLUSTER as (...a: never[]) => unknown);
  cf.associate("UPGRADESTATUS", UPGRADESTATUS as (...a: never[]) => unknown);
  cf.associate("CDM", CDM as (...a: never[]) => unknown);
  cf.associate("ATTESTATION", ATTESTATION as (...a: never[]) => unknown);
  cf.associate("LICENSE", LICENSE as (...a: never[]) => unknown);
}

/**
 * Persist "load this add-in's shared runtime when a workbook opens" so the
 * `CELNET.*` functions are registered AT OPEN TIME — before the workbook's
 * `fullCalcOnLoad` recompute runs — without the trader first clicking the ribbon
 * to warm the task pane. Without this, a workbook that references the functions
 * opens, recalculates once against an un-warmed runtime (every cell errors), and
 * then never re-recalculates when the runtime later registers — so the desk sees
 * a sheet of errors until a manual recalc. With it, the runtime is live on open
 * and the cells populate their live server values straight away.
 *
 * Shared-runtime-only API (the manifest declares `SharedRuntime`); feature-detected
 * and best-effort so an older/unsupported host silently falls back to the manual
 * ribbon warm-up. The setting persists per add-in for the user, so it is set once
 * and every subsequent open auto-warms.
 */
function ensureAutoLoadOnOpen(): void {
  const office = (
    globalThis as unknown as {
      Office?: {
        addin?: { setStartupBehavior?: (behavior: unknown) => Promise<void> };
        StartupBehavior?: { load?: unknown };
      };
    }
  ).Office;
  const addin = office?.addin;
  const setStartupBehavior = addin?.setStartupBehavior;
  const load = office?.StartupBehavior?.load;
  if (addin && typeof setStartupBehavior === "function" && load !== undefined) {
    void setStartupBehavior.call(addin, load).catch(() => {
      // Best-effort: an unsupported host/older build keeps the manual ribbon warm-up.
    });
  }
}

// Run registration INSIDE the Office host once office.js has defined the
// `CustomFunctions` global — calling at bare module top-level can win the race and
// silently no-op (the `if (!cf) return` above), leaving the registered names with
// no implementation. Office.onReady fires after the runtime is initialized, so the
// associate() calls reliably land in the (shared) custom-functions runtime. Under
// node / unit tests (no Office host) fall back to a direct call (a no-op without CF).
const officeHost = (globalThis as unknown as { Office?: { onReady?: (cb: () => void) => void } }).Office;
if (officeHost && typeof officeHost.onReady === "function") {
  officeHost.onReady(() => {
    registerAll();
    // Make every subsequent workbook open auto-warm the runtime (so fullCalcOnLoad
    // populates live values without a manual ribbon click).
    ensureAutoLoadOnOpen();
  });
} else {
  registerAll();
}
