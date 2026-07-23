/**
 * The deterministic in-app mock/replay source. Implements the `CelnetTransport`
 * seam end-to-end so the GUI runs standalone now, with the live gRPC-Web/WS
 * client isolated behind the same interface (src/data/transport.ts).
 *
 * Everything is reproducible: a seeded PRNG (src/data/rng.ts) drives a
 * frame-clocked vol/price tape; prices and the 14-Greek set come from the
 * deterministic GK core (src/data/pricing.ts); surfaces are calibrated by
 * src/data/surface.ts; scenarios reprice on the same core. No randomness leaks
 * into the render path — the source ticks on a fixed cadence and pushes events.
 */

import type {
  AcceptDeskQuoteRequest,
  AcceptDeskQuoteResponse,
  AdditiveRisk,
  AggregateRatesRiskRequest,
  AggregateRatesRiskResponse,
  AggregateRiskRequest,
  AggregateRiskResponse,
  CombinedTailRiskRequest,
  CombinedTailRiskResponse,
  BookRatesPositionRequest,
  BookRatesPositionResponse,
  BrokerQuoteSet,
  Capability,
  CapabilityAction,
  CapabilityAsset,
  CcyExposureLeg,
  CcyPair,
  Conventions,
  CreateUserInput,
  EntityDesc,
  EntityInput,
  BookDesc,
  BookInput,
  AggregatedBookDesc,
  AggregatedBookSpec,
  AggregatedBookComposite,
  AggregatedInstrument,
  LpContribution,
  InstrumentDef,
  InstrumentInput,
  Deal,
  DealerQuote,
  DeskDesc,
  DeskRequest,
  DeskRequestKind,
  DeskRequestState,
  DrillRiskRequest,
  DrillRiskResponse,
  Executed,
  Execution,
  FixConnection,
  FixConnectionSpec,
  FixMessage,
  FixMessagePage,
  Greeks,
  ListDealsRequest,
  ListDealsResponse,
  ListDeskRequestsRequest,
  ListDeskRequestsResponse,
  ListRatesPositionsRequest,
  ListRatesPositionsResponse,
  LoginResult,
  Heartbeat,
  Instrument,
  LimitStatusRequest,
  LimitStatusResponse,
  ListPositionsRequest,
  ListPositionsResponse,
  MarkedSurface,
  MarketContext,
  MarketSeriesPoint,
  MultiDealerQuote,
  NonAdditiveRisk,
  Notification,
  NotificationKind,
  NotificationScope,
  OisInstrument,
  Quote,
  RatesCurveSet,
  RatesInstrument,
  BrokenDate,
  PillarTenor,
  BuildCurveRequest,
  CalibratedCurve,
  CalibratedCurvePoint,
  CurvePoint,
  CurveParPillar,
  GetCurveResult,
  MarkedCurve,
  CurveScenarioResult,
  CurveScenarioReprice,
  DatePillar,
  RatesPosition,
  RatesPricingResult,
  RatesQuote,
  RatesRiskNode,
  RatesStreamSnapshot,
  RatesStreamUpdate,
  ReportingNumeraire,
  RespondDeskRequestRequest,
  RespondDeskRequestResponse,
  RiskBucketRequest,
  RiskNode,
  ScenarioPoint,
  ScenarioResult,
  ShockAxis,
  ShockFactor,
  Side,
  Smile,
  SmileModel,
  Snapshot,
  SubmitDeskRequestRequest,
  SubmitDeskRequestResponse,
  TradableToken,
  TwoWayPrice,
  Update,
  UpdateUserInput,
  UserCapabilities,
  RoleCapabilities,
  UserDesc,
  UserRole,
  VegaBucket,
  XvaPricingRequest,
  XvaResult,
} from "./contract";
import { CAPABILITY_ACTIONS, CAPABILITY_ASSETS, pillarYears } from "./contract";
import { forward, priceInstrument, strikeFromDelta } from "./pricing";
import {
  DEFAULT_USD_SOFR_CURVE,
  combinedTailRiskOffline,
  priceRatesOffline,
  priceRatesInstrumentOffline,
  ratesRfqTwoWayOffline,
  bootstrapCurveFromSet,
  discountFactorAt,
  zeroRateAt,
  pillarMaturityYears,
} from "./ratesPricing";
import { computeXvaOffline } from "./xvaPricing";
import { Rng } from "./rng";
import {
  brokerLadder,
  DEFAULT_CONVENTIONS,
  PAIRS,
  seedSubscriptions,
  type PairContext,
} from "./seed";
import { calibrateSmile, markSurface } from "./surface";
import { tenorYearsOf } from "../lib/trend";
import type {
  CelnetTransport,
  MarketSeriesParams,
  PriceResult,
  StreamEvent,
  StreamSession,
} from "./transport";

const NS_PER_MS = 1_000_000n;

/** The desk the exception-contract sample notifications are attributed to. */
const SAMPLE_DESK = "g10-rates";
/** Delay before the quiet (alertWorthy:false) auto-priced sample fires (ms). */
const SAMPLE_QUIET_DELAY_MS = 1_500;
/** Delay before the growl-worthy manual-intervention sample fires (ms). */
const SAMPLE_ALERT_DELAY_MS = 3_000;

/**
 * The per-curve additive parallel-shift magnitude per tick, in decimal rate
 * (`0.0001` = ±1 basis point). The offline mirror of the server fan-out's
 * `RATE_STREAM_BUMP` (crates/celnet-server/src/services/pricefanout.rs): each
 * rates tick draws an i.i.d. uniform shift in `[-RATE_STREAM_BUMP,
 * RATE_STREAM_BUMP)` applied to every pillar par rate (the FI analogue of the
 * FX spot walk), so the streamed line breathes at a realistic curve scale.
 */
const RATE_STREAM_BUMP = 0.0001;

/**
 * A parallel-shifted copy of a curve: every OIS pillar's par rate moved by
 * `shift` (decimal). A `shift` of exactly `0` returns a value-identical clone, so
 * a re-price at shift 0 equals the un-shifted baseline exactly — the offline
 * mirror of the server's `shifted_curve` (crates/celnet-server/src/services
 * /stream.rs), which guarantees the baseline snapshot == `price_rates`.
 */
function shiftedRatesCurve(curve: RatesCurveSet, shift: number): RatesCurveSet {
  if (shift === 0) return curve;
  return {
    ...curve,
    pillars: curve.pillars.map((p) => ({ ...p, parRate: p.parRate + shift })),
  };
}

/** A wall-clock source in nanoseconds since epoch, monotone within a session. */
function nowNanos(): bigint {
  return (
    BigInt(Math.round(performance.timeOrigin + performance.now())) * NS_PER_MS
  );
}

function findPair(pair: CcyPair): PairContext {
  const found = PAIRS.find(
    (p) => p.pair.base === pair.base && p.pair.quote === pair.quote,
  );
  return found ?? PAIRS[0]!;
}

/** A premium two-way around a mid, with a convention-appropriate spread. */
function twoWayAround(midPct: number, spreadPct: number): TwoWayPrice {
  return { bid: midPct - spreadPct / 2, offer: midPct + spreadPct / 2 };
}

// ---------------------------------------------------------------------------
// multi-dealer panel (offline) — the SAME deterministic synthetic-LP law the
// server's LpPanelConfig demo panel applies (services/quote.rs), so the offline
// panel ranks identically to the live demo edge. Honest boundary: these are
// labeled deterministic synthetic demo dealers quoting around the SAME mock-
// priced mid — never a claim of live bank LP connectivity (that is ENV).
// ---------------------------------------------------------------------------

/** The native maker's audit `lpId` (mirrors the server's auto-pricer seat id). */
const MAKER_LP_ID = "celnet-auto-pricer";

/** Offline synthetic demo panel breadth (mirrors the demo edge's 3-LP default). */
const MOCK_SYNTHETIC_LPS = 3;

/** The stable audit `lpId` of synthetic demo dealer `k` (1-based). */
function syntheticLpId(k: number): string {
  return `SYNTH-LP-${k}`;
}

/**
 * The deterministic two-way synthetic demo dealer `k` (1-based) quotes around
 * the maker mid — the server's `synthetic_lp_two_way` law verbatim: dealer `k`
 * quotes `5%·k` wider than the maker half-spread and shades its mid by a quarter
 * half-spread (odd dealers up, even dealers down), so the panel is reproducible
 * and the touch is never crossed.
 */
function syntheticLpTwoWay(
  k: number,
  mid: number,
  halfSpread: number,
): TwoWayPrice {
  const widen = 1 + 0.05 * k;
  const shade = 0.25 * halfSpread;
  const skew = k % 2 === 1 ? shade : -shade;
  const m = mid + skew;
  const h = halfSpread * widen;
  return { bid: m - h, offer: m + h };
}

/**
 * The touch winner of one panel side — the engine's ranking law: highest bid /
 * lowest offer, deterministic lexicographic `lpId` tie-break. Empty ⇒ no rows.
 */
function rankSide(rows: readonly DealerQuote[], side: "BID" | "OFFER"): string {
  let won: DealerQuote | undefined;
  for (const row of rows) {
    if (!won) {
      won = row;
      continue;
    }
    const better =
      side === "BID"
        ? row.price.bid > won.price.bid ||
          (row.price.bid === won.price.bid && row.lpId < won.lpId)
        : row.price.offer < won.price.offer ||
          (row.price.offer === won.price.offer && row.lpId < won.lpId);
    if (better) won = row;
  }
  return won?.lpId ?? "";
}

/**
 * The `q`-quantile (0..1) of a sample of nanosecond latencies as a `bigint` ns —
 * nearest-rank on the sorted copy (matching an HdrHistogram's percentile lookup
 * closely enough for the standalone surfacing). Empty ⇒ 0n (no measurement yet).
 */
function percentileNs(samples: number[], q: number): bigint {
  if (samples.length === 0) return 0n;
  const sorted = [...samples].sort((a, b) => a - b);
  const idx = Math.min(
    sorted.length - 1,
    Math.max(0, Math.ceil(q * sorted.length) - 1),
  );
  return BigInt(Math.round(sorted[idx] ?? 0));
}

interface LiveSubscription {
  id: bigint;
  instrument: Instrument;
  conventions: Conventions;
  label: string;
  sequence: bigint;
  ctx: PairContext;
  /** Per-subscription vol random walk state (absolute vol). */
  vol: number;
  rng: Rng;
  health: "HEALTHY" | "RESYNCING" | "STALE";
  /** Frames until the next health perturbation (deterministic). */
  healthTimer: number;
  tokens: TradableToken[];
  /**
   * Bounded ring of REAL measured price-compute durations (ns) for this line —
   * the same drain-side latency the server reports on its heartbeat, measured here
   * around the standalone `priceSub`. Used to compute an honest p50/p99/p99.9.
   */
  latencyRing: number[];
  /** Frames until the next liveness heartbeat for this subscription. */
  beatCountdown: number;
}

/** A live market-series subscription: a deterministic walk of one observable. */
interface LiveSeries {
  id: bigint;
  params: MarketSeriesParams;
  ctx: PairContext;
  sequence: bigint;
  /** Current value of the observable (its natural unit). */
  value: number;
  /** The mean-reversion anchor (the observable's central level). */
  anchor: number;
  /** Per-series volatility of the walk step (scaled to the observable). */
  stepScale: number;
  rng: Rng;
  /** Frames until the next appended point (honours the throttle hint, coarsely). */
  cadence: number;
  countdown: number;
}

/**
 * A live fixed-income (linear-rates) streaming line — the FI analogue of
 * {@link LiveSubscription}. The offline mirror of the server's `RatesSubscription`
 * (crates/celnet-server/src/services/stream.rs): it holds the baseline curve +
 * instrument and re-prices on each deterministic parallel-shift tick through the
 * SAME offline `price_rates` mirror the rates unary edge uses, so the baseline
 * (shift 0) equals `priceRatesInstrumentOffline(curveSet, instrument)` EXACTLY.
 */
interface LiveRatesSubscription {
  id: bigint;
  instrument: RatesInstrument;
  /** The subscribed baseline curve the deterministic parallel shift moves around. */
  curveSet: RatesCurveSet;
  label: string;
  sequence: bigint;
  /** Per-line deterministic PRNG driving the ±1bp parallel shift path. */
  rng: Rng;
}

/**
 * One instrument in the offline aggregated-book universe — the static identity
 * (`instrumentId`/`displayName`/`isin`/`cusip`) plus a base clean price the
 * synthetic member two-ways walk around. Prices are clean prices per 100 face,
 * exactly as the LP-SIM Treasury feed publishes.
 */
interface MockAggInstrumentSeed {
  instrumentId: string;
  displayName: string;
  isin: string;
  cusip: string;
  baseMid: number;
}

/**
 * A faithful offline Treasury universe for the aggregated-book composite — the
 * SAME shape the LP-SIM Treasury feed streams into the live server (clean prices
 * per 100 face, real-looking on-the-run identities). NOT a placeholder: the mock
 * consolidates synthetic member two-ways around these bases exactly as the server
 * consolidates the LP feed, so the offline price view exercises the whole render
 * path. The live transport reads the server's real composite instead.
 */
const MOCK_AGG_TREASURY_UNIVERSE: readonly MockAggInstrumentSeed[] = [
  { instrumentId: "912797KX5", displayName: "T-Bill 3M", isin: "US912797KX52", cusip: "912797KX5", baseMid: 98.72 },
  { instrumentId: "91282CJL6", displayName: "UST 2Y 4.25%", isin: "US91282CJL63", cusip: "91282CJL6", baseMid: 99.61 },
  { instrumentId: "91282CJK8", displayName: "UST 3Y 4.00%", isin: "US91282CJK80", cusip: "91282CJK8", baseMid: 99.18 },
  { instrumentId: "91282CJM4", displayName: "UST 5Y 4.125%", isin: "US91282CJM47", cusip: "91282CJM4", baseMid: 98.84 },
  { instrumentId: "91282CJN2", displayName: "UST 7Y 4.25%", isin: "US91282CJN20", cusip: "91282CJN2", baseMid: 98.05 },
  { instrumentId: "91282CJP7", displayName: "UST 10Y 4.375%", isin: "US91282CJP77", cusip: "91282CJP7", baseMid: 97.41 },
  { instrumentId: "912810UC8", displayName: "UST 30Y 4.625%", isin: "US912810UC80", cusip: "912810UC8", baseMid: 95.62 },
];

/** The default synthetic LP members when a book names none (the LP-SIM fleet). */
const MOCK_AGG_DEFAULT_MEMBERS: readonly string[] = [
  "LP-SIM-01",
  "LP-SIM-02",
  "LP-SIM-03",
  "LP-SIM-04",
];

/** A live offline aggregated-book composite line. */
interface LiveAggBook {
  id: bigint;
  bookId: string;
  /** The members whose synthetic two-ways feed the composite (resolved at subscribe). */
  members: readonly string[];
  /** The in-scope instruments (the universe, filtered to an EXPLICIT book's scope). */
  seeds: readonly MockAggInstrumentSeed[];
  sequence: bigint;
  rng: Rng;
}

/**
 * Consolidate the members' synthetic two-ways for one instrument into a composite
 * line — the offline mirror of the server's `celnet-aggregation` pass. Each member
 * quotes a tight two-way around the instrument's base mid (a small deterministic
 * offset + spread); one member per instrument is occasionally marked STALE and
 * excluded from the best price (exactly the wire semantics). The best bid is the
 * max fresh member bid; the best offer the min fresh member offer; confidence is
 * coverage · agreement over the fresh members.
 */
