/**
 * Typed mirror of the single, current Celnet wire contract (`celnet.proto`,
 * package `celnet.wire`). One contract, two transports (gRPC-Web/Connect and the
 * designed WebSocket JSON-mirror) — the GUI is a consumer of that contract, never
 * a fork of it (CLAUDE.md rule 9: no versioned APIs).
 *
 * These shapes are written here by hand for the standalone build; when the build
 * wave wires `buf`-generated types from `celnet-proto`, this module is replaced
 * by the generated module and the rest of the app keeps compiling unchanged
 * (every consumer imports from here, so the seam is one file). Field names use
 * the proto's camelCase JSON projection; enums mirror the proto enums one-to-one.
 *
 * Provenance note (CLAUDE.md rule 8): identifiers are purpose-named and
 * vendor/research-neutral exactly as the proto demands (`VanillaInputs`, never a
 * person/method name). No mathematical-method names appear in any identifier.
 */

// --- vocabulary enums (mirror celnet.wire enums one-to-one) -----------------

export type OptionType = "CALL" | "PUT";

export type Side = "BUY" | "SELL" | "TWO_WAY";

export type DeltaConvention =
  | "SPOT_UNADJUSTED"
  | "FORWARD_UNADJUSTED"
  | "SPOT_PREMIUM_ADJUSTED"
  | "FORWARD_PREMIUM_ADJUSTED";

export type AtmConvention = "ATM_FORWARD" | "DELTA_NEUTRAL_STRADDLE";

export type PremiumStyle =
  | "DOMESTIC_PIPS"
  | "PERCENT_FOREIGN"
  | "PERCENT_DOMESTIC"
  | "FOREIGN_PIPS";

export type Cut = "NEW_YORK_1000" | "TOKYO_1500";

export type DayCount = "ACT_365_FIXED" | "ACT_360";

export type Settlement = "DELIVERABLE" | "NON_DELIVERABLE";

export type StrategyKind = "RISK_REVERSAL" | "STRANGLE" | "STRADDLE" | "SEAGULL";

export type TenorUnit =
  | "OVERNIGHT"
  | "WEEKS"
  | "MONTHS"
  | "YEARS"
  | "TOM_NEXT"
  | "SPOT_NEXT"
  | "IMM"
  | "BROKEN_DATE";

/**
 * The smile-calibration model the surface is marked under (`celnet.wire.SmileModel`).
 * Vendor/method-neutral, purpose-named (CLAUDE.md rule 8) — the mathematical family
 * each maps to lives in the server's doc comments, never in this identifier:
 *  - `MARKET_HEDGE`        — the desk's market-hedge (vanna-volga) construction (default)
 *  - `STOCHASTIC_VOL`      — a stochastic-vol parameterisation fitted to the same anchors
 *  - `PARAMETRIC`          — a parametric per-slice fit
 *  - `PARAMETRIC_SURFACE`  — a parametric whole-surface fit
 */
export type SmileModel =
  | "MARKET_HEDGE"
  | "STOCHASTIC_VOL"
  | "PARAMETRIC"
  | "PARAMETRIC_SURFACE";

/**
 * A market observable a time-series feed can stream (`celnet.wire.MarketObservable`).
 * The value's natural unit is vol for ATM_VOL/RISK_REVERSAL/BUTTERFLY and a rate for
 * SPOT/FORWARD. RISK_REVERSAL/BUTTERFLY additionally need a signed delta wing; all of
 * ATM_VOL/RISK_REVERSAL/BUTTERFLY/FORWARD need a tenor; SPOT is tenor-independent.
 */
export type MarketObservable =
  | "ATM_VOL"
  | "SPOT"
  | "RISK_REVERSAL"
  | "BUTTERFLY"
  | "FORWARD";

// --- value messages ---------------------------------------------------------

/** An FX currency pair BASE/QUOTE (market form CCY1CCY2), e.g. EUR/USD. */
export interface CcyPair {
  /** Foreign / asset currency (CCY1), e.g. "EUR". */
  base: string;
  /** Domestic / numeraire currency (CCY2), e.g. "USD". */
  quote: string;
}

