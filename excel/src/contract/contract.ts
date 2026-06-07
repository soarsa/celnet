// ONE CONTRACT — minimal duplicate of `gui/src/data/contract.ts`, kept
// semantics-identical (CLAUDE.md rule 9: one current contract, no fork). The
// add-in is a separate Vite project (outside the GUI package and the cargo
// workspace), so it cannot import across that boundary; rather than fork the
// wire semantics we duplicate the *definitions* unchanged. Do NOT add
// Excel-specific fields or rename anything here — both files are TS projections
// of the SAME `celnet.wire` contract and must evolve together.
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

export type TenorUnit = "OVERNIGHT" | "WEEKS" | "MONTHS" | "YEARS";

/**
 * The smile-calibration family a surface is marked with (proto `SmileModel`).
 * Vendor/method-neutral names mirroring `celnet_proto::SmileModel`: MARKET_HEDGE
 * (the default Vanna-Volga market-hedge construction), and the three fitted
 * parametric families. Provenance of the model a surface was marked under is
 * echoed by the server on `Smile.arbitrage.note` as `model=<family>` (the frozen
 * contract carries no echo field — the note is the honest provenance channel).
 */
export type SmileModel =
  | "MARKET_HEDGE"
  | "STOCHASTIC_VOL"
  | "PARAMETRIC"
  | "PARAMETRIC_SURFACE";

/**
 * A streamable market observable (proto `MarketObservable`). The market-series
 * feed (`market_series_subscribe`) streams one of these as a scalar time series:
 * ATM_VOL/RISK_REVERSAL/BUTTERFLY are vols, SPOT/FORWARD are rates. RR/BF require
 * a delta wing; ATM_VOL/RR/BF/FORWARD require a tenor; SPOT is tenor-independent.
 */
export type MarketObservable =
  | "ATM_VOL"
  | "SPOT"
  | "RISK_REVERSAL"
  | "BUTTERFLY"
  | "FORWARD";

/**
 * How an Asian option's averaging observations are laid out over the window
 * (proto `AveragingStyle`). DISCRETE samples a fixed number of equally-spaced
 * future fixings; CONTINUOUS is the continuous-monitoring limit. Vendor/method-
 * neutral names mirroring `celnet_proto::AveragingStyle`.
 */
export type AveragingStyle = "DISCRETE" | "CONTINUOUS";

/**
 * Barrier crossing semantics for a single barrier level (proto `BarrierKind`).
 * KNOCK_IN activates the option when the barrier is touched; KNOCK_OUT
 * extinguishes it. Mirrors `celnet_proto::BarrierKind`
 * (BARRIER_KIND_KNOCK_IN=0, BARRIER_KIND_KNOCK_OUT=1).
 */
export type BarrierKind = "KNOCK_IN" | "KNOCK_OUT";

/**
 * Where a single barrier sits relative to spot at inception (proto `BarrierSide`).
 * UP is above spot (up-and-*); DOWN is below spot (down-and-*). Mirrors
 * `celnet_proto::BarrierSide` (BARRIER_SIDE_UP=0, BARRIER_SIDE_DOWN=1).
 */
export type BarrierSide = "UP" | "DOWN";

/**
 * How a barrier / touch is monitored along the path (proto `MonitoringStyle`).
 * CONTINUOUS triggers on any touch at any instant; DISCRETE tests only at the
 * scheduled fixing instants. Mirrors `celnet_proto::MonitoringStyle`
 * (MONITORING_STYLE_CONTINUOUS=0, MONITORING_STYLE_DISCRETE=1).
 */
export type MonitoringStyle = "CONTINUOUS" | "DISCRETE";

/**
 * The touch family for one-/no-/double-no-/double-one-touch structures (proto
 * `TouchKind`). ONE_TOUCH pays if the barrier IS touched; NO_TOUCH if it is NOT;
 * DOUBLE_NO_TOUCH if NEITHER of two barriers is touched; DOUBLE_ONE_TOUCH if
 * EITHER is touched. Mirrors `celnet_proto::TouchKind` (TOUCH_KIND_ONE_TOUCH=0,
 * NO_TOUCH=1, DOUBLE_NO_TOUCH=2, DOUBLE_ONE_TOUCH=3).
 */