function synthAggInstrument(
  seed: MockAggInstrumentSeed,
  members: readonly string[],
  rng: Rng,
): AggregatedInstrument {
  const contributions: LpContribution[] = [];
  // Deterministically stale at most one member (when > 2 members quote) so the
  // exclusion path is exercised without ever dropping below the quorum.
  const staleIdx = members.length > 2 && rng.next() < 0.35 ? Math.floor(rng.next() * members.length) : -1;
  let bestBid = Number.NEGATIVE_INFINITY;
  let bestOffer = Number.POSITIVE_INFINITY;
  let bidSize = 0;
  let offerSize = 0;
  let freshCount = 0;
  members.forEach((lpName, i) => {
    // A per-member mid skew kept STRICTLY below the half-spread so the consolidated
    // top-of-book stays uncrossed (max fresh bid < min fresh offer) — a healthy
    // composite, exactly as divergence gating would keep it. The dispersion still
    // shows in each member's own two-way (the per-LP breakdown).
    const skew = (rng.next() * 2 - 1) * 0.012;
    const halfSpread = 0.025 + rng.next() * 0.015;
    const mid = seed.baseMid + skew;
    const bid = round3(mid - halfSpread);
    const offer = round3(mid + halfSpread);
    const stale = i === staleIdx;
    contributions.push({ lpName, bid, offer, stale });
    if (!stale) {
      freshCount += 1;
      if (bid > bestBid) {
        bestBid = bid;
        bidSize = 1_000_000 + Math.floor(rng.next() * 4) * 1_000_000;
      }
      if (offer < bestOffer) {
        bestOffer = offer;
        offerSize = 1_000_000 + Math.floor(rng.next() * 4) * 1_000_000;
      }
    }
  });
  // Confidence: coverage (fresh / total) tempered by two-way agreement (a tight
  // consolidated spread ⇒ high agreement). Clamped to [0, 1]; honest, not faked.
  const coverage = members.length === 0 ? 0 : freshCount / members.length;
  const spread = Number.isFinite(bestOffer - bestBid) ? bestOffer - bestBid : 1;
  const agreement = Math.max(0, 1 - spread / 0.5);
  const confidence = Math.max(0, Math.min(1, coverage * (0.6 + 0.4 * agreement)));
  return {
    instrumentId: seed.instrumentId,
    displayName: seed.displayName,
    isin: seed.isin,
    cusip: seed.cusip,
    bestBid: freshCount > 0 ? bestBid : 0,
    bestOffer: freshCount > 0 ? bestOffer : 0,
    bidSize,
    offerSize,
    confidence: round3(confidence),
    contributions,
  };
}

/** Round to 3 decimals (clean-price cent resolution) without float drift artifacts. */
function round3(x: number): number {
  return Math.round(x * 1000) / 1000;
}

/** Build the full composite body for a live aggregated-book line at this tick. */
function synthAggComposite(live: LiveAggBook): AggregatedBookComposite {
  return {
    bookId: live.bookId,
    instruments: live.seeds.map((seed) => synthAggInstrument(seed, live.members, live.rng)),
  };
}

/**
 * The mock multiplexed stream session. One instance multiplexes many
 * subscriptions over a single ticking loop, exactly as the contract's single
 * bidirectional `StreamSession` multiplexes by `SubscriptionId`.
 */
class MockStreamSession implements StreamSession {
  private readonly subs = new Map<bigint, LiveSubscription>();
  /** Live market-series subscriptions (same id space as price streams). */
  private readonly series = new Map<bigint, LiveSeries>();
  /** Live fixed-income streaming lines (same id space as price/market streams). */
  private readonly ratesSubs = new Map<bigint, LiveRatesSubscription>();
  /** Live aggregated-book composite lines (same id space as the other streams). */
  private readonly aggBooks = new Map<bigint, LiveAggBook>();
  private readonly listeners = new Set<(e: StreamEvent) => void>();
  private nextSubId = 1n;
  private nextToken = 1n;
  private frame = 0;
  private timer: ReturnType<typeof setInterval> | undefined;
  private readonly consumedTokens = new Set<bigint>();
  private readonly tickMs: number;
  private readonly seed: bigint;
  /**
   * Resolve an aggregated-book definition by id (the transport's in-memory
   * store), so the offline composite reflects the admin-created book's actual
   * members + instrument scope. Returns undefined for an unknown id (the mock
   * then falls back to the LP-SIM fleet over the full Treasury universe).
   */
  private readonly resolveAggBook: (id: string) => AggregatedBookDesc | undefined;

  constructor(
    seed: bigint,
    tickMs: number,
    resolveAggBook: (id: string) => AggregatedBookDesc | undefined = () => undefined,
  ) {
    this.seed = seed;
    this.tickMs = tickMs;
    this.resolveAggBook = resolveAggBook;
  }

  private emit(event: StreamEvent): void {
    for (const l of this.listeners) l(event);
  }

