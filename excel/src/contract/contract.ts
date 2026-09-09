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

/**
 * A precious metal traded as the asset (base) leg of a metal pair (proto `Metal`,
 * mirrors `celnet_types::Metal`). The four LBMA/LPPM precious metals; each
 * projects to an ISO-4217 "X"-prefixed asset code (XAU/XAG/XPT/XPD) so a metal
 * pair overlaps the FX `CcyPair` encoding byte-for-byte on the metal leg. The
 * string members are listed in proto enum-number order (GOLD=0, …).
 */
export type Metal = "GOLD" | "SILVER" | "PLATINUM" | "PALLADIUM";

/**
 * Contract settlement mechanics — how a contract's PnL is denominated and
 * margined (proto `SettlementStyle`, mirrors `celnet_types::SettlementStyle`;
 * carried on `Instrument.settlement_style`, field 29). LINEAR is the proto3 zero
 * default (the ordinary quote-currency-margined contract, byte-identical to the
 * contract before this field existed) for EVERY asset class; INVERSE_COIN is the
 * coin-margined digital-asset convention whose payoff is the `1/S_T`
 * (base-coin-denominated) form, meaningful only for a `digital_asset` underlying.
 */
export type SettlementStyle = "LINEAR" | "INVERSE_COIN";

export type StrategyKind = "RISK_REVERSAL" | "STRANGLE" | "STRADDLE" | "SEAGULL";

export type TenorUnit = "OVERNIGHT" | "WEEKS" | "MONTHS" | "YEARS";

/**
 * The smile-calibration family a surface is marked with (proto `SmileModel`).
 * Vendor/method-neutral names mirroring `celnet_proto::SmileModel`: MARKET_HEDGE
 * (the default Vanna-Volga market-hedge construction), and the four fitted
 * parametric families. EXTENDED_SURFACE is the surface-level parametric family
 * with maturity-dependent correlation (eSSVI method; provenance in this doc
 * comment only — the trader-facing/wire name is purpose-named "Extended").
 * Provenance of the model a surface was marked under is the TYPED, authoritative
 * `ArbReport.smileModel` field (proto `ArbReport.smile_model`, appended) — read
 * that, never the legacy `model=<family>` token in `note` (kept human-only).
 */
export type SmileModel =
  | "MARKET_HEDGE"
  | "STOCHASTIC_VOL"
  | "PARAMETRIC"
  | "PARAMETRIC_SURFACE"
  | "EXTENDED_SURFACE";

/**
 * The booking / pricing model a request is priced under (proto `PricingModel`):
 * a *pricing directive*, NOT an API version. DEFAULT is the product's native
 * analytic / closed-form engine (Garman-Kohlhagen and the product's own closed
 * form); LOCAL_STOCH_VOL routes the supported products (vanilla, continuously-
 * monitored single-barrier knock-out, window-barrier) through the
 * local-stochastic-volatility engine — a particle-calibrated leverage surface
 * over a mean-reverting square-root variance backbone, priced on a 2-D ADI PDE
 * (and a counter-based Monte-Carlo engine where a standard error is reported).
 * Provenance of the method is in this doc comment only — the wire/trader-facing
 * name is the purpose-named "Local-Stoch-Vol" (`celnet_proto::PricingModel`).
 *
 * An absent / DEFAULT model is byte-identical to the contract before this field
 * existed (proto3 zero value), so every existing flow is unchanged. Selecting
 * LOCAL_STOCH_VOL for a product the engine does not price is a hard server
 * `INVALID_ARGUMENT`, never a silent fallback; a window-barrier MUST carry
 * LOCAL_STOCH_VOL (it has no closed form, so DEFAULT on it is rejected).
 */
export type PricingModel = "DEFAULT" | "LOCAL_STOCH_VOL";

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
 * The premium margining convention of an option on a listed future (proto
 * `Margining`). EQUITY_STYLE (premium-upfront) pays the full premium at trade
 * date, so the price is the discounted expectation; FUTURES_STYLE margins the
 * premium daily like the future itself, so the price is the undiscounted
 * expectation. Meaningful-zero (like `SettlementStyle`): an absent/zero tag is
 * the ordinary equity-style contract. Mirrors `celnet_proto::Margining`
 * (MARGINING_EQUITY_STYLE=0, MARGINING_FUTURES_STYLE=1).
 */
export type Margining = "EQUITY_STYLE" | "FUTURES_STYLE";

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

/**
 * A precious-metal pair (proto `MetalPair`, mirrors `celnet_types::MetalPair`):
 * the `metal` is the base/asset leg, `quote` the fiat numeraire (3-letter code).
 * Projects byte-identically onto a metal-base `CcyPair` (base = the metal's
 * ISO-4217 "X"-prefixed asset code) the convention/calendar registries key on.
 */
export interface MetalPair {
  /** The precious metal (the base/asset leg). */
  metal: Metal;
  /** The fiat quote (numeraire) currency, e.g. "USD", "EUR", "JPY". */
  quote: string;
}

/**
 * A free-form instrument ticker (proto `Symbol`, mirrors `celnet_types::Symbol`)
 * — the vendor-neutral identifier for an asset not named by a currency-/metal-pair
 * leg structure (an equity, a commodity, a digital-asset coin). No vendor product
 * names appear: the purpose-named "instrument symbol", nothing more.
 */
export interface Symbol {
  /** The trading symbol / ticker, e.g. "AAPL", "BRENT", an ISIN. UTF-8. */
  ticker: string;
  /** The listing venue / exchange MIC, e.g. "XNAS"; empty when unambiguous. */
  venue: string;
}

/**
 * An equity (single-name or index) underlying (proto `EquityRef`, mirrors
 * `celnet_types::EquityRef`). The `symbol` names the listed instrument; `currency`
 * is the trading/settlement currency (a 3-letter code). The dividend treatment is
 * a carry-layer concern (generalized cost-of-carry), not encoded here.
 */
export interface EquityRef {
  /** The listed equity symbol (single name or index). */
  symbol: Symbol;
  /** The currency the equity is quoted / settled in (a 3-letter code). */
  currency: string;
}

/**
 * A commodity underlying (a futures-style or spot commodity; proto `CommodityRef`,
 * mirrors `celnet_types::CommodityRef`). The `symbol` names the commodity /
 * contract; `currency` is the quote/settlement currency (a 3-letter code). The
 * cost-of-carry (storage/convenience yield) is a carry-layer concern, not here.
 */
export interface CommodityRef {
  /** The commodity / contract symbol, e.g. "BRENT". */
  symbol: Symbol;
  /** The currency the commodity is quoted / settled in (a 3-letter code). */
  currency: string;
}

