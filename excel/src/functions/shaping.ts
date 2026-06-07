/**
 * Pure request-shaping + dynamic-array formatting for the CELNET.* functions.
 *
 * These functions are deliberately side-effect-free: they translate the
 * Excel-cell argument strings a trader types (`"EURUSD"`, `"1Y"`, `"25dP"`,
 * `"C"`) into the ONE current contract's typed shapes (Instrument / Conventions),
 * and format contract results into the dynamic-array geometries Excel spills.
 * No transport, no Office globals — so they are exhaustively unit-testable under
 * node with no server and no mocks of our own functionality (CLAUDE.md: verify
 * against the real contract shapes, not fakes).
 *
 * Convention transparency (docs/EXCEL-INTEGRATION.md §3.4): every spill carries a
 * footer row with the resolved convention + surface_version + timestamp, so a
 * number never appears in the grid without the convention that produced it — the
 * #1 cure for FX-options mismarks.
 */

import type {
  Accumulator,
  AccumulatorMonitoring,
  AsianMethod,
  AveragingStyle,
  BarrierKind,
  BarrierSide,
  CcyPair,
  Cliquet,
  Conventions,
  Digital,
  DigitalStyle,
  DoubleBarrier,
  FixingSchedule,
  Greeks,
  Instrument,
  Lookback,
  LookbackMonitoring,
  LookbackStyle,
  MarketObservable,
  MonitoringStyle,
  OptionType,
  Product,
  QuantoPayoff,
  Side,
  SingleBarrier,
  SmileModel,
  StrikeOrDelta,
  Tarf,
  TarfRedemption,
  Tenor,
  TenorUnit,
  Touch,
  TouchKind,
} from "../contract/contract";
import type {
  AdditiveRisk,
  LimitUtilization,
  NumeraireRate,
  ReportingNumeraire,
  RiskDimension,
  RiskNode,
  RiskPosition,
  RiskScope,
} from "../contract/riskCodec";
import { SMILE_MODEL_MEMBERS } from "../contract/enums";

/** The canonical desk default convention (spot-unadjusted Δ / ATM-forward / …),
 * matching the server fixture (`crates/celnet-server/tests/common` wire_conventions).
 * A trader rarely overrides it; an explicit `conv` range can replace any field. */
export const DEFAULT_CONVENTIONS: Conventions = {
  deltaConvention: "SPOT_UNADJUSTED",
  atmConvention: "ATM_FORWARD",
  premiumStyle: "DOMESTIC_PIPS",
  cut: "NEW_YORK_1000",
  dayCount: "ACT_365_FIXED",
  settlement: "DELIVERABLE",
};

/** A shaping error: a malformed argument the trader typed. Carried to the cell. */
export class ShapingError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ShapingError";
  }
}

// ---------------------------------------------------------------------------
// argument parsing
// ---------------------------------------------------------------------------

/** Parse a `"EURUSD"` or `"EUR/USD"` pair string into a `CcyPair`. */
export function parsePair(raw: string): CcyPair {
  const s = raw.trim().toUpperCase().replace("/", "");
  if (s.length !== 6 || !/^[A-Z]{6}$/.test(s)) {
    throw new ShapingError(`invalid pair \`${raw}\` (expected e.g. EURUSD or EUR/USD)`);
  }
  return { base: s.slice(0, 3), quote: s.slice(3, 6) };
}

/** Years-per-unit for converting a parsed tenor to the contract `expiryYears`. */
const YEARS_PER_UNIT: Record<TenorUnit, number> = {
  OVERNIGHT: 1 / 365,
  WEEKS: 7 / 365,
  MONTHS: 1 / 12,
  YEARS: 1,
};

/** A parsed tenor: the structured `Tenor` plus its expiry year fraction. */
export interface ParsedTenor {
  readonly tenor: Tenor;
  readonly expiryYears: number;
}

/**
 * Parse a tenor string into the contract `Tenor` + expiry year fraction.
 * Accepts `ON`/`O/N` (overnight), `<n>W`, `<n>M`, `<n>Y` (case-insensitive).
 */
export function parseTenor(raw: string): ParsedTenor {
  const s = raw.trim().toUpperCase();
  if (s === "ON" || s === "O/N") {
    return { tenor: { unit: "OVERNIGHT", count: 1 }, expiryYears: YEARS_PER_UNIT.OVERNIGHT };
  }
  const m = /^(\d+)\s*([WMY])$/.exec(s);
  if (!m) {
    throw new ShapingError(`invalid tenor \`${raw}\` (expected ON, 1W, 3M, 1Y, …)`);
  }
  const count = Number(m[1]);
  const unit: TenorUnit = m[2] === "W" ? "WEEKS" : m[2] === "M" ? "MONTHS" : "YEARS";
  if (count <= 0) throw new ShapingError(`tenor count must be positive in \`${raw}\``);
  return { tenor: { unit, count }, expiryYears: count * YEARS_PER_UNIT[unit] };
}

/** Parse a `"C"`/`"P"` (or CALL/PUT) string into the contract `OptionType`. */
export function parseOptionType(raw: string): OptionType {
  const s = raw.trim().toUpperCase();
  if (s === "C" || s === "CALL") return "CALL";
  if (s === "P" || s === "PUT") return "PUT";
  throw new ShapingError(`invalid call/put \`${raw}\` (expected C or P)`);
}

/**
 * Parse a strike-or-delta argument. Accepts:
 *  - a number (absolute strike), e.g. `1.12`;
 *  - a delta string `"<n>dC"`/`"<n>dP"` (e.g. `"25dP"` ⇒ -0.25, `"25dC"` ⇒ +0.25);
 *  - `"ATM"` / `"DNS"` (delta-neutral) ⇒ the convention's ATM strike (delta 0).
 * The sign convention matches the contract `SmilePoint.delta` (call +, put −).
 */
export function parseStrikeOrDelta(raw: string | number): StrikeOrDelta {
  if (typeof raw === "number") {
    if (!Number.isFinite(raw) || raw <= 0) {
      throw new ShapingError(`invalid absolute strike \`${raw}\``);
    }
    return { kind: "strike", strike: raw };
  }
  const s = raw.trim().toUpperCase();
  if (s === "ATM" || s === "DNS") return { kind: "delta", delta: 0 };
  const numeric = Number(s);
  if (Number.isFinite(numeric) && numeric > 0) return { kind: "strike", strike: numeric };
  const m = /^(\d+(?:\.\d+)?)\s*D\s*([CP])$/.exec(s);
  if (!m) {
    throw new ShapingError(`invalid strike/delta \`${raw}\` (expected 1.12, 25dP, 10dC, ATM, DNS)`);
  }
  const pct = Number(m[1]);
  if (pct <= 0 || pct >= 100) throw new ShapingError(`delta percent out of range in \`${raw}\``);
  const magnitude = pct / 100;
  const signed = m[2] === "C" ? magnitude : -magnitude;
  return { kind: "delta", delta: signed };
}

/**
 * Parse a smile-model selector string into the contract `SmileModel`. Accepts the
 * trader-facing short names (`VV`, `SABR`, `SVI`, `SSVI`, `ESSVI`/`EXTENDED`) and
 * the canonical contract names (`MARKET_HEDGE`, `STOCHASTIC_VOL`, `PARAMETRIC`,
 * `PARAMETRIC_SURFACE`, `EXTENDED_SURFACE`), case-insensitive. Empty/absent ⇒
 * `MARKET_HEDGE` (the server default Vanna-Volga construction), so an omitted
 * argument is the unchanged current behaviour. The rejection message lists the
 * canonical members straight from `SMILE_MODEL_MEMBERS` (the same list the wire
 * codec is built from) so it can never drift from the supported set.
 */
export function parseSmileModel(raw: string | undefined): SmileModel {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "VV":
    case "VANNA_VOLGA":
    case "VANNA-VOLGA":
    case "MARKET_HEDGE":
    case "MARKET-HEDGE":
      return "MARKET_HEDGE";
    case "SABR":
    case "STOCHASTIC_VOL":
    case "STOCHASTIC-VOL":
    case "STOCHVOL":
      return "STOCHASTIC_VOL";
    case "SVI":
    case "PARAMETRIC":
      return "PARAMETRIC";
    case "SSVI":
    case "PARAMETRIC_SURFACE":
    case "PARAMETRIC-SURFACE":
      return "PARAMETRIC_SURFACE";
    case "ESSVI":
    case "EXTENDED":
    case "EXTENDED_SURFACE":
    case "EXTENDED-SURFACE":
      return "EXTENDED_SURFACE";
    default:
      throw new ShapingError(
        `invalid smile model \`${raw}\` (expected one of: ${SMILE_MODEL_MEMBERS.join(", ")}; ` +
          `or the trader short names VV, SABR, SVI, SSVI, ESSVI)`,
      );
  }
}

/**
 * Parse a market-observable selector string into the contract `MarketObservable`.
 * Accepts the trader-facing names (`ATM`/`ATMVOL`, `SPOT`, `RR`/`RISK_REVERSAL`,
 * `BF`/`FLY`/`BUTTERFLY`, `FWD`/`FORWARD`), case-insensitive.
 */
export function parseObservable(raw: string): MarketObservable {
  const s = raw.trim().toUpperCase();
  switch (s) {
    case "ATM":
    case "ATMVOL":
    case "ATM_VOL":
    case "VOL":
      return "ATM_VOL";
    case "SPOT":
      return "SPOT";
    case "RR":
    case "RISK_REVERSAL":
    case "RISKREVERSAL":
      return "RISK_REVERSAL";
    case "BF":
    case "FLY":
    case "BUTTERFLY":
      return "BUTTERFLY";
    case "FWD":
    case "FORWARD":
      return "FORWARD";
    default:
      throw new ShapingError(
        `invalid observable \`${raw}\` (expected ATM, SPOT, RR, BF or FWD)`,
      );
  }
}

/**
 * Parse a delta-wing argument for the wing observables (RR/BF) and surface marks.
 * Accepts a signed/unsigned fraction (`0.25`, `0.10`) or a percent-delta string
 * (`25`, `25d`, `10`). Always returns the positive wing magnitude (the server
 * reads both signed wings for RR/BF). Throws on a wing outside (0, 0.5).
 */
export function parseDeltaWing(raw: string | number): number {
  const numeric = typeof raw === "number" ? raw : Number(String(raw).trim().replace(/D$/i, ""));
  if (!Number.isFinite(numeric) || numeric <= 0) {
    throw new ShapingError(`invalid delta wing \`${raw}\` (expected 0.25 or 25)`);
  }
  const wing = numeric >= 1 ? numeric / 100 : numeric;
  if (wing <= 0 || wing >= 0.5) {
    throw new ShapingError(`delta wing \`${raw}\` out of range (expected 0 < δ < 0.5)`);
  }
  return wing;
}

// ---------------------------------------------------------------------------
// risk shaping (RiskService — server-side hierarchical risk)
// ---------------------------------------------------------------------------