  onEvent(listener: (event: StreamEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  subscribe(
    instrument: Instrument,
    conventions: Conventions,
    label: string,
  ): bigint {
    const id = this.nextSubId;
    this.nextSubId += 1n;
    const ctx = findPair(instrument.pair);
    const sub: LiveSubscription = {
      id,
      instrument,
      conventions,
      label,
      sequence: 0n,
      ctx,
      vol: ctx.market.vol,
      rng: new Rng(this.seed ^ (id * 0x100_0001n)),
      health: "HEALTHY",
      healthTimer: 40 + Number(id % 11n) * 7,
      tokens: [],
      latencyRing: [],
      // Stagger the first beat across lines so they don't all fire on one frame.
      beatCountdown: 12 + Number(id % 7n) * 3,
    };
    this.subs.set(id, sub);
    // Immediate baseline snapshot.
    this.emit({ kind: "snapshot", snapshot: this.buildSnapshot(sub) });
    this.ensureRunning();
    return id;
  }

  unsubscribe(subscriptionId: bigint): void {
    this.subs.delete(subscriptionId);
    this.stopIfIdle();
  }

  execute(
    subscriptionId: bigint,
    token: bigint,
    _idempotencyKey: string,
  ): void {
    const sub = this.subs.get(subscriptionId);
    const now = nowNanos();
    if (!sub) {
      this.emit({
        kind: "reject",
        reject: {
          subscriptionId,
          token,
          reason: "UNKNOWN_TOKEN",
          epochNanos: now,
        },
      });
      return;
    }
    if (this.consumedTokens.has(token)) {
      this.emit({
        kind: "reject",
        reject: {
          subscriptionId,
          token,
          reason: "ALREADY_CONSUMED",
          epochNanos: now,
        },
      });
      return;
    }
    const matched = sub.tokens.find((t) => t.token === token);
    if (!matched) {
      this.emit({
        kind: "reject",
        reject: {
          subscriptionId,
          token,
          reason: "UNKNOWN_TOKEN",
          epochNanos: now,
        },
      });
      return;
    }
    if (matched.validUntilNanos <= now) {
      this.emit({
        kind: "reject",
        reject: { subscriptionId, token, reason: "EXPIRED", epochNanos: now },
      });
      return;
    }
    this.consumedTokens.add(token);
    const executed: Executed = {
      subscriptionId,
      token,
      executionId: token ^ 0xbeefn,
      side: matched.side,
      tradedPremium: matched.premium,
      epochNanos: now,
    };
    this.emit({ kind: "executed", executed });
  }

  subscribeMarketSeries(params: MarketSeriesParams): bigint {
    const id = this.nextSubId;
    this.nextSubId += 1n;
    const ctx = findPair(params.pair);
    const { anchor, stepScale } = this.observableAnchor(ctx, params);
    // A short deterministic history (oldest → newest) seeds the snapshot so the
    // trend tile draws a line immediately rather than waiting for live points.
    const rng = new Rng(this.seed ^ (id * 0x51ed_0b5en));
    const HISTORY = Math.min(
      48,
      params.historyLimit && params.historyLimit > 0 ? params.historyLimit : 24,
    );
    let value = anchor;
    const now = nowNanos();
    const stepNanos = 1_000_000_000n; // 1s spacing for the seeded history
    const points: MarketSeriesPoint[] = [];
    let seq = 0n;
    for (let i = HISTORY; i >= 1; i -= 1) {
      seq += 1n;
      value += 0.04 * (anchor - value) + stepScale * rng.normal();
      points.push({
        subscriptionId: id,
        sequence: seq,
        value,
        epochNanos: now - BigInt(i) * stepNanos,
      });
    }
    // The throttle hint coarsely maps to a frame cadence (>= 1 frame); a 0 hint
    // appends every few frames so the line breathes without flooding.
    const throttleMs = Number(params.throttleNanos ?? 0n) / 1_000_000;
    const cadence = Math.max(1, Math.round(throttleMs / this.tickMs) || 3);
    const live: LiveSeries = {
      id,
      params,
      ctx,
      sequence: seq,
      value,
      anchor,
      stepScale,
      rng,
      cadence,
      countdown: cadence,
    };
    this.series.set(id, live);
    this.emit({
      kind: "marketSeriesSnapshot",
      snapshot: {
        subscriptionId: id,
        sequence: seq,
        pair: params.pair,
        observable: params.observable,
        points,
        epochNanos: now,
      },
    });
    this.ensureRunning();
    return id;
  }

  unsubscribeMarketSeries(subscriptionId: bigint): void {
    this.series.delete(subscriptionId);
    this.stopIfIdle();
  }

  subscribeRates(
    instrument: RatesInstrument,
    curveSet: RatesCurveSet,
    label: string,
  ): bigint {
    const id = this.nextSubId;
    this.nextSubId += 1n;
    const sub: LiveRatesSubscription = {
      id,
      instrument,
      curveSet,
      label,
      sequence: 0n,
      rng: new Rng(this.seed ^ (id * 0x9e37_79b9n)),
    };
    this.ratesSubs.set(id, sub);
    // Immediate baseline snapshot at shift 0 — byte-for-byte the offline
    // `price_rates(instrument, curve_set)`, faithful to the server contract.
    this.emit({ kind: "ratesSnapshot", snapshot: this.buildRatesSnapshot(sub) });
    this.ensureRunning();
    return id;
  }

  unsubscribeRates(subscriptionId: bigint): void {
    this.ratesSubs.delete(subscriptionId);
    this.stopIfIdle();
  }

  subscribeAggregatedBook(bookId: string, throttleNanos = 0n): bigint {
    void throttleNanos; // the offline mock ticks at its own cadence
    const id = this.nextSubId;
    this.nextSubId += 1n;
    // Resolve the book's actual members + instrument scope so the composite the
    // offline view renders is faithful to the admin-created definition. An unknown
    // book (or one that names no members) falls back to the LP-SIM fleet.
    const def = this.resolveAggBook(bookId);
    const members =
      def && def.memberConnectionIds.length > 0
        ? def.memberConnectionIds
        : MOCK_AGG_DEFAULT_MEMBERS;
    const seeds =
      def && def.scopeMode === "EXPLICIT" && def.instrumentIds.length > 0
        ? MOCK_AGG_TREASURY_UNIVERSE.filter((s) =>
            def.instrumentIds.includes(s.instrumentId),
          )
        : MOCK_AGG_TREASURY_UNIVERSE;
    const live: LiveAggBook = {
      id,
      bookId,
      members,
      seeds,
      sequence: 0n,
      rng: new Rng(this.seed ^ (id * 0xc2b2_ae35n)),
    };
    this.aggBooks.set(id, live);
    // Immediate baseline snapshot (sequence 1), exactly as the server emits.
    live.sequence += 1n;
    this.emit({
      kind: "aggregatedBookSnapshot",
      snapshot: {
        subscriptionId: id,
        sequence: live.sequence,
        book: synthAggComposite(live),
        epochNanos: nowNanos(),
      },
    });
    this.ensureRunning();
    return id;
  }

  unsubscribeAggregatedBook(subscriptionId: bigint): void {
    this.aggBooks.delete(subscriptionId);
    this.stopIfIdle();
  }

  /**
   * Build the baseline [`RatesStreamSnapshot`] for a line: the offline
   * `price_rates` at the un-shifted baseline curve (shift 0), so a consumer's
   * baseline equals `priceRatesInstrumentOffline(curveSet, instrument)` exactly.
   */
  private buildRatesSnapshot(sub: LiveRatesSubscription): RatesStreamSnapshot {
    sub.sequence += 1n;
    const result: RatesPricingResult = priceRatesInstrumentOffline(
      sub.curveSet,
      sub.instrument,
    );
    return {
      subscriptionId: sub.id,
      sequence: sub.sequence,
      result,
      curveShift: 0,
      epochNanos: nowNanos(),
    };
  }

  /** Stop the tick timer once no price / market-series / rates / composite line remains. */
  private stopIfIdle(): void {
    if (
      this.subs.size === 0 &&
      this.series.size === 0 &&
      this.ratesSubs.size === 0 &&
      this.aggBooks.size === 0 &&
      this.timer !== undefined
    ) {
      clearInterval(this.timer);
      this.timer = undefined;
    }
  }

  /**
   * The central level + step scale for an observable, derived from the pair's
   * market state and broker ladder — NOT invented. ATM_VOL/RR/BF read the nearest
   * broker tenor; SPOT reads the pair spot; FORWARD reads the outright forward at
   * the tenor. The step scale is sized to the observable's natural unit so the
   * walk moves realistically (a few vol-bps for vols, a few pips for rates).
   */
  private observableAnchor(
    ctx: PairContext,
    params: MarketSeriesParams,
  ): { anchor: number; stepScale: number } {
    const tenorYears = params.tenor ? tenorYearsOf(params.tenor) : 1 / 12;
    const ladder = brokerLadder(ctx);
    const nearest = ladder.reduce((best, q) =>
      Math.abs(q.tenorYears - tenorYears) <
      Math.abs(best.tenorYears - tenorYears)
        ? q
        : best,
    );
    const wing10 =
      nearest.hasTenDelta && Math.abs(params.delta ?? 0.25) <= 0.18;
    switch (params.observable) {
      case "ATM_VOL":
        return { anchor: nearest.atmVol, stepScale: 0.0006 };
      case "RISK_REVERSAL":
        return {
          anchor: wing10 ? nearest.rr10 : nearest.rr25,
          stepScale: 0.0004,
        };
      case "BUTTERFLY":
        return {
          anchor: wing10 ? nearest.bf10 : nearest.bf25,
          stepScale: 0.0003,
        };
      case "SPOT":
        return { anchor: ctx.market.spot, stepScale: ctx.market.spot * 0.0004 };
      case "FORWARD":
        return {
          anchor: forward(ctx.market, tenorYears),
          stepScale: ctx.market.spot * 0.0004,
        };
    }
  }

  close(): void {
    if (this.timer !== undefined) {
      clearInterval(this.timer);
      this.timer = undefined;
    }
    this.subs.clear();
    this.series.clear();
    this.ratesSubs.clear();
    this.aggBooks.clear();
    this.listeners.clear();
  }

  private ensureRunning(): void {
    if (this.timer !== undefined) return;
    this.timer = setInterval(() => this.tick(), this.tickMs);
  }

  private mintTokens(
    sub: LiveSubscription,
    price: TwoWayPrice,
  ): TradableToken[] {
    const validUntil = nowNanos() + 6_000n * NS_PER_MS; // 6s last-look window
    const sell: TradableToken = {
      token: this.nextToken++,
      side: "SELL",
      premium: price.bid,
      validUntilNanos: validUntil,
    };
    const buy: TradableToken = {
      token: this.nextToken++,
      side: "BUY",
      premium: price.offer,
      validUntilNanos: validUntil,
    };
    sub.tokens = [sell, buy];
    return sub.tokens;
  }

  private priceSub(sub: LiveSubscription): {
    price: TwoWayPrice;
    greeks: Greeks;
    strike: number;
  } {
    const market: MarketContext = { ...sub.ctx.market, vol: sub.vol };
    const { greeks, resolvedStrike } = priceInstrument(sub.instrument, market);
    // Premium as percent-of-foreign: GK price is per unit base in domestic; for a
    // %-foreign display we normalize by spot. Strategies sum signed leg premia.
    const midPct = Math.abs(greeks.price / market.spot) * 100;
    // Spread scales with vega magnitude and tenor (wider for longer-dated/illiquid).
    const spread = Math.max(
      0.004,
      Math.abs(greeks.vega) * 0.06 + sub.instrument.expiryYears * 0.02,
    );
    return {
      price: twoWayAround(midPct, spread),
      greeks,
      strike: resolvedStrike,
    };
  }

  private buildSnapshot(sub: LiveSubscription): Snapshot {
    const { price, greeks, strike } = this.priceSub(sub);
    sub.sequence += 1n;
    return {
      subscriptionId: sub.id,
      sequence: sub.sequence,
      price,
      greeks,
      vol: sub.vol,
      conventions: sub.conventions,
      resolvedStrike: strike,
      tradable: this.mintTokens(sub, price),
      surfaceVersion: 1n,
      epochNanos: nowNanos(),
    };
  }

  /**
   * Build a liveness [`Heartbeat`] for a subscription with HONEST observability:
   * real p50/p99/p99.9 (ns) from the line's measured latency ring, the live
   * surface_version (1), correlation 0 (the mock opens with none), and
   * conflation_drops=0 — the standalone mock has no fan-out ring, so it reports 0
   * truthfully rather than inventing a drop count. (The live WS transport carries
   * the server's real ring skip count straight through.)
   */
  private buildHeartbeat(sub: LiveSubscription): Heartbeat {
    return {
      subscriptionId: sub.id,
      sequence: sub.sequence,
      conflationDrops: 0n,
      serverPriceP50Nanos: percentileNs(sub.latencyRing, 0.5),
      serverPriceP99Nanos: percentileNs(sub.latencyRing, 0.99),
      serverPriceP999Nanos: percentileNs(sub.latencyRing, 0.999),
      surfaceVersion: 1n,
      correlationId: 0n,
      epochNanos: nowNanos(),
    };
  }

  private tick(): void {
    this.frame += 1;
    // Advance every live fixed-income line: draw one deterministic parallel curve
    // shift in [-1bp, +1bp) (the offline mirror of the server fan-out's per-curve
    // shift path), re-price the baseline curve shifted by it through the SAME
    // offline `price_rates` mirror, and emit ONE conflated `RatesStreamUpdate`
    // (the FI analogue of the FX one-Update-per-pass conflation throttle). No
    // click-to-trade token — rates click-to-trade books through RFQ/desk.
    for (const rs of this.ratesSubs.values()) {
      const shift = (rs.rng.next() * 2 - 1) * RATE_STREAM_BUMP;
      let result: RatesPricingResult;
      try {
        result = priceRatesInstrumentOffline(
          shiftedRatesCurve(rs.curveSet, shift),
          rs.instrument,
        );
      } catch {
        // A transient shifted-curve re-price failure SKIPS this pass (honest:
        // never a fabricated or stale point), exactly as the server driver does.
        continue;
      }
      rs.sequence += 1n;
      const update: RatesStreamUpdate = {
        subscriptionId: rs.id,
        sequence: rs.sequence,
        result,
        curveShift: shift,
        epochNanos: nowNanos(),
      };
      this.emit({ kind: "ratesUpdate", update });
    }
    // Advance every live aggregated-book composite line: re-consolidate the
    // members' synthetic two-ways into a fresh composite and emit ONE conflated
    // `AggregatedBookStreamUpdate` (the offline mirror of the server's per-book
    // re-consolidation on member re-quote).
    for (const ab of this.aggBooks.values()) {
      ab.sequence += 1n;
      this.emit({
        kind: "aggregatedBookUpdate",
        update: {
          subscriptionId: ab.id,
          sequence: ab.sequence,
          book: synthAggComposite(ab),
          epochNanos: nowNanos(),
        },
      });
    }
    // Advance every live market series: a deterministic mean-reverting walk around
    // the observable's anchor, appended at the series' cadence (throttle hint).
    for (const s of this.series.values()) {
      s.countdown -= 1;
      if (s.countdown > 0) continue;
      s.countdown = s.cadence;
      s.value += 0.05 * (s.anchor - s.value) + s.stepScale * s.rng.normal();
      if (
        s.params.observable === "ATM_VOL" ||
        s.params.observable === "BUTTERFLY"
      ) {
        s.value = Math.max(0.0001, s.value); // vols/flies stay positive
      }
      s.sequence += 1n;
      this.emit({
        kind: "marketSeriesPoint",
        point: {
          subscriptionId: s.id,
          sequence: s.sequence,
          value: s.value,
          epochNanos: nowNanos(),
        },
      });
    }
    for (const sub of this.subs.values()) {
      // Deterministic health cycling so the blotter shows honest seq/resync state.
      sub.healthTimer -= 1;
      if (sub.healthTimer <= 0) {
        if (sub.health === "HEALTHY") {
          sub.health = "RESYNCING";
          sub.healthTimer = 6;
        } else if (sub.health === "RESYNCING") {
          sub.health = "HEALTHY";
          sub.healthTimer = 50 + Number(sub.id % 9n) * 9;
          // A resync re-baselines with a fresh snapshot at the next sequence.
          this.emit({ kind: "snapshot", snapshot: this.buildSnapshot(sub) });
        }
        this.emit({
          kind: "health",
          subscriptionId: sub.id,
          health: sub.health,
        });
      }
      if (sub.health === "RESYNCING") continue;

      // Emit a liveness heartbeat on cadence — carrying the current sequence PLUS
      // honest observability: a real measured price-compute p50/p99/p99.9 (ns) from
      // this line's latency ring, surface_version=1 (the mock's single live mark),
      // and conflation_drops=0 (the standalone mock has no fan-out ring to drop from
      // — reported honestly as 0, never a fabricated non-zero).
      sub.beatCountdown -= 1;
      if (sub.beatCountdown <= 0) {
        sub.beatCountdown = 24 + Number(sub.id % 5n) * 6;
        this.emit({ kind: "heartbeat", heartbeat: this.buildHeartbeat(sub) });
      }

      // Only a fraction of subscriptions tick each frame (calm under fire): the
      // tape is bursty but the blotter flashes only the changed lines.
      const draw = sub.rng.next();
      if (draw > 0.55) continue;

      // Mean-reverting vol random walk (Ornstein-Uhlenbeck-flavoured), deterministic.
      const anchor = sub.ctx.market.vol;
      sub.vol += 0.04 * (anchor - sub.vol) + 0.0006 * sub.rng.normal();
      sub.vol = Math.max(0.01, sub.vol);

      // Time the standalone price-compute — a REAL drain-side measurement, mirroring
      // the server's per-subscription LatencyRecorder. Push into a bounded ring.
      const t0 = performance.now();
      const { price, greeks, strike } = this.priceSub(sub);
      const elapsedNs = Math.max(
        0,
        Math.round((performance.now() - t0) * 1_000_000),
      );
      sub.latencyRing.push(elapsedNs);
      if (sub.latencyRing.length > 256) sub.latencyRing.shift();
      sub.sequence += 1n;
      const update: Update = {
        subscriptionId: sub.id,
        sequence: sub.sequence,
        price,
        greeks,
        vol: sub.vol,
        tradable: this.mintTokens(sub, price),
        surfaceVersion: 1n,
        epochNanos: nowNanos(),
      };
      void strike;
      this.emit({ kind: "update", update });
    }
  }
}

/** The standalone mock transport. Construct once and inject at the app root. */
export class MockTransport implements CelnetTransport {
  readonly label = "mock/replay";
  private readonly seed: bigint;
  private readonly tickMs: number;
  private surfaceVersion = 1n;
  /** The monotonic marked-curve version authority (SurfaceService.MarkCurve). */
  private curveVersion = 0n;
  /**
   * The offline marked-curve store: a `MarkCurve` deposits the bootstrapped
   * `curveSet` here keyed by its assigned version, so a later `GetCurve` pinned to
   * that version re-bootstraps + reproduces the exact same curve (the offline
   * mirror of the server's version-pinned read).
   */
  private readonly markedCurves = new Map<bigint, RatesCurveSet>();
  private quoteSeq = 1n;
  /** Stored quotes; `dealers` is pinned by a multi-dealer request so an accept naming an `lpId` books exactly the line shown. */
  private readonly quotes = new Map<
    bigint,
    { quote: Quote; instrument: Instrument; dealers?: DealerQuote[] }
  >();
  private readonly idempotency = new Map<string, Quote>();
  /**
   * The offline managed-FIX registry: an in-memory list mirroring the server's
   * `FixAcceptorRegistry` semantics (unique id/name, address conflict, slug mint)
   * so the `?mock` GUI exercises the Connections workspace + wizard offline. Seeded
   * with one example Options acceptor.
   */
  private readonly fixConnections: FixConnection[] = [
    {
      id: "demo-options",
      name: "Demo bank — Options",
      kind: "OPTIONS",
      bindAddr: "127.0.0.1:9099",
      senderCompId: "CELNET",
      targetCompId: "CELNET-CPTY",
      enabled: true,
      running: true,
      boundAddr: "127.0.0.1:9099",
      desk: "g10",
    },
  ];

  /**
   * The bearer session token the auth flow installs, mirroring the live
   * transport. The offline mock does not enforce authentication on data RPCs, so
   * it simply retains the token for parity (and so a `?mock` login round-trips);
   * gated mock calls behave identically with or without it.
   */
  private sessionToken: string | null = null;

  /**
   * The offline identity store: an in-memory user/desk roster mirroring the
   * server's `AuthService` semantics so the `?mock` GUI exercises the Admin
   * workspace + sign-in offline. Seeded with the SAME default admin the server
   * seeds on first run (`admin@celnet.com` / `password`). The plaintext password
   * is held here ONLY for the offline mock — the real server stores Argon2id
   * hashes and never round-trips a password. No desks are seeded (parity with
   * the server seed); an admin creates them in the workspace.
   */
  private readonly mockUsers: {
    user: UserDesc;
    password: string;
    /** Per-user capability overlay (empty until an admin edits it). */
    grants: Capability[];
    denies: Capability[];
  }[] = [
    {
      user: {
        id: "admin",
        email: "admin@celnet.com",
        displayName: "Administrator",
        role: "ADMIN",
        deskIds: [],
        allDesks: false,
        disabled: false,
      },
      password: "password",
      grants: [],
      denies: [],
    },
  ];
  private readonly mockDesks: DeskDesc[] = [];
  /**
   * The offline legal-entity / netting-book registry, seeded to MIRROR the
   * server's default registry (`celnet-server` `IdentityStore::seed_registry`) so
   * the rates booking form's named dropdowns and the Book/blotter name-resolution
   * behave identically with no server. Stores are real (admin CRUD mutates them);
   * `next*Key` mints the lowest free key, exactly like the server.
   */
  private readonly mockEntities: EntityDesc[] = [
    { key: 1, name: "Celnet Global Markets", code: "CGM" },
    { key: 2, name: "Celnet Securities", code: "CSEC" },
  ];
  private readonly mockBooks: BookDesc[] = [
    { key: 1, name: "Rates Trading", entityKey: 1 },
    { key: 2, name: "Rates Relative Value", entityKey: 1 },
    { key: 3, name: "Government Bonds", entityKey: 2 },
    { key: 4, name: "Swaps", entityKey: 2 },
  ];
  /**
   * The offline aggregated-book registry (a GENUINE in-memory store, not a stub):
   * admin CRUD mutates it and `openStreamSession` reads it so the offline price
   * view renders the composite for the admin-created book. Seeded with one book
   * over the LP-SIM fleet so the price view is exercisable end-to-end with no
   * server; the live server owns its own store.
   */
  private readonly mockAggregatedBooks: AggregatedBookDesc[] = [
    {
      id: "us-treasuries",
      name: "US Treasuries",
      memberConnectionIds: ["LP-SIM-01", "LP-SIM-02", "LP-SIM-03", "LP-SIM-04"],
      scopeMode: "ALL_MEMBERS_QUOTE",
      instrumentIds: [],
      params: {
        stalenessTauMs: 2000,
        maxQuoteAgeMs: 5000,
        divergenceGating: true,
        minContributors: 2,
        depthLevels: 1,
      },
      enabled: true,
    },
  ];
  /**
   * The offline instrument reference-data registry (a GENUINE in-memory store,
   * not a stub): seeded with one OIS and one bond definition so the Reference
   * Data workspace is exercisable end-to-end with no server. Each carries a
   * server-style minted `instrumentId` and real external identifiers.
   */
  private readonly mockInstruments: InstrumentDef[] = [
    {
      instrumentId: "usd-sofr-ois-5y",
      name: "USD SOFR OIS 5Y",
      description: "USD overnight-indexed swap vs SOFR, 5Y",
      currency: "USD",
      externalIds: [{ scheme: "ticker", value: "USOSFR5" }],
      family: "ois",
      ois: {
        tenor: "5Y",
        index: "SOFR",
        fixedFrequency: "annual",
        fixedDayCount: "act_360",
        floatDayCount: "act_360",
        businessDayConvention: "modified_following",
        calendars: ["united_states"],
        spotLagDays: 2,
      },
    },
    {
      instrumentId: "us-treasury-4-25-2035",
      name: "US Treasury 4.25% 2035",
      description: "US Treasury note, 4.25% semi-annual coupon, maturing 2035",
      currency: "USD",
      externalIds: [
        { scheme: "isin", value: "US91282CHK24" },
        { scheme: "cusip", value: "91282CHK2" },
      ],
      family: "bond",
      bond: {
        issuer: "US Treasury",
        couponRate: 4.25,
        couponType: "fixed",
        couponFrequency: "semi_annual",
        dayCount: "act_act",
        issueDate: { year: 2025, month: 2, day: 15 },
        datedDate: { year: 2025, month: 2, day: 15 },
        firstCouponDate: { year: 2025, month: 8, day: 15 },
        maturityDate: { year: 2035, month: 2, day: 15 },
        redemption: 100,
        calendars: ["united_states"],
      },
    },
  ];
  /**
   * The admin-editable per-role capability bundles (the role's base authority).
   * Only **non-admin** roles are ever stored (`ADMIN` is grant-all and immutable); a
   * role absent here resolves to {@link mockDefaultTraderBundle}, mirroring the
   * server's `IdentityStore::role_bundles` serde-default behaviour.
   */
  private readonly mockRoleBundles = new Map<UserRole, Capability[]>();
  /** Issued session tokens (offline liveness for `logout`'s `ended` result). */
  private readonly mockTokens = new Set<string>();
  private mockTokenSeq = 0n;

  // --- dealer-quoting desk + rates Book (offline, fully in-memory) -----------
  //
  // A GENUINE offline implementation of the RfqDeskService / rates-Book lifecycle
  // (not a stub): the desk inbox, the received-deals blotter and the booked rates
  // book are real in-memory stores with monotonic ids; submit enqueues + pushes a
  // notification; respond quotes/rejects; accept books a deal + a rates position;
  // every list reads. Notifications fan out to local subscribers (the GUI's
  // NotificationCenter), so the page is exercisable end-to-end with no server.

  /** The desk inbox: request_id → DeskRequest (PENDING → QUOTED/REJECTED → ACCEPTED). */
  private readonly deskRequests = new Map<string, DeskRequest>();
  /** The received-deals blotter: deal_id → Deal (insertion order = mint order). */
  private readonly deals = new Map<string, Deal>();
  /** The booked linear-rates book: position_id → RatesPosition. */
  private readonly ratesPositions = new Map<bigint, RatesPosition>();
  /** Live notification subscribers (the global NotificationCenter), with desk scope. */
  private readonly notificationSubs = new Set<{
    scope: NotificationScope | undefined;
    onNotification: (n: Notification) => void;
  }>();
  private deskRequestSeq = 1n;
  private dealSeq = 1n;
  private notificationSeq = 1n;
  private ratesPositionSeq = 1n;
  /** One-shot guard: the sample exception-contract alerts are scheduled once. */
  private sampleAlertsScheduled = false;
  /** Pending sample-alert timers, cleared when the last subscriber disposes. */
  private sampleAlertTimers: ReturnType<typeof setTimeout>[] = [];

  constructor(opts: { seed?: bigint; tickMs?: number } = {}) {
    this.seed = opts.seed ?? 0xce1_5eed_d00dn;
    this.tickMs = opts.tickMs ?? 100; // 10 Hz tape; render conflates to a frame
    this.seedOfflineDesk();
  }

  /** Retain the bearer session token (the offline mock does not enforce auth). */
  setSessionToken(token: string | null): void {
    this.sessionToken = token;
  }

  async price(
    instrument: Instrument,
    market: MarketContext,
    conventions: Conventions,
  ): Promise<PriceResult> {
    const { greeks, resolvedStrike, priceStdError } = priceInstrument(
      instrument,
      market,
    );
    const midPct = Math.abs(greeks.price / market.spot) * 100;
    const spread = Math.max(
      0.004,
      Math.abs(greeks.vega) * 0.06 + instrument.expiryYears * 0.02,
    );
    const result: PriceResult = {
      greeks,
      resolvedStrike,
      conventions,
      twoWay: twoWayAround(midPct, spread),
      surfaceVersion: this.surfaceVersion,
    };
    // Surface the MC standard error for an MC-priced product (a clamped cliquet)
    // exactly as the live server does (`PriceResponse.price_std_error`); a
    // closed-form product carries no stderr.
    if (priceStdError !== undefined) result.priceStdError = priceStdError;
    return result;
  }

  async priceRates(
    curve: RatesCurveSet,
    instrument: RatesInstrument,
  ): Promise<RatesPricingResult> {
    // A GENUINE in-browser linear-rates computation: bootstrap the self-discounting
    // curve from the par-OIS pillars and price the arm (OIS / IRS / FRA / bond),
    // reproducing the server's `celnet-rates` / `celnet-bond` math exactly so the
    // offline number agrees with the live `price_rates` RPC. A malformed
    // curve/instrument throws (mirroring the server refusal), surfaced by the
    // workspace exactly as a live transport error would be.
    return priceRatesInstrumentOffline(curve, instrument);
  }

  async priceXva(request: XvaPricingRequest): Promise<XvaResult> {
    // A GENUINE in-browser XVA computation: reproduce the server's `compute_xva`
    // aggregation (discounted expected exposure × marginal default probability ×
    // LGD, plus funding on the net exposure) over a deterministic-quadrature
    // exposure profile that reproduces `celnet_vanilla::price` marks and the
    // hazard-curve survival exactly. A malformed request throws (mirroring the
    // server refusal), surfaced by the workspace as a live transport error would
    // be. Honest boundary (see xvaPricing.ts): the offline estimator is quadrature,
    // the edge is Monte-Carlo — the two converge but are not bit-identical.
    return computeXvaOffline(request);
  }

  async requestQuote(
    instrument: Instrument,
    conventions: Conventions,
    idempotencyKey: string,
  ): Promise<Quote> {
    const existing = this.idempotency.get(idempotencyKey);
    if (existing) return existing;
    const ctx = findPair(instrument.pair);
    const result = await this.price(instrument, ctx.market, conventions);
    const now = nowNanos();
    const quote: Quote = {
      quoteId: this.quoteSeq++,
      idempotencyKey,
      price: result.twoWay,
      greeks: result.greeks,
      conventions,
      resolvedStrike: result.resolvedStrike,
      epochNanos: now,
      validUntilNanos: now + 8_000n * NS_PER_MS, // 8s RFQ last-look
      surfaceVersion: this.surfaceVersion,
    };
    // Carry the MC standard error onto the quote for an MC-priced product (a
    // clamped cliquet); a closed-form product leaves it undefined.
    if (result.priceStdError !== undefined)
      quote.priceStdError = result.priceStdError;
    this.quotes.set(quote.quoteId, { quote, instrument });
    this.idempotency.set(idempotencyKey, quote);
    return quote;
  }

  async requestMultiDealerQuote(
    instrument: Instrument,
    conventions: Conventions,
    idempotencyKey: string,
  ): Promise<MultiDealerQuote> {
    // Reuse the single-dealer pricing/idempotency path verbatim — the native
    // maker line is byte-identical to the `requestQuote` it mirrors — then fan
    // the deterministic synthetic demo dealers around the SAME mid (the server's
    // LpPanelConfig law) and rank them.
    const quote = await this.requestQuote(
      instrument,
      conventions,
      idempotencyKey,
    );
    const mid = (quote.price.bid + quote.price.offer) / 2;
    const halfSpread = (quote.price.offer - quote.price.bid) / 2;
    const native: DealerQuote = {
      lpId: MAKER_LP_ID,
      price: quote.price,
      greeks: quote.greeks,
      resolvedStrike: quote.resolvedStrike,
      validUntilNanos: quote.validUntilNanos,
    };
    if (quote.priceStdError !== undefined)
      native.priceStdError = quote.priceStdError;
    const dealers: DealerQuote[] = [native];
    for (let k = 1; k <= MOCK_SYNTHETIC_LPS; k += 1) {
      // A synthetic dealer discloses a price, not its greeks (honest absence).
      dealers.push({
        lpId: syntheticLpId(k),
        price: syntheticLpTwoWay(k, mid, halfSpread),
        resolvedStrike: quote.resolvedStrike,
        validUntilNanos: quote.validUntilNanos,
      });
    }
    // The engine's deterministic audit order: rows sorted by lpId.
    dealers.sort((a, b) => (a.lpId < b.lpId ? -1 : a.lpId > b.lpId ? 1 : 0));
    // Pin the issued rows on the stored record so an accept naming any lpId
    // books exactly the line shown (mirrors the server's quote-record pinning).
    const entry = this.quotes.get(quote.quoteId);
    if (entry) entry.dealers = dealers;
    const panel: MultiDealerQuote = {
      quoteId: quote.quoteId,
      idempotencyKey: quote.idempotencyKey,
      dealers,
      bestBidLpId: rankSide(dealers, "BID"),
      bestOfferLpId: rankSide(dealers, "OFFER"),
      conventions,
      epochNanos: quote.epochNanos,
    };
    if (quote.surfaceVersion !== undefined)
      panel.surfaceVersion = quote.surfaceVersion;
    return panel;
  }

  async requestRatesQuote(
    curve: RatesCurveSet,
    instrument: RatesInstrument,
    notional: number,
    side: Side,
    idempotencyKey: string,
  ): Promise<RatesQuote> {
    // A GENUINE in-browser fixed-income two-way: reproduce the server's
    // `quote_rates_two_way` — the side-independent fair level (par rate for an
    // OIS/IRS/FRA, clean price for a cash bond) split the same maker half-spread
    // either side, with the FULL `priceRates` risk signed to the RFQ envelope side —
    // so the offline two-way mid equals the offline `priceRates` mid EXACTLY and
    // agrees with the live `request_rates_quote` RPC. A malformed curve/instrument
    // throws (mirroring the server refusal), surfaced by the workspace as a live
    // transport error would be. Like the server handler, the quote is a pure
    // calculation and is not deduplicated on the idempotency key (a fresh quote id
    // + timestamps each call); the key is echoed for the taker's correlation.
    const two = ratesRfqTwoWayOffline(curve, instrument, side);
    const now = nowNanos();
    return {
      quoteId: this.quoteSeq++,
      idempotencyKey,
      price: { bid: two.bid, offer: two.offer },
      result: two.result,
      notional,
      epochNanos: now,
      validUntilNanos: now + 8_000n * NS_PER_MS, // 8s RFQ last-look
    };
  }

  async acceptQuote(
    quoteId: bigint,
    side: "BUY" | "SELL",
    _idempotencyKey: string,
    lpId?: string,
  ): Promise<Execution> {
    const entry = this.quotes.get(quoteId);
    if (!entry) throw new Error(`unknown quote ${quoteId}`);
    const now = nowNanos();
    // A non-empty, non-native lpId books that pinned panel row's line; an absent/
    // empty/native lpId books the single-dealer quote (the server's accept law).
    let line: { price: TwoWayPrice; validUntilNanos: bigint } = entry.quote;
    if (lpId !== undefined && lpId.length > 0 && lpId !== MAKER_LP_ID) {
      const row = entry.dealers?.find((d) => d.lpId === lpId);
      if (!row)
        throw new Error(`unknown dealer line ${lpId} on quote ${quoteId}`);
      line = row;
    }
    if (line.validUntilNanos <= now)
      throw new Error("quote expired (last-look)");
    const premium = side === "BUY" ? line.price.offer : line.price.bid;
    return {
      executionId: quoteId ^ 0xfacen,
      quoteId,
      side,
      tradedPremium: premium,
      instrument: entry.instrument,
      epochNanos: now,
    };
  }

  async rejectQuote(quoteId: bigint, _reason: string): Promise<void> {
    this.quotes.delete(quoteId);
  }

  openStreamSession(): StreamSession {
    // Pass a live resolver so an offline composite reflects the admin-created
    // book's actual members + instrument scope (the store the CRUD methods mutate).
    return new MockStreamSession(this.seed, this.tickMs, (id) =>
      this.mockAggregatedBooks.find((b) => b.id === id),
    );
  }

  async getSmile(
    pair: CcyPair,
    tenorYears: number,
    conventions: Conventions,
  ): Promise<Smile> {
    const ctx = findPair(pair);
    const ladder = brokerLadder(ctx);
    const nearest = ladder.reduce((best, q) =>
      Math.abs(q.tenorYears - tenorYears) <
      Math.abs(best.tenorYears - tenorYears)
        ? q
        : best,
    );
    return calibrateSmile(pair, nearest, conventions, nowNanos());
  }

  async markSurface(
    pair: CcyPair,
    brokerQuotes: BrokerQuoteSet[],
    conventions: Conventions,
    smileModel?: SmileModel,
  ): Promise<MarkedSurface> {
    this.surfaceVersion += 1n;
    return markSurface(
      pair,
      brokerQuotes,
      conventions,
      this.surfaceVersion,
      nowNanos(),
      smileModel,
    );
  }

  async scenario(
    instrument: Instrument,
    baseMarket: MarketContext,
    _conventions: Conventions,
    axes: ShockAxis[],
    riskBuckets?: RiskBucketRequest,
  ): Promise<ScenarioResult> {
    const points: ScenarioPoint[] = [];
    // Lock strikes to absolute levels at the base market so a spot/vol shock
    // moves a *fixed* position (a real book has fixed strikes), rather than
    // silently re-striking to the same delta on every shocked market — which
    // would neutralize the directional P&L the trader is shocking for.
    const fixed = freezeStrikes(instrument, baseMarket);

    // Cartesian product of the axes (the contract's grid semantics).
    const indices = axes.map(() => 0);
    const total = axes.reduce((acc, a) => acc * Math.max(1, a.steps.length), 1);
    for (let n = 0; n < total; n += 1) {
      const applied: number[] = [];
      let market: MarketContext = { ...baseMarket };
      let expiryYears = instrument.expiryYears;
      axes.forEach((axis, ai) => {
        const step = axis.steps[indices[ai]!] ?? 0;
        applied.push(step);
        market = applyShock(market, axis.factor, step, axis.relative);
        if (axis.factor === "TIME")
          expiryYears = Math.max(1 / 365, expiryYears - step);
      });
      const shockedInstrument: Instrument = { ...fixed, expiryYears };
      const greeks = priceInstrument(shockedInstrument, market).greeks;
      points.push({
        appliedShocks: applied,
        shockedMarket: market,
        greeks,
        expiryYears,
      });
      // Advance the mixed-radix index.
      for (let ai = axes.length - 1; ai >= 0; ai -= 1) {
        indices[ai] = (indices[ai]! + 1) % Math.max(1, axes[ai]!.steps.length);
        if (indices[ai] !== 0) break;
      }
    }

    // The book-shaped risk decomposition is computed ONLY when requested — exactly
    // like the live server, which returns `bucketed_risk: null` for a bare scenario.
    if (!riskBuckets) {
      return { points, bucketedRisk: null };
    }
    const vegaBuckets: VegaBucket[] = riskBuckets.vegaPillars.map((p) => {
      const strike = strikeFromDelta(p.delta, baseMarket, p.tenorYears);
      const bumped: Instrument = {
        ...instrument,
        expiryYears: p.tenorYears,
        product: {
          kind: "vanilla",
          vanilla: {
            optionType: p.delta >= 0 ? "CALL" : "PUT",
            strike: { kind: "strike", strike },
          },
        },
      };
      const vega = priceInstrument(bumped, baseMarket).greeks.vega;
      return { tenorYears: p.tenorYears, delta: p.delta, vega };
    });
    const crossGammas = riskBuckets.crossGammaPairs.map((pair) => ({
      factorA: pair.factorA,
      factorB: pair.factorB,
      value: crossGamma(instrument, baseMarket, pair.factorA, pair.factorB),
    }));
    // The theta roll is the *absolute* repriced value at each rolled expiry
    // (matching the server's `theta_roll`, which carries values, not differences);
    // the consumer differences against the base value to read the decay P&L.
    const thetaRoll = riskBuckets.rollHorizonsYears.map((h) => {
      const rolled: Instrument = {
        ...fixed,
        expiryYears: Math.max(1 / 365, fixed.expiryYears - h),
      };
      return priceInstrument(rolled, baseMarket).greeks.price;
    });
    return {
      points,
      bucketedRisk: {
        vegaBuckets,
        crossGammas,
        thetaRoll,
        rollHorizonsYears: riskBuckets.rollHorizonsYears,
      },
    };
  }

  // --- RiskService (offline) -------------------------------------------------
  //
  // In offline mode the mock IS the server: it aggregates its own deterministic
  // seed book (the same structures the RFS blotter streams) GENUINELY — pricing
  // each position on the real GK core and collapsing every leg into the requested
  // reporting numeraire (src/data/contract.ReportingNumeraire). This is NOT a
  // placeholder: every number is a real repriced exposure, exactly as the live
  // server computes it; only the data source differs (a local seed book vs the
  // server's live `PositionStore`). The offline demo book carries no per-position
  // org attribution, so it rolls up to a SINGLE node (one firm/desk/book) — for
  // any group-by dimension the honest answer is that one node; the offline build
  // has no finer org structure to fabricate. Limits have no offline tree, so
  // `limitStatus` returns an honest empty utilization set.

  async listPositions(
    request: ListPositionsRequest,
  ): Promise<ListPositionsResponse> {
    const res: ListPositionsResponse = { positions: [] };
    if (request.correlationId !== undefined)
      res.correlationId = request.correlationId;
    // The offline book has no canonical-vanilla leaf per booked structure (the
    // seed positions are multi-leg strategies/vanillas without server-side
    // canonicalisation), so we honestly report no flat `RiskPosition` rows rather
    // than fabricate convention-free leaves the offline core cannot derive. The
    // aggregate (below) is the genuine offline risk view.
    return res;
  }

  async aggregateRisk(
    request: AggregateRiskRequest,
  ): Promise<AggregateRiskResponse> {
    const node = aggregateSeedBook(request.dimension, request.numeraire);
    const res: AggregateRiskResponse = {
      dimension: request.dimension,
      numeraire: request.numeraire.numeraire,
      nodes: node.positionCount > 0 ? [node] : [],
    };
    if (request.correlationId !== undefined)
      res.correlationId = request.correlationId;
    return res;
  }

  async aggregateRatesRisk(
    request: AggregateRatesRiskRequest,
    _conventions: Conventions,
  ): Promise<AggregateRatesRiskResponse> {
    // A genuine in-browser portfolio-risk rollup — NOT a fabricated stub. Each
    // position is priced through the SAME offline OIS core the live edge mirrors
    // (`priceRatesOffline`), and the signed PV / PV01 / DV01 + per-pillar key-rate
    // DV01 ladder are folded ADDITIVELY into one node per settlement currency.
    //
    // This reproduces the server's `services::rates_risk` semantics exactly: the
    // settlement currency is the curve currency (never carried per position); each
    // position is priced BEFORE the optional `(entity, book, ccy)` scope filter is
    // applied; the fold is purely additive and per-ccy partitioned; and the ladder
    // sums DV01 per curve-pillar tenor in ascending order (deterministic output).
    const curve = request.curveSet;
    const ccy = curve.currency;
    const scope = request.scope;

    // Accumulator: settlement ccy → netted scalars + a tenor→DV01 ladder map.
    interface RatesNodeAccum {
      netPv: number;
      netPv01: number;
      netDv01: number;
      ladder: Map<number, number>;
    }
    const byCcy = new Map<string, RatesNodeAccum>();

    for (const position of request.positions) {
      // Price every position (the server prices then filters); an unpriceable
      // position — e.g. a non-USD curve the offline core cannot bootstrap, or a
      // malformed OIS — rejects the whole request, exactly as the live edge does.
      const priced = priceRatesOffline(curve, position.instrument);

      const inScope =
        (scope?.entity === undefined || scope.entity === position.entity) &&
        (scope?.book === undefined || scope.book === position.book) &&
        (scope?.ccy === undefined ||
          scope.ccy.toUpperCase() === ccy.toUpperCase());
      if (!inScope) continue;

      let existing = byCcy.get(ccy);
      if (existing === undefined) {
        existing = { netPv: 0, netPv01: 0, netDv01: 0, ladder: new Map() };
        byCcy.set(ccy, existing);
      }
      const node = existing;
      node.netPv += priced.pv;
      node.netPv01 += priced.pv01;
      node.netDv01 += priced.dv01;
      // Zip each per-pillar DV01 onto its curve-pillar tenor and sum per bucket —
      // the same pillar alignment the server's `fact_from_position` performs.
      curve.pillars.forEach((pillar, i) => {
        // Month/broken-date pillars aren't whole-year-labelled, so they are not
        // bucketed here — matching the server federation's deferred key-rate ladder.
        const years = pillarYears(pillar.tenor);
        if (years === undefined) return;
        const prev = node.ladder.get(years) ?? 0;
        node.ladder.set(years, prev + (priced.keyRateLadder[i] ?? 0));
      });
    }

    const nodes: RatesRiskNode[] = [...byCcy.entries()]
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
      .map(([nodeCcy, n]) => ({
        ccy: nodeCcy,
        netPv: n.netPv,
        netPv01: n.netPv01,
        netDv01: n.netDv01,
        keyRateLadder: [...n.ladder.entries()]
          .sort(([a], [b]) => a - b)
          .map(([tenorYears, dv01]) => ({ tenorYears, dv01 })),
      }));

    const res: AggregateRatesRiskResponse = { nodes };
    if (request.correlationId !== undefined)
      res.correlationId = request.correlationId;
    return res;
  }

  async combinedTailRisk(
    request: CombinedTailRiskRequest,
  ): Promise<CombinedTailRiskResponse> {
    // A genuine in-browser JOINT tail cube — NOT a fabricated stub. The option
    // legs + FI legs are repriced through the SAME closed forms the live edge
    // mirrors (Garman-Kohlhagen via `pricing`, the self-discounting OIS PV via
    // `ratesPricing`), summed per aligned scenario, and reduced by the one
    // platform-wide VaR/ES tail primitive — reproducing
    // `celnet_risk_cube::fi::combined_tail_risk` (options-only ⇒ the options VaR,
    // FI-only ⇒ the rate VaR, mixed ⇒ the joint diversifying tail). A malformed
    // request (a rate shock whose length ≠ the base-curve pillar count) rejects
    // the whole call, exactly as the live edge does.
    return combinedTailRiskOffline(request);
  }

  async drillRisk(request: DrillRiskRequest): Promise<DrillRiskResponse> {
    // The offline book is a single node; a drill to a finer dimension yields the
    // same genuine aggregate as its one child (honest: there is no finer org
    // structure offline). Positions are omitted for the same reason listPositions
    // reports none (no offline canonical leaf).
    const child = aggregateSeedBook(request.childDimension, request.numeraire);
    const res: DrillRiskResponse = {
      node: request.node,
      children:
        request.includeChildren && child.positionCount > 0 ? [child] : [],
      positions: [],
    };
    if (request.correlationId !== undefined)
      res.correlationId = request.correlationId;
    return res;
  }

  async limitStatus(request: LimitStatusRequest): Promise<LimitStatusResponse> {
    // No limit tree is configured in the offline mock; report an honest empty set
    // (GREEN, no breach) rather than fabricate caps the desk never set.
    const res: LimitStatusResponse = {
      scope: request.scope,
      limits: [],
      worst: "GREEN",
      hardBreach: false,
    };
    if (request.correlationId !== undefined)
      res.correlationId = request.correlationId;
    return res;
  }

  // --- FixAdminService — offline in-memory managed-acceptor registry ----------

  /** The offline captured-traffic ring (the monitor feed); see `listFixMessages`. */
  private readonly fixMessages: FixMessage[] = [];
  /** Monotonic capture cursor for the synthetic monitor feed. */
  private fixSeq = 0n;

  async listFixConnections(): Promise<FixConnection[]> {
    return this.fixConnections.map((c) => ({ ...c }));
  }

  async createFixConnection(spec: FixConnectionSpec): Promise<FixConnection> {
    const conn = this.fixFromSpec(
      spec,
      spec.id?.trim() || mockSlugify(spec.name),
    );
    if (this.fixConnections.some((c) => c.id === conn.id)) {
      throw new Error(`a connection with id \`${conn.id}\` already exists`);
    }
    if (this.fixConnections.some((c) => c.name === conn.name)) {
      throw new Error(`a connection named \`${conn.name}\` already exists`);
    }
    this.assertNoEnabledAddrConflict(conn);
    this.fixConnections.push(conn);
    return { ...conn };
  }

  async updateFixConnection(
    id: string,
    spec: FixConnectionSpec,
  ): Promise<FixConnection> {
    const idx = this.fixConnections.findIndex((c) => c.id === id);
    if (idx < 0) throw new Error(`no connection with id \`${id}\``);
    if (this.fixConnections.some((c) => c.id !== id && c.name === spec.name)) {
      throw new Error(`a connection named \`${spec.name}\` already exists`);
    }
    const conn = this.fixFromSpec(spec, id);
    this.assertNoEnabledAddrConflict(conn);
    this.fixConnections.splice(idx, 1, conn);
    return { ...conn };
  }

  async deleteFixConnection(id: string): Promise<void> {
    const idx = this.fixConnections.findIndex((c) => c.id === id);
    if (idx < 0) throw new Error(`no connection with id \`${id}\``);
    this.fixConnections.splice(idx, 1);
  }

  async setFixConnectionEnabled(
    id: string,
    enabled: boolean,
  ): Promise<FixConnection> {
    const conn = this.fixConnections.find((c) => c.id === id);
    if (!conn) throw new Error(`no connection with id \`${id}\``);
    if (enabled) this.assertNoEnabledAddrConflict({ ...conn, enabled });
    conn.enabled = enabled;
    conn.running = enabled;
    conn.boundAddr = enabled ? conn.bindAddr : "";
    return { ...conn };
  }

  async listFixMessages(
    connectionId: string | undefined,
    afterSeq: bigint,
    limit = 0,
  ): Promise<FixMessagePage> {
    // Offline liveness: the mock has no real socket, so it synthesises a plausible
    // session transcript. The first poll seeds a short RFQ conversation for each
    // running acceptor; every poll then appends a heartbeat so the monitor tails.
    if (this.fixSeq === 0n) {
      for (const c of this.fixConnections) {
        if (c.running) this.seedFixTranscript(c.id);
      }
    }
    for (const c of this.fixConnections) {
      if (c.running && (connectionId === undefined || connectionId === c.id)) {
        this.pushFix(c.id, "INBOUND", "0", "Heartbeat");
      }
    }
    const cap = limit > 0 ? limit : 500;
    const messages = this.fixMessages
      .filter((m) => m.seq > afterSeq)
      .filter(
        (m) => connectionId === undefined || m.connectionId === connectionId,
      )
      .slice(0, cap)
      .map((m) => ({ ...m }));
    return { messages, latestSeq: this.fixSeq };
  }

  /** Seed a short, realistic inbound/outbound RFQ transcript for `connectionId`. */
  private seedFixTranscript(connectionId: string): void {
    this.pushFix(connectionId, "INBOUND", "A", "Logon");
    this.pushFix(connectionId, "OUTBOUND", "A", "Logon");
    this.pushFix(connectionId, "INBOUND", "R", "QuoteRequest");
    this.pushFix(connectionId, "OUTBOUND", "S", "Quote");
    this.pushFix(connectionId, "INBOUND", "D", "NewOrderSingle");
    this.pushFix(connectionId, "OUTBOUND", "8", "ExecutionReport");
  }

  /** Append one synthetic captured frame to the offline monitor ring (cap 500). */
  private pushFix(
    connectionId: string,
    direction: FixMessage["direction"],
    msgType: string,
    summary: string,
  ): void {
    this.fixSeq += 1n;
    const seq = this.fixSeq;
    const inbound = direction === "INBOUND";
    const raw =
      `8=FIX.4.4|9=0|35=${msgType}|49=${inbound ? "CELNET-CPTY" : "CELNET"}|` +
      `56=${inbound ? "CELNET" : "CELNET-CPTY"}|34=${seq.toString()}|10=000`;
    this.fixMessages.push({
      seq,
      connectionId,
      direction,
      msgType,
      summary,
      epochNanos: BigInt(Date.now()) * 1_000_000n,
      raw,
    });
    if (this.fixMessages.length > 500) this.fixMessages.shift();
  }

  /** Build a descriptor from a spec, reflecting `enabled` into the offline runtime status. */
  private fixFromSpec(spec: FixConnectionSpec, id: string): FixConnection {
    // Server parity: the routing desk is OPTIONAL — a blank desk is a valid,
    // intentionally-unrouted connection (no desk's users receive its RFQs/deals).
    // A NON-blank desk must name a defined desk, else the server rejects it.
    const desk = (spec.desk ?? "").trim();
    if (desk.length > 0 && !this.mockDesks.some((d) => d.id === desk)) {
      throw new Error(`routing desk \`${desk}\` is not a defined desk`);
    }
    return {
      id,
      name: spec.name,
      kind: spec.kind,
      bindAddr: spec.bindAddr,
      senderCompId: spec.senderCompId,
      targetCompId: spec.targetCompId,
      enabled: spec.enabled,
      running: spec.enabled,
      boundAddr: spec.enabled ? spec.bindAddr : "",
      desk,
    };
  }

  /** Reject an enabled acceptor sharing a bind address with another enabled one. */
  private assertNoEnabledAddrConflict(conn: FixConnection): void {
    if (!conn.enabled) return;
    const clash = this.fixConnections.find(
      (c) => c.id !== conn.id && c.enabled && c.bindAddr === conn.bindAddr,
    );
    if (clash) {
      throw new Error(
        `address \`${conn.bindAddr}\` is already used by enabled connection \`${clash.id}\``,
      );
    }
  }

  // --- AuthService (offline) -------------------------------------------------
  //
  // An in-memory mirror of the server's session + user/desk administration so the
  // `?mock` GUI signs in and administers offline. Validation parity: case-
  // insensitive unique emails, a 12-char minimum on create/reset, and a
  // no-last-admin-lockout guard (a desk delete unassigns its members), so the
  // offline workspace surfaces the same errors the live edge would.

  async login(email: string, password: string): Promise<LoginResult> {
    const key = email.trim().toLowerCase();
    const found = this.mockUsers.find(
      (u) => u.user.email.toLowerCase() === key,
    );
    // A single opaque error for every failure mode — never leak which factor failed.
    if (!found || found.user.disabled || found.password !== password) {
      throw new Error("invalid email or password");
    }
    this.mockTokenSeq += 1n;
    const token = `mock-session-${this.mockTokenSeq.toString()}`;
    this.mockTokens.add(token);
    // A 12-hour session, mirroring the server's TTL.
    const expiresNanos = nowNanos() + 12n * 60n * 60n * 1_000_000_000n;
    // The caller's OWN effective set — the SAME `role bundle ∪ grants ∖ denies`
    // resolution the live server runs (and that GetUserCapabilities returns), so
    // offline affordance gating is coherent real behaviour, not a stub.
    const capabilities = mockResolveEffective(
      found.user.role,
      this.mockRoleBase(found.user.role),
      found.grants,
      found.denies,
    );
    return { token, user: { ...found.user }, expiresNanos, capabilities };
  }

  async logout(): Promise<boolean> {
    const token = this.sessionToken;
    const ended = token !== null && this.mockTokens.delete(token);
    return ended;
  }

  async listUsers(): Promise<UserDesc[]> {
    return this.mockUsers.map((u) => ({ ...u.user }));
  }

  async createUser(input: CreateUserInput): Promise<UserDesc> {
    const email = input.email.trim();
    if (email.length === 0) throw new Error("email is required");
    if (input.password.length < MOCK_MIN_PASSWORD_LEN) {
      throw new Error(
        `password must be at least ${MOCK_MIN_PASSWORD_LEN} characters`,
      );
    }
    const key = email.toLowerCase();
    if (this.mockUsers.some((u) => u.user.email.toLowerCase() === key)) {
      throw new Error(`a user with email \`${email}\` already exists`);
    }
    const user: UserDesc = {
      id: mockSlugify(email),
      email,
      displayName: input.displayName.trim() || email,
      role: input.role,
      deskIds: input.allDesks ? [] : [...input.deskIds],
      allDesks: input.allDesks,
      disabled: false,
    };
    this.mockUsers.push({
      user,
      password: input.password,
      grants: [],
      denies: [],
    });
    return { ...user };
  }

  async updateUser(id: string, input: UpdateUserInput): Promise<UserDesc> {
    const entry = this.mockUsers.find((u) => u.user.id === id);
    if (!entry) throw new Error(`no user with id \`${id}\``);
    // No-last-admin-lockout: refuse to demote/disable the only remaining admin.
    const wasActiveAdmin = entry.user.role === "ADMIN" && !entry.user.disabled;
    const willBeActiveAdmin = input.role === "ADMIN" && !input.disabled;
    if (wasActiveAdmin && !willBeActiveAdmin && this.activeAdminCount() <= 1) {
      throw new Error("cannot remove the last administrator");
    }
    const next: UserDesc = {
      id: entry.user.id,
      email: entry.user.email,
      displayName: input.displayName.trim() || entry.user.email,
      role: input.role,
      deskIds: input.allDesks ? [] : [...input.deskIds],
      allDesks: input.allDesks,
      disabled: input.disabled,
    };
    entry.user = next;
    return { ...next };
  }

  async deleteUser(id: string): Promise<boolean> {
    const idx = this.mockUsers.findIndex((u) => u.user.id === id);
    if (idx < 0) return false;
    const entry = this.mockUsers[idx]!;
    if (
      entry.user.role === "ADMIN" &&
      !entry.user.disabled &&
      this.activeAdminCount() <= 1
    ) {
      throw new Error("cannot delete the last administrator");
    }
    this.mockUsers.splice(idx, 1);
    return true;
  }

  async resetPassword(id: string, newPassword: string): Promise<void> {
    const entry = this.mockUsers.find((u) => u.user.id === id);
    if (!entry) throw new Error(`no user with id \`${id}\``);
    if (newPassword.length < MOCK_MIN_PASSWORD_LEN) {
      throw new Error(
        `password must be at least ${MOCK_MIN_PASSWORD_LEN} characters`,
      );
    }
    entry.password = newPassword;
  }

  async getUserCapabilities(id: string): Promise<UserCapabilities> {
    const entry = this.mockUsers.find((u) => u.user.id === id);
    if (!entry) throw new Error(`no user with id \`${id}\``);
    return {
      grants: entry.grants.map((c) => ({ ...c })),
      denies: entry.denies.map((c) => ({ ...c })),
      effective: mockResolveEffective(
        entry.user.role,
        this.mockRoleBase(entry.user.role),
        entry.grants,
        entry.denies,
      ),
    };
  }

  async setUserCapabilities(
    id: string,
    grants: readonly Capability[],
    denies: readonly Capability[],
  ): Promise<UserCapabilities> {
    const entry = this.mockUsers.find((u) => u.user.id === id);
    if (!entry) throw new Error(`no user with id \`${id}\``);
    // Server parity: an unknown action/asset label is rejected (invalid_argument),
    // never silently dropped — the overlay that lands is exactly what was sent.
    for (const cap of [...grants, ...denies]) {
      if (
        !CAPABILITY_ACTIONS.includes(cap.action) ||
        !CAPABILITY_ASSETS.includes(cap.asset)
      ) {
        throw new Error(`unknown capability \`${cap.action}/${cap.asset}\``);
      }
    }
    // The overlay is replaced wholesale (not a delta). A successful set revokes the
    // target's live sessions server-side; offline the single in-browser session is
    // the admin's own, so there is nothing to revoke here — the contract semantics
    // are surfaced to the admin by the workspace's success note.
    entry.grants = grants.map((c) => ({ ...c }));
    entry.denies = denies.map((c) => ({ ...c }));
    return {
      grants: entry.grants.map((c) => ({ ...c })),
      denies: entry.denies.map((c) => ({ ...c })),
      effective: mockResolveEffective(
        entry.user.role,
        this.mockRoleBase(entry.user.role),
        entry.grants,
        entry.denies,
      ),
    };
  }

  /**
   * The resolved capability base a role confers (the admin-editable bundle).
   * `ADMIN` resolves to an empty base here — its grant-all is handled by the
   * resolver's role check, never a stored bundle; a non-admin role resolves to its
   * stored bundle, or the default trader bundle when none has been set.
   */
  private mockRoleBase(role: UserRole): Capability[] {
    if (role === "ADMIN") return [];
    return this.mockRoleBundles.get(role) ?? mockDefaultTraderBundle();
  }

  async getRoleCapabilities(role: UserRole): Promise<RoleCapabilities> {
    // ADMIN is grant-all and immutable; report the full surface, never a stored bundle.
    const capabilities =
      role === "ADMIN" ? mockGrantAll() : this.mockRoleBase(role);
    return { capabilities: capabilities.map((c) => ({ ...c })) };
  }

  async setRoleCapabilities(
    role: UserRole,
    capabilities: readonly Capability[],
  ): Promise<RoleCapabilities> {
    // The Admin role is grant-all and can never be narrowed (mirrors the server's
    // `failed_precondition`).
    if (role === "ADMIN") {
      throw new Error("the Admin role is grant-all and cannot be narrowed");
    }
    // Server parity: an unknown action/asset label is rejected, never silently dropped.
    for (const cap of capabilities) {
      if (
        !CAPABILITY_ACTIONS.includes(cap.action) ||
        !CAPABILITY_ASSETS.includes(cap.asset)
      ) {
        throw new Error(`unknown capability \`${cap.action}/${cap.asset}\``);
      }
    }
    // The bundle is replaced wholesale. A successful set revokes the live sessions of
    // every user holding the role server-side; offline the single in-browser session
    // is the admin's own, so there is nothing to revoke here — the contract semantics
    // are surfaced to the admin by the workspace's success note.
    this.mockRoleBundles.set(
      role,
      capabilities.map((c) => ({ ...c })),
    );
    return { capabilities: this.mockRoleBase(role).map((c) => ({ ...c })) };
  }

  async listDesks(): Promise<DeskDesc[]> {
    return this.mockDesks.map((d) => ({ ...d }));
  }

  async createDesk(name: string): Promise<DeskDesc> {
    const label = name.trim();
    if (label.length === 0) throw new Error("desk name is required");
    const id = mockSlugify(label);
    if (this.mockDesks.some((d) => d.id === id)) {
      throw new Error(`a desk with id \`${id}\` already exists`);
    }
    const desk: DeskDesc = { id, name: label };
    this.mockDesks.push(desk);
    return { ...desk };
  }

  async updateDesk(id: string, name: string): Promise<DeskDesc> {
    // Server parity: unknown id ⇒ not_found; blank name ⇒ invalid_argument; a name
    // that case-insensitively collides with ANOTHER desk ⇒ already_exists. The `id`
    // is the immutable routing key — only the display label changes.
    const desk = this.mockDesks.find((d) => d.id === id);
    if (!desk) throw new Error(`no desk with id \`${id}\``);
    const label = name.trim();
    if (label.length === 0) throw new Error("desk name is required");
    const clash = this.mockDesks.some(
      (d) => d.id !== id && d.name.toLowerCase() === label.toLowerCase(),
    );
    if (clash) throw new Error(`a desk named \`${label}\` already exists`);
    desk.name = label;
    return { ...desk };
  }

  async deleteDesk(id: string): Promise<boolean> {
    const idx = this.mockDesks.findIndex((d) => d.id === id);
    if (idx < 0) return false;
    this.mockDesks.splice(idx, 1);
    // Members of the deleted desk drop it from their membership (server parity).
    for (const entry of this.mockUsers) {
      if (entry.user.deskIds.includes(id)) {
        entry.user = {
          ...entry.user,
          deskIds: entry.user.deskIds.filter((d) => d !== id),
        };
      }
    }
    return true;
  }

  /** The number of enabled (non-disabled) administrators in the offline roster. */
  private activeAdminCount(): number {
    return this.mockUsers.filter(
      (u) => u.user.role === "ADMIN" && !u.user.disabled,
    ).length;
  }

  // --- legal-entity / netting-book registry (offline) ------------------------
  //
  // A GENUINE in-memory registry (not a stub): admin CRUD mutates the stores and
  // create auto-assigns the lowest free key (`key: 0` ⇒ auto), exactly like the
  // server. Listing is unauthenticated here (offline has no role gate); the live
  // server enforces admin-only mutation and any-user listing.

  /** The lowest free key ≥ 1 across the given used keys (server parity). */
  private static lowestFreeKey(used: readonly number[]): number {
    const set = new Set(used);
    let k = 1;
    while (set.has(k)) k += 1;
    return k;
  }

  async listEntities(): Promise<EntityDesc[]> {
    return this.mockEntities.map((e) => ({ ...e }));
  }

  async createEntity(input: EntityInput): Promise<EntityDesc> {
    const name = input.name.trim();
    const code = input.code.trim();
    if (name.length === 0) throw new Error("entity name is required");
    if (code.length === 0) throw new Error("entity code is required");
    if (this.mockEntities.some((e) => e.name === name)) {
      throw new Error(`an entity named \`${name}\` already exists`);
    }
    if (this.mockEntities.some((e) => e.code === code)) {
      throw new Error(`an entity with code \`${code}\` already exists`);
    }
    const key = MockTransport.lowestFreeKey(
      this.mockEntities.map((e) => e.key),
    );
    const entity: EntityDesc = { key, name, code };
    this.mockEntities.push(entity);
    return { ...entity };
  }

  async updateEntity(key: number, input: EntityInput): Promise<EntityDesc> {
    const existing = this.mockEntities.find((e) => e.key === key);
    if (!existing) throw new Error(`no entity with key ${key}`);
    const name = input.name.trim();
    const code = input.code.trim();
    if (name.length === 0) throw new Error("entity name is required");
    if (code.length === 0) throw new Error("entity code is required");
    if (this.mockEntities.some((e) => e.key !== key && e.name === name)) {
      throw new Error(`an entity named \`${name}\` already exists`);
    }
    if (this.mockEntities.some((e) => e.key !== key && e.code === code)) {
      throw new Error(`an entity with code \`${code}\` already exists`);
    }
    existing.name = name;
    existing.code = code;
    return { ...existing };
  }

  async deleteEntity(key: number): Promise<boolean> {
    // Referential integrity: refuse while any book still references the entity
    // (mirrors the server's FailedPrecondition).
    if (this.mockBooks.some((b) => b.entityKey === key)) {
      throw new Error("cannot delete an entity while books still reference it");
    }
    const idx = this.mockEntities.findIndex((e) => e.key === key);
    if (idx < 0) return false;
    this.mockEntities.splice(idx, 1);
    return true;
  }

  async listBooks(): Promise<BookDesc[]> {
    return this.mockBooks.map((b) => ({ ...b }));
  }

  async createBook(input: BookInput): Promise<BookDesc> {
    const name = input.name.trim();
    if (name.length === 0) throw new Error("book name is required");
    if (!this.mockEntities.some((e) => e.key === input.entityKey)) {
      throw new Error(`no entity with key ${input.entityKey}`);
    }
    if (this.mockBooks.some((b) => b.name === name)) {
      throw new Error(`a book named \`${name}\` already exists`);
    }
    const key = MockTransport.lowestFreeKey(this.mockBooks.map((b) => b.key));
    const book: BookDesc = { key, name, entityKey: input.entityKey };
    this.mockBooks.push(book);
    return { ...book };
  }

  async updateBook(key: number, input: BookInput): Promise<BookDesc> {
    const existing = this.mockBooks.find((b) => b.key === key);
    if (!existing) throw new Error(`no book with key ${key}`);
    const name = input.name.trim();
    if (name.length === 0) throw new Error("book name is required");
    if (!this.mockEntities.some((e) => e.key === input.entityKey)) {
      throw new Error(`no entity with key ${input.entityKey}`);
    }
    if (this.mockBooks.some((b) => b.key !== key && b.name === name)) {
      throw new Error(`a book named \`${name}\` already exists`);
    }
    existing.name = name;
    existing.entityKey = input.entityKey;
    return { ...existing };
  }

  async deleteBook(key: number): Promise<boolean> {
    const idx = this.mockBooks.findIndex((b) => b.key === key);
    if (idx < 0) return false;
    this.mockBooks.splice(idx, 1);
    return true;
  }

  // --- FI Aggregated Book (ADR-0022) — offline in-memory registry -------------
  //
  // A GENUINE in-memory registry (not a stub): admin CRUD mutates the store and
  // create mints an id from `name` (or honours a client-suggested slug), exactly
  // like the server. Listing is unauthenticated offline; the live server enforces
  // admin-only mutation and any-user listing.

  async listAggregatedBooks(): Promise<AggregatedBookDesc[]> {
    return this.mockAggregatedBooks.map((b) => cloneAggBook(b));
  }

  async createAggregatedBook(spec: AggregatedBookSpec): Promise<AggregatedBookDesc> {
    const name = spec.name.trim();
    if (name.length === 0) throw new Error("aggregated-book name is required");
    const id = spec.id.trim().length > 0 ? mockSlugify(spec.id) : mockSlugify(name);
    if (this.mockAggregatedBooks.some((b) => b.id === id)) {
      throw new Error(`an aggregated book with id \`${id}\` already exists`);
    }
    if (
      this.mockAggregatedBooks.some(
        (b) => b.name.toLowerCase() === name.toLowerCase(),
      )
    ) {
      throw new Error(`an aggregated book named \`${name}\` already exists`);
    }
    const book = aggBookFromSpec(id, name, spec);
    this.mockAggregatedBooks.push(book);
    return cloneAggBook(book);
  }

  async updateAggregatedBook(
    id: string,
    spec: AggregatedBookSpec,
  ): Promise<AggregatedBookDesc> {
    const existing = this.mockAggregatedBooks.find((b) => b.id === id);
    if (!existing) throw new Error(`no aggregated book with id \`${id}\``);
    const name = spec.name.trim();
    if (name.length === 0) throw new Error("aggregated-book name is required");
    if (
      this.mockAggregatedBooks.some(
        (b) => b.id !== id && b.name.toLowerCase() === name.toLowerCase(),
      )
    ) {
      throw new Error(`an aggregated book named \`${name}\` already exists`);
    }
    // The `id` is the immutable identity; the spec's own `id` field is ignored.
    const updated = aggBookFromSpec(id, name, spec);
    const idx = this.mockAggregatedBooks.indexOf(existing);
    this.mockAggregatedBooks.splice(idx, 1, updated);
    return cloneAggBook(updated);
  }

  async deleteAggregatedBook(id: string): Promise<boolean> {
    const idx = this.mockAggregatedBooks.findIndex((b) => b.id === id);
    if (idx < 0) return false;
    this.mockAggregatedBooks.splice(idx, 1);
    return true;
  }

  // --- instrument reference-data registry (offline) --------------------------

  /** Mint a stable instrument id from a name (mirrors the server's slugify). */
  private static slugifyInstrumentId(name: string): string {
    return name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "");
  }

  async listInstruments(): Promise<InstrumentDef[]> {
    return this.mockInstruments.map((d) => structuredClone(d));
  }

  async getInstrument(id: string): Promise<InstrumentDef | null> {
    const found = this.mockInstruments.find((d) => d.instrumentId === id);
    return found ? structuredClone(found) : null;
  }

  async createInstrument(input: InstrumentInput): Promise<InstrumentDef> {
    const name = input.name.trim();
    if (name.length === 0) throw new Error("instrument name is required");
    if (this.mockInstruments.some((d) => d.name === name)) {
      throw new Error(`an instrument named \`${name}\` already exists`);
    }
    const requested = input.instrumentId.trim();
    const id =
      requested.length > 0
        ? requested
        : MockTransport.slugifyInstrumentId(name);
    if (id.length === 0) {
      throw new Error("could not derive an instrument id from the name");
    }
    if (this.mockInstruments.some((d) => d.instrumentId === id)) {
      throw new Error(`an instrument with id \`${id}\` already exists`);
    }
    const created = structuredClone(input);
    created.instrumentId = id;
    created.name = name;
    this.mockInstruments.push(created);
    return structuredClone(created);
  }

  async updateInstrument(input: InstrumentInput): Promise<InstrumentDef> {
    const id = input.instrumentId.trim();
    if (id.length === 0) throw new Error("instrument id is required");
    const idx = this.mockInstruments.findIndex((d) => d.instrumentId === id);
    if (idx < 0) throw new Error(`no instrument with id \`${id}\``);
    const name = input.name.trim();
    if (name.length === 0) throw new Error("instrument name is required");
    if (
      this.mockInstruments.some((d) => d.instrumentId !== id && d.name === name)
    ) {
      throw new Error(`an instrument named \`${name}\` already exists`);
    }
    const updated = structuredClone(input);
    updated.instrumentId = id;
    updated.name = name;
    this.mockInstruments[idx] = updated;
    return structuredClone(updated);
  }

  async deleteInstrument(id: string): Promise<boolean> {
    const idx = this.mockInstruments.findIndex((d) => d.instrumentId === id);
    if (idx < 0) return false;
    this.mockInstruments.splice(idx, 1);
    return true;
  }

  // --- curve bootstrap from registry-referenced instruments (offline) --------

  /**
   * Resolve every pillar against the registry, derive each instrument's maturity
   * arm, order short→long, and bootstrap the SAME in-browser discount curve the
   * OIS pricer uses — the offline mirror of the server's `BuildCurve`. The numbers
   * are real (no fabricated curve): an unresolved id, a currency mismatch, or a
   * degenerate pillar set surfaces the genuine error.
   */
  async buildCurve(request: BuildCurveRequest): Promise<CalibratedCurve> {
    if (request.pillars.length === 0 && request.datePillars.length === 0) {
      throw new Error("a curve needs at least one calibrating pillar");
    }

    // Registry-instrument pillars: resolve, order short→long, and bootstrap the SAME
    // in-browser discount curve the OIS pricer uses (the offline mirror of the server).
    const instrumentPoints: CalibratedCurvePoint[] = [];
    if (request.pillars.length > 0) {
      const resolved = request.pillars.map((p) => {
        const def = this.mockInstruments.find(
          (d) => d.instrumentId === p.instrumentId,
        );
        if (!def) throw new Error(`no instrument with id \`${p.instrumentId}\``);
        if (def.currency !== request.currency) {
          throw new Error(
            `instrument \`${p.instrumentId}\` is ${def.currency}, not the curve currency ${request.currency}`,
          );
        }
        const tenor = MockTransport.instrumentPillarTenor(
          def,
          request.referenceDate,
        );
        return {
          instrumentId: p.instrumentId,
          tenor,
          parRate: p.quote,
          timeYears: pillarMaturityYears(tenor, request.referenceDate),
        };
      });
      const sorted = [...resolved].sort((a, b) => a.timeYears - b.timeYears);
      const curveSet: RatesCurveSet = {
        currency: request.currency,
        referenceDate: request.referenceDate,
        pillars: sorted.map((r) => ({ tenor: r.tenor, parRate: r.parRate })),
      };
      const discount = bootstrapCurveFromSet(curveSet);
      for (const r of sorted) {
        instrumentPoints.push({
          instrumentId: r.instrumentId,
          timeYears: r.timeYears,
          discountFactor: discountFactorAt(discount, r.timeYears),
          zeroRate: zeroRateAt(discount, r.timeYears),
          label: "",
        });
      }
    }

    // Date-anchored pillars: each is a synthetic money-market cash deposit from the
    // reference date to the chosen date, so its discount factor is the closed-form
    // `DF = 1/(1 + r·τ)` (τ = ACT/360 fraction) — the SAME value the server's deposit
    // pillar carries, independent of the other pillars. (The offline mirror omits the
    // second-order coupling of a longer swap pillar onto a shorter date pillar; the
    // live server bootstrap calibrates the mixed ladder jointly.)
    const datePoints = request.datePillars.map((dp) =>
      MockTransport.datePillarPoint(dp, request.referenceDate),
    );

    const points = [...instrumentPoints, ...datePoints].sort(
      (a, b) => a.timeYears - b.timeYears,
    );
    return {
      requestId: request.requestId,
      currency: request.currency,
      referenceDate: request.referenceDate,
      points,
    };
  }

  async getCurve(
    curveSet: RatesCurveSet | null,
    queryTenorYears: readonly number[],
    curveVersion?: bigint,
  ): Promise<GetCurveResult> {
    // Exactly one source (the wire rule): a pinned marked version reads the stored
    // curve; else the inline `curveSet` is bootstrapped live. An unknown version is
    // a `failed_precondition` refusal (mirrors the server), surfaced as a throw.
    let source: RatesCurveSet;
    if (curveVersion !== undefined) {
      const stored = this.markedCurves.get(curveVersion);
      if (!stored) {
        throw new Error(`no marked curve with version ${curveVersion}`);
      }
      source = stored;
    } else if (curveSet) {
      source = curveSet;
    } else {
      throw new Error(
        "get_curve needs an inline curve_set or a pinned curve_version",
      );
    }
    // A GENUINE in-browser read: bootstrap the self-discounting curve and sample the
    // requested tenor axis with the SAME log-linear-on-log-DF math the OIS pricer
    // uses, so the read curve agrees with the live `get_curve` RPC exactly.
    const discount = bootstrapCurveFromSet(source);
    return {
      currency: source.currency,
      referenceDate: source.referenceDate,
      points: MockTransport.curvePointsAt(discount, queryTenorYears),
      parPillars: MockTransport.curveParPillars(source),
      // Echo the pinned version only when reading one (omit the key otherwise —
      // exactOptionalPropertyTypes; a live bootstrap has no version).
      ...(curveVersion !== undefined ? { curveVersion } : {}),
      epochNanos: nowNanos(),
    };
  }

  async markCurve(curveSet: RatesCurveSet): Promise<MarkedCurve> {
    // Bootstrap once to validate + resolve, then deposit under a fresh version so a
    // later pinned `getCurve` reproduces it. A malformed set throws (server refusal).
    const discount = bootstrapCurveFromSet(curveSet);
    this.curveVersion += 1n;
    const version = this.curveVersion;
    this.markedCurves.set(version, curveSet);
    const parPillars = MockTransport.curveParPillars(curveSet);
    return {
      currency: curveSet.currency,
      curveVersion: version,
      parPillars,
      // The self-describing marked curve: bootstrapped points at the pillar tenors.
      points: MockTransport.curvePointsAt(
        discount,
        parPillars.map((p) => p.tenorYears),
      ),
      epochNanos: nowNanos(),
    };
  }

  async curveScenario(
    curveSet: RatesCurveSet,
    parallelShiftBp: number,
    keyRateShiftBp: readonly number[],
    queryTenorYears: readonly number[],
    instrument?: RatesInstrument,
  ): Promise<CurveScenarioResult> {
    // Build the shifted set: every pillar par += (parallel + key-rate_i) bp. An empty
    // key-rate vector ⇒ parallel-only; a non-empty one must match the pillar count
    // (the server enforces the same alignment). bp → decimal is ·1e-4.
    if (
      keyRateShiftBp.length > 0 &&
      keyRateShiftBp.length !== curveSet.pillars.length
    ) {
      throw new Error(
        `key_rate_shift_bp length ${keyRateShiftBp.length} must equal the pillar count ${curveSet.pillars.length}`,
      );
    }
    const shifted: RatesCurveSet = {
      ...curveSet,
      pillars: curveSet.pillars.map((p, i) => ({
        ...p,
        parRate: p.parRate + (parallelShiftBp + (keyRateShiftBp[i] ?? 0)) * 1e-4,
      })),
    };
    const shiftedDiscount = bootstrapCurveFromSet(shifted);
    const points = MockTransport.curvePointsAt(shiftedDiscount, queryTenorYears);

    let reprice: CurveScenarioReprice | null = null;
    if (instrument) {
      // Reprice the leg on the base and shifted curves through the SAME offline FI
      // pricer `price_rates` uses, so `basePv` equals the offline `priceRates` PV
      // exactly. `dv01` is the base-curve DV01 (the first-order predictor of ΔPV).
      const base = priceRatesInstrumentOffline(curveSet, instrument);
      const shiftedPriced = priceRatesInstrumentOffline(shifted, instrument);
      reprice = {
        basePv: base.pv,
        shiftedPv: shiftedPriced.pv,
        pvChange: shiftedPriced.pv - base.pv,
        dv01: base.dv01,
      };
    }
    return { currency: curveSet.currency, points, reprice };
  }

  /** Resolve a curve set's calibrating par pillars to `{ tenorYears, parRate }`. */
  private static curveParPillars(curveSet: RatesCurveSet): CurveParPillar[] {
    return curveSet.pillars.map((p) => ({
      tenorYears: pillarMaturityYears(p.tenor, curveSet.referenceDate),
      parRate: p.parRate,
    }));
  }

  /** Sample a bootstrapped curve at each query tenor (zero rate + discount factor). */
  private static curvePointsAt(
    discount: ReturnType<typeof bootstrapCurveFromSet>,
    tenors: readonly number[],
  ): CurvePoint[] {
    return tenors.map((t) => ({
      tenorYears: t,
      zeroRate: zeroRateAt(discount, t),
      discountFactor: discountFactorAt(discount, t),
    }));
  }

  /**
   * Resolve one date-anchored pillar to its calibrated point: a closed-form cash
   * deposit `DF = 1/(1 + r·τ)` with ACT/360 accrual from the reference date, placed on
   * the ACT/365F curve-time axis, labelled `Date YYYY-MM-DD`.
   */
  private static datePillarPoint(
    dp: DatePillar,
    referenceDate: BrokenDate,
  ): CalibratedCurvePoint {
    const t = pillarMaturityYears(
      { kind: "date", maturityDate: dp.maturityDate },
      referenceDate,
    );
    if (!(t > 0)) {
      throw new Error("a date pillar maturity must be after the reference date");
    }
    // ACT/360 fraction from the ACT/365F time: both count actual days, so the ratio is
    // exact (days = t·365 ⇒ τ_360 = t·365/360).
    const tauAct360 = (t * 365) / 360;
    const discountFactor = 1 / (1 + dp.quote * tauAct360);
    const zeroRate = -Math.log(discountFactor) / t;
    const { year, month, day } = dp.maturityDate;
    const label = `Date ${String(year).padStart(4, "0")}-${String(month).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
    return { instrumentId: "", timeYears: t, discountFactor, zeroRate, label };
  }

  /** Derive the curve-pillar maturity arm for one registry instrument. */
  private static instrumentPillarTenor(
    def: InstrumentDef,
    referenceDate: BrokenDate,
  ): PillarTenor {
    switch (def.family) {
      case "deposit":
        return MockTransport.tenorToPillar(def.deposit.tenor, referenceDate);
      case "fra":
        return MockTransport.tenorToPillar(def.fra.endTenor, referenceDate);
      case "stir_future":
        return MockTransport.tenorToPillar(
          def.stirFuture.referenceEnd,
          referenceDate,
        );
      case "vanilla_irs":
        return MockTransport.tenorToPillar(def.vanillaIrs.tenor, referenceDate);
      case "ois":
        return MockTransport.tenorToPillar(def.ois.tenor, referenceDate);
      case "bond":
        return { kind: "date", maturityDate: def.bond.maturityDate };
    }
  }

  /** Parse a `"<n><D|W|M|Y>"` tenor token to a {@link PillarTenor}. */
  private static tenorToPillar(
    tenor: string,
    referenceDate: BrokenDate,
  ): PillarTenor {
    const m = /^(\d+)\s*([DWMY])$/i.exec(tenor.trim());
    if (!m || m[1] === undefined || m[2] === undefined) {
      throw new Error(`unsupported pillar tenor \`${tenor}\``);
    }
    const count = Number(m[1]);
    switch (m[2].toUpperCase()) {
      case "Y":
        return { kind: "years", years: count };
      case "M":
        return { kind: "months", months: count };
      case "W":
        return {
          kind: "date",
          maturityDate: MockTransport.addDays(referenceDate, count * 7),
        };
      case "D":
        return {
          kind: "date",
          maturityDate: MockTransport.addDays(referenceDate, count),
        };
      default:
        throw new Error(`unsupported pillar tenor unit in \`${tenor}\``);
    }
  }

  /** Add `days` calendar days to a civil date (UTC arithmetic, no clamping). */
  private static addDays(ref: BrokenDate, days: number): BrokenDate {
    const d = new Date(Date.UTC(ref.year, ref.month - 1, ref.day));
    d.setUTCDate(d.getUTCDate() + days);
    return {
      year: d.getUTCFullYear(),
      month: d.getUTCMonth() + 1,
      day: d.getUTCDate(),
    };
  }

  // --- RfqDeskService (offline) ----------------------------------------------

  async submitDeskRequest(
    request: SubmitDeskRequestRequest,
  ): Promise<SubmitDeskRequestResponse> {
    return { request: this.enqueueDeskRequest(request) };
  }

  async respondDeskRequest(
    request: RespondDeskRequestRequest,
  ): Promise<RespondDeskRequestResponse> {
    const existing = this.deskRequests.get(request.requestId);
    if (!existing) throw new Error(`unknown desk request ${request.requestId}`);
    if (existing.state !== "PENDING") {
      throw new Error(
        `desk request ${request.requestId} is ${existing.state}, not PENDING`,
      );
    }
    let updated: DeskRequest;
    if (request.response.kind === "quote") {
      // Quote the request → QUOTED, storing the DeskQuote (no notification).
      updated = { ...existing, state: "QUOTED", quote: request.response.quote };
    } else {
      // Reject the request → REJECTED, pushing a QUOTE_REJECTED notification.
      updated = { ...existing, state: "REJECTED" };
      this.emitNotification({
        notificationId: `ntf-${this.notificationSeq++}`,
        kind: "QUOTE_REJECTED",
        atNanos: nowNanos(),
        requestId: existing.requestId,
        desk: existing.desk,
        counterparty: existing.counterparty,
        requestKind: existing.kind,
        headline: `${existing.counterparty} ${existing.kind} declined`,
        detail: request.response.reject.reason,
        alertWorthy: true,
      });
    }
    this.deskRequests.set(existing.requestId, updated);
    return { request: updated };
  }

  async acceptDeskQuote(
    request: AcceptDeskQuoteRequest,
  ): Promise<AcceptDeskQuoteResponse> {
    const existing = this.deskRequests.get(request.requestId);
    if (!existing) throw new Error(`unknown desk request ${request.requestId}`);
    if (existing.state !== "QUOTED" || !existing.quote) {
      throw new Error(
        `desk request ${request.requestId} is ${existing.state}; only a QUOTED request can be accepted`,
      );
    }
    const now = nowNanos();
    const quote = existing.quote;
    // Book a rates position from the dealt OIS struck at the quoted rate, so the
    // accepted deal shows in BOTH the deals blotter and the rates book.
    const positionId = this.ratesPositionSeq++;
    this.ratesPositions.set(positionId, {
      positionId,
      entity: 0,
      book: 0,
      instrument: {
        ...existing.instrument,
        fixedRate: quote.price,
        notional: quote.notional,
      },
    });
    const dealId = `deal-${this.dealSeq++}`;
    const deal: Deal = {
      dealId,
      requestId: existing.requestId,
      kind: existing.kind,
      counterparty: existing.counterparty,
      desk: existing.desk,
      instrument: existing.instrument,
      curveSet: existing.curveSet,
      side: existing.side,
      notional: quote.notional,
      price: quote.price,
      executedAtNanos: now,
      trader: quote.trader,
      positionId,
    };
    this.deals.set(dealId, deal);
    const updated: DeskRequest = { ...existing, state: "ACCEPTED" };
    this.deskRequests.set(existing.requestId, updated);
    this.emitNotification({
      notificationId: `ntf-${this.notificationSeq++}`,
      kind: "QUOTE_ACCEPTED",
      atNanos: now,
      requestId: existing.requestId,
      desk: existing.desk,
      counterparty: existing.counterparty,
      requestKind: existing.kind,
      headline: `${existing.counterparty} lifted ${existing.kind}: ${existing.instrument.tenorYears}y OIS @ ${(quote.price * 100).toFixed(3)}%`,
      detail: `deal ${dealId} · ${(quote.notional / 1_000_000).toFixed(0)}mm`,
      alertWorthy: true,
    });
    return { deal, request: updated };
  }

  async listDeskRequests(
    request: ListDeskRequestsRequest,
  ): Promise<ListDeskRequestsResponse> {
    // Map insertion order is mint order; reverse for newest-first.
    let requests = [...this.deskRequests.values()].reverse();
    const scope = request.scope;
    if (scope) {
      if (scope.states && scope.states.length > 0) {
        const states = new Set<DeskRequestState>(scope.states);
        requests = requests.filter((r) => states.has(r.state));
      }
      if (scope.desk !== undefined)
        requests = requests.filter((r) => r.desk === scope.desk);
    }
    return { requests };
  }

  async listDeals(request: ListDealsRequest): Promise<ListDealsResponse> {
    let deals = [...this.deals.values()].reverse();
    if (request.scope?.desk !== undefined) {
      deals = deals.filter((d) => d.desk === request.scope?.desk);
    }
    return { deals };
  }

  // --- RiskService rates Book (offline) --------------------------------------

  async bookRatesPosition(
    request: BookRatesPositionRequest,
  ): Promise<BookRatesPositionResponse> {
    const incoming = request.position;
    // Mint a stable id when the caller books with a placeholder (0) id.
    const positionId =
      incoming.positionId > 0n ? incoming.positionId : this.ratesPositionSeq++;
    const position: RatesPosition = { ...incoming, positionId };
    this.ratesPositions.set(positionId, position);
    return { position };
  }

  async listRatesPositions(
    request: ListRatesPositionsRequest,
  ): Promise<ListRatesPositionsResponse> {
    let positions = [...this.ratesPositions.values()];
    const scope = request.scope;
    if (scope) {
      if (scope.entity !== undefined)
        positions = positions.filter((p) => p.entity === scope.entity);
      if (scope.book !== undefined)
        positions = positions.filter((p) => p.book === scope.book);
      // An OIS books in its curve currency (USD for the P0 arm); a non-USD ccy
      // filter matches nothing, exactly as the server narrows by settlement ccy.
      if (scope.ccy !== undefined && scope.ccy.toUpperCase() !== "USD")
        positions = [];
    }
    return { positions };
  }

  // --- NotificationService (offline) -----------------------------------------

  streamNotifications(
    scope: NotificationScope | undefined,
    onNotification: (notification: Notification) => void,
  ): () => void {
    const sub = { scope, onNotification };
    this.notificationSubs.add(sub);
    // On the first UNSCOPED subscription (the live NotificationCenter path),
    // schedule a small, bounded set of exception-contract sample events so the
    // e2e can observe both a growl-worthy manual-intervention alert AND a quiet
    // auto-priced event. Scoped subscribers (desk-filtered) never trigger it.
    if (scope === undefined) this.scheduleSampleAlerts();
    return () => {
      this.notificationSubs.delete(sub);
      // Once no one is listening, drop any still-pending sample timers so a
      // unit-test teardown never leaks a live timer.
      if (this.notificationSubs.size === 0) this.clearSampleAlertTimers();
    };
  }

  /** Cancel and forget any pending sample-alert timers. */
  private clearSampleAlertTimers(): void {
    for (const t of this.sampleAlertTimers) clearTimeout(t);
    this.sampleAlertTimers = [];
  }

  /**
   * Fire a bounded, one-shot pair of exception-contract sample notifications
   * shortly after a client subscribes: (1) an AUTO-priced event with
   * `alertWorthy:false` that must land QUIETLY (no popup), then (2) a
   * `MANUAL_INTERVENTION_REQUIRED` event with `alertWorthy:true` + an
   * `UNCONFIGURED_TENOR` reason that must escalate (toast + growl). Timers are
   * `unref`'d where supported so they never keep a test/process event loop alive.
   */
  private scheduleSampleAlerts(): void {
    if (this.sampleAlertsScheduled) return;
    this.sampleAlertsScheduled = true;
    const schedule = (delayMs: number, build: () => Notification): void => {
      const t = setTimeout(() => this.emitNotification(build()), delayMs);
      // Node's timer object exposes `unref`; the browser's numeric id does not.
      (t as unknown as { unref?: () => void }).unref?.();
      this.sampleAlertTimers.push(t);
    };
    // (1) Quiet auto-priced confirmation — alertWorthy:false ⇒ centre-only.
    schedule(SAMPLE_QUIET_DELAY_MS, () => ({
      notificationId: `ntf-${this.notificationSeq++}`,
      kind: "QUOTE_ACCEPTED",
      atNanos: nowNanos(),
      desk: SAMPLE_DESK,
      counterparty: "Aster Global",
      requestKind: "RFQ",
      headline: "Auto-priced RFQ from Aster Global: 2y OIS 25mm",
      detail: "auto-quoted · no action needed",
      alertWorthy: false,
    }));
    // (2) Growl-worthy manual-intervention exception — alertWorthy:true.
    schedule(SAMPLE_ALERT_DELAY_MS, () => ({
      notificationId: `ntf-${this.notificationSeq++}`,
      kind: "MANUAL_INTERVENTION_REQUIRED",
      atNanos: nowNanos(),
      desk: SAMPLE_DESK,
      counterparty: "Meridian Capital",
      requestKind: "RFQ",
      headline: "Manual pricing needed",
      detail: "USD-OIS 15Y",
      reason: "UNCONFIGURED_TENOR",
      alertWorthy: true,
    }));
  }

  /** Enqueue a fresh PENDING desk request and push its `*_RECEIVED` notification. */
  private enqueueDeskRequest(request: SubmitDeskRequestRequest): DeskRequest {
    const now = nowNanos();
    const ttlMs = request.ttlMs > 0 ? request.ttlMs : 60_000;
    const requestId = `req-${this.deskRequestSeq++}`;
    const desk: DeskRequest = {
      requestId,
      kind: request.kind,
      counterparty: request.counterparty,
      desk: request.desk,
      instrument: request.instrument,
      curveSet: request.curveSet,
      side: request.side,
      notional: request.notional,
      receivedAtNanos: now,
      expiresAtNanos: now + BigInt(ttlMs) * NS_PER_MS,
      state: "PENDING",
    };
    this.deskRequests.set(requestId, desk);
    const kind: NotificationKind =
      request.kind === "IOI" ? "IOI_RECEIVED" : "RFQ_RECEIVED";
    this.emitNotification({
      notificationId: `ntf-${this.notificationSeq++}`,
      kind,
      atNanos: now,
      requestId,
      desk: request.desk,
      counterparty: request.counterparty,
      requestKind: request.kind,
      headline: `${request.kind} from ${request.counterparty}: ${request.instrument.tenorYears}y OIS ${(request.notional / 1_000_000).toFixed(0)}mm`,
      detail: `${request.desk} · ${request.side === "BUY" ? "pay" : "receive"} fixed`,
      alertWorthy: true,
    });
    return desk;
  }

  /** Fan a notification out to every subscriber whose desk scope admits it. */
  private emitNotification(n: Notification): void {
    for (const sub of this.notificationSubs) {
      if (
        sub.scope &&
        sub.scope.desks.length > 0 &&
        !sub.scope.desks.includes(n.desk)
      )
        continue;
      sub.onNotification(n);
    }
  }

  /**
   * Seed a small offline book so the rates Book blotter and the desk inbox show
   * genuine content immediately (real positions/requests derived from the
   * calibrating curve pillars — never baked-in results). The desk requests start
   * PENDING; the trader quotes/rejects them in the Quoting workspace.
   */
  private seedOfflineDesk(): void {
    const curve = DEFAULT_USD_SOFR_CURVE;
    const parOf = (tenorYears: number): number =>
      curve.pillars.find((p) => pillarYears(p.tenor) === tenorYears)?.parRate ??
      0.04;

    const bookSeeds: {
      tenorYears: number;
      entity: number;
      book: number;
      notionalMm: number;
      direction: OisInstrument["direction"];
      offsetBp: number;
    }[] = [
      {
        tenorYears: 2,
        entity: 1,
        book: 10,
        notionalMm: 50,
        direction: "RECEIVE_FIXED",
        offsetBp: -6,
      },
      {
        tenorYears: 5,
        entity: 1,
        book: 10,
        notionalMm: 100,
        direction: "PAY_FIXED",
        offsetBp: 4,
      },
      {
        tenorYears: 10,
        entity: 2,
        book: 20,
        notionalMm: 25,
        direction: "RECEIVE_FIXED",
        offsetBp: 9,
      },
    ];
    for (const s of bookSeeds) {
      const positionId = this.ratesPositionSeq++;
      this.ratesPositions.set(positionId, {
        positionId,
        entity: s.entity,
        book: s.book,
        instrument: {
          tenorYears: s.tenorYears,
          fixedRate: parOf(s.tenorYears) + s.offsetBp / 10_000,
          notional: s.notionalMm * 1_000_000,
          direction: s.direction,
        },
      });
    }

    const inboxSeeds: {
      kind: DeskRequestKind;
      counterparty: string;
      tenorYears: number;
      notionalMm: number;
      side: "BUY" | "SELL";
    }[] = [
      {
        kind: "RFQ",
        counterparty: "Meridian Capital",
        tenorYears: 5,
        notionalMm: 75,
        side: "BUY",
      },
      {
        kind: "IOI",
        counterparty: "Northwind AM",
        tenorYears: 10,
        notionalMm: 40,
        side: "SELL",
      },
    ];
    for (const s of inboxSeeds) {
      this.enqueueDeskRequest({
        kind: s.kind,
        counterparty: s.counterparty,
        desk: "g10-rates",
        instrument: {
          tenorYears: s.tenorYears,
          fixedRate: parOf(s.tenorYears),
          notional: s.notionalMm * 1_000_000,
          direction: s.side === "BUY" ? "PAY_FIXED" : "RECEIVE_FIXED",
        },
        curveSet: curve,
        side: s.side,
        notional: s.notionalMm * 1_000_000,
        ttlMs: 0,
      });
    }
  }
}

