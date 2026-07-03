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
  AmericanOption,
  AsianMethod,
  AveragingStyle,
  BarrierKind,
  BarrierSide,
  BasketKind,
  BasketLeg,
  BasketOption,
  BondCouponFrequency,
  BondDayCount,
  BondInstrument,
  BondPositionSide,
  BrokenDate,
  BuildCurveRequest,
  CalibratedCurve,
  CalibratedCurvePoint,
  CcyPair,
  Cliquet,
  CommodityRef,
  Conventions,
  CryptoPair,
  DatePillar,
  Digital,
  DigitalStyle,
  DoubleBarrier,
  EquityRef,
  ExerciseStyle,
  FixingSchedule,
  FixingSource,
  FraInstrument,
  GetCurveRequest,
  GetCurveResponse,
  Greeks,
  Heartbeat,
  Instrument,
  InstrumentQuote,
  Leg,
  Lookback,
  LookbackMonitoring,
  LookbackStyle,
  Margining,
  MarkCurveRequest,
  MarkCurveResponse,
  MarketObservable,
  Metal,
  MetalPair,
  MonitoringStyle,
  OisCurvePillar,
  OisDirection,
  OisInstrument,
  OptionType,
  PaymentFrequency,
  Pivot,
  PricingModel,
  Product,
  QuantoPayoff,
  RatesAccrualBasis,
  RatesCurveSet,
  RatesInstrument,
  RatesLegDayCount,
  RatesPricingResult,
  RatesQuote,
  SettlementStyle,
  Side,
  SingleBarrier,
  SmileModel,
  StrategyKind,
  StreamHealth,
  StrikeOrDelta,
  Tarf,
  TarfRedemption,
  Tenor,
  TenorUnit,
  Touch,
  TouchKind,
  Underlying,
  VanillaIrsInstrument,
  WindowBarrier,
  XvaPricingRequest,
  XvaResult,
  XvaSurvivalCurve,
  XvaTrade,
} from "../contract/contract";
import type {
  AdditiveRisk,
  LimitUtilization,
  NumeraireRate,
  RatesPosition,
  RatesRiskNode,
  RatesRiskScope,
  ReportingNumeraire,
  RiskDimension,
  RiskNode,
  RiskPosition,
  RiskScope,
} from "../contract/riskCodec";
import type { InstrumentDef } from "../contract/referenceDataCodec";
import { PRICING_MODEL_MEMBERS, SMILE_MODEL_MEMBERS } from "../contract/enums";

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

/**
 * Resolve a pair argument that is either the trader-typed string (`"EURUSD"`) or
 * an already-parsed `CcyPair` (the polymorphic `CELNET.INSTRUMENT` path passes the
 * underlier's leg-string projection, whose legs — an equity ticker, a commodity
 * symbol, a crypto coin — are not constrained to the 6-letter FX shape). The
 * string form keeps the exact legacy parse, so every FX flow is byte-unchanged.
 */