/**
 * Parse an org-dimension selector string into the contract `RiskDimension` (the
 * `aggregate_risk` group-by axis / scope dimension). Accepts the trader-facing
 * short names (`FIRM`, `TRADER`, `BOOK`, `DESK`, `PAIR`/`CCYPAIR`, `LOCATION`/
 * `LOC`, `ENTITY`/`LE`), case-insensitive. Empty/absent ⇒ `FIRM` (the apex, the
 * proto3 default), so an omitted argument rolls the whole entitled book to one node.
 */
export function parseRiskDimension(raw: string | undefined): RiskDimension {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "FIRM":
      return "FIRM";
    case "TRADER":
      return "TRADER";
    case "BOOK":
      return "BOOK";
    case "DESK":
      return "DESK";
    case "PAIR":
    case "CCYPAIR":
    case "CCY_PAIR":
      return "CCY_PAIR";
    case "LOCATION":
    case "LOC":
      return "LOCATION";
    case "ENTITY":
    case "LE":
      return "ENTITY";
    default:
      throw new ShapingError(
        `invalid dimension \`${raw}\` (expected FIRM, TRADER, BOOK, DESK, PAIR, LOCATION or ENTITY)`,
      );
  }
}

/** Parse a numeraire ccy code (3 letters), upper-cased. */
export function parseNumeraireCcy(raw: string | undefined): string {
  const s = (raw ?? "").trim().toUpperCase();
  if (s === "") return "USD"; // a sensible reporting default; overridable per call
  if (!/^[A-Z]{3}$/.test(s)) {
    throw new ShapingError(`invalid numeraire \`${raw}\` (expected a 3-letter code, e.g. USD)`);
  }
  return s;
}

/**
 * Shape a `ReportingNumeraire` from a numeraire code and an optional `[ccy, rate]`
 * spill range. The numeraire's own rate is implicit 1.0; every other currency in
 * the book needs a rate (units of numeraire per 1 unit of ccy at spot). The rates
 * arrive as the 2-D cell range Excel passes for the optional argument — rows of
 * `[ccyString, rateNumber]`; a blank/short row is skipped. A row naming the
 * numeraire itself is dropped (it is implicitly 1.0).
 */
export function shapeReportingNumeraire(
  numeraireCcy: string,
  rateRange: unknown,
): ReportingNumeraire {
  const numeraire = parseNumeraireCcy(numeraireCcy);
  const rates: NumeraireRate[] = [];
  if (Array.isArray(rateRange)) {
    for (const row of rateRange) {
      if (!Array.isArray(row) || row.length < 2) continue;
      const ccyCell = row[0];
      const rateCell = row[1];
      if (ccyCell === "" || ccyCell === null || ccyCell === undefined) continue;
      const ccy = String(ccyCell).trim().toUpperCase();
      if (!/^[A-Z]{3}$/.test(ccy)) {
        throw new ShapingError(`invalid rate ccy \`${ccyCell}\` (expected a 3-letter code)`);
      }
      if (ccy === numeraire) continue; // implicitly 1.0
      const rate = typeof rateCell === "number" ? rateCell : Number(rateCell);
      if (!Number.isFinite(rate) || rate <= 0) {
        throw new ShapingError(`invalid numeraire rate for ${ccy}: \`${rateCell}\` (finite, > 0)`);
      }
      rates.push({ ccy, rate });
    }
  }
  return { numeraire, rates };
}

/**
 * Parse a scope argument `<DIM>:<value>` (e.g. `DESK:99`) into a `RiskScope`, or
 * undefined for an empty/`ALL`/`FIRM` scope (the whole entitled book). The value is
 * the cube's interned group handle (a non-negative integer).
 */
export function parseRiskScope(raw: string | undefined): RiskScope | undefined {
  const s = (raw ?? "").trim();
  if (s === "" || s.toUpperCase() === "ALL" || s.toUpperCase() === "FIRM") return undefined;
  const m = /^([A-Za-z_]+)\s*[:=]\s*(\d+)$/.exec(s);
  if (!m || m[1] === undefined || m[2] === undefined) {
    throw new ShapingError(`invalid scope \`${raw}\` (expected DIM:value, e.g. DESK:99, or ALL)`);
  }
  return { dimension: parseRiskDimension(m[1]), value: BigInt(m[2]) };
}

// ---------------------------------------------------------------------------
// instrument shaping (one current contract)
// ---------------------------------------------------------------------------

/** The fully-parsed inputs a vanilla CELNET.* function shapes into a request. */
export interface VanillaArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly strikeOrDelta: string | number;
  readonly callPut: string;
  readonly notional: number;
}

/**
 * Shape a vanilla instrument from the cell arguments. `side` is TWO_WAY (the cell
 * reads a market, not a directional ticket); a directional booking goes through the
 * task-pane Trade button. The notional is in the base/foreign currency (CCY1).
 */
export function shapeVanillaInstrument(args: VanillaArgs): Instrument {
  if (!Number.isFinite(args.notional) || args.notional <= 0) {
    throw new ShapingError(`invalid notional \`${args.notional}\``);
  }
  const { tenor, expiryYears } = parseTenor(args.tenor);
  return {
    pair: parsePair(args.pair),
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: {
      kind: "vanilla",
      vanilla: {
        optionType: parseOptionType(args.callPut),
        strike: parseStrikeOrDelta(args.strikeOrDelta),
      },
    },
  };
}

/** The fully-parsed inputs a model-selected surface calibration shapes. */
export interface CalibrateArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly model: string | undefined;
  readonly atmVol: number;
  readonly rr25: number;
  readonly bf25: number;
  /** Optional 10Δ wings; both present ⇒ a five-point smile is calibrated. */
  readonly rr10?: number | undefined;
  readonly bf10?: number | undefined;
}

/** The validated, contract-shaped pieces of a calibration request. */
export interface ShapedCalibration {
  readonly pair: CcyPair;
  readonly tenorYears: number;
  readonly model: SmileModel;
  readonly atmVol: number;
  readonly rr25: number;
  readonly bf25: number;
  readonly rr10: number;
  readonly bf10: number;
  readonly hasTenDelta: boolean;
}

/**
 * Shape a model-selected surface calibration from cell arguments: parse the pair,
 * tenor and model, and validate the broker marks. No transport — the function
 * layer turns this into the `mark_surface` body (broker_quotes + smile_model).
 */
export function shapeCalibration(args: CalibrateArgs): ShapedCalibration {
  const pair = parsePair(args.pair);
  const { expiryYears } = parseTenor(args.tenor);
  const model = parseSmileModel(args.model);
  if (!Number.isFinite(args.atmVol) || args.atmVol <= 0 || args.atmVol >= 5) {
    throw new ShapingError(`invalid ATM vol \`${args.atmVol}\` (absolute, e.g. 0.102)`);
  }
  if (!Number.isFinite(args.rr25) || !Number.isFinite(args.bf25)) {
    throw new ShapingError("rr25/bf25 must be finite vols (e.g. 0.01, 0.003)");
  }
  const hasTenDelta = args.rr10 !== undefined && args.bf10 !== undefined;
  if (hasTenDelta && (!Number.isFinite(args.rr10) || !Number.isFinite(args.bf10))) {
    throw new ShapingError("rr10/bf10 must be finite vols when supplied");
  }
  return {
    pair,
    tenorYears: expiryYears,
    model,
    atmVol: args.atmVol,
    rr25: args.rr25,
    bf25: args.bf25,
    rr10: hasTenDelta ? (args.rr10 as number) : 0,
    bf10: hasTenDelta ? (args.bf10 as number) : 0,
    hasTenDelta,
  };
}

// ---------------------------------------------------------------------------
// swap + Asian shaping (variance/volatility swaps, arithmetic Asian options)
// ---------------------------------------------------------------------------

/**
 * Parse an averaging-style selector for an Asian option. Accepts `DISCRETE`/`D`
 * (a fixed number of equally-spaced fixings) or `CONTINUOUS`/`C`/`CONT` (the
 * continuous-monitoring limit), case-insensitive. Empty/absent ⇒ `DISCRETE`
 * (the proto3 zero value), the common desk default.
 */
export function parseAveragingStyle(raw: string | undefined): AveragingStyle {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "DISCRETE":
    case "D":
      return "DISCRETE";
    case "CONTINUOUS":
    case "CONT":
    case "C":
      return "CONTINUOUS";
    default:
      throw new ShapingError(
        `invalid averaging \`${raw}\` (expected DISCRETE or CONTINUOUS)`,
      );
  }
}

/**
 * Parse an Asian analytic-estimator selector. Accepts `CURRAN` (the geometric-
 * conditioning default) or `TW`/`TURNBULL_WAKEMAN`/`TURNBULL-WAKEMAN` (the
 * two-moment lognormal-matching estimator), case-insensitive. Empty/absent ⇒
 * `CURRAN` (the proto3 zero value, the accurate default).
 */
export function parseAsianMethod(raw: string | undefined): AsianMethod {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "CURRAN":
      return "CURRAN";
    case "TW":
    case "TURNBULL_WAKEMAN":
    case "TURNBULL-WAKEMAN":
    case "TURNBULLWAKEMAN":
      return "TURNBULL_WAKEMAN";
    default:
      throw new ShapingError(
        `invalid Asian method \`${raw}\` (expected CURRAN or TW)`,
      );
  }
}

/** The fully-parsed inputs a swap CELNET.* function shapes into a request. */
export interface SwapArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly notional: number;
  /** Fixed strike vol; `0`/absent ⇒ a fresh request reading the fair strike off the response. */
  readonly strikeVol?: number | undefined;
}

/** Validate the common (pair, tenor, notional) of a swap/Asian request. */
function shapeSwapBase(args: { pair: string; tenor: string; notional: number }): {
  pair: CcyPair;
  tenor: Tenor;
  expiryYears: number;
} {
  if (!Number.isFinite(args.notional) || args.notional <= 0) {
    throw new ShapingError(`invalid notional \`${args.notional}\``);
  }
  const { tenor, expiryYears } = parseTenor(args.tenor);
  return { pair: parsePair(args.pair), tenor, expiryYears };
}

/** Validate an optional fixed strike-vol; absent/zero ⇒ 0 (read fair off the response). */
function shapeStrikeVol(raw: number | undefined): number {
  if (raw === undefined) return 0;
  if (!Number.isFinite(raw) || raw < 0 || raw >= 5) {
    throw new ShapingError(`invalid strike vol \`${raw}\` (absolute vol ≥ 0, e.g. 0.11)`);
  }
  return raw;
}

/**
 * Shape a variance-swap instrument from the cell arguments. `side` is TWO_WAY (the
 * cell reads a fair-strike market); the notional is in the base/foreign ccy (CCY1).
 */
export function shapeVarianceSwap(args: SwapArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: { kind: "varianceSwap", varianceSwap: { strikeVol: shapeStrikeVol(args.strikeVol) } },
  };
}