/** The minimum password length on create/reset (mirrors the server's `MIN_PASSWORD_LEN`). */
const MOCK_MIN_PASSWORD_LEN = 12;

/** The action set a `TRADER` holds by default on each asset class — every action
 * except `administer` (mirrors the server's `TRADER_ACTIONS`). `ADMIN` is grant-all. */
const MOCK_TRADER_DENIED_ACTION: CapabilityAction = "administer";

/** The full action-by-asset surface (the ADMIN grant-all bundle). */
function mockGrantAll(): Capability[] {
  const caps: Capability[] = [];
  for (const action of CAPABILITY_ACTIONS) {
    for (const asset of CAPABILITY_ASSETS) caps.push({ action, asset });
  }
  return caps;
}

/**
 * The default non-admin role bundle: every action except `administer` on both
 * asset classes (mirrors the server's `default_trader_bundle`). The base a role
 * confers until an admin narrows or widens it.
 */
function mockDefaultTraderBundle(): Capability[] {
  const caps: Capability[] = [];
  for (const action of CAPABILITY_ACTIONS) {
    if (action === MOCK_TRADER_DENIED_ACTION) continue;
    for (const asset of CAPABILITY_ASSETS) caps.push({ action, asset });
  }
  return caps;
}

/** A stable key for set membership over a capability. */
function mockCapKey(action: CapabilityAction, asset: CapabilityAsset): string {
  return `${action} ${asset}`;
}

