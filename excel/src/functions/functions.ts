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
  formatGreeksSpill,
  formatInstrumentsSpill,
  formatLimitsSpill,
  formatMarkStatusSpill,
  formatPositionsSpill,
  formatPremiumSpill,
  formatRatesBookSpill,
  formatRatesRiskSpill,
  formatRatesSpill,
  formatRfqPanelSpill,
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
  parseRatesRiskScope,
  parseRfqPanelFlag,
  parseRiskDimension,
  parseRiskScope,
  parseSmileModel,
  parseTenor,
  shapeBondInstrument,
  shapeBuildCurveRequest,
  shapeCalibration,
  shapeOisInstrument,
  shapeRatesCurve,
  shapeRatesRiskPositions,
  shapeReportingNumeraire,
  shapeVanillaInstrument,
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
import { getConnection, getRegistry, getSeriesRegistry, getSession } from "./runtime";
import type { EntryPointId } from "../contract/access";
import { brokerQuoteSetToWire, ccyPairToWire, conventionsToWire, smileFromWire, type WireObject } from "../contract/wsCodec";
import { smileModel } from "../contract/enums";
import type { Instrument, Quote } from "../contract/contract";
import type { LiveTick } from "./streamRegistry";
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
  cf.associate("RATESRISK", RATESRISK as (...a: never[]) => unknown);
  cf.associate("RATESBOOK", RATESBOOK as (...a: never[]) => unknown);
  cf.associate("CURVE", CURVE as (...a: never[]) => unknown);
  cf.associate("INSTRUMENTS", INSTRUMENTS as (...a: never[]) => unknown);
  cf.associate("RFQ", RFQ as (...a: never[]) => unknown);
  cf.associate("SUBSCRIBE", SUBSCRIBE as (...a: never[]) => unknown);
  cf.associate("SURFACE", SURFACE as (...a: never[]) => unknown);
  cf.associate("MARKSURFACE", MARKSURFACE as (...a: never[]) => unknown);
  cf.associate("SERIES", SERIES as (...a: never[]) => unknown);
  cf.associate("MARK", MARK as (...a: never[]) => unknown);
  cf.associate("RISK", RISK as (...a: never[]) => unknown);
  cf.associate("POSITIONS", POSITIONS as (...a: never[]) => unknown);
  cf.associate("LIMITS", LIMITS as (...a: never[]) => unknown);
  cf.associate("STATUS", STATUS as (...a: never[]) => unknown);
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
