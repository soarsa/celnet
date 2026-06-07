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

/**
 * The crossing semantics of a barrier level (`celnet.wire.BarrierKind`):
 * `KNOCK_IN` activates the option only once the barrier is touched; `KNOCK_OUT`
 * extinguishes it when the barrier is touched. Purpose-named, vendor/method-neutral.
 */
export type BarrierKind = "KNOCK_IN" | "KNOCK_OUT";

/**
 * Where a single barrier sits relative to spot at inception
 * (`celnet.wire.BarrierSide`): `UP` is above spot (up-and-*), `DOWN` is below
 * (down-and-*).
 */
export type BarrierSide = "UP" | "DOWN";

/**
 * How a barrier / touch is monitored along the path (`celnet.wire.MonitoringStyle`):
 * `CONTINUOUS` triggers on any touch at any instant (the closed-form
 * reflection-principle regime), `DISCRETE` only at scheduled fixing instants.
 */
export type MonitoringStyle = "CONTINUOUS" | "DISCRETE";

/**
 * The touch family of a touch structure (`celnet.wire.TouchKind`): `ONE_TOUCH`
 * pays if the (single) barrier IS touched before expiry; `NO_TOUCH` pays if it is
 * NOT; `DOUBLE_NO_TOUCH` pays if NEITHER of two barriers is touched;
 * `DOUBLE_ONE_TOUCH` pays if EITHER is touched. Purpose-named, vendor/method-neutral.
 */
export type TouchKind =
  | "ONE_TOUCH"
  | "NO_TOUCH"
  | "DOUBLE_NO_TOUCH"
  | "DOUBLE_ONE_TOUCH";

/**
 * The settlement style of a digital (binary) option (`celnet.wire.DigitalStyle`):
 * `CASH_OR_NOTHING` pays a fixed cash amount if in-the-money at expiry;
 * `ASSET_OR_NOTHING` pays one unit of the asset (worth `S_T`) if in-the-money.
 */
export type DigitalStyle = "CASH_OR_NOTHING" | "ASSET_OR_NOTHING";

/**
 * The averaging schedule of an Asian option (`celnet.wire.AveragingStyle`):
 * `DISCRETE` samples at a finite count of fixings, `CONTINUOUS` averages the
 * whole window (the `n → ∞` limit). Purpose-named, vendor/method-neutral.
 */
export type AveragingStyle = "DISCRETE" | "CONTINUOUS";

/**
 * The analytic Asian estimator the server prices the arithmetic average with
 * (`celnet.wire.AsianMethod`): `CURRAN` (geometric-conditioning, the default)
 * or `TURNBULL_WAKEMAN` (two-moment lognormal matching). The mathematical
 * family each maps to lives in the server's doc comments, never in this
 * identifier (CLAUDE.md rule 8).
 */
export type AsianMethod = "CURRAN" | "TURNBULL_WAKEMAN";

/**
 * The single payoff a quanto wraps (`celnet.wire.QuantoPayoff`): a plain
 * `VANILLA` call/put, or a cash-or-nothing `DIGITAL` that pays one unit of the
 * fixed settlement currency when in the money. Purpose-named, vendor/method-neutral.
 */
export type QuantoPayoff = "VANILLA" | "DIGITAL";

/**
 * How the redeeming (target-breaching) fixing of a TARF settles
 * (`celnet.wire.TarfRedemption`): `FULL_GAIN` pays the breaching fixing's full
 * intrinsic gain (the accumulated gain may overshoot the target — genuine gap
 * exposure); `CAPPED_GAIN` pays only the remaining target (exact redemption, no
 * overshoot). The spread between the two is the explicit gap-risk premium.
 */
export type TarfRedemption = "FULL_GAIN" | "CAPPED_GAIN";

/**
 * The knock-out monitoring convention for an accumulator's up-and-out barrier
 * (`celnet.wire.AccumulatorMonitoring`): `DISCRETE` tests the barrier only at the
 * fixing dates; `CONTINUOUS` monitors it between fixings via the Brownian-bridge
 * crossing probability (knocks out more often, shrinking the structure's value).
 */
export type AccumulatorMonitoring = "DISCRETE" | "CONTINUOUS";

/**
 * The two lookback families (`celnet.wire.LookbackStyle`): `FLOATING` settles
 * against the path extremum (`S_T − min` call, `max − S_T` put; always finishes
 * at or above zero); `FIXED` exercises optimally against a fixed `K`
 * (`(max − K)⁺` call, `(K − min)⁺` put).
 */
export type LookbackStyle = "FLOATING" | "FIXED";

/**
 * How a lookback's running extremum is monitored (`celnet.wire.LookbackMonitoring`):
 * `CONTINUOUS` is priced by the exact closed form (no Monte-Carlo std-error);
 * `DISCRETE` is priced by Monte-Carlo with the Brownian-bridge extremum
 * correction over a fixed observation count, reporting a standard error
 * (`Quote.priceStdError`). The mathematical family each maps to lives in the
 * server's doc comments, never in this identifier (CLAUDE.md rule 8).
 */