/**
 * Resolve the effective capability set the offline edge admits, enumerated over
 * every action × asset — a GENUINE mirror of the server's
 * `AuthenticatedUser::capabilities`: `role bundle ∪ grants ∖ denies`, deny-wins.
 * `ADMIN` ⇒ grant-all; `TRADER` ⇒ every action except `administer` on both asset
 * classes; then per-user grants widen and denies narrow, with deny checked first.
 */
function mockResolveEffective(
  role: UserRole,
  roleBase: readonly Capability[],
  grants: readonly Capability[],
  denies: readonly Capability[],
): Capability[] {
  const baseSet = new Set(roleBase.map((c) => mockCapKey(c.action, c.asset)));
  const grantSet = new Set(grants.map((c) => mockCapKey(c.action, c.asset)));
  const denySet = new Set(denies.map((c) => mockCapKey(c.action, c.asset)));
  const effective: Capability[] = [];
  for (const action of CAPABILITY_ACTIONS) {
    for (const asset of CAPABILITY_ASSETS) {
      const key = mockCapKey(action, asset);
      if (denySet.has(key)) continue; // deny-wins
      const roleAllows = role === "ADMIN" || baseSet.has(key);
      if (roleAllows || grantSet.has(key)) effective.push({ action, asset });
    }
  }
  return effective;
}

