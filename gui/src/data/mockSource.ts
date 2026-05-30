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
  BrokerQuoteSet,
  CcyPair,
  Conventions,
  Executed,
  Execution,
  Greeks,
  Instrument,
  MarkedSurface,
  MarketContext,
  Quote,
  ScenarioPoint,
  ScenarioResult,
  ShockAxis,
  ShockFactor,
  Smile,
  Snapshot,
  TradableToken,
  TwoWayPrice,
  Update,
  VegaBucket,
} from "./contract";
import { forward, priceInstrument, strikeFromDelta } from "./pricing";
import { Rng } from "./rng";
import {
  brokerLadder,
  DEFAULT_CONVENTIONS,
  PAIRS,
  TENOR_LADDER,
  type PairContext,
} from "./seed";
import { calibrateSmile, markSurface } from "./surface";
import type {
  CelnetTransport,
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
}

/**
 * The mock multiplexed stream session. One instance multiplexes many
 * subscriptions over a single ticking loop, exactly as the contract's single
 * bidirectional `StreamSession` multiplexes by `SubscriptionId`.
 */
class MockStreamSession implements StreamSession {
  private readonly subs = new Map<bigint, LiveSubscription>();
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

  close(): void {
    if (this.timer !== undefined) {
      clearInterval(this.timer);
      this.timer = undefined;
    }
    this.subs.clear();
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

  private tick(): void {
    this.frame += 1;
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

      // Only a fraction of subscriptions tick each frame (calm under fire): the
      // tape is bursty but the blotter flashes only the changed lines.
      const draw = sub.rng.next();
      if (draw > 0.55) continue;

      // Mean-reverting vol random walk (Ornstein-Uhlenbeck-flavoured), deterministic.
      const anchor = sub.ctx.market.vol;
      sub.vol += 0.04 * (anchor - sub.vol) + 0.0006 * sub.rng.normal();
      sub.vol = Math.max(0.01, sub.vol);

      const { price, greeks, strike } = this.priceSub(sub);
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
  private readonly quotes = new Map<bigint, { quote: Quote; instrument: Instrument }>();
  private readonly idempotency = new Map<string, Quote>();

  constructor(opts: { seed?: bigint; tickMs?: number } = {}) {
    this.seed = opts.seed ?? 0xce1_5eed_d00dn;
    this.tickMs = opts.tickMs ?? 100; // 10 Hz tape; render conflates to a frame
  }

  async price(
    instrument: Instrument,
    market: MarketContext,
    conventions: Conventions,
  ): Promise<PriceResult> {
    const { greeks, resolvedStrike } = priceInstrument(instrument, market);
    const midPct = Math.abs(greeks.price / market.spot) * 100;
    const spread = Math.max(0.004, Math.abs(greeks.vega) * 0.06 + instrument.expiryYears * 0.02);
    return {
      greeks,
      resolvedStrike,
      conventions,
      twoWay: twoWayAround(midPct, spread),
      surfaceVersion: this.surfaceVersion,
    };
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
    this.quotes.set(quote.quoteId, { quote, instrument });
    this.idempotency.set(idempotencyKey, quote);
    return quote;
  }

  async acceptQuote(
    quoteId: bigint,
    side: "BUY" | "SELL",
    _idempotencyKey: string,
  ): Promise<Execution> {
    const entry = this.quotes.get(quoteId);
    if (!entry) throw new Error(`unknown quote ${quoteId}`);
    const now = nowNanos();
    if (entry.quote.validUntilNanos <= now) throw new Error("quote expired (last-look)");
    const premium = side === "BUY" ? entry.quote.price.offer : entry.quote.price.bid;
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
  ): Promise<MarkedSurface> {
    this.surfaceVersion += 1n;
    return markSurface(pair, brokerQuotes, conventions, this.surfaceVersion, nowNanos());
  }

  async scenario(
    instrument: Instrument,
    baseMarket: MarketContext,
    _conventions: Conventions,
    axes: ShockAxis[],
  ): Promise<ScenarioResult> {
    const points: ScenarioPoint[] = [];
    // Lock strikes to absolute levels at the base market so a spot/vol shock
    // moves a *fixed* position (a real book has fixed strikes), rather than
    // silently re-striking to the same delta on every shocked market — which
    // would neutralize the directional P&L the trader is shocking for.
    const fixed = freezeStrikes(instrument, baseMarket);
    const basePrice = priceInstrument(fixed, baseMarket).greeks.price;

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

    const vegaBuckets = this.bucketVega(instrument, baseMarket);
    return {
      points,
      bucketedRisk: {
        vegaBuckets,
        crossGammas: [
          {
            factorA: "SPOT",
            factorB: "VOL",
            value: crossGamma(instrument, baseMarket, "SPOT", "VOL"),
          },
        ],
        thetaRoll: [1 / 365, 3 / 365, 7 / 365].map((h) => {
          const rolled: Instrument = {
            ...fixed,
            expiryYears: Math.max(1 / 365, fixed.expiryYears - h),
          };
          return priceInstrument(rolled, baseMarket).greeks.price - basePrice;
        }),
        rollHorizonsYears: [1 / 365, 3 / 365, 7 / 365],
      },
    };
  }

  /** Bucketed vega across (tenor, delta-pillar) — the desk's hedge representation. */
  private bucketVega(instrument: Instrument, market: MarketContext): VegaBucket[] {
    const pillars = [-0.1, -0.25, 0.5, 0.25, 0.1];
    return TENOR_LADDER.slice(0, 5).flatMap((t) =>
      pillars.map((delta) => {
        const strike = strikeFromDelta(delta, market, t.years);
        const bumped: Instrument = {
          ...instrument,
          expiryYears: t.years,
          product: {
            kind: "vanilla",
            vanilla: { optionType: delta >= 0 ? "CALL" : "PUT", strike: { kind: "strike", strike } },
          },
        };
        const vega = priceInstrument(bumped, market).greeks.vega;
        return { tenorYears: t.years, delta, vega };
      }),
    );
  }
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

  if (instrument.product.kind === "vanilla") {
    const v = instrument.product.vanilla;
    return {
      ...instrument,
      product: { kind: "vanilla", vanilla: { ...v, strike: freezeStrike(v.strike) } },
    };
  }
  const s = instrument.product.strategy;
  return {
    ...instrument,
    product: {
      kind: "strategy",
      strategy: { ...s, legs: s.legs.map((leg) => ({ ...leg, strike: freezeStrike(leg.strike) })) },
    },
  };
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