export type LookbackMonitoring = "CONTINUOUS" | "DISCRETE";

/**
 * The pricing/booking model an instrument is priced under
 * (`celnet.wire.PricingModel`). Purpose-named, vendor/method-neutral (CLAUDE.md
 * rule 8) — the mathematical family lives in the server's doc comments, never in
 * this identifier:
 *  - `DEFAULT`          — the per-product closed-form / analytic engine (proto 0)
 *  - `LOCAL_STOCH_VOL`  — a particle-calibrated local-stochastic-volatility model
 *                         priced on an ADI-PDE (+ Monte-Carlo) engine (proto 1)
 *
 * Carried on the `Instrument` (proto field 22, ALONGSIDE `solve` at field 6) so it
 * reaches every flow — price / quote / stream / scenario — uniformly, exactly like
 * the `Solve` directive. Absent ⇒ `DEFAULT` ⇒ byte-identical to the legacy path.
 */
export type PricingModel = "DEFAULT" | "LOCAL_STOCH_VOL";

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
 *  - `EXTENDED_SURFACE`    — an extended whole-surface calibration with a
 *                            maturity-dependent skew family (proto value 4)
 */
export type SmileModel =
  | "MARKET_HEDGE"
  | "STOCHASTIC_VOL"
  | "PARAMETRIC"
  | "PARAMETRIC_SURFACE"
  | "EXTENDED_SURFACE";

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

/**
 * A single-barrier knock-in / knock-out option (`celnet.wire.SingleBarrier`): a
 * vanilla payoff (`vanilla`) that activates (`KNOCK_IN`) or extinguishes
 * (`KNOCK_OUT`) when the continuously-monitored spot first touches `barrier`,
 * optionally paying a `rebate` (at hit for a knocked-out out-option, at expiry for
 * a never-knocked-in in-option). `side` is the barrier's position relative to spot
 * at inception (up/down). Priced by the reflection-principle closed form
 * server-side; in/out parity (`knock_in + knock_out = vanilla`) holds by
 * construction.
 */
export interface SingleBarrier {
  /** The underlying vanilla payoff (call/put + strike or delta). */
  vanilla: Vanilla;
  kind: BarrierKind;
  side: BarrierSide;
  /** The barrier level `H` (quote per 1 unit of base). */
  barrier: number;
  /** Rebate paid on the barrier event (`0` for a plain barrier). */
  rebate: number;
  monitoring: MonitoringStyle;
}

/**
 * A double-barrier option bounded by a lower and an upper barrier
 * (`celnet.wire.DoubleBarrier`): a vanilla that knocks out if EITHER barrier is
 * touched (`KNOCK_OUT`), or activates if either is touched (`KNOCK_IN`, priced by
 * parity `KI = vanilla − KO`). Requires `0 < lowerBarrier < upperBarrier`. Priced
 * server-side by the method-of-images corridor series.
 */
export interface DoubleBarrier {
  /** The underlying vanilla payoff (call/put + strike or delta). */
  vanilla: Vanilla;
  kind: BarrierKind;
  /** The lower barrier level `L` (`0 < L < U`). */
  lowerBarrier: number;
  /** The upper barrier level `U`. */
  upperBarrier: number;
  /** Rebate paid on the barrier event (`0` for a plain corridor). */
  rebate: number;
  monitoring: MonitoringStyle;
}

/**
 * A digital (binary) option (`celnet.wire.Digital`): pays a fixed `payout` (in the
 * settlement style's unit) if it finishes in the money at expiry. A digital `CALL`
 * pays when `S_T > strike`, a `PUT` when `S_T < strike`. `style` selects
 * cash-or-nothing (one unit of domestic cash) vs asset-or-nothing (one unit of the
 * foreign asset). Priced by the closed-form digital value scaled by `payout`.
 */
export interface Digital {
  optionType: OptionType;
  /** The strike `K` (quote per 1 unit of base). */
  strike: number;
  style: DigitalStyle;
  /** The fixed payout amount (in domestic ccy for cash-or-nothing). */
  payout: number;
}

/**
 * A touch structure (`celnet.wire.Touch`): one-touch / no-touch / double-no-touch /
 * double-one-touch. `lowerBarrier` is the sole barrier for the single-barrier
 * kinds (`ONE_TOUCH`/`NO_TOUCH`); the double kinds additionally use `upperBarrier`
 * (requiring `0 < lowerBarrier < upperBarrier`). Pays the `rebate` (in domestic
 * cash) when its touch condition is satisfied. Priced server-side by the
 * reflection-principle first-passage / corridor-survival closed forms.
 */