/**
 * A digital-asset (crypto) pair (proto `CryptoPair`, mirrors
 * `celnet_types::CryptoPair`). The `base` is the coin/asset leg (e.g. "BTC",
 * "ETH"); the `quote` is the numeraire — a fiat ("USD") or coin/stablecoin
 * ("USDT"). Both legs are UTF-8 strings (crypto tickers are not constrained to the
 * 3-letter ISO-4217 shape). The linear/inverse settlement of a coin-margined
 * contract is carried on `Instrument.settlementStyle`, NOT here (it is a
 * contract-mechanics convention, not part of the pair identity).
 */
export interface CryptoPair {
  /** The coin/asset (base) leg, e.g. "BTC", "ETH". UTF-8. */
  base: string;
  /** The numeraire (quote) leg — fiat ("USD") or coin/stablecoin ("USDT"). */
  quote: string;
}

/**
 * The instrument's underlying — the asset-class discriminator (proto `Underlying`,
 * mirrors `celnet_types::Underlying`). Exactly one `ref` arm is set. FX is the
 * first-class arm (and is also carried directly as `Instrument.pair` for the FX
 * WS surface's byte-identical projection); the metal/equity/commodity/digital-asset
 * arms are the cross-asset extensions (proto `oneof ref` field numbers fx=1,
 * metal=3, equity=4, commodity=5, digital_asset=6). `settlementCcy` is the
 * settlement / numeraire currency code (for FX, the pair's quote currency).
 */
export type Underlying =
  | { kind: "fx"; fx: CcyPair; settlementCcy: string }
  | { kind: "metal"; metal: MetalPair; settlementCcy: string }
  | { kind: "equity"; equity: EquityRef; settlementCcy: string }
  | { kind: "commodity"; commodity: CommodityRef; settlementCcy: string }
  | { kind: "digitalAsset"; digitalAsset: CryptoPair; settlementCcy: string };

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
 * A pivot Target-Redemption Accumulator (proto `Pivot`): the TARF mechanic with
 * a distinct `pivot` kink — each fixing's leg is SELECTED by the pivot and
 * VALUED by the strike intrinsic, the favourable leg accruing toward `target`
 * (knock-out on target, the shared `TarfRedemption` gap-risk convention), the
 * adverse leg geared by `leverage`. `pivot === strike` is the exact plain-TARF
 * slice. Monte-Carlo priced — the premium carries a standard error
 * (`price_std_error`).
 */
export interface Pivot {
  /** The favourable direction (PUT = the exporter orientation, gains below `strike`). */
  optionType: OptionType;
  /** The target strike `K` intrinsic is measured against. */
  strike: number;
  /** The pivot `P` at which the geared adverse leg engages (`P === K` ⇒ TARF). */
  pivot: number;
  /** The cumulative gain target; accumulated client gain at/above it redeems. */
  target: number;
  /** The gearing/leverage multiplier on the adverse leg (`≥ 0`). */
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

/**
 * A window (partial-time) barrier (proto `WindowBarrier`, product field 23): a
 * terminal vanilla payoff that knocks out only while spot crosses the barrier
 * INSIDE the active window `[windowStart, windowEnd] ⊆ [0, expiryYears]` — a
 * "front" partial barrier sets `windowStart = 0`, a "back" partial barrier sets
 * `windowEnd = expiryYears`. A window barrier is always a knock-out and has NO
 * closed form, so it is priced only under `PricingModel` LOCAL_STOCH_VOL (the
 * LSV ADI-PDE engine, or the counter-based Monte-Carlo engine when `mcPairs > 0`,
 * which then carries a `priceStdError`); selecting DEFAULT for it is rejected.
 */
export interface WindowBarrier {
  /** The terminal vanilla payoff (call/put + strike). */
  vanilla: Vanilla;
  /** The barrier level `H` (quote per 1 unit of base). */
  barrier: number;
  /** UP (barrier above spot at inception, up-and-out) or DOWN (down-and-out). */
  side: BarrierSide;
  /** The start of the active window, in years from inception (`>= 0`). */
  windowStart: number;
  /** The end of the active window, in years (`windowStart < windowEnd <= expiryYears`). */
  windowEnd: number;
  /**
   * Antithetic Monte-Carlo path pairs: `0` selects the exact ADI-PDE engine (no
   * std-error); `> 0` selects the Monte-Carlo engine (the reply carries a
   * `priceStdError`).
   */
  mcPairs: number;
  /** Monte-Carlo time steps (ignored when `mcPairs == 0`; `0` ⇒ server default). */
  mcSteps: number;
  /** Counter-RNG seed (bit-reproducible; ignored when `mcPairs == 0`). */
  mcSeed: bigint;
}

/**
 * The early-exercise style of an option (proto `ExerciseStyle`): AMERICAN
 * (continuous exercise up to expiry) or BERMUDAN (exercise only on the discrete
 * dates in `AmericanOption.bermudanDates`).
 *
 * Mirrors `celnet_proto::ExerciseStyle`
 */
export type ExerciseStyle = "AMERICAN" | "BERMUDAN";

/**
 * An American / Bermudan early-exercise vanilla (proto `AmericanOption`, product
 * field 24). Physically-settled FX options trade American-style. The default
 * engine is the projected-SOR free-boundary finite difference (exact, no
 * std-error); setting `lsmPaths > 0` selects the Longstaff-Schwartz regression
 * Monte-Carlo (which carries a `priceStdError`). BERMUDAN exercises only on
 * `bermudanDates` (year-fractions in `(0, expiryYears]`; expiry is always
 * exercisable). Scope: American/Bermudan VANILLA only.
 */
/**
 * How the per-leg terminal levels of a correlated multi-asset option combine
 * into the option underlying.
 *
 * Mirrors `celnet_proto::BasketKind`
 * (BASKET_KIND_BASKET=0, BASKET_KIND_BEST_OF=1, BASKET_KIND_WORST_OF=2).
 */
export type BasketKind = "BASKET" | "BEST_OF" | "WORST_OF";

/**
 * One leg of a correlated multi-asset (basket / best-of / worst-of) option: an
 * FX underlying with its own market data and basket weight (proto `BasketLeg`).
 * Per-leg market data travels IN the leg (the single-pair request market context
 * cannot hold N underlyings); the shared domestic rate is the request market
 * context's `rDom`.
 */
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
 * Priced by Cholesky-correlated multi-asset GBM Monte-Carlo, so the reported
 * price carries a `priceStdError`. Multi-asset Greeks are deferred (the strip is
 * zeroed). The enclosing `Instrument.pair` is the settlement / numeraire pair;
 * the underlyings are the per-leg `BasketLeg.pair`s and the shared domestic rate
 * is the request market context's `rDom`.
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
  /** The base scramble seed (bit-reproducible). */
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
  /** Longstaff-Schwartz path count: `0` ⇒ the exact FD engine; `> 0` ⇒ LSM (carries a std-error). */
  lsmPaths: number;
  /** LSM equally-spaced exercise opportunities for AMERICAN (`0` ⇒ server default; ignored for BERMUDAN). */
  lsmExerciseDates: number;
  /** Sobol scramble seed for the LSM engine (bit-reproducible; ignored for FD). */
  lsmSeed: bigint;
}

