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
