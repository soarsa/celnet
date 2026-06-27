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
  AdditiveRisk,
  AggregateRiskRequest,
  AggregateRiskResponse,
  BrokerQuoteSet,
  CcyExposureLeg,
  CcyPair,
  Conventions,
  CreateUserInput,
  DealerQuote,
  DeskDesc,
  DrillRiskRequest,
  DrillRiskResponse,
  Executed,
  Execution,
  FixConnection,
  FixConnectionSpec,
  FixMessage,
  FixMessagePage,
  Greeks,
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
  OisInstrument,
  Quote,
  RatesCurveSet,
  RatesPricingResult,
  ReportingNumeraire,
  RiskBucketRequest,
  RiskNode,
  ScenarioPoint,
  ScenarioResult,
  ShockAxis,
  ShockFactor,
  Smile,
  SmileModel,
  Snapshot,
  TradableToken,
  TwoWayPrice,
  Update,
  UpdateUserInput,
  UserDesc,
  VegaBucket,
} from "./contract";
import { forward, priceInstrument, strikeFromDelta } from "./pricing";
import { priceRatesOffline } from "./ratesPricing";
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

/** A wall-clock source in nanoseconds since epoch, monotone within a session. */
function nowNanos(): bigint {
  return BigInt(Math.round(performance.timeOrigin + performance.now())) * NS_PER_MS;
}