export type TouchKind = "ONE_TOUCH" | "NO_TOUCH" | "DOUBLE_NO_TOUCH" | "DOUBLE_ONE_TOUCH";

/**
 * The settlement style of a digital (binary) option (proto `DigitalStyle`).
 * CASH_OR_NOTHING pays a fixed cash amount if in-the-money at expiry;
 * ASSET_OR_NOTHING pays one unit of the asset. Mirrors `celnet_proto::DigitalStyle`
 * (DIGITAL_STYLE_CASH_OR_NOTHING=0, DIGITAL_STYLE_ASSET_OR_NOTHING=1).
 */
export type DigitalStyle = "CASH_OR_NOTHING" | "ASSET_OR_NOTHING";

/**
 * The analytic estimator an Asian option is priced with (proto `AsianMethod`).
 * The arithmetic average of lognormal observations is not lognormal, so it is
 * priced by a fast analytic estimator: CURRAN (geometric-conditioning, the
 * accurate default) or TURNBULL_WAKEMAN (two-moment lognormal matching).
 * Vendor/method-neutral names mirroring `celnet_proto::AsianMethod`; the method
 * provenance (Curran / Turnbull-Wakeman) is documentation-only per the naming
 * guardrail — the identifiers are purpose-named.
 */
export type AsianMethod = "CURRAN" | "TURNBULL_WAKEMAN";

/**
 * The payoff a quanto option settles (proto `QuantoPayoff`). VANILLA pays the
 * standard call/put intrinsic in the (fixed) settlement currency; DIGITAL pays a
 * fixed cash amount on finishing in-the-money. Vendor/method-neutral names
 * mirroring `celnet_proto::QuantoPayoff`
 * (QUANTO_PAYOFF_VANILLA=0, QUANTO_PAYOFF_DIGITAL=1).
 */
export type QuantoPayoff = "VANILLA" | "DIGITAL";

/**
 * How the redeeming (target-breaching) fixing of a TARF settles — the gap-risk
 * convention (proto `TarfRedemption`). FULL_GAIN pays the breaching fixing's full
 * intrinsic gain (accumulated client gain may overshoot the target — genuine gap
 * exposure); CAPPED_GAIN pays only the remaining target (exact redemption, no
 * overshoot). Vendor/method-neutral names mirroring `celnet_proto::TarfRedemption`
 * (TARF_REDEMPTION_FULL_GAIN=0, TARF_REDEMPTION_CAPPED_GAIN=1).
 */
export type TarfRedemption = "FULL_GAIN" | "CAPPED_GAIN";

/**
 * The knock-out monitoring convention for an accumulator's up-and-out barrier
 * (proto `AccumulatorMonitoring`). DISCRETE tests the barrier only at the fixing
 * dates; CONTINUOUS monitors it between fixings via the Brownian-bridge crossing
 * probability (knocks out more often). Both are Monte-Carlo priced (the premium
 * carries a standard error). Mirrors `celnet_proto::AccumulatorMonitoring`
 * (ACCUMULATOR_MONITORING_DISCRETE=0, ACCUMULATOR_MONITORING_CONTINUOUS=1).
 */
export type AccumulatorMonitoring = "DISCRETE" | "CONTINUOUS";

/**
 * The two lookback families (proto `LookbackStyle`). FLOATING settles against the
 * path extremum (`S_T − min` call, `max − S_T` put — always ≥ 0); FIXED exercises
 * against a fixed `K` (`(max − K)⁺` call, `(K − min)⁺` put). Mirrors
 * `celnet_proto::LookbackStyle`
 * (LOOKBACK_STYLE_FLOATING=0, LOOKBACK_STYLE_FIXED=1).
 */