/**
 * The published settlement-rate option a non-deliverable forward fixes against
 * (proto `FixingSource`, mirrors `celnet_types::FixingSource`). Convention
 * identity only — the live fixing VALUE is an estate-gated feed, never sourced
 * in-repo; it does not enter the deterministic discounted-cashflow PV. The string
 * members are listed in proto enum-number order (KRW_KFTC18=0, …).
 */
export type FixingSource =
  | "KRW_KFTC18"
  | "TWD_TAIPEI"
  | "INR_RBI_REF"
  | "BRL_PTAX"
  | "CLP_DOLAR_OBS"
  | "COP_TRM";

/**
 * An FX outright forward (proto `FxForward`) — a linear, closed-form
 * discounted-cashflow product (NOT an option payoff): PV is
 * `side · notional · discount_df(t) · (forward_rate − contractRate)`. Exact ⇒ no
 * standard error. Valid for a deliverable underlying; an NDF pair is rejected.
 */
export interface FxForward {
  /** The contract (delivery) rate `K`, quote per 1 unit of base/asset. */
  contractRate: number;
  /** The notional (always positive; direction is `side`). */
  notional: number;
  /** The directional side (BUY = long the base/asset forward; never TWO_WAY). */
  side: Side;
}

/**
 * An FX swap (proto `FxSwap`) — a near leg + a far leg, each an `FxForward`. By
 * convention the two legs trade opposite directions; the swap PV is the sum of
 * the two leg PVs. The near leg settles at the spot date and the far leg at the
 * instrument's forward tenor (`expiryYears`).
 */
export interface FxSwap {
  /** The near (shorter-dated, spot-settling) leg. */
  near: FxForward;
  /** The far (longer-dated) leg; the opposite side at the same contract rate. */
  far: FxForward;
}

/**
 * A non-deliverable forward (proto `Ndf`) — cash-settled in the convertible
 * (settlement) currency at a named fixing. The risk-neutral PV is identical to a
 * deliverable forward of equal terms (non-deliverability changes only settlement
 * mechanics). Valid ONLY for a non-deliverable underlying.
 */
export interface Ndf {
  /** The contract (forward) rate `K`, settlement-ccy per 1 unit of base. */
  contractRate: number;
  /** The notional (always positive; direction is `side`). */
  notional: number;
  /** The directional side (BUY = long the base/asset forward; never TWO_WAY). */
  side: Side;
  /** The published settlement-rate option the contract fixes against (identity only). */
  fixing: FixingSource;
  /** The convertible (settlement) currency code, e.g. "USD". */
  settlementCcy: string;
}

/**
 * A perpetual (no-expiry) American option (proto `PerpetualOption`, product
 * field 30): exercisable at any time, forever. The enclosing instrument is the
 * one TENORLESS, expiryless shape on the contract — `expiryYears` MUST be `0`
 * exactly and no `tenor` is carried (the server's
 * `convert::validate_perpetual_terms` rejects anything else as
 * INVALID_ARGUMENT, never silently ignores it). Priced by the stationary-ODE
 * closed form on the same carry branching as the vanilla; exact ⇒ no standard
 * error, and theta is identically zero (the value is time-homogeneous).
 */
export interface PerpetualOption {
  optionType: OptionType;
  /** The strike `K` (absolute level, quote per 1 unit of base/asset). */
  strike: number;
  /** The booked notional (always positive; direction is the instrument `side`). */
  notional: number;
}

/**
 * An option on a listed future, for any asset class (proto `ListedFutureOption`,
 * product field 31): the enclosing `Instrument.underlying` names the asset class
 * and `futureSymbol` names the specific listed contract the option exercises
 * into. The future must outlive the option —
 * `futureExpiryYears >= Instrument.expiryYears > 0` is validity-checked by the
 * server (`convert::validate_listed_future_terms`), INVALID_ARGUMENT otherwise.
 * The quoted futures price (the request market context's `spot`) already
 * embodies the underlying's carry, so every asset class prices by the same
 * futures-measure closed form; `margining` decides whether the premium is paid
 * upfront (equity-style, discounted) or margined daily (futures-style,
 * undiscounted).
 */
export interface ListedFutureOption {
  /** The listed future contract the option exercises into (ticker + venue MIC). */
  futureSymbol: Symbol;
  /** The FUTURE's own expiry (year fraction); `>= Instrument.expiryYears > 0`. */
  futureExpiryYears: number;
  optionType: OptionType;
  /** The strike `K` (absolute level, in the future's quote units). */
  strike: number;
  /** The booked notional (always positive; direction is the instrument `side`). */
  notional: number;
  /** The premium margining convention (equity-style upfront vs futures-style daily). */
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
   * The cross-asset underlying (proto `Instrument.underlying`, field 1 — an
   * `Underlying` oneof). Presence-tracked: absent ⇒ the FX projection carried by
   * `pair` (the FX WS surface keys on `pair`). A non-FX instrument sets this to a
   * metal / equity / commodity / digital-asset arm; `pair` then carries the
   * underlying's leg-string projection so the FX-keyed surfaces stay total.
   */
  underlying?: Underlying;
  /**
   * The standard tenor label (proto `Instrument.tenor`, a message field —
   * presence-tracked). Present on every dated product; ABSENT only for the one
   * tenorless shape on the contract, the perpetual (no-expiry) American option
   * (`perpetualOption`, whose canonical wire form carries no tenor and
   * `expiryYears = 0` exactly — mirrors the SDK's `tenor: None`).
   */
  tenor?: Tenor;
  /** Expiry year fraction on the surface day-count (authoritative for pricing). */
  expiryYears: number;
  quantity: Quantity;
  side: Side;
  solve?: Solve;
  /**
   * The contract settlement mechanics (proto `Instrument.settlement_style`, field
   * 29). Presence-tracked: absent/`LINEAR` ⇒ the ordinary quote-currency-margined
   * linear contract (the proto3 zero value, byte-identical to the contract before
   * this field existed); `INVERSE_COIN` selects the coin-margined `1/S_T`
   * digital-asset convention. Travels uniformly on the Instrument exactly like
   * `pricingModel`, so it reaches price/quote/stream/scenario.
   */
  settlementStyle?: SettlementStyle;
  /**
   * The booking / pricing model the request is priced under (proto
   * `Instrument.pricing_model`, field 22). Absent ⇒ DEFAULT (the product's native
   * analytic engine), byte-identical to the contract before this field existed.
   * Carried uniformly on the Instrument, so it reaches every flow
   * (price/quote/stream/scenario) exactly like the `solve` directive.
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
  /**
   * For a Monte-Carlo-priced product (e.g. a clamped cliquet), the standard error
   * of the mean of the `greeks.price` estimate (proto `price_std_error`, field 7;
   * presence-tracked — absent for the closed-form products whose price is exact).
   * Surfaced honestly so a cell never mistakes an MC estimate for closed-form
   * precision; the same field the SDK reads as `PricedLine::price_std_error`.
   */
  priceStdError?: number;
}