export interface Touch {
  kind: TouchKind;
  /** The (lower / sole) barrier level. */
  lowerBarrier: number;
  /** The upper barrier level (double structures only; ignored otherwise). */
  upperBarrier: number;
  /** The rebate paid when the touch condition is satisfied. */
  rebate: number;
  monitoring: MonitoringStyle;
}

/**
 * A variance swap (`celnet.wire.VarianceSwap`): pays realised variance against a
 * fixed variance strike. `strikeVol` is the strike quoted in VOL terms (the fair
 * variance strike is `strikeVol²`); `0` ⇒ request the fair strike off the reply.
 */
export interface VarianceSwap {
  strikeVol: number;
}

/**
 * A volatility swap (`celnet.wire.VolatilitySwap`): pays realised volatility
 * against a fixed vol strike. `strikeVol` is the strike in vol terms; `0` ⇒
 * request the fair strike off the reply.
 */
export interface VolatilitySwap {
  strikeVol: number;
}

/**
 * An arithmetic-average-rate Asian option (`celnet.wire.AsianOption`).
 * `averaging` selects discrete vs continuous fixings; `observations` is the
 * fixing count for the discrete schedule (ignored for continuous); `method`
 * selects the analytic estimator; `elapsedAvg`/`elapsedWeight` carry the
 * in-progress (seasoned) average and its weight `∈ [0,1)` (both `0` ⇒ fresh).
 */
export interface AsianOption {
  optionType: OptionType;
  strike: number;
  averaging: AveragingStyle;
  observations: number;
  method: AsianMethod;
  elapsedAvg: number;
  elapsedWeight: number;
}

/**
 * A forward-start vanilla (`celnet.wire.ForwardStart`): the strike is fixed at
 * the reset date `reset` to `moneyness · S(reset)` and the option pays the
 * vanilla payoff at the enclosing `Instrument.expiryYears`. Priced by the FX
 * dual-carry forward-start closed form `V = e^{−r_f·t₁}·S₀·u(m, T−t₁)` (with
 * `u` the unit-spot vanilla over the residual maturity); requires `0 ≤ reset ≤ T`.
 */
export interface ForwardStart {
  optionType: OptionType;
  /** Strike-reset multiple `m` (`m = 1` is the at-the-money-forward reset). */
  moneyness: number;
  /** Reset (strike-fixing) date `t₁` in years, with `0 ≤ reset ≤ expiryYears`. */
  reset: number;
}

/**
 * A cliquet / ratchet (`celnet.wire.Cliquet`): a strip of consecutive
 * forward-start vanillas over an evenly-spaced reset schedule, with optional
 * per-period local floor/cap on each leg's option return and an optional global
 * floor/cap on the accumulated payoff. A PLAIN (unclamped) ratchet prices in
 * closed form as the exact sum of forward-start legs; ANY local/global clamp
 * switches the pricer to a Monte-Carlo estimator that reports `price` plus its
 * standard error (`Quote.priceStdError`). `0` `mcPairs` ⇒ a server default;
 * `mcPairs`/`mcSeed` are ignored for a plain ratchet. Each clamp is
 * presence-tracked — `undefined` ⇒ unconstrained.
 */
export interface Cliquet {
  optionType: OptionType;
  /** Per-period strike-reset multiple `m` (applied to every leg). */
  moneyness: number;
  /** Number of evenly-spaced ratchet periods over `[0, expiryYears]`; `≥ 1`. */
  periods: number;
  localFloor?: number;
  localCap?: number;
  globalFloor?: number;
  globalCap?: number;
  /** Antithetic Monte-Carlo path pairs for the clamped variant; `0` ⇒ default. */
  mcPairs: number;
  /** Counter-RNG seed for the clamped Monte-Carlo estimator (reproducible). */
  mcSeed: bigint;
}

/**
 * A quanto option (`celnet.wire.Quanto`): a vanilla or cash-or-nothing digital
 * whose natural payoff is converted into a fixed settlement currency at a fixed
 * rate. Priced by the closed-form quanto-drift adjustment `−ρ·σ_S·σ_Z` to the
 * underlying carry under the settlement-currency measure; at `correlation = 0`
 * the adjustment vanishes and the price collapses to the plain vanilla/digital.
 */
export interface Quanto {
  payoff: QuantoPayoff;
  optionType: OptionType;
  /** The strike `K` (quote per 1 unit of base). */
  strike: number;
  /** Annualised volatility `σ_Z` of the settlement-conversion rate (`≥ 0`). */
  conversionVol: number;
  /** Correlation `ρ ∈ [−1, 1]` between the spot and the conversion rate. */
  correlation: number;
}