/** Shape a volatility-swap instrument from the cell arguments. */
export function shapeVolatilitySwap(args: SwapArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: { kind: "volatilitySwap", volatilitySwap: { strikeVol: shapeStrikeVol(args.strikeVol) } },
  };
}

/** The fully-parsed inputs an Asian CELNET.* function shapes into a request. */
export interface AsianArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly strike: string | number;
  readonly callPut: string;
  readonly notional: number;
  readonly averaging?: string | undefined;
  readonly observations?: number | undefined;
  readonly method?: string | undefined;
  readonly elapsedAvg?: number | undefined;
  readonly elapsedWeight?: number | undefined;
}

/**
 * Shape an arithmetic-average-rate Asian-option instrument from the cell
 * arguments. The strike must be an absolute level (an Asian has no delta-quoted
 * strike convention). For DISCRETE averaging `observations` must be `≥ 1`; for
 * CONTINUOUS it is ignored (encoded as 0, matching the contract). The seasoning
 * pair (`elapsedAvg`, `elapsedWeight`) prices an in-progress average; an absent
 * pair ⇒ a fresh average (weight 0). `elapsedWeight` must be `∈ [0, 1)`.
 */
export function shapeAsianOption(args: AsianArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  const strike = parseStrikeOrDelta(args.strike);
  if (strike.kind !== "strike") {
    throw new ShapingError(
      `Asian strike must be an absolute level (e.g. 1.10), not a delta \`${args.strike}\``,
    );
  }
  const averaging = parseAveragingStyle(args.averaging);
  const method = parseAsianMethod(args.method);
  let observations = 0;
  if (averaging === "DISCRETE") {
    const n = args.observations;
    if (n === undefined || !Number.isFinite(n) || !Number.isInteger(n) || n < 1) {
      throw new ShapingError(
        `DISCRETE averaging needs observations ≥ 1 (e.g. 12); got \`${args.observations}\``,
      );
    }
    observations = n;
  }
  const elapsedWeight = args.elapsedWeight ?? 0;
  if (!Number.isFinite(elapsedWeight) || elapsedWeight < 0 || elapsedWeight >= 1) {
    throw new ShapingError(
      `elapsed weight \`${elapsedWeight}\` out of range (expected 0 ≤ w < 1)`,
    );
  }
  const elapsedAvg = args.elapsedAvg ?? 0;
  if (!Number.isFinite(elapsedAvg) || elapsedAvg < 0) {
    throw new ShapingError(`elapsed average \`${elapsedAvg}\` must be finite and ≥ 0`);
  }
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: {
      kind: "asianOption",
      asianOption: {
        optionType: parseOptionType(args.callPut),
        strike: strike.strike,
        averaging,
        observations,
        method,
        elapsedAvg,
        elapsedWeight,
      },
    },
  };
}

// ---------------------------------------------------------------------------
// forward-start / cliquet / quanto shaping (structured products)
// ---------------------------------------------------------------------------

/**
 * Parse a quanto-payoff selector. Accepts `VANILLA`/`V`/`OPT` (the standard
 * call/put intrinsic) or `DIGITAL`/`DIG`/`D` (a fixed cash-or-nothing payoff),
 * case-insensitive. Empty/absent ⇒ `VANILLA` (the proto3 zero value).
 */
export function parseQuantoPayoff(raw: string | undefined): QuantoPayoff {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "VANILLA":
    case "V":
    case "OPT":
      return "VANILLA";
    case "DIGITAL":
    case "DIG":
    case "D":
      return "DIGITAL";
    default:
      throw new ShapingError(
        `invalid quanto payoff \`${raw}\` (expected VANILLA or DIGITAL)`,
      );
  }
}

/** Validate a strictly-positive proportional strike multiplier (moneyness). */
function shapeMoneyness(raw: number): number {
  if (!Number.isFinite(raw) || raw <= 0) {
    throw new ShapingError(`invalid moneyness \`${raw}\` (proportional strike > 0, e.g. 1.0)`);
  }
  return raw;
}

/** The fully-parsed inputs the CELNET.FORWARDSTART function shapes into a request. */
export interface ForwardStartArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly callPut: string;
  readonly moneyness: number;
  /** The reset (strike-fixing) date as a year fraction `∈ [0, expiryYears]`. */
  readonly reset: number;
  readonly notional: number;
}

/**
 * Shape a forward-start vanilla instrument from the cell arguments. The strike is
 * proportional (`moneyness × S(reset)`) — no delta-quoted strike applies — and the
 * reset `t₁` must lie in `[0, expiryYears]` (the contract's domain). `side` is
 * TWO_WAY (the cell reads a two-way market); the notional is in the base ccy.
 */
export function shapeForwardStart(args: ForwardStartArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  const moneyness = shapeMoneyness(args.moneyness);
  const reset = args.reset;
  if (!Number.isFinite(reset) || reset < 0 || reset > expiryYears) {
    throw new ShapingError(
      `reset \`${reset}\` out of range (expected 0 ≤ t₁ ≤ expiry ${expiryYears})`,
    );
  }
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: {
      kind: "forwardStart",
      forwardStart: { optionType: parseOptionType(args.callPut), moneyness, reset },
    },
  };
}

/** The fully-parsed inputs the CELNET.CLIQUET function shapes into a request. */
export interface CliquetArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly callPut: string;
  readonly moneyness: number;
  readonly periods: number;
  readonly notional: number;
  readonly localFloor?: number | undefined;
  readonly localCap?: number | undefined;
  readonly globalFloor?: number | undefined;
  readonly globalCap?: number | undefined;
  readonly mcPairs?: number | undefined;
  readonly mcSeed?: number | undefined;
}

/** Validate an optional presence-tracked clamp (floor/cap); absent ⇒ undefined. */
function shapeClamp(raw: number | undefined, what: string): number | undefined {
  if (raw === undefined) return undefined;
  if (!Number.isFinite(raw)) {
    throw new ShapingError(`invalid ${what} \`${raw}\` (must be a finite number)`);
  }
  return raw;
}

/**
 * Shape a cliquet (ratchet) instrument from the cell arguments. A *plain* ratchet
 * (no local/global floor or cap supplied) is the exact sum of forward-start legs
 * (closed form); supplying any clamp makes it a *clamped* cliquet priced by Monte
 * Carlo (its premium carries a standard error, surfaced honestly in the spill).
 * The four clamps are presence-tracked — an omitted clamp is unconstrained on that
 * side, never sent as `0`. A floor strictly above its cap is rejected. The
 * `mcPairs`/`mcSeed` knobs tune the clamped MC (`mcPairs 0` ⇒ the server default);
 * they are ignored by a plain ratchet.
 */
export function shapeCliquet(args: CliquetArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  const moneyness = shapeMoneyness(args.moneyness);
  const periods = args.periods;
  if (!Number.isFinite(periods) || !Number.isInteger(periods) || periods < 1) {
    throw new ShapingError(`cliquet periods must be an integer ≥ 1; got \`${args.periods}\``);
  }
  const localFloor = shapeClamp(args.localFloor, "local floor");
  const localCap = shapeClamp(args.localCap, "local cap");
  const globalFloor = shapeClamp(args.globalFloor, "global floor");
  const globalCap = shapeClamp(args.globalCap, "global cap");
  if (localFloor !== undefined && localCap !== undefined && localFloor > localCap) {
    throw new ShapingError(`local floor \`${localFloor}\` exceeds local cap \`${localCap}\``);
  }
  if (globalFloor !== undefined && globalCap !== undefined && globalFloor > globalCap) {
    throw new ShapingError(`global floor \`${globalFloor}\` exceeds global cap \`${globalCap}\``);
  }
  const mcPairs = args.mcPairs ?? 0;
  if (!Number.isFinite(mcPairs) || !Number.isInteger(mcPairs) || mcPairs < 0) {
    throw new ShapingError(`mc pairs must be a non-negative integer; got \`${args.mcPairs}\``);
  }
  const mcSeedRaw = args.mcSeed ?? 0;
  if (!Number.isFinite(mcSeedRaw) || !Number.isInteger(mcSeedRaw) || mcSeedRaw < 0) {
    throw new ShapingError(`mc seed must be a non-negative integer; got \`${args.mcSeed}\``);
  }
  const cliquet: Cliquet = {
    optionType: parseOptionType(args.callPut),
    moneyness,
    periods,
    mcPairs,
    mcSeed: BigInt(mcSeedRaw),
  };
  // Presence-tracked: attach a clamp only when supplied (mirrors the proto
  // `optional double` / the server's `opt_f64`), so the absent side is genuinely
  // unconstrained rather than floored/capped at zero.
  if (localFloor !== undefined) cliquet.localFloor = localFloor;
  if (localCap !== undefined) cliquet.localCap = localCap;
  if (globalFloor !== undefined) cliquet.globalFloor = globalFloor;
  if (globalCap !== undefined) cliquet.globalCap = globalCap;
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: { kind: "cliquet", cliquet },
  };
}

/** True iff a cliquet carries any clamp ⇒ it is priced by Monte Carlo (carries a std-error). */
export function cliquetIsMonteCarlo(c: Cliquet): boolean {
  return (
    c.localFloor !== undefined ||
    c.localCap !== undefined ||
    c.globalFloor !== undefined ||
    c.globalCap !== undefined
  );
}

/** The fully-parsed inputs the CELNET.QUANTO function shapes into a request. */
export interface QuantoArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly callPut: string;
  readonly strike: string | number;
  readonly notional: number;
  readonly conversionVol: number;
  readonly correlation: number;
  readonly payoff?: string | undefined;
}

/**
 * Shape a quanto-option instrument from the cell arguments. The strike must be an
 * absolute level (a quanto is struck in the underlying's quote terms, not a delta).
 * The conversion vol is the settlement-FX volatility (`≥ 0`) and the correlation
 * is the underlying↔settlement-FX correlation `∈ [-1, 1]` driving the quanto drift
 * adjustment. `payoff` selects VANILLA (default) or DIGITAL.
 */
export function shapeQuanto(args: QuantoArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  const strike = parseStrikeOrDelta(args.strike);
  if (strike.kind !== "strike") {
    throw new ShapingError(
      `quanto strike must be an absolute level (e.g. 1.10), not a delta \`${args.strike}\``,
    );
  }
  const conversionVol = args.conversionVol;
  if (!Number.isFinite(conversionVol) || conversionVol < 0 || conversionVol >= 5) {
    throw new ShapingError(
      `conversion vol \`${conversionVol}\` out of range (absolute vol ≥ 0, e.g. 0.09)`,
    );
  }
  const correlation = args.correlation;
  if (!Number.isFinite(correlation) || correlation < -1 || correlation > 1) {
    throw new ShapingError(`correlation \`${correlation}\` out of range (expected -1 ≤ ρ ≤ 1)`);
  }
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: {
      kind: "quanto",
      quanto: {
        payoff: parseQuantoPayoff(args.payoff),
        optionType: parseOptionType(args.callPut),
        strike: strike.strike,
        conversionVol,
        correlation,
      },
    },
  };
}