/** A lowercase, hyphen-separated slug of `name` (mirrors the server's `slugify`). */
function mockSlugify(name: string): string {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug.length > 0 ? slug : "connection";
}

/** Deep-clone a persisted aggregated book so a caller can never mutate the store. */
function cloneAggBook(b: AggregatedBookDesc): AggregatedBookDesc {
  return {
    ...b,
    memberConnectionIds: [...b.memberConnectionIds],
    instrumentIds: [...b.instrumentIds],
    params: { ...b.params },
  };
}

/** Materialize a persisted aggregated book from a create/update spec + resolved id. */
function aggBookFromSpec(
  id: string,
  name: string,
  spec: AggregatedBookSpec,
): AggregatedBookDesc {
  return {
    id,
    name,
    memberConnectionIds: [...spec.memberConnectionIds],
    scopeMode: spec.scopeMode,
    instrumentIds: [...spec.instrumentIds],
    params: { ...spec.params },
    enabled: spec.enabled,
  };
}

/**
 * Genuinely aggregate the offline seed book into one `RiskNode`, collapsing every
 * position's signed-notional-scaled Greeks into the reporting numeraire. The mock
 * holds one book (no org attribution), so the result is independent of the
 * requested `dimension` — the single node IS the firm/desk/book/… view offline.
 */
