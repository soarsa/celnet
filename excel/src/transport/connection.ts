/**
 * The shared WS connection + multiplexed RFS session for the add-in.
 *
 * Ported from `gui/src/data/wsTransport.ts` (one contract, same WS mirror), with
 * the GUI's browser-`WebSocket` dependency replaced by the `WebSocketLike`/factory
 * seam (src/transport/socket.ts) so the identical code runs in the Office.js host
 * AND a node harness. It speaks the type-tagged snake_case JSON the server's
 * `crates/celnet-server/src/ws` mirror defines, encoded/decoded by the shared
 * contract codec (src/contract/wsCodec.ts).
 *
 * One connection IS one multiplexed RFS session AND the request/response channel
 * for pricing / RFQ / surface calls — exactly as the server multiplexes one gRPC
 * `StreamSession`. Robustness parity with the SDK/GUI: per-subscription sequence
 * tracking with gap-detect → server-assisted `resync`, snapshot rebaseline, a
 * transparent auto-reconnect with capped backoff that re-opens every live
 * subscription and resyncs from the last good sequence, and pending request/
 * response waiters failed fast on a drop (never an infinite await).
 *
 * Beyond the GUI: an explicit per-subscription **liveness / stale** indicator.
 * Each subscription tracks the wall-clock of its last server frame (snapshot /
 * update / heartbeat). A monitor flips a subscription to STALE when no frame —
 * including the contract's heartbeat — has arrived within a staleness window, so
 * a streamed cell is NEVER shown as live when the stream has silently frozen
 * (docs/EXCEL-INTEGRATION.md §5 "No stale stream"). The window is reset on every
 * frame; a resync/snapshot returns it to HEALTHY.
 */

import type {
  CcyPair,
  Conventions,
  Executed,
  Heartbeat,
  Instrument,
  MarketObservable,
  MarketSeriesPoint,
  MarketSeriesSnapshot,
  Quote,
  Snapshot,
  StreamHealth,
  StreamReject,
  Tenor,
  Update,
} from "../contract/contract";
import {
  ccyPairToWire,
  conventionsToWire,
  executedFromWire,
  greeksFromWire,
  heartbeatFromWire,
  instrumentToWire,
  marketSeriesPointFromWire,
  marketSeriesSnapshotFromWire,
  marketToWire,
  quoteFromWire,
  snapshotFromWire,
  streamRejectFromWire,
  updateFromWire,
  type WireObject,
} from "../contract/wsCodec";
import * as enums from "../contract/enums";
import type { MarketContext } from "../contract/contract";
import { WS_OPEN, type WebSocketFactory, type WebSocketLike } from "./socket";

/** A monotonic clock seam (injectable so the staleness logic is unit-testable). */
export type Clock = () => number;

/** Live transport tuning (all optional; sane defaults dial localhost). */
export interface ConnectionOptions {
  /** The mirror endpoint, e.g. `ws://127.0.0.1:8081`. */
  readonly url: string;
  /** Opens a `WebSocketLike` for a URL (browser or node `ws`). */
  readonly factory: WebSocketFactory;
  /** First reconnect backoff in ms (doubles up to `maxBackoffMs`). */
  readonly baseBackoffMs?: number;
  /** Reconnect backoff ceiling in ms. */
  readonly maxBackoffMs?: number;
  /** Per-request timeout in ms for request/response calls. */
  readonly requestTimeoutMs?: number;
  /**
   * Staleness window in ms: a subscription with no server frame (snapshot /
   * update / heartbeat) for longer than this is flipped to STALE. Must exceed the
   * server heartbeat cadence. `0` disables the monitor (tests drive it manually).
   */
  readonly stalenessWindowMs?: number;
  /** Monotonic clock (ms); defaults to `Date.now`. Injected for tests. */
  readonly clock?: Clock;
  /** Schedules a one-shot timer; defaults to `setTimeout`. Injected for tests. */
  readonly setTimer?: (fn: () => void, ms: number) => unknown;
  /** Cancels a timer from `setTimer`; defaults to `clearTimeout`. */
  readonly clearTimer?: (handle: unknown) => void;
}