// ---------------------------------------------------------------------------
// TARF / accumulator / lookback shaping (path-dependent Monte-Carlo products)
// ---------------------------------------------------------------------------

/**
 * Build the equally-spaced fixing year-fractions a count-based ticket implies: `n`
 * points at `k/n` for `k = 1..=n`. This is bit-identical to the SDK's
 * `equal_fixing_years` (`crates/celnet-client/src/vocab.rs`), so an Excel TARF /
 * accumulator encodes the SAME `FixingSchedule.fixing_years` the SDK does and the
 * engine (which normalises the fractions to the instrument expiry, spacing fixings
 * equally over `[0, T]`) prices a cell identically to the SDK/CLI.
 */
function equalFixingYears(fixings: number): number[] {
  const n = Math.max(1, fixings);
  const years: number[] = [];
  for (let k = 1; k <= n; k += 1) years.push(k / n);
  return years;
}

/** Validate a fixing count: a positive integer (`≥ 1`). */
function shapeFixings(raw: number, what: string): number {
  if (!Number.isFinite(raw) || !Number.isInteger(raw) || raw < 1) {
    throw new ShapingError(`${what} must be an integer ≥ 1; got \`${raw}\``);
  }
  return raw;
}

/** Validate a non-negative integer MC pair count (`0` ⇒ the server default). */
function shapeMcPairs(raw: number | undefined): number {
  const v = raw ?? 0;
  if (!Number.isFinite(v) || !Number.isInteger(v) || v < 0) {
    throw new ShapingError(`mc pairs must be a non-negative integer; got \`${raw}\``);
  }
  return v;
}

/** Validate a non-negative integer MC seed and lift it to a bigint (`0` ⇒ default). */
function shapeMcSeed(raw: number | undefined): bigint {
  const v = raw ?? 0;
  if (!Number.isFinite(v) || !Number.isInteger(v) || v < 0) {
    throw new ShapingError(`mc seed must be a non-negative integer; got \`${raw}\``);
  }
  return BigInt(v);
}

/** Validate a non-negative per-fixing notional (`0`/absent ⇒ the unit leg, 1.0). */
function shapeFixingNotional(raw: number | undefined): number {
  if (raw === undefined) return 1.0;
  if (!Number.isFinite(raw) || raw <= 0) {
    throw new ShapingError(`fixing notional \`${raw}\` must be a positive number`);
  }
  return raw;
}

/**
 * Parse a TARF gap-risk redemption selector. Accepts `FULL_GAIN`/`FULL`/`F` (the
 * breaching fixing pays its full intrinsic gain — genuine gap exposure) or
 * `CAPPED_GAIN`/`CAPPED`/`C` (exact redemption, no overshoot), case-insensitive.
 * Empty/absent ⇒ `FULL_GAIN` (the proto3 zero value).
 */
export function parseTarfRedemption(raw: string | undefined): TarfRedemption {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "FULL_GAIN":
    case "FULL":
    case "F":
      return "FULL_GAIN";
    case "CAPPED_GAIN":
    case "CAPPED":
    case "C":
      return "CAPPED_GAIN";
    default:
      throw new ShapingError(
        `invalid TARF redemption \`${raw}\` (expected FULL_GAIN or CAPPED_GAIN)`,
      );
  }
}

/**
 * Parse an accumulator knock-out monitoring selector. Accepts `DISCRETE`/`DISC`/
 * `D` (tested at fixings) or `CONTINUOUS`/`CONT`/`C` (Brownian-bridge between
 * fixings), case-insensitive. Empty/absent ⇒ `DISCRETE` (the proto3 zero value).
 */
export function parseAccumulatorMonitoring(raw: string | undefined): AccumulatorMonitoring {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "DISCRETE":
    case "DISC":
    case "D":
      return "DISCRETE";
    case "CONTINUOUS":
    case "CONT":
    case "C":
      return "CONTINUOUS";
    default:
      throw new ShapingError(
        `invalid accumulator monitoring \`${raw}\` (expected DISCRETE or CONTINUOUS)`,
      );
  }
}

/**
 * Parse a lookback style selector. Accepts `FLOATING`/`FLOAT`/`FL` (settle against
 * the path extremum) or `FIXED`/`FIX`/`FX` (exercise against a fixed strike),
 * case-insensitive. Empty/absent ⇒ `FLOATING` (the proto3 zero value).
 */
export function parseLookbackStyle(raw: string | undefined): LookbackStyle {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "FLOATING":
    case "FLOAT":
    case "FL":
      return "FLOATING";
    case "FIXED":
    case "FIX":
    case "FX":
      return "FIXED";
    default:
      throw new ShapingError(
        `invalid lookback style \`${raw}\` (expected FLOATING or FIXED)`,
      );
  }
}

/**
 * Parse a lookback monitoring selector. Accepts `CONTINUOUS`/`CONT`/`C` (exact
 * closed form, no MC std-error) or `DISCRETE`/`DISC`/`D` (Monte-Carlo, carries a
 * std-error), case-insensitive. Empty/absent ⇒ `CONTINUOUS` (the proto3 zero
 * value).
 */
export function parseLookbackMonitoring(raw: string | undefined): LookbackMonitoring {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "CONTINUOUS":
    case "CONT":
    case "C":
      return "CONTINUOUS";
    case "DISCRETE":
    case "DISC":
    case "D":
      return "DISCRETE";
    default:
      throw new ShapingError(
        `invalid lookback monitoring \`${raw}\` (expected CONTINUOUS or DISCRETE)`,
      );
  }
}

/** The fully-parsed inputs the CELNET.TARF function shapes into a request. */
export interface TarfArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly callPut: string;
  readonly strike: string | number;
  readonly target: number;
  readonly leverage: number;
  readonly fixings: number;
  readonly notional: number;
  readonly redemption?: string | undefined;
  readonly fixingNotional?: number | undefined;
  readonly mcPairs?: number | undefined;
  readonly mcSeed?: number | undefined;
}

/**
 * Shape a Target-Redemption Forward from the cell arguments. The strike must be an
 * absolute level (a TARF fixes at a level, not a delta); `target > 0` is the
 * cumulative gain that redeems; `leverage ≥ 0` gears the adverse leg; `fixings ≥ 1`
 * is the (equally-spaced) fixing count. Always Monte-Carlo priced — the premium
 * carries a standard error, surfaced honestly in the spill. `side` is TWO_WAY.
 */
export function shapeTarf(args: TarfArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  const strike = parseStrikeOrDelta(args.strike);
  if (strike.kind !== "strike") {
    throw new ShapingError(
      `TARF strike must be an absolute level (e.g. 1.10), not a delta \`${args.strike}\``,
    );
  }
  if (!Number.isFinite(args.target) || args.target <= 0) {
    throw new ShapingError(`TARF target \`${args.target}\` must be a positive cumulative gain`);
  }
  if (!Number.isFinite(args.leverage) || args.leverage < 0) {
    throw new ShapingError(`TARF leverage \`${args.leverage}\` must be ≥ 0`);
  }
  const fixings = shapeFixings(args.fixings, "TARF fixings");
  const schedule: FixingSchedule = {
    fixingYears: equalFixingYears(fixings),
    fixingNotional: shapeFixingNotional(args.fixingNotional),
  };
  const tarf: Tarf = {
    optionType: parseOptionType(args.callPut),
    strike: strike.strike,
    target: args.target,
    leverage: args.leverage,
    redemption: parseTarfRedemption(args.redemption),
    schedule,
    mcPairs: shapeMcPairs(args.mcPairs),
    mcSeed: shapeMcSeed(args.mcSeed),
  };
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: { kind: "tarf", tarf },
  };
}

/** The fully-parsed inputs the CELNET.ACCUMULATOR function shapes into a request. */
export interface AccumulatorArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly pivot: string | number;
  readonly barrier: number;
  readonly leverage: number;
  readonly fixings: number;
  readonly notional: number;
  readonly monitoring?: string | undefined;
  readonly fixingNotional?: number | undefined;
  readonly mcPairs?: number | undefined;
  readonly mcSeed?: number | undefined;
}

/**
 * Shape an accumulator from the cell arguments. The pivot must be an absolute
 * level; the up-and-out `barrier` must be strictly above the pivot; `leverage ≥ 0`
 * gears the below-pivot leg; `fixings ≥ 1` is the (equally-spaced) fixing count.
 * Always Monte-Carlo priced — the premium carries a standard error. `side` is
 * TWO_WAY.
 */
export function shapeAccumulator(args: AccumulatorArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  const pivotSod = parseStrikeOrDelta(args.pivot);
  if (pivotSod.kind !== "strike") {
    throw new ShapingError(
      `accumulator pivot must be an absolute level (e.g. 1.10), not a delta \`${args.pivot}\``,
    );
  }
  const pivot = pivotSod.strike;
  if (!Number.isFinite(args.barrier) || args.barrier <= pivot) {
    throw new ShapingError(
      `accumulator barrier \`${args.barrier}\` must be strictly above the pivot \`${pivot}\``,
    );
  }
  if (!Number.isFinite(args.leverage) || args.leverage < 0) {
    throw new ShapingError(`accumulator leverage \`${args.leverage}\` must be ≥ 0`);
  }
  const fixings = shapeFixings(args.fixings, "accumulator fixings");
  const schedule: FixingSchedule = {
    fixingYears: equalFixingYears(fixings),
    fixingNotional: shapeFixingNotional(args.fixingNotional),
  };
  const accumulator: Accumulator = {
    pivot,
    barrier: args.barrier,
    leverage: args.leverage,
    monitoring: parseAccumulatorMonitoring(args.monitoring),
    schedule,
    mcPairs: shapeMcPairs(args.mcPairs),
    mcSeed: shapeMcSeed(args.mcSeed),
  };
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: { kind: "accumulator", accumulator },
  };
}

/** The fully-parsed inputs the CELNET.LOOKBACK function shapes into a request. */
export interface LookbackArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly callPut: string;
  readonly notional: number;
  readonly style?: string | undefined;
  readonly monitoring?: string | undefined;
  readonly strike?: string | number | undefined;
  readonly observations?: number | undefined;
  readonly mcPairs?: number | undefined;
  readonly mcSeed?: number | undefined;
}

/**
 * Shape a lookback option from the cell arguments. A FLOATING-strike lookback
 * settles against the path extremum (no strike — any supplied strike is rejected);
 * a FIXED-strike lookback requires an absolute strike level. CONTINUOUS monitoring
 * is exact closed form (no std-error); DISCRETE monitoring is Monte-Carlo (the
 * premium carries a std-error, surfaced honestly). `observations` applies only to
 * the DISCRETE variant. `side` is TWO_WAY.
 */