/** An explicit calendar date for a `BROKEN_DATE` tenor (`celnet.wire.BrokenDate`). */
export interface BrokenDate {
  year: number;
  month: number;
  day: number;
}

/**
 * A standard FX-options tenor. `unit` selects the family; `count` is the
 * weeks/months/years count (or the 1-based IMM ordinal for `IMM`), ignored for
 * OVERNIGHT/TOM_NEXT/SPOT_NEXT; `brokenDate` is the explicit date for `BROKEN_DATE`.
 */
export interface Tenor {
  unit: TenorUnit;
  /** Units count (weeks/months/years) or IMM ordinal; ignored for the short-end units. */
  count: number;
  /** The explicit expiry date for `BROKEN_DATE` (presence-tracked; absent otherwise). */
  brokenDate?: BrokenDate;
}

/** The trade conventions a quote / surface is expressed under. */
export interface Conventions {
  deltaConvention: DeltaConvention;
  atmConvention: AtmConvention;
  premiumStyle: PremiumStyle;
  cut: Cut;
  dayCount: DayCount;
  settlement: Settlement;
}

/** The market context an instrument is priced against. */
export interface MarketContext {
  /** Spot FX rate (quote per 1 unit of base). */
  spot: number;
  /** Annualized volatility (absolute, e.g. 0.10 = 10 vol). */
  vol: number;
  /** Continuously-compounded domestic (quote) interest rate. */
  rDom: number;
  /** Continuously-compounded foreign (base) interest rate. */
  rFor: number;
}

/** A trade notional and the currency leg it is denominated in. */
export interface Quantity {
  notional: number;
  /** True if notional is in base/foreign (CCY1); false if quote/domestic (CCY2). */
  baseCcy: boolean;
}

/** The full FX-options Greek set — mirrors celnet.wire.Greeks field-for-field. */
export interface Greeks {
  price: number;
  deltaSpot: number;
  deltaForward: number;
  gamma: number;
  vega: number;
  theta: number;
  rhoDom: number;
  rhoFor: number;
  vanna: number;
  volga: number;
  charm: number;
  speed: number;
  zomma: number;
  color: number;
}

/** A two-way (bid / offer) market in the conventions' premium-style units. */
export interface TwoWayPrice {
  bid: number;
  offer: number;
}

/**
 * A short-lived click-to-trade token stamped on a streamed price line. Binds a
 * (subscription, sequence, side, premium) to a bounded validity window; an
 * Execute carrying the token within the window books at the stamped price.
 */
export interface TradableToken {
  token: bigint;
  /** BUY lifts the offer; SELL hits the bid. */
  side: Side;
  /** The premium this token books at (bid for SELL, offer for BUY). */
  premium: number;
  /** Token validity deadline, nanoseconds since the Unix epoch (UTC). */
  validUntilNanos: bigint;
}

// --- instrument model -------------------------------------------------------

/** A strike expressed either as an absolute level or a signed convention delta. */
export type StrikeOrDelta =
  | { kind: "strike"; strike: number }
  | { kind: "delta"; delta: number };

export interface Vanilla {
  optionType: OptionType;
  strike: StrikeOrDelta;
}

export interface Leg {
  optionType: OptionType;
  strike: StrikeOrDelta;
  side: Side;
  /** Leg ratio relative to base notional (e.g. 1.0, or 2.0 for a 1x2). */
  ratio: number;
}

export interface Strategy {
  kind: StrategyKind;
  legs: Leg[];
}

/** The product payoff carried by an Instrument (the proto `product` oneof). */
export type Product =
  | { kind: "vanilla"; vanilla: Vanilla }
  | { kind: "strategy"; strategy: Strategy };

/** Solve directive: solve a free parameter to hit a target (e.g. zero premium). */
export interface Solve {
  target: "NONE" | "STRIKE" | "PREMIUM";
  targetPremium: number;
}

/** The unified instrument every Celnet workflow speaks. */
export interface Instrument {
  pair: CcyPair;
  tenor: Tenor;
  /** Expiry year fraction on the surface day-count (authoritative for pricing). */
  expiryYears: number;
  quantity: Quantity;
  side: Side;
  solve?: Solve;
  product: Product;
}

// --- quote (RFQ) lifecycle --------------------------------------------------