const DEFAULT_BASE_BACKOFF_MS = 250;
const DEFAULT_MAX_BACKOFF_MS = 5_000;
const DEFAULT_REQUEST_TIMEOUT_MS = 10_000;
const DEFAULT_STALENESS_WINDOW_MS = 5_000;

/** An error surfaced when a request/response call cannot complete. */
export class TransportError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TransportError";
  }
}

/** A stream event delivered to a subscriber (custom function or task pane). */
export type StreamEvent =
  | { readonly kind: "snapshot"; readonly snapshot: Snapshot }
  | { readonly kind: "update"; readonly update: Update }
  | { readonly kind: "executed"; readonly executed: Executed }
  | { readonly kind: "reject"; readonly reject: StreamReject }
  | {
      readonly kind: "health";
      readonly subscriptionId: bigint;
      readonly health: StreamHealth;
    }
  | { readonly kind: "series_snapshot"; readonly snapshot: MarketSeriesSnapshot }
  | { readonly kind: "series_point"; readonly point: MarketSeriesPoint }
  | { readonly kind: "heartbeat"; readonly heartbeat: Heartbeat };

/** The shape of a market-series subscription request (one observable time series). */
export interface MarketSeriesRequest {
  readonly pair: CcyPair;
  readonly observable: MarketObservable;
  /** The pillar tenor for tenor-dependent observables (ATM_VOL/RR/BF/FORWARD). */
  readonly tenor?: Tenor;
  /** The signed delta wing for the wing observables (RR/BF), e.g. 0.25. */
  readonly delta?: number;
  /** Minimum nanoseconds between appended points the client wants (0 = none). */
  readonly throttleNanos?: bigint;
  /** Max history points in the opening snapshot (0 ⇒ server default window). */
  readonly historyLimit?: number;
}

/** A pending request/response waiter, keyed by its correlation id. */
interface Waiter {
  readonly expect: string;
  readonly resolve: (frame: WireObject) => void;
  readonly reject: (err: Error) => void;
  readonly timer: unknown;
}

/** Per-subscription state for gap-detect, reconnect, and staleness. */
interface Sub {
  readonly id: bigint;
  readonly instrument: Instrument;
  readonly conventions: Conventions;
  readonly label: string;
  /** The last in-sequence number applied (0 until first snapshot). */
  lastSequence: bigint;
  /** True once the baseline snapshot has been seen (post-subscribe/post-resync). */
  baselined: boolean;
  /** Wall-clock (ms) of the last server frame on this subscription. */
  lastFrameAt: number;
  /** The health last emitted, so the monitor only emits on a transition. */
  health: StreamHealth;
}

/**
 * The shared per-connection socket manager. Owns exactly one socket at a time,
 * routes inbound frames either to a request/response waiter (by `correlation_id`)
 * or to the live RFS session (by subscription id), and transparently reconnects
 * with capped backoff. The staleness monitor runs while any subscription is live.
 */
export class Connection {
  private ws: WebSocketLike | null = null;
  private readonly url: string;
  private readonly factory: WebSocketFactory;
  private readonly baseBackoffMs: number;
  private readonly maxBackoffMs: number;
  private readonly requestTimeoutMs: number;
  private readonly stalenessWindowMs: number;
  private readonly clock: Clock;
  private readonly setTimer: (fn: () => void, ms: number) => unknown;
  private readonly clearTimer: (handle: unknown) => void;

  private nextCorrelation = 1n;
  private backoff: number;
  private reconnectTimer: unknown = undefined;
  private stalenessTimer: unknown = undefined;
  private closed = false;

  private readonly waiters = new Map<bigint, Waiter>();
  /** Frames queued while the socket is not OPEN, flushed on connect. */
  private readonly outbox: string[] = [];

  private readonly subs = new Map<bigint, Sub>();
  /** Live market-series subscriptions (kept so a reconnect re-issues them). */
  private readonly seriesSubs = new Map<bigint, MarketSeriesRequest>();
  private readonly listeners = new Set<(e: StreamEvent) => void>();
  private nextSubId = 1n;
  private readonly stateListeners = new Set<(open: boolean) => void>();