export type LookbackStyle = "FLOATING" | "FIXED";

/**
 * How a lookback's running extremum is monitored (proto `LookbackMonitoring`).
 * CONTINUOUS prices by the exact closed form (no Monte-Carlo std-error); DISCRETE
 * prices by Monte-Carlo over a fixed number of observations with the Brownian-
 * bridge extremum correction, reporting a standard error (`price_std_error`).
 * Mirrors `celnet_proto::LookbackMonitoring`
 * (LOOKBACK_MONITORING_CONTINUOUS=0, LOOKBACK_MONITORING_DISCRETE=1).
 */
export type LookbackMonitoring = "CONTINUOUS" | "DISCRETE";

// --- value messages ---------------------------------------------------------

/** An FX currency pair BASE/QUOTE (market form CCY1CCY2), e.g. EUR/USD. */
export interface CcyPair {
  /** Foreign / asset currency (CCY1), e.g. "EUR". */
  base: string;
  /** Domestic / numeraire currency (CCY2), e.g. "USD". */
  quote: string;
}

/** A standard FX-options tenor measured from the spot date. */
export interface Tenor {
  unit: TenorUnit;
  /** Number of units (weeks/months/years); ignored for OVERNIGHT. */
  count: number;
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

/**
 * A variance swap: pays realised variance against a fixed variance strike,
 * priced by 1/K² log-contract static replication (proto `VarianceSwap`). The
 * variance strike is `strikeVol²`; a fresh request may leave `strikeVol` zero
 * and read the fair strike off the response.
 */
export interface VarianceSwap {
  /** The fixed variance strike expressed as a volatility (absolute, e.g. 0.10). */
  strikeVol: number;
}

/**
 * A volatility swap: pays realised volatility against a fixed fair-vol strike,
 * priced by the convexity (Jensen) adjustment to the companion variance-swap
 * strike (proto `VolatilitySwap`).
 */
export interface VolatilitySwap {
  /** The fixed volatility strike (absolute, e.g. 0.10); zero ⇒ read fair off the response. */
  strikeVol: number;
}

/**
 * A fixed-strike arithmetic-average-rate Asian option (proto `AsianOption`). The
 * realised arithmetic average is not lognormal, so it is priced by a fast
 * analytic estimator (`method`); the geometric-conditioning estimator is the
 * accurate default.
 */
export interface AsianOption {
  optionType: OptionType;
  /** The strike `K` (quote per 1 unit of base). */
  strike: number;
  /** How the averaging observations are laid out across the window. */
  averaging: AveragingStyle;
  /** Number of equally-spaced future fixings; required `≥ 1` for DISCRETE, ignored for CONTINUOUS. */
  observations: number;
  /** Analytic estimator to price with (default CURRAN). */
  method: AsianMethod;
  /** Realised running arithmetic average of the already-fixed observations (seasoning). */
  elapsedAvg: number;
  /** Fraction `∈ [0, 1)` of the total average weight already accumulated; `0` ⇒ a fresh average. */
  elapsedWeight: number;
}

/**
 * A forward-start vanilla (proto `ForwardStart`). The strike is fixed at a future
 * reset date `reset` to `moneyness × S(reset)` (a proportional strike), and the
 * option then runs to the instrument's `expiryYears`. Priced by the dual-carry
 * strike-reset closed form. Vendor/method-neutral; provenance is doc-only.
 */
export interface ForwardStart {
  optionType: OptionType;
  /** Proportional strike multiplier applied to the spot fixed at the reset (e.g. 1.0 = ATM-at-reset). */
  moneyness: number;
  /** The reset (strike-fixing) date as a year fraction `∈ [0, expiryYears]`. */
  reset: number;
}

/**
 * A cliquet (ratchet) option (proto `Cliquet`): a sum of `periods` consecutive
 * forward-start legs over equal sub-periods. A *plain* ratchet (no per-period
 * clamps) is the exact sum of forward-start legs (closed form); supplying any
 * local/global floor or cap makes it a *clamped* cliquet, priced by Monte Carlo
 * (its `greeks.price` carries a standard error). All four clamp fields are
 * presence-tracked: absent ⇒ that side is unconstrained.
 */
export interface Cliquet {
  optionType: OptionType;
  /** Proportional strike multiplier for each period's forward-start leg (e.g. 1.0). */
  moneyness: number;
  /** Number of equal ratchet sub-periods (`≥ 1`). */
  periods: number;
  /** Per-period return floor (absent ⇒ no floor). */
  localFloor?: number;
  /** Per-period return cap (absent ⇒ no cap). */
  localCap?: number;
  /** Global (summed) payoff floor (absent ⇒ no floor). */
  globalFloor?: number;
  /** Global (summed) payoff cap (absent ⇒ no cap). */
  globalCap?: number;
  /** Monte-Carlo antithetic pairs for the clamped case (`0` ⇒ server default); ignored for a plain ratchet. */
  mcPairs: number;
  /** Monte-Carlo seed for the clamped case (bit-reproducible); ignored for a plain ratchet. */
  mcSeed: bigint;
}

/**
 * A quanto option (proto `Quanto`): an option whose payoff is computed on a
 * foreign-denominated underlying but settled in a fixed (domestic) currency at a
 * pre-agreed conversion, carrying a drift adjustment from the underlying↔FX
 * correlation. Priced by the closed-form quanto-adjusted estimator. Vendor/
 * method-neutral; provenance is doc-only.
 */
export interface Quanto {
  /** Whether the payoff is a vanilla intrinsic or a fixed-cash digital. */
  payoff: QuantoPayoff;
  optionType: OptionType;
  /** The strike `K` (absolute level in the underlying's quote terms). */
  strike: number;
  /** The volatility of the settlement-FX conversion rate (absolute, e.g. 0.09). */
  conversionVol: number;
  /** The correlation `∈ [-1, 1]` between the underlying and the settlement-FX rate. */
  correlation: number;
}

/**
 * A discrete fixing schedule for path-dependent structures (proto
 * `FixingSchedule`): the ascending fixing year-fractions (each ≤ the structure's
 * expiry) and the notional that accrues at each fixing. The server reads the
 * count (`fixingYears.length`) and per-fixing notional; a count-based ticket
 * encodes the year-fractions as `k/n` for `k = 1..=n` (the engine spaces fixings
 * equally over `[0, T]`), exactly matching the SDK's `equal_fixing_years`.
 */
export interface FixingSchedule {
  /** The fixing year fractions, ascending, each ≤ the structure's expiry. */
  fixingYears: number[];
  /** The notional that accrues at each fixing (usually one leg notional). */
  fixingNotional: number;
}

/**
 * A Target-Redemption Forward (proto `Tarf`): a strip of `schedule` geared
 * fixings at `strike` whose accumulated client gain knocks the structure out once
 * it reaches `target`. Monte-Carlo priced — the premium carries a standard error
 * (`price_std_error`). `optionType` selects the favourable direction (PUT = the
 * classic exporter TARF, gains when `S_k < strike`); `leverage` gears the adverse
 * leg; `redemption` is the gap-risk settlement of the breaching fixing.
 */
export interface Tarf {
  optionType: OptionType;
  /** The strike `K` of every fixing. */
  strike: number;
  /** The cumulative gain target; accumulated client gain at/above it redeems. */
  target: number;
  /** The gearing/leverage multiplier on the adverse (loss) leg (`≥ 0`). */
  leverage: number;
  /** The gap-risk settlement convention of the redeeming fixing. */
  redemption: TarfRedemption;
  /** The fixing schedule (count + per-fixing notional). */
  schedule: FixingSchedule;
  /** Monte-Carlo antithetic pairs (`0` ⇒ server default). */
  mcPairs: number;
  /** Monte-Carlo seed (bit-reproducible). */
  mcSeed: bigint;
}

/**
 * An accumulator (proto `Accumulator`): periodic accumulation at a `pivot` strike
 * with an up-and-out knock-out `barrier` (`barrier > pivot`) and `leverage`
 * gearing on the below-pivot (loss) leg. Monte-Carlo priced — the premium carries
 * a standard error (`price_std_error`). `monitoring` selects discrete (at fixings)
 * or continuous (Brownian-bridge between fixings) knock-out testing.
 */
export interface Accumulator {
  /** The pivot strike `K` at which the client accumulates each fixing. */
  pivot: number;
  /** The up-and-out knock-out barrier `B` (`B > pivot`). */
  barrier: number;
  /** The gearing/leverage multiplier on the below-pivot (loss) leg (`≥ 0`). */
  leverage: number;
  /** The knock-out monitoring convention. */
  monitoring: AccumulatorMonitoring;
  /** The fixing schedule (count + per-fixing notional). */
  schedule: FixingSchedule;
  /** Monte-Carlo antithetic pairs (`0` ⇒ server default). */
  mcPairs: number;
  /** Monte-Carlo seed (bit-reproducible). */
  mcSeed: bigint;
}

/**
 * A lookback option on the running extremum of the path (proto `Lookback`). The
 * CONTINUOUS variant prices by exact closed form (no std-error); the DISCRETE
 * variant prices by Monte-Carlo with the Brownian-bridge extremum correction and
 * reports a standard error (`price_std_error`). `strike` is used only by the FIXED
 * family (ignored for FLOATING, whose strike is the path extremum); `observations`
 * applies only to the DISCRETE variant.
 */
export interface Lookback {
  /** Floating- or fixed-strike family. */
  style: LookbackStyle;
  optionType: OptionType;
  /** Continuous (closed-form) or discrete (Monte-Carlo) monitoring. */
  monitoring: LookbackMonitoring;
  /** The strike `K`, used only by the FIXED-strike family (`0` for FLOATING). */
  strike: number;
  /** Equally-spaced monitoring observations for the DISCRETE variant (`0` ⇒ server default). */
  observations: number;
  /** Monte-Carlo antithetic pairs for the DISCRETE variant (`0` ⇒ server default). */
  mcPairs: number;
  /** Monte-Carlo seed for the DISCRETE variant (bit-reproducible). */
  mcSeed: bigint;
}

/**
 * A single-barrier knock-in / knock-out option (proto `SingleBarrier`): a vanilla
 * payoff plus one barrier. `kind` selects knock-in/out, `side` whether the barrier
 * sits above (up) or below (down) spot at inception, `barrier` the level, `rebate`
 * the amount paid if the barrier event extinguishes/never activates the option, and
 * `monitoring` continuous vs discrete crossing.
 */
export interface SingleBarrier {
  vanilla: Vanilla;
  kind: BarrierKind;
  side: BarrierSide;
  /** The barrier level (quote per 1 unit of base). */
  barrier: number;
  /** Rebate paid if the barrier event extinguishes/never activates the option. */
  rebate: number;
  monitoring: MonitoringStyle;
}

/**
 * A double-barrier option bounded by a lower and an upper barrier (proto
 * `DoubleBarrier`): a vanilla payoff plus two barriers. `kind` applies the
 * knock-in/out to whichever barrier is touched.
 */
export interface DoubleBarrier {
  vanilla: Vanilla;
  kind: BarrierKind;
  /** The lower barrier level (quote per 1 unit of base). */
  lowerBarrier: number;
  /** The upper barrier level (quote per 1 unit of base). */
  upperBarrier: number;
  /** Rebate paid on the barrier event. */
  rebate: number;
  monitoring: MonitoringStyle;
}

/**
 * A digital (binary) option paying a fixed amount on an in-the-money expiry
 * (proto `Digital`). `optionType` selects the above-strike (call) vs below-strike
 * (put) payoff; `style` cash-or-nothing vs asset-or-nothing; `payout` the fixed
 * payout amount (in domestic ccy for cash-or-nothing).
 */
export interface Digital {
  optionType: OptionType;
  /** The strike (quote per 1 unit of base). */
  strike: number;
  style: DigitalStyle;
  /** The fixed payout amount (in domestic ccy for cash-or-nothing). */
  payout: number;
}

/**
 * A touch structure — one-touch / no-touch / double-no-touch / double-one-touch
 * (proto `Touch`). `lowerBarrier` is the sole barrier for the single-barrier
 * (one-/no-touch) kinds; `upperBarrier` is used only by the double structures.
 * `rebate` is paid when the touch condition is satisfied.
 */
export interface Touch {
  kind: TouchKind;
  /** The (lower / sole) barrier level (quote per 1 unit of base). */
  lowerBarrier: number;
  /** The upper barrier level (double structures only; ignored for the single kinds). */
  upperBarrier: number;
  /** The rebate paid when the touch condition is satisfied. */
  rebate: number;
  monitoring: MonitoringStyle;
}

/** The product payoff carried by an Instrument (the proto `product` oneof). */
export type Product =
  | { kind: "vanilla"; vanilla: Vanilla }
  | { kind: "strategy"; strategy: Strategy }
  | { kind: "singleBarrier"; singleBarrier: SingleBarrier }
  | { kind: "doubleBarrier"; doubleBarrier: DoubleBarrier }
  | { kind: "digital"; digital: Digital }
  | { kind: "touch"; touch: Touch }
  | { kind: "varianceSwap"; varianceSwap: VarianceSwap }
  | { kind: "volatilitySwap"; volatilitySwap: VolatilitySwap }
  | { kind: "asianOption"; asianOption: AsianOption }
  | { kind: "forwardStart"; forwardStart: ForwardStart }
  | { kind: "cliquet"; cliquet: Cliquet }
  | { kind: "quanto"; quanto: Quanto }
  | { kind: "tarf"; tarf: Tarf }
  | { kind: "accumulator"; accumulator: Accumulator }
  | { kind: "lookback"; lookback: Lookback };

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
  /**
   * For a Monte-Carlo-priced product (e.g. a clamped cliquet), the standard error
   * of the mean of the `greeks.price` estimate (proto `price_std_error`, field 7;
   * presence-tracked — absent for the closed-form products whose price is exact).
   * Surfaced honestly so a cell never mistakes an MC estimate for closed-form
   * precision; the same field the SDK reads as `PricedLine::price_std_error`.
   */
  priceStdError?: number;
}