/**
 * A discrete fixing schedule for path-dependent structures
 * (`celnet.wire.FixingSchedule`): the ascending fixing year fractions (each ≤ the
 * structure's expiry) and the notional that accrues at each fixing. The number of
 * fixings is `fixingYears.length`. Shared by the TARF and accumulator (CLAUDE.md
 * rule 9: one current contract — the same message is reused, not forked).
 */
export interface FixingSchedule {
  /** The fixing year fractions, ascending, each ≤ the structure's expiry. */
  fixingYears: number[];
  /** The notional that accrues at each fixing (usually one leg notional). */
  fixingNotional: number;
}

/**
 * A Target-Redemption Forward (`celnet.wire.Tarf`): a strip of geared fixings at
 * a single `strike`, accumulating client gains until a cumulative `target` is
 * reached (which redeems the structure). Always priced by Monte-Carlo (the value
 * is the BANK's present value), so the reply always carries a standard error
 * (`Quote.priceStdError`). `0` `mcPairs` ⇒ a server default; the number of fixings
 * is `schedule.fixingYears.length`.
 */
export interface Tarf {
  /**
   * The payoff direction whose favourable side accrues client gains: a `PUT`
   * means the client gains when `S_k < strike` (the classic exporter TARF), a
   * `CALL` when `S_k > strike`.
   */
  optionType: OptionType;
  /** The strike `K` of every fixing. */
  strike: number;
  /** The cumulative gain target; accumulated gain at or above it redeems. */
  target: number;
  /** The gearing/leverage multiplier on the adverse (loss) leg (`≥ 0`). */
  leverage: number;
  /** The gap-risk settlement convention of the redeeming fixing. */
  redemption: TarfRedemption;
  /** The fixing schedule (year fractions + per-fixing notional). */
  schedule: FixingSchedule;
  /** Antithetic Monte-Carlo path pairs; `0` ⇒ a server default. */
  mcPairs: number;
  /** Counter-RNG seed (identical seeds reproduce results bit-for-bit). */
  mcSeed: bigint;
}

/**
 * An accumulator (`celnet.wire.Accumulator`): periodic accumulation at a `pivot`
 * strike with an up-and-out knock-out `barrier` (`barrier > pivot`) and `leverage`
 * gearing on the below-pivot (loss) leg. Always priced by Monte-Carlo (the value
 * is the CLIENT's present value), so the reply always carries a standard error
 * (`Quote.priceStdError`). `monitoring` selects discrete-at-fixings vs continuous
 * (Brownian-bridge) knock-out testing; the number of fixings is
 * `schedule.fixingYears.length`.
 */
export interface Accumulator {
  /** The pivot strike `K` at which the client accumulates each fixing. */
  pivot: number;
  /** The up-and-out knock-out barrier `B` (`B > pivot`). */
  barrier: number;
  /** The gearing/leverage multiplier on the below-pivot (loss) leg (`≥ 0`). */
  leverage: number;
  /** The knock-out monitoring convention (discrete-at-fixings or continuous). */
  monitoring: AccumulatorMonitoring;
  /** The fixing schedule (year fractions + per-fixing notional). */
  schedule: FixingSchedule;
  /** Antithetic Monte-Carlo path pairs; `0` ⇒ a server default. */
  mcPairs: number;
  /** Counter-RNG seed (identical seeds reproduce results bit-for-bit). */
  mcSeed: bigint;
}

/**
 * A lookback option on the running path extremum (`celnet.wire.Lookback`). The
 * CONTINUOUS-monitoring variant prices by the exact closed form (no Monte-Carlo
 * std-error); the DISCRETE-monitoring variant prices by Monte-Carlo with the
 * Brownian-bridge extremum correction over `observations` observations and reports
 * a standard error (`Quote.priceStdError`). `strike` is used only by the FIXED
 * family (the FLOATING family's strike is the path extremum). `observations`,
 * `mcPairs` and `mcSeed` are ignored for CONTINUOUS monitoring.
 */
export interface Lookback {
  /** Floating- or fixed-strike family. */
  style: LookbackStyle;
  optionType: OptionType;
  /** Continuous (closed-form) or discrete (Monte-Carlo) monitoring. */
  monitoring: LookbackMonitoring;
  /** The strike `K`, used only by the FIXED family (ignored for FLOATING). */
  strike: number;
  /** Equally-spaced monitoring observations for DISCRETE; `0` ⇒ a server default. */
  observations: number;
  /** Antithetic Monte-Carlo path pairs for DISCRETE; `0` ⇒ a server default. */
  mcPairs: number;
  /** Counter-RNG seed for the DISCRETE Monte-Carlo estimator (reproducible). */
  mcSeed: bigint;
}