function aggregateSeedBook(
  dimension: AggregateRiskRequest["dimension"],
  numeraire: ReportingNumeraire,
): RiskNode {
  // Resolve the spot rate of a currency INTO the reporting numeraire. The
  // numeraire's own rate is implicitly 1.0; any other ccy must be supplied (a
  // missing rate fails loudly, mirroring the server's fail-on-missing-rate).
  const rateOf = (ccy: string): number => {
    if (ccy === numeraire.numeraire) return 1;
    const r = numeraire.rates.find((x) => x.ccy === ccy);
    if (!r || !(r.rate > 0) || !Number.isFinite(r.rate)) {
      throw new Error(
        `reporting numeraire ${numeraire.numeraire} has no rate for ${ccy}`,
      );
    }
    return r.rate;
  };

  const deltaByCcy = new Map<string, number>();
  let deltaNumeraire = 0;
  let gamma = 0;
  let vegaNumeraire = 0;
  let theta = 0;
  let vanna = 0;
  let volga = 0;
  let charm = 0;
  let speed = 0;
  let zomma = 0;
  let color = 0;
  let premiumNumeraire = 0;
  let positionCount = 0;

  for (const { instrument } of seedSubscriptions()) {
    const ctx = findPair(instrument.pair);
    const greeks = priceInstrument(instrument, ctx.market).greeks;
    // Sign by direction; offline seed positions are the long the desk carries.
    const sign = instrument.side === "SELL" ? -1 : 1;
    const notional = instrument.quantity.notional * sign;
    // Premium/price is quoted as a fraction; the base-ccy delta exposure is
    // notional×delta, converted into the reporting numeraire by the base ccy's
    // rate (the quote/premium ccy is the pair's quote leg).
    const baseRate = rateOf(instrument.pair.base);
    const quoteRate = rateOf(instrument.pair.quote);
    const baseDelta = notional * greeks.deltaSpot;
    deltaByCcy.set(
      instrument.pair.base,
      (deltaByCcy.get(instrument.pair.base) ?? 0) + baseDelta,
    );
    deltaNumeraire += baseDelta * baseRate;
    // Vega / premium are premium-ccy (quote) amounts per unit notional.
    const wn = notional * quoteRate;
    gamma += notional * greeks.gamma * baseRate;
    vegaNumeraire += greeks.vega * wn;
    theta += greeks.theta * wn;
    vanna += greeks.vanna * wn;
    volga += greeks.volga * wn;
    charm += greeks.charm * wn;
    speed += notional * greeks.speed * baseRate;
    zomma += notional * greeks.zomma * baseRate;
    color += greeks.color * wn;
    premiumNumeraire += greeks.price * wn;
    positionCount += 1;
  }

  const deltaVector: CcyExposureLeg[] = [...deltaByCcy.entries()]
    .map(([ccy, amount]) => ({ ccy, amount }))
    .sort((a, b) => Math.abs(b.amount) - Math.abs(a.amount));

  const additive: AdditiveRisk = {
    deltaNumeraire,
    deltaVector,
    gamma,
    vegaNumeraire,
    theta,
    vanna,
    volga,
    charm,
    speed,
    zomma,
    color,
    premiumNumeraire,
    // The offline aggregate does not bucket the vega ladder server-side (the
    // pillar grid is request data the offline core does not slice the book on);
    // an empty ladder is honest, not a row of zeros.
    vegaLadder: [],
  };

  // Non-additive measures are not evaluated offline (no historical shock engine);
  // every field absent ⇒ the consumer shows "not evaluated", never a spurious 0.
  const nonadditive: NonAdditiveRisk = {};

  return { dimension, group: 0n, additive, nonadditive, positionCount };
}