export interface Execution {
  executionId: bigint;
  quoteId: bigint;
  side: Side;
  tradedPremium: number;
  instrument: Instrument;
  epochNanos: bigint;
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
}

export type StreamRejectReason = "EXPIRED" | "UNKNOWN_TOKEN" | "ALREADY_CONSUMED";

export interface StreamReject {
  subscriptionId: bigint;
  token: bigint;
  reason: StreamRejectReason;
  correlationId?: bigint;
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
  /** Input broker marks that produced this smile — optional: a calibrated
   *  (returned) smile need not echo them, so the server may omit it. */
  brokerQuotes?: BrokerQuoteSet;
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

// --- market series (trend feed) ---------------------------------------------

/** One observed point in a market series: a timestamped scalar value. */
export interface MarketSeriesPoint {
  subscriptionId: bigint;
  /** Monotonic per-subscription sequence (snapshot seq + n). */
  sequence: bigint;
  /** Observed value in the observable's natural unit (vol or rate). */
  value: number;
  epochNanos: bigint;
}

/** The opening baseline for a market series (history + observable identity). */
export interface MarketSeriesSnapshot {
  subscriptionId: bigint;
  sequence: bigint;
  pair: CcyPair;
  observable: MarketObservable;
  /** Recent history, oldest → newest (at most the requested history limit). */
  points: MarketSeriesPoint[];
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
  bucketedRisk: BucketedRisk;
}