/**
 * A window-barrier option (`celnet.wire.WindowBarrier`, proto field 23 in the
 * product oneof): a vanilla payoff that knocks OUT only if the
 * continuously-monitored spot breaches `barrier` DURING the active window
 * `[windowStart, windowEnd]` (a partial-time / window barrier); outside the window
 * the barrier is dormant. It has NO closed form, so it is LOCAL_STOCH_VOL-only —
 * an `Instrument` carrying a window barrier MUST select `pricingModel:
 * "LOCAL_STOCH_VOL"` (the DEFAULT model rejects it server-side). Priced server-side
 * on the LSV ADI-PDE (`mcPairs = 0`, no std-error) or by Monte-Carlo (`mcPairs >
 * 0`, carries `Quote.priceStdError`).
 */
export interface WindowBarrier {
  /** The underlying vanilla payoff (call/put + strike or delta). */
  vanilla: Vanilla;
  /** The barrier level `H` (quote per 1 unit of base). */
  barrier: number;
  /** Where the barrier sits relative to spot (`UP` = up-and-out, `DOWN` = down-and-out). */
  side: BarrierSide;
  /** Window open in years (`0 ≤ windowStart < windowEnd ≤ expiryYears`). */
  windowStart: number;
  /** Window close in years. */
  windowEnd: number;
  /**
   * Antithetic Monte-Carlo path pairs: `0` ⇒ the exact ADI PDE (no std-error);
   * `> 0` ⇒ Monte-Carlo on the LSV engine (reports `Quote.priceStdError`).
   */
  mcPairs: number;
  /** Monitoring time steps for the PDE/MC engine; `0` ⇒ a server default. */
  mcSteps: number;
  /** Counter-RNG seed for the Monte-Carlo estimator (reproducible). */
  mcSeed: bigint;
}

/**
 * The early-exercise style of an option (proto `ExerciseStyle`): AMERICAN
 * (continuous exercise up to expiry) or BERMUDAN (exercise only on the discrete
 * `AmericanOption.bermudanDates`).
 */
export type ExerciseStyle = "AMERICAN" | "BERMUDAN";

/**
 * An American / Bermudan early-exercise vanilla (proto `AmericanOption`, product
 * field 24). Physically-settled FX options trade American-style. The default
 * engine is the projected-SOR free-boundary finite difference (exact, no
 * std-error); `lsmPaths > 0` selects the Longstaff-Schwartz regression
 * Monte-Carlo (which reports `Quote.priceStdError`). BERMUDAN exercises only on
 * `bermudanDates` (year-fractions in `(0, expiryYears]`; expiry always
 * exercisable). Scope: American/Bermudan VANILLA.
 */
/**
 * How the per-leg terminal levels of a correlated multi-asset option combine
 * into the option underlying (mirrors `celnet_proto::BasketKind`).
 */
export type BasketKind = "BASKET" | "BEST_OF" | "WORST_OF";

/** One leg of a correlated multi-asset option (proto `BasketLeg`). */
export interface BasketLeg {
  /** The currency pair of this leg (identifies the underlying). */
  pair: CcyPair;
  /** The leg weight `w_a` (may be negative for a short leg). */
  weight: number;
  /** The leg spot FX level `S_a(0)`. */
  spot: number;
  /** The leg annualised lognormal volatility `σ_a`. */
  vol: number;
  /** The leg continuously-compounded foreign (base) rate `r_f,a`. */
  rFor: number;
}

/**
 * A correlated multi-asset FX option (proto `BasketOption`, product field 25): a
 * weighted BASKET, or a BEST_OF / WORST_OF (rainbow) over N currency-pair legs.
 * Priced by Cholesky-correlated multi-asset GBM Monte-Carlo, so the price carries
 * a `priceStdError`. Multi-asset Greeks are deferred (the strip is zeroed). The
 * enclosing `Instrument.pair` is the settlement / numeraire pair; the underlyings
 * are the per-leg pairs and the shared domestic rate is the request market
 * context's `rDom`.
 */
export interface BasketOption {
  /** The legs (one FX underlying each); at least one. */
  legs: BasketLeg[];
  /** The row-major N×N correlation matrix (length N²); SPD required. */
  correlations: number[];
  optionType: OptionType;
  /** The strike `K` on the aggregated underlying. */
  strike: number;
  /** The aggregation kind. */
  kind: BasketKind;
  /** Scrambled-Sobol points per replication (`0` ⇒ server default). */
  mcPaths: number;
  /** Independent randomized scrambles (`0` ⇒ server default; `≥ 2`). */
  mcReplications: number;
  /** Time steps per path (`0` ⇒ server default). */
  mcSteps: number;
  /** The base scramble seed (reproducible). */
  mcSeed: bigint;
}