/**
 * One liquidity provider's line in a multi-dealer (RFQ-to-many) panel (proto
 * `DealerQuote`). The aggregate `MultiDealerQuote.quoteId` keys the request; this
 * line's `lpId` disambiguates which dealer's price a `QuoteAccept` lifts/hits.
 * The in-repo demo panel is the native maker plus deterministic synthetic
 * dealers (`SYNTH-LP-k`) quoting around the same edge mid — live LP connectivity
 * is environment-provided, never claimed by these types.
 */
export interface DealerQuote {
  /** The liquidity provider's stable identifier (the dealer/LP key). */
  lpId: string;
  /** This dealer's two-way premium in the request's premium-style units. */
  price: TwoWayPrice;
  /**
   * The full Greek set — present ONLY on the native maker's line (proto
   * `optional`; an LP discloses a price, not its greeks). Absent ⇒ `undefined`,
   * so a panel row never fabricates a zero Greek strip.
   */
  greeks?: Greeks;
  /** The strike this dealer resolved (if the request used a delta / solve). */
  resolvedStrike: number;
  /** This dealer's last-look deadline, nanoseconds since the Unix epoch (UTC). */
  validUntilNanos: bigint;
  /** MC standard error of this dealer's premium (presence-tracked; native-only). */
  priceStdError?: number;
}

/**
 * The multi-dealer (RFQ-to-many) panel response (proto `MultiDealerQuote`): the
 * competing dealer lines for one request, PRE-RANKED by the server's aggregator
 * (`dealers` is ordered best-first; a client never re-sorts). `bestBidLpId` /
 * `bestOfferLpId` name the touch dealers (empty ⇒ no dealer quoted that side).
 * A `QuoteAccept` echoes the aggregate `quoteId` with the chosen line's `lpId`.
 */