function findPair(pair: CcyPair): PairContext {
  const found = PAIRS.find((p) => p.pair.base === pair.base && p.pair.quote === pair.quote);
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
function syntheticLpTwoWay(k: number, mid: number, halfSpread: number): TwoWayPrice {
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
  const idx = Math.min(sorted.length - 1, Math.max(0, Math.ceil(q * sorted.length) - 1));
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
 * The mock multiplexed stream session. One instance multiplexes many
 * subscriptions over a single ticking loop, exactly as the contract's single
 * bidirectional `StreamSession` multiplexes by `SubscriptionId`.
 */
class MockStreamSession implements StreamSession {
  private readonly subs = new Map<bigint, LiveSubscription>();
  /** Live market-series subscriptions (same id space as price streams). */
  private readonly series = new Map<bigint, LiveSeries>();
  private readonly listeners = new Set<(e: StreamEvent) => void>();
  private nextSubId = 1n;
  private nextToken = 1n;
  private frame = 0;
  private timer: ReturnType<typeof setInterval> | undefined;
  private readonly consumedTokens = new Set<bigint>();
  private readonly tickMs: number;
  private readonly seed: bigint;

  constructor(seed: bigint, tickMs: number) {
    this.seed = seed;
    this.tickMs = tickMs;
  }

  private emit(event: StreamEvent): void {
    for (const l of this.listeners) l(event);
  }

  onEvent(listener: (event: StreamEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  subscribe(instrument: Instrument, conventions: Conventions, label: string): bigint {
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
    if (this.subs.size === 0 && this.timer !== undefined) {
      clearInterval(this.timer);
      this.timer = undefined;
    }
  }

  execute(subscriptionId: bigint, token: bigint, _idempotencyKey: string): void {
    const sub = this.subs.get(subscriptionId);
    const now = nowNanos();
    if (!sub) {
      this.emit({
        kind: "reject",
        reject: { subscriptionId, token, reason: "UNKNOWN_TOKEN", epochNanos: now },
      });
      return;
    }
    if (this.consumedTokens.has(token)) {
      this.emit({
        kind: "reject",
        reject: { subscriptionId, token, reason: "ALREADY_CONSUMED", epochNanos: now },
      });
      return;
    }
    const matched = sub.tokens.find((t) => t.token === token);
    if (!matched) {
      this.emit({
        kind: "reject",
        reject: { subscriptionId, token, reason: "UNKNOWN_TOKEN", epochNanos: now },
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
    const HISTORY = Math.min(48, params.historyLimit && params.historyLimit > 0 ? params.historyLimit : 24);
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
    if (this.subs.size === 0 && this.series.size === 0 && this.timer !== undefined) {
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
      Math.abs(q.tenorYears - tenorYears) < Math.abs(best.tenorYears - tenorYears) ? q : best,
    );
    const wing10 = nearest.hasTenDelta && Math.abs(params.delta ?? 0.25) <= 0.18;
    switch (params.observable) {
      case "ATM_VOL":
        return { anchor: nearest.atmVol, stepScale: 0.0006 };
      case "RISK_REVERSAL":
        return { anchor: wing10 ? nearest.rr10 : nearest.rr25, stepScale: 0.0004 };
      case "BUTTERFLY":
        return { anchor: wing10 ? nearest.bf10 : nearest.bf25, stepScale: 0.0003 };
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
    this.listeners.clear();
  }

  private ensureRunning(): void {
    if (this.timer !== undefined) return;
    this.timer = setInterval(() => this.tick(), this.tickMs);
  }

  private mintTokens(sub: LiveSubscription, price: TwoWayPrice): TradableToken[] {
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

  private priceSub(sub: LiveSubscription): { price: TwoWayPrice; greeks: Greeks; strike: number } {
    const market: MarketContext = { ...sub.ctx.market, vol: sub.vol };
    const { greeks, resolvedStrike } = priceInstrument(sub.instrument, market);
    // Premium as percent-of-foreign: GK price is per unit base in domestic; for a
    // %-foreign display we normalize by spot. Strategies sum signed leg premia.
    const midPct = Math.abs(greeks.price / market.spot) * 100;
    // Spread scales with vega magnitude and tenor (wider for longer-dated/illiquid).
    const spread = Math.max(0.004, Math.abs(greeks.vega) * 0.06 + sub.instrument.expiryYears * 0.02);
    return { price: twoWayAround(midPct, spread), greeks, strike: resolvedStrike };
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
    // Advance every live market series: a deterministic mean-reverting walk around
    // the observable's anchor, appended at the series' cadence (throttle hint).
    for (const s of this.series.values()) {
      s.countdown -= 1;
      if (s.countdown > 0) continue;
      s.countdown = s.cadence;
      s.value += 0.05 * (s.anchor - s.value) + s.stepScale * s.rng.normal();
      if (s.params.observable === "ATM_VOL" || s.params.observable === "BUTTERFLY") {
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
        this.emit({ kind: "health", subscriptionId: sub.id, health: sub.health });
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
      const elapsedNs = Math.max(0, Math.round((performance.now() - t0) * 1_000_000));
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
  private readonly mockUsers: { user: UserDesc; password: string }[] = [
    {
      user: {
        id: "admin",
        email: "admin@celnet.com",
        displayName: "Administrator",
        role: "ADMIN",
        disabled: false,
      },
      password: "password",
    },
  ];
  private readonly mockDesks: DeskDesc[] = [];
  /** Issued session tokens (offline liveness for `logout`'s `ended` result). */
  private readonly mockTokens = new Set<string>();
  private mockTokenSeq = 0n;

  constructor(opts: { seed?: bigint; tickMs?: number } = {}) {
    this.seed = opts.seed ?? 0xce1_5eed_d00dn;
    this.tickMs = opts.tickMs ?? 100; // 10 Hz tape; render conflates to a frame
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
    const { greeks, resolvedStrike, priceStdError } = priceInstrument(instrument, market);
    const midPct = Math.abs(greeks.price / market.spot) * 100;
    const spread = Math.max(0.004, Math.abs(greeks.vega) * 0.06 + instrument.expiryYears * 0.02);
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
    instrument: OisInstrument,
  ): Promise<RatesPricingResult> {
    // A GENUINE in-browser OIS computation: bootstrap the self-discounting curve
    // from the par-OIS pillars and price the swap (PV / par / PV01 / DV01 /
    // key-rate ladder), reproducing the server's `celnet-rates` math exactly so
    // the offline number agrees with the live `price_rates` RPC. A malformed
    // curve/instrument throws (mirroring the server refusal), surfaced by the
    // workspace exactly as a live transport error would be.
    return priceRatesOffline(curve, instrument);
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
    if (result.priceStdError !== undefined) quote.priceStdError = result.priceStdError;
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
    const quote = await this.requestQuote(instrument, conventions, idempotencyKey);
    const mid = (quote.price.bid + quote.price.offer) / 2;
    const halfSpread = (quote.price.offer - quote.price.bid) / 2;
    const native: DealerQuote = {
      lpId: MAKER_LP_ID,
      price: quote.price,
      greeks: quote.greeks,
      resolvedStrike: quote.resolvedStrike,
      validUntilNanos: quote.validUntilNanos,
    };
    if (quote.priceStdError !== undefined) native.priceStdError = quote.priceStdError;
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
    if (quote.surfaceVersion !== undefined) panel.surfaceVersion = quote.surfaceVersion;
    return panel;
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
      if (!row) throw new Error(`unknown dealer line ${lpId} on quote ${quoteId}`);
      line = row;
    }
    if (line.validUntilNanos <= now) throw new Error("quote expired (last-look)");
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
    return new MockStreamSession(this.seed, this.tickMs);
  }

  async getSmile(pair: CcyPair, tenorYears: number, conventions: Conventions): Promise<Smile> {
    const ctx = findPair(pair);
    const ladder = brokerLadder(ctx);
    const nearest = ladder.reduce((best, q) =>
      Math.abs(q.tenorYears - tenorYears) < Math.abs(best.tenorYears - tenorYears) ? q : best,
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
        if (axis.factor === "TIME") expiryYears = Math.max(1 / 365, expiryYears - step);
      });
      const shockedInstrument: Instrument = { ...fixed, expiryYears };
      const greeks = priceInstrument(shockedInstrument, market).greeks;
      points.push({ appliedShocks: applied, shockedMarket: market, greeks, expiryYears });
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
          vanilla: { optionType: p.delta >= 0 ? "CALL" : "PUT", strike: { kind: "strike", strike } },
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

  async listPositions(request: ListPositionsRequest): Promise<ListPositionsResponse> {
    const res: ListPositionsResponse = { positions: [] };
    if (request.correlationId !== undefined) res.correlationId = request.correlationId;
    // The offline book has no canonical-vanilla leaf per booked structure (the
    // seed positions are multi-leg strategies/vanillas without server-side
    // canonicalisation), so we honestly report no flat `RiskPosition` rows rather
    // than fabricate convention-free leaves the offline core cannot derive. The
    // aggregate (below) is the genuine offline risk view.
    return res;
  }

  async aggregateRisk(request: AggregateRiskRequest): Promise<AggregateRiskResponse> {
    const node = aggregateSeedBook(request.dimension, request.numeraire);
    const res: AggregateRiskResponse = {
      dimension: request.dimension,
      numeraire: request.numeraire.numeraire,
      nodes: node.positionCount > 0 ? [node] : [],
    };
    if (request.correlationId !== undefined) res.correlationId = request.correlationId;
    return res;
  }

  async drillRisk(request: DrillRiskRequest): Promise<DrillRiskResponse> {
    // The offline book is a single node; a drill to a finer dimension yields the
    // same genuine aggregate as its one child (honest: there is no finer org
    // structure offline). Positions are omitted for the same reason listPositions
    // reports none (no offline canonical leaf).
    const child = aggregateSeedBook(request.childDimension, request.numeraire);
    const res: DrillRiskResponse = {
      node: request.node,
      children: request.includeChildren && child.positionCount > 0 ? [child] : [],
      positions: [],
    };
    if (request.correlationId !== undefined) res.correlationId = request.correlationId;
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
    if (request.correlationId !== undefined) res.correlationId = request.correlationId;
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
    const conn = this.fixFromSpec(spec, spec.id?.trim() || mockSlugify(spec.name));
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

  async updateFixConnection(id: string, spec: FixConnectionSpec): Promise<FixConnection> {
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

  async setFixConnectionEnabled(id: string, enabled: boolean): Promise<FixConnection> {
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
      .filter((m) => connectionId === undefined || m.connectionId === connectionId)
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
    // Every managed connection belongs to a desk — no unowned "house" acceptors
    // (server parity: `def_from_spec` rejects a blank desk with `invalid_argument`).
    const desk = (spec.desk ?? "").trim();
    if (desk.length === 0) {
      throw new Error("a FIX connection must belong to a desk (select the owning desk)");
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
    const found = this.mockUsers.find((u) => u.user.email.toLowerCase() === key);
    // A single opaque error for every failure mode — never leak which factor failed.
    if (!found || found.user.disabled || found.password !== password) {
      throw new Error("invalid email or password");
    }
    this.mockTokenSeq += 1n;
    const token = `mock-session-${this.mockTokenSeq.toString()}`;
    this.mockTokens.add(token);
    // A 12-hour session, mirroring the server's TTL.
    const expiresNanos = nowNanos() + 12n * 60n * 60n * 1_000_000_000n;
    return { token, user: { ...found.user }, expiresNanos };
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
      throw new Error(`password must be at least ${MOCK_MIN_PASSWORD_LEN} characters`);
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
      disabled: false,
    };
    if (input.deskId && input.deskId.length > 0) user.deskId = input.deskId;
    this.mockUsers.push({ user, password: input.password });
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
      disabled: input.disabled,
    };
    if (input.deskId && input.deskId.length > 0) next.deskId = input.deskId;
    entry.user = next;
    return { ...next };
  }

  async deleteUser(id: string): Promise<boolean> {
    const idx = this.mockUsers.findIndex((u) => u.user.id === id);
    if (idx < 0) return false;
    const entry = this.mockUsers[idx]!;
    if (entry.user.role === "ADMIN" && !entry.user.disabled && this.activeAdminCount() <= 1) {
      throw new Error("cannot delete the last administrator");
    }
    this.mockUsers.splice(idx, 1);
    return true;
  }

  async resetPassword(id: string, newPassword: string): Promise<void> {
    const entry = this.mockUsers.find((u) => u.user.id === id);
    if (!entry) throw new Error(`no user with id \`${id}\``);
    if (newPassword.length < MOCK_MIN_PASSWORD_LEN) {
      throw new Error(`password must be at least ${MOCK_MIN_PASSWORD_LEN} characters`);
    }
    entry.password = newPassword;
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

  async deleteDesk(id: string): Promise<boolean> {
    const idx = this.mockDesks.findIndex((d) => d.id === id);
    if (idx < 0) return false;
    this.mockDesks.splice(idx, 1);
    // Members of the deleted desk become unassigned (server parity).
    for (const entry of this.mockUsers) {
      if (entry.user.deskId === id) {
        const { deskId: _dropped, ...rest } = entry.user;
        entry.user = rest;
      }
    }
    return true;
  }

  /** The number of enabled (non-disabled) administrators in the offline roster. */
  private activeAdminCount(): number {
    return this.mockUsers.filter((u) => u.user.role === "ADMIN" && !u.user.disabled).length;
  }
}

/** The minimum password length on create/reset (mirrors the server's `MIN_PASSWORD_LEN`). */
const MOCK_MIN_PASSWORD_LEN = 12;

/** A lowercase, hyphen-separated slug of `name` (mirrors the server's `slugify`). */
function mockSlugify(name: string): string {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug.length > 0 ? slug : "connection";
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
      throw new Error(`reporting numeraire ${numeraire.numeraire} has no rate for ${ccy}`);
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
    deltaByCcy.set(instrument.pair.base, (deltaByCcy.get(instrument.pair.base) ?? 0) + baseDelta);
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
    spec.kind === "strike" ? spec : { kind: "strike", strike: strikeFromDelta(spec.delta, m, t) };

  switch (instrument.product.kind) {
    case "vanilla": {
      const v = instrument.product.vanilla;
      return {
        ...instrument,
        product: { kind: "vanilla", vanilla: { ...v, strike: freezeStrike(v.strike) } },
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
            legs: s.legs.map((leg) => ({ ...leg, strike: freezeStrike(leg.strike) })),
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
  const pp = priceInstrument(instrument, applyShock(applyShock(m, a, ha, false), b, hb, false)).greeks.price;
  const pm = priceInstrument(instrument, applyShock(applyShock(m, a, ha, false), b, -hb, false)).greeks.price;
  const mp = priceInstrument(instrument, applyShock(applyShock(m, a, -ha, false), b, hb, false)).greeks.price;
  const mm = priceInstrument(instrument, applyShock(applyShock(m, a, -ha, false), b, -hb, false)).greeks.price;
  return (pp - pm - mp + mm) / (4 * ha * hb);
}

/** Construct the default standalone transport. */
export function createMockTransport(): CelnetTransport {
  void forward; // re-exported by pricing; referenced to keep tree-shaking honest
  void DEFAULT_CONVENTIONS;
  return new MockTransport();
}