export interface Quote {
  quoteId: bigint;
  idempotencyKey: string;
  price: TwoWayPrice;
  greeks: Greeks;
  conventions: Conventions;
  resolvedStrike: number;
  epochNanos: bigint;
  /** Last-look window deadline, nanoseconds since the Unix epoch (UTC). */
  validUntilNanos: bigint;
  correlationId?: bigint;
  surfaceVersion?: bigint;
  /** Who quoted/holds this line (maker auto-pricer / requesting seat), if emitted. */
  attribution?: AttributionRecord;
}

export interface Execution {
  executionId: bigint;
  quoteId: bigint;
  side: Side;
  tradedPremium: number;
  instrument: Instrument;
  epochNanos: bigint;
  /** Attribution chain on the booked trade, if emitted. */
  attribution?: AttributionRecord;
}

// --- stream (RFS) -----------------------------------------------------------

/** Per-row stream health, derived from the contract's seq/resync semantics. */
export type StreamHealth = "HEALTHY" | "RESYNCING" | "STALE";

/** The full state of a streamed instrument at a sequence point. */
export interface Snapshot {
  subscriptionId: bigint;
  sequence: bigint;
  price: TwoWayPrice;
  greeks: Greeks;
  vol: number;
  conventions: Conventions;
  resolvedStrike: number;
  /** SELL@bid and BUY@offer tokens; empty ⇒ indicative-only. */
  tradable: TradableToken[];
  surfaceVersion?: bigint;
  correlationId?: bigint;
  epochNanos: bigint;
  /** Who quoted/holds this streamed line (maker auto-pricer / requesting seat), if emitted. */
  attribution?: AttributionRecord;
}

/** A sequenced delta update on a streamed instrument. */
export interface Update {
  subscriptionId: bigint;
  sequence: bigint;
  price: TwoWayPrice;
  greeks: Greeks;
  vol: number;
  tradable: TradableToken[];
  surfaceVersion?: bigint;
  epochNanos: bigint;
}

export type StreamEndReason =
  | "LAGGED"
  | "DRAINING"
  | "UNSUBSCRIBED"
  | "EXPIRED";

export interface Executed {
  subscriptionId: bigint;
  token: bigint;
  executionId: bigint;
  side: Side;
  tradedPremium: number;
  correlationId?: bigint;
  epochNanos: bigint;
  /** Attribution chain on the click-to-trade fill, if emitted. */
  attribution?: AttributionRecord;
}

export type StreamRejectReason = "EXPIRED" | "UNKNOWN_TOKEN" | "ALREADY_CONSUMED";

export interface StreamReject {
  subscriptionId: bigint;
  token: bigint;
  reason: StreamRejectReason;
  correlationId?: bigint;
  epochNanos: bigint;
}

// --- attribution (book / seat identity) -------------------------------------

/**
 * Who owns a quoted/held line — a human trader seat OR an automated pricer
 * (`celnet.wire.Owner`). Exactly one arm is set; engine-quoted edge flow is the
 * `autoPricer` seat (never anonymous).
 */
export type Owner =
  | { kind: "trader"; trader: string }
  | { kind: "autoPricer"; autoPricer: string };

/** A book/owner identity (`celnet.wire.BookId`): a named book plus its owner seat. */
export interface BookId {
  book: string;
  owner?: Owner;
}

/**
 * The attribution chain on a quoted line (`celnet.wire.AttributionRecord`):
 * `quotedBy` is the maker that priced it (the auto-pricer for engine-quoted flow);
 * `heldBy` is the requesting seat that holds the resulting position; `won`/`lpCount`
 * record the LP competition outcome when known.
 */
export interface AttributionRecord {
  quotedBy?: BookId;
  heldBy?: BookId;
  won?: boolean;
  lpCount?: number;
}

// --- market-series feed (multiplexed on the stream session) -----------------

/** One observed point of a market series: a timestamped value of the observable. */
export interface MarketSeriesPoint {
  subscriptionId: bigint;
  sequence: bigint;
  /** The observed value in the observable's natural unit (vol or a rate). */
  value: number;
  epochNanos: bigint;
}

/**
 * The opening baseline of a market series: recent history (oldest → newest) plus
 * the observable's identity, so a trend tile can label its unit and seed its line
 * before live points arrive.
 */