  /**
   * The most recent server observability beat seen on this connection (drain-side
   * latency percentiles + ring conflation drops + provenance echo). Updated on
   * every `heartbeat` frame; `undefined` until the first beat arrives. Surfaced
   * by `CELNET.STATUS` so a desk sees the live server health without a stream row.
   */
  private lastHeartbeat: Heartbeat | undefined = undefined;

  constructor(opts: ConnectionOptions) {
    this.url = opts.url;
    this.factory = opts.factory;
    this.baseBackoffMs = opts.baseBackoffMs ?? DEFAULT_BASE_BACKOFF_MS;
    this.maxBackoffMs = opts.maxBackoffMs ?? DEFAULT_MAX_BACKOFF_MS;
    this.requestTimeoutMs = opts.requestTimeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS;
    this.stalenessWindowMs = opts.stalenessWindowMs ?? DEFAULT_STALENESS_WINDOW_MS;
    this.clock = opts.clock ?? (() => Date.now());
    this.setTimer = opts.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
    this.clearTimer = opts.clearTimer ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>));
    this.backoff = this.baseBackoffMs;
    this.open();
  }

  /** True iff the underlying socket is currently OPEN. */
  isOpen(): boolean {
    return this.ws?.readyState === WS_OPEN;
  }

  /**
   * The most recent server observability beat, or `undefined` if none has been
   * seen yet. Read by `CELNET.STATUS` to render live server health (drain-side
   * price latency percentiles, ring conflation drops, provenance echo).
   */
  latestHeartbeat(): Heartbeat | undefined {
    return this.lastHeartbeat;
  }

  onState(listener: (open: boolean) => void): () => void {
    this.stateListeners.add(listener);
    return () => this.stateListeners.delete(listener);
  }

  onEvent(listener: (event: StreamEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private emit(event: StreamEvent): void {
    for (const l of this.listeners) l(event);
  }

  private open(): void {
    if (this.closed) return;
    let ws: WebSocketLike;
    try {
      ws = this.factory(this.url);
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.ws = ws;
    ws.onopen = () => {
      this.backoff = this.baseBackoffMs;
      for (const frame of this.outbox.splice(0)) ws.send(frame);
      this.onReconnect();
      for (const l of this.stateListeners) l(true);
      this.armStalenessMonitor();
    };
    ws.onmessage = (data: string) => this.dispatch(data);
    ws.onerror = () => {
      // `onclose` follows; reconnect is handled there to avoid a double schedule.
    };
    ws.onclose = () => {
      if (this.ws === ws) this.ws = null;
      for (const l of this.stateListeners) l(false);
      // A drop is not (yet) durable failure: hold the blotter, flip live cells to
      // STALE rather than spraying errors, and fail outstanding waiters fast.
      this.markAllStale();
      this.failAllWaiters(new TransportError("connection closed; reconnecting"));
      this.scheduleReconnect();
    };
  }

  private scheduleReconnect(): void {
    if (this.closed || this.reconnectTimer !== undefined) return;
    const delay = this.backoff;
    this.backoff = Math.min(this.maxBackoffMs, this.backoff * 2);
    this.reconnectTimer = this.setTimer(() => {
      this.reconnectTimer = undefined;
      this.open();
    }, delay);
  }

  private dispatch(raw: string): void {
    let frame: WireObject;
    try {
      const parsed: unknown = JSON.parse(raw);
      if (!parsed || typeof parsed !== "object") return;
      frame = parsed as WireObject;
    } catch {
      return;
    }
    const type = typeof frame["type"] === "string" ? (frame["type"] as string) : "";
    const corr = frame["correlation_id"];
    if (typeof corr === "number" || typeof corr === "bigint") {
      const key = BigInt(corr as number | bigint);
      const waiter = this.waiters.get(key);
      if (waiter) {
        this.waiters.delete(key);
        this.clearTimer(waiter.timer);
        if (type === "error") {
          waiter.reject(new TransportError(String(frame["message"] ?? "server error")));
        } else {
          waiter.resolve(frame);
        }
        return;
      }
    }
    // Some contract reply messages do not carry a `correlation_id` (e.g. the
    // `smile` reply — the `Smile` message has no correlation field; the same is
    // true for any reply whose proto message lacks the optional field). Match
    // such a reply to the oldest in-flight waiter that `expect`s this reply type
    // (FIFO over a single connection preserves request/reply order). This keeps
    // the contract single — we do not invent a field the server must echo.
    if (type !== "" && type !== "error") {
      for (const [key, waiter] of this.waiters) {
        if (waiter.expect === type) {
          this.waiters.delete(key);
          this.clearTimer(waiter.timer);
          waiter.resolve(frame);
          return;
        }
      }
    }
    this.onServerFrame(type, frame);
  }

  /** Send a fire-and-forget control frame (queued if the socket is down). */
  send(frame: WireObject): void {
    const text = JSON.stringify(frame);
    if (this.isOpen() && this.ws) {
      this.ws.send(text);
    } else {
      this.outbox.push(text);
    }
  }

  /**
   * Issue a request/response call: mint a correlation id, send the request, and
   * resolve when the matching reply arrives (or reject on timeout / drop / error
   * frame). Never hangs — a closed connection fails the waiter immediately.
   */
  request(type: string, body: WireObject, expect: string): Promise<WireObject> {
    if (this.closed) {
      return Promise.reject(new TransportError("transport closed"));
    }
    const correlationId = this.nextCorrelation++;
    return new Promise<WireObject>((resolve, reject) => {
      const timer = this.setTimer(() => {
        if (this.waiters.delete(correlationId)) {
          reject(new TransportError(`request \`${type}\` timed out`));
        }
      }, this.requestTimeoutMs);
      this.waiters.set(correlationId, { expect, resolve, reject, timer });
      this.send({ ...body, type, correlation_id: Number(correlationId) });
    });
  }

  private failAllWaiters(err: Error): void {
    for (const [, w] of this.waiters) {
      this.clearTimer(w.timer);
      w.reject(err);
    }
    this.waiters.clear();
  }

  // --- RFS session surface (multiplexed on this one connection) -------------

  /** Register a subscription; returns its client `SubscriptionId`. */
  subscribe(instrument: Instrument, conventions: Conventions, label: string): bigint {
    const id = this.nextSubId++;
    this.subs.set(id, {
      id,
      instrument,
      conventions,
      label,
      lastSequence: 0n,
      baselined: false,
      lastFrameAt: this.clock(),
      health: "RESYNCING",
    });
    this.sendSubscribe(id, instrument, conventions);
    this.armStalenessMonitor();
    return id;
  }

  private sendSubscribe(id: bigint, instrument: Instrument, conventions: Conventions): void {
    this.send({
      type: "subscribe",
      subscription: { value: Number(id) },
      instrument: instrumentToWire(instrument),
      conventions: conventionsToWire(conventions),
      throttle_nanos: 0,
    });
  }

  unsubscribe(subscriptionId: bigint): void {
    if (!this.subs.delete(subscriptionId)) return;
    this.send({ type: "unsubscribe", subscription: { value: Number(subscriptionId) } });
  }

  execute(subscriptionId: bigint, token: bigint, idempotencyKey: string): void {
    this.send({
      type: "execute",
      subscription: { value: Number(subscriptionId) },
      token: Number(token),
      idempotency_key: idempotencyKey,
    });
  }

  /**
   * Open a market-series subscription (one observable time series) on the single
   * multiplexed session, in the same subscription-id space as price streams.
   * Returns the client `SubscriptionId`; `series_snapshot` then `series_point`
   * events arrive on the event listeners keyed by this id. Survives reconnect:
   * the request is re-issued on a fresh socket like a price subscription.
   */
  subscribeSeries(req: MarketSeriesRequest): bigint {
    const id = this.nextSubId++;
    this.seriesSubs.set(id, req);
    this.sendSeriesSubscribe(id, req);
    return id;
  }

  private sendSeriesSubscribe(id: bigint, req: MarketSeriesRequest): void {
    const frame: WireObject = {
      type: "market_series_subscribe",
      subscription: { value: Number(id) },
      pair: ccyPairToWire(req.pair),
      observable: enums.marketObservable.toWire(req.observable),
      throttle_nanos: Number(req.throttleNanos ?? 0n),
      history_limit: req.historyLimit ?? 0,
    };
    if (req.tenor) {
      frame["tenor"] = { unit: enums.tenorUnit.toWire(req.tenor.unit), count: req.tenor.count };
    }
    if (req.delta !== undefined) frame["delta"] = req.delta;
    this.send(frame);
  }

  unsubscribeSeries(subscriptionId: bigint): void {
    if (!this.seriesSubs.delete(subscriptionId)) return;
    this.send({
      type: "market_series_unsubscribe",
      subscription: { value: Number(subscriptionId) },
    });
  }

  /**
   * On a fresh connection (initial open or post-drop), re-issue every live
   * subscription, then resync each from its last good sequence so the server
   * replays anything missed (or re-baselines with a fresh snapshot).
   */
  private onReconnect(): void {
    if (this.closed) return;
    const now = this.clock();
    for (const sub of this.subs.values()) {
      sub.baselined = false;
      sub.lastFrameAt = now;
      this.sendSubscribe(sub.id, sub.instrument, sub.conventions);
      if (sub.lastSequence > 0n) {
        this.send({
          type: "resync",
          subscription: { value: Number(sub.id) },
          last_sequence: Number(sub.lastSequence),
        });
      }
      this.setHealth(sub, "RESYNCING");
    }
    // Re-open every live market-series subscription (the server re-baselines each
    // with a fresh `series_snapshot`); no per-series sequence resync is needed —
    // a series is a conflatable trend, so a fresh baseline is the recovery.
    for (const [id, req] of this.seriesSubs) this.sendSeriesSubscribe(id, req);
  }

  /** Route an inbound RFS server frame (already typed by `dispatch`). */
  private onServerFrame(type: string, frame: WireObject): void {
    switch (type) {
      case "snapshot": {
        const snapshot = snapshotFromWire(frame);
        const sub = this.subs.get(snapshot.subscriptionId);
        if (!sub) return;
        sub.lastSequence = snapshot.sequence;
        sub.baselined = true;
        sub.lastFrameAt = this.clock();
        this.emit({ kind: "snapshot", snapshot });
        this.setHealth(sub, "HEALTHY");
        break;
      }
      case "update": {
        const update = updateFromWire(frame);
        const sub = this.subs.get(update.subscriptionId);
        if (!sub || !sub.baselined) return;
        sub.lastFrameAt = this.clock();
        const expected = sub.lastSequence + 1n;
        if (update.sequence > expected) {
          // Detected a gap: ask the server to resync from our last good sequence
          // and mark the row resyncing until a fresh baseline lands. We still
          // apply this update so the price stays live, then reconcile on snapshot.
          this.setHealth(sub, "RESYNCING");
          this.send({
            type: "resync",
            subscription: { value: Number(sub.id) },
            last_sequence: Number(sub.lastSequence),
          });
        } else {
          this.setHealth(sub, "HEALTHY");
        }
        if (update.sequence > sub.lastSequence) sub.lastSequence = update.sequence;
        this.emit({ kind: "update", update });
        break;
      }
      case "heartbeat": {
        // Liveness only: a heartbeat carries the current sequence so a silent gap
        // is detectable and resets the staleness window — but does not advance the
        // applied sequence. A heartbeat whose sequence is ahead of ours signals a
        // missed update; resync to recover. The beat also carries the server's
        // drain-side observability (latency percentiles + ring conflation drops +
        // provenance echo); cache the latest for CELNET.STATUS and emit it so a
        // live consumer can react without polling.
        const beat = heartbeatFromWire(frame);
        this.lastHeartbeat = beat;
        this.emit({ kind: "heartbeat", heartbeat: beat });
        const subId = subscriptionIdOf(frame);
        if (subId === undefined) {
          // Connection-level heartbeat: refresh every subscription's liveness.
          const now = this.clock();
          for (const sub of this.subs.values()) sub.lastFrameAt = now;
          break;
        }
        const sub = this.subs.get(subId);
        if (!sub) break;
        sub.lastFrameAt = this.clock();
        const seq = bigField(frame, "sequence");
        if (sub.baselined && seq > sub.lastSequence) {
          this.setHealth(sub, "RESYNCING");
          this.send({
            type: "resync",
            subscription: { value: Number(sub.id) },
            last_sequence: Number(sub.lastSequence),
          });
        }
        break;
      }
      case "executed": {
        this.emit({ kind: "executed", executed: executedFromWire(frame) });
        break;
      }
      case "stream_reject": {
        this.emit({ kind: "reject", reject: streamRejectFromWire(frame) });
        break;
      }
      case "stream_end": {
        const subId = subscriptionIdOf(frame);
        if (subId === undefined) break;
        const sub = this.subs.get(subId);
        if (sub) {
          sub.baselined = false;
          this.setHealth(sub, "STALE");
        }
        break;
      }
      case "market_series_snapshot": {
        const snapshot = marketSeriesSnapshotFromWire(frame);
        if (!this.seriesSubs.has(snapshot.subscriptionId)) break;
        this.emit({ kind: "series_snapshot", snapshot });
        break;
      }
      case "market_series_point": {
        const point = marketSeriesPointFromWire(frame);
        if (!this.seriesSubs.has(point.subscriptionId)) break;
        this.emit({ kind: "series_point", point });
        break;
      }
      default:
        break;
    }
  }

  // --- staleness monitor ----------------------------------------------------

  /** Arm the periodic staleness sweep if a window is configured and not closed. */
  private armStalenessMonitor(): void {
    if (this.closed || this.stalenessWindowMs <= 0) return;
    if (this.stalenessTimer !== undefined) return;
    if (this.subs.size === 0) return;
    const tick = (): void => {
      this.stalenessTimer = undefined;
      this.sweepStaleness();
      // Re-arm while there is anything to watch.
      if (!this.closed && this.subs.size > 0) {
        this.stalenessTimer = this.setTimer(tick, this.stalenessWindowMs);
      }
    };
    this.stalenessTimer = this.setTimer(tick, this.stalenessWindowMs);
  }

  /**
   * Flip any subscription with no frame within the staleness window to STALE.
   * Exposed for the unit harness to drive deterministically with an injected
   * clock (no real timers needed). A subsequent frame returns it to HEALTHY.
   */
  sweepStaleness(): void {
    const now = this.clock();
    for (const sub of this.subs.values()) {
      if (now - sub.lastFrameAt >= this.stalenessWindowMs && sub.health !== "STALE") {
        this.setHealth(sub, "STALE");
      }
    }
  }

  private markAllStale(): void {
    for (const sub of this.subs.values()) {
      sub.baselined = false;
      this.setHealth(sub, "STALE");
    }
  }

  private setHealth(sub: Sub, health: StreamHealth): void {
    if (sub.health === health) return;
    sub.health = health;
    this.emit({ kind: "health", subscriptionId: sub.id, health });
  }

  /** Tear down the connection permanently (no further reconnects). */
  close(): void {
    this.closed = true;
    if (this.reconnectTimer !== undefined) {
      this.clearTimer(this.reconnectTimer);
      this.reconnectTimer = undefined;
    }
    if (this.stalenessTimer !== undefined) {
      this.clearTimer(this.stalenessTimer);
      this.stalenessTimer = undefined;
    }
    this.failAllWaiters(new TransportError("transport closed"));
    this.subs.clear();
    this.seriesSubs.clear();
    this.listeners.clear();
    const ws = this.ws;
    this.ws = null;
    if (ws) {
      ws.onopen = null;
      ws.onmessage = null;
      ws.onerror = null;
      ws.onclose = null;
      try {
        ws.close();
      } catch {
        // already closing
      }
    }
  }

  // --- request/response surface (pricing / RFQ / surface) -------------------

  async price(
    instrument: Instrument,
    market: MarketContext,
    conventions: Conventions,
  ): Promise<{
    greeks: ReturnType<typeof greeksFromWire>;
    resolvedStrike: number;
    surfaceVersion: bigint;
    /** The MC standard error (proto `price_std_error`), present ONLY for a
     * Monte-Carlo-priced product; `undefined` for an exact closed form. */
    priceStdError?: number;
  }> {
    const reply = await this.request(
      "price",
      {
        instrument: instrumentToWire(instrument),
        market: marketToWire(market),
        conventions: conventionsToWire(conventions),
      },
      "price_response",
    );
    const out: {
      greeks: ReturnType<typeof greeksFromWire>;
      resolvedStrike: number;
      surfaceVersion: bigint;
      priceStdError?: number;
    } = {
      greeks: greeksFromWire(asChild(reply, "greeks")),
      resolvedStrike: numField(reply, "resolved_strike"),
      surfaceVersion: bigField(reply, "surface_version"),
    };
    // Presence-tracked: surface the server's MC stderr only when present (the same
    // honest contract `quoteFromWire` follows), so a closed-form price never
    // fabricates a precision claim and the MC conformance band can combine it.
    const stdErr = optNumField(reply, "price_std_error");
    if (stdErr !== undefined) out.priceStdError = stdErr;
    return out;
  }

  async requestQuote(
    instrument: Instrument,
    conventions: Conventions,
    idempotencyKey: string,
  ): Promise<Quote> {
    const reply = await this.request(
      "request_quote",
      {
        idempotency_key: idempotencyKey,
        instrument: instrumentToWire(instrument),
        conventions: conventionsToWire(conventions),
      },
      "quote",
    );
    return quoteFromWire(reply);
  }

  async getSmile(
    pair: CcyPair,
    tenorYears: number,
    conventions: Conventions,
  ): Promise<WireObject> {
    return this.request(
      "get_smile",
      {
        pair: ccyPairToWire(pair),
        tenor_years: tenorYears,
        conventions: conventionsToWire(conventions),
      },
      "smile",
    );
  }

  async markSurface(body: WireObject): Promise<WireObject> {
    return this.request("mark_surface", body, "mark_surface_response");
  }

  // --- RiskService (server-side hierarchical risk) --------------------------
  //
  // The four `RiskService` RPCs over the same single multiplexed connection. The
  // body is the snake_case proto request (built by `src/contract/riskCodec.ts`);
  // the reply is matched by the echoed `correlation_id` the server stamps (and,
  // defensively, by reply `type` as a fallback). Aggregation is SERVER-OWNED — the
  // client sends scope/principal/numeraire and receives the rolled-up node tree;
  // it never loops positions and sums (the API-first parity rule).

  /** `RiskService.ListPositions` — the entitled open book (each with attribution). */
  async listPositions(body: WireObject): Promise<WireObject> {
    return this.request("list_positions", body, "list_positions_response");
  }

  /** `RiskService.AggregateRisk` — the SERVER's rolled-up node tree for a dimension. */
  async aggregateRisk(body: WireObject): Promise<WireObject> {
    return this.request("aggregate_risk", body, "aggregate_risk_response");
  }

  /** `RiskService.DrillRisk` — drill a node into child sub-nodes and/or positions. */
  async drillRisk(body: WireObject): Promise<WireObject> {
    return this.request("drill_risk", body, "drill_risk_response");
  }

  /** `RiskService.LimitStatus` — per-limit utilization/RAG for a scope node. */
  async limitStatus(body: WireObject): Promise<WireObject> {
    return this.request("limit_status", body, "limit_status_response");
  }
}

// ---------------------------------------------------------------------------
// frame helpers
// ---------------------------------------------------------------------------

function asChild(o: WireObject, key: string): WireObject {
  const v = o[key];
  return v && typeof v === "object" ? (v as WireObject) : {};
}

/** Read an optional finite number field; absent / null / non-finite ⇒ undefined. */
function optNumField(o: WireObject, key: string): number | undefined {
  const v = o[key];
  return typeof v === "number" && Number.isFinite(v) ? v : undefined;
}

function numField(o: WireObject, key: string): number {
  const v = o[key];
  return typeof v === "number" ? v : 0;
}

function bigField(o: WireObject, key: string): bigint {
  const v = o[key];
  if (typeof v === "number" && Number.isFinite(v)) return BigInt(Math.trunc(v));
  if (typeof v === "bigint") return v;
  return 0n;
}

/** The subscription id of an RFS server frame, or undefined for a bare frame. */
function subscriptionIdOf(o: WireObject): bigint | undefined {
  const sub = o["subscription"];
  if (!sub || typeof sub !== "object") return undefined;
  const value = (sub as WireObject)["value"];
  if (typeof value === "number") return BigInt(value);
  if (typeof value === "bigint") return value;
  return undefined;
}