export interface AmericanOption {
  optionType: OptionType;
  /** The strike `K` (absolute level, quote per 1 unit of base). */
  strike: number;
  /** Continuous (AMERICAN) or discrete-date (BERMUDAN) exercise. */
  exerciseStyle: ExerciseStyle;
  /** BERMUDAN exercise dates as year-fractions in `(0, expiryYears]`. Ignored for AMERICAN. */
  bermudanDates: number[];
  /** Longstaff-Schwartz path count: `0` ⇒ the exact FD engine; `> 0` ⇒ LSM (reports a std-error). */
  lsmPaths: number;
  /** LSM equally-spaced exercise opportunities for AMERICAN (`0` ⇒ server default; ignored for BERMUDAN). */
  lsmExerciseDates: number;
  /** Sobol scramble seed for the LSM engine (reproducible; ignored for FD). */
  lsmSeed: bigint;
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
  | { kind: "lookback"; lookback: Lookback }
  | { kind: "windowBarrier"; windowBarrier: WindowBarrier }
  | { kind: "american"; american: AmericanOption }
  | { kind: "basket"; basket: BasketOption };

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
  /**
   * The pricing/booking model the instrument is priced under (`Instrument
   * .pricing_model`, proto field 22). Presence-tracked: absent/`DEFAULT` ⇒ the
   * per-product closed-form engine (byte-identical to the legacy path);
   * `LOCAL_STOCH_VOL` routes the supported products (vanilla / single barrier /
   * window barrier) through the server's local-stochastic-volatility engine.
   */
  pricingModel?: PricingModel;
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
  /**
   * The Monte-Carlo standard error of the priced `price` (`PriceResponse
   * .price_std_error`, field 7). Presence-tracked: set ONLY for an MC-priced
   * product (a clamped cliquet), `undefined` for every closed-form product — so
   * the GUI shows an honest precision band for MC and never claims a stderr for
   * a closed form.
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

/**
 * The server liveness beat on a subscription (`celnet.wire.Heartbeat`). Carries
 * the current sequence (so a silent gap is detectable) PLUS additive server
 * observability surfaced off the hot path (proto fields 4–9, all appended — the
 * pinned zero-alloc pricing core is untouched; these are real drain-side
 * measurements, never fabricated):
 *  - `conflationDrops`     — the exact `celnet-fanout` SPMC-ring skip count for
 *                            this subscription (`received + skipped == produced`);
 *                            non-decreasing, 0 ⇒ the consumer never lagged.
 *  - `serverPriceP50Nanos` / `serverPriceP99Nanos` / `serverPriceP999Nanos` —
 *                            the drain-side price-compute latency percentiles
 *                            (HdrHistogram, ns); 0 until the first update is timed.
 *  - `surfaceVersion`      — provenance echo (0 ⇒ live/unpinned).
 *  - `correlationId`       — provenance echo (0 ⇒ none).
 */
export interface Heartbeat {
  subscriptionId: bigint;
  sequence: bigint;
  conflationDrops: bigint;
  serverPriceP50Nanos: bigint;
  serverPriceP99Nanos: bigint;
  serverPriceP999Nanos: bigint;
  surfaceVersion?: bigint;
  correlationId?: bigint;
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
  /**
   * The TYPED, authoritative calibration-family provenance (`celnet.wire.ArbReport.smile_model`,
   * proto field 5). This is the single source of truth for "which model was this
   * marked under" — read it directly. The `note` still embeds a human `model=<label>`
   * token, but it is NOT authoritative and must never be regex-scraped by code.
   */
  model: SmileModel;
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

// --- hierarchical risk (RiskService) ----------------------------------------
//
// The wire face of the single-node risk estate: server-side hierarchical risk
// over the org cube (`celnet-risk-cube` aggregation, `celnet-risk-normalize`
// common-numeraire conversion, `celnet-limits` utilization/RAG,
// `celnet-entitlements` pre-aggregation pruning). Aggregation is owned by the
// SERVER (CLAUDE.md rule 11 / API-first parity): a client never loops positions
// and sums — it lists positions, asks for a rolled-up node tree over an org
// dimension, drills a node to its constituents, and reads limit utilization, all
// behind this one contract. Mirrors `celnet.proto` `service RiskService`.

/**
 * The organizational dimension the cube groups by (`celnet.wire.RiskDimension`,
 * mirrors `celnet_risk_cube::DimensionId` + the firm apex). ORTHOGONAL axes —
 * these are different ways to slice the SAME fact cube, not a nesting order. The
 * proto enum NUMBER is canonical on both gRPC and the WS mirror.
 */
export type RiskDimension =
  | "FIRM" // =0 (apex; the cube firm_aggregate, default)
  | "TRADER" // =1
  | "BOOK" // =2
  | "DESK" // =3
  | "CCY_PAIR" // =4
  | "LOCATION" // =5
  | "ENTITY"; // =6

/**
 * The org key of a position/leaf (`celnet.wire.OrgKey`, mirrors
 * `celnet_risk_cube::FactKey`). Identifiers are the cube's interned dimension
 * handles (`u32`); `desk=0` ⇒ resolve from the book's Book→Desk parent pointer,
 * `entity=0` ⇒ resolve from Location→Entity. The GUI treats these as opaque
 * numeric handles (it never invents them; the server interns them from a fill's
 * attribution).
 */
export interface OrgKey {
  trader: number;
  book: number;
  desk: number;
  ccyPair: CcyPair;
  location: number;
  entity: number;
}

/**
 * A `(dimension, value)` key — the space the cube groups by and an entitlement
 * scope covers (`celnet.wire.RiskScope`). `value` is `FactKey::group_value` (for
 * Desk/Entity the resolved ancestor handle); ignored for `FIRM`.
 */
export interface RiskScope {
  dimension: RiskDimension;
  value: bigint;
}

/**
 * An entitlement rule (`celnet.wire.EntitlementRule`): a conjunction of scopes an
 * actor may see. Empty `scopes` ⇒ covers everything (the firm root).
 */
export interface EntitlementRule {
  scopes: RiskScope[];
}

/**
 * The entitlement principal (`celnet.wire.EntitlementPrincipal`, mirrors
 * `celnet_entitlements`). DEFAULT = grant-all: a request that OMITS the principal
 * is treated by the server as grant-all (the GUI's show-all-now posture). A
 * present principal with `grantAll=false` and no grants is deny-by-default; a
 * `deny` rule applies to any principal (deny wins).
 */
export interface EntitlementPrincipal {
  grantAll: boolean;
  grants: EntitlementRule[];
  denies: EntitlementRule[];
}

/** One reporting-numeraire spot rate (`celnet.wire.NumeraireRate`): units of the
 * numeraire per 1 unit of `ccy` at spot (finite, > 0). */
export interface NumeraireRate {
  ccy: string;
  rate: number;
}

/**
 * The reporting numeraire (`celnet.wire.ReportingNumeraire`, mirrors a server
 * `SpotResolver`): the currency every node measure is collapsed into, plus the
 * spot rates to convert each leg into it. The numeraire's own rate is implicitly
 * 1.0; a missing rate fails the request loudly server-side (no silent leg drop).
 */
export interface ReportingNumeraire {
  numeraire: string;
  rates: NumeraireRate[];
}

/**
 * A vega-ladder pillar key (`celnet.wire.VegaPillar`, mirrors
 * `celnet_risk_cube::VegaPillar`): tenor in DAYS and delta in BASIS POINTS
 * (0.25Δ → 2500). The grid is external request data, never compiled in.
 */
export interface RiskVegaPillar {
  tenorDays: number;
  deltaBp: number;
}

/** One rung of the aggregated vega ladder (`celnet.wire.VegaLadderBucket`), in the
 * reporting numeraire. */
export interface VegaLadderBucket {
  pillar: RiskVegaPillar;
  vega: number;
}

/** One signed leg of a netted delta vector (`celnet.wire.CcyExposureLeg`). */
export interface CcyExposureLeg {
  ccy: string;
  amount: number;
}

/**
 * The ADDITIVE risk of a node (`celnet.wire.AdditiveRisk`): the per-leaf Greeks
 * summed and collapsed into the reporting numeraire. `deltaVector` is the netted
 * per-ccy delta exposure (a high-spot pair no longer dominates a raw sum — every
 * leg is already in common units). `vegaNumeraire` is converted through each
 * leaf's PREMIUM currency.
 */
export interface AdditiveRisk {
  deltaNumeraire: number;
  deltaVector: CcyExposureLeg[];
  gamma: number;
  vegaNumeraire: number;
  theta: number;
  vanna: number;
  volga: number;
  charm: number;
  speed: number;
  zomma: number;
  color: number;
  premiumNumeraire: number;
  vegaLadder: VegaLadderBucket[];
}

/**
 * The NON-ADDITIVE risk of a node (`celnet.wire.NonAdditiveRisk`): RE-DERIVED per
 * node (never summed). Each field is presence-tracked — `undefined` ⇒ not
 * evaluated this cycle (e.g. no VaR shocks requested), NEVER a spurious zero.
 */
export interface NonAdditiveRisk {
  var?: number;
  es?: number;
  varAlpha?: number;
  curvatureSpot?: number;
}

/**
 * One rolled-up node of the risk tree (`celnet.wire.RiskNode`): the aggregate at a
 * `(dimension, group)` cell, with its additive + non-additive measures and the
 * count of contributing positions.
 */
export interface RiskNode {
  dimension: RiskDimension;
  group: bigint;
  additive: AdditiveRisk;
  nonadditive: NonAdditiveRisk;
  positionCount: number;
}

/** Canonical vanilla pricing inputs (`celnet.wire.VanillaInputs`), the
 * convention-free leaf the server re-derives a position's risk from. */
export interface VanillaInputs {
  spot: number;
  strike: number;
  vol: number;
  t: number;
  rDom: number;
  rFor: number;
}

/**
 * An open position the cube aggregates (`celnet.wire.RiskPosition`): a risk fact +
 * the originating leaf. `notionalBase` is signed (+ = long). The canonical
 * convention-free leaf is re-derived server-side, never sent by a client.
 */
export interface RiskPosition {
  positionId: bigint;
  org: OrgKey;
  optionType: OptionType;
  notionalBase: number;
  inputs: VanillaInputs;
  quotedDelta: DeltaConvention;
  premiumStyle: PremiumStyle;
  surfaceVersion: bigint;
  attribution?: AttributionRecord;
}

/** The limit metric a utilization row measures (`celnet.wire.LimitMetricKind`),
 * mirrors `celnet_limits`. */
export type LimitMetricKind =
  | "DELTA" // =0
  | "GAMMA"
  | "VEGA"
  | "VANNA"
  | "VOLGA"
  | "VEGA_BUCKET" // payload: vegaPillar
  | "TENOR_VEGA" // payload: tenorDays
  | "CONCENTRATION_DELTA"
  | "CONCENTRATION_VEGA"
  | "VAR"
  | "EXPECTED_SHORTFALL"
  | "STOP_LOSS";

/** Limit RAG status (`celnet.wire.RagStatus`), ordered by severity. */
export type RagStatus = "GREEN" | "AMBER" | "RED" | "BREACH";

/** Limit enforcement posture (`celnet.wire.Enforcement`). */
export type Enforcement = "SOFT" | "HARD";

/**
 * One limit utilization row (`celnet.wire.LimitUtilization`): the metric, its cap
 * and current exposure (reporting numeraire), the utilization ratio, the RAG
 * status, the enforcement posture, and the remaining headroom. `vegaPillar` is set
 * for VEGA_BUCKET; `tenorDays` for TENOR_VEGA.
 */
export interface LimitUtilization {
  metric: LimitMetricKind;
  vegaPillar: RiskVegaPillar;
  tenorDays: number;
  cap: number;
  exposure: number;
  ratio: number;
  status: RagStatus;
  enforcement: Enforcement;
  headroom: number;
}

// --- RiskService requests / responses ---------------------------------------

/** `RiskService.ListPositions` request — the entitled open book, optionally scoped. */
export interface ListPositionsRequest {
  scope?: RiskScope;
  principal?: EntitlementPrincipal;
  correlationId?: bigint;
}

export interface ListPositionsResponse {
  positions: RiskPosition[];
  correlationId?: bigint;
}

/**
 * `RiskService.AggregateRisk` request — prune by principal BEFORE roll-up, group
 * by `dimension`, sum the additive measures + re-derive the non-additive ones per
 * node, and collapse everything into the reporting `numeraire`. `vegaPillars`
 * empty ⇒ server default grid; `varSpotShocks` empty ⇒ no VaR/ES; `varAlpha` 0 ⇒
 * 0.99 when shocks present; `curvatureRiskWeight` 0 ⇒ no curvature.
 */
export interface AggregateRiskRequest {
  dimension: RiskDimension;
  numeraire: ReportingNumeraire;
  principal?: EntitlementPrincipal;
  scope?: RiskScope;
  vegaPillars: RiskVegaPillar[];
  varSpotShocks: number[];
  varAlpha: number;
  curvatureRiskWeight: number;
  correlationId?: bigint;
}

export interface AggregateRiskResponse {
  dimension: RiskDimension;
  numeraire: string;
  nodes: RiskNode[];
  correlationId?: bigint;
}

/**
 * `RiskService.DrillRisk` request — drill one node into child sub-nodes at a finer
 * dimension and/or its contributing positions (the Book→Risk drill),
 * entitlement-pruned.
 */
export interface DrillRiskRequest {
  node: RiskScope;
  childDimension: RiskDimension;
  numeraire: ReportingNumeraire;
  principal?: EntitlementPrincipal;
  vegaPillars: RiskVegaPillar[];
  includeChildren: boolean;
  includePositions: boolean;
  correlationId?: bigint;
}

export interface DrillRiskResponse {
  node: RiskScope;
  children: RiskNode[];
  positions: RiskPosition[];
  correlationId?: bigint;
}

/**
 * `RiskService.LimitStatus` request — the limit tree + per-limit utilization/RAG
 * for a scope node, with the `hardBreach` escalation flag.
 */
export interface LimitStatusRequest {
  scope: RiskScope;
  numeraire: ReportingNumeraire;
  principal?: EntitlementPrincipal;
  vegaPillars: RiskVegaPillar[];
  varSpotShocks: number[];
  varAlpha: number;
  correlationId?: bigint;
}

export interface LimitStatusResponse {
  scope: RiskScope;
  limits: LimitUtilization[];
  worst: RagStatus;
  hardBreach: boolean;
  correlationId?: bigint;
}