/**
 * Resolve every delta-specified strike on an instrument to an absolute level at
 * the given market, returning a structurally-identical instrument with fixed
 * strikes. This is what a booked position is: fixed strikes, not "always the
 * 25-delta". Used by the scenario grid so shocks move a real position.
 */
function freezeStrikes(instrument: Instrument, m: MarketContext): Instrument {
  const t = instrument.expiryYears;
  const freezeStrike = (
    spec: { kind: "strike"; strike: number } | { kind: "delta"; delta: number },
  ): { kind: "strike"; strike: number } =>
    spec.kind === "strike"
      ? spec
      : { kind: "strike", strike: strikeFromDelta(spec.delta, m, t) };

  switch (instrument.product.kind) {
    case "vanilla": {
      const v = instrument.product.vanilla;
      return {
        ...instrument,
        product: {
          kind: "vanilla",
          vanilla: { ...v, strike: freezeStrike(v.strike) },
        },
      };
    }
    case "strategy": {
      const s = instrument.product.strategy;
      return {
        ...instrument,
        product: {
          kind: "strategy",
          strategy: {
            ...s,
            legs: s.legs.map((leg) => ({
              ...leg,
              strike: freezeStrike(leg.strike),
            })),
          },
        },
      };
    }
    case "singleBarrier": {
      const b = instrument.product.singleBarrier;
      return {
        ...instrument,
        product: {
          kind: "singleBarrier",
          singleBarrier: {
            ...b,
            vanilla: { ...b.vanilla, strike: freezeStrike(b.vanilla.strike) },
          },
        },
      };
    }
    case "doubleBarrier": {
      const b = instrument.product.doubleBarrier;
      return {
        ...instrument,
        product: {
          kind: "doubleBarrier",
          doubleBarrier: {
            ...b,
            vanilla: { ...b.vanilla, strike: freezeStrike(b.vanilla.strike) },
          },
        },
      };
    }
    case "windowBarrier": {
      // Like the single/double barrier, the window barrier wraps a delta-able
      // vanilla strike — freeze it so a scenario shock moves a real position.
      const b = instrument.product.windowBarrier;
      return {
        ...instrument,
        product: {
          kind: "windowBarrier",
          windowBarrier: {
            ...b,
            vanilla: { ...b.vanilla, strike: freezeStrike(b.vanilla.strike) },
          },
        },
      };
    }
    case "digital":
    case "touch":
    case "varianceSwap":
    case "volatilitySwap":
    case "asianOption":
    case "forwardStart":
    case "cliquet":
    case "quanto":
    case "tarf":
    case "pivot":
    case "accumulator":
    case "lookback":
    case "american":
    case "basket":
    case "fxForward":
    case "fxSwap":
    case "ndf":
    case "perpetualOption":
    case "listedFutureOption":
      // These products carry no delta-specified strike to freeze (the swaps and the
      // touch have none; the digital/Asian/quanto/TARF/lookback/American/basket
      // strikes are absolute; the forward-start/cliquet/accumulator strikes
      // reset/pivot off the spot path; the basket strike is on the aggregated
      // underlying; the W2 linear products carry an absolute contract rate; the
      // perpetual and listed-future-option strikes are absolute levels). Scenario
      // shocks move them through the market alone, so return unchanged. (The
      // single-/double-barrier ARE frozen above — they wrap a delta-able vanilla strike.)
      return instrument;
  }
}

function applyShock(
  m: MarketContext,
  factor: ShockFactor,
  step: number,
  relative: boolean,
): MarketContext {
  const bump = (base: number) => (relative ? base * (1 + step) : base + step);
  switch (factor) {
    case "SPOT":
      return { ...m, spot: bump(m.spot) };
    case "VOL":
      return { ...m, vol: Math.max(0.001, bump(m.vol)) };
    case "RATE_DOM":
      return { ...m, rDom: bump(m.rDom) };
    case "RATE_FOR":
      return { ...m, rFor: bump(m.rFor) };
    case "TIME":
      return m; // handled by expiry roll in the caller
  }
}

/** A finite-difference cross-gamma d²V/(dx_a dx_b). */
function crossGamma(
  instrument: Instrument,
  m: MarketContext,
  a: ShockFactor,
  b: ShockFactor,
): number {
  const ha = a === "SPOT" ? m.spot * 0.01 : 0.005;
  const hb = b === "SPOT" ? m.spot * 0.01 : 0.005;
  const pp = priceInstrument(
    instrument,
    applyShock(applyShock(m, a, ha, false), b, hb, false),
  ).greeks.price;
  const pm = priceInstrument(
    instrument,
    applyShock(applyShock(m, a, ha, false), b, -hb, false),
  ).greeks.price;
  const mp = priceInstrument(
    instrument,
    applyShock(applyShock(m, a, -ha, false), b, hb, false),
  ).greeks.price;
  const mm = priceInstrument(
    instrument,
    applyShock(applyShock(m, a, -ha, false), b, -hb, false),
  ).greeks.price;
  return (pp - pm - mp + mm) / (4 * ha * hb);
}

/** Construct the default standalone transport. */
export function createMockTransport(): CelnetTransport {
  void forward; // re-exported by pricing; referenced to keep tree-shaking honest
  void DEFAULT_CONVENTIONS;
  return new MockTransport();
}
