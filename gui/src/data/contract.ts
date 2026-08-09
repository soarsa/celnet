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
  "DOMESTIC_PIPS" | "PERCENT_FOREIGN" | "PERCENT_DOMESTIC" | "FOREIGN_PIPS";

export type Cut = "NEW_YORK_1000" | "TOKYO_1500";

export type DayCount = "ACT_365_FIXED" | "ACT_360";

export type Settlement = "DELIVERABLE" | "NON_DELIVERABLE";

/**
 * A precious metal traded as the asset (base) leg of a metal pair
 * (`celnet.wire.Metal`, mirrors `celnet_types::Metal`). The four LBMA/LPPM
 * precious metals; each projects to an ISO-4217 "X"-prefixed asset code
 * (XAU/XAG/XPT/XPD) so a metal pair overlaps the FX `CcyPair` encoding
 * byte-for-byte on the metal leg. Listed in proto enum-number order (GOLD=0, …).
 */
export type Metal = "GOLD" | "SILVER" | "PLATINUM" | "PALLADIUM";

/**
 * Contract settlement mechanics — how a contract's PnL is denominated and
 * margined (`celnet.wire.SettlementStyle`, carried on `Instrument.settlement_style`,
 * field 29). LINEAR is the proto3 zero default (the ordinary quote-currency-margined
 * contract, byte-identical to the contract before this field existed) for EVERY
 * asset class; INVERSE_COIN is the coin-margined digital-asset convention whose
 * payoff is the `1/S_T` (base-coin-denominated) form — meaningful only for a
 * `digitalAsset` underlying.
 */
export type SettlementStyle = "LINEAR" | "INVERSE_COIN";

/**
 * The premium margining convention of an option on a listed future
 * (`celnet.wire.Margining`, carried on `ListedFutureOption.margining`).
 * EQUITY_STYLE is the proto3 zero default (premium paid upfront ⇒ the priced
 * value is discounted, the ordinary contract); FUTURES_STYLE margins the premium
 * daily like the future itself ⇒ the priced value is undiscounted.
 */
export type Margining = "EQUITY_STYLE" | "FUTURES_STYLE";

/**
 * The published settlement-rate option a non-deliverable forward fixes against
 * (`celnet.wire.FixingSource`). This names *which* published rate the contract
 * settles to (each EMTA / ISDA per-currency template names exactly one) — it is
 * convention IDENTITY, NOT a market-data input: the live fixing VALUE is an
 * estate-gated feed, never sourced in-repo, so only the identity is on the wire.
 * Purpose-named, vendor/method-neutral (CLAUDE.md rule 8).
 */
export type FixingSource =
  | "KRW_KFTC18"
  | "TWD_TAIPEI"
  | "INR_RBI_REF"
  | "BRL_PTAX"
  | "CLP_DOLAR_OBS"
  | "COP_TRM";

export type StrategyKind =
  "RISK_REVERSAL" | "STRANGLE" | "STRADDLE" | "SEAGULL";

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
  "ONE_TOUCH" | "NO_TOUCH" | "DOUBLE_NO_TOUCH" | "DOUBLE_ONE_TOUCH";

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
  "ATM_VOL" | "SPOT" | "RISK_REVERSAL" | "BUTTERFLY" | "FORWARD";

// --- value messages ---------------------------------------------------------

/** An FX currency pair BASE/QUOTE (market form CCY1CCY2), e.g. EUR/USD. */
export interface CcyPair {
  /** Foreign / asset currency (CCY1), e.g. "EUR". */
  base: string;
  /** Domestic / numeraire currency (CCY2), e.g. "USD". */
  quote: string;
}

/**
 * A precious-metal pair (`celnet.wire.MetalPair`): the `metal` is the base/asset
 * leg, `quote` the fiat numeraire (3-letter code). Projects byte-identically onto
 * a metal-base `CcyPair` (base = the metal's ISO-4217 "X"-prefixed asset code).
 */
export interface MetalPair {
  /** The precious metal (the base/asset leg). */
  metal: Metal;
  /** The fiat quote (numeraire) currency, e.g. "USD", "EUR", "JPY". */
  quote: string;
}

/**
 * A free-form instrument ticker (`celnet.wire.Symbol`) — the vendor-neutral
 * identifier for an asset not named by a currency-/metal-pair leg structure (an
 * equity, a commodity, a digital-asset coin).
 */
export interface Symbol {
  /** The trading symbol / ticker, e.g. "AAPL", "BRENT", an ISIN. UTF-8. */
  ticker: string;
  /** The listing venue / exchange MIC, e.g. "XNAS"; empty when unambiguous. */
  venue: string;
}

/**
 * An equity (single-name or index) underlying (`celnet.wire.EquityRef`), priced
 * through the generalized cost-of-carry seam (dividend yield as carry `b`). The
 * dividend treatment is a carry-layer concern, not encoded here.
 */
export interface EquityRef {
  /** The listed equity symbol (single name or index). */
  symbol: Symbol;
  /** The currency the equity is quoted / settled in (a 3-letter code). */
  currency: string;
}

/**
 * A commodity underlying (`celnet.wire.CommodityRef`), priced through the
 * generalized cost-of-carry seam (storage/convenience yield as carry `b`). The
 * cost-of-carry is a carry-layer concern, not encoded here.
 */
export interface CommodityRef {
  /** The commodity / contract symbol, e.g. "BRENT". */
  symbol: Symbol;
  /** The currency the commodity is quoted / settled in (a 3-letter code). */
  currency: string;
}

/**
 * A digital-asset (crypto) pair (`celnet.wire.CryptoPair`). The `base` is the
 * coin/asset leg (e.g. "BTC", "ETH"); the `quote` is the numeraire — a fiat
 * ("USD") or coin/stablecoin ("USDT"). Both legs are UTF-8 strings (crypto tickers
 * are not constrained to the 3-letter ISO-4217 shape). The linear/inverse
 * settlement of a coin-margined contract is carried on `Instrument.settlementStyle`,
 * NOT here (a contract-mechanics convention, not part of the pair identity).
 */
export interface CryptoPair {
  /** The coin/asset (base) leg, e.g. "BTC", "ETH". UTF-8. */
  base: string;
  /** The numeraire (quote) leg — fiat ("USD") or coin/stablecoin ("USDT"). */
  quote: string;
}

/**
 * The instrument's underlying — the asset-class discriminator (`celnet.wire
 * .Underlying`). Exactly one `ref` arm is set. FX is the first-class arm (also
 * carried directly as `Instrument.pair` for the FX surfaces' byte-identical
 * projection); metal/equity/commodity/digitalAsset are the cross-asset extensions
 * (proto `oneof ref` field numbers fx=1, metal=3, equity=4, commodity=5,
 * digital_asset=6). `settlementCcy` is the settlement / numeraire currency code
 * (for FX, the pair's quote currency).
 */
export type Underlying =
  | { kind: "fx"; fx: CcyPair; settlementCcy: string }
  | { kind: "metal"; metal: MetalPair; settlementCcy: string }
  | { kind: "equity"; equity: EquityRef; settlementCcy: string }
  | { kind: "commodity"; commodity: CommodityRef; settlementCcy: string }
  | { kind: "digitalAsset"; digitalAsset: CryptoPair; settlementCcy: string };

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
  { kind: "strike"; strike: number } | { kind: "delta"; delta: number };

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
 * A pivot Target-Redemption Accumulator (`celnet.wire.Pivot`, arm 32): the TARF
 * mechanic with a distinct `pivot` kink — each fixing's leg is SELECTED by the
 * pivot and VALUED by the strike intrinsic, the favourable leg accruing toward
 * `target` (knock-out on target, the shared `TarfRedemption` gap-risk
 * convention), the adverse leg geared by `leverage`. `pivot === strike` is the
 * exact plain-TARF slice. Always priced by Monte-Carlo (the value is the BANK's
 * present value), so the reply always carries a standard error
 * (`Quote.priceStdError`).
 */