export function shapeLookback(args: LookbackArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  const style = parseLookbackStyle(args.style);
  const monitoring = parseLookbackMonitoring(args.monitoring);
  let strike = 0;
  if (style === "FIXED") {
    if (args.strike === undefined || args.strike === "") {
      throw new ShapingError("a FIXED-strike lookback requires an absolute strike level");
    }
    const sod = parseStrikeOrDelta(args.strike);
    if (sod.kind !== "strike") {
      throw new ShapingError(
        `lookback strike must be an absolute level (e.g. 1.10), not a delta \`${args.strike}\``,
      );
    }
    strike = sod.strike;
  } else if (args.strike !== undefined && args.strike !== "") {
    throw new ShapingError(
      `a FLOATING-strike lookback takes no strike (its strike is the path extremum); got \`${args.strike}\``,
    );
  }
  const observations = monitoring === "DISCRETE" ? shapeMcPairs(args.observations) : 0;
  const lookback: Lookback = {
    style,
    optionType: parseOptionType(args.callPut),
    monitoring,
    strike,
    observations,
    mcPairs: monitoring === "DISCRETE" ? shapeMcPairs(args.mcPairs) : 0,
    mcSeed: monitoring === "DISCRETE" ? shapeMcSeed(args.mcSeed) : 0n,
  };
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: { kind: "lookback", lookback },
  };
}

/**
 * True iff a lookback is priced by Monte-Carlo (its DISCRETE variant) ⇒ it carries
 * a std-error. The CONTINUOUS variant is exact closed form (no std-error). Mirrors
 * `cliquetIsMonteCarlo` — the function gates the std-error row on this, so a
 * closed-form lookback never surfaces a stray precision claim.
 */
export function lookbackIsMonteCarlo(l: Lookback): boolean {
  return l.monitoring === "DISCRETE";
}

// ---------------------------------------------------------------------------
// barrier / digital / touch shaping (already-contracted exotics, W4)
// ---------------------------------------------------------------------------

/**
 * Parse a barrier knock-direction selector. Accepts `KNOCK_IN`/`KI`/`IN` (the
 * option activates on a touch) or `KNOCK_OUT`/`KO`/`OUT` (it extinguishes on a
 * touch), case-insensitive. Empty/absent ⇒ `KNOCK_IN` (the proto3 zero value).
 */
export function parseBarrierKind(raw: string | undefined): BarrierKind {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "KNOCK_IN":
    case "KNOCK-IN":
    case "KNOCKIN":
    case "KI":
    case "IN":
      return "KNOCK_IN";
    case "KNOCK_OUT":
    case "KNOCK-OUT":
    case "KNOCKOUT":
    case "KO":
    case "OUT":
      return "KNOCK_OUT";
    default:
      throw new ShapingError(`invalid barrier kind \`${raw}\` (expected KNOCK_IN or KNOCK_OUT)`);
  }
}

/**
 * Parse a barrier-side selector (where the barrier sits relative to spot at
 * inception). Accepts `UP`/`U` or `DOWN`/`DN`/`D`, case-insensitive. Empty/absent
 * ⇒ `UP` (the proto3 zero value).
 */
export function parseBarrierSide(raw: string | undefined): BarrierSide {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "UP":
    case "U":
      return "UP";
    case "DOWN":
    case "DN":
    case "D":
      return "DOWN";
    default:
      throw new ShapingError(`invalid barrier side \`${raw}\` (expected UP or DOWN)`);
  }
}

/**
 * Parse a barrier/touch monitoring selector. Accepts `CONTINUOUS`/`CONT`/`C` (any
 * touch at any instant triggers) or `DISCRETE`/`DISC`/`D` (scheduled fixings only),
 * case-insensitive. Empty/absent ⇒ `CONTINUOUS` (the proto3 zero value, the OTC
 * standard for barriers/touches).
 */
export function parseMonitoringStyle(raw: string | undefined): MonitoringStyle {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "CONTINUOUS":
    case "CONT":
    case "C":
      return "CONTINUOUS";
    case "DISCRETE":
    case "DISC":
    case "D":
      return "DISCRETE";
    default:
      throw new ShapingError(`invalid monitoring \`${raw}\` (expected CONTINUOUS or DISCRETE)`);
  }
}

/**
 * Parse a touch-family selector. Accepts the trader-facing short names — `ONE_TOUCH`/
 * `OT`/`ONE`, `NO_TOUCH`/`NT`/`NO`, `DOUBLE_NO_TOUCH`/`DNT`, `DOUBLE_ONE_TOUCH`/`DOT`
 * — case-insensitive. Empty/absent ⇒ `ONE_TOUCH` (the proto3 zero value).
 */
export function parseTouchKind(raw: string | undefined): TouchKind {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "ONE_TOUCH":
    case "ONE-TOUCH":
    case "ONETOUCH":
    case "OT":
    case "ONE":
      return "ONE_TOUCH";
    case "NO_TOUCH":
    case "NO-TOUCH":
    case "NOTOUCH":
    case "NT":
    case "NO":
      return "NO_TOUCH";
    case "DOUBLE_NO_TOUCH":
    case "DOUBLE-NO-TOUCH":
    case "DNT":
      return "DOUBLE_NO_TOUCH";
    case "DOUBLE_ONE_TOUCH":
    case "DOUBLE-ONE-TOUCH":
    case "DOT":
      return "DOUBLE_ONE_TOUCH";
    default:
      throw new ShapingError(
        `invalid touch kind \`${raw}\` (expected ONE_TOUCH, NO_TOUCH, DNT or DOT)`,
      );
  }
}

/**
 * Parse a digital settlement-style selector. Accepts `CASH_OR_NOTHING`/`CASH`/`C`
 * (a fixed cash payout) or `ASSET_OR_NOTHING`/`ASSET`/`A` (one unit of the asset),
 * case-insensitive. Empty/absent ⇒ `CASH_OR_NOTHING` (the proto3 zero value).
 */
export function parseDigitalStyle(raw: string | undefined): DigitalStyle {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "CASH_OR_NOTHING":
    case "CASH-OR-NOTHING":
    case "CASH":
    case "C":
      return "CASH_OR_NOTHING";
    case "ASSET_OR_NOTHING":
    case "ASSET-OR-NOTHING":
    case "ASSET":
    case "A":
      return "ASSET_OR_NOTHING";
    default:
      throw new ShapingError(
        `invalid digital style \`${raw}\` (expected CASH_OR_NOTHING or ASSET_OR_NOTHING)`,
      );
  }
}

/** Validate an optional non-negative rebate (`0`/absent ⇒ no rebate). */
function shapeRebate(raw: number | undefined): number {
  if (raw === undefined) return 0;
  if (!Number.isFinite(raw) || raw < 0) {
    throw new ShapingError(`rebate \`${raw}\` must be a finite, non-negative number`);
  }
  return raw;
}

/** Validate an absolute barrier level (strictly positive). */
function shapeBarrierLevel(raw: number, what: string): number {
  if (!Number.isFinite(raw) || raw <= 0) {
    throw new ShapingError(`${what} \`${raw}\` must be a positive level (quote per 1 unit of base)`);
  }
  return raw;
}

/** The fully-parsed inputs the CELNET.BARRIER function shapes (single OR double). */
export interface BarrierArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly strikeOrDelta: string | number;
  readonly callPut: string;
  readonly notional: number;
  readonly kind?: string | undefined;
  /**
   * The barrier level. A single-barrier ticket supplies `barrier` (with `side`);
   * a double-barrier ticket supplies BOTH `barrier` (the lower) and `upperBarrier`.
   */
  readonly barrier: number;
  /** The upper barrier — supplying it makes the ticket a DOUBLE-barrier. */
  readonly upperBarrier?: number | undefined;
  /** UP/DOWN — single-barrier only (a double barrier brackets spot, so no side). */
  readonly side?: string | undefined;
  readonly rebate?: number | undefined;
  readonly monitoring?: string | undefined;
}

/**
 * Shape a barrier instrument from the cell arguments. ONE function covers both the
 * single- and double-barrier products (the §4 spec: "single + double via params"):
 * supplying `upperBarrier` selects the DOUBLE-barrier product (the supplied
 * `barrier` is then the lower barrier and `side` is rejected — a double barrier
 * brackets spot and has no single side); omitting it selects the SINGLE-barrier
 * product with the given `side`. The vanilla payoff (call/put + strike) carries the
 * exotic; `side` on the instrument is TWO_WAY (the cell reads a market).
 */
export function shapeBarrier(args: BarrierArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  const vanilla = {
    optionType: parseOptionType(args.callPut),
    strike: parseStrikeOrDelta(args.strikeOrDelta),
  };
  const kind = parseBarrierKind(args.kind);
  const rebate = shapeRebate(args.rebate);
  const monitoring = parseMonitoringStyle(args.monitoring);
  const base = {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
  };
  if (args.upperBarrier !== undefined) {
    // Double-barrier: `barrier` is the lower, `upperBarrier` the upper; reject a
    // single `side` (a double barrier brackets spot — it has no single side).
    if (args.side !== undefined && String(args.side).trim() !== "") {
      throw new ShapingError(
        "a double-barrier (upperBarrier supplied) takes no UP/DOWN side; omit it",
      );
    }
    const lower = shapeBarrierLevel(args.barrier, "lower barrier");
    const upper = shapeBarrierLevel(args.upperBarrier, "upper barrier");
    if (upper <= lower) {
      throw new ShapingError(
        `upper barrier \`${upper}\` must be strictly above the lower barrier \`${lower}\``,
      );
    }
    const doubleBarrier: DoubleBarrier = {
      vanilla,
      kind,
      lowerBarrier: lower,
      upperBarrier: upper,
      rebate,
      monitoring,
    };
    return { ...base, product: { kind: "doubleBarrier", doubleBarrier } };
  }
  const singleBarrier: SingleBarrier = {
    vanilla,
    kind,
    side: parseBarrierSide(args.side),
    barrier: shapeBarrierLevel(args.barrier, "barrier"),
    rebate,
    monitoring,
  };
  return { ...base, product: { kind: "singleBarrier", singleBarrier } };
}

/** The fully-parsed inputs the CELNET.DIGITAL function shapes into a request. */
export interface DigitalArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly strike: string | number;
  readonly callPut: string;
  readonly notional: number;
  readonly style?: string | undefined;
  readonly payout?: number | undefined;
}

/**
 * Shape a digital (binary) option from the cell arguments. The strike must be an
 * absolute level (a digital is struck at a level). `callPut` selects the above-
 * strike (call) vs below-strike (put) payoff; `style` cash-or-nothing (default) vs
 * asset-or-nothing; `payout` the fixed cash payout (`0`/absent ⇒ unit payout for a
 * cash digital). `side` on the instrument is TWO_WAY.
 */