export interface MarketSeriesSnapshot {
  subscriptionId: bigint;
  sequence: bigint;
  pair: CcyPair;
  observable: MarketObservable;
  points: MarketSeriesPoint[];
  epochNanos: bigint;
}

// --- surface ----------------------------------------------------------------

/** One quoted vol point on the surface, keyed by signed convention delta. */
export interface SmilePoint {
  /** Signed convention delta (0.25 = 25Δ call, -0.25 = 25Δ put). */
  delta: number;
  tenorYears: number;
  /** Absolute volatility (0.10 = 10 vol). */
  vol: number;
}

/** The broker market-quote set for a single tenor (ATM + 25Δ/10Δ RR&BF). */
export interface BrokerQuoteSet {
  tenorYears: number;
  atmVol: number;
  rr25: number;
  bf25: number;
  rr10: number;
  bf10: number;
  hasTenDelta: boolean;
}

/** The arbitrage status of a calibrated smile/surface. */
export interface ArbReport {
  butterflyArbitrageFree: boolean;
  calendarArbitrageFree: boolean;
  /** Worst negative density observed (0.0 = none); larger magnitude = worse. */
  worstDensity: number;
  note: string;
}

/** The smile for one (pair, tenor): broker marks, calibrated points, arb report. */
export interface Smile {
  pair: CcyPair;
  tenorYears: number;
  brokerQuotes: BrokerQuoteSet;
  points: SmilePoint[];
  conventions: Conventions;
  arbitrage: ArbReport;
  epochNanos: bigint;
}

/** A marked surface version: per-tenor calibrated smiles. */
export interface MarkedSurface {
  pair: CcyPair;
  surfaceVersion: bigint;
  smiles: Smile[];
  epochNanos: bigint;
}

// --- scenario / risk --------------------------------------------------------

export type ShockFactor = "SPOT" | "VOL" | "RATE_DOM" | "RATE_FOR" | "TIME";

export interface ShockAxis {
  factor: ShockFactor;
  /** True ⇒ each step is relative (multiply 1+step); false ⇒ absolute (add). */
  relative: boolean;
  steps: number[];
}

/** One repriced node of a scenario grid. */
export interface ScenarioPoint {
  /** The applied shock per axis, parallel to the request axes. */
  appliedShocks: number[];
  shockedMarket: MarketContext;
  greeks: Greeks;
  expiryYears: number;
}

/** Vega bucketed by (tenor, delta-pillar). */
export interface VegaBucket {
  tenorYears: number;
  delta: number;
  vega: number;
}

export interface CrossGamma {
  factorA: ShockFactor;
  factorB: ShockFactor;
  value: number;
}

/** Book-shaped risk: bucketed vega, off-diagonal cross-gamma, theta roll. */
export interface BucketedRisk {
  vegaBuckets: VegaBucket[];
  crossGammas: CrossGamma[];
  thetaRoll: number[];
  rollHorizonsYears: number[];
}

export interface ScenarioResult {
  points: ScenarioPoint[];
  /**
   * Book-shaped risk decomposition. Present only when the scenario request asked
   * for it (a `RiskBucketRequest`); `null` when the request omitted risk buckets,
   * so a caller can render an honest "not requested" state rather than zeros.
   */
  bucketedRisk: BucketedRisk | null;
}

/** A single vega pillar to bucket on: a `(tenor, delta)` node of the surface. */
export interface VegaPillar {
  tenorYears: number;
  delta: number;
}

/** A factor pair to compute the off-diagonal cross-gamma for. */
export interface CrossGammaPair {
  factorA: ShockFactor;
  factorB: ShockFactor;
}

/**
 * The book-shaped risk decomposition request that accompanies a `Scenario` call.
 * The server only computes `bucketed_risk` when this is supplied — vega bucketed
 * per `(tenor, delta)` pillar, cross-gamma per distinct factor pair, and the theta
 * roll over the requested horizons (years rolled forward).
 */
export interface RiskBucketRequest {
  vegaPillars: VegaPillar[];
  crossGammaPairs: CrossGammaPair[];
  rollHorizonsYears: number[];
}