export interface Pivot {
  /** The favourable direction (PUT = the exporter orientation, gains below `strike`). */
  optionType: OptionType;
  /** The target strike `K` intrinsic is measured against. */
  strike: number;
  /** The pivot `P` at which the geared adverse leg engages (`P === K` ⇒ TARF). */
  pivot: number;
  /** The cumulative gain target; accumulated gain at or above it redeems. */
  target: number;
  /** The gearing/leverage multiplier on the adverse leg (`≥ 0`). */
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

/**
 * An outright forward (`celnet.wire.FxForward`, product field 26) — the
 * `celnet-linear` forward leaf. A linear, closed-form discounted-cashflow product
 * (NOT an option payoff): its PV is `side · notional · discount_df(t) ·
 * (forward_rate − contractRate)` where `forward_rate = spot · forward_factor(t)`
 * comes from the instrument's carry. Asset-class-agnostic (an FX or metal forward
 * uses the identical engine) and EXACT — the reply never carries a
 * `Quote.priceStdError`. Valid for a DELIVERABLE underlying only.
 */
export interface FxForward {
  /** The agreed contract (delivery) rate `K` in quote per 1 unit of base/asset. */
  contractRate: number;
  /** The notional amount (always positive; direction is carried by `side`). */
  notional: number;
  /** The direction taken (`BUY` = long the base/asset forward; `SELL` = short). */
  side: Side;
}

/**
 * An FX swap (`celnet.wire.FxSwap`, product field 27) — a near leg and a far leg,
 * each an outright {@link FxForward}. By market convention the two legs trade in
 * OPPOSITE directions (`far.side` is the opposite of `near.side`); the swap PV is
 * the independent sum of the two leg PVs. The near leg settles at the spot date
 * (`t = 0`) and the far leg at the instrument's expiry/tenor (the single ticket
 * tenor anchors the far leg). Deliverable underlying only.
 */
export interface FxSwap {
  /** The near (spot-dated) leg. */
  near: FxForward;
  /** The far (tenor-dated) leg; by convention the opposite side to `near`. */
  far: FxForward;
}

/**
 * A non-deliverable forward (`celnet.wire.Ndf`, product field 28) — the
 * `celnet-linear` NDF leaf. The risk-neutral PV is identical to a deliverable
 * forward of equal terms (`side · notional · discount_df(t) · (forward_rate −
 * contractRate)`, discounted at the convertible/settlement-ccy rate);
 * non-deliverability changes only the settlement mechanics, not the PV. Valid
 * ONLY for a NON-DELIVERABLE underlying. `fixing` names the published
 * settlement-rate option — identity only; the live fixing VALUE is an estate-gated
 * feed, never sourced in-repo.
 */
export interface Ndf {
  /** The agreed contract (forward) rate `K` in settlement-ccy per 1 unit of base. */
  contractRate: number;
  /** The notional amount (always positive; direction is carried by `side`). */
  notional: number;
  /** The direction taken (`BUY` = long the base/asset forward; `SELL` = short). */
  side: Side;
  /** The published settlement-rate option fixed against (identity only — no value). */
  fixing: FixingSource;
  /** The convertible (settlement) currency the net cash settlement is paid in. */
  settlementCcy: string;
}

/**
 * A perpetual (no-expiry) American option (`celnet.wire.PerpetualOption`, product
 * field 30): the holder may exercise at any time, with no terminal date —
 * exercise is the only way the contract ends, so the product is American by
 * construction and carries no exercise-style field. Because there is no expiry
 * to encode, the enclosing `Instrument.expiryYears` MUST be 0 exactly for this
 * arm (and the instrument carries NO `tenor` — no tenor label exists); a
 * non-zero expiry on a perpetual is rejected by the server's term validator as
 * INVALID_ARGUMENT — never silently ignored. Priced in exact closed form (the
 * value is time-homogeneous), so the reply never carries a `Quote.priceStdError`.
 */
export interface PerpetualOption {
  optionType: OptionType;
  /** Strike `K` (absolute level, quote per 1 unit of base/asset). */
  strike: number;
  /** The notional amount (always positive; direction is carried by the instrument's `side`). */
  notional: number;
}

/**
 * An option on a listed future (`celnet.wire.ListedFutureOption`, product field
 * 31), for any asset class: the enclosing `Instrument.underlying` names the
 * class and `futureSymbol` names the specific listed contract the option
 * exercises into. The future must outlive the option —
 * `futureExpiryYears >= Instrument.expiryYears > 0` is validity-checked
 * server-side, INVALID_ARGUMENT otherwise. The quoted futures price already
 * embodies the underlying's carry, so every asset class prices by the same
 * futures-measure closed form; `margining` decides whether the premium is paid
 * upfront (EQUITY_STYLE, discounted) or margined daily (FUTURES_STYLE,
 * undiscounted).
 */
export interface ListedFutureOption {
  /** The listed future contract the option exercises into (ticker + venue MIC). */
  futureSymbol: Symbol;
  /** The FUTURE's own expiry (years); `>= expiryYears > 0` — the future outlives the option. */
  futureExpiryYears: number;
  /** Call or put on the future. */
  optionType: OptionType;
  /** Strike `K` (absolute level, in the future's quote units). */
  strike: number;
  /** The notional amount (always positive; direction is carried by the instrument's `side`). */
  notional: number;
  /** The premium margining convention (EQUITY_STYLE is the meaningful proto3 zero). */
  margining: Margining;
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
  | { kind: "pivot"; pivot: Pivot }
  | { kind: "accumulator"; accumulator: Accumulator }
  | { kind: "lookback"; lookback: Lookback }
  | { kind: "windowBarrier"; windowBarrier: WindowBarrier }
  | { kind: "american"; american: AmericanOption }
  | { kind: "basket"; basket: BasketOption }
  | { kind: "fxForward"; fxForward: FxForward }
  | { kind: "fxSwap"; fxSwap: FxSwap }
  | { kind: "ndf"; ndf: Ndf }
  | { kind: "perpetualOption"; perpetualOption: PerpetualOption }
  | { kind: "listedFutureOption"; listedFutureOption: ListedFutureOption };

/** Solve directive: solve a free parameter to hit a target (e.g. zero premium). */
export interface Solve {
  target: "NONE" | "STRIKE" | "PREMIUM";
  targetPremium: number;
}

/** The unified instrument every Celnet workflow speaks. */
export interface Instrument {
  pair: CcyPair;
  /**
   * The cross-asset underlying (`Instrument.underlying`, proto field 1 — an
   * `Underlying` oneof). Presence-tracked: absent ⇒ the FX projection carried by
   * `pair` (the FX surfaces key on `pair`). A non-FX instrument sets this to a
   * metal / equity / commodity / digital-asset arm; `pair` then carries the
   * underlying's leg-string projection so the FX-keyed surfaces stay total.
   */
  underlying?: Underlying;
  /**
   * The trader-facing tenor label (presence-tracked, mirroring the SDK's
   * optional tenor): absent ONLY for the one tenorless product — the perpetual
   * option, which has no expiry date to label (its canonical wire shape is
   * `expiryYears = 0` with no `tenor` key). Every dated product carries it, so
   * the wire encoding of the dated families is unchanged.
   */
  tenor?: Tenor;
  /** Expiry year fraction on the surface day-count (authoritative for pricing). */
  expiryYears: number;
  quantity: Quantity;
  side: Side;
  solve?: Solve;
  /**
   * The contract settlement mechanics (`Instrument.settlement_style`, proto field
   * 29). Presence-tracked: absent/`LINEAR` ⇒ the ordinary quote-currency-margined
   * linear contract (the proto3 zero value, byte-identical to the contract before
   * this field existed); `INVERSE_COIN` selects the coin-margined `1/S_T`
   * digital-asset convention. Travels uniformly on the Instrument exactly like
   * `pricingModel`.
   */
  settlementStyle?: SettlementStyle;
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

/**
 * One liquidity provider's line of a multi-dealer panel (`celnet.wire
 * .DealerQuote`). Every line quotes the SAME resolved instrument (the
 * maker-priced strike); `greeks`/`priceStdError` are present on the NATIVE maker
 * line only — an LP discloses a price, not its greeks — so their absence on a
 * dealer row is honest, never a row of zeros.
 */
export interface DealerQuote {
  /** The dealer's stable audit identity; echoed on `acceptQuote` to book this line. */
  lpId: string;
  price: TwoWayPrice;
  /** The edge-priced Greeks — the native maker line only. */
  greeks?: Greeks;
  resolvedStrike: number;
  /** This line's own last-look deadline, nanoseconds since the Unix epoch (UTC). */
  validUntilNanos: bigint;
  /** The LP seat that quoted this line (+ the requesting holder), if emitted. */
  attribution?: AttributionRecord;
  /** MC standard error — the native maker line of an MC-priced product only. */
  priceStdError?: number;
}

/**
 * The ranked multi-dealer (RFQ-to-many) panel (`celnet.wire.MultiDealerQuote`):
 * one row per responding LP plus the touch winners. `dealers` arrives in the
 * server's deterministic audit order and is rendered AS-IS (frame order ==
 * render order); the ranking is surfaced by `bestBidLpId`/`bestOfferLpId`
 * (empty ⇒ no live line won that side). An `acceptQuote` carrying a row's
 * `lpId` books exactly that pinned dealer line.
 */
export interface MultiDealerQuote {
  quoteId: bigint;
  idempotencyKey: string;
  /** Responding dealer lines, in the server's deterministic audit order. */
  dealers: DealerQuote[];
  /** The `lpId` whose bid won the panel (highest bid; empty ⇒ none live). */
  bestBidLpId: string;
  /** The `lpId` whose offer won the panel (lowest offer; empty ⇒ none live). */
  bestOfferLpId: string;
  conventions: Conventions;
  epochNanos: bigint;
  correlationId?: bigint;
  surfaceVersion?: bigint;
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
  "LAGGED" | "DRAINING" | "UNSUBSCRIBED" | "EXPIRED";

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

export type StreamRejectReason =
  "EXPIRED" | "UNKNOWN_TOKEN" | "ALREADY_CONSUMED";

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
 * `celnet_entitlements`). DEFAULT = grant-all: a request that carries no principal
 * is encoded with an **explicit** grant-all (the GUI's show-all-now posture — see
 * `principalOrGrantAllToWire`), so the headline view clears the server's production
 * deny-by-default boundary (`AccessMode::Enforce`), which denies a *genuinely*
 * absent principal. A present principal with `grantAll=false` and no grants is
 * deny-by-default; a `deny` rule applies to any principal (deny wins). A deployment
 * gateway injects/validates the real principal in production.
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

// ---------------------------------------------------------------------------
// fix-admin — manage the inbound FIX acceptor connections the edge binds
// ---------------------------------------------------------------------------

/**
 * The dialect an inbound FIX acceptor speaks (`celnet.wire.FixAcceptorKind`).
 * `"OPTIONS"` is the FX-options dialect; `"FIXED_INCOME_QUOTE"` is the rates/OIS
 * one-shot RFQ dialect and `"FIXED_INCOME_STREAM"` the rates/OIS streaming RFS
 * dialect — standing either FI venue up requires the matching FI capability
 * (`quote_respond` / `stream` on `fixed_income`). Kind-generic: the spot dialect
 * adds a member in a later phase without reshaping the contract or the UI.
 */
export type FixConnectionKind =
  "OPTIONS" | "FIXED_INCOME_QUOTE" | "FIXED_INCOME_STREAM";

/**
 * A managed inbound FIX-acceptor connection: the persisted definition plus its
 * live runtime status (`running`/`boundAddr` are server-owned, read-only).
 * Mirrors `celnet.wire.FixConnectionDesc`.
 */
export interface FixConnection {
  /** Stable identifier (the management key). */
  id: string;
  /** Human-friendly label shown in the UI. */
  name: string;
  /** Which dialect the acceptor speaks. */
  kind: FixConnectionKind;
  /** The `host:port` the acceptor binds, e.g. `127.0.0.1:9099`. */
  bindAddr: string;
  /** Our venue `SenderCompID`. */
  senderCompId: string;
  /** The expected counterparty `SenderCompID` (the FSM rejects any other peer). */
  targetCompId: string;
  /** Whether the acceptor should be (and stay) bound. */
  enabled: boolean;
  /** Whether an acceptor is currently listening (runtime status). */
  running: boolean;
  /** The actually-bound address when running (resolves an ephemeral `:0`); else "". */
  boundAddr: string;
  /**
   * The routing desk id (`DeskDesc.id`), or `""` when the connection is
   * intentionally unrouted. The routing desk determines which desk's users
   * receive this venue's RFQs/deals; a desk-scoped (non-admin) session sees only
   * the connections whose `desk` matches its own, while an admin sees all of them.
   * An unrouted (blank) connection accepts the session but delivers to no desk.
   */
  desk: string;
}

/**
 * The editable fields of a connection (the create/update payload). Runtime
 * status (`running`/`boundAddr`) is server-owned and not part of the spec.
 * Mirrors `celnet.wire.FixConnectionSpec`.
 */
export interface FixConnectionSpec {
  /** Optional client-suggested id on create (a slug of `name` is minted when absent). */
  id?: string;
  name: string;
  kind: FixConnectionKind;
  bindAddr: string;
  senderCompId: string;
  targetCompId: string;
  enabled: boolean;
  /**
   * The routing desk id (`DeskDesc.id`), OPTIONAL. When set (non-blank) it must
   * name a defined desk — the server validates this on create/update and rejects
   * an unknown id — and it determines which desk's users receive the venue's
   * RFQs/deals. Absent/blank ⇒ the wire codec sends `""` and the connection is
   * accepted as intentionally unrouted (delivers to no desk).
   */
  desk?: string;
}

/**
 * The travel direction of a captured FIX frame, from the acceptor's vantage
 * (`celnet.wire.FixMsgDirection`). Mirrors the wire enum (INBOUND=0/OUTBOUND=1).
 */
export type FixMsgDirection = "INBOUND" | "OUTBOUND";

/**
 * One captured FIX frame on a managed acceptor session — the monitor screen feed
 * (`celnet.wire.FixMessage`). `seq` and `epochNanos` are 64-bit, carried as bigint.
 */
export interface FixMessage {
  /** Monotonic per-process capture sequence (the poll cursor). */
  seq: bigint;
  /** The managing connection's id this frame was captured on. */
  connectionId: string;
  /** Travel direction from the acceptor's vantage. */
  direction: FixMsgDirection;
  /** The FIX MsgType(35) value, e.g. `R`, `S`, `A`. */
  msgType: string;
  /** A human label for the MsgType, e.g. `QuoteRequest`. */
  summary: string;
  /** Edge capture timestamp (epoch nanos). */
  epochNanos: bigint;
  /** The raw FIX message with SOH rendered as `|`. */
  raw: string;
}

/** A cursored page of captured session traffic (`FixAdminService.ListMessages`). */
export interface FixMessagePage {
  /** The captured frames after the requested cursor (oldest-first). */
  messages: FixMessage[];
  /** The highest capture sequence assigned — the next poll's cursor. */
  latestSeq: bigint;
}

// --- AuthService — server-enforced sessions + user/desk administration -------
//
// Login mints a bearer token the server validates on every gated RPC; admin RPCs
// additionally require the resolved identity to be an admin. Mirrors the
// `celnet.wire` Auth messages (`UserDesc`, `DeskDesc`, `LoginResponse`). Passwords
// NEVER appear on a descriptor — they ride only on login / create / reset inputs.

/**
 * A user's authority level (`celnet.wire.UserRole`). `TRADER` (the wire zero
 * default) sees their desk's inbound RFQ traffic; `ADMIN` has full user/desk/FIX
 * administration. A defaulted/forgotten value can never grant administration.
 */
export type UserRole = "TRADER" | "ADMIN";

/**
 * A user account as exposed on the wire (`celnet.wire.UserDesc`) — carries NO
 * password material.
 */
export interface UserDesc {
  /** Stable user id (the admin-API key). */
  id: string;
  /** Login email (unique, case-insensitive). */
  email: string;
  /** Human-friendly display name. */
  displayName: string;
  /** The user's authority level. */
  role: UserRole;
  /**
   * The desks the user belongs to, by `DeskDesc.id`. A user may belong to zero,
   * one, or many desks; a quote/deal routed to ANY desk in this set reaches the
   * user. ALWAYS an array (`[]` when none) — never absent. Empty ⇒ deskless (sees
   * no desk-routed traffic) unless `allDesks` is set.
   */
  deskIds: string[];
  /** When true the user belongs to EVERY desk; `deskIds` is then empty/ignored. */
  allDesks: boolean;
  /** Whether the account is disabled (retained but cannot log in). */
  disabled: boolean;
}

/** A desk: a named group traders belong to (`celnet.wire.DeskDesc`). */
export interface DeskDesc {
  /** Stable desk id (the admin-API key). */
  id: string;
  /** Human-friendly desk label. */
  name: string;
}

// --- legal-entity / netting-book registry (`AuthService` entity/book admin) ---
//
// The admin-managed registry that names the `(entity, book)` `uint32` partition
// keys a `RatesPosition` books into (`celnet.wire.EntityDesc`/`BookDesc`). A
// position still carries opaque `uint32` keys on the wire; this registry is the
// display-name ↔ key map the booking form resolves a named selection through and
// the Book/blotter views resolve a key back to a name with. Listing is open to
// any authenticated user (it populates the booking form); create/update/delete
// are admin-only (server-enforced). The wire JSON carries snake_case
// `entity_key`; the codec layer maps it to this camelCase `entityKey`.

/** A named legal entity / account a position books into (`celnet.wire.EntityDesc`). */
export interface EntityDesc {
  /** The `uint32` partition key carried on `RatesPosition.entity` (immutable identity). */
  key: number;
  /** Human-friendly legal-entity name, e.g. "Celnet Global Markets". */
  name: string;
  /** Short code, e.g. "CGM" (unique). */
  code: string;
}

/** A named netting book under an entity (`celnet.wire.BookDesc`). */
export interface BookDesc {
  /** The `uint32` partition key carried on `RatesPosition.book` (immutable identity). */
  key: number;
  /** Human-friendly book name, e.g. "Rates Trading". */
  name: string;
  /** The owning entity's `EntityDesc.key`. */
  entityKey: number;
}

/** The create/update-an-entity payload (`AuthService.{Create,Update}Entity`). */
export interface EntityInput {
  name: string;
  code: string;
}

/** The create/update-a-book payload (`AuthService.{Create,Update}Book`). */
export interface BookInput {
  name: string;
  /** The owning entity's `EntityDesc.key`. */
  entityKey: number;
}

// --- instrument reference-data registry (`AuthService` instrument admin) ------
//
// The admin-managed registry of instrument DEFINITIONS (`celnet.wire`
// `InstrumentDefDesc`) — the canonical static terms of a tradable rates/credit
// instrument (its conventions, schedule terms, identifiers). Listing
// (`list_instruments`/`get_instrument`) is open to any authenticated user (it
// populates pickers and reference views); create/update/delete are admin-only
// (server-enforced). A definition carries exactly ONE family sub-object keyed by
// its family token; the GUI keeps that as a `family` discriminant plus a single
// per-family field bag (camelCase). The codec layer maps every field to the
// snake_case wire form (`day_count`, `external_ids`, the family key itself).

/** A vendor/identifier scheme an instrument may be tagged with (wire labels). */
export type ExternalIdScheme =
  "isin" | "cusip" | "sedol" | "figi" | "ticker" | "internal";

/** The full set of {@link ExternalIdScheme} labels, in canonical order. */
export const EXTERNAL_ID_SCHEMES: readonly ExternalIdScheme[] = [
  "isin",
  "cusip",
  "sedol",
  "figi",
  "ticker",
  "internal",
];

/** One external identifier on an instrument (`celnet.wire.ExternalId`). */
export interface ExternalIdEntry {
  scheme: ExternalIdScheme;
  value: string;
}

/**
 * A day-count basis. `act_act` is bond-only (it is rejected by the server on the
 * money-market / swap families); the others are valid everywhere.
 */
export type RatesDayCount =
  "act_360" | "act_365_fixed" | "thirty_360_bond_basis" | "act_act";

/** Day-count labels valid on the money-market / swap families (no `act_act`). */
export const RATES_DAY_COUNTS: readonly RatesDayCount[] = [
  "act_360",
  "act_365_fixed",
  "thirty_360_bond_basis",
];

/** Day-count labels valid on the bond family (adds `act_act`). */
export const BOND_DAY_COUNTS: readonly RatesDayCount[] = [
  "act_360",
  "act_365_fixed",
  "thirty_360_bond_basis",
  "act_act",
];

/** A schedule date-roll/business-day adjustment convention (wire labels). */
export type BusinessDayConvention =
  "unadjusted" | "following" | "preceding" | "modified_following";

/** The full set of {@link BusinessDayConvention} labels, in canonical order. */
export const BUSINESS_DAY_CONVENTIONS: readonly BusinessDayConvention[] = [
  "unadjusted",
  "following",
  "preceding",
  "modified_following",
];

/** A holiday calendar a schedule is adjusted against (wire labels). */
export type Calendar =
  | "united_states"
  | "target2"
  | "united_kingdom"
  | "japan"
  | "switzerland"
  | "australia"
  | "canada"
  | "new_zealand"
  | "mexico"
  | "south_africa"
  | "norway"
  | "sweden";

/** The full set of {@link Calendar} labels, in canonical order. */
export const CALENDARS: readonly Calendar[] = [
  "united_states",
  "target2",
  "united_kingdom",
  "japan",
  "switzerland",
  "australia",
  "canada",
  "new_zealand",
  "mexico",
  "south_africa",
  "norway",
  "sweden",
];

/** A coupon/leg payment frequency (wire labels). */
export type Frequency = "annual" | "semi_annual" | "quarterly";

/** The full set of {@link Frequency} labels, in canonical order. */
export const FREQUENCIES: readonly Frequency[] = [
  "annual",
  "semi_annual",
  "quarterly",
];

/** A schedule roll convention (wire labels). */
export type RollConvention = "none" | "eom" | "imm";

/** The full set of {@link RollConvention} labels, in canonical order. */
export const ROLL_CONVENTIONS: readonly RollConvention[] = [
  "none",
  "eom",
  "imm",
];

/** A bond coupon type (wire labels). `zero` ⇒ no coupon schedule. */
export type CouponType = "fixed" | "frn" | "zero";

/** The full set of {@link CouponType} labels, in canonical order. */
export const COUPON_TYPES: readonly CouponType[] = ["fixed", "frn", "zero"];

/** The family discriminant of an {@link InstrumentDef} (the wire family token). */
export type InstrumentFamily =
  "deposit" | "fra" | "stir_future" | "vanilla_irs" | "ois" | "bond";

/** The full set of {@link InstrumentFamily} tokens, in canonical order. */
export const INSTRUMENT_FAMILIES: readonly InstrumentFamily[] = [
  "deposit",
  "fra",
  "stir_future",
  "vanilla_irs",
  "ois",
  "bond",
];

/** Human-friendly labels for each family (UI display only; never on the wire). */
export const INSTRUMENT_FAMILY_LABELS: Readonly<
  Record<InstrumentFamily, string>
> = {
  deposit: "Deposit",
  fra: "FRA",
  stir_future: "STIR future",
  vanilla_irs: "Vanilla IRS",
  ois: "OIS",
  bond: "Bond",
};

/** A money-market deposit's terms. */
export interface DepositDef {
  index: string;
  tenor: string;
  dayCount: RatesDayCount;
  businessDayConvention: BusinessDayConvention;
  calendars: Calendar[];
  spotLagDays: number;
}

/** A forward-rate-agreement's terms. */
export interface FraDef {
  floatIndex: string;
  startTenor: string;
  endTenor: string;
  accrualDayCount: RatesDayCount;
  businessDayConvention: BusinessDayConvention;
  calendars: Calendar[];
  spotLagDays: number;
}

/** A short-term-interest-rate future's terms. */
export interface StirFutureDef {
  contractCode: string;
  referenceStart: string;
  referenceEnd: string;
  dayCount: RatesDayCount;
  calendars: Calendar[];
  convexityVol: number;
  contractSize: number;
}

/** A vanilla fixed-vs-float interest-rate swap's terms. */
export interface VanillaIrsDef {
  tenor: string;
  fixedFrequency: Frequency;
  fixedDayCount: RatesDayCount;
  floatIndex: string;
  floatFrequency: Frequency;
  floatDayCount: RatesDayCount;
  businessDayConvention: BusinessDayConvention;
  calendars: Calendar[];
  rollConvention: RollConvention;
  spotLagDays: number;
}

/** An overnight-indexed swap's terms. */
export interface OisDef {
  tenor: string;
  index: string;
  fixedFrequency: Frequency;
  fixedDayCount: RatesDayCount;
  floatDayCount: RatesDayCount;
  businessDayConvention: BusinessDayConvention;
  calendars: Calendar[];
  spotLagDays: number;
}

/**
 * A bond's terms. For a zero-coupon bond (`couponType === "zero"`) the
 * `couponFrequency` MUST be blank and `couponRate` is `0`. `maturityDate` is
 * required; the other dates are optional (omitted on the wire when absent).
 */
export interface BondDef {
  issuer: string;
  couponRate: number;
  couponType: CouponType;
  /** Blank for a zero-coupon bond; a {@link Frequency} otherwise. */
  couponFrequency: Frequency | "";
  dayCount: RatesDayCount;
  issueDate?: BrokenDate;
  datedDate?: BrokenDate;
  firstCouponDate?: BrokenDate;
  maturityDate: BrokenDate;
  redemption: number;
  calendars: Calendar[];
}

/** The fields every instrument definition carries, regardless of family. */
interface InstrumentDefBase {
  /** Stable id; blank on create ⇒ the server mints one from the name. */
  instrumentId: string;
  name: string;
  description: string;
  /** ISO 4217 currency, e.g. "USD". */
  currency: string;
  externalIds: ExternalIdEntry[];
}

/**
 * An instrument definition (`celnet.wire.InstrumentDefDesc`): the base fields plus
 * exactly one family sub-object, modelled as a discriminated union on `family`.
 * The discriminant value is the wire family token; the matching field bag is held
 * under a camelCase key (`stirFuture`, `vanillaIrs`) the codec maps to/from the
 * snake_case wire key.
 */
export type InstrumentDef =
  | (InstrumentDefBase & { family: "deposit"; deposit: DepositDef })
  | (InstrumentDefBase & { family: "fra"; fra: FraDef })
  | (InstrumentDefBase & { family: "stir_future"; stirFuture: StirFutureDef })
  | (InstrumentDefBase & { family: "vanilla_irs"; vanillaIrs: VanillaIrsDef })
  | (InstrumentDefBase & { family: "ois"; ois: OisDef })
  | (InstrumentDefBase & { family: "bond"; bond: BondDef });

/**
 * The create/update payload (`AuthService.{Create,Update}Instrument`). It is the
 * full definition; on create the `instrumentId` is blank (server-minted), on
 * update it identifies the record being replaced.
 */
export type InstrumentInput = InstrumentDef;

/** The issued session on a successful login (`celnet.wire.LoginResponse`). */
export interface LoginResult {
  /** The opaque bearer token to present on subsequent RPCs (a secret). */
  token: string;
  /** The authenticated user's profile. */
  user: UserDesc;
  /** Absolute session expiry (epoch nanos); re-login is required past it. */
  expiresNanos: bigint;
  /**
   * The caller's OWN fully-resolved effective capability set (`role bundle ∪
   * grants ∖ denies`, deny-wins, enumerated over every action × asset). This is
   * the single source for "what may THIS signed-in user do" and drives the
   * client's affordance gating. It self-refreshes: any capability change to a
   * user revokes their sessions, so their next login re-derives this set.
   */
  capabilities: Capability[];
}

/** The create-a-user payload (`AuthService.CreateUser`). */
export interface CreateUserInput {
  email: string;
  displayName: string;
  role: UserRole;
  /**
   * The desks to assign, by `DeskDesc.id` (zero, one, or many). Ignored when
   * `allDesks` is set. Always an array (`[]` for none/all).
   */
  deskIds: string[];
  /** When true the new user belongs to EVERY desk; `deskIds` is then ignored. */
  allDesks: boolean;
  /** The initial plaintext password (hashed at rest; min length enforced server-side). */
  password: string;
}

/**
 * The update-a-user payload (`AuthService.UpdateUser`). The password is changed
 * only through `resetPassword`, never here.
 */
export interface UpdateUserInput {
  displayName: string;
  role: UserRole;
  /**
   * The new desk membership, by `DeskDesc.id` (zero, one, or many). Ignored when
   * `allDesks` is set. Always an array (`[]` for none/all).
   */
  deskIds: string[];
  /** When true the user belongs to EVERY desk; `deskIds` is then ignored. */
  allDesks: boolean;
  disabled: boolean;
}

// --- per-user capability overlay (`AuthService.{Get,Set}UserCapabilities`) -----
//
// The "who may act" layer (orthogonal to the desk-scope read predicate): one
// capability is one action on one asset class. The server resolves the effective
// set a user holds as `role bundle ∪ grants ∖ denies`, deny-wins (mirrors
// `celnet-entitlements::CapabilitySet`). Labels are the canonical snake_case the
// wire carries; provenance lives in comments only (guardrail #8).

/**
 * One action a capability may authorize (`celnet.wire.CapabilityDesc.action`).
 * The full set, in canonical order, is {@link CAPABILITY_ACTIONS}.
 */
export type CapabilityAction =
  | "view"
  | "price"
  | "quote_respond"
  | "rfq_respond"
  | "ioi_respond"
  | "stream"
  | "execute"
  | "book"
  | "risk_transfer"
  | "simulate"
  | "administer"
  | "risk_manage"
  | "manage_pricing"
  | "manage_liquidity"
  | "view_analytics"
  | "hedge"
  | "refdata"
  | "manage_acceptance";

/** The asset class a capability applies to (`celnet.wire.CapabilityDesc.asset`). */
export type CapabilityAsset = "fx_options" | "fixed_income";

/** One capability: an {@link CapabilityAction} on a {@link CapabilityAsset}. */
export interface Capability {
  action: CapabilityAction;
  asset: CapabilityAsset;
}

/**
 * The full action set in canonical (server discriminant) order — the row axis of
 * the capability matrix and the enumeration domain for resolving `effective`.
 */
export const CAPABILITY_ACTIONS: readonly CapabilityAction[] = [
  "view",
  "price",
  "quote_respond",
  "rfq_respond",
  "ioi_respond",
  "stream",
  "execute",
  "book",
  "risk_transfer",
  "simulate",
  "administer",
  "risk_manage",
  "manage_pricing",
  "manage_liquidity",
  "view_analytics",
  "hedge",
  // Master reference data: confirm / apply bond corporate actions (the CAEV/CAMV
  // lifecycle). Held back from the default trader bundle (an explicitly-granted
  // steward seat) — see `TRADER_HELD_BACK_ACTIONS`.
  "refdata",
  // Author the incoming-quote-acceptance rule graph (the third trader-configurable
  // rule engine — `celnet-acceptance`). The 18th action (server `Action::ALL`
  // discriminant order — LAST, after `refdata`). Held back from the default trader
  // bundle (an explicitly-granted acceptance-policy author seat).
  "manage_acceptance",
];

/** Both asset classes in canonical order — the column axis of the matrix. */
export const CAPABILITY_ASSETS: readonly CapabilityAsset[] = [
  "fx_options",
  "fixed_income",
];

/**
 * A user's capability overlay plus the fully-resolved effective set
 * (`AuthService.{Get,Set}UserCapabilities` response). `grants` widen beyond the
 * user's role bundle, `denies` narrow it (deny-wins over any grant or role
 * default), and `effective` is the server-computed `role bundle ∪ grants ∖
 * denies` enumerated over every action × asset — the read-only source of truth
 * for what the user can actually do. A successful Set replaces the overlay
 * wholesale and revokes the target user's live sessions server-side.
 */
export interface UserCapabilities {
  /** Per-user widenings beyond the role bundle. */
  grants: Capability[];
  /** Per-user narrowings; deny-wins over any grant or role default. */
  denies: Capability[];
  /** The fully-resolved set the server admits. Server-computed, read-only. */
  effective: Capability[];
}

/**
 * A role's capability **bundle** — the base authority the role confers before any
 * per-user overlay (`AuthService.{Get,Set}RoleCapabilities` response). For `ADMIN`
 * this is the full action × asset surface (grant-all, immutable — a Set is
 * rejected server-side); for a non-admin role it is the admin-editable bundle,
 * defaulting to every action but `administer` on both asset classes when none has
 * been stored. A successful Set replaces the bundle wholesale and revokes the live
 * sessions of every user holding the role, so it takes effect on their next login.
 */
export interface RoleCapabilities {
  /** The capabilities the role confers as its base. */
  capabilities: Capability[];
}

// ---------------------------------------------------------------------------
// analytics — the cross-asset client-flow / P&L-attribution rollup
// (`AuthService.ListClientFlowMetrics`; docs/ANALYTICS-REQUIREMENTS.md §11.1a).
// Mirrors `celnet.wire.ClientFlowMetricsDesc` one-to-one: a flat per-group-key
// row of margin-efficiency ($/mm), spread economics and quote-fishing signals.
// Gated on the `view_analytics` capability × the caller's assets.
// ---------------------------------------------------------------------------

/**
 * The dimension a client-flow rollup groups by
 * (`celnet.wire.FlowGroupBy`; CLIENT=0, COUNTERPARTY=1, INSTRUMENT=2, ASSET=3).
 * `asset` is the cross-product slice (FI vs FXO); the others slice per requesting
 * client, per covering counterparty, or per instrument.
 */
export type FlowGroupBy = "client" | "counterparty" | "instrument" | "asset";

/**
 * One client-flow rollup row (`celnet.wire.ClientFlowMetricsDesc`). The `Option<f64>`
 * server fields (`optional double` on the wire) arrive as `number | undefined` — a
 * ratio's denominator was zero, so the value is genuinely ABSENT (rendered "—",
 * never a fabricated `0` or `NaN`). Non-optional fields (counts, notional, the P&L
 * components, `fishingScore`) are always present.
 */
export interface ClientFlowMetrics {
  /** The group key this row belongs to (client / counterparty / instrument / asset). */
  label: string;
  /** Number of quote/RFQ responses issued to this key. */
  quoteCount: number;
  /** Number of fills done with this key. */
  tradedCount: number;
  /** Sum of traded notional magnitude. */
  tradedNotional: number;
  /** Gross margin captured on fills (Σ margin over traded records). */
  grossPnl: number;
  /** Total adverse-selection cost (Σ markout, positive = cost to us). */
  totalMarkout: number;
  /** Total hedging/warehousing cost (Σ hedge cost, positive = cost). */
  totalHedgeCost: number;
  /** Net P&L = grossPnl − totalMarkout − totalHedgeCost. */
  netPnl: number;
  /** Gross margin per USD 1mm traded. ABSENT when no notional traded. */
  dpmGross?: number | undefined;
  /** Net P&L per USD 1mm traded. ABSENT when no notional traded. */
  dpmNet?: number | undefined;
  /** Realised margin ÷ quoted spread over fills. ABSENT when we quoted no spread. */
  capturedVsOffered?: number | undefined;
  /** Mean cover distance (we vs second-best panel quote). ABSENT when no cover. */
  meanCoverDistance?: number | undefined;
  /** Offered spread ($/mm) at which net $/mm hits zero. ABSENT when undefined. */
  breakevenSpread?: number | undefined;
  /** Quotes ÷ trades. ABSENT when no trades (zero-trade fisher caught by fishingScore). */
  quoteToTradeRatio?: number | undefined;
  /** Trades ÷ quotes (the RFQ hit-rate). ABSENT when no quotes. */
  hitRate?: number | undefined;
  /** Bounded [0,1] quote-fishing score (high quote-to-trade × low net $/mm). */
  fishingScore: number;
}

/**
 * An optional epoch-nanos time window for a client-flow query — both bounds
 * optional (absent ⇒ open on that side). Passed to
 * {@link CelnetTransport.listClientFlowMetrics}.
 */
export interface FlowWindow {
  /** Inclusive lower bound (epoch nanos); absent ⇒ open. */
  fromNanos?: bigint;
  /** Exclusive upper bound (epoch nanos); absent ⇒ open. */
  toNanos?: bigint;
}

/**
 * One street-side LP liquidity row (`celnet.wire.LpFlowMetricsDesc`; the LP-side
 * pillar of docs/ANALYTICS-REQUIREMENTS.md §11). Mirrors the server rollup
 * one-to-one — the raw activity/outcome counts are ALWAYS present, while the two
 * `optional double` ratio fields (`winRate`, `meanCover`) arrive as
 * `number | undefined`: a zero-denominator guard means the value is genuinely
 * ABSENT (rendered "—", never a fabricated `0` or `NaN`). This is the street side —
 * who we trade WITH on the LP side — the mirror of {@link ClientFlowMetrics}'s
 * client side.
 */
export interface LpFlowMetrics {
  /** The liquidity-provider id / venue name this row belongs to. */
  lpId: string;
  /** Quote-update ticks observed from this LP over the window (its update frequency). */
  tickCount: number;
  /** Panel responses from this LP (Σ was_quoted) — the win-rate denominator. */
  quoteCount: number;
  /** Deals this LP won (booked_lp_id == lpId). */
  dealsWon: number;
  /** Summed notional magnitude of the deals this LP won (USD). */
  wonNotional: number;
  /** Panel appearances where the LP was quoted but the deal went elsewhere. */
  missed: number;
  /** Times this LP's quote was rejected at ranking on last-look. */
  lastLookRejects: number;
  /** Win-rate = dealsWon ÷ quoteCount. ABSENT when the LP made no quotes. */
  winRate?: number | undefined;
  /**
   * Mean cover distance (this LP's price vs the winner when it was the runner-up).
   * ABSENT when the LP was never the cover.
   */
  meanCover?: number | undefined;
}

// ---------------------------------------------------------------------------
// latency / ops analytics — the per-stage pipeline-latency rollup
// (`AuthService.ListLatencyMetrics`; docs/ANALYTICS-REQUIREMENTS.md §11 latency).
// One row per instrumented stage of the tick→quote→book pipeline, each a
// nanosecond HdrHistogram digest (p50/p99/p99.9/p99.99, min/max/mean, count),
// plus a telemetry-health digest for the bounded offload queue. Read-only and
// gated on the `view_analytics` capability, exactly like the client-flow rollup.
// ---------------------------------------------------------------------------

/**
 * One instrumented pipeline stage's latency digest (`celnet.wire.LatencyStageDesc`).
 * `op` is the stable OpKind label (e.g. `vanilla_price`) the server emits; `stageLabel`
 * is its human title. All latencies are **nanoseconds**; the percentiles are monotone
 * (`p50Ns ≤ p99Ns ≤ p999Ns ≤ p9999Ns ≤ maxNs`). Every field is always present — a stage
 * with no samples reports zeros (rendered "—" by the workspace, never a fabricated value).
 */
export interface LatencyStage {
  /** Stable OpKind label the stage is keyed by (e.g. `vanilla_price`). */
  op: string;
  /** Human-readable stage title (e.g. "Price (pinned core)"). */
  stageLabel: string;
  /** Number of samples in the digest. */
  count: number;
  /** 50th-percentile (median) latency, nanoseconds. */
  p50Ns: number;
  /** 99th-percentile latency, nanoseconds. */
  p99Ns: number;
  /** 99.9th-percentile latency, nanoseconds. */
  p999Ns: number;
  /** 99.99th-percentile latency, nanoseconds. */
  p9999Ns: number;
  /** Minimum observed latency, nanoseconds. */
  minNs: number;
  /** Maximum observed latency, nanoseconds. */
  maxNs: number;
  /** Arithmetic mean latency, nanoseconds. */
  meanNs: number;
}

/**
 * The telemetry offload-queue health digest (`celnet.wire.LatencyHealthDesc`). The
 * pinned hot core stays alloc/log/lock-free (guardrail 11) and offloads samples over a
 * bounded queue; this reports how many were drained vs dropped, any observed gaps in the
 * sample stream, and the histogram tick frequency (ticks per second) the ns figures derive from.
 */
export interface LatencyHealth {
  /** Total samples drained from the offload queue into the digests. */
  drainedTotal: number;
  /** Total samples dropped (bounded queue full — the hot core never blocks). */
  droppedTotal: number;
  /** Observed gaps in the sample sequence (a monotonic-counter skip). */
  observedGaps: number;
  /** Histogram tick frequency (ticks/second) the nanosecond figures derive from. */
  tickHz: number;
}

/**
 * The full latency/ops rollup (`AuthService.ListLatencyMetrics` reply): one
 * {@link LatencyStage} per instrumented stage (server order) plus the offload-queue
 * {@link LatencyHealth} digest.
 */
export interface LatencyMetrics {
  stages: LatencyStage[];
  health: LatencyHealth;
}

// ---------------------------------------------------------------------------
// fixed-income (rates) — the linear-rates pricing contract (`PricingService
// .PriceRates`). Mirrors the `celnet.wire` rates messages one-to-one: a
// `CurveSet` of par-OIS pillars + an `OisInstrument`, priced to a
// `RatesPricingResult`. The oneof grows additively (FRA, IRS, basis) as each is
// backed end-to-end; the OIS arm is the USD-SOFR P0 arm.
// ---------------------------------------------------------------------------

/**
 * Where an OIS curve pillar matures (`celnet.wire.PillarTenor`): a whole-year
 * tenor, a month tenor (sub-/broken-year pillars), or an explicit odd-dated
 * ("broken date") maturity. Exactly one arm, discriminated by `kind`.
 */
export type PillarTenor =
  | { kind: "years"; years: number }
  | { kind: "months"; months: number }
  | { kind: "date"; maturityDate: BrokenDate };

/** A short human label for a pillar tenor: `"5Y"`, `"18M"`, or `"2031-06-30"`. */
export function pillarTenorLabel(tenor: PillarTenor): string {
  switch (tenor.kind) {
    case "years":
      return `${tenor.years}Y`;
    case "months":
      return `${tenor.months}M`;
    case "date": {
      const d = tenor.maturityDate;
      const mm = String(d.month).padStart(2, "0");
      const dd = String(d.day).padStart(2, "0");
      return `${d.year}-${mm}-${dd}`;
    }
  }
}

/** A whole-year pillar tenor — the canonical liquid-grid arm. */
export function yearsPillarTenor(years: number): PillarTenor {
  return { kind: "years", years };
}

/** The whole-year tenor a pillar labels, or `undefined` for month/dated arms. */
export function pillarYears(tenor: PillarTenor): number | undefined {
  return tenor.kind === "years" ? tenor.years : undefined;
}

/** One self-discounting OIS curve pillar (`celnet.wire.OisPillar`). */
export interface OisCurvePillar {
  /** Where this pillar matures: whole-year tenor, month tenor, or broken date. */
  tenor: PillarTenor;
  /** The quoted par (fair fixed) rate as a decimal (0.041 = 4.10%). */
  parRate: number;
}

/**
 * A calibrated set of interest-rate curves (`celnet.wire.CurveSet`). For
 * USD-SOFR (the P0 arm) this is exactly one self-discounting SOFR curve, carried
 * as its dated par-OIS pillars plus the reference (spot-anchor) date; the engine
 * bootstraps the discount/forward term structure from them.
 */
export interface RatesCurveSet {
  /** ISO-4217 currency of the curve (USD for the P0 arm). */
  currency: string;
  /** The curve reference (spot-anchor) civil date the pillar schedules roll from. */
  referenceDate: BrokenDate;
  /** The self-discounting OIS pillars, in strictly increasing tenor order. */
  pillars: readonly OisCurvePillar[];
}

/**
 * The discount-curve interpolation scheme (`celnet.wire.CurveInterpolation`). Two
 * server-backed schemes: LOG-LINEAR on the discount factor (the shipping default —
 * arbitrage-free in DF space, piecewise-constant instantaneous forwards) and
 * MONOTONE-CONVEX on the instantaneous forward (Hagan–West), selectable per
 * {@link CurveDefinition}. The wire carries the int code (0 = log-linear-df,
 * 1 = monotone-convex-forward); the string union is the ergonomic in-app form.
 */
export type CurveInterpolation = "log-linear-df" | "monotone-convex-forward";

/** Every curve interpolation scheme, in wire-code order (index === wire int). */
export const CURVE_INTERPOLATIONS: readonly CurveInterpolation[] = [
  "log-linear-df",
  "monotone-convex-forward",
] as const;

/** The wire int code for a {@link CurveInterpolation} (0 = log-linear-df, 1 = monotone-convex-forward). */
export function curveInterpolationCode(x: CurveInterpolation): number {
  return x === "monotone-convex-forward" ? 1 : 0;
}

/** Decode a wire interpolation int code to a {@link CurveInterpolation} (unknown ⇒ the default). */
export function curveInterpolationFromCode(code: number): CurveInterpolation {
  return code === 1 ? "monotone-convex-forward" : "log-linear-df";
}

/** A short human label for a curve interpolation scheme. */
export function curveInterpolationLabel(x: CurveInterpolation): string {
  return x === "monotone-convex-forward"
    ? "Monotone convex (forward)"
    : "Log-linear (DF)";
}

/**
 * A named, persisted interest-rate curve DEFINITION (`celnet.wire.CurveDefinition`)
 * — the reference-data record the multi-curve manager creates / edits / deletes.
 * `curveId` is an IMMUTABLE unique slug; `pillars` is the calibrating {@link
 * RatesCurveSet} (currency + reference date + ≥1 pillar); `primary` is
 * SERVER-MAINTAINED (exactly one per currency is the default curve). The metadata
 * (index / day-count / calendar / interpolation) describes how the engine
 * bootstraps the pillars.
 */
export interface CurveDefinition {
  /** Immutable unique slug identifying the curve (e.g. `usd-sofr`). */
  curveId: string;
  /** Human display name (e.g. "USD SOFR"). */
  displayName: string;
  /** The rate index the curve projects / discounts (e.g. "USD-SOFR"). */
  indexLabel: string;
  /** The day-count convention label (e.g. "ACT/360"). */
  dayCount: string;
  /** The holiday-calendar label (e.g. "USD"). */
  calendar: string;
  /** The bootstrap interpolation scheme. */
  interpolation: CurveInterpolation;
  /** The calibrating par-OIS pillar set (currency + reference date + ≥1 pillar). */
  pillars: RatesCurveSet;
  /** Server-maintained: the primary (default) curve for its currency — one per ccy. */
  primary: boolean;
}

/**
 * The fixed-leg direction of an OIS from the client's perspective. The wire
 * `Side` carries this: SIDE_BUY pays fixed (payer), SIDE_SELL receives fixed.
 */
export type OisDirection = "PAY_FIXED" | "RECEIVE_FIXED";

/**
 * An overnight-indexed swap to price (`celnet.wire.OisInstrument`) — fixed vs
 * compounded overnight floating on a single self-discounting curve. The schedule
 * is the spot-starting USD-SOFR schedule of `tenorYears`, reconstructed from the
 * `RatesCurveSet` reference date.
 */
export interface OisInstrument {
  /** The swap tenor in whole years from spot (e.g. 2, 5, 10); `>= 1`. */
  tenorYears: number;
  /** The fixed-leg rate as a decimal (0.041 = 4.10%). */
  fixedRate: number;
  /** The notional in the curve currency (always positive; direction is `direction`). */
  notional: number;
  /** Pay-fixed (payer) or receive-fixed (receiver). */
  direction: OisDirection;
}

/**
 * Coupon / leg payment frequency for a linear-rates product (`celnet.wire
 * .PaymentFrequency`) — the number of coupon periods per year the schedule rolls at.
 */
export type PaymentFrequency = "ANNUAL" | "SEMI_ANNUAL" | "QUARTERLY";

/**
 * A curve/leg day-count basis (`celnet.wire.DayCount`) — the money-market subset an
 * IRS leg accrues on (ACT/365F or ACT/360). Distinct from {@link RatesAccrualBasis}:
 * an IRS leg carries no 30/360 arm (the market 30/360 fixed leg awaits the shared
 * curve-time day-count extension), matching the server `VanillaIrsInstrument`.
 */
export type RatesLegDayCount = "ACT_365_FIXED" | "ACT_360";

/**
 * An instrument-level accrual basis (`celnet.wire.AccrualBasis`) — the money-market
 * bases plus 30/360 Bond Basis (the standard USD fixed-bond / fixed-swap basis). Used
 * by the FRA accrual window and the bond coupon accrual.
 */
export type RatesAccrualBasis = "ACT_360" | "ACT_365_FIXED" | "THIRTY_360_BOND_BASIS";

/**
 * A vanilla fixed-vs-float interest-rate swap to price (`celnet.wire
 * .VanillaIrsInstrument`) — a fixed leg vs a projected floating leg on the single
 * self-discounting curve, each leg at its own frequency + day-count. The PV is
 * `N·(K·A_fixed − F_float)` (receive-fixed); `direction` maps to the wire `Side`.
 */
export interface VanillaIrsInstrument {
  /** The swap tenor in whole years from spot (e.g. 2, 5, 10); `>= 1`. */
  tenorYears: number;
  /** The fixed-leg rate as a decimal (0.041 = 4.10%). */
  fixedRate: number;
  /** The notional in the curve currency (always positive; direction is `direction`). */
  notional: number;
  /** Pay-fixed (payer) or receive-fixed (receiver). */
  direction: OisDirection;
  /** Fixed-leg payment frequency (USD market: semi-annual). */
  fixedFrequency: PaymentFrequency;
  /** Fixed-leg accrual day-count (ACT/365F or ACT/360). */
  fixedDayCount: RatesLegDayCount;
  /** Float-leg payment frequency (USD market: quarterly). */
  floatFrequency: PaymentFrequency;
  /** Float-leg accrual day-count (ACT/360 typical for USD). */
  floatDayCount: RatesLegDayCount;
}

/**
 * A forward rate agreement to price (`celnet.wire.FraInstrument`) — a single accrual
 * window `[startMonths, endMonths]` on the projected float index, rebuilt server-side
 * to roll-adjusted dates from the curve reference date. The PV is the single-period
 * swaplet `N·(K·τ·DF(end) − (DF(start) − DF(end)))` (receive-fixed); `direction` maps
 * to the wire `Side`.
 */
export interface FraInstrument {
  /** The window start (fixing) tenor in months from spot (e.g. 3); `< endMonths`. */
  startMonths: number;
  /** The window end (maturity) tenor in months from spot (e.g. 6); `> startMonths`. */
  endMonths: number;
  /** The contractual fixed rate K as a decimal (0.033 = 3.30%). */
  fixedRate: number;
  /** The notional in the curve currency (always positive; direction is `direction`). */
  notional: number;
  /** Pay-fixed (payer) or receive-fixed (receiver). */
  direction: OisDirection;
  /** The accrual day-count basis for τ (ACT/360 typical; 30/360 supported). */
  accrualBasis: RatesAccrualBasis;
}

/** A cash-bond position direction — `LONG` (bought, +PV) or `SHORT` (sold, −PV). */
export type BondPosition = "LONG" | "SHORT";

/**
 * A fixed-coupon cash bond to price off the calibrated curve (`celnet.wire
 * .BondInstrument`) — each cashflow discounted at the bootstrapped curve (dirty
 * price), reported with the implied yield-risk set. Settlement is the curve
 * reference (spot-anchor) date; the coupon schedule is the regular month-step dates
 * rolled back from `maturityDate` at `couponFrequency`. `position` maps to the wire
 * `Side` (LONG = SIDE_BUY, SHORT = SIDE_SELL).
 */
export interface BondInstrument {
  /** The annual coupon rate as a decimal (0.06 = 6%); 0 for a zero-coupon bond. */
  couponRate: number;
  /** The coupon payment frequency (also the yield compounding basis). */
  couponFrequency: PaymentFrequency;
  /** The accrual day-count basis for accrued interest (ACT/365F / ACT/360 / 30/360). */
  dayCount: RatesAccrualBasis;
  /** The maturity (final-redemption) date; must be strictly after settlement. */
  maturityDate: BrokenDate;
  /** The par redemption / face value (e.g. 100); required, strictly positive. */
  redemption: number;
  /** Long (bought, +PV) or short (sold, −PV). */
  position: BondPosition;
}

/**
 * A linear interest-rate instrument to price (`celnet.wire.RatesInstrument` oneof) —
 * exactly one arm is set, discriminated by `kind`. The browser projection of the
 * server's `rates_instrument` oneof: the ticket builds one of these and prices it via
 * `CelnetTransport.priceRates`. The arms grow additively (rule 9), never renumbering.
 */
export type RatesInstrument =
  | { kind: "ois"; ois: OisInstrument }
  | { kind: "irs"; irs: VanillaIrsInstrument }
  | { kind: "fra"; fra: FraInstrument }
  | { kind: "bond"; bond: BondInstrument };

/** Wrap a bare {@link OisInstrument} as the `ois` arm of a {@link RatesInstrument}. */
export function oisRatesInstrument(ois: OisInstrument): RatesInstrument {
  return { kind: "ois", ois };
}

/**
 * The priced result for a linear-rates instrument (`celnet.wire
 * .RatesPricingResult`). All measures are in the curve currency and already
 * carry the instrument direction sign (a payer and a receiver of the same swap
 * report equal-and-opposite PV / PV01 / DV01 / ladder).
 */
export interface RatesPricingResult {
  /** Present value in the curve currency (sign per direction). */
  pv: number;
  /** The par (fair fixed) rate of the schedule on the calibrated curve, a decimal. */
  parRate: number;
  /** Analytic PV01: the PV change per 1bp move in the fixed rate (signed per direction). */
  pv01: number;
  /** DV01: the PV change for a +1bp parallel bump of every calibrating pillar. */
  dv01: number;
  /**
   * The key-rate (bucketed) DV01 ladder: one entry per curve pillar, in pillar
   * order, each the PV change for a +1bp bump of that pillar alone. Sums to
   * `dv01` to first order (the residual is curve cross-gamma).
   */
  keyRateLadder: readonly number[];
}

/**
 * A tradeable two-way quote for a linear-rates instrument (`celnet.wire
 * .RatesQuote`, returned by `QuoteService.RequestRatesQuote`) — the fixed-income
 * analogue of {@link Quote}. The taker's price-discovery two-way, mirroring the
 * FIX venue auto-quote: `price` is struck around the SIDE-INDEPENDENT fair level —
 * a RATE market (bid/offer around the par rate) for an OIS/IRS/FRA, a clean-PRICE
 * market (bid/offer around the clean price per 100 face) for a cash bond — so
 * `(bid+offer)/2` is the fair level the FI pricing path computed. `result` carries
 * the full linear-rates risk (PV, par rate, PV01, DV01, key-rate ladder) at the
 * requested `side`. There is NO multi-dealer rates path on the contract: the
 * client-reachable FI RFQ is this single two-way (the `QuoteRequest` panel wire is
 * FX-`Instrument` only), so no `DealerQuote` ladder is fabricated for rates.
 */
export interface RatesQuote {
  /** Server-assigned stable, unguessable quote identifier (minted like `Quote.quoteId`). */
  quoteId: bigint;
  /** Echo of the originating request's idempotency key. */
  idempotencyKey: string;
  /**
   * The two-way bid/offer: a rate market for an OIS/IRS/FRA, a clean-price market
   * for a cash bond. `(bid+offer)/2` is the side-independent fair level.
   */
  price: TwoWayPrice;
  /** The full linear-rates risk of the quoted position at the requested `side`. */
  result: RatesPricingResult;
  /** The RFQ size the two-way is good for (curve currency), echoed from the request. */
  notional: number;
  /** Publication time, nanoseconds since the Unix epoch (UTC). */
  epochNanos: bigint;
  /** Quote validity deadline (last-look window), nanoseconds since the Unix epoch (UTC). */
  validUntilNanos: bigint;
  /** Echo of the originating request's correlation id, if one was supplied. */
  correlationId?: bigint;
}

// ---------------------------------------------------------------------------
// Fixed-income (linear-rates) LIVE STREAMING — the FI analogue of the FX
// Snapshot/Update RFS line, folded onto the SAME multiplexed StreamSession (and
// the SAME server PriceFanout). A rates line opens with a `RatesInstrument`
// priced against a baseline `RatesCurveSet` (the FI analogue of the FX
// instrument + market), and the server streams a baseline `RatesStreamSnapshot`
// then sequenced `RatesStreamUpdate`s as the curve deterministically ticks (a
// parallel par-rate shift). The streamed line is INDICATIVE (PV + first-order
// risk) — click-to-trade for rates books through the RFQ/desk path, so NO
// TradableToken rides this stream. Mirrors `celnet.wire.RatesStreamSnapshot` /
// `RatesStreamUpdate` field-for-field.
// ---------------------------------------------------------------------------

/**
 * The baseline state of a streamed fixed-income line at a sequence point — the
 * priced {@link RatesPricingResult} a consumer applies whole before consuming
 * deltas. Faithful to `PricingService.PriceRates`: the snapshot's `result` at
 * the subscribed baseline curve (`curveShift === 0`) equals
 * `price_rates(instrument, curve_set)` EXACTLY.
 */
export interface RatesStreamSnapshot {
  /** The client-assigned subscription id this snapshot answers. */
  subscriptionId: bigint;
  /** The monotonic per-subscription sequence number of this snapshot (1). */
  sequence: bigint;
  /** The priced PV + par + PV01 / DV01 / key-rate ladder at this snapshot. */
  result: RatesPricingResult;
  /**
   * The parallel curve shift (decimal, added to every pillar par rate) applied
   * relative to the subscribed baseline curve — always `0` at the baseline.
   */
  curveShift: number;
  /** Echo of the opening subscribe correlation id, when one was supplied. */
  correlationId?: bigint;
  /** Snapshot time, nanoseconds since the Unix epoch (UTC). */
  epochNanos: bigint;
}

/**
 * A sequenced delta on a streamed fixed-income line (`celnet.wire
 * .RatesStreamUpdate`): the line re-priced at the next sequence against the
 * baseline curve shifted by `curveShift`. A gap in `sequence` signals loss.
 */
export interface RatesStreamUpdate {
  /** The subscription this update advances. */
  subscriptionId: bigint;
  /** The monotonic per-subscription sequence number (snapshot seq + n). */
  sequence: bigint;
  /** The re-priced PV + first-order risk at this sequence. */
  result: RatesPricingResult;
  /** The parallel curve shift (decimal) applied to the baseline curve this tick. */
  curveShift: number;
  /** Update time, nanoseconds since the Unix epoch (UTC). */
  epochNanos: bigint;
}

// ---------------------------------------------------------------------------
// FI Aggregated Book (ADR-0022) — an admin-defined composite book that
// consolidates N inbound liquidity members' two-way quotes into ONE best
// bid/offer per instrument. Two surfaces on the single contract:
//   • admin CRUD (`AuthService.{List,Create,Update,Delete}AggregatedBook`) — the
//     persisted definition (members, instrument scope, consolidation tuning);
//   • the live composite (`StreamService.StreamSession`, keyed by the SAME
//     `SubscriptionId` space as the FX/rates lines) — the consolidated best
//     bid/offer + per-member contribution report a subscriber renders.
// The `id` is the store/API key; `member_connection_ids` are transport-agnostic
// connection identities (a FIX acceptor id, or an LP-feed member name such as
// `LP-SIM-01`). Mirrors `celnet.wire` `AggregatedBookDesc`/`AggregatedBookSpec`.

/**
 * The instrument-coverage mode of an aggregated book. `ALL_MEMBERS_QUOTE`
 * consolidates every instrument any member quotes (the union of the members'
 * live streams); `EXPLICIT` consolidates only the listed `instrumentIds`. Maps
 * onto the wire `AggregationScopeMode` enum (`ALL_MEMBERS_QUOTE = 0`,
 * `EXPLICIT = 1`).
 */
export type AggregationScopeMode = "ALL_MEMBERS_QUOTE" | "EXPLICIT";

/**
 * The consolidation-engine tuning of an aggregated book (`celnet.wire
 * .AggregationParamsDesc`): the knobs the `celnet-aggregation` engine consumes
 * once a book is wired. `stalenessTauMs`/`maxQuoteAgeMs` are millisecond
 * durations (held as `number` — well within the JS safe-integer range); the
 * counts are small non-negative integers.
 */
export interface AggregationParams {
  /** Staleness half-life τ (ms): a member quote decays as `2^{-dt/τ}` with age `dt`. `> 0`. */
  stalenessTauMs: number;
  /** Hard maximum quote age (ms): a member quote older than this is fully excluded. */
  maxQuoteAgeMs: number;
  /** Whether MAD-based divergence gating drops outlier members before consolidation. */
  divergenceGating: boolean;
  /** Minimum surviving contributors required to publish a composite. `>= 1`. */
  minContributors: number;
  /** How many stacked depth levels the composite exposes (1 = top-of-book only). */
  depthLevels: number;
}

// ---------------------------------------------------------------------------
// Outbound price tiering (FI-TIERING) — the margin/markup config the server's
// `celnet-tiering` engine applies: it widens around mid (half-spread) and/or skews
// (inventory), clamped by guardrails. Mirrors `celnet.wire.TieringConfigDesc`
// field-for-field. Tiering is composed per-client via a {@link FeatureSpec.tiering}
// TIERING feature in a pricing-group pipeline (it is NOT carried on the aggregated
// book). The GUI hand-decodes the WS JSON, so the wire codec (`wsCodec.ts`) matches
// the server's snake_case names + NUMERIC enums exactly (see `docs/FI-TIERING-RESEARCH.md`).

/**
 * The unit an outbound-tiering spread magnitude is expressed in (mirrors the wire
 * `TieringSpreadUnit`: PRICE_BPS=0, YIELD_BPS=1, PRICE_POINTS=2, PERCENT=3). A bond
 * "25 bps" is ambiguous — price bps (a fixed price offset) vs duration-consistent
 * yield bps (converted via the bond's DV01) — so the unit is carried explicitly.
 */
export type TieringSpreadUnit = "PRICE_BPS" | "YIELD_BPS" | "PRICE_POINTS" | "PERCENT";

/**
 * What the engine does when the upstream composite is stale / the LP quorum is lost
 * (mirrors the wire `TieringStalePolicy`: SUPPRESS=0, WIDEN_TO_MAX=1). `SUPPRESS`
 * publishes no quote for the line (the safe default); `WIDEN_TO_MAX` keeps a market
 * but widens to the guardrail `hMax` with zero skew.
 */
export type TieringStalePolicy = "SUPPRESS" | "WIDEN_TO_MAX";

/**
 * Which pluggable tiering strategy a {@link TieringStrategy} carries (mirrors the
 * wire `TieringStrategyKind`: FLAT_MARKUP=0, INVENTORY_SKEW=1). Phase 2a ships the
 * two streaming-relevant strategies; the remaining four (vol scale, size ladder,
 * toxicity, per-client tier) slot in additively as new union members later.
 */
export type TieringStrategyKind = "FLAT_MARKUP" | "INVENTORY_SKEW" | "SCALED_SMOOTHED_SPREAD";

/**
 * One enabled tiering strategy and its parameters (mirrors `celnet.wire
 * .TieringStrategyDesc`). Magnitudes are in the parent {@link TieringConfig.unit}
 * for FLAT_MARKUP / INVENTORY_SKEW; the SCALED_SMOOTHED_SPREAD spread params
 * (`expectedSpread`/`maxDivergence`/`coreSpread`/`maxOutputSpread`) are ABSOLUTE
 * price offsets. Fields a `kind` does not use are ignored (a `FLAT_MARKUP` strategy
 * ignores every field but `halfSpread`).
 */
export interface TieringStrategy {
  /** Which strategy this entry configures. */
  kind: TieringStrategyKind;
  /** The (base) half-spread magnitude `H` (FLAT_MARKUP / INVENTORY_SKEW). */
  halfSpread: number;
  /** Inventory-skew gain `kappa` (magnitude per unit inventory); INVENTORY_SKEW only. */
  kappa: number;
  /** Inventory-skew strategy-local cap `sMax` (magnitude); INVENTORY_SKEW only. */
  sMax: number;
  /** Scaled-Smoothed Smoothing Weight `w ∈ (0, 1]` (1 ⇒ off); SCALED_SMOOTHED_SPREAD only. */
  smoothingWeight: number;
  /** Scaled-Smoothed Expected Spread `e > 0` (absolute); SCALED_SMOOTHED_SPREAD only. */
  expectedSpread: number;
  /** Scaled-Smoothed Max Divergence `d ≥ 0` dead-band (absolute); SCALED_SMOOTHED_SPREAD only. */
  maxDivergence: number;
  /** Scaled-Smoothed Core spread `c ≥ 0` (absolute); SCALED_SMOOTHED_SPREAD only. */
  coreSpread: number;
  /** Scaled-Smoothed Max Output Spread `m ≥ c` (absolute); SCALED_SMOOTHED_SPREAD only. */
  maxOutputSpread: number;
  /** Scaled-Smoothed Spread Scale Factor `f ≥ 0`; SCALED_SMOOTHED_SPREAD only. */
  spreadScaleFactor: number;
}

/**
 * The price-space guardrail bounds (mirrors `celnet.wire.TieringGuardrailsDesc`);
 * all four are absolute price offsets (points), independent of the spread unit.
 */
export interface TieringGuardrails {
  /** Minimum half-spread `hMin >= 0`. */
  hMin: number;
  /** Maximum half-spread `hMax >= hMin` (also the widen-to-max width). */
  hMax: number;
  /** Maximum absolute skew `sMax >= 0`. */
  sMax: number;
  /** Minimum tradeable spread `spreadFloor > 0` (`offer - bid >= spreadFloor`). */
  spreadFloor: number;
}

/**
 * A book's complete outbound-tiering configuration (mirrors `celnet.wire
 * .TieringConfigDesc`). Present ⇒ tiering enabled; the raw composite is widened /
 * skewed by the composed `strategies` and clamped by `guardrails` before publish.
 * `guardrails` may be `null` on the wire (the server renders an absent guardrails
 * block as `null`); the admin form always sends a full guardrails block.
 */
export interface TieringConfig {
  /** The unit every strategy magnitude below is expressed in. */
  unit: TieringSpreadUnit;
  /** The enabled strategies, composed additively in order. */
  strategies: TieringStrategy[];
  /** The price-space guardrail bounds clamping the composed result (`null` ⇒ none). */
  guardrails: TieringGuardrails | null;
  /** What to do on stale/absent upstream inputs. */
  stalePolicy: TieringStalePolicy;
}

/** A persisted aggregated book (`celnet.wire.AggregatedBookDesc`). */
export interface AggregatedBookDesc {
  /** Stable identifier (the store/API key), minted from `name` on create. */
  id: string;
  /** Human-friendly book label (unique, case-insensitive). */
  name: string;
  /** The inbound liquidity members whose quotes feed the composite, by connection id. */
  memberConnectionIds: string[];
  /** Which instruments the composite is produced for. */
  scopeMode: AggregationScopeMode;
  /** The explicit instrument ids when `scopeMode === "EXPLICIT"`; empty otherwise. */
  instrumentIds: string[];
  /** The consolidation-engine tuning. */
  params: AggregationParams;
  /** Whether the book is active (a disabled book stands up no engine, publishes nothing). */
  enabled: boolean;
}

/**
 * The editable fields of an aggregated book (the create/update payload,
 * `celnet.wire.AggregatedBookSpec`). On create `id` is a client-suggested slug
 * (a slug of `name` is minted when empty); on update the request's own `id`
 * governs and this `id` is ignored.
 */
export interface AggregatedBookSpec {
  /** On create: a client-suggested id (slug of `name` when empty). On update: ignored. */
  id: string;
  /** Human-friendly book label (unique across the store, case-insensitive). */
  name: string;
  /** The inbound liquidity members whose quotes feed the composite, by connection id. */
  memberConnectionIds: string[];
  /** Which instruments the composite is produced for. */
  scopeMode: AggregationScopeMode;
  /** The explicit instrument ids when `scopeMode === "EXPLICIT"`; ignored for ALL_MEMBERS_QUOTE. */
  instrumentIds: string[];
  /** The consolidation-engine tuning applied when the book is wired to the engine. */
  params: AggregationParams;
  /** Whether the book is active. */
  enabled: boolean;
}

/**
 * One inbound member's contribution to an instrument's composite (`celnet.wire
 * .LpContribution`): the member's own two-way, and whether it was excluded from
 * the consolidated best price (`stale` — aged out past the book's max age, or
 * gated as a divergent outlier; a subscriber greys a stale contributor).
 */
export interface LpContribution {
  /** The contributing LP connection name (the consolidation venue id). */
  lpName: string;
  /** The member's own bid for this instrument. */
  bid: number;
  /** The member's own offer for this instrument. */
  offer: number;
  /** Whether the member was excluded from the consolidated best bid/offer. */
  stale: boolean;
}

/**
 * The consolidated composite for one instrument in an aggregated book
 * (`celnet.wire.AggregatedInstrument`): the best bid/offer + firm size across
 * the fresh members, a confidence measure, and the per-member contribution
 * report. Identity fields (`displayName`/`isin`/`cusip`) are resolved from the
 * server's reference-data registry best-effort (empty when unresolved).
 */
export interface AggregatedInstrument {
  /** The canonical server `instrument_id` (the composite line's identity key). */
  instrumentId: string;
  /** Human-friendly label from reference data (empty if unresolved). */
  displayName: string;
  /** ISO 6166 ISIN from reference data (empty if unresolved). */
  isin: string;
  /** CUSIP from reference data (empty if unresolved). */
  cusip: string;
  /** Consolidated best bid — the max fresh member bid. */
  bestBid: number;
  /** Consolidated best offer — the min fresh member offer. */
  bestOffer: number;
  /** Firm size stacked at the consolidated best bid. */
  bidSize: number;
  /** Firm size stacked at the consolidated best offer. */
  offerSize: number;
  /** Confidence in the composite, `∈ [0, 1]` (coverage · freshness · agreement). */
  confidence: number;
  /** The per-member contribution / exclusion report (member id order). */
  contributions: LpContribution[];
}

/**
 * The full composite state of an aggregated book at a sequence point
 * (`celnet.wire.AggregatedBookSnapshot`): the priced composite for every
 * in-scope instrument that currently meets the book's quorum, in `instrumentId`
 * order. A consumer applies a snapshot whole before consuming deltas.
 */
export interface AggregatedBookComposite {
  /** The aggregated-book id this composite is for. */
  bookId: string;
  /** The composite for each in-scope instrument, in `instrumentId` order. */
  instruments: AggregatedInstrument[];
}

/**
 * The baseline composite of a subscribed aggregated book (`celnet.wire
 * .AggregatedBookStreamSnapshot`). `sequence` starts at 1; a consumer applies
 * the whole `book` before consuming `AggregatedBookStreamUpdate` deltas.
 */
export interface AggregatedBookStreamSnapshot {
  /** The client-assigned subscription id this snapshot answers. */
  subscriptionId: bigint;
  /** The monotonic per-subscription sequence number of this snapshot (1). */
  sequence: bigint;
  /** The full composite state at this snapshot. */
  book: AggregatedBookComposite;
  /** Echo of the opening subscribe correlation id, when one was supplied. */
  correlationId?: bigint;
  /** Snapshot time, nanoseconds since the Unix epoch (UTC). */
  epochNanos: bigint;
}

/**
 * A sequenced delta on a subscribed aggregated book (`celnet.wire
 * .AggregatedBookStreamUpdate`): the full re-consolidated composite at the next
 * sequence. A gap in `sequence` signals loss and prompts a resync.
 */
export interface AggregatedBookStreamUpdate {
  /** The subscription this update advances. */
  subscriptionId: bigint;
  /** The monotonic per-subscription sequence number (snapshot seq + n). */
  sequence: bigint;
  /** The re-consolidated composite at this sequence. */
  book: AggregatedBookComposite;
  /** Update time, nanoseconds since the Unix epoch (UTC). */
  epochNanos: bigint;
}

// ---------------------------------------------------------------------------
// FI Pricing Groups (docs/FI-PRICING-GROUPS-DESIGN.md) — a trader-composable,
// ordered pipeline of pricing FEATURES bound to a named group of clients (FIX
// connections / GUI-API users / desks). The running two-way flows RAW → the
// trader's ordered features → OUTBOUND (an ESP stream or an RFS/RFQ quote). Each
// pricing mode (ESP / RFQ) carries its own pipeline; `sharePipeline` makes RFQ
// mirror ESP. Mirrors `celnet.wire` `PricingGroupDesc`/`FeaturePipelineDesc`/
// `FeatureSpecDesc` field-for-field; the GUI hand-decodes the WS JSON so the wire
// codec (`wsCodec.ts`) matches the server's snake_case names + NUMERIC enums
// (kind / unit / axe_side / mode) exactly. Reuses the shipped {@link TieringConfig}
// verbatim for the TIERING feature and {@link TieringGuardrails} for the pipeline
// guardrails. Server backend landed at server commit 07fc99f.

/**
 * Which pricing feature a {@link FeatureSpec} configures (mirrors the wire
 * `PricingFeatureKind`: MID_SHIFT=0, TIERING=1, AXE=2, POSITION=3, PANIC_SKEW=4).
 * Each is a self-contained transform on the running two-way; the trader drags them
 * from the palette into a group's pipeline and the LIST ORDER is the pipeline.
 */
export type FeatureKind = "MID_SHIFT" | "TIERING" | "AXE" | "POSITION" | "PANIC_SKEW";

/**
 * Which outbound pricing mode a pipeline drives (mirrors the wire `EspOrRfq`:
 * ESP=0, RFQ=1). ESP is the executable streaming price; RFQ is the request-for-
 * quote / order price. A group configures each mode independently unless
 * {@link PricingGroup.sharePipeline} makes RFQ mirror ESP.
 */
export type PricingMode = "ESP" | "RFQ";

/**
 * Which side the AXE feature leans toward (mirrors the wire `AxeSide`: BUY=0,
 * SELL=1) — the direction the desk wants to trade, so the two-way is skewed to
 * attract that flow.
 */
export type AxeSide = "BUY" | "SELL";

/**
 * One pricing feature and every parameter any kind could use (mirrors `celnet.wire
 * .FeatureSpecDesc`). Every field EXCEPT `reference` is always present on the wire;
 * a feature ignores the fields its `kind` does not use. `reference` is OPTIONAL: it
 * is emitted only when set (a MID_SHIFT reference-price override) and decodes to
 * `null` when absent. The TIERING feature reuses {@link TieringConfig} verbatim.
 */
export interface FeatureSpec {
  /** Which feature this entry configures. */
  kind: FeatureKind;
  /** The unit the MID_SHIFT / AXE magnitudes are expressed in (reuses the tiering unit vocabulary). */
  unit: TieringSpreadUnit;
  /** MID_SHIFT: the signed shift applied to mid (in `unit`). */
  shift: number;
  /** MID_SHIFT: an absolute reference-price override for mid; `null` ⇒ none (absent on the wire). */
  reference: number | null;
  /** TIERING: the reused margin/markup config; `null` ⇒ the feature applies no tiering. */
  tiering: TieringConfig | null;
  /** AXE: which side to lean toward. */
  axeSide: AxeSide;
  /** AXE: how far to skew mid toward `axeSide` (in `unit`). */
  magnitude: number;
  /** POSITION: inventory-skew gain κ (skew per unit net inventory). */
  kappa: number;
  /** POSITION: the inventory-skew clamp sMax (max absolute skew). */
  sMax: number;
  /** PANIC_SKEW: the signed emergency skew applied to mid when `triggered`. */
  skew: number;
  /** PANIC_SKEW: whether the overlay skew is currently active. */
  triggered: boolean;
}

/**
 * An ordered feature pipeline for one pricing mode (mirrors `celnet.wire
 * .FeaturePipelineDesc`): the features in run order plus the price-space
 * {@link TieringGuardrails} clamping the composed result (`null` ⇒ none).
 */
export interface FeaturePipeline {
  /** The features to run, in order (RAW is the implicit start; OUTBOUND the end). */
  features: FeatureSpec[];
  /** The guardrail bounds clamping the pipeline's output; `null` ⇒ none. */
  guardrails: TieringGuardrails | null;
}

/**
 * How a pricing group's rates/bond FIX auto-quotes source their RAW price (mirrors
 * `celnet.wire.PricingSourceMode`, wire integer tags). Orthogonal to the feature
 * pipeline: the pipeline SHAPES the quote, this selects the price the pipeline shapes.
 *  - `0` COMPOSITE_FIRST_CURVE_FALLBACK (default): price off the aggregated book when
 *    it is fed, else fall back to the curve.
 *  - `1` CURVE_ONLY: always price off the bootstrapped curve, ignoring the book.
 *  - `2` PRODUCT_SPLIT: bonds price off the book, OIS off the curve.
 *  - `3` CURVE_ANCHORED_BOOK_SKEW: curve backbone with the mid pulled `bookSkewWeight`
 *    ∈ [0,1] toward the composite (and the composite's half-spread).
 */
export type PricingSourceMode = 0 | 1 | 2 | 3;

/**
 * The dealer last-look policy for a streamed-quote lift when the market moved
 * between quote and order (mirrors `celnet.wire.LastLookMode`, wire integer tag).
 * Governs ONLY the favorable side — an adverse move beyond
 * {@link PricingGroup.lastLookToleranceBps} is always rejected regardless of mode.
 *  - `0` SYNC (default): the client is filled at exactly the price they requested;
 *    the desk keeps the ENTIRE favorable move.
 *  - `1` ASYNC: the client gets price improvement — {@link PricingGroup.asyncGivebackPct}%
 *    of the favorable move is passed back to them; the desk keeps the rest.
 */
export type LastLookMode = 0 | 1;

/**
 * A persisted pricing group (mirrors `celnet.wire.PricingGroupDesc`). Membership is
 * many-to-one (many FIX connections / users / desks resolve to ONE group). Each
 * mode carries its own pipeline; `esp_pipeline` / `rfq_pipeline` may be `null`
 * (that mode falls back to the book-default tiering). On CREATE, `id` is a client-
 * suggested slug (the server mints one from `name` when empty); on UPDATE it is the
 * immutable identity. The same interface is the create/update `spec` payload.
 */
export interface PricingGroup {
  /** Stable slug (the store/API key); a client-suggested slug on create. */
  id: string;
  /** The trader's group code name ("GROUP-A"), unique case-insensitive. */
  name: string;
  /** A free-text description of the group. */
  description: string;
  /** Member inbound FIX sessions, by `FixConnection.id`. */
  memberConnectionIds: string[];
  /** Member GUI/API principals, by `UserDesc.id`. */
  memberUserIds: string[];
  /** Member desks (a desk-level default tier), by `DeskDesc.id`. */
  memberDesks: string[];
  /** The ESP / streaming feature pipeline; `null` ⇒ book-default fallback. */
  espPipeline: FeaturePipeline | null;
  /** The RFS/RFQ feature pipeline; `null` ⇒ book-default fallback (ignored when `sharePipeline`). */
  rfqPipeline: FeaturePipeline | null;
  /** When true, RFQ uses the ESP pipeline (the two modes share one pipeline). */
  sharePipeline: boolean;
  /** Whether the group is active (a disabled group prices nobody). */
  enabled: boolean;
  /** How rates/bond FIX auto-quotes source their raw price (default `0` composite-first). */
  pricingSourceMode: PricingSourceMode;
  /**
   * Book skew weight ∈ [0,1], only meaningful when {@link pricingSourceMode} is `3`
   * (CURVE_ANCHORED_BOOK_SKEW). `null` ⇒ omitted on write so the server applies its
   * default (0.5); present on read only when the server has a stored value.
   */
  bookSkewWeight: number | null;
  /**
   * The market-data last-look policy for a streamed-quote lift (default `0` SYNC).
   * Always present as an int on the wire (a plain, non-`optional` proto enum).
   */
  lastLookMode: LastLookMode;
  /**
   * Adverse-move tolerance in bps of price: a lift is rejected when the market has
   * moved AGAINST the desk by more than this between quote and order; within it, the
   * lift is honored. `null` ⇒ omitted on write so the server applies its default
   * (1.0); present on read only when the server has a stored value.
   */
  lastLookToleranceBps: number | null;
  /**
   * The % of a FAVORABLE move passed back to the client as price improvement, `0..100`.
   * Only meaningful when {@link lastLookMode} is `1` (ASYNC). `null` ⇒ omitted on write
   * so the server applies its default (50); present on read only when set.
   */
  asyncGivebackPct: number | null;
}

// ---------------------------------------------------------------------------
// FI Risk routing & risk books (docs/FI-RISK-ROUTING-REQUIREMENTS.md §6-8). When
// an order / RFQ fills, the resulting RISK is routed into a trader-defined RISK
// BOOK (a portfolio node) so limits / greeks / PnL are managed per book. Books
// form a TREE (a parent aggregates its own routed positions plus every
// descendant's); a firm-wide DECISION GRAPH routes each fill to a leaf book. This
// block mirrors the pricing-group CRUD shape field-for-field with the server store
// types (`config::identity::{RiskBookDef, RiskLimits}` +
// `celnet_risk_routing::{RiskRoutingGraph, RoutingNode, RouteField, RouteOp,
// RouteValue}`). The GUI hand-decodes the WS JSON so {@link wsCodec} matches the
// server's snake_case names, its NUMERIC enum tags (`field` / `op` / `band`), and
// its variant-keyed oneofs (a node is a condition OR a book leaf; a condition's
// right-hand value is num / text / string-list / numeric-range). Every RPC is
// ADMIN-gated (a finer `risk_manage·fixed_income` capability is a later
// refinement); edit affordances gate on `auth.isAdmin`.

/**
 * Per-book pre-trade limits (mirrors `celnet.wire.RiskLimitsDesc` /
 * `config::identity::RiskLimits`). Each cap is OPTIONAL: `null` ⇒ uncapped (the
 * key is OMITTED on the wire, never sent as a zero — a real cap of 0 differs from
 * "no cap"). Every present cap is finite and non-negative (server-validated).
 */
export interface RiskLimits {
  /** Cap on the book's net (signed-then-absolute) base-currency notional; `null` ⇒ uncapped. */
  maxNetNotional: number | null;
  /** Cap on the book's gross (sum-of-absolute) base-currency notional; `null` ⇒ uncapped. */
  maxGrossNotional: number | null;
  /** Cap on the book's net DV01 magnitude (PV per +1bp); `null` ⇒ uncapped. */
  maxDv01: number | null;
}

/**
 * A persisted risk book (mirrors `celnet.wire.RiskBookDesc` /
 * `config::identity::RiskBookDef`). Books form a TREE via {@link parentId} (a
 * top-level book has `parentId === null`); a book may tag an owning {@link deskId}.
 * On CREATE, `id` is a client-suggested slug (the server mints one from `name`
 * when empty); on UPDATE it is the immutable identity. The same interface is the
 * create/update `spec` payload (the spec's own `id` is ignored on update).
 */
export interface RiskBook {
  /** Stable slug (the store/API key); a client-suggested slug on create. */
  id: string;
  /** Human-friendly book label (unique across the store, case-insensitive). */
  name: string;
  /** The parent book by id, or `null` for a top-level book (a tree edge). */
  parentId: string | null;
  /** The owning desk by id, or `null` (unowned). Must resolve to a desk when set. */
  deskId: string | null;
  /** Free-text operator description of what the book is for. */
  description: string;
  /** Optional per-book pre-trade limits; `null` ⇒ the book carries no caps yet. */
  limits: RiskLimits | null;
  /** Whether the book is active. Only enabled books are valid routing targets. */
  enabled: boolean;
}

/**
 * Which trade field a routing condition matches (mirrors the wire `RouteFieldEnum`
 * / `celnet_risk_routing::RouteField`, same ordinal order 0..10). The codec maps
 * this string union to/from the wire i32 tag.
 */
export type RouteField =
  | "instrument_id"
  | "ccy"
  | "product"
  | "side"
  | "notional"
  | "tenor"
  | "strike"
  | "counterparty"
  | "user"
  | "desk"
  | "price";

/**
 * A comparison operator in a routing condition (mirrors the wire `RouteOpEnum` /
 * `celnet_risk_routing::RouteOp`, same ordinal order 0..8). Which ops are valid for
 * a field is pinned by the field's kind (server-validated on write).
 */
export type RouteOp = "eq" | "ne" | "gt" | "ge" | "lt" | "le" | "contains" | "in" | "between";

/**
 * The literal on the right-hand side of a routing condition (mirrors the wire
 * `RouteValueDesc` oneof / `celnet_risk_routing::RouteValue`). Exactly one variant
 * is live; which is legal is operator-constrained (server-validated). Carried
 * variant-keyed on the wire: `{num}` | `{text}` | `{list:{values}}` |
 * `{range:{lo,hi}}`.
 */
export type RouteValue =
  | { kind: "num"; num: number }
  | { kind: "text"; text: string }
  | { kind: "list"; values: string[] }
  | { kind: "range"; lo: number; hi: number };

/**
 * A decision node body: evaluate `field op value` on the fill; on `true` follow
 * {@link onTrue}, else {@link onFalse} (mirrors `RouteConditionDesc`). `value` is
 * `null` for a malformed / not-yet-set condition (server rejects on write).
 */
export interface RouteCondition {
  /** The trade field to test. */
  field: RouteField;
  /** The comparison operator. */
  op: RouteOp;
  /** The literal compared against; `null` ⇒ unset. */
  value: RouteValue | null;
  /** Successor node id when the condition holds. */
  onTrue: number;
  /** Successor node id when the condition does not hold. */
  onFalse: number;
}

/**
 * One node in a {@link RiskRoutingGraph}, keyed by its {@link id} (mirrors the wire
 * `RoutingNodeDesc` oneof / `celnet_risk_routing::RoutingNode`). Either an internal
 * `condition` test or a terminal `book` leaf carrying its target risk-book id.
 */
export type RoutingNode =
  | { kind: "condition"; id: number; condition: RouteCondition }
  | { kind: "book"; id: number; bookId: string };

/**
 * The firm-wide routing decision graph (mirrors `celnet.wire.RiskRoutingGraphDesc`
 * / `celnet_risk_routing::RiskRoutingGraph`): the walk begins at {@link entry} and
 * follows condition successors until a book leaf. Well-formedness (acyclic, every
 * path terminates at a known enabled book, type-consistent conditions) is validated
 * server-side on write. Consumed by pass 6b (the flow-canvas routing editor).
 */
export interface RiskRoutingGraph {
  /** The node id at which every fill's walk begins. */
  entry: number;
  /** All nodes (id-carrying); the store rebuilds the id→node map from them. */
  nodes: RoutingNode[];
}

// --- Auto-hedging / risk internalisation (docs/AUTO-HEDGING-AND-INTERNALISATION- ---
//    REQUIREMENTS.md) ---------------------------------------------------------
//
// The THIRD risk-lifecycle operation (after routing + transfer): manage WAREHOUSED
// risk against a soft, banded threshold — internalise below the "100", hedge the
// overflow above it — via a trader-composed EXIT-POLICY decision graph. Built on the
// shipped risk-routing graph engine: a `HedgeGraph` reuses {@link RouteOp} /
// {@link RouteValue} verbatim, differing only in that its leaves are EXIT ACTIONS
// ({@link ExitAction}) rather than book targets, and it branches on a per-book
// RISK-STATE ({@link HedgeField}) rather than a per-fill trade snapshot. Every type
// mirrors its `celnet.proto` message field-for-field; the GUI hand-decodes the WS
// JSON so {@link wsCodec} matches the server descriptor codec's snake_case names,
// numeric enum tags and the `condition|action` node oneof. Authoring gates on the
// narrow `hedge` capability × asset.

/**
 * Which risk-state attribute a hedge condition matches (mirrors the wire
 * `HedgeFieldEnum` / `celnet_hedge_routing::HedgeField`, same ordinal order 0..18).
 * `breached` is an ENUM field compared by `== "true"/"false"`; `instrument_id` and
 * `counterparty` are free STRINGs; every other risk number is NUMERIC (its kind is
 * pinned in `lib/hedgeFields.ts`, the mirror of `HedgeField::kind`).
 */
export type HedgeField =
  | "instrument_id"
  | "ccy"
  | "product"
  | "book"
  | "desk"
  | "net_dv01"
  | "net_notional"
  | "net_vega"
  | "net_gamma"
  | "inventory_sign"
  | "threshold"
  | "utilization"
  | "overflow"
  | "breached"
  | "counterparty_toxicity"
  | "inventory_age_secs"
  | "internal_offset_available"
  | "hedge_cost_bp"
  | "counterparty";

/**
 * The execution schedule of an external hedge (mirrors the wire `ExecStyleEnum`,
 * ordinals immediate=0 / worked=1): `immediate` = one clip / back-to-back;
 * `worked` = an Almgren–Chriss-scheduled slice series.
 */
export type ExecStyle = "immediate" | "worked";

/**
 * The budget metric a warehouse threshold caps (mirrors `HedgeMetricEnum`,
 * ordinals dv01=0 / net_notional=1 / net_delta=2 / net_vega=3).
 */
export type HedgeMetric = "dv01" | "net_notional" | "net_delta" | "net_vega";

/**
 * The scope a warehouse threshold binds to (most-specific-wins resolution
 * instrument > book > desk; mirrors `HedgeScopeKindEnum`, ordinals desk=0 /
 * book=1 / instrument=2).
 */
export type HedgeScopeKind = "desk" | "book" | "instrument";

/**
 * How much of a position an exit action targets (mirrors `HedgeSizeKind`,
 * ordinals overflow=0 / full=1 / fixed=2): `overflow` hedges to the band edge,
 * `full` flattens, `fixed` uses the explicit {@link HedgeSize.fixed} magnitude.
 */
export type HedgeSizeKind = "overflow" | "full" | "fixed";

/** A sizing choice (mirrors `HedgeSizeDesc`): a kind + explicit magnitude for `fixed`. */
export interface HedgeSize {
  /** The sizing rule. */
  kind: HedgeSizeKind;
  /** The explicit magnitude — only read when `kind === "fixed"`. */
  fixed: number;
}

/**
 * The kind of exit-action leaf (mirrors `ExitActionKind`, ordinals warehouse=0 /
 * cross_internal=1 / skew=2 / submit_market_order=3 / rfq_out=4 / split=5 /
 * escalate=6).
 */
export type ExitActionKind =
  | "warehouse"
  | "cross_internal"
  | "skew"
  | "submit_market_order"
  | "rfq_out"
  | "split"
  | "escalate";

/**
 * One exit action — a terminal leaf of a {@link HedgeGraph} (mirrors `ExitActionDesc`,
 * a FLAT `kind` discriminant + the union of every arm's fields; only the fields
 * relevant to `kind` are read). See docs §5.3.
 */
export interface ExitAction {
  /** Which exit action this leaf fires. */
  kind: ExitActionKind;
  /** CROSS_INTERNAL: the aggregation instrument to cross against. */
  instrument: string;
  /** CROSS_INTERNAL / SUBMIT_MARKET_ORDER / RFQ_OUT: how much to shed. */
  size: HedgeSize;
  /** SKEW: an explicit skew in bp, or `null` to lean to the band edge (`toEdge`). */
  skewBp: number | null;
  /** SKEW: size the lean off the band-edge overflow rather than `skewBp`. */
  toEdge: boolean;
  /** SUBMIT_MARKET_ORDER / SPLIT: the execution schedule for the externalised leg. */
  style: ExecStyle;
  /** RFQ_OUT: the LPs to fan the request to. */
  lps: string[];
  /** SPLIT: net internally first (vs. externalise first). */
  internalFirst: boolean;
  /** ESCALATE: the rationale surfaced on the notification. */
  reason: string;
}

/**
 * A decision node body: evaluate `field op value` on the risk state; on `true`
 * follow {@link onTrue}, else {@link onFalse} (mirrors `HedgeConditionDesc`;
 * reuses {@link RouteOp} / {@link RouteValue} verbatim). `value` is `null` for a
 * not-yet-set condition (server rejects on write).
 */
export interface HedgeCondition {
  /** The risk-state field to test. */
  field: HedgeField;
  /** The comparison operator. */
  op: RouteOp;
  /** The literal compared against; `null` ⇒ unset. */
  value: RouteValue | null;
  /** Successor node id when the condition holds. */
  onTrue: number;
  /** Successor node id when the condition does not hold. */
  onFalse: number;
}

/**
 * One node in a {@link HedgeGraph}, keyed by its {@link id} (mirrors the wire
 * `HedgeNodeDesc` oneof / `HedgeNode`). Either an internal `condition` test or a
 * terminal `action` leaf carrying its {@link ExitAction}.
 */
export type HedgeNode =
  | { kind: "condition"; id: number; condition: HedgeCondition }
  | { kind: "action"; id: number; action: ExitAction };

/**
 * The firm-wide hedge-policy decision graph (mirrors `HedgeGraphDesc` /
 * `celnet_hedge_routing::HedgeGraph`): the walk begins at {@link entry} and follows
 * condition successors until an action leaf. Well-formedness is validated
 * server-side on write against the aggregation instruments + LP registry.
 */
export interface HedgeGraph {
  /** The node id at which every risk-state walk begins. */
  entry: number;
  /** All nodes (id-carrying); the store rebuilds the id→node map from them. */
  nodes: HedgeNode[];
}

// --- Incoming-quote acceptance (docs — celnet-acceptance) --------------------
//
// The THIRD trader-configurable rule engine (after risk-routing + auto-hedge): a
// first-match decision graph that runs AT ACCEPTANCE (after last-look, before
// booking) on each incoming client LIFT and resolves it to ACCEPT / REJECT /
// HOLD_FOR_REVIEW. Built on the SAME graph engine: an `AcceptanceGraph` reuses
// {@link RouteOp} / {@link RouteValue} verbatim, differing only in that its leaves
// are DECISIONS ({@link AcceptanceAction}) rather than book targets / exit actions,
// and it branches on a per-lift {@link AcceptanceField} snapshot. Every type mirrors
// its `celnet.wire` message field-for-field; the GUI hand-decodes the WS JSON so
// {@link wsCodec} matches the server descriptor codec's snake_case names, numeric
// enum tags and the `condition|decision` node oneof. Authoring gates on the narrow
// `manage_acceptance` capability × asset.

/**
 * Which lift attribute an acceptance condition matches (mirrors the wire
 * `AcceptanceFieldEnum` / `celnet_acceptance::AcceptanceField`, same ordinal order
 * 0..8). `counterparty` / `side` / `asset_class` / `desk` are ENUM fields compared
 * by equality / membership; `instrument_symbol` is a free STRING (supports
 * substring); every other attribute is NUMERIC (its kind is pinned in
 * `lib/acceptanceFields.ts`, the mirror of `AcceptanceField::kind`).
 */
export type AcceptanceField =
  | "counterparty"
  | "notional_usd"
  | "tenor_years"
  | "instrument_symbol"
  | "side"
  | "edge_bps"
  | "quote_age_ms"
  | "asset_class"
  | "desk";

/**
 * The kind of acceptance decision leaf (mirrors `AcceptanceActionKind`, ordinals
 * accept=0 / reject=1 / hold_for_review=2): `accept` books the lift; `reject`
 * declines it with the reason (surfaced on the FIX `Text(58)`); `hold_for_review`
 * routes the lift to the desk inbox for a human to accept manually.
 */
export type AcceptanceActionKind = "accept" | "reject" | "hold_for_review";

/**
 * One acceptance decision — a terminal leaf of an {@link AcceptanceGraph} (mirrors
 * the wire `AcceptanceActionDesc`: a `kind` tag + the `reason` string; the `reason`
 * is read only for `reject` / `hold_for_review`).
 */
export interface AcceptanceAction {
  /** Which decision this leaf fires. */
  kind: AcceptanceActionKind;
  /** REJECT / HOLD_FOR_REVIEW: the rationale surfaced to the counterparty / desk. */
  reason: string;
}

/**
 * A decision node body: evaluate `field op value` on the lift; on `true` follow
 * {@link onTrue}, else {@link onFalse} (mirrors `AcceptanceConditionDesc`; reuses
 * {@link RouteOp} / {@link RouteValue} verbatim). `value` is `null` for a
 * not-yet-set condition (server rejects on write).
 */
export interface AcceptanceCondition {
  /** The lift field to test. */
  field: AcceptanceField;
  /** The comparison operator. */
  op: RouteOp;
  /** The literal compared against; `null` ⇒ unset. */
  value: RouteValue | null;
  /** Successor node id when the condition holds. */
  onTrue: number;
  /** Successor node id when the condition does not hold. */
  onFalse: number;
}

/**
 * One node in an {@link AcceptanceGraph}, keyed by its {@link id} (mirrors the wire
 * `AcceptanceNodeDesc` oneof / `celnet_acceptance::AcceptanceNode`). Either an
 * internal `condition` test or a terminal `decision` leaf carrying its
 * {@link AcceptanceAction}.
 */
export type AcceptanceNode =
  | { kind: "condition"; id: number; condition: AcceptanceCondition }
  | { kind: "decision"; id: number; action: AcceptanceAction };

/**
 * The firm-wide incoming-quote-acceptance decision graph (mirrors
 * `AcceptanceGraphDesc` / `celnet_acceptance::AcceptanceGraph`): the walk begins at
 * {@link entry} and follows condition successors until a decision leaf. The default
 * graph is a single accept-all decision leaf. Well-formedness is validated
 * server-side on write.
 */
export interface AcceptanceGraph {
  /** The node id at which every lift walk begins. */
  entry: number;
  /** All nodes (id-carrying); the store rebuilds the id→node map from them. */
  nodes: AcceptanceNode[];
}

/**
 * A soft, banded warehouse threshold for one scope — the configurable "100"
 * (mirrors `WarehouseThresholdDesc` / `celnet_hedge_routing::WarehouseThreshold`).
 */
export interface WarehouseThreshold {
  /** What the {@link scopeId} names (desk / book / instrument). */
  scopeKind: HedgeScopeKind;
  /** The scope identifier (a desk id / book id / instrument id). */
  scopeId: string;
  /** The budget metric. */
  metric: HedgeMetric;
  /** The budget magnitude — the "100", in the metric's native units. */
  cap: number;
  /** Amber utilisation fraction (start skewing) in [0, red]. */
  amber: number;
  /** Red utilisation fraction (start hedging the overflow) in [amber, 1]. */
  red: number;
  /** Band-edge target as a fraction of cap (default = amber): hedge the overflow to here. */
  targetFraction: number;
  /** Minimum hedge clip (fixed-cost / minimum-ticket floor). */
  minClip: number;
  /** Maximum single hedge clip; a larger overflow is worked. */
  maxClip: number;
  /** Whether to ramp the hedged fraction with utilisation (soft externalisation). */
  ramped: boolean;
  /** The ramp gain k in `hedge_fraction = clamp(k·(utilization − 1), 0, 1)`. */
  rampK: number;
}

/**
 * The immutable audit record stamped on every fired hedge (mirrors `HedgeProvenance`
 * / the `RiskTransferProvenance` discipline). Surfaced on the hedge monitor.
 */
export interface HedgeProvenance {
  /** Stable hedge id (audit key). */
  hedgeId: string;
  /** The book whose risk fired the hedge. */
  book: string;
  /** The instrument hedged. */
  instrument: string;
  /** When it fired (epoch millis, UTC). */
  firedAt: number;
  /** The budget metric that tripped. */
  metric: HedgeMetric;
  /** The resolved threshold ("100"). */
  threshold: number;
  /** The signed net risk at fire. */
  netRisk: number;
  /** `|netRisk| / threshold`. */
  utilization: number;
  /** The RAG band label at fire ("green"/"amber"/"red"/"breach"). */
  band: string;
  /** The exact graph path walked (the "why this action"). */
  policyPath: number[];
  /** The exit action fired, or `null` when absent. */
  action: ExitAction | null;
  /** Crossed against opposing internal flow (the Agg Book). */
  internalCrossed: number;
  /** Externalised onto the RFQ/FIX panel. */
  externalHedged: number;
  /** The unshed residual (warehoused / escalated). */
  residual: number;
  /** The realised hedge price (0 for a no-trade action). */
  hedgePrice: number;
  /** The consolidated mid at fire. */
  midAtFire: number;
  /** Realised slippage in bp versus the mid. */
  slippageBp: number;
  /** The winning external LP, or `null` for internal / no-trade. */
  lpWon: string | null;
  /** Whether this was an ADVISORY (dry-run) fire — computed but not traded. */
  advisory: boolean;
  /**
   * The effective LP set the hedge TARGETED — the resolved include/exclude panel
   * (or, when no scope panel is set, the per-rule RFQ include list / full known
   * panel). Empty for internal / no-trade actions. Answers "which LPs did we hedge
   * on?" (mirrors `HedgeProvenance.lps`, field 20).
   */
  lps: string[];
}

/**
 * The advisory shadow-run projection of one hedge decision — what the engine WOULD
 * do (or did) for a (book × instrument) risk state on a `risk_version` bump
 * (mirrors `HedgeIntent`).
 */
export interface HedgeIntent {
  /** The book whose risk moved. */
  book: string;
  /** The instrument. */
  instrument: string;
  /** The resolved exit action, or `null` when absent. */
  action: ExitAction | null;
  /** The RAG band label ("green"/"amber"/"red"/"breach"). */
  band: string;
  /** The signed net risk. */
  netRisk: number;
  /** The resolved threshold. */
  threshold: number;
  /** `|netRisk| / threshold`. */
  utilization: number;
  /** The overflow beyond the band edge. */
  overflow: number;
  /** The sized hedge magnitude. */
  size: number;
  /** The internal-cross portion of the sized hedge. */
  internalCrossed: number;
  /** The external portion of the sized hedge. */
  externalHedged: number;
  /** Whether the policy is armed advisory-only (no live trading). */
  advisory: boolean;
  /** When resolved (epoch millis, UTC). */
  firedAt: number;
  /** The exact graph path walked. */
  policyPath: number[];
  /** A short human rationale (band / action summary). */
  reason: string;
  /**
   * The effective LP set the hedge would TARGET — the resolved include/exclude panel
   * (or the per-rule RFQ include list / full known panel when no scope panel is set).
   * Empty for internal / no-trade actions. Lets a desk see which LPs an advisory hedge
   * would fan to (mirrors `HedgeIntent.lps`, field 16).
   */
  lps: string[];
}

/** One per-desk enable toggle in the engine config (mirrors `HedgeDeskToggle`). */
export interface HedgeDeskToggle {
  /** The desk id. */
  desk: string;
  /** Whether auto-hedging is enabled for the desk. */
  enabled: boolean;
}

/**
 * A standing per-scope hedging LP panel — the include/exclude liquidity-provider
 * selection every EXTERNAL exit action inherits (mirrors `HedgeLpPanelDesc` /
 * `crate::config::hedge_policy::ScopedLpPanel` + the pure
 * `celnet_hedge_routing::HedgeLpPanel`). Resolution is most-specific-wins across
 * scopes: instrument > book > desk. An empty {@link include} starts from ALL known
 * LPs; {@link exclude} is then subtracted. An unrestricted panel (empty include AND
 * exclude) is the "no restriction" default. The server rejects unknown ids and any
 * panel whose effective set is empty.
 */
export interface HedgeLpPanel {
  /** What the {@link scopeId} names (desk / book / instrument). */
  scopeKind: HedgeScopeKind;
  /** The scope identifier (a desk id / book id / instrument id). */
  scopeId: string;
  /** LPs to start from (empty ⇒ all known LPs); a non-empty include is the base set, in order. */
  include: string[];
  /** LPs to subtract from the base set (the exclude semantics). Each must be a known LP. */
  exclude: string[];
}

/** The auto-hedge engine's global controls (mirrors `HedgeConfigDesc`). */
export interface HedgeConfig {
  /** Global kill-switch: when true, ALL auto-hedging halts (positions warehouse). */
  killSwitch: boolean;
  /** Advisory-only: compute + emit intents/provenance but never trade externally. */
  advisoryOnly: boolean;
  /** Per-desk enable overrides. */
  deskEnabled: HedgeDeskToggle[];
  /** A hard ceiling on any single hedge clip (native metric units). */
  maxClip: number;
  /** Max hedges fired per rate-limit interval (0 ⇒ unbounded). */
  maxHedgesPerInterval: number;
  /** A daily externalised-notional cap (0 ⇒ unbounded). */
  dailyExternalNotionalCap: number;
  /**
   * The standing hedging LP panels — per-scope include/exclude LP selection every
   * external exit action inherits (mirrors `HedgeConfigDesc.lp_panels`, field 7).
   */
  lpPanels: HedgeLpPanel[];
}

/**
 * The traffic-light band for a limit utilization (mirrors the wire `RagBand`,
 * ordinals green=0 / amber=1 / red=2): GREEN when used/limit < 0.8, AMBER in
 * [0.8, 1.0), RED at or above 1.0 (a breach).
 */
export type RagBand = "green" | "amber" | "red";

/**
 * One book's utilization of a single limit cap (mirrors `celnet.wire
 * .LimitUtilizationDesc`): the used magnitude against the configured cap, the
 * ratio, and its RAG band. Only caps present on the book AND computable at this
 * seam are emitted (net/gross notional today).
 */
export interface RiskLimitUtilization {
  /** The metric this cap governs (`net_notional` / `gross_notional`). */
  metric: string;
  /** The book's used magnitude for this metric (net uses |signed sum|). */
  used: number;
  /** The configured cap from the book's {@link RiskLimits}. */
  limit: number;
  /** used / limit as a fraction (`+Infinity` when the cap is 0 and something is used). */
  fraction: number;
  /** The traffic-light band derived from {@link fraction}. */
  band: RagBand;
}

/**
 * One risk book's aggregated risk, rolled up its subtree (mirrors `celnet.wire
 * .RiskBookRiskDesc`). The additive metrics sum the book's own routed positions
 * plus every descendant's. {@link dv01} / {@link pnl} arrive as `null` when NOT yet
 * evaluable at this seam (rates DV01 / mark PnL) — render as "—", never as 0.
 */
export interface RiskBookRisk {
  /** The risk book this row is for (`RiskBook.id`). */
  bookId: string;
  /** The book's human-friendly name (`RiskBook.name`). */
  name: string;
  /** Net (signed sum) base-currency notional across the subtree. */
  netNotional: number;
  /** Gross (sum of absolute) base-currency notional across the subtree. */
  grossNotional: number;
  /** The number of positions rolled into this book (own + descendants'). */
  positionCount: number;
  /** Canonical, premium-excluded delta × notional, summed. */
  delta: number;
  /** Canonical gamma × notional, summed. */
  gamma: number;
  /** Canonical vega (per 1.0 vol) × notional, summed. */
  vega: number;
  /** Canonical theta (per year) × notional, summed. */
  theta: number;
  /** Net DV01 (PV per +1bp), or `null` when not yet evaluated (rates-book seam). */
  dv01: number | null;
  /** Live mark-to-market PnL, or `null` when not yet evaluated (no mark pass here). */
  pnl: number | null;
  /** Per-cap limit utilization for the caps present on the book AND computable now. */
  limits: RiskLimitUtilization[];
}

/**
 * The baseline live risk push over the multiplexed RFS session (`celnet.wire
 * .RiskBookRiskSnapshot`): the SAME rows {@link RiskBookRisk} the poll returns,
 * carried on a subscription instead of a request. `version` is monotonic — a
 * consumer applies a frame only when its `version` exceeds the last applied.
 */
export interface RiskBookRiskStreamSnapshot {
  /** The client-assigned subscription id this snapshot answers. */
  subscriptionId: bigint;
  /** The monotonic per-subscription sequence number of this snapshot (1). */
  sequence: bigint;
  /** Every enabled book's rolled-up risk at this snapshot. */
  books: RiskBookRisk[];
  /** The monotonic risk-state version (advances on every routed fill / book edit). */
  version: number;
  /** Echo of the opening subscribe correlation id, when one was supplied. */
  correlationId?: bigint;
  /** Snapshot time, nanoseconds since the Unix epoch (UTC). */
  epochNanos: bigint;
}

/**
 * A sequenced live risk update (`celnet.wire.RiskBookRiskUpdate`): the full
 * re-rolled book set at the next `version` (a routed fill or an admin book edit).
 */
export interface RiskBookRiskStreamUpdate {
  /** The subscription this update advances. */
  subscriptionId: bigint;
  /** The monotonic per-subscription sequence number (snapshot seq + n). */
  sequence: bigint;
  /** Every enabled book's re-rolled risk at this version. */
  books: RiskBookRisk[];
  /** The monotonic risk-state version (strictly greater than the prior frame's). */
  version: number;
  /** Update time, nanoseconds since the Unix epoch (UTC). */
  epochNanos: bigint;
}

// ---------------------------------------------------------------------------
// Risk transfer — the MANUAL move of EXISTING risk (docs/RISK-TRANSFER-
// REQUIREMENTS.md). The exact complement to risk ROUTING (routing auto-assigns
// NEW fills; transfer moves already-open risk). A trader selects position(s) and
// moves the risk (all or part) to (a) a different risk PORTFOLIO on the SAME desk
// (a light re-attribution — economics unchanged), (b) ANOTHER desk's portfolio
// (an economic internal cross at a transfer price — needs the target desk's
// acceptance), or (c) another TRADER (a hand-off the recipient must accept). This
// block mirrors `celnet.wire.{RiskTransfer,TransferLeg,RiskTransferProvenance,…}`
// / `celnet_risk_transfer::*` field-for-field. The sum-types are carried FLAT
// (oneof-free, per the wire): quantity = `quantityFull` + optional
// `partialNotional`; price = `priceBasis` + optional `agreedPrice`. The GUI
// hand-decodes the WS JSON so {@link wsCodec} matches the server's snake_case
// names and its NUMERIC i32 enum tags (`kind`/`state`/`price_basis`). Initiate /
// accept gate on the NARROW `risk_transfer` capability × the asset class (NOT the
// default trader bundle — an explicit grant, like `administer`); view is open to
// any FI trader. Server enforces approver ≠ initiator (four-eyes) on accept.

/**
 * The kind of move, fixing the economics + the approval model (mirrors
 * `celnet.wire.TransferKind` / `celnet_risk_transfer::TransferKind`). The GUI
 * union names only the meaningful members; the codec offsets the first to proto
 * tag 1 (proto reserves 0 for `TRANSFER_KIND_UNSPECIFIED`).
 * - `RE_ATTRIBUTE` — portfolio → portfolio within the SAME desk; economics
 *   unchanged (a re-stamp of the routing dimension); single-control, books
 *   immediately (no Pending).
 * - `DESK_TO_DESK` — portfolio/desk → ANOTHER desk's portfolio; an economic
 *   internal cross at a transfer price; requires the target desk's acceptance.
 * - `TRADER_TO_TRADER` — trader → trader hand-off; requires recipient acceptance.
 */
export type TransferKind = "RE_ATTRIBUTE" | "DESK_TO_DESK" | "TRADER_TO_TRADER";

/**
 * The transfer lifecycle state (mirrors `celnet.wire.TransferState` /
 * `celnet_risk_transfer::TransferState`). Named members start at proto tag 1
 * (`TRANSFER_STATE_UNSPECIFIED` = 0 is not named). `DRAFT` (being composed) is
 * client-only in practice; the server returns `PENDING` (awaiting acceptance) or
 * `BOOKED` (a re-attribution, applied immediately) from initiate.
 */
export type TransferState =
  | "DRAFT"
  | "PENDING"
  | "ACCEPTED"
  | "REJECTED"
  | "BOOKED"
  | "CANCELLED";

/**
 * Which basis a REQUESTED transfer price resolves to (mirrors
 * `celnet.wire.TransferPriceBasis` / the request-side
 * `celnet_risk_transfer::TransferPrice` discriminant). `AGREED`'s numeric override
 * rides alongside as {@link RiskTransfer.agreedPrice}. `MID`/`MARK_TO_MARKET` are
 * auto-filled read-only from the live composite; `AGREED` requires a reason.
 */
export type TransferPriceBasis = "MID" | "MARK_TO_MARKET" | "AGREED";

/**
 * The basis recorded on a BOOKED transfer's immutable provenance (mirrors
 * `celnet.wire.PriceBasis` / `celnet_risk_transfer::PriceBasis`; the same value
 * set as {@link TransferPriceBasis}, carried on the audit record rather than the
 * request — named distinctly to mirror the domain).
 */
export type PriceBasis = "MID" | "MARK_TO_MARKET" | "AGREED";

/**
 * A pass-through risk vector carried / moved by a transfer (mirrors
 * `celnet.wire.RiskVectorDesc` / `celnet_risk_transfer::RiskVector`). FX-vanilla
 * desks populate the greeks; linear-rates / bond desks populate {@link dv01}.
 * Irrelevant fields are 0 (never `null` — this is a dense numeric vector).
 */
export interface RiskVector {
  /** Sensitivity to a 1bp parallel rate move (linear-rates / bond risk). */
  dv01: number;
  /** Sensitivity to spot (FX-vanilla delta). */
  delta: number;
  /** Sensitivity of delta to spot (FX-vanilla gamma). */
  gamma: number;
  /** Sensitivity to volatility (FX-vanilla vega). */
  vega: number;
  /** Sensitivity to the passage of time (FX-vanilla theta). */
  theta: number;
}

/**
 * The aggregate risk a transfer moves (mirrors `celnet.wire.MovedRiskDesc` /
 * `celnet_risk_transfer::MovedRisk`).
 */
export interface MovedRisk {
  /** Signed base-currency notional moved (+ long risk to target, − short). */
  notionalBase: number;
  /** The summed, fraction-scaled pass-through risk vector of the moved slices. */
  risk: RiskVector;
}

/**
 * One end of a transfer (mirrors `celnet.wire.TransferLeg` /
 * `celnet_risk_transfer::TransferLeg`). The SOURCE carries the selected
 * {@link positionIds}; the TARGET leaves them empty (its `riskBookId` is where the
 * risk arrives). `trader` may be empty on a desk-level target.
 */
export interface TransferLeg {
  /** The risk portfolio (internally a risk-book id). */
  riskBookId: string;
  /** The desk that owns the book. */
  deskId: string;
  /** The trader on this end (may be empty on a desk-level target). */
  trader: string;
  /** The positions selected (populated on the source; empty on the target). */
  positionIds: bigint[];
}

/**
 * The immutable audit record stamped on a BOOKED transfer (mirrors
 * `celnet.wire.RiskTransferProvenance` / `celnet_risk_transfer::
 * RiskTransferProvenance`, the `PricingProvenance` waterfall discipline).
 * Structured, additive, carried on the record — so the blotter/dashboard can show
 * exactly what moved, at what price, by whom, approved by whom.
 */
export interface RiskTransferProvenance {
  /** The transfer's stable slug. */
  transferId: string;
  /** The kind of move. */
  kind: TransferKind;
  /** The authenticated initiator. */
  initiatedBy: string;
  /** Trusted-source initiation timestamp (epoch nanos, UTC). */
  initiatedAt: bigint;
  /** The accepting/approving user (four-eyes); `null` for single-control re-attribution. */
  approver: string | null;
  /** When the decision was made (epoch nanos, UTC); `null` until decided. */
  decidedAt: bigint | null;
  /** The source book the risk left. */
  sourceBookId: string;
  /** The target book the risk arrived in. */
  targetBookId: string;
  /** The positions moved. */
  positionIds: bigint[];
  /** Whether the whole selection moved (Full); else a Partial notional moved. */
  quantityFull: boolean;
  /** The Partial notional magnitude moved; `null` ⇒ Full. */
  partialNotional: number | null;
  /** The numeric transfer price the legs booked at. */
  transferPrice: number;
  /** Which basis that price came from. */
  priceBasis: PriceBasis;
  /** The rationale (required for an agreed price). */
  reason: string;
  /** P&L crystallised in the source at the transfer price versus each slice's mark. */
  realizedPnlSource: number;
  /** The aggregate risk moved from source to target. */
  riskMoved: MovedRisk;
}

/**
 * A risk-transfer request/record (mirrors `celnet.wire.RiskTransfer` /
 * `celnet_risk_transfer::RiskTransfer`). The quantity + price sum-types are
 * carried FLAT: {@link quantityFull} + optional {@link partialNotional};
 * {@link priceBasis} + optional {@link agreedPrice}. `provenance` is stamped on
 * `BOOKED`; `approver`/`decidedAt`/`transferPrice` fill in on the accept/book.
 */
export interface RiskTransfer {
  /** Stable slug (audit key). */
  id: string;
  /** The kind of move (fixes economics + approval). */
  kind: TransferKind;
  /** The source end — the book/desk/trader the risk leaves, with the positions. */
  source: TransferLeg;
  /** The target end — the book/desk/trader the risk arrives in. */
  target: TransferLeg;
  /** Whether the whole selection moves (Full); else {@link partialNotional} moves. */
  quantityFull: boolean;
  /** The Partial notional magnitude to move; `null` ⇒ Full. */
  partialNotional: number | null;
  /** The requested transfer-price basis. */
  priceBasis: TransferPriceBasis;
  /** The agreed override level (present ONLY when {@link priceBasis} is `AGREED`); else `null`. */
  agreedPrice: number | null;
  /** Free-text rationale (required when {@link priceBasis} is `AGREED`). */
  reason: string;
  /** The authenticated user who initiated the transfer. */
  initiatedBy: string;
  /** Trusted-source initiation timestamp (epoch nanos, UTC). */
  initiatedAt: bigint;
  /** The current lifecycle state. */
  state: TransferState;
  /** The accepting/approving user (four-eyes); `null` until decided. */
  approver: string | null;
  /** When the accept/reject decision was made (epoch nanos); `null` until decided. */
  decidedAt: bigint | null;
  /** The resolved numeric transfer price (present once known, on `BOOKED`); else `null`. */
  transferPrice: number | null;
  /** The immutable audit record, stamped on `BOOKED`; `null` until then. */
  provenance: RiskTransferProvenance | null;
}

/**
 * The parameters to initiate a transfer (the ticket → `InitiateRiskTransfer`).
 * `kind` is INFERRED by the ticket from the target selection (same desk ⇒
 * `RE_ATTRIBUTE`; another desk ⇒ `DESK_TO_DESK`; a trader ⇒ `TRADER_TO_TRADER`).
 */
export interface InitiateRiskTransferInput {
  kind: TransferKind;
  source: TransferLeg;
  target: TransferLeg;
  quantityFull: boolean;
  /** The Partial notional to move; `null` ⇒ Full (ignored when `quantityFull`). */
  partialNotional: number | null;
  priceBasis: TransferPriceBasis;
  /** Required (non-null) when `priceBasis` is `AGREED`; else `null`. */
  agreedPrice: number | null;
  /** Required (non-empty) when `priceBasis` is `AGREED`. */
  reason: string;
}

/**
 * Filters for the audit-trail query (`ListRiskTransfers`). Any absent (`null` /
 * empty) filter is unrestricted; the server prunes to what the caller may see.
 */
export interface ListRiskTransfersFilter {
  /** Restrict to this desk (`null` ⇒ all entitled desks). */
  desk: string | null;
  /** Restrict to this trader (`null` ⇒ all). */
  trader: string | null;
  /** Restrict to transfers touching this risk book as source or target (`null` ⇒ all). */
  riskBookId: string | null;
  /** Restrict to these states (empty ⇒ all states). */
  states: TransferState[];
}

// ---------------------------------------------------------------------------
// XVA — counterparty valuation adjustments (`PricingService.PriceXva`).
//
// A netting set of FX vanillas priced for its all-in credit / funding valuation
// adjustments: CVA (counterparty default), DVA (own default), FVA (funding of the
// uncollateralised net expected exposure). The wire request carries the netting
// set, the single-factor exposure-model market (spot / vol / rates + the MC
// path/step budget), the counterparty & own survival (hazard) curves, the two
// LGDs and the funding spread; the wire response (`XvaResult`) carries ONLY the
// four scalar adjustments — the simulated exposure PROFILE (EPE/ENE per time
// bucket) is a server-internal of the estimator and is NOT projected onto the
// contract. Mirrors `celnet.wire.PriceXvaRequest` / `XvaResult` field-for-field.
// ---------------------------------------------------------------------------

/**
 * One trade of an XVA netting set (`celnet.wire.XvaTrade`): a single FX vanilla
 * carried by its payoff terms. `notional` is SIGNED (a negative notional is the
 * opposite direction), so the netting set's value is the signed sum of its trade
 * marks.
 */
export interface XvaTrade {
  optionType: OptionType;
  /** Absolute strike `K` (quote per 1 unit of base); `> 0`. */
  strike: number;
  /** Time to expiry in years; `> 0`. */
  expiryYears: number;
  /** Annualised lognormal volatility of this trade's mark; `> 0`. */
  vol: number;
  /** Signed notional (a negative notional flips the trade direction). */
  notional: number;
}

/**
 * A survival (hazard-rate) curve for one party (`celnet.wire.XvaSurvivalCurve`).
 * Two shapes share the one message: a FLAT curve carries a single `hazardRates`
 * entry and NO `pillarTimes` (constant hazard `λ`, survival `e^{−λt}`); a
 * PIECEWISE-constant curve carries equal-length, strictly-increasing positive
 * `pillarTimes` and their per-segment `hazardRates`. The server validates the
 * shape and rejects a malformed curve.
 */
export interface XvaSurvivalCurve {
  /** Segment end times (years), strictly increasing; EMPTY for the flat curve. */
  pillarTimes: number[];
  /** Per-segment hazard rates (≥ 0); exactly one entry for the flat curve. */
  hazardRates: number[];
}

/**
 * A request to price a netting set's XVA (`celnet.wire.PriceXvaRequest`). The
 * exposure profile is simulated under a single-factor lognormal spot model
 * (`spot0` / `sigma` with carry `rDom − rFor`) over `exposureSteps` buckets to the
 * set's horizon, `paths` Monte-Carlo paths at the given `seed`; the adjustments
 * integrate the discounted expected exposure against each party's marginal default
 * probability (`counterparty` / `own`) scaled by its LGD, plus the funding spread
 * on the net expected exposure.
 */
export interface XvaPricingRequest {
  /** The netting set (at least one trade). */
  trades: XvaTrade[];
  /** Continuously-compounded domestic (quote) rate. */
  rDom: number;
  /** Continuously-compounded foreign (base) rate. */
  rFor: number;
  /** Initial spot `S₀` (quote per 1 unit of base); `> 0`. */
  spot0: number;
  /** Exposure-model annualised volatility `σ` (`≥ 0`). */
  sigma: number;
  /** Monte-Carlo paths for the exposure estimator; `≥ 1`. */
  paths: number;
  /** Counter-RNG seed (identical seeds reproduce the estimate bit-for-bit). */
  seed: number;
  /** Exposure time buckets to the set horizon; `≥ 1`. */
  exposureSteps: number;
  /** The counterparty's survival (hazard) curve — drives CVA. */
  counterparty: XvaSurvivalCurve;
  /** Our own survival (hazard) curve — drives DVA. */
  own: XvaSurvivalCurve;
  /** Counterparty loss-given-default `∈ [0, 1]`. */
  lgdCounterparty: number;
  /** Own loss-given-default `∈ [0, 1]`. */
  lgdOwn: number;
  /** Funding spread over the risk-free rate (absolute, e.g. 0.008 = 80bp). */
  fundingSpread: number;
}

/**
 * The all-in XVA of a netting set (`celnet.wire.XvaResult`): the three adjustment
 * legs plus their signed total. `totalAdjustment = cva − dva + fva` is the amount
 * subtracted from the risk-free value. This is the WHOLE wire result — no exposure
 * profile crosses the contract.
 */
export interface XvaResult {
  /** Credit valuation adjustment (`≥ 0`): expected loss from counterparty default. */
  cva: number;
  /** Debit valuation adjustment (`≥ 0`): expected benefit from own default. */
  dva: number;
  /** Funding valuation adjustment (signed): funding cost/benefit of the net exposure. */
  fva: number;
  /** The all-in adjustment `cva − dva + fva` subtracted from the risk-free value. */
  totalAdjustment: number;
}

// ---------------------------------------------------------------------------
// curve bootstrap from registry-referenced instruments (`AuthService.BuildCurve`).
// Distinct from the slice-A par-OIS-pillar authoring above: here the trader picks
// reference-data instruments (the `ListInstruments` roster) and supplies one
// calibrating quote each; the SERVER resolves every id against the registry,
// rebuilds each schedule from the reference date, runs the sequential bootstrap,
// and returns per-instrument calibrated points (short→long by resolved maturity).
// ---------------------------------------------------------------------------

/**
 * One calibrating quote for a {@link BuildCurveRequest} (`celnet.wire
 * .InstrumentQuote`): a reference-data instrument id paired with its observed
 * market quote. The id resolves against the instrument registry; the quote is the
 * instrument's calibration observable as a decimal (a deposit/FRA/par-swap fixed
 * rate, or a STIR future's `(100 − price) / 100` futures rate).
 */
export interface InstrumentQuote {
  /** The registry instrument id to resolve (e.g. `usd-irs-10y`). */
  instrumentId: string;
  /** The observed calibrating quote as a decimal (0.0405 = 4.05%). */
  quote: number;
}

/**
 * A standalone date-anchored calibration pillar (`celnet.wire.DatePillar`): an
 * explicit maturity date paired with its observed simple ACT/360 rate. The server
 * resolves it to a synthetic money-market cash deposit (`DF = 1/(1 + r·τ)`) from the
 * curve reference date to `maturityDate`, pinning the curve at a date with no
 * registry instrument maturing there (a turn, an IMM, a central-bank meeting).
 */
export interface DatePillar {
  /** The pillar maturity date the synthetic deposit runs to (after the reference date). */
  maturityDate: BrokenDate;
  /** The observed simple ACT/360 rate to that date as a decimal (0.0415 = 4.15%). */
  quote: number;
}

/**
 * A request to bootstrap a single-currency discount curve (`celnet.wire
 * .BuildCurveRequest`). Pillars may arrive in any order — the server orders them by
 * resolved maturity. Calibrating pillars are either registry instruments
 * ({@link pillars}) or standalone date-anchored pillars ({@link datePillars}); at
 * least one pillar across both lists is required.
 */
export interface BuildCurveRequest {
  /** Caller-supplied correlation token, echoed back on the result. */
  requestId: string;
  /** ISO-4217 currency of the curve (the pillar set must be single-currency). */
  currency: string;
  /** The curve reference (spot-anchor) civil date the pillar schedules roll from. */
  referenceDate: BrokenDate;
  /** The calibrating instrument quotes; maturity order not required. */
  pillars: readonly InstrumentQuote[];
  /** Standalone date-anchored pillars, calibrated alongside the instrument pillars. */
  datePillars: readonly DatePillar[];
}

/**
 * One bootstrapped pillar of a {@link CalibratedCurve} (`celnet.wire
 * .CalibratedCurvePoint`): the resolved curve coordinate and discount factor at one
 * calibrating instrument's maturity.
 */
export interface CalibratedCurvePoint {
  /** The input instrument id this pillar calibrates; empty for a date-anchored pillar. */
  instrumentId: string;
  /** The pillar maturity on the ACT/365F curve year-fraction axis. */
  timeYears: number;
  /** The bootstrapped discount factor at `timeYears`. */
  discountFactor: number;
  /** The continuously-compounded zero rate at `timeYears` (decimal). */
  zeroRate: number;
  /**
   * A display label for the pillar: the date-anchored pillar's `Date YYYY-MM-DD`, or
   * empty for an instrument pillar (the client resolves its name by {@link instrumentId}).
   */
  label: string;
}

/**
 * A bootstrapped discount curve (`celnet.wire.CalibratedCurve`): the calibrated
 * pillar points (one per input instrument, ordered short→long by maturity) plus the
 * echoed request header.
 */
export interface CalibratedCurve {
  /** Echoed {@link BuildCurveRequest.requestId}. */
  requestId: string;
  /** ISO-4217 currency of the curve. */
  currency: string;
  /** The curve reference (spot-anchor) date, echoed from the request. */
  referenceDate: BrokenDate;
  /** The bootstrapped pillars, ordered short→long by maturity. */
  points: readonly CalibratedCurvePoint[];
}

// ---------------------------------------------------------------------------
// fixed-income curve query — the FI market-data query surface (ADR-0021: the
// asset-class-agnostic market-data query seam the FX vol surface already has,
// generalized so fixed income rides it too). The discount-curve analogues of the
// options `GetSmile` / `MarkSurface` / `Scenario` verbs: read a marked/bootstrapped
// curve on a tenor axis, pin it under a fresh version, and bump-and-reprice it.
// Mirrors the `celnet.wire` curve-query messages one-to-one.
// ---------------------------------------------------------------------------

/**
 * A queried point on a discount curve (`celnet.wire.CurvePoint`): the
 * continuously-compounded zero rate and the discount factor at `tenorYears` (a year
 * fraction from the curve reference/spot-anchor date). The curve analogue of a
 * {@link SmilePoint}.
 */
export interface CurvePoint {
  /** The tenor (year fraction from the reference date) this point reports. */
  tenorYears: number;
  /** The continuously-compounded zero rate z(t) = −ln DF(t) / t. */
  zeroRate: number;
  /** The discount factor DF(t) = exp(−z(t)·t). */
  discountFactor: number;
}

/**
 * A calibrating par pillar resolved to its final-payment year fraction and observed
 * par (fair fixed) rate (`celnet.wire.CurveParPillar`) — the curve analogue of a
 * `Smile`'s echoed broker quotes.
 */
export interface CurveParPillar {
  /** The resolved final-payment year fraction from spot for this pillar. */
  tenorYears: number;
  /** The observed par (fair fixed) rate as a decimal (0.0405 = 4.05%). */
  parRate: number;
}

/**
 * A read discount curve (`celnet.wire.GetCurveResponse`, `SurfaceService.GetCurve`)
 * — the FI analogue of a {@link Smile}: the currency + reference date, the queried
 * points, the echoed calibrating par pillars, and (when the request pinned one) the
 * marked version it was read from.
 */
export interface GetCurveResult {
  /** ISO-4217 currency of the curve. */
  currency: string;
  /** The curve reference (spot-anchor) date the tenor axis is measured from. */
  referenceDate: BrokenDate;
  /** The queried points (zero rate + discount factor per requested tenor). */
  points: readonly CurvePoint[];
  /** The calibrating par pillars echoed (resolved tenor + par rate). */
  parPillars: readonly CurveParPillar[];
  /** The marked version this was read from — present iff the request pinned one. */
  curveVersion?: bigint;
  /** Read time, nanoseconds since the Unix epoch (UTC). */
  epochNanos: bigint;
}

/**
 * A marked/persisted discount curve under a fresh pinned version
 * (`celnet.wire.MarkCurveResponse`, `SurfaceService.MarkCurve`) — the FI analogue of
 * a {@link MarkedSurface}: a later {@link GetCurveResult} pinned to `curveVersion`
 * reproduces this exact curve.
 */
export interface MarkedCurve {
  /** ISO-4217 currency of the marked curve. */
  currency: string;
  /** The monotonic server-assigned curve version id (a `GetCurve` pins against it). */
  curveVersion: bigint;
  /** The resolved calibrating par pillars marked. */
  parPillars: readonly CurveParPillar[];
  /** The bootstrapped points (zero rate + discount factor) at the pillar tenors. */
  points: readonly CurvePoint[];
  /** Mark time, nanoseconds since the Unix epoch (UTC). */
  epochNanos: bigint;
}

/**
 * The repriced instrument leg of a curve scenario
 * (`celnet.wire.CurveScenarioReprice`): the PV before and after the shift, the
 * resulting PV change, and the base-curve DV01 (PV change per +1bp parallel pillar
 * bump) so a consumer can check `pvChange ≈ dv01 · parallelShiftBp`.
 */
export interface CurveScenarioReprice {
  /** The instrument PV on the unshifted (base) curve. */
  basePv: number;
  /** The instrument PV on the shifted curve. */
  shiftedPv: number;
  /** The PV change (`shiftedPv − basePv`). */
  pvChange: number;
  /** The base-curve DV01 (PV change per +1bp parallel bump), signed per side. */
  dv01: number;
}

/**
 * A bump-and-reprice curve scenario result (`celnet.wire.CurveScenarioResponse`,
 * `SurfaceService.CurveScenario`) — the FI analogue of a vol-surface
 * {@link ScenarioResult}: the shifted curve on the query axis plus the optional
 * repriced instrument leg (present iff the request carried an instrument).
 */
export interface CurveScenarioResult {
  /** ISO-4217 currency of the curve. */
  currency: string;
  /** The shifted curve points (zero rate + discount factor per requested tenor). */
  points: readonly CurvePoint[];
  /** The repriced instrument leg — present iff the request carried an instrument. */
  reprice: CurveScenarioReprice | null;
}

// ---------------------------------------------------------------------------
// fixed-income (rates) portfolio risk — the additive book-level risk rollup
// (`RiskService.AggregateRatesRisk`). Mirrors the `celnet.wire` rates-risk
// messages one-to-one: a `CurveSet` + signed `RatesPosition`s roll up additively
// into one `RatesRiskNode` per settlement currency (netted PV / PV01 / DV01 + a
// tenor-bucketed key-rate DV01 ladder). Purely additive, per-ccy partitioned,
// deterministic — the exact wire analogue of the options `AggregateRisk` path.
// ---------------------------------------------------------------------------

/**
 * One open linear-rates position the rollup nets (`celnet.wire.RatesPosition`):
 * the `(entity, book)` cell it books into plus the `OisInstrument` to price. The
 * instrument carries its own signed direction (PAY_FIXED / RECEIVE_FIXED), so the
 * priced PV / PV01 / DV01 already net by sign across long and short books.
 */
export interface RatesPosition {
  /** Stable position identity (the pricer's `request_id` echo); informational. */
  positionId: bigint;
  /** The legal-entity id the position books into (a scope filter dimension). */
  entity: number;
  /** The trading-book id the position books into (a scope filter dimension). */
  book: number;
  /** The OIS to price against the request `curveSet` (the only P0 arm). */
  instrument: OisInstrument;
}

/**
 * The optional `(entity, book, ccy)` filter applied BEFORE the rollup
 * (`celnet.wire.RatesRiskScope`): each present field narrows the contributing
 * positions; an absent field does not constrain. `ccy` matches case-insensitively.
 */
export interface RatesRiskScope {
  /** Keep only positions in this legal entity, when set. */
  entity?: number;
  /** Keep only positions in this trading book, when set. */
  book?: number;
  /** Keep only positions whose settlement currency matches, when set. */
  ccy?: string;
}

/**
 * `RiskService.AggregateRatesRisk` request — price every `RatesPosition` against
 * the shared `curveSet`, narrow by the optional `scope`, then sum additively into
 * one `RatesRiskNode` per settlement currency. The market is the request-supplied
 * `curveSet`, so the rollup is a pure, deterministic calculation.
 */
export interface AggregateRatesRiskRequest {
  /** The calibrated curve set every position prices against (the shared market). */
  curveSet: RatesCurveSet;
  /** The positions to net; empty ⇒ an empty rollup. */
  positions: readonly RatesPosition[];
  /** The optional pre-rollup `(entity, book, ccy)` filter. */
  scope?: RatesRiskScope;
  /** Entitlement principal; omitted ⇒ the audited explicit grant-all default. */
  principal?: EntitlementPrincipal;
  /** Optional client correlation echo. */
  correlationId?: bigint;
}

/**
 * One tenor bucket of a node's key-rate DV01 ladder (`celnet.wire.KeyRateDv01`):
 * the netted PV change for a +1bp bump of the curve pillar at `tenorYears` alone.
 */
export interface KeyRateDv01 {
  /** The curve pillar tenor (whole years) this bucket bumps. */
  tenorYears: number;
  /** The netted DV01 contribution at this pillar (curve currency). */
  dv01: number;
}

/**
 * The netted risk of one settlement currency (`celnet.wire.RatesRiskNode`): the
 * additively summed PV / PV01 / DV01 across every contributing position, plus the
 * per-pillar key-rate DV01 ladder (which sums to `netDv01` to first order).
 */
export interface RatesRiskNode {
  /** ISO-4217 settlement currency of this node (the rollup partition key). */
  ccy: string;
  /** Summed present value across the node's positions (curve currency). */
  netPv: number;
  /** Summed analytic PV01 across the node's positions. */
  netPv01: number;
  /** Summed parallel DV01 across the node's positions. */
  netDv01: number;
  /** The tenor-bucketed key-rate DV01 ladder, in ascending pillar order. */
  keyRateLadder: readonly KeyRateDv01[];
}

/**
 * `RiskService.AggregateRatesRisk` response — one `RatesRiskNode` per settlement
 * currency, in ascending-currency order.
 */
export interface AggregateRatesRiskResponse {
  nodes: readonly RatesRiskNode[];
  correlationId?: bigint;
}

// ---------------------------------------------------------------------------
// combined options+FI JOINT tail risk (`RiskService.CombinedTailRisk`) — the C2c
// "true single VaR engine". ONE non-additive cube over a portfolio's vanilla FX
// option legs AND its linear-FI (OIS-swap) legs, by full joint bump-and-revalue
// over aligned (options-shock, rate-shock) scenarios, plus the FI signed key-rate
// DV01 ladder and signed parallel DV01. Mirrors `celnet.wire` field-for-field
// (`crates/celnet-proto/proto/celnet.proto`); the whole portfolio + scenario
// config travels INLINE on the request (a pure calculation — no store/market
// read), so every replica computes the identical result. Options-only ⇒ empty
// `fiPositions` (reduces to the options VaR); FI-only ⇒ empty `optionLegs`
// (reduces to the rate VaR); a mixed book shows the joint cross-risk-class
// diversification.
// ---------------------------------------------------------------------------

/**
 * One vanilla FX option leg of the joint tail-risk portfolio
 * (`celnet.wire.TailRiskOptionLeg`): the raw Garman-Kohlhagen inputs plus quoted
 * conventions — exactly the arguments of the server's `PositionRisk::fx`. The
 * canonical convention-free risk is re-derived server-side from these inputs; the
 * carry rides as flat `rDom`/`rFor` (the FX two-rate carry).
 */
export interface TailRiskOptionLeg {
  /** The currency pair (BASE/QUOTE). */
  pair: CcyPair;
  /** Call or put on the base currency. */
  optionType: OptionType;
  /** Signed base-currency notional (positive = long the option). */
  notionalBase: number;
  /** Spot FX rate (quote per 1 unit of base). */
  spot: number;
  /** Strike (quote per 1 unit of base). */
  strike: number;
  /** Annualized volatility (absolute, e.g. 0.10 = 10 vol). */
  vol: number;
  /** Time to expiry in years (vol-time). */
  t: number;
  /** Continuously-compounded domestic (quote / numeraire) rate `r_dom`. */
  rDom: number;
  /** Continuously-compounded foreign (base) rate `r_for`. */
  rFor: number;
  /** The delta convention the leg was quoted under (provenance only). */
  quotedDelta: DeltaConvention;
  /** The premium style the leg was quoted under (provenance only). */
  premiumStyle: PremiumStyle;
}

/**
 * One accrual period of an OIS fixed leg in curve year-fraction coordinates
 * (`celnet.wire.OisFixedPeriod`, mirrors `celnet_rates::FixedPeriod`).
 */
export interface TailRiskOisFixedPeriod {
  /** Payment time (period end) in curve year-fraction coordinates (strictly increasing). */
  pay: number;
  /** Year-fraction accrual for the period (e.g. ACT/360; strictly positive). */
  accrual: number;
}

/**
 * A fixed-vs-OIS swap leg priced off the (shocked) discount curve
 * (`celnet.wire.OisSwapLeg`, mirrors `celnet_rates_risk::FiPosition::OisSwap`).
 * Valued by the self-discounting OIS identity `N·(K·A − (DF(start) − DF(mat)))`,
 * the receive-fixed sign, negated for pay-fixed.
 */
export interface TailRiskOisSwap {
  /** The swap effective (start) time in curve year-fraction coordinates. */
  start: number;
  /** The ordered fixed-leg accrual periods (the final period's `pay` is maturity). */
  periods: readonly TailRiskOisFixedPeriod[];
  /** The fixed rate `K` paid/received (decimal, e.g. 0.033). */
  fixedRate: number;
  /** The swap notional `N` in curve-currency units (positive). */
  notional: number;
  /** True to receive fixed (the raw OIS-PV sign), false to pay fixed (negated). */
  receiveFixed: boolean;
}

/**
 * One linear-FI position of the joint tail-risk portfolio
 * (`celnet.wire.TailRiskFiPosition`): a oneof mirroring
 * `celnet_rates_risk::FiPosition`; the swap arm is wired now (the cash-bond arm
 * grows additively — one contract, no versioning).
 */
export interface TailRiskFiPosition {
  /** The fixed-vs-OIS swap arm (the only wired arm today). */
  oisSwap: TailRiskOisSwap;
}

/**
 * One dated pillar of the base discount curve the FI legs reprice off
 * (`celnet.wire.TailRiskCurvePillar`): a `(time, continuously-compounded zero
 * rate)` knot. The curve origin `(0, DF=1)` is implicit.
 */
export interface TailRiskCurvePillar {
  /** The pillar time in curve year-fraction coordinates (strictly increasing, > 0). */
  t: number;
  /** The continuously-compounded zero rate at `t`. */
  zeroRate: number;
}

/**
 * One joint cross-risk-class scenario (`celnet.wire.JointTailScenario`, mirrors
 * `celnet_risk_cube::JointScenario`): an options spot/vol/carry shock applied to
 * the option legs AND a per-pillar rate shock applied to the FI legs, as ONE
 * market state (aligned by scenario index).
 */
export interface JointTailScenario {
  /** Relative spot shock (0.01 = +1%): `spot *= 1 + spotRel`. */
  spotRel: number;
  /** Absolute vol shock in vol units (0.01 = +1 vol point): `vol += volAbs`. */
  volAbs: number;
  /** Absolute discount-rate shock `Δr` (the numeraire rate; `r_dom` for FX). */
  discountAbs: number;
  /** Absolute net-carry shock `Δb` (`r_dom − r_for` for FX). */
  carryAbs: number;
  /**
   * Per-pillar absolute additive zero-rate shifts, aligned with `baseCurve` (the
   * `RateShock`). Its length must equal the base-curve pillar count when FI legs
   * are present; ignored (and may be empty) when there are no FI legs.
   */
  rateShifts: readonly number[];
}

/**
 * A value-at-risk / expected-shortfall pair (`celnet.wire.VarEs`, mirrors
 * `celnet_risk_cube::VarEs`): the alpha-quantile loss and the mean tail loss
 * beyond it (both non-negative loss magnitudes).
 */
export interface VarEs {
  /** Value-at-Risk: the `alpha`-quantile loss. */
  var: number;
  /** Expected Shortfall: the mean loss in the tail beyond VaR. */
  es: number;
}

/**
 * One point of the FI signed key-rate DV01 ladder
 * (`celnet.wire.TailRiskKeyRate`, mirrors `celnet_rates_risk::KeyRatePoint`): the
 * signed ΔPV per +1bp up-bump of one curve pillar's zero rate. The tenor is a
 * real pillar time (fractional tenors allowed).
 */
export interface TailRiskKeyRate {
  /** The pillar tenor in years. */
  tenorYears: number;
  /** The signed DV01 at this tenor (a rate rise is a loss ⇒ negative for a long bond / receiver). */
  dv01: number;
}

/**
 * `RiskService.CombinedTailRisk` request — one-cube joint options+FI tail risk
 * over an inline portfolio + scenario config.
 */
export interface CombinedTailRiskRequest {
  /** The vanilla FX option legs (may be empty for an FI-only request). */
  optionLegs: readonly TailRiskOptionLeg[];
  /** The linear-FI legs (may be empty for an options-only request). */
  fiPositions: readonly TailRiskFiPosition[];
  /**
   * The base discount-curve zero-rate pillars the rate shocks perturb and the FI
   * key-rate ladder is measured off (required, ≥ 1 pillar, strictly increasing in
   * time). Present even for an options-only request (the FI ladder is then empty).
   */
  baseCurve: readonly TailRiskCurvePillar[];
  /** The aligned joint scenarios (one market state each) reduced to the tail. */
  scenarios: readonly JointTailScenario[];
  /** The VaR/ES confidence level (e.g. 0.99). Defaults to 0.99 when 0/absent. */
  alpha?: number;
  /** Optional caller correlation id, echoed on the response. */
  correlationId?: bigint;
}

/**
 * `RiskService.CombinedTailRisk` response — the joint tail plus the FI key-rate
 * axis and signed parallel DV01 (mirrors `celnet_risk_cube::CombinedTailRisk`).
 */
export interface CombinedTailRiskResponse {
  /** The joint options-spot/vol + FI-rate tail — one non-additive VaR/ES over the union. */
  jointVarEs: VarEs;
  /**
   * The FI per-tenor signed key-rate DV01 ladder, ascending by pillar tenor (empty
   * when there are no FI legs; one point per base-curve pillar otherwise).
   */
  keyRate: readonly TailRiskKeyRate[];
  /**
   * The FI signed parallel DV01 (a rate rise is a loss ⇒ negative for a long bond
   * / receiver; 0 when there are no FI legs).
   */
  fiParallelDv01: number;
  /** Echo of the request's `correlationId`, if supplied. */
  correlationId?: bigint;
}

// ---------------------------------------------------------------------------
// dealer-quoting desk + linear-rates Book/List + notification push. The WS
// mirror of RfqDeskService / RiskService(BookRatesPosition, ListRatesPositions)
// / NotificationService (crates/celnet-proto/proto/celnet.proto). One current
// contract (CLAUDE.md rule 9): a desk request carries the SAME `OisInstrument`
// + `RatesCurveSet` the `price_rates` path prices, so a desk RFQ is priced by
// the identical engine the `priceRates` seam exposes.
// ---------------------------------------------------------------------------

/**
 * The flavour of an inbound dealer request (`celnet.wire.DeskRequestKind`,
 * proto RFQ=1 / IOI=2): an `RFQ` is a firm request-for-quote the desk responds
 * to with a price; an `IOI` is an indication-of-interest (an advertised axe the
 * desk may also price). Purpose-named, vendor-neutral.
 */
export type DeskRequestKind = "RFQ" | "IOI";

/**
 * The lifecycle state of a `DeskRequest` (`celnet.wire.DeskRequestState`, proto
 * PENDING=1 … WITHDRAWN=6). A request is `PENDING` on receipt, becomes `QUOTED`
 * when the desk prices it or `REJECTED` when the desk declines, `ACCEPTED` when
 * the counterparty lifts the quote (booking a deal), and `EXPIRED`/`WITHDRAWN`
 * on timeout / counterparty pull.
 */
export type DeskRequestState =
  "PENDING" | "QUOTED" | "ACCEPTED" | "REJECTED" | "EXPIRED" | "WITHDRAWN";

/**
 * The kind of a push `Notification` (`celnet.wire.NotificationKind`, proto
 * RFQ_RECEIVED=1 … QUOTE_REJECTED=6, MANUAL_INTERVENTION_REQUIRED=7,
 * ORDER_RECEIVED=8, FILL=9). The `*_RECEIVED` kinds signal a new inbound request
 * requiring desk attention; the lifecycle kinds report a request's resolution;
 * `MANUAL_INTERVENTION_REQUIRED` (added by the server exception contract, commit
 * 542e547) signals the auto-pricer could not handle a request and the desk must
 * step in — its {@link Notification.reason} names why. `ORDER_RECEIVED` (a firm
 * order landed) and `FILL` (an own execution booked) are the phase-5 arms — a
 * FIX-venue firm-order lift emits both, distinct from `QUOTE_ACCEPTED` (an
 * explicit desk-quote lift over the platform).
 */
export type NotificationKind =
  | "RFQ_RECEIVED"
  | "IOI_RECEIVED"
  | "REQUEST_WITHDRAWN"
  | "REQUEST_EXPIRED"
  | "QUOTE_ACCEPTED"
  | "QUOTE_REJECTED"
  | "MANUAL_INTERVENTION_REQUIRED"
  | "ORDER_RECEIVED"
  | "FILL";

/**
 * Why the server raised a `MANUAL_INTERVENTION_REQUIRED` notification
 * (`celnet.wire.ManualInterventionReason`, proto UNCONFIGURED_TENOR=1,
 * CREDIT_RISK_BREAK=2, UNKNOWN_SECURITY=3, PRICING_FAILURE=4). Carried on the
 * wire `reason` field and meaningful ONLY when the notification `kind` is
 * `MANUAL_INTERVENTION_REQUIRED`.
 */
export type ManualInterventionReason =
  | "UNCONFIGURED_TENOR"
  | "CREDIT_RISK_BREAK"
  | "UNKNOWN_SECURITY"
  | "PRICING_FAILURE";

/**
 * The desk's response to an RFQ/IOI (`celnet.wire.DeskQuote`): the quoted
 * `price` (a fixed rate for an OIS), the quoted `notional`, the quote's validity
 * window `validForMs`, and the `trader` seat that priced it.
 */
export interface DeskQuote {
  /** The quoted level — a fixed rate for an OIS (decimal, 0.041 = 4.10%). */
  price: number;
  /** The notional the quote is good for (curve currency). */
  notional: number;
  /** The quote's last-look validity window in milliseconds. */
  validForMs: number;
  /** The trader seat that priced the quote. */
  trader: string;
}

/** A desk's decline of an RFQ/IOI (`celnet.wire.DeskReject`): a free-text reason. */
export interface DeskReject {
  reason: string;
}

/**
 * The desk's view of one inbound dealer request (`celnet.wire.DeskRequest`): the
 * `OisInstrument` + `RatesCurveSet` to price, the `side`/`notional` requested, its
 * lifecycle `state`, and (once priced) the `quote`. Timestamps are nanoseconds
 * since the Unix epoch (UTC).
 */
export interface DeskRequest {
  /** Stable request identity (server-minted). */
  requestId: string;
  kind: DeskRequestKind;
  /** The counterparty that originated the request. */
  counterparty: string;
  /** The desk the request is routed to. */
  desk: string;
  /** The OIS to price (the P0 rates arm). */
  instrument: OisInstrument;
  /** The curve set the request prices against. */
  curveSet: RatesCurveSet;
  /** The direction requested (BUY = pay fixed / receive; per the wire `Side`). */
  side: Side;
  /** The requested notional (curve currency). */
  notional: number;
  /** Receipt timestamp, nanoseconds since the Unix epoch (UTC). */
  receivedAtNanos: bigint;
  /** Expiry deadline, nanoseconds since the Unix epoch (UTC). */
  expiresAtNanos: bigint;
  state: DeskRequestState;
  /** The desk's quote once priced (absent while `PENDING`/`REJECTED`). */
  quote?: DeskQuote;
  /** Optional client correlation echo. */
  correlationId?: string;
}

/**
 * The DV01-utilisation band of a lifted FI fill vs the internal warehouse cap,
 * as stamped by the server's internalise strategy: `green` (comfortable), `amber`
 * (approaching the cap), `red` (at/over the cap), `breach` (hard limit exceeded).
 * Tinted through the existing bid/warn/offer/danger semantic tokens.
 */
export type HedgeBand = "green" | "amber" | "red" | "breach";

/**
 * The internalise / auto-hedge provenance the server stamps on an FI lift
 * (`celnet.wire.Deal.internalise`): the outcome of running the internalise
 * strategy on the fill. Present ONLY for FI lifts that ran the evaluation —
 * absent (undefined) on every other deal, never fabricated.
 */
export interface Internalise {
  /** `true` = warehoused the fill from risk; `false` = went external/back-to-back. */
  internalised: boolean;
  /** DV01 warehoused internally. */
  internalDv01: number;
  /** DV01 shed to external back-to-back (advisory). */
  externalDv01: number;
  /** Dealer captured edge vs engine mid, in bp (can be negative = losing). */
  edgeBps: number;
  /** `true` when `edgeBps` clears the min-edge floor (are we making money). */
  withinTolerance: boolean;
  /** DV01-utilisation band vs the warehouse cap. */
  hedgeBand: HedgeBand;
}

/**
 * The rates product FAMILY a booked {@link Deal} carries — the discriminant of the
 * wire `RatesInstrument` oneof arm the deal dealt (`ois | irs | fra | bond`). Threaded
 * onto every `Deal` so the blotter's Product column and the Risk Dashboard's
 * "By product type" breakdown classify a fill by its real arm rather than assuming OIS.
 * The full instrument economics still project onto {@link Deal.instrument} (the OIS
 * shape) for the shared tenor/notional/direction reads; `productKind` is the honest
 * arm tag alongside it.
 */
export type RatesProductKind = "OIS" | "IRS" | "FRA" | "BOND";

/**
 * A booked received deal (`celnet.wire.Deal`): the executed terms of an
 * `ACCEPTED` desk request — the dealt `price`, `notional`, `side`, the booking
 * `trader`, and (when the deal booked a rates position) its `positionId`.
 */
export interface Deal {
  dealId: string;
  /** The originating `DeskRequest.requestId`. */
  requestId: string;
  kind: DeskRequestKind;
  counterparty: string;
  desk: string;
  /**
   * The dealt rates product family — which arm of the wire `RatesInstrument` oneof
   * this deal dealt (`OIS`/`IRS`/`FRA`/`BOND`). Decoded from the actual arm present;
   * the tenor/notional/direction still project onto {@link instrument} for the shared
   * reads, but this preserves the arm discriminant (dropped by the OIS-only projection).
   */
  productKind: RatesProductKind;
  instrument: OisInstrument;
  curveSet: RatesCurveSet;
  side: Side;
  notional: number;
  /** The dealt level — the accepted `DeskQuote.price`. */
  price: number;
  /** Execution timestamp, nanoseconds since the Unix epoch (UTC). */
  executedAtNanos: bigint;
  trader: string;
  /** The booked `RatesPosition.positionId`, when the deal booked one. */
  positionId?: bigint;
  correlationId?: string;
  /**
   * The Risk Portfolio (risk book) id the fill's risk routed into, when a firm-wide
   * risk-routing graph is installed. Absent when the fill routed to no portfolio (no
   * graph configured, or a routing fall-back) — never fabricated.
   */
  riskBookId?: string;
  /**
   * The internalise / auto-hedge provenance the server stamped when this fill ran
   * the internalise strategy — present ONLY for FI lifts that ran the evaluation;
   * absent (undefined) on every other deal.
   */
  internalise?: Internalise;
}

/**
 * A server→client push event (`celnet.wire.Notification`): a desk-attention
 * signal (a new RFQ/IOI), a lifecycle resolution (accepted/rejected/expired), or
 * a manual-intervention exception, carrying a human `headline` (+ optional
 * `detail`) and the `requestId` it concerns.
 */
export interface Notification {
  notificationId: string;
  kind: NotificationKind;
  /** Event timestamp, nanoseconds since the Unix epoch (UTC). */
  atNanos: bigint;
  /** The `DeskRequest.requestId` this notification concerns (when applicable). */
  requestId?: string;
  desk: string;
  counterparty: string;
  requestKind: DeskRequestKind;
  headline: string;
  detail?: string;
  /**
   * Wire `alert_worthy` (server exception contract, commit 542e547). The server's
   * authoritative decision on whether this event warrants a POPUP: `true` ⇒ toast
   * / desktop growl / sound; `false` ⇒ the notification lands QUIETLY in the
   * notification centre / blotter only (still stored + counted, no popup, no
   * sound). ALL client-side popup escalation gates on this flag.
   */
  alertWorthy: boolean;
  /**
   * Wire `reason` — the {@link ManualInterventionReason} enum ordinal. Present and
   * meaningful ONLY when `kind === "MANUAL_INTERVENTION_REQUIRED"`; absent /
   * undefined for every other kind.
   */
  reason?: ManualInterventionReason;
}

// --- firm-wide pricing kill-switch (server `pricing_control`) ----------------

/**
 * The firm-wide pricing kill-switch state — the server's `set_pricing_control`
 * committed value + the unsolicited `pricing_control` push. Two independent gates:
 *   - `outboundEnabled` — whether the edge QUOTES FIX-connected clients (ESP/RFQ
 *     outbound). `false` ⇒ ALL outbound pricing to clients is halted (the desk may
 *     still receive inbound LP aggregated pricing into the books).
 *   - `inboundEnabled`  — whether inbound LP liquidity is AGGREGATED into the
 *     books. `false` ⇒ inbound aggregation is stopped too.
 * `version` is the server's MONOTONIC counter: a client reconciles push frames by
 * it (ignore a frame whose `version <=` the last seen) so a stale / out-of-order
 * push never regresses the displayed state. Mutating it is gated on
 * `ManageLiquidity·FixedIncome`; the resulting state is broadcast to EVERY client
 * (so the halted banner shows to all users, operable only by the entitled few).
 */
export interface PricingControl {
  outboundEnabled: boolean;
  inboundEnabled: boolean;
  version: number;
}

// --- desk request/response messages -----------------------------------------

/**
 * `RfqDeskService.SubmitDeskRequest` request — inject an inbound RFQ/IOI. The desk
 * enqueues it `PENDING` and pushes a `*_RECEIVED` notification.
 */
export interface SubmitDeskRequestRequest {
  kind: DeskRequestKind;
  counterparty: string;
  desk: string;
  instrument: OisInstrument;
  curveSet: RatesCurveSet;
  side: Side;
  notional: number;
  /** The request's time-to-live in milliseconds (0 ⇒ a server default). */
  ttlMs: number;
  /** Entitlement principal; omitted ⇒ the audited explicit grant-all default. */
  principal?: EntitlementPrincipal;
  correlationId?: string;
}

export interface SubmitDeskRequestResponse {
  request: DeskRequest;
}

/** The oneof arm a `RespondDeskRequest` carries: a quote OR a reject. */
export type DeskResponseArm =
  { kind: "quote"; quote: DeskQuote } | { kind: "reject"; reject: DeskReject };

/**
 * `RfqDeskService.RespondDeskRequest` request — the desk's response to a
 * `PENDING` request: quote it (→ `QUOTED`) or reject it (→ `REJECTED`).
 */
export interface RespondDeskRequestRequest {
  requestId: string;
  response: DeskResponseArm;
  principal?: EntitlementPrincipal;
  correlationId?: string;
}

export interface RespondDeskRequestResponse {
  request: DeskRequest;
}

/**
 * `RfqDeskService.AcceptDeskQuote` request — the counterparty lifts a `QUOTED`
 * request, booking a `Deal` (+ a `RatesPosition`) and moving it to `ACCEPTED`.
 */
export interface AcceptDeskQuoteRequest {
  requestId: string;
  principal?: EntitlementPrincipal;
  correlationId?: string;
}

export interface AcceptDeskQuoteResponse {
  deal: Deal;
  request: DeskRequest;
}

/** The optional `(states, desk)` filter on a `ListDeskRequests` query. */
export interface DeskRequestScope {
  /** Keep only requests in these states, when non-empty. */
  states?: DeskRequestState[];
  /** Keep only requests routed to this desk, when set. */
  desk?: string;
}

export interface ListDeskRequestsRequest {
  scope?: DeskRequestScope;
  principal?: EntitlementPrincipal;
  correlationId?: string;
}

export interface ListDeskRequestsResponse {
  requests: DeskRequest[];
}

/** The optional `(desk)` filter on a `ListDeals` query. */
export interface DealScope {
  desk?: string;
}

export interface ListDealsRequest {
  scope?: DealScope;
  principal?: EntitlementPrincipal;
  correlationId?: string;
}

export interface ListDealsResponse {
  deals: Deal[];
}

// --- linear-rates Book/List (RiskService rates Book) ------------------------

/**
 * `RiskService.BookRatesPosition` request — book one open `RatesPosition` into the
 * desk's in-memory rates book (what `ListRatesPositions` reads and the rates Book
 * workspace renders).
 */
export interface BookRatesPositionRequest {
  position: RatesPosition;
  principal?: EntitlementPrincipal;
  correlationId?: string;
}

export interface BookRatesPositionResponse {
  position: RatesPosition;
}

/**
 * `RiskService.ListRatesPositions` request — the booked rates positions, narrowed
 * by the optional `(entity, book, ccy)` scope.
 */
export interface ListRatesPositionsRequest {
  scope?: RatesRiskScope;
  principal?: EntitlementPrincipal;
  correlationId?: string;
}

export interface ListRatesPositionsResponse {
  positions: RatesPosition[];
}

// --- notification push stream -----------------------------------------------

/** The desks a notification subscription scopes to (`celnet.wire.NotificationScope`). */
export interface NotificationScope {
  desks: string[];
}

/**
 * `NotificationService.StreamNotifications` request — open the dedicated push
 * stream, optionally scoped to a set of desks.
 */
export interface StreamNotificationsRequest {
  scope?: NotificationScope;
}

// --- bond corporate actions (CorporateActionsService) -----------------------
//
// The bond corporate-action surface — the effective post-CA instrument schedule
// read, the CA inbox read, and the confirm/apply lifecycle. Messages are FLAT and
// civil dates are ISO `YYYY-MM-DD` strings (the wire carries them verbatim);
// `caev`/`camv`/`status` ride as their proto enum tags (see `data/enums.ts`). The
// reads sit on the `view` floor; confirm/apply require the dedicated `refdata`
// capability (gated in the workspace + enforced server-side).

/**
 * CAEV — the corporate-action event type (proto `CorpEventType`, tags 1..10; the
 * proto3 `0`/`UNSPECIFIED` sentinel is unnamed here). The govvie-deterministic set
 * (REDM/INTR/MCAL/PCAL/PRED/DRAW/BPUT) is derivable in-house; the corporate set
 * (TEND/EXOF/CONV) is sourced only via a customer-wired vendor adapter.
 */
export type CorpEventType =
  | "REDM"
  | "INTR"
  | "MCAL"
  | "PCAL"
  | "PRED"
  | "DRAW"
  | "BPUT"
  | "TEND"
  | "EXOF"
  | "CONV";

/**
 * CAMV — the mandatory / voluntary indicator that drives the election lifecycle
 * (proto `CorpMandatory`, tags 1..3): MAND applies automatically; VOLU opens a
 * holder-optional election; CHOS must respond, choosing among outcomes.
 */
export type CorpMandatory = "MAND" | "VOLU" | "CHOS";

/**
 * The lifecycle status of a corporate action (proto `CorpActionStatus`, tags 1..6):
 * `ANNOUNCED → (ELECTED) → CONFIRMED → APPLIED`, plus `REVERSED` / `CANCELLED`. The
 * proto3 `0`/`UNSPECIFIED` sentinel is unnamed.
 */
export type CorpActionStatus =
  | "ANNOUNCED"
  | "ELECTED"
  | "CONFIRMED"
  | "APPLIED"
  | "REVERSED"
  | "CANCELLED";

/**
 * A normalized corporate-action event as stored in the golden source
 * (`celnet.wire.CorporateActionDesc`), flattened for the CA inbox. Dates are ISO
 * `YYYY-MM-DD` strings; `responseDeadline` is present only for a VOLU/CHOS election.
 */
export interface CorporateAction {
  /** The stable id of the corporate action (shared across its lifecycle versions). */
  caId: string;
  /** The ISO 6166 ISIN of the affected instrument (the join key to the schedule). */
  isin: string;
  /** The event type. */
  caev: CorpEventType;
  /** The mandatory / voluntary indicator. */
  camv: CorpMandatory;
  /** The current lifecycle status. */
  status: CorpActionStatus;
  /** Announcement date (ISO `YYYY-MM-DD`). */
  announcementDate: string;
  /** Record date — the balance snapshot that fixes entitlement. */
  recordDate: string;
  /** Ex date — from which the security trades without the entitlement. */
  exDate: string;
  /** Response / market deadline for a VOLU/CHOS election (absent for MAND). */
  responseDeadline?: string;
  /** Payment / effective date — when the movement settles and the effect applies. */
  paymentDate: string;
  /** Cash per 100 face of a redemption / call / put / tender (or coupon cash). */
  cashPer100: number;
  /** Fraction of outstanding nominal redeemed by a partial event, in [0, 1]. */
  redeemedFraction: number;
  /** Exchange / conversion target instrument id (EXOF / CONV); empty otherwise. */
  targetInstrument: string;
  /** Units of the target created per 100 face of the source; 0 otherwise. */
  targetUnitsPer100: number;
  /** Originating source-message reference (MT 564 / seev.031 id) for audit lineage. */
  sourceRef: string;
  /** The source-priority the mastering provenance stamped (survivorship ordering). */
  sourcePriority: number;
  /** The source label that produced this version (lineage). */
  source: string;
}

/**
 * One dated cashflow of the effective (post-any-applied-CA) bond schedule, in
 * per-100-original-face cash (`celnet.wire.InstrumentScheduleFlow`).
 */
export interface InstrumentScheduleFlow {
  /** The payment date, ISO `YYYY-MM-DD`. */
  date: string;
  /** Coupon / interest cash on this date, per 100 face. */
  coupon: number;
  /** Principal / redemption cash on this date, per 100 face (0 except a redemption). */
  principal: number;
}

/** `CorporateActionsService.ListInstrumentSchedule` request. */
export interface ListInstrumentScheduleRequest {
  /** The reference-data instrument id whose schedule to resolve. */
  instrumentId: string;
}

/** The effective (post-any-applied-CA) schedule of the requested instrument. */
export interface ListInstrumentScheduleResponse {
  /** The instrument the schedule belongs to. */
  instrumentId: string;
  /** The remaining cashflows in ascending date order. */
  flows: InstrumentScheduleFlow[];
  /** Current outstanding nominal as a fraction of original (1.0 = full). */
  poolFactor: number;
}

/** `CorporateActionsService.ListCorporateActions` request; absent `isin` ⇒ all. */
export interface ListCorporateActionsRequest {
  /** Restrict to actions targeting this ISIN (absent ⇒ all known actions). */
  isin?: string;
}

/** The current version of every matching corporate action. */
export interface ListCorporateActionsResponse {
  actions: CorporateAction[];
}

/**
 * `CorporateActionsService.ConfirmCorporateAction` request — confirm a CA's
 * movement (`announced|elected → confirmed`). Requires the `refdata` capability.
 */
export interface ConfirmCorporateActionRequest {
  caId: string;
}

/** The corporate action after the confirm transition. */
export interface ConfirmCorporateActionResponse {
  action: CorporateAction;
}

/**
 * `CorporateActionsService.ApplyCorporateAction` request — apply a confirmed CA:
 * re-derive the schedule and book the position effect on the desk's held face.
 * Requires the `refdata` capability.
 */
export interface ApplyCorporateActionRequest {
  caId: string;
  /** The desk's held face (nominal) in the instrument the effect acts on. */
  heldFace: number;
}

/** The result of applying a corporate action. */
export interface ApplyCorporateActionResponse {
  /** The instrument the effect touched. */
  instrumentId: string;
  /** The change in held face; negative = a reduction, `-heldFace` = realised. */
  faceDelta: number;
  /** The cash thrown off by the event on the holding (coupon or principal). */
  cash: number;
  /** Cashflows remaining in the instrument's post-event schedule. */
  remainingFlows: number;
  /** The corporate action after the apply (status APPLIED). */
  action: CorporateAction;
}

/** Human-readable labels for the {@link CorpEventType} rows (CAEV). */
export const CORP_EVENT_TYPE_LABELS: Record<CorpEventType, string> = {
  REDM: "Redemption",
  INTR: "Coupon / interest",
  MCAL: "Full call",
  PCAL: "Partial call",
  PRED: "Partial redemption",
  DRAW: "Sinking-fund drawing",
  BPUT: "Put",
  TEND: "Tender offer",
  EXOF: "Exchange offer",
  CONV: "Conversion",
};

/** Human-readable labels for the {@link CorpMandatory} indicator (CAMV). */
export const CORP_MANDATORY_LABELS: Record<CorpMandatory, string> = {
  MAND: "Mandatory",
  VOLU: "Voluntary",
  CHOS: "Choice",
};

/** Human-readable labels for the {@link CorpActionStatus} lifecycle. */
export const CORP_ACTION_STATUS_LABELS: Record<CorpActionStatus, string> = {
  ANNOUNCED: "Announced",
  ELECTED: "Elected",
  CONFIRMED: "Confirmed",
  APPLIED: "Applied",
  REVERSED: "Reversed",
  CANCELLED: "Cancelled",
};