export function shapeDigital(args: DigitalArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  const sod = parseStrikeOrDelta(args.strike);
  if (sod.kind !== "strike") {
    throw new ShapingError(
      `digital strike must be an absolute level (e.g. 1.10), not a delta \`${args.strike}\``,
    );
  }
  const payout = args.payout ?? 0;
  if (!Number.isFinite(payout) || payout < 0) {
    throw new ShapingError(`digital payout \`${payout}\` must be a finite, non-negative amount`);
  }
  const digital: Digital = {
    optionType: parseOptionType(args.callPut),
    strike: sod.strike,
    style: parseDigitalStyle(args.style),
    payout,
  };
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: { kind: "digital", digital },
  };
}

/** The fully-parsed inputs the CELNET.TOUCH function shapes into a request. */
export interface TouchArgs {
  readonly pair: string;
  readonly tenor: string;
  readonly notional: number;
  readonly kind?: string | undefined;
  /** The (lower / sole) barrier level. */
  readonly barrier: number;
  /** The upper barrier — required for the DOUBLE (DNT/DOT) kinds; rejected for the single kinds. */
  readonly upperBarrier?: number | undefined;
  readonly rebate?: number | undefined;
  readonly monitoring?: string | undefined;
}

/** True iff a touch kind is a double-barrier structure (DNT / DOT). */
function isDoubleTouch(kind: TouchKind): boolean {
  return kind === "DOUBLE_NO_TOUCH" || kind === "DOUBLE_ONE_TOUCH";
}

/**
 * Shape a touch structure from the cell arguments. The single-barrier kinds
 * (ONE_TOUCH / NO_TOUCH) use `barrier` as the sole level and reject an
 * `upperBarrier`; the double kinds (DOUBLE_NO_TOUCH / DOUBLE_ONE_TOUCH) require
 * BOTH `barrier` (the lower) and a strictly-greater `upperBarrier`. A touch has no
 * vanilla payoff (it pays the `rebate` on the touch condition); `side` on the
 * instrument is TWO_WAY.
 */
export function shapeTouch(args: TouchArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  const kind = parseTouchKind(args.kind);
  const lower = shapeBarrierLevel(args.barrier, "barrier");
  let upper = 0;
  if (isDoubleTouch(kind)) {
    if (args.upperBarrier === undefined) {
      throw new ShapingError(`a ${kind} requires both a lower and an upper barrier`);
    }
    upper = shapeBarrierLevel(args.upperBarrier, "upper barrier");
    if (upper <= lower) {
      throw new ShapingError(
        `upper barrier \`${upper}\` must be strictly above the lower barrier \`${lower}\``,
      );
    }
  } else if (args.upperBarrier !== undefined) {
    throw new ShapingError(
      `a ${kind} is single-barrier and takes no upper barrier; use DNT/DOT for a double structure`,
    );
  }
  const touch: Touch = {
    kind,
    lowerBarrier: lower,
    upperBarrier: upper,
    rebate: shapeRebate(args.rebate),
    monitoring: parseMonitoringStyle(args.monitoring),
  };
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: { kind: "touch", touch },
  };
}

// ---------------------------------------------------------------------------
// dynamic-array formatting (spill geometries)
// ---------------------------------------------------------------------------

/** A cell matrix Excel spills: rows of (string | number) cells. */
export type SpillMatrix = (string | number)[][];

/**
 * The 13-Greek vector in the canonical contract order (the celnet-vanilla set,
 * `Greeks` field order in `celnet.proto` / contract.ts). `price` is the premium
 * and is surfaced separately by CELNET.PRICE; CELNET.GREEKS spills the 13 risk
 * Greeks (delta_spot … color) so the spill is exactly 13 rows, as the design says.
 */
export const GREEK_ROWS: readonly { readonly label: string; readonly key: keyof Greeks }[] = [
  { label: "delta_spot", key: "deltaSpot" },
  { label: "delta_forward", key: "deltaForward" },
  { label: "gamma", key: "gamma" },
  { label: "vega", key: "vega" },
  { label: "theta", key: "theta" },
  { label: "rho_dom", key: "rhoDom" },
  { label: "rho_for", key: "rhoFor" },
  { label: "vanna", key: "vanna" },
  { label: "volga", key: "volga" },
  { label: "charm", key: "charm" },
  { label: "speed", key: "speed" },
  { label: "zomma", key: "zomma" },
  { label: "color", key: "color" },
];

/**
 * Format the convention footer string stamped on every spill — the resolved
 * convention + surface_version + capture time (docs §3.4). Vendor-neutral, terse.
 */
export function conventionFooter(
  conv: Conventions,
  surfaceVersion: bigint | undefined,
  epochNanos: bigint,
): string {
  const ver = surfaceVersion === undefined ? "live" : `v${surfaceVersion}`;
  const ms = Number(epochNanos / 1_000_000n);
  const iso = epochNanos === 0n ? "—" : new Date(ms).toISOString();
  return (
    `conv: Δ ${conv.deltaConvention} | ATM ${conv.atmConvention} | ` +
    `${conv.premiumStyle} | ${conv.dayCount} | ${conv.settlement} | ` +
    `surface ${ver} | t=${iso}`
  );
}

/**
 * Format CELNET.GREEKS as a vertical 13×2 spill `[name, value]`, plus a footer
 * row carrying the convention transparency. Returns a `(rows)×2` matrix.
 */
export function formatGreeksSpill(
  greeks: Greeks,
  conv: Conventions,
  surfaceVersion: bigint | undefined,
  epochNanos: bigint,
): SpillMatrix {
  const rows: SpillMatrix = GREEK_ROWS.map((r) => [r.label, greeks[r.key]]);
  rows.push([conventionFooter(conv, surfaceVersion, epochNanos), ""]);
  return rows;
}

/** A decoded smile point for surface formatting (delta, vol). */
export interface SurfacePoint {
  readonly delta: number;
  readonly vol: number;
}

/**
 * Format a single-tenor smile row as a spill: a header row of delta pillars and a
 * value row of vols, plus an arb-status + convention footer. Points are sorted by
 * signed delta (puts negative … calls positive) for a stable left-to-right grid.
 */
export function formatSmileSpill(
  points: readonly SurfacePoint[],
  arbFree: boolean,
  conv: Conventions,
  surfaceVersion: bigint | undefined,
  epochNanos: bigint,
): SpillMatrix {
  const sorted = [...points].sort((a, b) => a.delta - b.delta);
  const header: (string | number)[] = ["delta", ...sorted.map((p) => p.delta)];
  const vols: (string | number)[] = ["vol", ...sorted.map((p) => p.vol)];
  const footer: (string | number)[] = [
    `${arbFree ? "arb-free" : "ARB!"} | ${conventionFooter(conv, surfaceVersion, epochNanos)}`,
  ];
  return [header, vols, footer];
}

/**
 * Format a multi-tenor surface cube as a spill grid: a header row of delta
 * pillars, one row per tenor `[tenorYears, vol@pillar…]`, plus a footer. The
 * pillar set is the union of deltas across tenors, sorted ascending; a missing
 * (tenor, pillar) cell is left blank.
 */
export function formatSurfaceCubeSpill(
  tenors: readonly { readonly tenorYears: number; readonly points: readonly SurfacePoint[] }[],
  conv: Conventions,
  surfaceVersion: bigint | undefined,
  epochNanos: bigint,
): SpillMatrix {
  const pillarSet = new Set<number>();
  for (const t of tenors) for (const p of t.points) pillarSet.add(p.delta);
  const pillars = [...pillarSet].sort((a, b) => a - b);
  const header: (string | number)[] = ["tenor\\delta", ...pillars];
  const rows: SpillMatrix = [header];
  for (const t of tenors) {
    const byDelta = new Map(t.points.map((p) => [p.delta, p.vol]));
    const row: (string | number)[] = [t.tenorYears];
    for (const d of pillars) {
      const vol = byDelta.get(d);
      row.push(vol === undefined ? "" : vol);
    }
    rows.push(row);
  }
  rows.push([conventionFooter(conv, surfaceVersion, epochNanos)]);
  return rows;
}

/** The decoded fields a CELNET.RFQ spill renders. */
export interface RfqResult {
  readonly bid: number;
  readonly offer: number;
  readonly quoteId: bigint;
  readonly validUntilNanos: bigint;
  readonly conventions: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}

/**
 * Format CELNET.RFQ as a 1×4 horizontal spill `[bid, offer, quoteId, validUntil]`
 * followed by a convention footer row. `quoteId`/`validUntil` are rendered as
 * strings to avoid JS number precision loss on the 64-bit wire ids.
 */
export function formatRfqSpill(r: RfqResult): SpillMatrix {
  const validIso = r.validUntilNanos === 0n
    ? "—"
    : new Date(Number(r.validUntilNanos / 1_000_000n)).toISOString();
  return [
    [r.bid, r.offer, r.quoteId.toString(), validIso],
    [conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)],
  ];
}

/** The decoded fields a CELNET.VARSWAP spill renders (fair variance + fair vol). */
export interface VarSwapResult {
  /** The fair variance strike `K_var` (the server's `resolved_strike` / `greeks.price`). */
  readonly fairVariance: number;
  readonly conventions: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}

/**
 * Format CELNET.VARSWAP as a labelled 2×2 spill — `["fair_variance", K_var]` and
 * `["fair_vol", √K_var]` — followed by a convention footer. The fair vol is the
 * √ of the fair variance the server returns; both are shown so the desk reads the
 * variance strike AND its vol-equivalent without a hidden √.
 */
export function formatVarSwapSpill(r: VarSwapResult): SpillMatrix {
  const fairVol = r.fairVariance >= 0 ? Math.sqrt(r.fairVariance) : Number.NaN;
  return [
    ["fair_variance", r.fairVariance],
    ["fair_vol", fairVol],
    [conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)],
  ];
}

/** The decoded fields a CELNET.VOLSWAP spill renders (the convexity-adjusted fair vol). */
export interface VolSwapResult {
  /** The fair volatility strike `K_vol` (the server's `resolved_strike` / `greeks.price`). */
  readonly fairVol: number;
  readonly conventions: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}

/**
 * Format CELNET.VOLSWAP as a labelled 1×2 spill `["fair_vol", K_vol]` followed by
 * a convention footer. `K_vol` is the convexity-adjusted fair volatility strike,
 * strictly below `√K_var` for any non-degenerate smile.
 */
export function formatVolSwapSpill(r: VolSwapResult): SpillMatrix {
  return [
    ["fair_vol", r.fairVol],
    [conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)],
  ];
}

/** The decoded fields a CELNET.ASIAN spill renders (the discounted option PV + Greeks). */
export interface AsianResult {
  /** The discounted Asian-option premium (the server's `greeks.price`). */
  readonly premium: number;
  readonly greeks: Greeks;
  readonly conventions: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}