function resolvePair(p: string | CcyPair): CcyPair {
  return typeof p === "string" ? parsePair(p) : p;
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
 * Parse a strategy-template kind into the contract `StrategyKind`. Accepts the
 * canonical contract names (`RISK_REVERSAL`, `STRADDLE`, `STRANGLE`, `SEAGULL`)
 * case-/separator-insensitively ("risk reversal" works), plus the desk
 * shorthand `RR` for the risk reversal.
 */
export function parseStrategyKind(raw: string): StrategyKind {
  const s = raw.trim().toUpperCase().replace(/[^A-Z0-9]/g, "");
  switch (s) {
    case "RISKREVERSAL":
    case "RR":
      return "RISK_REVERSAL";
    case "STRADDLE":
      return "STRADDLE";
    case "STRANGLE":
      return "STRANGLE";
    case "SEAGULL":
      return "SEAGULL";
    default:
      throw new ShapingError(
        `invalid strategy kind \`${raw}\` (expected RISK_REVERSAL, STRADDLE, STRANGLE, SEAGULL)`,
      );
  }
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
 * Parse a booking / pricing-model selector string into the contract
 * `PricingModel`. Accepts the trader-facing short name (`LSV`) and the canonical
 * contract names (`DEFAULT`/`ANALYTIC`, `LOCAL_STOCH_VOL`/`LOCAL-STOCH-VOL`),
 * case-insensitive. Empty/absent ⇒ `DEFAULT` (the product's native analytic
 * engine), so an omitted argument is the unchanged current behaviour. The
 * rejection message lists the canonical members straight from
 * `PRICING_MODEL_MEMBERS` (the same list the wire codec is built from) so it can
 * never drift from the supported set. Method provenance (the local-stochastic-
 * volatility / leverage-surface construction) stays in doc comments only; the
 * trader-facing name is the purpose-named `LOCAL_STOCH_VOL`.
 */
export function parsePricingModel(raw: string | undefined): PricingModel {
  const s = (raw ?? "").trim().toUpperCase();
  switch (s) {
    case "":
    case "DEFAULT":
    case "ANALYTIC":
    case "CLOSED_FORM":
    case "CLOSED-FORM":
      return "DEFAULT";
    case "LSV":
    case "LOCAL_STOCH_VOL":
    case "LOCAL-STOCH-VOL":
    case "LOCAL_STOCHASTIC_VOLATILITY":
      return "LOCAL_STOCH_VOL";
    default:
      throw new ShapingError(
        `invalid pricing model \`${raw}\` (expected one of: ${PRICING_MODEL_MEMBERS.join(", ")}; ` +
          `or the trader short names ANALYTIC, LSV)`,
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

/**
 * Parse the optional `CELNET.RFQ` panel-mode flag. Absent / FALSE / `""` ⇒ the
 * unchanged single-dealer RFQ (byte-identical to the contract before the panel
 * existed); TRUE or the word `PANEL` (case-insensitive) selects the multi-dealer
 * ranked panel (`request_multi_dealer_quote`). Anything else is a typo and is
 * rejected loudly, never silently treated as single-dealer.
 */
export function parseRfqPanelFlag(raw: boolean | string | undefined): boolean {
  if (raw === undefined || raw === false) return false;
  if (raw === true) return true;
  const s = raw.trim().toUpperCase();
  if (s === "") return false;
  if (s === "PANEL" || s === "TRUE") return true;
  if (s === "FALSE") return false;
  throw new ShapingError(`invalid RFQ panel flag \`${raw}\` (expected TRUE or "PANEL")`);
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
  readonly pair: string | CcyPair;
  readonly tenor: string;
  readonly strikeOrDelta: string | number;
  readonly callPut: string;
  readonly notional: number;
}

/**
 * Build the FX `pair` leg-string projection an `Underlying` overlays onto the
 * instrument (exported for the polymorphic `CELNET.INSTRUMENT` underlier path —
 * the projection keeps the FX-keyed surfaces total for every asset class).
 */
export function underlyingPairProjection(u: Underlying): CcyPair {
  switch (u.kind) {
    case "fx":
      return u.fx;
    case "metal":
      return { base: METAL_ISO_CODE[u.metal.metal], quote: u.metal.quote };
    case "equity":
      return { base: u.equity.symbol.ticker, quote: u.equity.currency };
    case "commodity":
      return { base: u.commodity.symbol.ticker, quote: u.commodity.currency };
    case "digitalAsset":
      return { base: u.digitalAsset.base, quote: u.digitalAsset.quote };
  }
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
    pair: resolvePair(args.pair),
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

/**
 * The fixed leg count each strategy template books (proto `Strategy`: "A risk
 * reversal has 2; a seagull has 3" — the four desk templates are booked as one
 * contract with exactly their ladder).
 */
const STRATEGY_TEMPLATE_LEGS: Record<StrategyKind, number> = {
  RISK_REVERSAL: 2,
  STRADDLE: 2,
  STRANGLE: 2,
  SEAGULL: 3,
};

export interface StrategyArgs {
  readonly pair: string | CcyPair;
  readonly tenor: string;
  readonly notional: number;
  /** The strategy template the legs realize (RISK_REVERSAL / STRADDLE / STRANGLE / SEAGULL). */
  readonly kind: string;
  /**
   * The legs as a row-per-leg matrix `[callPut, strike, side, ratio?]` (an Excel
   * range). `callPut` is `C`/`P`; `strike` is an absolute level (1.12) or a
   * convention delta (`25dC`, `25dP`, `ATM`); `side` is `BUY`/`SELL`; `ratio` is
   * the leg ratio relative to the base notional (defaults to 1; 2 for a 1x2).
   */
  readonly legs: ReadonlyArray<ReadonlyArray<string | number>>;
}

/**
 * Shape a recognized multi-leg vol strategy (proto `Strategy`, product field 8)
 * from the cell arguments. Each leg row is `[callPut, strike, side, ratio?]` —
 * exactly the `Leg` fields the server's `leg_from_json` decodes; delta strikes
 * resolve to levels server-side against the marked surface, per leg, and the
 * priced value is the server's signed `side·ratio` leg sum. The template `kind`
 * fixes the leg count (risk reversal / straddle / strangle book 2 legs, the
 * seagull 3); a mismatched ladder is a typed error, never a silently mislabeled
 * structure. `side` is TWO_WAY (the cell reads a market, not a directional
 * ticket), exactly like every other product cell.
 */
export function shapeStrategy(args: StrategyArgs): Instrument {
  if (!Number.isFinite(args.notional) || args.notional <= 0) {
    throw new ShapingError(`invalid notional \`${args.notional}\``);
  }
  const kind = parseStrategyKind(args.kind);
  const expected = STRATEGY_TEMPLATE_LEGS[kind];
  if (!Array.isArray(args.legs) || args.legs.length !== expected) {
    throw new ShapingError(
      `${kind} books exactly ${expected} legs [callPut, strike, side, ratio?], got ${
        Array.isArray(args.legs) ? args.legs.length : 0
      }`,
    );
  }
  const legs: Leg[] = args.legs.map((row, i) => {
    if (!Array.isArray(row) || row.length < 3 || row.length > 4) {
      throw new ShapingError(
        `strategy leg ${i + 1} must be [callPut, strike, side, ratio?]; got ${JSON.stringify(row)}`,
      );
    }
    const ratioCell = row[3];
    const ratio =
      ratioCell === undefined
        ? 1.0
        : typeof ratioCell === "number"
          ? ratioCell
          : Number(String(ratioCell).trim());
    if (!Number.isFinite(ratio) || ratio <= 0) {
      throw new ShapingError(
        `strategy leg ${i + 1} ratio \`${ratioCell}\` must be a positive number ` +
          "(the direction lives on `side`, never a negative ratio)",
      );
    }
    return {
      optionType: parseOptionType(String(row[0])),
      strike: parseStrikeOrDelta(row[1] as string | number),
      side: shapeForwardSide(String(row[2])),
      ratio,
    };
  });
  const { tenor, expiryYears } = parseTenor(args.tenor);
  return {
    pair: resolvePair(args.pair),
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: { kind: "strategy", strategy: { kind, legs } },
  };
}

/** The fully-parsed inputs a model-selected surface calibration shapes. */
export interface CalibrateArgs {
  readonly pair: string | CcyPair;
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
  const pair = resolvePair(args.pair);
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
  readonly pair: string | CcyPair;
  readonly tenor: string;
  readonly notional: number;
  /** Fixed strike vol; `0`/absent ⇒ a fresh request reading the fair strike off the response. */
  readonly strikeVol?: number | undefined;
}

/** Validate the common (pair, tenor, notional) of a swap/Asian request. */
function shapeSwapBase(args: { pair: string | CcyPair; tenor: string; notional: number }): {
  pair: CcyPair;
  tenor: Tenor;
  expiryYears: number;
} {
  if (!Number.isFinite(args.notional) || args.notional <= 0) {
    throw new ShapingError(`invalid notional \`${args.notional}\``);
  }
  const { tenor, expiryYears } = parseTenor(args.tenor);
  return { pair: resolvePair(args.pair), tenor, expiryYears };
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

// --- linear book (forward / swap / NDF) -------------------------------------

/**
 * Parse a directional side for a linear product: BUY / B (default) or SELL / S,
 * case-insensitive. A linear product needs a definite side (its PV has a sign);
 * TWO_WAY is not a valid linear direction.
 */
export function shapeForwardSide(raw: string | undefined): Side {
  const t = (raw ?? "BUY").trim().toUpperCase();
  if (t === "BUY" || t === "B") return "BUY";
  if (t === "SELL" || t === "S") return "SELL";
  throw new ShapingError(`invalid side \`${raw}\` (expected BUY/B or SELL/S)`);
}

/** Validate a strictly-positive contract (forward) rate `K`. */
function shapeContractRate(raw: number): number {
  if (!Number.isFinite(raw) || raw <= 0) {
    throw new ShapingError(`invalid contract rate \`${raw}\` (must be positive)`);
  }
  return raw;
}

/** Validate a strictly-positive linear notional. */
function shapeLinearNotional(raw: number): number {
  if (!Number.isFinite(raw) || raw <= 0) {
    throw new ShapingError(`invalid notional \`${raw}\` (must be positive)`);
  }
  return raw;
}

/**
 * Parse the published NDF fixing identity (EMTA/ISDA per-currency templates),
 * case-insensitive, accepting the dotted code (`BRL.PTAX`), the bare token
 * (`BRLPTAX`/`PTAX`) or the wire member (`BRL_PTAX`). Identity only — the live
 * fixing value is never sourced in-repo.
 */
export function shapeFixingSource(raw: string): FixingSource {
  const t = raw.trim().toUpperCase().replace(/[._\s]/g, "");
  switch (t) {
    case "KRWKFTC18":
    case "KFTC18":
      return "KRW_KFTC18";
    case "TWDTAIPEI":
    case "TAIPEI":
      return "TWD_TAIPEI";
    case "INRRBIREF":
    case "INRRBIB":
    case "RBIB":
      return "INR_RBI_REF";
    case "BRLPTAX":
    case "PTAX":
      return "BRL_PTAX";
    case "CLPDOLAROBS":
    case "DOLAROBS":
      return "CLP_DOLAR_OBS";
    case "COPTRM":
    case "TRM":
      return "COP_TRM";
    default:
      throw new ShapingError(
        `invalid fixing \`${raw}\` (expected one of KRW.KFTC18, TWD.TAIPEI, ` +
          `INR.RBIB, BRL.PTAX, CLP.DOLAROBS, COP.TRM)`,
      );
  }
}

/** Cell arguments for an FX outright forward / swap. */
export interface ForwardArgs {
  readonly pair: string | CcyPair;
  readonly tenor: string;
  readonly contractRate: number;
  readonly notional: number;
  readonly side?: string | undefined;
}

/**
 * Shape an FX outright forward (deliverable). The top-level instrument side and the
 * product side are the directional side (a linear PV needs a sign). The server
 * rejects a non-deliverable pair as INVALID_ARGUMENT (use NDF).
 */
export function shapeForward(args: ForwardArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase({ ...args, notional: args.notional });
  const side = shapeForwardSide(args.side);
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: shapeLinearNotional(args.notional), baseCcy: true },
    side,
    product: {
      kind: "fxForward",
      fxForward: {
        contractRate: shapeContractRate(args.contractRate),
        notional: shapeLinearNotional(args.notional),
        side,
      },
    },
  };
}

/**
 * Shape an FX swap (deliverable). The near leg's contract rate / notional / side
 * drive both legs; the far leg is the opposite side at the same contract rate,
 * settling at the forward tenor (`expiryYears`), while the near leg settles at the
 * spot date. The server reads the near leg's economics (mirroring the SDK's
 * single-near-leg swap shape) and forms the far leg.
 */
export function shapeSwap(args: ForwardArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase({ ...args, notional: args.notional });
  const side = shapeForwardSide(args.side);
  const contractRate = shapeContractRate(args.contractRate);
  const notional = shapeLinearNotional(args.notional);
  const oppositeSide: Side = side === "BUY" ? "SELL" : "BUY";
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional, baseCcy: true },
    side,
    product: {
      kind: "fxSwap",
      fxSwap: {
        near: { contractRate, notional, side },
        far: { contractRate, notional, side: oppositeSide },
      },
    },
  };
}

/** Cell arguments for a non-deliverable forward. */
export interface NdfArgs {
  readonly pair: string | CcyPair;
  readonly tenor: string;
  readonly contractRate: number;
  readonly notional: number;
  readonly fixing: string;
  readonly settlementCcy?: string | undefined;
  readonly side?: string | undefined;
}

/**
 * Shape a non-deliverable forward (non-deliverable underlying only). The fixing is
 * convention identity only; the live fixing VALUE is never sourced in-repo. The
 * settlement currency defaults to the convertible USD leg the EM panel cash-settles
 * in. The server rejects a deliverable pair as INVALID_ARGUMENT (use FORWARD).
 */
export function shapeNdf(args: NdfArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase({ ...args, notional: args.notional });
  const side = shapeForwardSide(args.side);
  const settlementCcy = (args.settlementCcy ?? "USD").trim().toUpperCase();
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: shapeLinearNotional(args.notional), baseCcy: true },
    side,
    product: {
      kind: "ndf",
      ndf: {
        contractRate: shapeContractRate(args.contractRate),
        notional: shapeLinearNotional(args.notional),
        side,
        fixing: shapeFixingSource(args.fixing),
        settlementCcy,
      },
    },
  };
}

// --- new payoff shapes: perpetual (arm 30) + listed-future option (arm 31) ---

/**
 * Parse the listed-future-option premium margining convention (proto
 * `Margining`), case-/separator-insensitive: `EQUITY`/`EQUITY_STYLE`/`UPFRONT`
 * (premium paid at trade date — the discounted price) or
 * `FUTURES`/`FUTURES_STYLE`/`DAILY` (premium margined daily like the future —
 * the undiscounted price). Absent ⇒ EQUITY_STYLE (the meaningful proto3 zero,
 * the ordinary upfront-premium contract).
 */
export function parseMargining(raw: string | undefined): Margining {
  const t = (raw ?? "EQUITY_STYLE").trim().toUpperCase().replace(/[._\s-]/g, "");
  switch (t) {
    case "EQUITYSTYLE":
    case "EQUITY":
    case "UPFRONT":
      return "EQUITY_STYLE";
    case "FUTURESSTYLE":
    case "FUTURES":
    case "DAILY":
      return "FUTURES_STYLE";
    default:
      throw new ShapingError(
        `invalid margining \`${raw}\` (expected EQUITY_STYLE or FUTURES_STYLE)`,
      );
  }
}

/** Validate a strictly-positive absolute strike for the closed-form arms 30/31. */
function shapeAbsoluteStrike(raw: number, family: string): number {
  if (!Number.isFinite(raw) || raw <= 0) {
    throw new ShapingError(`invalid ${family} strike \`${raw}\` (must be a positive level)`);
  }
  return raw;
}

/** Cell arguments for a perpetual (no-expiry) American option. */
export interface PerpetualArgs {
  readonly pair: string | CcyPair;
  /** Absolute strike level (a perpetual has no delta-quoted strike convention). */
  readonly strike: number;
  readonly callPut: string;
  readonly notional: number;
}

/**
 * Shape a perpetual (no-expiry) American option (proto arm 30) — the one
 * TENORLESS product on the contract: NO tenor exists for it and the canonical
 * wire shape is `expiry_years = 0` exactly (the server's
 * `convert::validate_perpetual_terms` guard; a non-zero/NaN expiry is
 * INVALID_ARGUMENT, never waved through). The product `notional` mirrors the one
 * quantity, exactly like the SDK's `InstrumentSpec::perpetual`. `side` is
 * TWO_WAY (a cell reads a market, not a directional ticket).
 */
export function shapePerpetual(args: PerpetualArgs): Instrument {
  if (!Number.isFinite(args.notional) || args.notional <= 0) {
    throw new ShapingError(`invalid notional \`${args.notional}\``);
  }
  return {
    pair: resolvePair(args.pair),
    expiryYears: 0,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: {
      kind: "perpetualOption",
      perpetualOption: {
        optionType: parseOptionType(args.callPut),
        strike: shapeAbsoluteStrike(args.strike, "perpetual"),
        notional: args.notional,
      },
    },
  };
}

/**
 * Parse a listed-future contract identity `TICKER[@VENUE]` (e.g. `CL@XNYM`,
 * `BRN-DEC26@IFEU`, or a bare `ES`) into the vendor-neutral symbol (ticker +
 * listing venue MIC; an omitted venue is the empty string, matching the proto
 * `Symbol` zero value for an unambiguous contract).
 */
export function parseFutureSymbol(raw: string): { ticker: string; venue: string } {
  const s = raw.trim().toUpperCase();
  const at = s.indexOf("@");
  const ticker = (at >= 0 ? s.slice(0, at) : s).trim();
  const venue = (at >= 0 ? s.slice(at + 1) : "").trim();
  if (ticker.length === 0) {
    throw new ShapingError(
      `invalid future symbol \`${raw}\` (expected TICKER or TICKER@VENUE, e.g. CL@XNYM)`,
    );
  }
  if (!/^[A-Z0-9]*$/.test(venue)) {
    throw new ShapingError(`invalid future venue \`${raw}\` (expected a MIC, e.g. XNYM, XCME)`);
  }
  return { ticker, venue };
}

/** Cell arguments for an option on a listed future. */
export interface ListedFutureOptionArgs {
  readonly pair: string | CcyPair;
  /** The OPTION's tenor (e.g. "9M"); the future's own expiry is `futureExpiry`. */
  readonly tenor: string;
  /** Absolute strike level (in the future's quote units). */
  readonly strike: number;
  readonly callPut: string;
  readonly notional: number;
  /** The listed future contract identity, `TICKER[@VENUE]` (e.g. `CL@XNYM`). */
  readonly futureSymbol: string;
  /** The FUTURE's own expiry as a year fraction; `>=` the option's expiry. */
  readonly futureExpiry: number;
  /** EQUITY_STYLE (upfront, default) or FUTURES_STYLE (daily-margined). */
  readonly margining?: string | undefined;
}

/**
 * Shape an option on a listed future (proto arm 31). The future must outlive
 * the option — `futureExpiry >= expiryYears > 0`, the same term guard the
 * server enforces (`convert::validate_listed_future_terms`), rejected here with
 * a typed error naming the constraint rather than a wire round-trip. The
 * product `notional` mirrors the one quantity (the SDK's
 * `InstrumentSpec::listed_future_option`); `side` is TWO_WAY (a cell reads a
 * market).
 */
export function shapeListedFutureOption(args: ListedFutureOptionArgs): Instrument {
  if (!Number.isFinite(args.notional) || args.notional <= 0) {
    throw new ShapingError(`invalid notional \`${args.notional}\``);
  }
  const { tenor, expiryYears } = parseTenor(args.tenor);
  if (!Number.isFinite(args.futureExpiry) || args.futureExpiry < expiryYears) {
    throw new ShapingError(
      `invalid futureExpiry \`${args.futureExpiry}\` (the future must outlive the option: ` +
        `futureExpiry >= ${expiryYears} years)`,
    );
  }
  return {
    pair: resolvePair(args.pair),
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: {
      kind: "listedFutureOption",
      listedFutureOption: {
        futureSymbol: parseFutureSymbol(args.futureSymbol),
        futureExpiryYears: args.futureExpiry,
        optionType: parseOptionType(args.callPut),
        strike: shapeAbsoluteStrike(args.strike, "listed-future"),
        notional: args.notional,
        margining: parseMargining(args.margining),
      },
    },
  };
}

// ---------------------------------------------------------------------------
// cross-asset underlyings — equity / commodity / digital-asset (crypto) vanillas
// on the ONE wire contract's `Underlying` oneof + `settlement_style`
// ---------------------------------------------------------------------------

/**
 * The ISO-4217 "X"-prefixed asset code each precious {@link Metal} projects to,
 * so a metal pair overlaps the FX `CcyPair` encoding byte-for-byte on the metal
 * leg (the convention/calendar registries key on the X-code base).
 */
const METAL_ISO_CODE: Record<Metal, string> = {
  GOLD: "XAU",
  SILVER: "XAG",
  PLATINUM: "XPT",
  PALLADIUM: "XPD",
};

/**
 * Parse a precious-metal token (a metal name, its ISO "X"-code, or the wire
 * member) into the canonical {@link Metal}. Case-insensitive.
 */
export function shapeMetal(raw: string): Metal {
  const t = raw.trim().toUpperCase();
  switch (t) {
    case "GOLD":
    case "XAU":
      return "GOLD";
    case "SILVER":
    case "XAG":
      return "SILVER";
    case "PLATINUM":
    case "XPT":
      return "PLATINUM";
    case "PALLADIUM":
    case "XPD":
      return "PALLADIUM";
    default:
      throw new ShapingError(
        `invalid metal \`${raw}\` (expected GOLD/XAU, SILVER/XAG, PLATINUM/XPT, PALLADIUM/XPD)`,
      );
  }
}

/**
 * Parse the contract settlement mechanics (proto `SettlementStyle`). LINEAR
 * (quote-ccy-margined) is the default; INVERSE_COIN is the coin-margined `1/S_T`
 * digital-asset convention. Case-insensitive; accepts `INVERSE`/`COIN` aliases.
 */
export function shapeSettlementStyle(raw: string | undefined): SettlementStyle {
  const t = (raw ?? "LINEAR").trim().toUpperCase().replace(/[._\s]/g, "");
  switch (t) {
    case "LINEAR":
      return "LINEAR";
    case "INVERSECOIN":
    case "INVERSE":
    case "COIN":
      return "INVERSE_COIN";
    default:
      throw new ShapingError(
        `invalid settlement style \`${raw}\` (expected LINEAR or INVERSE_COIN)`,
      );
  }
}

/** Validate a strictly-positive vanilla notional (cross-asset shapes share this). */
function shapeCrossAssetNotional(raw: number): number {
  if (!Number.isFinite(raw) || raw <= 0) {
    throw new ShapingError(`invalid notional \`${raw}\` (must be positive)`);
  }
  return raw;
}

/**
 * Build (and validate) an equity `Underlying` arm — proto `Underlying.equity`
 * (oneof field 4). Shared by the per-class vanilla shaper and the polymorphic
 * `CELNET.INSTRUMENT` underlier grammar, so the wire shape is built in ONE place.
 */
export function shapeEquityUnderlying(args: {
  readonly ticker: string;
  readonly currency: string;
  readonly venue?: string | undefined;
}): Underlying {
  const ticker = args.ticker.trim().toUpperCase();
  if (ticker.length === 0) throw new ShapingError("equity ticker is required");
  const currency = args.currency.trim().toUpperCase();
  if (!/^[A-Z]{3}$/.test(currency)) {
    throw new ShapingError(`invalid equity currency \`${args.currency}\` (expected a 3-letter code)`);
  }
  const equity: EquityRef = {
    symbol: { ticker, venue: (args.venue ?? "").trim().toUpperCase() },
    currency,
  };
  return { kind: "equity", equity, settlementCcy: currency };
}

/**
 * Build (and validate) a commodity `Underlying` arm — proto `Underlying.commodity`
 * (oneof field 5). Shared by the per-class vanilla shaper and the polymorphic
 * `CELNET.INSTRUMENT` underlier grammar.
 */
export function shapeCommodityUnderlying(args: {
  readonly symbol: string;
  readonly currency: string;
  readonly venue?: string | undefined;
}): Underlying {
  const ticker = args.symbol.trim().toUpperCase();
  if (ticker.length === 0) throw new ShapingError("commodity symbol is required");
  const currency = args.currency.trim().toUpperCase();
  if (!/^[A-Z]{3}$/.test(currency)) {
    throw new ShapingError(
      `invalid commodity currency \`${args.currency}\` (expected a 3-letter code)`,
    );
  }
  const commodity: CommodityRef = {
    symbol: { ticker, venue: (args.venue ?? "").trim().toUpperCase() },
    currency,
  };
  return { kind: "commodity", commodity, settlementCcy: currency };
}

/**
 * Build a digital-asset (crypto) `Underlying` arm — proto `Underlying.digital_asset`
 * (oneof field 6) — from a crypto pair string (`"BTC-USD"`, `"ETH/USDT"`, `"BTCUSD"`).
 */
export function shapeCryptoUnderlying(pair: string): Underlying {
  const digitalAsset = parseCryptoPair(pair);
  return { kind: "digitalAsset", digitalAsset, settlementCcy: digitalAsset.quote };
}

/**
 * Build (and validate) a precious-metal `Underlying` arm — proto `Underlying.metal`
 * (oneof field 3) — from a metal token (name / ISO X-code) + fiat quote leg.
 */
export function shapeMetalUnderlying(args: {
  readonly metal: string;
  readonly quote: string;
}): Underlying {
  const metal = shapeMetal(args.metal);
  const quote = args.quote.trim().toUpperCase();
  if (!/^[A-Z]{3}$/.test(quote)) {
    throw new ShapingError(`invalid metal quote \`${args.quote}\` (expected a 3-letter code)`);
  }
  const metalPair: MetalPair = { metal, quote };
  return { kind: "metal", metal: metalPair, settlementCcy: quote };
}

/**
 * Shape a cross-asset vanilla instrument over an arbitrary {@link Underlying} arm:
 * the `underlying` oneof carries the asset-class identity, the FX `pair`
 * leg-string projection keeps the FX-keyed surfaces total, and `settlementStyle`
 * carries the linear/inverse contract mechanics (INVERSE_COIN is meaningful only
 * for a digital-asset underlying). `side` is TWO_WAY (a cell reads a market, not a
 * directional ticket). The notional is in the base/asset leg.
 */
function shapeCrossAssetVanilla(
  underlying: Underlying,
  args: CrossAssetVanillaArgs,
  settlementStyle: SettlementStyle,
): Instrument {
  const { tenor, expiryYears } = parseTenor(args.tenor);
  const instrument: Instrument = {
    pair: underlyingPairProjection(underlying),
    underlying,
    tenor,
    expiryYears,
    quantity: { notional: shapeCrossAssetNotional(args.notional), baseCcy: true },
    side: "TWO_WAY" as Side,
    product: {
      kind: "vanilla",
      vanilla: {
        optionType: parseOptionType(args.callPut),
        strike: parseStrikeOrDelta(args.strikeOrDelta),
      },
    },
  };
  // LINEAR is the proto3 zero default — presence-omit it so an FX/linear frame is
  // byte-identical to the contract before the field existed.
  if (settlementStyle !== "LINEAR") instrument.settlementStyle = settlementStyle;
  return instrument;
}

/** Cell arguments shared by every cross-asset vanilla (equity / commodity / crypto). */
export interface CrossAssetVanillaArgs {
  readonly tenor: string;
  readonly strikeOrDelta: string | number;
  readonly callPut: string;
  readonly notional: number;
}

/** Cell arguments for an equity (single-name / index) vanilla. */
export interface EquityVanillaArgs extends CrossAssetVanillaArgs {
  readonly ticker: string;
  readonly currency: string;
  readonly venue?: string | undefined;
}

/**
 * Shape an equity (single-name or index) vanilla — proto `Underlying.equity`
 * (oneof field 4), priced through the generalized cost-of-carry seam
 * (dividend yield as carry `b`). Quote-ccy-margined (LINEAR) settlement.
 */
export function shapeEquityVanilla(args: EquityVanillaArgs): Instrument {
  return shapeCrossAssetVanilla(shapeEquityUnderlying(args), args, "LINEAR");
}

/** Cell arguments for a commodity vanilla. */
export interface CommodityVanillaArgs extends CrossAssetVanillaArgs {
  readonly symbol: string;
  readonly currency: string;
  readonly venue?: string | undefined;
}

/**
 * Shape a commodity vanilla — proto `Underlying.commodity` (oneof field 5),
 * priced through the generalized cost-of-carry seam (storage/convenience yield as
 * carry `b`). Quote-ccy-margined (LINEAR) settlement.
 */
export function shapeCommodityVanilla(args: CommodityVanillaArgs): Instrument {
  return shapeCrossAssetVanilla(shapeCommodityUnderlying(args), args, "LINEAR");
}

/** Cell arguments for a digital-asset (crypto) vanilla. */
export interface CryptoVanillaArgs extends CrossAssetVanillaArgs {
  readonly pair: string;
  /** LINEAR (USD/stablecoin-margined) or INVERSE_COIN (coin-margined `1/S_T`). */
  readonly settlementStyle?: string | undefined;
}

/**
 * Parse a crypto pair string (`"BTC-USD"`, `"BTC/USDT"`, `"BTCUSD"` where the
 * quote is a known fiat/stablecoin) into a {@link CryptoPair}. Crypto tickers are
 * NOT constrained to the 3-letter ISO shape, so an explicit separator is the
 * unambiguous form; a separatorless string is split on a trailing known quote.
 */
export function parseCryptoPair(raw: string): CryptoPair {
  const s = raw.trim().toUpperCase();
  const sep = /[-/_]/.exec(s);
  if (sep) {
    const [base, quote] = s.split(/[-/_]/, 2);
    if (base && quote) return { base, quote };
  }
  // Separatorless: peel a trailing known numeraire (fiat or common stablecoin).
  for (const q of ["USDT", "USDC", "USD", "EUR", "BTC", "ETH"]) {
    if (s.length > q.length && s.endsWith(q)) {
      return { base: s.slice(0, s.length - q.length), quote: q };
    }
  }
  throw new ShapingError(
    `invalid crypto pair \`${raw}\` (expected e.g. BTC-USD, ETH/USDT, BTCUSD)`,
  );
}

/**
 * Shape a digital-asset (crypto) vanilla — proto `Underlying.digital_asset`
 * (oneof field 6). LINEAR (USD/stablecoin-margined) is the default; INVERSE_COIN
 * is the coin-margined `1/S_T` (base-coin-denominated) convention, carried on
 * `Instrument.settlement_style` (field 29) — the inverse-perpetual desk's payoff.
 */
export function shapeCryptoVanilla(args: CryptoVanillaArgs): Instrument {
  return shapeCrossAssetVanilla(
    shapeCryptoUnderlying(args.pair),
    args,
    shapeSettlementStyle(args.settlementStyle),
  );
}

/**
 * Shape a precious-metal vanilla — proto `Underlying.metal` (oneof field 3). The
 * metal is the base/asset leg; the FX `pair` projection uses the metal's ISO
 * "X"-code so the convention/calendar registries (keyed on the X-code base) stay
 * total. Quote-ccy-margined (LINEAR) settlement.
 */
export function shapeMetalVanilla(args: MetalVanillaArgs): Instrument {
  return shapeCrossAssetVanilla(shapeMetalUnderlying(args), args, "LINEAR");
}

/** Cell arguments for a precious-metal vanilla. */
export interface MetalVanillaArgs extends CrossAssetVanillaArgs {
  readonly metal: string;
  readonly quote: string;
}

/** The fully-parsed inputs an Asian CELNET.* function shapes into a request. */
export interface AsianArgs {
  readonly pair: string | CcyPair;
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

/** The fully-parsed inputs the FORWARDSTART family (CELNET.INSTRUMENT spec) shapes into a request. */
export interface ForwardStartArgs {
  readonly pair: string | CcyPair;
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

/** The fully-parsed inputs the CLIQUET family (CELNET.INSTRUMENT spec) shapes into a request. */
export interface CliquetArgs {
  readonly pair: string | CcyPair;
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

/** The fully-parsed inputs the QUANTO family (CELNET.INSTRUMENT spec) shapes into a request. */
export interface QuantoArgs {
  readonly pair: string | CcyPair;
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

/** Parse a basket aggregation-kind selector (case-insensitive, with aliases). */
export function parseBasketKind(raw: string | undefined): BasketKind {
  const s = (raw ?? "BASKET").trim().toUpperCase();
  switch (s) {
    case "":
    case "BASKET":
    case "WEIGHTED":
    case "SUM":
      return "BASKET";
    case "BEST_OF":
    case "BEST-OF":
    case "BESTOF":
    case "BEST":
    case "MAX":
      return "BEST_OF";
    case "WORST_OF":
    case "WORST-OF":
    case "WORSTOF":
    case "WORST":
    case "MIN":
      return "WORST_OF";
    default:
      throw new ShapingError(`unknown basket kind \`${raw}\` (BASKET / BEST_OF / WORST_OF)`);
  }
}

/** Arguments to shape a correlated multi-asset basket from worksheet cells. */
export interface BasketArgs {
  /** The settlement / numeraire pair (the top-level instrument pair). */
  readonly pair: string | CcyPair;
  readonly tenor: string;
  readonly notional: number;
  /** Call or put on the aggregated underlying. */
  readonly callPut: string;
  /** The strike `K` on the aggregated underlying. */
  readonly strike: number;
  /** The aggregation kind (BASKET / BEST_OF / WORST_OF; default BASKET). */
  readonly kind?: string | undefined;
  /**
   * The legs as a row-per-leg matrix `[pair, weight, spot, vol, rFor]` (an Excel
   * range). `pair` is a string like "EURUSD"; the rest are numbers.
   */
  readonly legs: ReadonlyArray<ReadonlyArray<string | number>>;
  /**
   * The N×N correlation matrix as a range (row-major), flattened to row-major on
   * the wire. Must be N×N for the N legs.
   */
  readonly correlations: ReadonlyArray<ReadonlyArray<number>>;
  /** Scrambled-Sobol points per replication (`0`/absent ⇒ server default). */
  readonly mcPaths?: number | undefined;
  /** Independent randomized scrambles (`0`/absent ⇒ server default). */
  readonly mcReplications?: number | undefined;
  /** Time steps per path (`0`/absent ⇒ server default). */
  readonly mcSteps?: number | undefined;
  /** The base scramble seed (`0`/absent ⇒ default). */
  readonly mcSeed?: number | undefined;
}

/**
 * Shape a correlated multi-asset basket from the cell arguments. Each leg row is
 * `[pair, weight, spot, vol, rFor]`; the correlation range is the N×N matrix,
 * flattened ROW-MAJOR for the wire `correlations` array the server decodes. The
 * shaper validates the leg/correlation shapes locally; the SPD check is the
 * server's (a non-PSD matrix is rejected `INVALID_ARGUMENT`). `side` is TWO_WAY.
 */
export function shapeBasket(args: BasketArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  if (!Array.isArray(args.legs) || args.legs.length < 1) {
    throw new ShapingError("a basket needs at least one leg row [pair, weight, spot, vol, rFor]");
  }
  const legs: BasketLeg[] = args.legs.map((row, i) => {
    if (!Array.isArray(row) || row.length < 5) {
      throw new ShapingError(
        `basket leg ${i + 1} must be [pair, weight, spot, vol, rFor]; got ${JSON.stringify(row)}`,
      );
    }
    const legPair = parsePair(String(row[0]));
    const num = (v: string | number, name: string): number => {
      const n = typeof v === "number" ? v : Number(v);
      if (!Number.isFinite(n)) {
        throw new ShapingError(`basket leg ${i + 1} ${name} \`${v}\` is not a number`);
      }
      return n;
    };
    const spot = num(row[2], "spot");
    const vol = num(row[3], "vol");
    if (!(spot > 0)) {
      throw new ShapingError(`basket leg ${i + 1} spot must be positive`);
    }
    if (!(vol >= 0)) {
      throw new ShapingError(`basket leg ${i + 1} vol must be non-negative`);
    }
    return {
      pair: legPair,
      weight: num(row[1], "weight"),
      spot,
      vol,
      rFor: num(row[4], "rFor"),
    };
  });
  const n = legs.length;
  if (!Array.isArray(args.correlations) || args.correlations.length !== n) {
    throw new ShapingError(`correlation matrix must be ${n}×${n} for ${n} legs`);
  }
  const correlations: number[] = [];
  for (let i = 0; i < n; i++) {
    const row = args.correlations[i];
    if (!Array.isArray(row) || row.length !== n) {
      throw new ShapingError(`correlation matrix row ${i + 1} must have ${n} entries`);
    }
    for (let j = 0; j < n; j++) {
      const v = Number(row[j]);
      if (!Number.isFinite(v)) {
        throw new ShapingError(`correlation entry (${i + 1},${j + 1}) is not a number`);
      }
      correlations.push(v);
    }
  }
  if (!(args.strike > 0)) {
    throw new ShapingError("basket strike must be a positive absolute level");
  }
  const basket: BasketOption = {
    legs,
    correlations,
    optionType: parseOptionType(args.callPut),
    strike: args.strike,
    kind: parseBasketKind(args.kind),
    mcPaths: shapeMcPairs(args.mcPaths),
    mcReplications: shapeMcPairs(args.mcReplications),
    mcSteps: shapeMcPairs(args.mcSteps),
    mcSeed: shapeMcSeed(args.mcSeed),
  };
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: { kind: "basket", basket },
  };
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

/** The fully-parsed inputs the TARF family (CELNET.INSTRUMENT spec) shapes into a request. */
export interface TarfArgs {
  readonly pair: string | CcyPair;
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

/** The fully-parsed inputs the PIVOT family (CELNET.INSTRUMENT spec) shapes into a request. */
export interface PivotArgs {
  readonly pair: string | CcyPair;
  readonly tenor: string;
  readonly callPut: string;
  readonly strike: string | number;
  readonly pivot: number;
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
 * Shape a pivot Target-Redemption Accumulator from the cell arguments — the TARF
 * shape plus the distinct `pivot` level at which the geared adverse leg engages
 * (`pivot === strike` is the exact plain-TARF slice). The strike must be an
 * absolute level; `pivot > 0` is the kink; `target > 0` is the cumulative gain
 * that redeems; `leverage ≥ 0` gears the adverse leg; `fixings ≥ 1` is the
 * (equally-spaced) fixing count. Always Monte-Carlo priced — the premium carries
 * a standard error, surfaced honestly in the spill. `side` is TWO_WAY.
 */
export function shapePivot(args: PivotArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  const strike = parseStrikeOrDelta(args.strike);
  if (strike.kind !== "strike") {
    throw new ShapingError(
      `PIVOT strike must be an absolute level (e.g. 1.10), not a delta \`${args.strike}\``,
    );
  }
  if (!Number.isFinite(args.pivot) || args.pivot <= 0) {
    throw new ShapingError(`PIVOT pivot \`${args.pivot}\` must be a positive level`);
  }
  if (!Number.isFinite(args.target) || args.target <= 0) {
    throw new ShapingError(`PIVOT target \`${args.target}\` must be a positive cumulative gain`);
  }
  if (!Number.isFinite(args.leverage) || args.leverage < 0) {
    throw new ShapingError(`PIVOT leverage \`${args.leverage}\` must be ≥ 0`);
  }
  const fixings = shapeFixings(args.fixings, "PIVOT fixings");
  const schedule: FixingSchedule = {
    fixingYears: equalFixingYears(fixings),
    fixingNotional: shapeFixingNotional(args.fixingNotional),
  };
  const pivot: Pivot = {
    optionType: parseOptionType(args.callPut),
    strike: strike.strike,
    pivot: args.pivot,
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
    product: { kind: "pivot", pivot },
  };
}

/** The fully-parsed inputs the ACCUMULATOR family (CELNET.INSTRUMENT spec) shapes into a request. */
export interface AccumulatorArgs {
  readonly pair: string | CcyPair;
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

/** The fully-parsed inputs the LOOKBACK family (CELNET.INSTRUMENT spec) shapes into a request. */
export interface LookbackArgs {
  readonly pair: string | CcyPair;
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
// American / Bermudan early-exercise shaping (proto AmericanOption, field 24)
// ---------------------------------------------------------------------------

/**
 * Parse an exercise-style selector string into the contract `ExerciseStyle`.
 * Accepts the trader-facing names (`AMERICAN`/`AMER`/`A`, `BERMUDAN`/`BERM`/`B`)
 * and the canonical contract names, case-insensitive. Empty/absent ⇒ `AMERICAN`
 * (the proto3 zero value), so an omitted argument is continuous early exercise.
 * The choice of BERMUDAN additionally requires a non-empty exercise-date set
 * (supplied via `bermudanSteps` to the shaper); a BERMUDAN with no dates is a
 * domain error the shaper rejects (mirroring the server's domain validation).
 */
export function parseExerciseStyle(raw: string | undefined): ExerciseStyle {
  const s = (raw ?? "").trim().toUpperCase();
  if (s === "" || s === "AMERICAN" || s === "AMER" || s === "A") return "AMERICAN";
  if (s === "BERMUDAN" || s === "BERM" || s === "B") return "BERMUDAN";
  throw new ShapingError(
    `invalid exercise style \`${raw}\` (expected AMERICAN or BERMUDAN)`,
  );
}

/** The fully-parsed inputs the AMERICAN family (CELNET.INSTRUMENT spec) shapes into a request. */
export interface AmericanArgs {
  readonly pair: string | CcyPair;
  readonly tenor: string;
  /** The strike `K` — must be an absolute level (an American is struck at a level). */
  readonly strike: string | number;
  readonly callPut: string;
  readonly notional: number;
  /** Optional AMERICAN (default, continuous exercise) or BERMUDAN (discrete dates). */
  readonly style?: string | undefined;
  /**
   * Optional number of equally-spaced Bermudan exercise dates `k/n · T` for
   * `k = 1..=n` over `(0, T]` (bit-identical to the CLI's `--bermudan-steps`).
   * Supplying `n ≥ 1` selects BERMUDAN even if `style` is omitted; `0`/absent ⇒
   * continuous AMERICAN. (Arbitrary non-uniform date sets are exposed via the SDK
   * `AmericanTerms::bermudan`; the Excel cell uses the count-based ticket.)
   */
  readonly bermudanSteps?: number | undefined;
  /**
   * Optional Longstaff-Schwartz path count: `0`/absent ⇒ the exact projected-SOR
   * finite-difference engine (no std-error); `> 0` ⇒ the regression Monte-Carlo
   * engine (which carries a `priceStdError`, surfaced honestly).
   */
  readonly lsmPaths?: number | undefined;
  /**
   * Optional equally-spaced AMERICAN exercise opportunities for the LSM engine
   * (`0` ⇒ the server default; ignored for the FD engine and for BERMUDAN, whose
   * dates are explicit).
   */
  readonly lsmExerciseDates?: number | undefined;
  /** Optional LSM scramble seed (bit-reproducible; ignored by the FD engine). */
  readonly lsmSeed?: number | undefined;
}

/**
 * Build the equally-spaced Bermudan exercise year-fractions a count-based ticket
 * implies: `n` points at `k/n · T` for `k = 1..=n` over `(0, T]`. Bit-identical to
 * the CLI's `american_spec` (`crates/celnet-cli/src/exotic.rs`), so an Excel
 * Bermudan encodes the SAME `bermudan_dates` the CLI does and the server prices a
 * cell identically to the SDK/CLI. The final point is exactly `T` (expiry is always
 * exercisable). A single date at `T` ⇒ the European identity the server proves.
 */
function equalBermudanDates(steps: number, expiryYears: number): number[] {
  const dates: number[] = [];
  for (let k = 1; k <= steps; k += 1) dates.push((expiryYears * k) / steps);
  return dates;
}

/**
 * Shape an American / Bermudan early-exercise vanilla from the cell arguments. The
 * strike must be an absolute level (rejected as a delta). BERMUDAN — selected by
 * `style` OR by a positive `bermudanSteps` — exercises only on the `n` equally-
 * spaced dates `k/n · T`; a BERMUDAN with no dates is a domain error. `lsmPaths`
 * of `0`/absent selects the exact FD engine (no std-error); `> 0` selects the
 * Longstaff-Schwartz Monte-Carlo engine (the reply then carries a `priceStdError`).
 * `side` on the instrument is TWO_WAY (the cell reads a market). The booking model
 * stays DEFAULT — early exercise is the product's native engine, not a pricing
 * directive. Scope: American/Bermudan VANILLA only.
 */
export function shapeAmerican(args: AmericanArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  const sod = parseStrikeOrDelta(args.strike);
  if (sod.kind !== "strike") {
    throw new ShapingError(
      `american strike must be an absolute level (e.g. 1.10), not a delta \`${args.strike}\``,
    );
  }
  // BERMUDAN is selected by an explicit style OR by a positive step count. A
  // count-based ticket implies BERMUDAN even when `style` is omitted (matching the
  // CLI's `--bermudan-steps n`); an explicit AMERICAN with a positive count is a
  // contradiction the shaper rejects rather than silently ignoring the count.
  const requested = parseExerciseStyle(args.style);
  const steps = args.bermudanSteps ?? 0;
  if (!Number.isFinite(steps) || !Number.isInteger(steps) || steps < 0) {
    throw new ShapingError(`bermudan steps must be a non-negative integer; got \`${args.bermudanSteps}\``);
  }
  if (requested === "AMERICAN" && steps > 0 && args.style !== undefined && args.style.trim() !== "") {
    throw new ShapingError(
      "an explicit AMERICAN style takes no bermudan steps (omit the style, or use BERMUDAN)",
    );
  }
  const style: ExerciseStyle = requested === "BERMUDAN" || steps > 0 ? "BERMUDAN" : "AMERICAN";
  let bermudanDates: number[] = [];
  if (style === "BERMUDAN") {
    if (steps < 1) {
      throw new ShapingError(
        "a BERMUDAN american requires a positive number of exercise dates (bermudanSteps ≥ 1)",
      );
    }
    bermudanDates = equalBermudanDates(steps, expiryYears);
  }
  const american: AmericanOption = {
    optionType: parseOptionType(args.callPut),
    strike: sod.strike,
    exerciseStyle: style,
    bermudanDates,
    lsmPaths: shapeMcPairs(args.lsmPaths),
    // The LSM American exercise-date resolution is only meaningful for the LSM
    // engine on an AMERICAN; the server ignores it for FD and for BERMUDAN.
    lsmExerciseDates: shapeMcPairs(args.lsmExerciseDates),
    lsmSeed: shapeMcSeed(args.lsmSeed),
  };
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    product: { kind: "american", american },
  };
}

/**
 * True iff an American/Bermudan is priced by the Longstaff-Schwartz Monte-Carlo
 * engine (`lsmPaths > 0`) ⇒ it carries a std-error. `lsmPaths == 0` selects the
 * exact projected-SOR finite-difference engine (no std-error). Mirrors
 * `lookbackIsMonteCarlo` — the function gates the std-error row on this, so an
 * exact FD price never surfaces a stray precision claim.
 */
export function americanIsMonteCarlo(a: AmericanOption): boolean {
  return a.lsmPaths > 0;
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

/** The fully-parsed inputs the BARRIER family (CELNET.INSTRUMENT spec) shapes (single OR double). */
export interface BarrierArgs {
  readonly pair: string | CcyPair;
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
  /**
   * Optional booking model: `ANALYTIC`/`DEFAULT` (closed-form, the default) or
   * `LSV`/`LOCAL_STOCH_VOL` (the local-stochastic-volatility ADI-PDE engine). The
   * server prices a single-barrier (continuously-monitored knock-out) under LSV;
   * selecting LSV for a DOUBLE barrier is a server `INVALID_ARGUMENT` (the LSV
   * engine prices single-barrier and window-barrier knock-outs only) — surfaced
   * honestly to the cell, never a silent fallback.
   */
  readonly model?: string | undefined;
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
  // The booking model travels uniformly on the instrument (proto field 22), so a
  // single- OR double-barrier ticket can be quoted under LSV (the server prices
  // single-barrier KO under LSV and rejects a double barrier with INVALID_ARGUMENT —
  // surfaced honestly, never silently). DEFAULT keeps the wire byte-identical.
  const pricingModel = parsePricingModel(args.model);
  const base = {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    pricingModel,
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

/** The fully-parsed inputs the WINDOWBARRIER family (CELNET.INSTRUMENT spec) shapes into a request. */
export interface WindowBarrierArgs {
  readonly pair: string | CcyPair;
  readonly tenor: string;
  readonly strikeOrDelta: string | number;
  readonly callPut: string;
  readonly notional: number;
  /** The knock-out barrier level (quote per 1 unit of base). */
  readonly barrier: number;
  /** UP (up-and-out, default) or DOWN (down-and-out) — a window barrier is always KO. */
  readonly side?: string | undefined;
  /** The start of the active window, in years (`>= 0`, default 0 — a "front" partial). */
  readonly windowStart?: number | undefined;
  /** The end of the active window, in years (`> windowStart`, `<= expiry`; default expiry — a "back" partial). */
  readonly windowEnd?: number | undefined;
  /** Optional MC antithetic pairs: `0`/absent ⇒ the exact ADI-PDE engine (no std-error). */
  readonly mcPairs?: number | undefined;
  /** Optional MC time steps (ignored when mcPairs == 0; `0` ⇒ server default). */
  readonly mcSteps?: number | undefined;
  /** Optional MC seed (bit-reproducible; ignored when mcPairs == 0). */
  readonly mcSeed?: number | undefined;
}

/** Validate a window endpoint in years (finite, non-negative). */
function shapeWindowYears(raw: number, what: string): number {
  if (!Number.isFinite(raw) || raw < 0) {
    throw new ShapingError(`${what} \`${raw}\` must be a finite, non-negative year fraction`);
  }
  return raw;
}

/**
 * Shape a window (partial-time) barrier from the cell arguments. A window barrier
 * is ALWAYS a knock-out that is active only inside `[windowStart, windowEnd] ⊆
 * [0, expiry]` and has no closed form, so it is shaped with `pricingModel`
 * LOCAL_STOCH_VOL already selected (the server rejects DEFAULT for it). The window
 * defaults to the full life (`[0, expiry]`); supplying only `windowStart` makes a
 * "back" partial, only `windowEnd` a "front" partial. `mcPairs > 0` selects the
 * Monte-Carlo engine (the reply then carries a `priceStdError`); `0` selects the
 * exact ADI-PDE engine. `side` on the instrument is TWO_WAY (the cell reads a
 * market). The function pre-selects LSV so the trader needs no separate model arg.
 */
export function shapeWindowBarrier(args: WindowBarrierArgs): Instrument {
  const { pair, tenor, expiryYears } = shapeSwapBase(args);
  const vanilla = {
    optionType: parseOptionType(args.callPut),
    strike: parseStrikeOrDelta(args.strikeOrDelta),
  };
  const windowStart = shapeWindowYears(args.windowStart ?? 0, "window start");
  const windowEnd = shapeWindowYears(args.windowEnd ?? expiryYears, "window end");
  if (windowEnd <= windowStart) {
    throw new ShapingError(
      `window end \`${windowEnd}\` must be strictly after window start \`${windowStart}\``,
    );
  }
  if (windowEnd > expiryYears) {
    throw new ShapingError(
      `window end \`${windowEnd}\` must not exceed the expiry \`${expiryYears}\` years`,
    );
  }
  const windowBarrier: WindowBarrier = {
    vanilla,
    barrier: shapeBarrierLevel(args.barrier, "barrier"),
    side: parseBarrierSide(args.side),
    windowStart,
    windowEnd,
    mcPairs: shapeMcPairs(args.mcPairs),
    mcSteps: shapeMcPairs(args.mcSteps),
    mcSeed: shapeMcSeed(args.mcSeed),
  };
  return {
    pair,
    tenor,
    expiryYears,
    quantity: { notional: args.notional, baseCcy: true },
    side: "TWO_WAY" as Side,
    pricingModel: "LOCAL_STOCH_VOL",
    product: { kind: "windowBarrier", windowBarrier },
  };
}

/** The fully-parsed inputs the DIGITAL family (CELNET.INSTRUMENT spec) shapes into a request. */
export interface DigitalArgs {
  readonly pair: string | CcyPair;
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

/** The fully-parsed inputs the TOUCH family (CELNET.INSTRUMENT spec) shapes into a request. */
export interface TouchArgs {
  readonly pair: string | CcyPair;
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
 * Pad a spill to a RECTANGULAR matrix — every row to the width of the widest row,
 * short rows right-filled with empty cells.
 *
 * Office.js custom functions REQUIRE a returned 2-D array to be rectangular (all
 * rows the same column count). A ragged return — e.g. a 4-column RFQ row followed
 * by a single-cell convention footer — fails in the Excel custom-functions runtime
 * with an opaque "add-in error", yet passes node/unit tests and the headless e2e,
 * which inspect the JS value directly and never round-trip it through the host's
 * matrix serializer (the same headless-passes / webview-fails trap as the
 * compiled-bundle root cause). Every `format*Spill` returns through here so the
 * invariant holds for all 24 product arms + the surface/risk/observability spills.
 */
export function rectangular(rows: SpillMatrix): SpillMatrix {
  let width = 0;
  for (const row of rows) if (row.length > width) width = row.length;
  return rows.map((row) =>
    row.length === width ? row : [...row, ...Array<string>(width - row.length).fill("")],
  );
}

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
 * The two rate-rho row labels for an underlying's asset class, mirroring the GUI's
 * `GreeksStrip.rhoGreeksFor`. The wire fields are unchanged — `rhoDom` is the
 * discount-rate rho, `rhoFor` is the carry rho the server computes (`b = r − carry`,
 * the carry seam's `RateSensitivities` arm losslessly projected to the flat pair) —
 * but their MEANING differs by class, so an equity never spills "rho_for": FX/metal
 * keep the two-rate domestic/foreign pair; equity shows rate + dividend-yield rho;
 * commodity rate + net-carry rho; crypto rate + funding rho. Labels only — the same
 * server-computed numbers, relabelled. An absent `underlying` (a legacy FX-only
 * instrument) keeps the FX pair. The carry seam reaching the Excel spill.
 */
function rhoRowLabels(underlying: Underlying | undefined): { dom: string; for_: string } {
  switch (underlying?.kind) {
    case "equity":
      return { dom: "rho_rate", for_: "rho_dividend_yield" };
    case "commodity":
      return { dom: "rho_rate", for_: "rho_net_carry" };
    case "digitalAsset":
      return { dom: "rho_rate", for_: "rho_funding" };
    case "fx":
    case "metal":
    case undefined:
    default:
      return { dom: "rho_dom", for_: "rho_for" };
  }
}

/**
 * The 13-Greek rows with the two rate-rho rows relabelled to the underlying's asset
 * class (FX/metal keep `rho_dom`/`rho_for`). FX is byte-identical to {@link GREEK_ROWS}.
 */
export function greekRowsFor(
  underlying: Underlying | undefined,
): readonly { readonly label: string; readonly key: keyof Greeks }[] {
  const { dom, for_ } = rhoRowLabels(underlying);
  return GREEK_ROWS.map((r) =>
    r.key === "rhoDom"
      ? { label: dom, key: r.key }
      : r.key === "rhoFor"
        ? { label: for_, key: r.key }
        : r,
  );
}

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
  /**
   * The instrument's underlying, so the rate-rho rows carry the asset-class-correct
   * carry label (e.g. an equity's `rho_dividend_yield`). Omitted ⇒ the FX
   * `rho_dom`/`rho_for` pair (byte-identical to the pre-carry-seam spill).
   */
  underlying?: Underlying,
): SpillMatrix {
  const rows: SpillMatrix = greekRowsFor(underlying).map((r) => [r.label, greeks[r.key]]);
  rows.push([conventionFooter(conv, surfaceVersion, epochNanos)]);
  return rectangular(rows);
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
  return rectangular([header, vols, footer]);
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
  return rectangular(rows);
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

/** Render a last-look deadline (nanos since the epoch) as ISO-8601; `0` ⇒ `—`. */
function validUntilIso(nanos: bigint): string {
  return nanos === 0n ? "—" : new Date(Number(nanos / 1_000_000n)).toISOString();
}

/**
 * Format CELNET.RFQ as a 1×4 horizontal spill `[bid, offer, quoteId, validUntil]`
 * followed by a convention footer row. `quoteId`/`validUntil` are rendered as
 * strings to avoid JS number precision loss on the 64-bit wire ids.
 */
export function formatRfqSpill(r: RfqResult): SpillMatrix {
  return rectangular([
    [r.bid, r.offer, r.quoteId.toString(), validUntilIso(r.validUntilNanos)],
    [conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)],
  ]);
}

/** One ranked LP line a CELNET.RFQ panel spill renders (server ranking order). */
export interface RfqPanelLine {
  readonly lpId: string;
  readonly bid: number;
  readonly offer: number;
  readonly validUntilNanos: bigint;
}

/** The decoded fields a CELNET.RFQ multi-dealer panel spill renders. */
export interface RfqPanelResult {
  /** The aggregate request id an accept echoes together with a line's `lpId`. */
  readonly quoteId: bigint;
  /** The dealer lines VERBATIM in the server aggregator's order (best-first). */
  readonly lines: readonly RfqPanelLine[];
  /** The touch dealers the aggregator named (empty ⇒ no quote on that side). */
  readonly bestBidLpId: string;
  readonly bestOfferLpId: string;
  readonly conventions: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}

/**
 * Format a CELNET.RFQ multi-dealer panel as a labelled spill: a header row
 * `[lp_id, bid, offer, valid_until, best]`, then ONE ROW PER LP in the server's
 * ranking order (never re-sorted client-side), the touch rows marked `BEST_BID`
 * / `BEST_OFFER` (one row may be both), then a `["quote_id", id]` row (the id as
 * a string — 64-bit precision — referenceable by an accept workflow together
 * with a row's `lp_id`), then the convention footer.
 */
export function formatRfqPanelSpill(r: RfqPanelResult): SpillMatrix {
  const rows: SpillMatrix = [["lp_id", "bid", "offer", "valid_until", "best"]];
  for (const line of r.lines) {
    const markers: string[] = [];
    if (line.lpId === r.bestBidLpId) markers.push("BEST_BID");
    if (line.lpId === r.bestOfferLpId) markers.push("BEST_OFFER");
    rows.push([line.lpId, line.bid, line.offer, validUntilIso(line.validUntilNanos), markers.join("+")]);
  }
  rows.push(["quote_id", r.quoteId.toString()]);
  rows.push([conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)]);
  return rectangular(rows);
}

/** The decoded fields a VARSWAP-family spill renders (fair variance + fair vol). */
export interface VarSwapResult {
  /** The fair variance strike `K_var` (the server's `resolved_strike` / `greeks.price`). */
  readonly fairVariance: number;
  readonly conventions: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}

/**
 * Format a VARSWAP-family quote as a labelled 2×2 spill — `["fair_variance", K_var]` and
 * `["fair_vol", √K_var]` — followed by a convention footer. The fair vol is the
 * √ of the fair variance the server returns; both are shown so the desk reads the
 * variance strike AND its vol-equivalent without a hidden √.
 */
export function formatVarSwapSpill(r: VarSwapResult): SpillMatrix {
  const fairVol = r.fairVariance >= 0 ? Math.sqrt(r.fairVariance) : Number.NaN;
  return rectangular([
    ["fair_variance", r.fairVariance],
    ["fair_vol", fairVol],
    [conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)],
  ]);
}

/** The decoded fields a VOLSWAP-family spill renders (the convexity-adjusted fair vol). */
export interface VolSwapResult {
  /** The fair volatility strike `K_vol` (the server's `resolved_strike` / `greeks.price`). */
  readonly fairVol: number;
  readonly conventions: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}

/**
 * Format a VOLSWAP-family quote as a labelled 1×2 spill `["fair_vol", K_vol]` followed by
 * a convention footer. `K_vol` is the convexity-adjusted fair volatility strike,
 * strictly below `√K_var` for any non-degenerate smile.
 */
export function formatVolSwapSpill(r: VolSwapResult): SpillMatrix {
  return rectangular([
    ["fair_vol", r.fairVol],
    [conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)],
  ]);
}

/**
 * The decoded fields a premium-headline product spill renders: the discounted PV
 * (the server's `greeks.price`), the OPTIONAL Monte-Carlo standard error, the 13
 * risk Greeks, and the convention footer. This is the ONE shared spill geometry
 * for every product whose headline is a premium (barrier, window barrier, digital,
 * touch, Asian, forward-start, cliquet, quanto, TARF, pivot TRA, accumulator,
 * lookback, American, basket, forward/swap/NDF, and a vanilla priced via an
 * instrument token). The swaps (variance/volatility) keep their fair-strike spills.
 */
export interface PremiumResult {
  /** The discounted premium (the server's `greeks.price`). */
  readonly premium: number;
  /**
   * The Monte-Carlo standard error of the premium (the server's
   * `price_std_error`), present ONLY for an MC-priced request (TARF / pivot TRA
   * / accumulator / basket always; clamped cliquet, DISCRETE lookback, LSM
   * American, MC window barrier, LSV-MC barrier when stamped); absent/`undefined`
   * for every exact
   * closed-form / PDE / FD price. Surfaced honestly so an MC premium is never
   * mistaken for closed-form precision — and vice-versa.
   */
  readonly stdError?: number | undefined;
  readonly greeks: Greeks;
  readonly conventions: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}

/**
 * Format a premium-headline product as a labelled spill: `["premium", PV]`, then —
 * ONLY when a Monte-Carlo standard error is supplied — a `["std_error", σ̄]` row,
 * then the 13 risk Greeks (the same set/order as CELNET.GREEKS), then a convention
 * footer. The geometry is honest about whether the headline carries MC noise: the
 * caller gates `stdError` on the shaped product (e.g. `cliquetIsMonteCarlo`), so a
 * closed-form price never surfaces a stray precision claim.
 */
export function formatPremiumSpill(r: PremiumResult): SpillMatrix {
  const rows: SpillMatrix = [["premium", r.premium]];
  if (r.stdError !== undefined) rows.push(["std_error", r.stdError]);
  for (const g of GREEK_ROWS) rows.push([g.label, r.greeks[g.key]]);
  rows.push([conventionFooter(r.conventions, r.surfaceVersion, r.epochNanos)]);
  return rectangular(rows);
}

/**
 * Format a calibrated smile (the result of a model-selected `mark_surface`) as a
 * spill: a delta-pillar header row, a vol row, then a footer carrying the marked
 * `surface_version`, the model provenance (the TYPED `ArbReport.smileModel` the
 * server stamps from the calibrated smile itself), the arb status, and the
 * convention transparency. The `requestedModel` is the model the cell REQUESTED;
 * `actualModel` is what the surface was actually calibrated under — the footer
 * shows `actualModel` and flags a mismatch so a silent server default is visible,
 * never assumed. We read the typed field, never the legacy `model=` note token.
 */
export function formatCalibratedSmileSpill(args: {
  readonly points: readonly SurfacePoint[];
  readonly requestedModel: SmileModel;
  readonly actualModel: SmileModel;
  readonly arbFree: boolean;
  readonly conv: Conventions;
  readonly surfaceVersion: bigint | undefined;
  readonly epochNanos: bigint;
}): SpillMatrix {
  const sorted = [...args.points].sort((a, b) => a.delta - b.delta);
  const header: (string | number)[] = ["delta", ...sorted.map((p) => p.delta)];
  const vols: (string | number)[] = ["vol", ...sorted.map((p) => p.vol)];
  // The authoritative provenance is the typed `smile_model` the server stamped on
  // the calibrated smile. Show it directly; when it differs from the requested
  // model (the surface was last marked under another family) flag the divergence
  // so the trader never mistakes a stale family for the one they asked for.
  const mismatch =
    args.actualModel !== args.requestedModel ? ` (requested ${args.requestedModel})` : "";
  const footer: (string | number)[] = [
    `model ${args.actualModel}${mismatch} | ${args.arbFree ? "arb-free" : "ARB!"} | ` +
      conventionFooter(args.conv, args.surfaceVersion, args.epochNanos),
  ];
  return rectangular([header, vols, footer]);
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

// ---------------------------------------------------------------------------
// fixed-income live streaming (CELNET.RATESSERIES): the streamable observable a
// rates line surfaces + the single-cell render. A streamed `RatesPricingResult`
// carries every measure at once (PV / par_rate / PV01 / DV01), so a live rates cell
// PROJECTS one of them, chosen by the observable selector (the analogue of the FX
// SERIES observable). The value is rendered as a display string (like SUBSCRIBE's
// two-way and SERIES's trend point), health-tagged so a stale line shows its
// last-good number dimmed, never frozen-as-live (docs §5).
// ---------------------------------------------------------------------------

/** The streamable observable a `CELNET.RATESSERIES` cell projects from the line's result. */
export type RatesObservable = "PV" | "PAR_RATE" | "PV01" | "DV01";

/**
 * Parse the rates-observable selector: PV (present value, the default), PAR /
 * PAR_RATE / RATE (the par/fair rate), PV01 (analytic PV01), or DV01. Accepts the
 * canonical names + desk short forms, case-/separator-insensitive. Empty/absent ⇒
 * PV (the line's headline measure).
 */
export function parseRatesObservable(raw: string | undefined): RatesObservable {
  if (raw === undefined || raw.trim() === "") return "PV";
  const t = raw.trim().toUpperCase().replace(/[._\s-]/g, "");
  switch (t) {
    case "PV":
    case "NPV":
    case "PRESENTVALUE":
      return "PV";
    case "PAR":
    case "PARRATE":
    case "RATE":
    case "FAIR":
    case "FAIRRATE":
      return "PAR_RATE";
    case "PV01":
      return "PV01";
    case "DV01":
      return "DV01";
    default:
      throw new ShapingError(`invalid rates observable \`${raw}\` (expected PV, PAR, PV01 or DV01)`);
  }
}

/** Select the chosen measure off a priced rates result. */
function selectRatesObservable(result: RatesPricingResult, observable: RatesObservable): number {
  switch (observable) {
    case "PV":
      return result.pv;
    case "PAR_RATE":
      return result.parRate;
    case "PV01":
      return result.pv01;
    case "DV01":
      return result.dv01;
  }
}

/** A decoded rates streaming tick for a single live cell render. */
export interface RatesSeriesCellInput {
  /** The latest priced result; `null`/absent until the opening snapshot baselines. */
  readonly result: RatesPricingResult | null;
  /** The observable this cell projects. */
  readonly observable: RatesObservable;
  /** The line health (HEALTHY / RESYNCING / STALE). */
  readonly health: StreamHealth;
  /** True once a baseline result has been seeded. */
  readonly baselined: boolean;
}

/**
 * Render a streamed rates tick to a single cell string. The par rate is shown as a
 * percentage (4dp of a percent, e.g. `4.1000%`); PV / PV01 / DV01 are currency
 * amounts (2dp for a value ≥ 1, else 6dp so a small sensitivity stays visible). An
 * un-baselined line renders an explicit waiting marker; a STALE line shows its
 * last-good number dimmed (`… <n> (stale)`); a resyncing line is tagged `(resync)`.
 */
export function formatRatesSeriesCell(t: RatesSeriesCellInput): string {
  if (!t.baselined || t.result === null) return "… (awaiting)";
  const v = selectRatesObservable(t.result, t.observable);
  if (!Number.isFinite(v)) return "… (awaiting)";
  const body =
    t.observable === "PAR_RATE"
      ? `${(v * 100).toFixed(4)}%`
      : Math.abs(v) >= 1
        ? v.toFixed(2)
        : v.toFixed(6);
  if (t.health === "STALE") return `… ${body} (stale)`;
  const tag = t.health === "RESYNCING" ? " (resync)" : "";
  return `${body}${tag}`;
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
// server observability (heartbeat) spill formatting
// ---------------------------------------------------------------------------

/**
 * Render a server-latency percentile (nanoseconds on the wire) for the status
 * spill: microseconds with two decimals so a desk reads it as a familiar µs
 * figure. A zero (no price timed yet on the streaming edge) renders an honest
 * waiting marker rather than a misleading "0.00µs".
 */
function formatLatencyNanos(nanos: bigint): string {
  if (nanos === 0n) return "—";
  // bigint → µs with 2dp; divide in integer hundredths-of-µs to avoid any float
  // drift on the (ms-resolution) wire values, then place the decimal point.
  const hundredthsMicros = (nanos * 100n) / 1000n; // ns → µs×100, truncated
  const whole = hundredthsMicros / 100n;
  const frac = (hundredthsMicros % 100n).toString().padStart(2, "0");
  return `${whole}.${frac}µs`;
}

/**
 * Format CELNET.STATUS as a 2-row server-observability spill: a header row and a
 * value row carrying the live connection state plus the latest server heartbeat's
 * drain-side price latency p50/p99/p99.9, the `celnet-fanout` ring conflation-drop
 * count, and the surface-version / correlation provenance echo. Every value is the
 * server's own observability stamp (HdrHistogram + the ring's
 * `received + skipped == produced` accounting); nothing is computed client-side.
 * Before the first beat arrives the value row is an honest waiting state.
 */
export function formatServerStatusSpill(open: boolean, hb: Heartbeat | undefined): SpillMatrix {
  const header: (string | number)[] = [
    "connection",
    "price_p50",
    "price_p99",
    "price_p99.9",
    "conflation_drops",
    "surface",
    "correlation",
  ];
  const link = open ? "LIVE" : "DOWN";
  if (hb === undefined) {
    return [header, [link, "…", "…", "…", "(awaiting beat)", "—", "—"]];
  }
  return [
    header,
    [
      link,
      formatLatencyNanos(hb.serverPriceP50Nanos),
      formatLatencyNanos(hb.serverPriceP99Nanos),
      formatLatencyNanos(hb.serverPriceP999Nanos),
      // The exact ring skip count; 0 ⇒ never lagged (honest, not absence).
      hb.conflationDrops.toString(),
      hb.surfaceVersion === 0n ? "live" : `v${hb.surfaceVersion}`,
      hb.correlationId === 0n ? "—" : hb.correlationId.toString(),
    ],
  ];
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
  return rectangular(rows);
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
  return rectangular(rows);
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
  return rectangular(rows);
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
    // The tenor is presence-tracked (absent only for the tenorless perpetual);
    // `null` is its canonical absent form. A dated instrument keys identically
    // to before the field became optional.
    tu: i.tenor?.unit ?? null,
    tc: i.tenor?.count ?? null,
    ey: i.expiryYears,
    n: i.quantity.notional,
    bc: i.quantity.baseCcy,
    side: i.side,
    // The booking model materially changes the priced value (LSV vs analytic for
    // the supported products), so two cells that differ only in model must NOT
    // coalesce onto one subscription. Absent ⇒ DEFAULT (the canonical default).
    pm: i.pricingModel ?? "DEFAULT",
    // The cross-asset identity: two instruments that share a pair PROJECTION but
    // differ in the underlying arm (e.g. an equity ticker colliding with a crypto
    // coin) or in settlement mechanics (LINEAR vs the coin-margined 1/S_T form)
    // must NOT coalesce. Absent ⇒ the FX projection / LINEAR (canonical defaults).
    u: i.underlying ?? null,
    ss: i.settlementStyle ?? "LINEAR",
    product,
  };
}

/** The order-stable canonical form of the product oneof for the coalescing key. */
function canonicalProduct(p: Product): unknown {
  switch (p.kind) {
    case "vanilla":
      return { k: "v", ot: p.vanilla.optionType, s: canonicalStrike(p.vanilla.strike) };
    case "strategy":
      // The ladder is the structure's identity: two same-kind strategies with
      // different legs must NOT coalesce onto one subscription.
      return {
        k: "s",
        kind: p.strategy.kind,
        legs: p.strategy.legs.map((l) => ({
          ot: l.optionType,
          s: canonicalStrike(l.strike),
          side: l.side,
          r: l.ratio,
        })),
      };
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
    case "pivot":
      return {
        k: "pvt",
        ot: p.pivot.optionType,
        strike: p.pivot.strike,
        pv: p.pivot.pivot,
        tgt: p.pivot.target,
        lev: p.pivot.leverage,
        red: p.pivot.redemption,
        fy: p.pivot.schedule.fixingYears,
        fn: p.pivot.schedule.fixingNotional,
        mcp: p.pivot.mcPairs,
        // The MC seed is a bigint; stringify so the coalescing key is JSON-serialisable.
        mcs: p.pivot.mcSeed.toString(),
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
    case "windowBarrier":
      return {
        k: "wbar",
        ot: p.windowBarrier.vanilla.optionType,
        strike: canonicalStrike(p.windowBarrier.vanilla.strike),
        bar: p.windowBarrier.barrier,
        bs: p.windowBarrier.side,
        ws: p.windowBarrier.windowStart,
        we: p.windowBarrier.windowEnd,
        mcp: p.windowBarrier.mcPairs,
        mst: p.windowBarrier.mcSteps,
        mcs: p.windowBarrier.mcSeed.toString(),
      };
    case "american":
      return {
        k: "amr",
        ot: p.american.optionType,
        strike: p.american.strike,
        ex: p.american.exerciseStyle,
        bd: [...p.american.bermudanDates],
        lp: p.american.lsmPaths,
        led: p.american.lsmExerciseDates,
        ls: p.american.lsmSeed.toString(),
      };
    case "basket":
      return {
        k: "bskt",
        ot: p.basket.optionType,
        strike: p.basket.strike,
        bk: p.basket.kind,
        legs: p.basket.legs.map((l) => [l.pair.base, l.pair.quote, l.weight, l.spot, l.vol, l.rFor]),
        cor: [...p.basket.correlations],
        mcp: p.basket.mcPaths,
        mcr: p.basket.mcReplications,
        mst: p.basket.mcSteps,
        mcs: p.basket.mcSeed.toString(),
      };
    case "fxForward":
      return {
        k: "fwd",
        cr: p.fxForward.contractRate,
        n: p.fxForward.notional,
        sd: p.fxForward.side,
      };
    case "fxSwap":
      return {
        k: "swp",
        ncr: p.fxSwap.near.contractRate,
        nn: p.fxSwap.near.notional,
        nsd: p.fxSwap.near.side,
        fcr: p.fxSwap.far.contractRate,
        fn: p.fxSwap.far.notional,
        fsd: p.fxSwap.far.side,
      };
    case "ndf":
      return {
        k: "ndf",
        cr: p.ndf.contractRate,
        n: p.ndf.notional,
        sd: p.ndf.side,
        fx: p.ndf.fixing,
        sc: p.ndf.settlementCcy,
      };
    case "perpetualOption":
      return {
        k: "perp",
        ot: p.perpetualOption.optionType,
        strike: p.perpetualOption.strike,
        n: p.perpetualOption.notional,
      };
    case "listedFutureOption":
      return {
        k: "lfo",
        ft: p.listedFutureOption.futureSymbol.ticker,
        fv: p.listedFutureOption.futureSymbol.venue,
        fe: p.listedFutureOption.futureExpiryYears,
        ot: p.listedFutureOption.optionType,
        strike: p.listedFutureOption.strike,
        n: p.listedFutureOption.notional,
        mg: p.listedFutureOption.margining,
      };
  }
}

function canonicalStrike(s: StrikeOrDelta): unknown {
  return s.kind === "strike" ? { k: "strike", v: s.strike } : { k: "delta", v: s.delta };
}

function canonicalConventions(c: Conventions): unknown {
  return [c.deltaConvention, c.atmConvention, c.premiumStyle, c.cut, c.dayCount, c.settlement];
}

// ---------------------------------------------------------------------------
// fixed-income (rates) — parse a curve range + scalar args into the typed
// `RatesCurveSet` + `OisInstrument` the `price_rates` RPC carries, and format the
// server's `RatesPricingResult` as a labelled spill. The add-in holds NO rates
// math: these helpers only validate and shape the inputs the `celnet-rates`
// engine prices, and lay out the engine's result for the grid.
// ---------------------------------------------------------------------------

/** The default curve currency — the USD-SOFR P0 arm (the only arm the engine prices). */
const DEFAULT_CURVE_CURRENCY = "USD";

/** Excel's serial-date epoch: serial 0 is 1899-12-30 (UTC), so day `n` is that plus `n` days. */
const EXCEL_EPOCH_MS = Date.UTC(1899, 11, 30);
const MS_PER_DAY = 86_400_000;

/** Read a curve cell as a finite number, accepting a numeric or numeric-string cell. */
function ratesCell(value: string | number | boolean, what: string): number {
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new ShapingError(`${what} must be a finite number`);
    return value;
  }
  if (typeof value === "boolean") throw new ShapingError(`${what} must be a number, not a boolean`);
  const s = value.trim();
  if (s === "") throw new ShapingError(`${what} is empty`);
  const n = Number(s);
  if (!Number.isFinite(n)) throw new ShapingError(`${what} \`${value}\` is not a number`);
  return n;
}

/** True when a curve row is entirely empty (Excel pads ranges with blank trailing cells). */
function isEmptyCurveRow(row: readonly (string | number | boolean)[]): boolean {
  return row.every((c) => c === "" || c === null || c === undefined);
}

/**
 * Parse the `reference_date` (spot-anchor civil date). Accepts an Excel date
 * serial (the default for a date-typed cell) or an ISO `YYYY-MM-DD` string.
 */
export function parseBrokenDate(raw: number | string): BrokenDate {
  if (typeof raw === "number") {
    if (!Number.isFinite(raw) || raw <= 0) {
      throw new ShapingError(`invalid reference date serial \`${raw}\``);
    }
    const d = new Date(EXCEL_EPOCH_MS + Math.trunc(raw) * MS_PER_DAY);
    return { year: d.getUTCFullYear(), month: d.getUTCMonth() + 1, day: d.getUTCDate() };
  }
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(raw.trim());
  if (!m) {
    throw new ShapingError(`invalid reference date \`${raw}\` (expected an Excel date or YYYY-MM-DD)`);
  }
  const year = Number(m[1]);
  const month = Number(m[2]);
  const day = Number(m[3]);
  if (month < 1 || month > 12) throw new ShapingError(`reference date month out of range in \`${raw}\``);
  if (day < 1 || day > 31) throw new ShapingError(`reference date day out of range in \`${raw}\``);
  return { year, month, day };
}

/** Parse an OIS tenor as a whole number of years (`>= 1`); accepts `5` or `"5Y"`. */
export function parseOisTenorYears(raw: number | string): number {
  let n: number;
  if (typeof raw === "number") {
    n = raw;
  } else {
    const m = /^(\d+)\s*Y?$/.exec(raw.trim().toUpperCase());
    if (!m) throw new ShapingError(`invalid OIS tenor \`${raw}\` (expected whole years, e.g. 5 or 5Y)`);
    n = Number(m[1]);
  }
  if (!Number.isInteger(n) || n < 1) {
    throw new ShapingError(`OIS tenor must be a whole number of years >= 1 (got \`${raw}\`)`);
  }
  return n;
}

/**
 * Parse the fixed-leg direction. `PAY_FIXED` (payer) / `RECEIVE_FIXED` (receiver),
 * with the common desk aliases (PAY/PAYER, RECEIVE/REC/RECEIVER).
 */
export function parseOisDirection(raw: string): OisDirection {
  const s = raw.trim().toUpperCase();
  if (s === "PAY_FIXED" || s === "PAYFIXED" || s === "PAY" || s === "PAYER") return "PAY_FIXED";
  if (
    s === "RECEIVE_FIXED" ||
    s === "RECEIVEFIXED" ||
    s === "RECEIVE" ||
    s === "REC" ||
    s === "RECEIVER"
  ) {
    return "RECEIVE_FIXED";
  }
  throw new ShapingError(`invalid OIS direction \`${raw}\` (expected PAY_FIXED or RECEIVE_FIXED)`);
}

/** Validate a positive rates notional (direction carries the sign, never the notional). */
function shapeRatesNotional(raw: number, product: string): number {
  if (!Number.isFinite(raw) || raw <= 0) {
    throw new ShapingError(`${product} notional must be a positive number (got \`${raw}\`)`);
  }
  return raw;
}

/** Validate + normalise the curve currency (3-letter ISO-4217; default USD). */
function parseCurveCurrency(raw: string | undefined): string {
  if (raw === undefined || raw.trim() === "") return DEFAULT_CURVE_CURRENCY;
  const s = raw.trim().toUpperCase();
  if (!/^[A-Z]{3}$/.test(s)) throw new ShapingError(`invalid currency \`${raw}\` (expected a 3-letter ISO code)`);
  return s;
}

/**
 * Parse the curve range — a 2-column `[tenorYears, parRate]` grid, one row per
 * self-discounting OIS pillar — into strictly-increasing-tenor pillars. Blank
 * trailing rows are ignored; out-of-order or duplicate tenors are rejected
 * (the engine bootstraps deterministically only from ordered pillars).
 */
function parseOisPillars(curve: readonly (readonly (string | number | boolean)[])[]): OisCurvePillar[] {
  const pillars: OisCurvePillar[] = [];
  for (const row of curve) {
    if (isEmptyCurveRow(row)) continue;
    if (row.length < 2) {
      throw new ShapingError("each curve row must have 2 columns: [tenorYears, parRate]");
    }
    const tenorYears = parseOisTenorYears(ratesCell(row[0]!, "pillar tenorYears"));
    const parRate = ratesCell(row[1]!, "pillar parRate");
    const prev = pillars[pillars.length - 1];
    if (prev !== undefined && tenorYears <= prev.tenorYears) {
      throw new ShapingError(
        `curve pillars must be in strictly increasing tenor order (\`${prev.tenorYears}Y\` then \`${tenorYears}Y\`)`,
      );
    }
    pillars.push({ tenorYears, parRate });
  }
  if (pillars.length === 0) throw new ShapingError("the curve needs at least one OIS pillar");
  return pillars;
}

/** The curve-range + reference-date + currency a CELNET.RATES call shapes into a `RatesCurveSet`. */
export interface RatesCurveArgs {
  /** The 2-column `[tenorYears, parRate]` curve range (one row per pillar). */
  readonly curve: readonly (readonly (string | number | boolean)[])[];
  /** The spot-anchor civil date (Excel date serial or `YYYY-MM-DD`). */
  readonly referenceDate: number | string;
  /** ISO-4217 currency; defaults to USD (the only priced arm). */
  readonly currency?: string | undefined;
}

/** Shape a curve range + reference date into the typed `RatesCurveSet`. */
export function shapeRatesCurve(args: RatesCurveArgs): RatesCurveSet {
  return {
    currency: parseCurveCurrency(args.currency),
    referenceDate: parseBrokenDate(args.referenceDate),
    pillars: parseOisPillars(args.curve),
  };
}

/** The scalar OIS terms a CELNET.RATES call shapes into an `OisInstrument`. */
export interface OisInstrumentArgs {
  /** The swap tenor in whole years (`5` or `"5Y"`). */
  readonly tenor: number | string;
  /** The fixed-leg rate as a decimal (0.041 = 4.10%). */
  readonly fixedRate: number;
  /** Pay-fixed (payer) or receive-fixed (receiver). */
  readonly direction: string;
  /** The (positive) notional in the curve currency. */
  readonly notional: number;
}

/** Shape the scalar OIS terms into the typed `OisInstrument`. */
export function shapeOisInstrument(args: OisInstrumentArgs): OisInstrument {
  if (!Number.isFinite(args.fixedRate)) {
    throw new ShapingError(`OIS fixed rate must be a finite decimal (got \`${args.fixedRate}\`)`);
  }
  return {
    tenorYears: parseOisTenorYears(args.tenor),
    fixedRate: args.fixedRate,
    notional: shapeRatesNotional(args.notional, "OIS"),
    direction: parseOisDirection(args.direction),
  };
}

/**
 * Format a `RatesPricingResult` as a labelled vertical spill: `pv`, `par_rate`,
 * `pv01`, `dv01`, then the key-rate DV01 ladder — one `kr_dv01[<tenor>Y]` row per
 * curve pillar, in pillar order (the ladder sums to `dv01` to first order). When
 * the engine's ladder length matches the curve pillars the rows are tenor-labelled;
 * a length mismatch falls back to positional `kr_dv01[i]` labels (never silently
 * dropped). Returns a rectangular `(4 + pillars)×2` matrix.
 */
export function formatRatesSpill(
  result: RatesPricingResult,
  pillars: readonly OisCurvePillar[],
): SpillMatrix {
  const rows: SpillMatrix = [
    ["pv", result.pv],
    ["par_rate", result.parRate],
    ["pv01", result.pv01],
    ["dv01", result.dv01],
  ];
  const tenorLabelled = result.keyRateLadder.length === pillars.length;
  result.keyRateLadder.forEach((value, i) => {
    const label = tenorLabelled ? `kr_dv01[${pillars[i]!.tenorYears}Y]` : `kr_dv01[${i}]`;
    rows.push([label, value]);
  });
  return rectangular(rows);
}

// ---------------------------------------------------------------------------
// fixed-income (rates) — swap/FRA arms: parse the scalar IRS + FRA terms into the
// typed `VanillaIrsInstrument` / `FraInstrument` the `price_rates` RPC carries on
// its `RatesInstrument.irs` / `.fra` oneof arms. Both reuse the OIS curve-set + the
// OIS result spill (`formatRatesSpill`): a swap and a FRA return the same
// `RatesPricingResult` shape (pv/par_rate/pv01/dv01 + a pillar-shaped key-rate
// ladder). The add-in holds NO rates math — it only shapes the inputs the live
// `celnet-rates` engine prices and lays out its authoritative result.
// ---------------------------------------------------------------------------

/**
 * Parse a leg / coupon payment frequency (`PaymentFrequency`) to the canonical
 * name. Accepts the contract names and desk short/synonym forms, case-/separator-
 * insensitive: ANNUAL/ANN/A/1Y/1; SEMI_ANNUAL/SEMI/SA/6M/2; QUARTERLY/QTR/Q/3M/4.
 * Empty/absent ⇒ the supplied `fallback` (the leg's market default).
 */
export function parsePaymentFrequency(
  raw: string | number | undefined,
  fallback: PaymentFrequency,
): PaymentFrequency {
  if (raw === undefined || raw === "") return fallback;
  const t = String(raw).trim().toUpperCase().replace(/[._\s-]/g, "");
  switch (t) {
    case "ANNUAL":
    case "ANN":
    case "A":
    case "1Y":
    case "1":
      return "ANNUAL";
    case "SEMIANNUAL":
    case "SEMI":
    case "SA":
    case "6M":
    case "2":
      return "SEMI_ANNUAL";
    case "QUARTERLY":
    case "QTR":
    case "Q":
    case "3M":
    case "4":
      return "QUARTERLY";
    default:
      throw new ShapingError(
        `invalid payment frequency \`${raw}\` (expected ANNUAL, SEMI_ANNUAL or QUARTERLY)`,
      );
  }
}

/**
 * Parse an instrument-level accrual basis (`RatesAccrualBasis` / the bond + FRA
 * accrual day-count). Accepts the contract names + desk short forms, case-/
 * separator-insensitive: ACT_360/ACT360; ACT_365_FIXED/ACT365/ACT365F;
 * THIRTY_360_BOND_BASIS/30_360/30360/30/BONDBASIS/BOND. Empty/absent ⇒ `fallback`.
 */
export function parseRatesAccrualBasis(
  raw: string | undefined,
  fallback: RatesAccrualBasis,
): RatesAccrualBasis {
  if (raw === undefined || raw.trim() === "") return fallback;
  const t = raw.trim().toUpperCase().replace(/[._\s/-]/g, "");
  switch (t) {
    case "ACT360":
    case "ACTUAL360":
      return "ACT_360";
    case "ACT365FIXED":
    case "ACT365F":
    case "ACT365":
    case "ACTUAL365FIXED":
      return "ACT_365_FIXED";
    case "THIRTY360BONDBASIS":
    case "30360BONDBASIS":
    case "30360":
    case "30":
    case "BONDBASIS":
    case "BOND":
      return "THIRTY_360_BOND_BASIS";
    default:
      throw new ShapingError(
        `invalid day count \`${raw}\` (expected ACT_360, ACT_365_FIXED or THIRTY_360_BOND_BASIS)`,
      );
  }
}

/**
 * Parse an IRS leg day-count (`RatesLegDayCount`) — the money-market subset a swap
 * leg accrues on: ACT_360 or ACT_365_FIXED only (an IRS leg carries no 30/360 arm,
 * matching the server `VanillaIrsInstrument`). Accepts the same short forms as the
 * accrual basis; 30/360 is rejected with an explicit reason (not silently coerced).
 * Empty/absent ⇒ `fallback` (the leg's market default).
 */
export function parseRatesLegDayCount(
  raw: string | undefined,
  fallback: RatesLegDayCount,
): RatesLegDayCount {
  if (raw === undefined || raw.trim() === "") return fallback;
  const t = raw.trim().toUpperCase().replace(/[._\s/-]/g, "");
  switch (t) {
    case "ACT360":
    case "ACTUAL360":
      return "ACT_360";
    case "ACT365FIXED":
    case "ACT365F":
    case "ACT365":
    case "ACTUAL365FIXED":
      return "ACT_365_FIXED";
    case "THIRTY360BONDBASIS":
    case "30360BONDBASIS":
    case "30360":
    case "30":
    case "BONDBASIS":
    case "BOND":
      throw new ShapingError(
        `invalid IRS leg day count \`${raw}\` (a swap leg accrues ACT_360 or ACT_365_FIXED — 30/360 is not a leg basis)`,
      );
    default:
      throw new ShapingError(
        `invalid IRS leg day count \`${raw}\` (expected ACT_360 or ACT_365_FIXED)`,
      );
  }
}

/**
 * Parse the fixed-leg direction with an optional fallback for the swap/FRA cells
 * (where `side` is an optional argument). Empty/absent ⇒ `fallback` (default
 * PAY_FIXED — a bought swap pays fixed, matching the wire `Side` zero = SIDE_BUY);
 * a present value is parsed by {@link parseOisDirection} (PAY/RECEIVE + aliases).
 */
export function parseRatesSide(
  raw: string | undefined,
  fallback: OisDirection = "PAY_FIXED",
): OisDirection {
  if (raw === undefined || raw.trim() === "") return fallback;
  return parseOisDirection(raw);
}

/** Parse a FRA window tenor as a whole number of months (`>= 0`); accepts `3` or `"3M"`. */
export function parseFraMonths(raw: number | string, label: string): number {
  let n: number;
  if (typeof raw === "number") {
    n = raw;
  } else {
    const m = /^(\d+)\s*M?$/.exec(raw.trim().toUpperCase());
    if (!m) {
      throw new ShapingError(`invalid FRA ${label} \`${raw}\` (expected whole months, e.g. 3 or 3M)`);
    }
    n = Number(m[1]);
  }
  if (!Number.isInteger(n) || n < 0) {
    throw new ShapingError(`FRA ${label} must be a whole number of months >= 0 (got \`${raw}\`)`);
  }
  return n;
}

/** The scalar IRS terms a CELNET.IRS call shapes into a `VanillaIrsInstrument`. */
export interface IrsInstrumentArgs {
  /** The swap tenor in whole years (`5` or `"5Y"`). */
  readonly tenor: number | string;
  /** The fixed-leg rate as a decimal (0.041 = 4.10%). */
  readonly fixedRate: number;
  /** The (positive) notional in the curve currency. */
  readonly notional: number;
  /** Pay-fixed (payer) or receive-fixed (receiver); absent ⇒ PAY_FIXED. */
  readonly side?: string | undefined;
  /** Fixed-leg payment frequency; absent ⇒ SEMI_ANNUAL (USD market). */
  readonly fixedFrequency?: string | undefined;
  /** Fixed-leg accrual day-count; absent ⇒ ACT_360. */
  readonly fixedDayCount?: string | undefined;
  /** Float-leg payment frequency; absent ⇒ QUARTERLY (USD market). */
  readonly floatFrequency?: string | undefined;
  /** Float-leg accrual day-count; absent ⇒ ACT_360. */
  readonly floatDayCount?: string | undefined;
}

/**
 * Shape the scalar IRS terms into the typed `VanillaIrsInstrument`. The fixed rate
 * must be a finite decimal; the notional must be strictly positive (direction carries
 * the sign). The leg conventions default to the USD-market swap (SEMI_ANNUAL/ACT_360
 * fixed vs QUARTERLY/ACT_360 float) when omitted. The tenor is validated as a whole
 * number of years `>= 1` (the swap is spot-starting, its schedule rebuilt server-side
 * from the curve reference date + tenor).
 */
export function shapeIrsInstrument(args: IrsInstrumentArgs): VanillaIrsInstrument {
  if (!Number.isFinite(args.fixedRate)) {
    throw new ShapingError(`IRS fixed rate must be a finite decimal (got \`${args.fixedRate}\`)`);
  }
  return {
    tenorYears: parseOisTenorYears(args.tenor),
    fixedRate: args.fixedRate,
    notional: shapeRatesNotional(args.notional, "IRS"),
    direction: parseRatesSide(args.side),
    fixedFrequency: parsePaymentFrequency(args.fixedFrequency, "SEMI_ANNUAL"),
    fixedDayCount: parseRatesLegDayCount(args.fixedDayCount, "ACT_360"),
    floatFrequency: parsePaymentFrequency(args.floatFrequency, "QUARTERLY"),
    floatDayCount: parseRatesLegDayCount(args.floatDayCount, "ACT_360"),
  };
}

/** The scalar FRA terms a CELNET.FRA call shapes into a `FraInstrument`. */
export interface FraInstrumentArgs {
  /** The window start (fixing) tenor in months from spot (`3` or `"3M"`). */
  readonly startMonths: number | string;
  /** The window end (maturity) tenor in months from spot (`6` or `"6M"`); `> startMonths`. */
  readonly endMonths: number | string;
  /** The contractual fixed rate K as a decimal (0.033 = 3.30%). */
  readonly fixedRate: number;
  /** The (positive) notional in the curve currency. */
  readonly notional: number;
  /** Pay-fixed (payer) or receive-fixed (receiver); absent ⇒ PAY_FIXED. */
  readonly side?: string | undefined;
  /** The accrual day-count basis for τ; absent ⇒ ACT_360 (the market FRA basis). */
  readonly accrualBasis?: string | undefined;
}

/**
 * Shape the scalar FRA terms into the typed `FraInstrument`. The fixed rate must be
 * a finite decimal; the notional strictly positive (direction carries the sign). The
 * accrual window is validated as whole months from spot with `endMonths >
 * startMonths` (a degenerate/inverted window is rejected here with a friendly message
 * rather than a wire round-trip). The accrual basis defaults to ACT_360 (the standard
 * money-market FRA basis) when omitted.
 */
export function shapeFraInstrument(args: FraInstrumentArgs): FraInstrument {
  if (!Number.isFinite(args.fixedRate)) {
    throw new ShapingError(`FRA fixed rate must be a finite decimal (got \`${args.fixedRate}\`)`);
  }
  const startMonths = parseFraMonths(args.startMonths, "start");
  const endMonths = parseFraMonths(args.endMonths, "end");
  if (endMonths <= startMonths) {
    throw new ShapingError(
      `FRA window end (${endMonths}M) must be strictly after the start (${startMonths}M)`,
    );
  }
  return {
    startMonths,
    endMonths,
    fixedRate: args.fixedRate,
    notional: shapeRatesNotional(args.notional, "FRA"),
    direction: parseRatesSide(args.side),
    accrualBasis: parseRatesAccrualBasis(args.accrualBasis, "ACT_360"),
  };
}

// ---------------------------------------------------------------------------
// fixed-income (rates) — RFQ: shape the `=CELNET.RATESRFQ` taker arguments into a
// `RatesInstrument` + taker `Side` for the `request_rates_quote` RPC (the FI twin of
// `=CELNET.RFQ`), and format the two-way `RatesQuote` reply. The RFQ mirrors the
// `=CELNET.RATES` arg grammar — the instrument is built from (tenor, fixedRate,
// notional) exactly like the unary rates cells, reusing the byte-verified arm
// shapers (`shapeOisInstrument` / `shapeIrsInstrument`). The server strikes the
// two-way around the SIDE-INDEPENDENT fair level and OVERRIDES the arm's own
// direction with the RFQ `side` (`rates_pricing::override_arm_side`), so the arm
// direction is derived from `side` purely for a coherent request body. The add-in
// holds NO FI math: it shapes the inputs the live engine quotes and lays out its
// authoritative two-way.
// ---------------------------------------------------------------------------

/**
 * Parse the `=CELNET.RATESRFQ` taker side into the wire `Side` (BUY / SELL /
 * TWO_WAY). Accepts the contract names + desk synonyms, case-/separator-insensitive:
 * BUY/B/PAY/PAYER/PAY_FIXED/LONG ⇒ BUY; SELL/S/RECEIVE/REC/RECEIVER/RECEIVE_FIXED/
 * SHORT ⇒ SELL; TWO_WAY/TWOWAY/TWO/2WAY/BOTH/MID ⇒ TWO_WAY. Empty/absent ⇒ TWO_WAY
 * — the natural RFQ default (request a two-way market with no firm direction). The
 * returned two-way is always struck around the side-independent fair level; `side`
 * selects the sign of the returned risk (a TWO_WAY request reports the
 * receive-fixed / long magnitude).
 */
export function parseRatesRfqSide(raw: string | undefined): Side {
  if (raw === undefined || String(raw).trim() === "") return "TWO_WAY";
  const t = String(raw).trim().toUpperCase().replace(/[._\s-]/g, "");
  switch (t) {
    case "BUY":
    case "B":
    case "PAY":
    case "PAYER":
    case "PAYFIXED":
    case "LONG":
      return "BUY";
    case "SELL":
    case "S":
    case "RECEIVE":
    case "REC":
    case "RECEIVER":
    case "RECEIVEFIXED":
    case "SHORT":
      return "SELL";
    case "TWOWAY":
    case "TWO":
    case "2WAY":
    case "BOTH":
    case "MID":
      return "TWO_WAY";
    default:
      throw new ShapingError(`invalid RFQ side \`${raw}\` (expected BUY, SELL or TWO_WAY)`);
  }
}

/** The `RatesInstrument` arm an `=CELNET.RATESRFQ` cell quotes from (tenor, fixedRate). */
export type RatesRfqInstrumentKind = "OIS" | "IRS";

/**
 * Parse the optional `instrument` selector for `=CELNET.RATESRFQ`. The RFQ grammar
 * carries only (tenor, fixedRate, notional), which fully specifies an OIS or a
 * spot-starting vanilla IRS (with the market-default leg frequencies/day-counts) —
 * so those are the two selectable arms. A FRA (needs a `[startMonths, endMonths]`
 * accrual window) or a cash bond (needs a maturity date + coupon) is not expressible
 * in this grammar; those are quoted through their dedicated `=CELNET.FRA` / `=CELNET
 * .BOND` pricing cells, so they are rejected here with a friendly message rather than
 * silently mis-shaped. Empty/absent ⇒ OIS (mirroring `=CELNET.RATES`).
 */
export function parseRatesRfqInstrumentKind(raw: string | undefined): RatesRfqInstrumentKind {
  if (raw === undefined || raw.trim() === "") return "OIS";
  const t = raw.trim().toUpperCase().replace(/[._\s-]/g, "");
  if (t === "OIS") return "OIS";
  if (t === "IRS" || t === "SWAP") return "IRS";
  if (t === "FRA" || t === "BOND") {
    throw new ShapingError(
      `RATESRFQ cannot quote a ${t} from (tenor, fixedRate) — price it via CELNET.${t} (its dedicated terms), then RFQ the OIS/IRS arm`,
    );
  }
  throw new ShapingError(`invalid RFQ instrument \`${raw}\` (expected OIS or IRS)`);
}

/** The taker `Side` → the arm's fixed-leg direction (BUY = pay fixed, SELL = receive fixed). */
function rfqSideToDirection(side: Side): OisDirection {
  return side === "SELL" ? "RECEIVE_FIXED" : "PAY_FIXED";
}

/** The scalar terms a `=CELNET.RATESRFQ` cell shapes into the quoted `RatesInstrument`. */
export interface RatesRfqInstrumentArgs {
  /** The swap tenor in whole years (`5` or `"5Y"`). */
  readonly tenor: number | string;
  /** The fixed-leg rate as a decimal (0.041 = 4.10%). */
  readonly fixedRate: number;
  /** The RFQ size in the curve currency (strictly positive). */
  readonly notional: number;
  /** The taker side — governs the returned risk sign and the arm's derived direction. */
  readonly side: Side;
  /** The instrument arm to quote (OIS default, or IRS). */
  readonly instrument?: string | undefined;
}

/**
 * Shape the RATESRFQ scalar terms into the typed {@link RatesInstrument} union arm.
 * The arm direction is DERIVED from the taker `side` (the server overrides it with
 * the RFQ side regardless — this only keeps the request body coherent). Reuses the
 * byte-verified {@link shapeOisInstrument} / {@link shapeIrsInstrument} arm shapers,
 * so the RFQ instrument body is identical to the `=CELNET.RATES`/`IRS` unary wire.
 */
export function shapeRatesRfqInstrument(args: RatesRfqInstrumentArgs): RatesInstrument {
  const kind = parseRatesRfqInstrumentKind(args.instrument);
  const direction = rfqSideToDirection(args.side);
  if (kind === "IRS") {
    return {
      kind: "irs",
      irs: shapeIrsInstrument({
        tenor: args.tenor,
        fixedRate: args.fixedRate,
        notional: args.notional,
        side: direction,
      }),
    };
  }
  return {
    kind: "ois",
    ois: shapeOisInstrument({
      tenor: args.tenor,
      fixedRate: args.fixedRate,
      direction,
      notional: args.notional,
    }),
  };
}

/**
 * Format a two-way `RatesQuote` (the `request_rates_quote` reply) as a labelled
 * vertical spill — the same `label × value` discipline as `=CELNET.RATES`. The
 * two-way is `bid` / `offer` with the derived `mid` = `(bid + offer) / 2` (the
 * side-independent fair level — for a swap/FRA it equals `result.par_rate`, a live
 * cross-check), then the RFQ metadata the wire carries (`notional`, the string
 * `quote_id` — 64-bit-exact, referenceable by a later accept — and the `valid_until`
 * last-look deadline as ISO-8601), then the full FI risk block at the taker side
 * (`pv`, `par_rate`, `pv01`, `dv01`). Returns a rectangular `10×2` matrix.
 */
export function formatRatesRfqSpill(quote: RatesQuote): SpillMatrix {
  const mid = (quote.price.bid + quote.price.offer) / 2;
  return rectangular([
    ["bid", quote.price.bid],
    ["offer", quote.price.offer],
    ["mid", mid],
    ["notional", quote.notional],
    ["quote_id", quote.quoteId.toString()],
    ["valid_until", validUntilIso(quote.validUntilNanos)],
    ["pv", quote.result.pv],
    ["par_rate", quote.result.parRate],
    ["pv01", quote.result.pv01],
    ["dv01", quote.result.dv01],
  ]);
}

// ---------------------------------------------------------------------------
// fixed-income (rates) — cash-bond arm: parse the scalar bond terms into the
// typed `BondInstrument` the `price_rates` RPC carries on its `RatesInstrument
// .bond` oneof arm, and format the server's `RatesPricingResult` (with bond
// semantics) as a labelled spill. The add-in holds NO bond math: it only shapes
// the inputs the `celnet-rates` bond engine prices and lays out its result.
// ---------------------------------------------------------------------------

/**
 * Parse the bond coupon frequency (also the yield-compounding basis). Accepts the
 * canonical contract names and the desk short/synonym forms, case-/separator-
 * insensitive: ANNUAL/ANN/A/1Y; SEMI_ANNUAL/SEMI/SA/6M/2; QUARTERLY/QTR/Q/3M/4.
 * Empty/absent ⇒ SEMI_ANNUAL — the standard cash-bond convention (a USD Treasury
 * / corporate pays semi-annually), the ergonomic default for an omitted argument.
 */
export function parseBondCouponFrequency(raw: string | number | undefined): BondCouponFrequency {
  // A bond coupon frequency IS a `PaymentFrequency` (one canonical wire enum); the
  // cash-bond default is SEMI_ANNUAL (a USD Treasury / corporate pays semi-annually).
  return parsePaymentFrequency(raw, "SEMI_ANNUAL");
}

/**
 * Parse the bond accrual day-count basis (`AccrualBasis`). Accepts the canonical
 * contract names and the desk short forms, case-/separator-insensitive:
 * ACT_360/ACT360; ACT_365_FIXED/ACT365/ACT365F; THIRTY_360_BOND_BASIS/30_360/
 * 30360/30/BONDBASIS/BOND. Empty/absent ⇒ THIRTY_360_BOND_BASIS — the standard
 * USD fixed-bond basis (the proto `AccrualBasis` doc default for a cash bond).
 */
export function parseBondDayCount(raw: string | undefined): BondDayCount {
  // A bond day-count IS an instrument-level `RatesAccrualBasis` (one canonical wire
  // enum); the cash-bond default is 30/360 Bond Basis (the standard USD fixed basis).
  return parseRatesAccrualBasis(raw, "THIRTY_360_BOND_BASIS");
}

/**
 * Parse the bond position direction. LONG (a bought bond, +PV) or SHORT (a sold
 * bond, −PV), with the desk aliases (LONG/L/BUY/B; SHORT/S/SELL). Empty/absent ⇒
 * LONG. TWO_WAY is not a valid outright bond direction (the engine rejects it).
 */
export function parseBondSide(raw: string | undefined): BondPositionSide {
  const t = (raw ?? "LONG").trim().toUpperCase();
  if (t === "" || t === "LONG" || t === "L" || t === "BUY" || t === "B") return "LONG";
  if (t === "SHORT" || t === "S" || t === "SELL") return "SHORT";
  throw new ShapingError(`invalid bond side \`${raw}\` (expected LONG/BUY or SHORT/SELL)`);
}

/** Validate a strictly-positive par redemption / face value (default 100 when absent). */
function shapeBondRedemption(raw: number | undefined): number {
  if (raw === undefined) return 100;
  if (!Number.isFinite(raw) || raw <= 0) {
    throw new ShapingError(`bond redemption/face must be a positive number (got \`${raw}\`)`);
  }
  return raw;
}

/** Compare two civil dates; `< 0` if a is before b, `0` if equal, `> 0` if after. */
function compareBrokenDate(a: BrokenDate, b: BrokenDate): number {
  return a.year - b.year || a.month - b.month || a.day - b.day;
}

/** The scalar cash-bond terms a CELNET.BOND call shapes into a `BondInstrument`. */
export interface BondInstrumentArgs {
  /** The maturity (final-redemption) civil date (Excel date serial or `YYYY-MM-DD`). */
  readonly maturity: number | string;
  /** The curve reference (settlement / spot-anchor) date, for a client-side maturity guard. */
  readonly referenceDate: BrokenDate;
  /** The annual coupon rate as a decimal (0.06 = 6%); 0 for a zero-coupon bond. */
  readonly couponRate: number;
  /** The par redemption / face value; absent ⇒ 100. */
  readonly redemption?: number | undefined;
  /** The coupon frequency; absent ⇒ SEMI_ANNUAL. */
  readonly frequency?: string | number | undefined;
  /** The accrual day-count basis; absent ⇒ THIRTY_360_BOND_BASIS. */
  readonly dayCount?: string | undefined;
  /** The position direction (LONG/SHORT); absent ⇒ LONG. */
  readonly side?: string | undefined;
}

/**
 * Shape the scalar bond terms into the typed `BondInstrument`. The coupon rate must
 * be a finite decimal (0 for a zero-coupon bond, but not negative); the maturity
 * must be strictly after the curve reference date (the engine settles on the
 * reference date rolled to the next business day and validates `maturity >
 * settlement`, so a maturity at/before the reference is always invalid — caught
 * here with a friendly message rather than a wire round-trip).
 */
export function shapeBondInstrument(args: BondInstrumentArgs): BondInstrument {
  if (!Number.isFinite(args.couponRate) || args.couponRate < 0) {
    throw new ShapingError(
      `bond coupon rate must be a finite non-negative decimal (got \`${args.couponRate}\`)`,
    );
  }
  const maturityDate = parseBrokenDate(args.maturity);
  if (compareBrokenDate(maturityDate, args.referenceDate) <= 0) {
    throw new ShapingError(
      `bond maturity ${maturityDate.year}-${maturityDate.month}-${maturityDate.day} must be ` +
        `strictly after the curve reference date ${args.referenceDate.year}-` +
        `${args.referenceDate.month}-${args.referenceDate.day}`,
    );
  }
  return {
    couponRate: args.couponRate,
    couponFrequency: parseBondCouponFrequency(args.frequency),
    dayCount: parseBondDayCount(args.dayCount),
    maturityDate,
    redemption: shapeBondRedemption(args.redemption),
    side: parseBondSide(args.side),
  };
}

/**
 * Format a bond `RatesPricingResult` as a labelled vertical spill. For a cash bond
 * the shared rates-result fields carry BOND semantics (the engine repurposes the
 * generic fields): `pv` is the DIRTY price (side-signed), `par_rate` is the yield
 * to maturity (side-independent), and `pv01` = `dv01` is the yield DV01 (both
 * coincide for a fixed-coupon bond) — so the rows are bond-labelled `dirty_price`,
 * `ytm`, `dv01`, never the swap `pv`/`par_rate` labels that would mislead. The
 * `key_rate_ladder` is empty for a bond (a closed-form yield-space risk with no
 * per-pillar decomposition); if the engine ever returns a ladder it is appended
 * positionally, never silently dropped. Only the fields the wire actually carries
 * are shown — clean price / accrued / duration are NOT on the `price_rates`
 * response and are never fabricated. Returns a rectangular `3×2` (+ any ladder)
 * matrix.
 */
export function formatBondSpill(result: RatesPricingResult): SpillMatrix {
  const rows: SpillMatrix = [
    ["dirty_price", result.pv],
    ["ytm", result.parRate],
    ["dv01", result.dv01],
  ];
  result.keyRateLadder.forEach((value, i) => {
    rows.push([`kr_dv01[${i}]`, value]);
  });
  return rectangular(rows);
}

// ---------------------------------------------------------------------------
// curve bootstrap (`build_curve` / `AuthService.BuildCurve`) — the CELNET.CURVE
// add-in path. Shape a pillar range into a `BuildCurveRequest` (registry-
// instrument OR date-anchored pillars) and lay out the server-bootstrapped
// `CalibratedCurve` as a spill. The add-in holds NO bootstrap math: the live
// `celnet-rates` engine resolves each registry pillar, decodes the schedule, and
// runs the sequential discount-curve bootstrap — this only shapes the inputs and
// lays out the reply (the same one unversioned contract the GUI CurveWorkspace
// consumes over `build_curve`).
// ---------------------------------------------------------------------------

/** The pillar-range + reference-date + currency a CELNET.CURVE call shapes into a `BuildCurveRequest`. */
export interface BuildCurveArgs {
  /**
   * The 2-column `[pillar, quote]` range. Each pillar (first column) is EITHER a
   * reference-data registry instrument id (a non-date string, e.g.
   * `usd-sofr-irs-10y`) OR a maturity date (an Excel date cell or `YYYY-MM-DD`);
   * the quote (second column) is the observed rate as a decimal (0.0431 = 4.31%).
   */
  readonly pillars: readonly (readonly (string | number | boolean)[])[];
  /** The curve reference (spot-anchor) civil date (Excel date serial or `YYYY-MM-DD`). */
  readonly referenceDate: number | string;
  /** ISO-4217 currency; defaults to USD (the USD-SOFR P0 arm). */
  readonly currency?: string | undefined;
}

/** True when a first-column cell denotes a maturity DATE (Excel serial or `YYYY-MM-DD`) rather than a registry id. */
function isDatePillarCell(cell: string | number | boolean): boolean {
  if (typeof cell === "number") return true;
  if (typeof cell === "string") return /^\d{4}-\d{2}-\d{2}$/.test(cell.trim());
  return false;
}

/**
 * Shape a `[pillar, quote]` range + reference date into the typed
 * `BuildCurveRequest`. Blank trailing rows are ignored; each non-blank row is
 * classified per its first cell — a date (Excel serial / `YYYY-MM-DD`) becomes a
 * date-anchored `datePillars` entry, any other non-empty string becomes a
 * registry-instrument `pillars` entry. At least one pillar (across both lists) is
 * required, exactly as the `BuildCurve` contract demands. The `requestId` is a
 * client tag the server echoes back verbatim (mirrors the GUI CurveWorkspace).
 */
export function shapeBuildCurveRequest(args: BuildCurveArgs): BuildCurveRequest {
  const pillars: InstrumentQuote[] = [];
  const datePillars: DatePillar[] = [];
  for (const row of args.pillars) {
    if (isEmptyCurveRow(row)) continue;
    if (row.length < 2) {
      throw new ShapingError(
        "each curve pillar row must have 2 columns: [instrumentId | maturityDate, quote]",
      );
    }
    const head = row[0]!;
    const quote = ratesCell(row[1]!, "pillar quote");
    if (isDatePillarCell(head)) {
      datePillars.push({ maturityDate: parseBrokenDate(head as number | string), quote });
    } else if (typeof head === "boolean") {
      throw new ShapingError(
        "a curve pillar must be an instrument id or a maturity date, not a boolean",
      );
    } else {
      const instrumentId = String(head).trim();
      if (instrumentId === "") {
        throw new ShapingError("a registry curve pillar needs a non-empty instrument id");
      }
      pillars.push({ instrumentId, quote });
    }
  }
  if (pillars.length === 0 && datePillars.length === 0) {
    throw new ShapingError(
      "the curve needs at least one pillar (a registry instrument or a date pillar)",
    );
  }
  return {
    requestId: `curve-${Date.now()}`,
    currency: parseCurveCurrency(args.currency),
    referenceDate: parseBrokenDate(args.referenceDate),
    pillars,
    datePillars,
  };
}

/**
 * Format a `CalibratedCurve` as a labelled spill: a header row then one row per
 * bootstrapped pillar — `[pillar, time_years, discount_factor, zero_rate]`, in the
 * server's short→long maturity order. The pillar label is the server's display
 * `label` (e.g. `Date 2027-12-31`) when present, else the resolving instrument id,
 * else a positional `pillar[i]` tag (never silently blank). Returns a rectangular
 * `(1 + points)×4` matrix.
 */
export function formatCalibratedCurveSpill(curve: CalibratedCurve): SpillMatrix {
  const rows: SpillMatrix = [["pillar", "time_years", "discount_factor", "zero_rate"]];
  curve.points.forEach((p: CalibratedCurvePoint, i) => {
    const label =
      p.label.trim() !== "" ? p.label : p.instrumentId.trim() !== "" ? p.instrumentId : `pillar[${i}]`;
    rows.push([label, p.timeYears, p.discountFactor, p.zeroRate]);
  });
  return rectangular(rows);
}

// ---------------------------------------------------------------------------
// curve query (`get_curve` / `SurfaceService.GetCurve`, ADR-0021) — the
// CELNET.GETCURVE add-in path. Shape a `[tenorYears, parRate]` curve range (the
// shared `RatesCurveSet`) + an optional pinned version into a `GetCurveRequest`,
// querying the curve at each pillar tenor, and lay out the server-read
// `GetCurveResponse` (per-tenor zero rate + discount factor, the echoed
// calibrating par pillars, the marked version) as a spill. The FI analogue of
// CELNET.SURFACE/GetSmile: the add-in holds NO curve math — the live
// `celnet-rates` engine bootstraps (or reads the pinned marked curve) and returns
// the authoritative reply over the one unversioned contract.
// ---------------------------------------------------------------------------

/** The curve-range + reference-date + optional pinned version + currency a CELNET.GETCURVE call shapes into a `GetCurveRequest`. */
export interface GetCurveArgs {
  /** The 2-column `[tenorYears, parRate]` curve range (one row per pillar). */
  readonly curve: readonly (readonly (string | number | boolean)[])[];
  /** The curve reference (spot-anchor) civil date (Excel date serial or `YYYY-MM-DD`). */
  readonly referenceDate: number | string;
  /** Optional pin to a `MarkCurve`d version — read the marked curve of that version instead of bootstrapping live. */
  readonly pinnedVersion?: number | string | undefined;
  /** ISO-4217 currency; defaults to USD (the USD-SOFR P0 arm). */
  readonly currency?: string | undefined;
}

/** Parse an optional pinned curve version — a non-negative integer; blank/absent ⇒ undefined (live bootstrap). */
function parsePinnedCurveVersion(cell: number | string | undefined): number | undefined {
  if (cell === undefined || cell === null || cell === "") return undefined;
  const n = typeof cell === "number" ? cell : Number(String(cell).trim());
  if (!Number.isFinite(n) || !Number.isInteger(n) || n < 0) {
    throw new ShapingError(`pinned curve version must be a non-negative integer (got \`${cell}\`)`);
  }
  return n;
}

/**
 * Shape a `[tenorYears, parRate]` curve range + reference date into the typed
 * `GetCurveRequest`. The curve reuses the shared `RatesCurveSet` shaping
 * (`shapeRatesCurve`), and the queried tenor axis is each pillar's `tenorYears` —
 * so the read reports one point per pillar. An optional `pinnedVersion` reads the
 * marked curve of that version instead of bootstrapping the curve live (the
 * `curveSet` is still shaped, for the tenor axis, exactly as the server ignores it
 * on a pinned read).
 */
export function shapeGetCurveRequest(args: GetCurveArgs): GetCurveRequest {
  const curveSet = shapeRatesCurve({
    curve: args.curve,
    referenceDate: args.referenceDate,
    currency: args.currency,
  });
  const request: GetCurveRequest = {
    curveSet,
    queryTenorYears: curveSet.pillars.map((p) => p.tenorYears),
  };
  const curveVersion = parsePinnedCurveVersion(args.pinnedVersion);
  if (curveVersion !== undefined) request.curveVersion = curveVersion;
  return request;
}

/**
 * Format a `GetCurveResponse` as a labelled spill: a queried-points block
 * (`point`, `tenor_years`, `discount_factor`, `zero_rate`), then a par-pillars
 * block (`par_pillar`, `tenor_years`, `par_rate`) padded to width, then a footer
 * row carrying the read version (`v<n>` for a pinned read, else `live`) and the
 * reply currency. Every measure is the server's own authoritative read — the cell
 * only lays it out. Returns a rectangular `(2 + points + parPillars + 1)×4` matrix.
 */
export function formatGetCurveSpill(reply: GetCurveResponse): SpillMatrix {
  const rows: SpillMatrix = [["point", "tenor_years", "discount_factor", "zero_rate"]];
  reply.points.forEach((p, i) => {
    rows.push([`point[${i}]`, p.tenorYears, p.discountFactor, p.zeroRate]);
  });
  rows.push(["par_pillar", "tenor_years", "par_rate", ""]);
  reply.parPillars.forEach((p, i) => {
    rows.push([`par[${i}]`, p.tenorYears, p.parRate, ""]);
  });
  const version = reply.curveVersion === undefined ? "live" : `v${reply.curveVersion}`;
  rows.push(["version", version, "currency", reply.currency]);
  return rectangular(rows);
}

// ---------------------------------------------------------------------------
// curve mark (`mark_curve` / `SurfaceService.MarkCurve`, ADR-0021) — the
// CELNET.MARKCURVE add-in path, the FI analogue of CELNET.MARKSURFACE. Shape a
// `[tenorYears, parRate]` curve range + reference date into the typed
// `MarkCurveRequest` (reusing the shared `RatesCurveSet` shaping verbatim), and lay
// out the `MarkCurveResponse` — the server-assigned pinned `curve_version` (the key
// output: a later CELNET.GETCURVE(…, pinnedVersion) reproduces this exact curve),
// the echoed calibrating par pillars, and the bootstrapped points. A MARK is a
// side-effecting write (it deposits a fresh version), never a live formula — the
// add-in holds no curve math: the live `celnet-rates` engine bootstraps + persists
// the authoritative curve and stamps the version.
// ---------------------------------------------------------------------------

/** The curve-range + reference-date + currency a CELNET.MARKCURVE call shapes into a `MarkCurveRequest`. */
export interface MarkCurveArgs {
  /** The 2-column `[tenorYears, parRate]` curve range (one row per pillar). */
  readonly curve: readonly (readonly (string | number | boolean)[])[];
  /** The curve reference (spot-anchor) civil date (Excel date serial or `YYYY-MM-DD`). */
  readonly referenceDate: number | string;
  /** ISO-4217 currency; defaults to USD (the USD-SOFR P0 arm). */
  readonly currency?: string | undefined;
}

/**
 * Shape a `[tenorYears, parRate]` curve range + reference date into the typed
 * `MarkCurveRequest`. The curve reuses the shared `RatesCurveSet` shaping
 * (`shapeRatesCurve`) verbatim — identical to CELNET.GETCURVE / CELNET.CURVE — so a
 * mark and a live read bootstrap byte-identically. A mark carries no tenor axis (it
 * reports at the calibrating pillars) and no pinned version (it CREATES one).
 */
export function shapeMarkCurveRequest(args: MarkCurveArgs): MarkCurveRequest {
  const curveSet = shapeRatesCurve({
    curve: args.curve,
    referenceDate: args.referenceDate,
    currency: args.currency,
  });
  return { curveSet };
}

/**
 * Format a `MarkCurveResponse` as a labelled spill: a bootstrapped-points block
 * (`point`, `tenor_years`, `discount_factor`, `zero_rate`), then a par-pillars block
 * (`par_pillar`, `tenor_years`, `par_rate`) padded to width, then a footer row
 * carrying the assigned pinned version (`v<n>` — ALWAYS a concrete version, since a
 * mark always stamps one) and the reply currency. The footer version is the id a
 * subsequent CELNET.GETCURVE(…, pinnedVersion) reproduces the curve from. Every
 * measure is the server's own authoritative mark — the cell only lays it out.
 * Returns a rectangular `(2 + points + parPillars + 1)×4` matrix.
 */
export function formatMarkCurveSpill(reply: MarkCurveResponse): SpillMatrix {
  const rows: SpillMatrix = [["point", "tenor_years", "discount_factor", "zero_rate"]];
  reply.points.forEach((p, i) => {
    rows.push([`point[${i}]`, p.tenorYears, p.discountFactor, p.zeroRate]);
  });
  rows.push(["par_pillar", "tenor_years", "par_rate", ""]);
  reply.parPillars.forEach((p, i) => {
    rows.push([`par[${i}]`, p.tenorYears, p.parRate, ""]);
  });
  rows.push(["version", `v${reply.curveVersion}`, "currency", reply.currency]);
  return rectangular(rows);
}

// ---------------------------------------------------------------------------
// linear-rates portfolio risk (`aggregate_rates_risk`) — the CELNET.RATESRISK
// add-in path. Shape a rates BOOK (an OIS-per-row range) + the optional
// `(entity, book, ccy)` scope into the typed `RatesPosition[]` / `RatesRiskScope`
// the server nets against one shared curve, and lay out the server-returned
// per-currency `RatesRiskNode` tree as a spill. The add-in holds NO rates-risk
// math: the live `celnet-rates` engine prices every position and the server sums
// them additively; this only shapes the inputs and lays out the netted reply.
// ---------------------------------------------------------------------------

/** Parse an optional booking-cell id (`entity`/`book`) — a non-negative integer; blank ⇒ 0. */
function shapeBookingId(cell: string | number | boolean | undefined, what: string): number {
  if (cell === undefined || cell === null || cell === "") return 0;
  if (typeof cell === "boolean") throw new ShapingError(`${what} must be a number, not a boolean`);
  const n = typeof cell === "number" ? cell : Number(String(cell).trim());
  if (!Number.isInteger(n) || n < 0) {
    throw new ShapingError(`${what} must be a non-negative integer (got \`${cell}\`)`);
  }
  return n;
}

/**
 * Parse the CELNET.RATESRISK positions range — one OIS position per row,
 * `[tenorYears, fixedRate, direction, notional, entity?, book?]` — into typed
 * `RatesPosition`s. Blank trailing rows are ignored. Each row's OIS reuses the
 * exact `price_rates` OIS shape (whole-year tenor, decimal fixed rate, positive
 * notional, PAY_FIXED/RECEIVE_FIXED direction carrying the sign); the optional
 * `entity`/`book` are the booking cell the server rolls up and the `scope`
 * filters on (absent ⇒ the `0` default). A synthetic 1-based `positionId` is
 * assigned per row (informational — the server echoes it, it does not affect the
 * netting). At least one position is required.
 */
export function shapeRatesRiskPositions(
  rows: readonly (readonly (string | number | boolean)[])[],
): RatesPosition[] {
  const positions: RatesPosition[] = [];
  let seq = 0;
  for (const row of rows) {
    if (isEmptyCurveRow(row)) continue;
    if (row.length < 4) {
      throw new ShapingError(
        "each rates position row needs [tenorYears, fixedRate, direction, notional] (entity, book optional)",
      );
    }
    // The tenor accepts EITHER a whole-year number (`5`) or a `"5Y"` string, exactly
    // like the RATES sibling — so parse it directly (a boolean is neither).
    const tenorCell = row[0]!;
    if (typeof tenorCell === "boolean") {
      throw new ShapingError("position tenorYears must be a whole-year number or a `5Y` string");
    }
    const tenorYears = parseOisTenorYears(tenorCell);
    const fixedRate = ratesCell(row[1]!, "position fixedRate");
    const direction = parseOisDirection(String(row[2]));
    const notional = shapeRatesNotional(ratesCell(row[3]!, "position notional"), "OIS");
    const entity = shapeBookingId(row[4], "entity");
    const book = shapeBookingId(row[5], "book");
    seq += 1;
    positions.push({
      positionId: BigInt(seq),
      entity,
      book,
      instrument: { tenorYears, fixedRate, notional, direction },
    });
  }
  if (positions.length === 0) {
    throw new ShapingError("the rates portfolio needs at least one OIS position");
  }
  return positions;
}

/** Parse one scope uint token value (`entity`/`book`) — a non-negative integer. */
function parseScopeUint(raw: string, what: string): number {
  const n = Number(raw.trim());
  if (!Number.isInteger(n) || n < 0) {
    throw new ShapingError(`scope ${what} must be a non-negative integer (got \`${raw}\`)`);
  }
  return n;
}

/**
 * Parse the optional CELNET.RATESRISK scope string into a `RatesRiskScope`, or
 * undefined for the whole book. The grammar is a comma/semicolon-separated list of
 * `KEY:value` tokens (case-insensitive): `ENTITY:<n>`, `BOOK:<n>`, `CCY:<xxx>` —
 * each present key narrows the rollup, an absent key does not constrain. Empty /
 * `ALL` / `FIRM` ⇒ undefined (no filter). A malformed token is rejected loudly,
 * never silently ignored.
 */
export function parseRatesRiskScope(raw: string | undefined): RatesRiskScope | undefined {
  const s = (raw ?? "").trim();
  if (s === "" || s.toUpperCase() === "ALL" || s.toUpperCase() === "FIRM") return undefined;
  const scope: { -readonly [K in keyof RatesRiskScope]?: RatesRiskScope[K] } = {};
  for (const tokenRaw of s.split(/[,;]+/)) {
    const token = tokenRaw.trim();
    if (token === "") continue;
    const m = /^([A-Za-z]+)\s*[:=]\s*(.+)$/.exec(token);
    if (!m || m[1] === undefined || m[2] === undefined) {
      throw new ShapingError(`invalid scope token \`${token}\` (expected ENTITY:n, BOOK:n or CCY:xxx)`);
    }
    const key = m[1].toUpperCase();
    const value = m[2].trim();
    switch (key) {
      case "ENTITY":
      case "ENT":
        scope.entity = parseScopeUint(value, "entity");
        break;
      case "BOOK":
      case "BK":
        scope.book = parseScopeUint(value, "book");
        break;
      case "CCY":
      case "CURRENCY": {
        const ccy = value.toUpperCase();
        if (!/^[A-Z]{3}$/.test(ccy)) {
          throw new ShapingError(`invalid scope ccy \`${value}\` (expected a 3-letter code)`);
        }
        scope.ccy = ccy;
        break;
      }
      default:
        throw new ShapingError(`invalid scope key \`${m[1]}\` (expected ENTITY, BOOK or CCY)`);
    }
  }
  if (scope.entity === undefined && scope.book === undefined && scope.ccy === undefined) {
    return undefined;
  }
  return scope;
}

/**
 * Format the CELNET.RATESRISK reply as a per-currency node grid: a header
 * `[ccy, net_pv, net_pv01, net_dv01, kr_dv01[<tenor>Y]…]`, one row per settlement-
 * currency node, then a summary footer. Every node prices against the ONE request
 * curve, so its key-rate ladder shares the curve's pillar tenors; the ladder
 * columns are the sorted union of every node's bucket tenors (a node missing a
 * bucket at a column tenor shows blank, never a spurious zero). The netting is the
 * server's; this only lays the server-returned node tree out in cells. An empty
 * rollup (no positions in scope) spills an honest empty-state row.
 */
export function formatRatesRiskSpill(nodes: readonly RatesRiskNode[]): SpillMatrix {
  const tenorSet = new Set<number>();
  for (const node of nodes) {
    for (const bucket of node.keyRateLadder) tenorSet.add(bucket.tenorYears);
  }
  const tenors = [...tenorSet].sort((a, b) => a - b);
  const header: (string | number)[] = [
    "ccy",
    "net_pv",
    "net_pv01",
    "net_dv01",
    ...tenors.map((t) => `kr_dv01[${t}Y]`),
  ];
  const rows: SpillMatrix = [header];
  for (const node of nodes) {
    const ladder = new Map<number, number>();
    for (const bucket of node.keyRateLadder) ladder.set(bucket.tenorYears, bucket.dv01);
    const row: (string | number)[] = [node.ccy, node.netPv, node.netPv01, node.netDv01];
    for (const t of tenors) {
      const v = ladder.get(t);
      row.push(v === undefined ? "" : v);
    }
    rows.push(row);
  }
  if (nodes.length === 0) {
    rows.push(["(no rates positions in scope)"]);
  } else {
    rows.push([
      `${nodes.length} currenc${nodes.length === 1 ? "y" : "ies"} | server-netted (celnet-rates-risk)`,
    ]);
  }
  return rectangular(rows);
}

// ---------------------------------------------------------------------------
// XVA — counterparty valuation adjustments (`price_xva` / `PricingService.PriceXva`)
// — the CELNET.XVA add-in path. Shape a netting set of FX vanillas (a row-per-trade
// range) + the single-factor exposure model + the counterparty/own survival (hazard)
// curves + LGDs + funding spread into the typed {@link XvaPricingRequest}, and lay
// out the four scalar adjustments the wire returns. The add-in holds NO XVA math:
// the live `celnet-xva` engine simulates the expected-exposure profile and
// aggregates CVA/DVA/FVA; this only shapes the inputs and lays the reply out.
// ---------------------------------------------------------------------------

/** The GUI XvaWorkspace exposure-estimator defaults (mirrors `MC_PATHS` / `MC_SEED`
 * / `EXPOSURE_STEPS` in `gui/src/workspaces/XvaWorkspace.tsx`), so an Excel cell that
 * omits the optional MC controls prices against the same budget as the GUI. */
const XVA_DEFAULT_PATHS = 4096;
const XVA_DEFAULT_SEED = 1;
const XVA_DEFAULT_EXPOSURE_STEPS = 16;

/** The non-empty cells of a range row (Excel right-pads ranges with blank cells). */
function nonEmptyCells(
  row: readonly (string | number | boolean)[],
): (string | number | boolean)[] {
  return row.filter((c) => c !== "" && c !== null && c !== undefined);
}

/**
 * Shape a survival (hazard-rate) curve cell/range into an {@link XvaSurvivalCurve}.
 * A single scalar cell is a FLAT curve — constant hazard `λ`, NO pillar times
 * (`{ pillarTimes: [], hazardRates: [λ] }`, survival `e^{−λt}`). A 2-column
 * `[pillarYears, hazard]` range is a PIECEWISE-constant curve — strictly-increasing
 * positive pillar times with their per-segment hazards, the last held flat past the
 * final pillar (the market convention). Blank trailing rows are ignored; a malformed
 * shape is rejected loudly. Mirrors the server's `SurvivalCurve::{flat,piecewise}`
 * decode (`crates/celnet-xva`) and the GUI's flat/piecewise `XvaSurvivalCurve`.
 */
export function shapeXvaSurvivalCurve(
  range: readonly (readonly (string | number | boolean)[])[],
  what: string,
): XvaSurvivalCurve {
  const rows = range.filter((r) => !isEmptyCurveRow(r));
  if (rows.length === 0) {
    throw new ShapingError(
      `${what} survival curve is empty (give a flat hazard λ, or a [pillarYears, hazard] range)`,
    );
  }
  // FLAT: a single scalar cell ⇒ a constant hazard with no pillar times.
  if (rows.length === 1 && nonEmptyCells(rows[0]!).length === 1) {
    const lambda = ratesCell(nonEmptyCells(rows[0]!)[0]!, `${what} flat hazard`);
    if (lambda < 0) throw new ShapingError(`${what} hazard must be >= 0 (got \`${lambda}\`)`);
    return { pillarTimes: [], hazardRates: [lambda] };
  }
  // PIECEWISE: [pillarYears, hazard] per row, strictly-increasing positive pillars.
  const pillarTimes: number[] = [];
  const hazardRates: number[] = [];
  let prev = 0;
  for (const row of rows) {
    if (nonEmptyCells(row).length < 2) {
      throw new ShapingError(
        `${what} piecewise survival row needs [pillarYears, hazard] (a lone value is read as a flat hazard)`,
      );
    }
    const t = ratesCell(row[0]!, `${what} pillar time`);
    const lambda = ratesCell(row[1]!, `${what} hazard`);
    if (!(t > prev)) {
      throw new ShapingError(
        `${what} pillar times must be strictly increasing and positive (got \`${t}\` after \`${prev}\`)`,
      );
    }
    if (lambda < 0) throw new ShapingError(`${what} hazard must be >= 0 (got \`${lambda}\`)`);
    pillarTimes.push(t);
    hazardRates.push(lambda);
    prev = t;
  }
  return { pillarTimes, hazardRates };
}

/**
 * Shape a netting-set range `[callPut, strike, expiryYears, vol, notional]` (one FX
 * vanilla per row) into {@link XvaTrade}s. `callPut` is `C`/`P` (or CALL/PUT);
 * `strike`/`expiryYears`/`vol` are strictly positive; `notional` is SIGNED (a
 * negative notional flips the trade direction). Blank trailing rows are ignored; an
 * empty set is rejected. Mirrors the RATESRISK positions shaper and the server's
 * `xva_trade_from_json` field order.
 */
export function shapeXvaTrades(
  rows: readonly (readonly (string | number | boolean)[])[],
): XvaTrade[] {
  const trades: XvaTrade[] = [];
  for (const row of rows) {
    if (isEmptyCurveRow(row)) continue;
    if (row.length < 5) {
      throw new ShapingError("each XVA trade row needs [callPut, strike, expiryYears, vol, notional]");
    }
    const optionType = parseOptionType(String(row[0]));
    const strike = ratesCell(row[1]!, "trade strike");
    const expiryYears = ratesCell(row[2]!, "trade expiryYears");
    const vol = ratesCell(row[3]!, "trade vol");
    const notional = ratesCell(row[4]!, "trade notional");
    if (!(strike > 0)) throw new ShapingError(`trade strike must be > 0 (got \`${strike}\`)`);
    if (!(expiryYears > 0)) throw new ShapingError(`trade expiryYears must be > 0 (got \`${expiryYears}\`)`);
    if (!(vol > 0)) throw new ShapingError(`trade vol must be > 0 (got \`${vol}\`)`);
    trades.push({ optionType, strike, expiryYears, vol, notional });
  }
  if (trades.length === 0) {
    throw new ShapingError("the XVA netting set needs at least one trade");
  }
  return trades;
}

/** Validate an optional whole-count exposure control, defaulting when omitted. */
function shapeXvaCount(
  raw: number | undefined,
  dflt: number,
  what: string,
  min: number,
): number {
  if (raw === undefined) return dflt;
  if (!Number.isInteger(raw) || raw < min) {
    throw new ShapingError(`${what} must be a whole number >= ${min} (got \`${raw}\`)`);
  }
  return raw;
}

/** Validate a loss-given-default fraction (a decimal in `[0, 1]`). */
function shapeXvaFraction(raw: number, what: string): number {
  if (!Number.isFinite(raw) || raw < 0 || raw > 1) {
    throw new ShapingError(`${what} must be a decimal in [0, 1] (got \`${raw}\`)`);
  }
  return raw;
}

/** The raw CELNET.XVA cell arguments before shaping into the typed request. */
export interface XvaRequestArgs {
  readonly trades: readonly (readonly (string | number | boolean)[])[];
  readonly spot0: number;
  readonly sigma: number;
  readonly rDom: number;
  readonly rFor: number;
  readonly counterparty: readonly (readonly (string | number | boolean)[])[];
  readonly own: readonly (readonly (string | number | boolean)[])[];
  readonly lgdCounterparty: number;
  readonly lgdOwn: number;
  readonly fundingSpread: number;
  readonly paths?: number;
  readonly seed?: number;
  readonly exposureSteps?: number;
}

/**
 * Shape the CELNET.XVA cell arguments into the typed {@link XvaPricingRequest} the
 * `price_xva` RPC decodes. Validates the netting set, the single-factor exposure
 * model (`spot0 > 0`, `sigma >= 0`, finite rates), both survival curves, the two
 * LGDs (`∈ [0, 1]`) and the funding spread, and defaults the optional MC controls to
 * the GUI's exposure-estimator budget. Every number is passed straight to the engine
 * as a decimal (no unit conversion) — exactly like the RATES/BOND cell shapers.
 */
export function shapeXvaRequest(args: XvaRequestArgs): XvaPricingRequest {
  const trades = shapeXvaTrades(args.trades);
  if (!(args.spot0 > 0)) throw new ShapingError(`spot0 must be > 0 (got \`${args.spot0}\`)`);
  if (!Number.isFinite(args.sigma) || args.sigma < 0) {
    throw new ShapingError(`sigma must be a decimal >= 0 (got \`${args.sigma}\`)`);
  }
  if (!Number.isFinite(args.rDom)) throw new ShapingError("rDom must be a finite decimal");
  if (!Number.isFinite(args.rFor)) throw new ShapingError("rFor must be a finite decimal");
  if (!Number.isFinite(args.fundingSpread)) {
    throw new ShapingError("fundingSpread must be a finite decimal");
  }
  return {
    trades,
    rDom: args.rDom,
    rFor: args.rFor,
    spot0: args.spot0,
    sigma: args.sigma,
    paths: shapeXvaCount(args.paths, XVA_DEFAULT_PATHS, "paths", 1),
    seed: shapeXvaCount(args.seed, XVA_DEFAULT_SEED, "seed", 0),
    exposureSteps: shapeXvaCount(args.exposureSteps, XVA_DEFAULT_EXPOSURE_STEPS, "exposureSteps", 1),
    counterparty: shapeXvaSurvivalCurve(args.counterparty, "counterparty"),
    own: shapeXvaSurvivalCurve(args.own, "own"),
    lgdCounterparty: shapeXvaFraction(args.lgdCounterparty, "lgdCounterparty"),
    lgdOwn: shapeXvaFraction(args.lgdOwn, "lgdOwn"),
    fundingSpread: args.fundingSpread,
  };
}

/**
 * Format an {@link XvaResult} as a labelled vertical spill: `cva`, `dva`, `fva`,
 * `total_adjustment` (the WHOLE wire result — the simulated exposure profile is a
 * server-internal and is NEVER on the wire, so it is never shown, never fabricated),
 * then a provenance footer stating the netting-set size + the pricing engine. The
 * four numbers ARE the server's `celnet-xva` figures — a cell is bit-consistent with
 * the GUI XvaWorkspace over the one contract. Returns a rectangular matrix.
 */
export function formatXvaSpill(result: XvaResult, tradeCount: number): SpillMatrix {
  const rows: SpillMatrix = [
    ["cva", result.cva],
    ["dva", result.dva],
    ["fva", result.fva],
    ["total_adjustment", result.totalAdjustment],
  ];
  rows.push([
    `${tradeCount} trade${tradeCount === 1 ? "" : "s"} netted | server-priced (celnet-xva)`,
  ]);
  return rectangular(rows);
}

// ---------------------------------------------------------------------------
// linear-rates BOOK ledger (`list_rates_positions`) — the CELNET.RATESBOOK add-in
// path. Lay out the SERVER-owned OIS position ledger, resolving each numeric
// `(entity, book)` partition key to its registry NAME through the caller-supplied
// resolvers (backed by `list_entities` / `list_books`), exactly like the GUI
// RatesBookWorkspace — an unknown key (or an unavailable registry) falls back to
// `#<key>`, so a raw number is never shown. The add-in holds no book state: the
// live `RiskService` owns it and this only lays the reply out in cells.
// ---------------------------------------------------------------------------

/**
 * Format CELNET.RATESBOOK as a position ledger: a header
 * `[position_id, entity, book, instrument, fixed_rate, notional, direction]`, one
 * row per booked OIS (the numeric `(entity, book)` keys resolved to their registry
 * NAMES via `entityName` / `bookName`; an unknown key resolves to `#<key>`), then a
 * count footer — or an honest empty-state row when the book is empty. The
 * `position_id` is rendered as a string to avoid 64-bit precision loss; the fixed
 * rate (decimal) and notional (absolute) are raw numbers so the desk can compute on
 * them. Returns a rectangular `(1 + positions + 1)×7` matrix.
 */
export function formatRatesBookSpill(
  positions: readonly RatesPosition[],
  entityName: (key: number) => string,
  bookName: (key: number) => string,
): SpillMatrix {
  const header: (string | number)[] = [
    "position_id",
    "entity",
    "book",
    "instrument",
    "fixed_rate",
    "notional",
    "direction",
  ];
  const rows: SpillMatrix = [header];
  for (const p of positions) {
    rows.push([
      p.positionId.toString(),
      entityName(p.entity),
      bookName(p.book),
      `${p.instrument.tenorYears}y OIS`,
      p.instrument.fixedRate,
      p.instrument.notional,
      p.instrument.direction,
    ]);
  }
  if (positions.length === 0) {
    rows.push(["(no rates positions)"]);
  } else {
    rows.push([`${positions.length} position${positions.length === 1 ? "" : "s"}`]);
  }
  return rectangular(rows);
}

// ---------------------------------------------------------------------------
// instrument reference-data roster (`list_instruments` / `get_instrument`) — the
// CELNET.INSTRUMENTS add-in path. Lay out the admin-managed registry of instrument
// DEFINITIONS the curve-bootstrap + pricing paths resolve against. The add-in holds
// no registry: the live `AuthService` owns it and this only lays the reply out
// (read-only; create/update/delete are admin-only, out of this surface). Every
// family renders uniformly from its verbatim terms bag.
// ---------------------------------------------------------------------------

/** Render an instrument's external ids as a terse `scheme=value; …` cell (empty ⇒ `—`). */
function renderExternalIds(ids: readonly { readonly scheme: string; readonly value: string }[]): string {
  if (ids.length === 0) return "—";
  return ids.map((x) => `${x.scheme}=${x.value}`).join("; ");
}

/**
 * Render one family-terms value: an array as `[a, b]`, a `BrokenDate` object as
 * `YYYY-MM-DD`, any other object as compact JSON, else the bare scalar. Keeps a
 * definition's terms readable in a single cell without dropping any field.
 */
function renderTermValue(v: unknown): string {
  if (Array.isArray(v)) return `[${v.map((x) => String(x)).join(", ")}]`;
  if (v !== null && typeof v === "object") {
    const o = v as Record<string, unknown>;
    if (
      typeof o["year"] === "number" &&
      typeof o["month"] === "number" &&
      typeof o["day"] === "number"
    ) {
      const pad = (n: number): string => String(n).padStart(2, "0");
      return `${o["year"]}-${pad(o["month"] as number)}-${pad(o["day"] as number)}`;
    }
    return JSON.stringify(v);
  }
  return String(v);
}

/**
 * Render a family terms bag as a terse `key=value; …` cell (empty ⇒ `—`). Null /
 * undefined / empty-string / empty-array fields are dropped so the cell shows only
 * the terms the family actually carries.
 */
function renderInstrumentTerms(terms: Readonly<Record<string, unknown>>): string {
  const parts = Object.entries(terms)
    .filter(
      ([, v]) =>
        v !== null &&
        v !== undefined &&
        v !== "" &&
        !(Array.isArray(v) && v.length === 0),
    )
    .map(([k, v]) => `${k}=${renderTermValue(v)}`);
  return parts.length === 0 ? "—" : parts.join("; ");
}

/**
 * Format CELNET.INSTRUMENTS as a reference-data grid: a header
 * `[instrument_id, name, family, currency, description, external_ids, terms]`, one
 * row per definition (its family token + the family's terms rendered verbatim),
 * then a count footer — or an honest empty-state row when the roster (or a `get` by
 * id) returns nothing. Every field is the server's authoritative definition over the
 * one contract; the add-in computes nothing. Returns a rectangular
 * `(1 + rows + 1)×7` matrix.
 */
export function formatInstrumentsSpill(defs: readonly InstrumentDef[]): SpillMatrix {
  const header: (string | number)[] = [
    "instrument_id",
    "name",
    "family",
    "currency",
    "description",
    "external_ids",
    "terms",
  ];
  const rows: SpillMatrix = [header];
  for (const d of defs) {
    rows.push([
      d.instrumentId,
      d.name,
      d.family === "" ? "?" : d.family,
      d.currency,
      d.description === "" ? "—" : d.description,
      renderExternalIds(d.externalIds),
      renderInstrumentTerms(d.terms),
    ]);
  }
  if (defs.length === 0) {
    rows.push(["(no instruments registered)"]);
  } else {
    rows.push([`${defs.length} instrument${defs.length === 1 ? "" : "s"}`]);
  }
  return rectangular(rows);
}