export interface MultiDealerQuote {
  quoteId: bigint;
  idempotencyKey: string;
  /** The competing dealer lines in the server's ranking order (best-first). */
  dealers: DealerQuote[];
  bestBidLpId: string;
  bestOfferLpId: string;
  conventions: Conventions;
  epochNanos: bigint;
  correlationId?: bigint;
  surfaceVersion?: bigint;
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

/**
 * A liveness beat (proto `Heartbeat`) carrying the current sequence plus the
 * server's drain-side observability (all appended fields — NO `schema_version`).
 * `subscriptionId` is `undefined` for a connection-level beat (one beat for the
 * whole session) and set for a per-subscription beat. The observability fields
 * are the EXACT values the server stamps: `conflationDrops` is the `celnet-fanout`
 * ring's real skip count (`received + skipped == produced`); the three latency
 * percentiles are the drain-side HdrHistogram of `sub.price(...)` in nanoseconds
 * (0 until the first timed price); `surfaceVersion`/`correlationId` echo
 * provenance (0 ⇒ live/unpinned / none).
 */
export interface Heartbeat {
  subscriptionId?: bigint;
  sequence: bigint;
  conflationDrops: bigint;
  serverPriceP50Nanos: bigint;
  serverPriceP99Nanos: bigint;
  serverPriceP999Nanos: bigint;
  surfaceVersion: bigint;
  correlationId: bigint;
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
  /**
   * The TYPED, authoritative calibration-family provenance (proto
   * `ArbReport.smile_model`, appended — NO `schema_version`). This is what the
   * surface was actually marked under, taken from the calibrated smile itself;
   * read it directly and never parse the legacy `model=<family>` token in `note`
   * (which is retained for human eyes only).
   */
  smileModel: SmileModel;
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

// ---------------------------------------------------------------------------
// fixed-income (rates) — the linear-rates pricing contract (`PricingService
// .PriceRates`). Mirrors the `celnet.wire` rates messages one-to-one: a
// `CurveSet` of par-OIS pillars + an `OisInstrument`, priced to a
// `RatesPricingResult`. The oneof grows additively (FRA, IRS, basis) as each is
// backed end-to-end; the OIS arm is the USD-SOFR P0 arm.
// ---------------------------------------------------------------------------

/** An explicit civil calendar date (`celnet.wire.BrokenDate`): 1-based month/day. */
export interface BrokenDate {
  year: number;
  /** 1-based calendar month (1 = January). */
  month: number;
  /** 1-based day of month. */
  day: number;
}

/** One self-discounting OIS curve pillar (`celnet.wire.OisPillar`). */
export interface OisCurvePillar {
  /** The swap tenor in whole years from spot (e.g. 2, 5, 10); `>= 1`. */
  tenorYears: number;
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

// --- curve bootstrap from registry-referenced instruments (`BuildCurve`) ------
// Mirrors the GUI client (`gui/src/data/contract.ts`) field-for-field: pick
// reference-data registry instruments + a calibrating quote each, the server
// resolves every id, bootstraps, and returns per-instrument calibrated points.

/**
 * One calibrating quote for a {@link BuildCurveRequest} (`celnet.wire
 * .InstrumentQuote`): a registry instrument id + its observed market quote as a
 * decimal (0.0405 = 4.05%).
 */
export interface InstrumentQuote {
  instrumentId: string;
  quote: number;
}

/**
 * A standalone date-anchored calibration pillar (`celnet.wire.DatePillar`): an
 * explicit maturity date + its observed simple ACT/360 rate (decimal). The server
 * resolves it to a synthetic cash deposit (`DF = 1/(1 + r·τ)`) from the curve
 * reference date, pinning the curve where no registry instrument matures.
 */
export interface DatePillar {
  maturityDate: BrokenDate;
  quote: number;
}

/**
 * A request to bootstrap a single-currency discount curve (`celnet.wire
 * .BuildCurveRequest`). Pillars may arrive in any order. Calibrating pillars are
 * registry instruments ({@link pillars}) or standalone date-anchored pillars
 * ({@link datePillars}); at least one across both is required.
 */
export interface BuildCurveRequest {
  requestId: string;
  currency: string;
  referenceDate: BrokenDate;
  pillars: readonly InstrumentQuote[];
  datePillars: readonly DatePillar[];
}

/**
 * One bootstrapped pillar of a {@link CalibratedCurve} (`celnet.wire
 * .CalibratedCurvePoint`).
 */
export interface CalibratedCurvePoint {
  /** The input instrument id; empty for a date-anchored pillar. */
  instrumentId: string;
  /** Pillar maturity on the ACT/365F curve year-fraction axis. */
  timeYears: number;
  discountFactor: number;
  /** Continuously-compounded zero rate (decimal). */
  zeroRate: number;
  /** Display label: `Date YYYY-MM-DD` for a date pillar, else empty. */
  label: string;
}

/**
 * A bootstrapped discount curve (`celnet.wire.CalibratedCurve`): the calibrated
 * pillar points (short→long by maturity) plus the echoed request header.
 */
export interface CalibratedCurve {
  requestId: string;
  currency: string;
  referenceDate: BrokenDate;
  points: readonly CalibratedCurvePoint[];
}

// --- curve query (`GetCurve` / `SurfaceService.GetCurve`, ADR-0021) -----------
// The fixed-income analogue of `GetSmile`: read a marked or freshly-bootstrapped
// discount curve on a tenor axis. Exactly one curve source — a pinned
// `curveVersion` reads the marked curve of that version from the store, otherwise
// the inline `curveSet` is bootstrapped live. Mirrors the server messages
// (`celnet.wire.GetCurveRequest` / `.GetCurveResponse` / `.CurvePoint` /
// `.CurveParPillar`) field-for-field over the one unversioned contract.

/**
 * A request to read a discount curve on a tenor axis (`celnet.wire
 * .GetCurveRequest`). The `curveSet` is bootstrapped and read unless a
 * `curveVersion` is pinned (then the marked curve of that version is read and the
 * `curveSet` is ignored, exactly like the pinned-price path). Points are reported
 * at each `queryTenorYears` (year fractions from the reference date).
 */
export interface GetCurveRequest {
  /** The calibrating curve set to bootstrap and read (ignored when `curveVersion` is pinned). */
  curveSet: RatesCurveSet;
  /** The tenors (year fractions from the reference date) to report zero rates / discount factors at. */
  queryTenorYears: readonly number[];
  /**
   * Optional pin to a `MarkCurve`d version: present ⇒ read the marked curve from
   * the store; absent ⇒ bootstrap `curveSet` live. A *data* field selecting a
   * marked curve, never an API version (the contract is unversioned, rule 9).
   */
  curveVersion?: number | undefined;
}

/**
 * One queried point of a read curve (`celnet.wire.CurvePoint`): the tenor plus the
 * continuously-compounded zero rate `z(t) = -ln DF(t) / t` and discount factor
 * `DF(t) = exp(-z(t)·t)` there.
 */
export interface CurveQueryPoint {
  /** The tenor (year fraction from the reference date) this point reports. */
  tenorYears: number;
  /** The continuously-compounded zero rate at this tenor (decimal). */
  zeroRate: number;
  /** The discount factor at this tenor. */
  discountFactor: number;
}

/**
 * A calibrating par pillar echoed by a read curve (`celnet.wire.CurveParPillar`):
 * the pillar resolved to its final-payment year fraction from spot plus its
 * observed par (fair fixed) rate — the curve analogue of a smile's broker quotes.
 */
export interface CurveParPillar {
  /** The resolved final-payment year fraction from spot for this pillar. */
  tenorYears: number;
  /** The observed par (fair fixed) rate as a decimal (0.0405 = 4.05%). */
  parRate: number;
}

/**
 * The read curve reply (`celnet.wire.GetCurveResponse`): the currency + reference
 * date, the queried points, the echoed calibrating par pillars, and (when the
 * request pinned a version) the marked version this curve was read from — the FI
 * analogue of a `Smile`.
 */
export interface GetCurveResponse {
  /** ISO-4217 currency of the curve (USD for the P0 arm). */
  currency: string;
  /** The curve reference (spot-anchor) date the tenor axis is measured from. */
  referenceDate: BrokenDate;
  /** The queried points (zero rate + discount factor per requested tenor). */
  points: readonly CurveQueryPoint[];
  /** The calibrating par pillars echoed (resolved tenor + par rate). */
  parPillars: readonly CurveParPillar[];
  /** The marked version this curve was read from; present only for a pinned read. */
  curveVersion?: bigint | undefined;
  /** Read time, nanoseconds since the Unix epoch (UTC). */
  epochNanos: bigint;
}

/**
 * A request to mark/persist a bootstrapped discount curve under a fresh pinned
 * version (`celnet.wire.MarkCurveRequest`) — the FI analogue of a surface mark
 * (`MarkSurfaceRequest`). The `curveSet` is bootstrapped and deposited through the
 * same versioning seam a surface mark uses, so a later `GetCurve` pinned to the
 * returned `curveVersion` reproduces this exact curve.
 */
export interface MarkCurveRequest {
  /** The calibrating curve set to bootstrap and persist. */
  curveSet: RatesCurveSet;
}

/**
 * The result of a curve mark (`celnet.wire.MarkCurveResponse`): a server-assigned
 * curve version id, the resolved calibrating par pillars, and the bootstrapped
 * points at the pillar tenors — the FI analogue of a `MarkSurfaceResponse`. The
 * returned `curveVersion` is always present (a mark always stamps a version) and
 * is the id a subsequent `GetCurve(pinnedVersion)` reproduces the curve from.
 */
export interface MarkCurveResponse {
  /** ISO-4217 currency of the marked curve. */
  currency: string;
  /**
   * The monotonic server-assigned curve version id this mark stamped — always
   * present (unlike a read's presence-tracked pin). A `data` field selecting a
   * marked curve, never an API version (the contract is unversioned, rule 9).
   */
  curveVersion: bigint;
  /** The resolved calibrating par pillars marked (resolved tenor + par rate). */
  parPillars: readonly CurveParPillar[];
  /** The bootstrapped points (zero rate + discount factor) at the pillar tenors. */
  points: readonly CurveQueryPoint[];
  /** Mark time, nanoseconds since the Unix epoch (UTC). */
  epochNanos: bigint;
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
 * The wire carries the numeric proto tag (ANNUAL=0, SEMI_ANNUAL=1, QUARTERLY=2).
 */
export type PaymentFrequency = "ANNUAL" | "SEMI_ANNUAL" | "QUARTERLY";

/**
 * A curve/leg day-count basis (`celnet.wire.DayCount`) — the money-market subset an
 * IRS leg accrues on (ACT/365F or ACT/360). Distinct from {@link RatesAccrualBasis}:
 * an IRS leg carries no 30/360 arm (the market 30/360 fixed leg awaits the shared
 * curve-time day-count extension), matching the server `VanillaIrsInstrument`. The
 * wire carries the numeric proto `DayCount` tag (ACT_365_FIXED=0, ACT_360=1).
 */
export type RatesLegDayCount = "ACT_365_FIXED" | "ACT_360";

/**
 * A vanilla fixed-vs-float interest-rate swap to price (`celnet.wire
 * .VanillaIrsInstrument`, the `RatesInstrument.irs` oneof arm) — a fixed leg vs a
 * projected floating leg on the single self-discounting curve, each leg at its own
 * frequency + day-count. Spot-starting: the schedule of `tenorYears` whole years is
 * reconstructed server-side from the `RatesCurveSet` reference date (there is no
 * separate effective/maturity date on the wire). The PV is
 * `N·(K·A_fixed − F_float)` (receive-fixed); `direction` maps to the wire `Side`.
 * Priced through the SAME `price_rates` RPC as the OIS/bond arms — every field is a
 * FLAT scalar or numeric enum tag (no nested date), byte-matching the server's
 * `vanilla_irs_instrument_from_json` decoder.
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
 * A forward rate agreement to price (`celnet.wire.FraInstrument`, the
 * `RatesInstrument.fra` oneof arm) — a single accrual window `[startMonths,
 * endMonths]` on the projected float index, rebuilt server-side to roll-adjusted
 * dates from the curve reference date. The window is quoted in whole months from
 * spot (the standard "3x6 FRA" market convention), so — like the IRS/OIS arms —
 * every field is a FLAT scalar or numeric enum tag (no nested date), byte-matching
 * the server's `fra_instrument_from_json` decoder. The PV is the single-period
 * swaplet `N·(K·τ·DF(end) − (DF(start) − DF(end)))` (receive-fixed); `direction`
 * maps to the wire `Side`.
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
   * `dv01` to first order (the residual is curve cross-gamma). EMPTY for a cash
   * bond (`celnet.wire.BondInstrument`): the engine's bond risk is a closed-form
   * yield-space measure with no per-calibrating-pillar decomposition.
   */
  keyRateLadder: readonly number[];
}

/**
 * An instrument-level accrual basis (`celnet.wire.AccrualBasis`) — the money-market
 * bases plus 30/360 Bond Basis (the standard USD fixed-bond / fixed-swap basis).
 * The instrument-level superset of the curve/leg {@link RatesLegDayCount}. Used by
 * the FRA accrual window and the bond coupon accrual. The wire carries the numeric
 * proto tag (ACT_360=0, ACT_365_FIXED=1, THIRTY_360_BOND_BASIS=2).
 */
export type RatesAccrualBasis = "ACT_360" | "ACT_365_FIXED" | "THIRTY_360_BOND_BASIS";

/**
 * The coupon payment frequency of a cash bond — also the yield-compounding basis.
 * Identical to the leg {@link PaymentFrequency} (one canonical wire enum).
 */
export type BondCouponFrequency = PaymentFrequency;

/**
 * The accrual day-count basis of a cash bond. Identical to the instrument-level
 * {@link RatesAccrualBasis} (one canonical wire enum) — carries 30/360 Bond Basis.
 */
export type BondDayCount = RatesAccrualBasis;

/**
 * The position direction of a cash bond (`celnet.wire.BondInstrument.side`): LONG
 * (a bought bond, +PV — wire `Side` SIDE_BUY = 0) or SHORT (a sold bond, −PV —
 * wire `Side` SIDE_SELL = 1). SIDE_TWO_WAY is rejected by the engine for an
 * outright bond price.
 */
export type BondPositionSide = "LONG" | "SHORT";

/**
 * A fixed-coupon cash bond to price off the calibrated curve
 * (`celnet.wire.BondInstrument`, the `RatesInstrument.bond` oneof arm). The engine
 * PVs each cashflow at the bootstrapped discount curve (the DIRTY price) and
 * reports the implied yield-risk set. Settlement is the `RatesCurveSet` reference
 * (spot-anchor) date rolled to the next US business day; the coupon schedule is the
 * regular month-step dates rolled back from `maturityDate` at `couponFrequency`.
 */
export interface BondInstrument {
  /** The annual coupon rate as a decimal (0.06 = 6%); 0 for a zero-coupon bond. */
  couponRate: number;
  /** The coupon payment frequency (also the yield compounding basis). */
  couponFrequency: BondCouponFrequency;
  /** The accrual day-count basis for accrued interest. */
  dayCount: BondDayCount;
  /** The maturity (final-redemption) civil date; must be strictly after settlement. */
  maturityDate: BrokenDate;
  /** The par redemption / face value (e.g. 100); strictly positive. */
  redemption: number;
  /** LONG (+PV) or SHORT (−PV); the position direction carries the PV sign. */
  side: BondPositionSide;
}

/**
 * A linear interest-rate instrument to stream/price (`celnet.wire.RatesInstrument`
 * oneof) — exactly one arm is set, discriminated by `kind`. The add-in projection
 * of the server's `rates_instrument` oneof: a rates streaming line
 * (`CELNET.RATESSERIES`) builds one of these and opens it via
 * `Connection.subscribeRates`. Byte-compatible with the unary `price_rates` arm
 * encoders (`ratesInstrumentUnionToWire`), so a streamed baseline is identical to
 * the `CELNET.RATES`/`IRS`/`FRA`/`BOND` unary price of the same instrument. The
 * arms grow additively (rule 9), never renumbering.
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
 * A tradeable two-way RFQ reply for a linear-rates instrument (`celnet.wire
 * .RatesQuote`, returned by `QuoteService.RequestRatesQuote`) — the fixed-income
 * analogue of {@link Quote}, bringing request-for-quote parity to the add-in's FI
 * surface (`=CELNET.RATESRFQ`; FI RFQ previously existed only over FIX). `price` is
 * the two-way struck around the SIDE-INDEPENDENT fair level: a RATE market (bid/offer
 * around the par rate) for an OIS/IRS/FRA, a clean-PRICE market for a cash bond, so
 * `(bid + offer) / 2` is the fair level the FI pricing path computed
 * (`result.parRate` for a swap/FRA). `result` is the FULL `price_rates` risk (PV,
 * par rate, PV01, DV01, key-rate ladder) at the requested taker `side` — the SAME
 * engine numbers the outright `CELNET.RATES`/`IRS` cells return (no second pricing
 * path). The add-in carries no FI math of its own; the one unversioned contract makes
 * this reply authoritative and bit-identical to the GUI / SDK rates RFQ.
 */
export interface RatesQuote {
  /** Server-assigned stable, unguessable quote identifier (minted like `Quote.quoteId`). */
  quoteId: bigint;
  /** Echo of the originating request's idempotency key. */
  idempotencyKey: string;
  /**
   * The two-way bid/offer — a rate market for an OIS/IRS/FRA, a clean-price market
   * for a cash bond. `(bid + offer) / 2` is the side-independent fair level.
   */
  price: TwoWayPrice;
  /** The full linear-rates risk of the quoted position at the requested taker `side`. */
  result: RatesPricingResult;
  /** The RFQ size the two-way is good for (curve currency), echoed from the request. */
  notional: number;
  /** Publication time, nanoseconds since the Unix epoch (UTC). */
  epochNanos: bigint;
  /**
   * Quote validity deadline (last-look window), nanoseconds since the Unix epoch
   * (UTC). An accept after this instant is treated as expired.
   */
  validUntilNanos: bigint;
  /**
   * Echo of the originating request's `correlationId`, if one was supplied
   * (presence-tracked; the transport routing id under the shared request/reply
   * correlation), so a client joins this reply to its request.
   */
  correlationId?: bigint;
}

/**
 * The baseline state of a streamed fixed-income line (`celnet.wire
 * .RatesStreamSnapshot`) — the priced `RatesPricingResult` a consumer applies
 * whole before consuming deltas. Faithful to `PricingService.PriceRates`: the
 * snapshot's `result` at the subscribed baseline curve equals
 * `price_rates(instrument, curve_set)` exactly (`curveShift` is always 0 at the
 * baseline). Carries NO tradable token — a rates stream is indicative (rates
 * click-to-trade books through the RFQ/desk path).
 */
export interface RatesStreamSnapshot {
  /** The client `SubscriptionId` this snapshot answers. */
  readonly subscriptionId: bigint;
  /** The monotonic per-subscription sequence number of this snapshot (1). */
  readonly sequence: bigint;
  /** The priced PV + PV01 / DV01 / key-rate ladder at this snapshot. */
  readonly result: RatesPricingResult;
  /** The parallel curve shift (decimal) applied vs the baseline — 0 at baseline. */
  readonly curveShift: number;
  /** Echo of the opening `RatesSubscribe.correlationId`, if one was supplied. */
  readonly correlationId?: bigint;
  /** Snapshot time (ns since the Unix epoch, UTC). */
  readonly epochNanos: bigint;
}

/**
 * A sequenced delta on a streamed fixed-income line (`celnet.wire
 * .RatesStreamUpdate`): the line re-priced at the next sequence against the
 * baseline curve shifted by `curveShift` (the deterministic per-curve evolution
 * the fan-out published). A gap in `sequence` never arises — the server only
 * advances the sequence on a successful send — so the client applies the freshest
 * update (latest-tick) and relies on reconnect → re-subscribe for recovery.
 */
export interface RatesStreamUpdate {
  /** The client `SubscriptionId` this update advances. */
  readonly subscriptionId: bigint;
  /** The monotonic per-subscription sequence number (snapshot seq + n). */
  readonly sequence: bigint;
  /** The re-priced PV + first-order risk at this sequence. */
  readonly result: RatesPricingResult;
  /** The parallel curve shift (decimal) applied to the baseline curve for this tick. */
  readonly curveShift: number;
  /** Update time (ns since the Unix epoch, UTC). */
  readonly epochNanos: bigint;
}

// ---------------------------------------------------------------------------
// XVA — counterparty valuation adjustments (CVA / DVA / FVA) over a netting set
// (`PricingService.PriceXva`). A netting set of FX vanillas is priced for its
// all-in credit / funding valuation adjustments: CVA (counterparty default), DVA
// (own default), FVA (funding of the uncollateralised net expected exposure). The
// wire request carries the netting set, the single-factor exposure-model market
// (spot / vol / rates + the MC path/step budget), the counterparty & own survival
// (hazard) curves, the two LGDs and the funding spread; the wire response
// (`XvaResult`) carries ONLY the four scalar adjustments — the simulated exposure
// PROFILE (EPE/ENE per bucket) is a server-internal of the estimator and is NOT
// projected onto the contract. Mirrors `celnet.wire.PriceXvaRequest` / `XvaResult`
// field-for-field (the Excel port of `gui/src/data/contract.ts`). The add-in holds
// no XVA math — the live `celnet-xva` engine prices; this only shapes/lays out.
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
  pillarTimes: readonly number[];
  /** Per-segment hazard rates (≥ 0); exactly one entry for the flat curve. */
  hazardRates: readonly number[];
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
  trades: readonly XvaTrade[];
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
// Desk notification push contract — ONE CONTRACT, minimal duplicate of
// `gui/src/data/contract.ts` (CLAUDE.md rule 9), semantics-identical. The typed
// face of `NotificationService.StreamNotifications`: the server pushes a
// `Notification` the instant a desk-lifecycle event occurs (an RFQ/IOI lands, a
// request is withdrawn/expires, a quote is accepted/rejected, an order lands, a
// deal fills). The add-in mirrors the GUI's kind taxonomy so an inbound
// order/fill labels identically across every client.
// ---------------------------------------------------------------------------

/**
 * The flavour of an inbound desk request (`celnet.wire.DeskRequestKind`, proto
 * RFQ=1, IOI=2, RFS=3, ESP=4): a firm-price request-for-quote, a non-firm
 * indication-of-interest, or a request-for-stream lift — the counterparty opened a
 * market-data stream for a named instrument and its own clip size and lifted the
 * streamed line. Carried on a `Notification.requestKind`.
 */
export type DeskRequestKind = "RFQ" | "IOI" | "RFS" | "ESP";

/**
 * What a pushed notification concerns (`celnet.wire.NotificationKind`, proto
 * RFQ_RECEIVED=1 … FILL=9). `ORDER_RECEIVED`=8 / `FILL`=9 are the notifications
 * phase-5 additive arms — a FIX-venue firm-order lift emits both (order landed →
 * deal booked). Every member is first-class: no silent default drops the new arms
 * into a generic bucket (four-client parity with the GUI + CLI + SDK).
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
 * CREDIT_RISK_BREAK=2, UNKNOWN_SECURITY=3, PRICING_FAILURE=4). Present only on the
 * manual-intervention kind.
 */
export type ManualInterventionReason =
  | "UNCONFIGURED_TENOR"
  | "CREDIT_RISK_BREAK"
  | "UNKNOWN_SECURITY"
  | "PRICING_FAILURE";

/**
 * One pushed desk-lifecycle event (`celnet.wire.Notification`) — the WHOLE wire
 * message, field-for-field. The add-in renders `kind` via
 * `notificationKindLabel` and keys de-dup on `notificationId`.
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
   * The server's authoritative decision on whether this event warrants a POPUP:
   * `true` ⇒ surface it; missing / non-`true` ⇒ it lands quietly. The client can
   * narrow this further but can never force a popup the server marked quiet.
   */
  alertWorthy: boolean;
  /** Only meaningful for `MANUAL_INTERVENTION_REQUIRED`. */
  reason?: ManualInterventionReason;
}

/**
 * `NotificationService.StreamNotifications` scope — narrow the subscription to a
 * set of desks (intersected server-side with the caller's entitlement); empty ⇒
 * every entitled desk.
 */
export interface NotificationScope {
  desks: string[];
}

// --- Clearing Initial Margin (SPAN 2 / SIMM) ---

export type MarginProductFamily =
  | "BOND_FUTURE"
  | "INTEREST_RATE_SWAP"
  | "FX_FORWARD"
  | "EQUITY_OPTION"
  | "OIS_SWAP";

export interface ClearedPosition {
  symbol: string;
  productFamily: MarginProductFamily;
  quantity: number;
  contractSize?: number;
  initialMarginPerContract?: number;
  isShort?: boolean;
  currentPrice?: number;
  pnlScenarios?: number[];
}

export interface MarginCalculationRequest {
  portfolioId: string;
  positions: ClearedPosition[];
  confidenceLevel?: number;
  lookbackDays?: number;
}

export interface MarginCalculationResponse {
  portfolioId: string;
  totalInitialMargin: number;
  expectedShortfall: number;
  valueAtRisk: number;
  stressComponent: number;
  currency: string;
  calculatedEpochNanos: bigint;
}

export type PreTradeMarginOutcome = "APPROVED" | "WARNING" | "EXCEEDS_COLLATERAL";

export interface PreTradeMarginRequest {
  portfolioId: string;
  existingPositions?: ClearedPosition[];
  candidatePosition: ClearedPosition;
  availableCollateral: number;
  creditLine?: number;
  confidenceLevel?: number;
}

export interface PreTradeMarginResponse {
  portfolioId: string;
  outcome: PreTradeMarginOutcome;
  initialMarginBefore: number;
  initialMarginAfter: number;
  deltaMargin: number;
  collateralHeadroom: number;
  reason: string;
}

// --- Algorithmic Execution (TWAP, VWAP, Optimal Liquidation) ---

export type AlgoStrategyType = "TWAP" | "VWAP" | "OPTIMAL_LIQUIDATION" | "POV";

export type AlgoPeggingStyle = "PRIMARY" | "MIDPOINT" | "MARKET" | "NONE";

export type AlgoOrderStatus = "PENDING" | "ACTIVE" | "COMPLETED" | "CANCELLED";

export type ChildSliceStatus = "PENDING" | "DISPATCHED" | "FILLED" | "CANCELLED";

export interface ChildSlice {
  sliceIndex: number;
  scheduledOffsetSeconds: number;
  targetQuantity: number;
  filledQuantity: number;
  avgFillPrice: number;
  status: ChildSliceStatus;
}

export interface AlgoOrderResponse {
  parentOrderId: string;
  clientOrderId: string;
  symbol: string;
  totalQuantity: number;
  executedQuantity: number;
  arrivalPrice: number;
  avgExecPrice: number;
  isBuy: boolean;
  status: AlgoOrderStatus;
  implementationShortfallBps: number;
  slices: ChildSlice[];
  createdEpochNanos: bigint;
}

export interface SubmitAlgoOrderRequest {
  clientOrderId?: string;
  symbol: string;
  totalQuantity: number;
  arrivalPrice: number;
  isBuy: boolean;
  strategyType?: AlgoStrategyType;
  twap?: {
    durationSeconds: number;
    sliceCount: number;
    jitterFactor?: number;
    peggingStyle?: AlgoPeggingStyle;
  };
  optimal?: {
    horizonSeconds: number;
    stepCount: number;
    volatility: number;
    riskAversion: number;
    tempImpactEta: number;
    permImpactGamma: number;
  };
}

export interface ListAlgoOrdersResponse {
  orders: AlgoOrderResponse[];
}

// --- Cluster Lifecycle, Twin Upgrade & Chaos Resilience ---

export type NodeLifecycleStatus = "PENDING" | "ACTIVE" | "DRAINING" | "RETIRED" | "FAILED";

export interface NodeMember {
  nodeId: string;
  endpoint: string;
  status: NodeLifecycleStatus;
  activeInFlightTrades: bigint;
  joinedEpochNanos: bigint;
}

export interface ClusterTopologyResponse {
  clusterId: string;
  leaderId: string;
  activeGeneration: bigint;
  members: NodeMember[];
  jointConsensusActive: boolean;
}

export interface UpgradeStatusResponse {
  activeGeneration: bigint;
  currentVersion: string;
  shadowVersion: string;
  twinComparisonPassed: boolean;
  maxUlpDivergence: bigint;
  evaluatedTradesCount: bigint;
  cutoverStatus: string;
}

export interface TwinValidationResponse {
  passed: boolean;
  maxUlpDivergence: bigint;
  bitExact: boolean;
  verdict: string;
}

export interface ChaosTestResponse {
  clusterResilient: boolean;
  recoveryTimeMs: bigint;
  details: string;
}

// --- ISDA CDM 2026 Digital Trade Lifecycle Event ---

export interface ExportCdmResponse {
  executionId: bigint;
  uti: string;
  cdmEventType: string;
  cdmJson: string;
}

// --- Hardware Attestation & Licensing ---

export interface AttestationResponse {
  valid: boolean;
  attestationTimestampNanos: bigint;
  hardwareFingerprint: string;
  statusMessage: string;
}

export interface LicenseCapabilityResponse {
  valid: boolean;
  subject: string;
  tier: string;
  activeCapabilities: string[];
  expiryEpochSecs: bigint;
}