/**
 * Format CELNET.ASIAN as a labelled spill: `["premium", PV]`, then the 13 risk
 * Greeks the server returns (the same set/order as CELNET.GREEKS), then a
 * convention footer. The Asian carries a genuine discounted PV + the full FD
 * Greek set (unlike the swaps, whose headline is a fair strike).
 */
export function formatAsianSpill(r: AsianResult): SpillMatrix {
  const rows: SpillMatrix = [["premium", r.premium]];
  for (const g of GREEK_ROWS) rows.push([g.label, r.greeks[g.key]]);
  rows.push([conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)]);
  return rows;
}

/** The decoded fields a CELNET.FORWARDSTART spill renders (discounted PV + Greeks). */
export interface ForwardStartResult {
  /** The discounted forward-start premium (the server's `greeks.price`). */
  readonly premium: number;
  readonly greeks: Greeks;
  readonly conventions: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}

/**
 * Format CELNET.FORWARDSTART as a labelled spill: `["premium", PV]`, then the 13
 * risk Greeks the server returns (the same set/order as CELNET.GREEKS), then a
 * convention footer. The forward-start is a closed-form (dual-carry strike-reset)
 * price — exact, so no standard-error row.
 */
export function formatForwardStartSpill(r: ForwardStartResult): SpillMatrix {
  const rows: SpillMatrix = [["premium", r.premium]];
  for (const g of GREEK_ROWS) rows.push([g.label, r.greeks[g.key]]);
  rows.push([conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)]);
  return rows;
}

/** The decoded fields a CELNET.CLIQUET spill renders (discounted PV + optional MC stderr + Greeks). */
export interface CliquetResult {
  /** The discounted cliquet premium (the server's `greeks.price`). */
  readonly premium: number;
  /**
   * The Monte-Carlo standard error of the premium (the server's `price_std_error`),
   * present ONLY for a clamped cliquet (MC-priced); `undefined` for a plain ratchet
   * (exact closed-form sum of legs). Surfaced honestly so a clamped premium is never
   * mistaken for closed-form precision.
   */
  readonly stdError: number | undefined;
  readonly greeks: Greeks;
  readonly conventions: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}

/**
 * Format CELNET.CLIQUET as a labelled spill: `["premium", PV]`, then — ONLY when
 * the server returned a Monte-Carlo standard error (a clamped cliquet) — a
 * `["std_error", σ̄]` row, then the 13 risk Greeks, then a convention footer. A
 * plain ratchet (closed-form, exact) omits the std-error row entirely, so the
 * geometry is honest about whether the headline carries MC noise.
 */
export function formatCliquetSpill(r: CliquetResult): SpillMatrix {
  const rows: SpillMatrix = [["premium", r.premium]];
  if (r.stdError !== undefined) rows.push(["std_error", r.stdError]);
  for (const g of GREEK_ROWS) rows.push([g.label, r.greeks[g.key]]);
  rows.push([conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)]);
  return rows;
}

/** The decoded fields a CELNET.QUANTO spill renders (discounted PV + Greeks). */
export interface QuantoResult {
  /** The discounted quanto-option premium (the server's `greeks.price`). */
  readonly premium: number;
  readonly greeks: Greeks;
  readonly conventions: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}

/**
 * Format CELNET.QUANTO as a labelled spill: `["premium", PV]`, then the 13 risk
 * Greeks the server returns (the same set/order as CELNET.GREEKS), then a
 * convention footer. The quanto is a closed-form (quanto-adjusted) price — exact,
 * so no standard-error row.
 */
export function formatQuantoSpill(r: QuantoResult): SpillMatrix {
  const rows: SpillMatrix = [["premium", r.premium]];
  for (const g of GREEK_ROWS) rows.push([g.label, r.greeks[g.key]]);
  rows.push([conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)]);
  return rows;
}

/**
 * The decoded fields a barrier / digital / touch spill renders (discounted PV +
 * the 13 Greeks). These already-contracted exotics are priced by the server's
 * exact/PDE closed forms — NO Monte-Carlo standard error (the server stamps
 * `std_error: None` for all four), so the spill carries no std-error row, exactly
 * like the forward-start / quanto closed-form spills.
 */
export interface ExoticPremiumResult {
  /** The discounted premium (the server's `greeks.price`). */
  readonly premium: number;
  readonly greeks: Greeks;
  readonly conventions: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}

/**
 * Format a barrier / digital / touch product as a labelled spill: `["premium", PV]`,
 * then the 13 risk Greeks the server returns (the same set/order as CELNET.GREEKS),
 * then a convention footer. Shared by CELNET.BARRIER / DIGITAL / TOUCH — these
 * products are priced exactly (closed-form / PDE), so there is no standard-error
 * row (mirrors `formatForwardStartSpill` / `formatQuantoSpill`).
 */
export function formatExoticPremiumSpill(r: ExoticPremiumResult): SpillMatrix {
  const rows: SpillMatrix = [["premium", r.premium]];
  for (const g of GREEK_ROWS) rows.push([g.label, r.greeks[g.key]]);
  rows.push([conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)]);
  return rows;
}

/**
 * The decoded fields a path-dependent product spill renders (discounted PV, the
 * optional MC standard error, the 13 Greeks, and the convention footer). Shared by
 * CELNET.TARF / ACCUMULATOR / LOOKBACK: the `stdError` is present ONLY for a
 * Monte-Carlo-priced product (always for TARF/accumulator; for a DISCRETE
 * lookback) and `undefined` for the exact CONTINUOUS lookback, so a cell never
 * mistakes a closed-form price for an MC estimate (or vice-versa).
 */
export interface PathDependentResult {
  /** The discounted premium (the server's `greeks.price`). */
  readonly premium: number;
  /**
   * The Monte-Carlo standard error of the premium (the server's `price_std_error`),
   * present ONLY for an MC-priced product; `undefined` for the exact closed-form
   * (continuous lookback) case. Surfaced honestly so an MC premium is never
   * mistaken for closed-form precision.
   */
  readonly stdError: number | undefined;
  readonly greeks: Greeks;
  readonly conventions: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}

/**
 * Format a path-dependent product (TARF / accumulator / lookback) as a labelled
 * spill: `["premium", PV]`, then — ONLY when the server returned a Monte-Carlo
 * standard error — a `["std_error", σ̄]` row, then the 13 risk Greeks, then a
 * convention footer. The TARF and accumulator are always MC (so always carry the
 * std-error row); the lookback carries it for the DISCRETE variant only (the
 * CONTINUOUS variant is exact closed form and omits it). The geometry is therefore
 * honest about whether the headline carries MC noise. Shared by CELNET.TARF /
 * ACCUMULATOR / LOOKBACK (mirrors `formatCliquetSpill`).
 */
export function formatPathDependentSpill(r: PathDependentResult): SpillMatrix {
  const rows: SpillMatrix = [["premium", r.premium]];
  if (r.stdError !== undefined) rows.push(["std_error", r.stdError]);
  for (const g of GREEK_ROWS) rows.push([g.label, r.greeks[g.key]]);
  rows.push([conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)]);
  return rows;
}

/**
 * Format a calibrated smile (the result of a model-selected `mark_surface`) as a
 * spill: a delta-pillar header row, a vol row, then a footer carrying the marked
 * `surface_version`, the model provenance (read from the server's
 * `arbitrage.note`, which the server stamps as `model=<family>`), the arb status,
 * and the convention transparency. The `model` argument is the model the cell
 * REQUESTED; the footer prefers the server-reported provenance when present so a
 * silent server default is visible, never assumed.
 */
export function formatCalibratedSmileSpill(args: {
  readonly points: readonly SurfacePoint[];
  readonly requestedModel: SmileModel;
  readonly providerNote: string;
  readonly arbFree: boolean;
  readonly conv: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}): SpillMatrix {
  const sorted = [...args.points].sort((a, b) => a.delta - b.delta);
  const header: (string | number)[] = ["delta", ...sorted.map((p) => p.delta)];
  const vols: (string | number)[] = ["vol", ...sorted.map((p) => p.vol)];
  // The server stamps the model used as `model=<family>` inside the arbitrage
  // note (the frozen contract has no echo field). Surface it verbatim when present
  // so the trader sees exactly what was calibrated, falling back to the requested
  // model name when the note carries no provenance token.
  const noteModel = /model=([A-Za-z0-9_-]+)/.exec(args.providerNote)?.[1];
  const modelLabel = noteModel ?? args.requestedModel;
  const footer: (string | number)[] = [
    `model ${modelLabel} | ${args.arbFree ? "arb-free" : "ARB!"} | ` +
      conventionFooter(args.conv, args.surfaceVersion, args.epochNanos),
  ];
  return [header, vols, footer];
}

/** A decoded market-series tick for a single trend cell render. */
export interface SeriesCellInput {
  readonly value: number;
  readonly observable: MarketObservable;
  readonly baselined: boolean;
  readonly epochNanos: bigint;
}

/**
 * Render a streamed market-series tick to a single cell string. Vols are shown in
 * vol points (×100, two-decimal); rates (SPOT/FORWARD) in full precision. An
 * un-baselined / non-finite value renders an explicit waiting marker rather than a
 * frozen number (docs §5: never show stale-as-live).
 */
export function formatSeriesCell(t: SeriesCellInput): string {
  if (!t.baselined || !Number.isFinite(t.value)) return "… (awaiting)";
  const isVol = t.observable === "ATM_VOL" || t.observable === "RISK_REVERSAL" || t.observable === "BUTTERFLY";
  return isVol ? `${(t.value * 100).toFixed(2)}v` : t.value.toFixed(5);
}

/** The decoded fields a CELNET.MARK status spill renders (two-phase staging). */
export interface MarkStatus {
  readonly status: "PENDING" | "COMMITTED" | "REJECTED";
  readonly stagingId: string;
  readonly surfaceVersionAfter: bigint | undefined;
  readonly detail: string;
}

/** Format CELNET.MARK as a 1×3 status spill `[status, surfaceVersionAfter, detail]`. */
export function formatMarkStatusSpill(m: MarkStatus): SpillMatrix {
  return [[m.status, m.surfaceVersionAfter === undefined ? m.stagingId : `v${m.surfaceVersionAfter}`, m.detail]];
}

// ---------------------------------------------------------------------------
// risk spill formatting (RiskService — server-side hierarchical risk)
// ---------------------------------------------------------------------------

/**
 * The additive node columns surfaced by CELNET.RISK, in a stable order: the
 * numeraire-collapsed measures the desk reads. `delta_numeraire`/`vega_numeraire`/
 * `premium_numeraire` are in the reporting numeraire (the §2.2/§2.3 collapse), so a
 * cell is NEVER in native premium units — the old "native premium units" caveat is
 * resolved (aggregation goes through celnet-risk-normalize server-side).
 */
const RISK_MEASURE_COLUMNS: readonly { readonly label: string; readonly key: keyof AdditiveRisk }[] =
  [
    { label: "delta", key: "deltaNumeraire" },
    { label: "gamma", key: "gamma" },
    { label: "vega", key: "vegaNumeraire" },
    { label: "theta", key: "theta" },
    { label: "vanna", key: "vanna" },
    { label: "volga", key: "volga" },
    { label: "premium", key: "premiumNumeraire" },
  ];

/** A footer summarising the reporting numeraire + roll-up dimension (transparency). */
function riskFooter(numeraire: string, dimension: RiskDimension, nodeCount: number): string {
  return `roll-up by ${dimension} | reporting ${numeraire} | ${nodeCount} node${nodeCount === 1 ? "" : "s"} | server-aggregated (celnet-risk-normalize)`;
}

/**
 * Format CELNET.RISK as a node grid: a header row
 * `[group, count, delta, gamma, vega, theta, vanna, volga, premium, VaR, ES, curvature]`,
 * one row per rolled-up node (the value column for each additive measure plus the
 * presence-tracked non-additive measures rendered as a blank when absent — never a
 * spurious zero), then a numeraire/dimension footer. Aggregation is the server's;
 * this only lays the server-returned node tree out in cells.
 */
export function formatRiskSpill(
  dimension: RiskDimension,
  numeraire: string,
  nodes: readonly RiskNode[],
): SpillMatrix {
  const header: (string | number)[] = [
    "group",
    "count",
    ...RISK_MEASURE_COLUMNS.map((c) => c.label),
    "VaR",
    "ES",
    "curvature",
  ];
  const rows: SpillMatrix = [header];
  for (const node of nodes) {
    const row: (string | number)[] = [
      // FIRM rolls to the apex (group 0); show "FIRM" rather than a bare 0 so the
      // single-node firm view reads honestly.
      node.dimension === "FIRM" ? "FIRM" : node.group.toString(),
      node.positionCount,
      ...RISK_MEASURE_COLUMNS.map((c) => node.additive[c.key] as number),
      node.nonadditive.var ?? "",
      node.nonadditive.es ?? "",
      node.nonadditive.curvatureSpot ?? "",
    ];
    rows.push(row);
  }
  rows.push([riskFooter(numeraire, dimension, nodes.length)]);
  return rows;
}

/**
 * Format CELNET.POSITIONS as a leaf grid: a header row
 * `[position_id, book, trader, pair, type, notional, strike, vol, t, surface, attribution]`,
 * one row per entitled open position, then a count footer. The position id /
 * surface version are rendered as strings to avoid 64-bit precision loss.
 */
export function formatPositionsSpill(positions: readonly RiskPosition[]): SpillMatrix {
  const header: (string | number)[] = [
    "position_id",
    "book",
    "trader",
    "pair",
    "type",
    "notional",
    "strike",
    "vol",
    "t",
    "surface",
    "attribution",
  ];
  const rows: SpillMatrix = [header];
  for (const p of positions) {
    rows.push([
      p.positionId.toString(),
      p.org.book,
      p.org.trader,
      `${p.org.ccyPair.base}${p.org.ccyPair.quote}`,
      p.optionType,
      p.notionalBase,
      p.inputs.strike,
      p.inputs.vol,
      p.inputs.t,
      `v${p.surfaceVersion}`,
      p.attribution ?? "—",
    ]);
  }
  if (positions.length === 0) {
    // Honest empty-state: no entitled open positions (a real, not error, state).
    rows.push(["(no entitled open positions)"]);
  } else {
    rows.push([`${positions.length} position${positions.length === 1 ? "" : "s"}`]);
  }
  return rows;
}

/**
 * Format CELNET.LIMITS as a utilization grid: a header row
 * `[metric, cap, exposure, ratio, status, enforcement, headroom]`, one row per
 * limit at the scope, then a worst-RAG / hard-breach footer. A `VEGA_BUCKET` /
 * `TENOR_VEGA` metric annotates its pillar/tenor on the metric label so the row is
 * self-describing.
 */
export function formatLimitsSpill(
  scopeLabel: string,
  limits: readonly LimitUtilization[],
  worst: string,
  hardBreach: boolean,
): SpillMatrix {
  const header: (string | number)[] = [
    "metric",
    "cap",
    "exposure",
    "ratio",
    "status",
    "enforcement",
    "headroom",
  ];
  const rows: SpillMatrix = [header];
  for (const l of limits) {
    let metricLabel: string = l.metric;
    if (l.metric === "VEGA_BUCKET") {
      metricLabel = `VEGA_BUCKET(${l.vegaPillar.tenorDays}d,${l.vegaPillar.deltaBp}bp)`;
    } else if (l.metric === "TENOR_VEGA") {
      metricLabel = `TENOR_VEGA(${l.tenorDays}d)`;
    }
    rows.push([
      metricLabel,
      l.cap,
      l.exposure,
      l.ratio,
      l.status,
      l.enforcement,
      l.headroom,
    ]);
  }
  if (limits.length === 0) {
    rows.push([`scope ${scopeLabel}: no limits configured`]);
  } else {
    rows.push([
      `scope ${scopeLabel} | worst ${worst}${hardBreach ? " | HARD BREACH" : ""}`,
    ]);
  }
  return rows;
}

// ---------------------------------------------------------------------------
// streaming dedup / coalescing — the multiplex-key for shared subscriptions
// ---------------------------------------------------------------------------

/**
 * The coalescing key for a streamed cell: identical-argument cells share ONE
 * subscription on the single multiplexed session (the add-in reference-counts
 * subscribers, docs §3.2). The key is the canonical, order-stable serialization
 * of (instrument, conventions) so two cells with equal arguments — regardless of
 * whitespace/case in what the trader typed — map to the same key.
 */
export function subscriptionKey(instrument: Instrument, conventions: Conventions): string {
  return JSON.stringify([canonicalInstrument(instrument), canonicalConventions(conventions)]);
}

function canonicalInstrument(i: Instrument): unknown {
  const product = canonicalProduct(i.product);
  return {
    base: i.pair.base,
    quote: i.pair.quote,
    tu: i.tenor.unit,
    tc: i.tenor.count,
    ey: i.expiryYears,
    n: i.quantity.notional,
    bc: i.quantity.baseCcy,
    side: i.side,
    product,
  };
}

/** The order-stable canonical form of the product oneof for the coalescing key. */
function canonicalProduct(p: Product): unknown {
  switch (p.kind) {
    case "vanilla":
      return { k: "v", ot: p.vanilla.optionType, s: canonicalStrike(p.vanilla.strike) };
    case "strategy":
      return { k: "s", kind: p.strategy.kind };
    case "singleBarrier":
      return {
        k: "sbar",
        ot: p.singleBarrier.vanilla.optionType,
        strike: canonicalStrike(p.singleBarrier.vanilla.strike),
        bk: p.singleBarrier.kind,
        bs: p.singleBarrier.side,
        bar: p.singleBarrier.barrier,
        reb: p.singleBarrier.rebate,
        mon: p.singleBarrier.monitoring,
      };
    case "doubleBarrier":
      return {
        k: "dbar",
        ot: p.doubleBarrier.vanilla.optionType,
        strike: canonicalStrike(p.doubleBarrier.vanilla.strike),
        bk: p.doubleBarrier.kind,
        lo: p.doubleBarrier.lowerBarrier,
        hi: p.doubleBarrier.upperBarrier,
        reb: p.doubleBarrier.rebate,
        mon: p.doubleBarrier.monitoring,
      };
    case "digital":
      return {
        k: "dig",
        ot: p.digital.optionType,
        strike: p.digital.strike,
        sty: p.digital.style,
        po: p.digital.payout,
      };
    case "touch":
      return {
        k: "tch",
        tk: p.touch.kind,
        lo: p.touch.lowerBarrier,
        hi: p.touch.upperBarrier,
        reb: p.touch.rebate,
        mon: p.touch.monitoring,
      };
    case "varianceSwap":
      return { k: "var", sv: p.varianceSwap.strikeVol };
    case "volatilitySwap":
      return { k: "vol", sv: p.volatilitySwap.strikeVol };
    case "asianOption":
      return {
        k: "asn",
        ot: p.asianOption.optionType,
        strike: p.asianOption.strike,
        avg: p.asianOption.averaging,
        obs: p.asianOption.observations,
        m: p.asianOption.method,
        ea: p.asianOption.elapsedAvg,
        ew: p.asianOption.elapsedWeight,
      };
    case "forwardStart":
      return {
        k: "fwds",
        ot: p.forwardStart.optionType,
        m: p.forwardStart.moneyness,
        r: p.forwardStart.reset,
      };
    case "cliquet":
      return {
        k: "clq",
        ot: p.cliquet.optionType,
        m: p.cliquet.moneyness,
        np: p.cliquet.periods,
        lf: p.cliquet.localFloor ?? null,
        lc: p.cliquet.localCap ?? null,
        gf: p.cliquet.globalFloor ?? null,
        gc: p.cliquet.globalCap ?? null,
        mcp: p.cliquet.mcPairs,
        // The MC seed is a bigint; stringify so the coalescing key is JSON-serialisable.
        mcs: p.cliquet.mcSeed.toString(),
      };
    case "quanto":
      return {
        k: "qto",
        py: p.quanto.payoff,
        ot: p.quanto.optionType,
        strike: p.quanto.strike,
        cv: p.quanto.conversionVol,
        rho: p.quanto.correlation,
      };
    case "tarf":
      return {
        k: "tarf",
        ot: p.tarf.optionType,
        strike: p.tarf.strike,
        tgt: p.tarf.target,
        lev: p.tarf.leverage,
        red: p.tarf.redemption,
        fy: p.tarf.schedule.fixingYears,
        fn: p.tarf.schedule.fixingNotional,
        mcp: p.tarf.mcPairs,
        // The MC seed is a bigint; stringify so the coalescing key is JSON-serialisable.
        mcs: p.tarf.mcSeed.toString(),
      };
    case "accumulator":
      return {
        k: "acc",
        pv: p.accumulator.pivot,
        bar: p.accumulator.barrier,
        lev: p.accumulator.leverage,
        mon: p.accumulator.monitoring,
        fy: p.accumulator.schedule.fixingYears,
        fn: p.accumulator.schedule.fixingNotional,
        mcp: p.accumulator.mcPairs,
        mcs: p.accumulator.mcSeed.toString(),
      };
    case "lookback":
      return {
        k: "lbk",
        st: p.lookback.style,
        ot: p.lookback.optionType,
        mon: p.lookback.monitoring,
        strike: p.lookback.strike,
        obs: p.lookback.observations,
        mcp: p.lookback.mcPairs,
        mcs: p.lookback.mcSeed.toString(),
      };
  }
}

function canonicalStrike(s: StrikeOrDelta): unknown {
  return s.kind === "strike" ? { k: "strike", v: s.strike } : { k: "delta", v: s.delta };
}

function canonicalConventions(c: Conventions): unknown {
  return [c.deltaConvention, c.atmConvention, c.premiumStyle, c.cut, c.dayCount, c.settlement];
}
