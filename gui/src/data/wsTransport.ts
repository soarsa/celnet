/**
 * The live WebSocket transport — a real `CelnetTransport` (src/data/transport.ts)
 * over the `celnet-server` WebSocket JSON mirror (crates/celnet-server/src/ws).
 * It is selectable alongside the deterministic in-app mock (the default), so the
 * GUI runs standalone out of the box and only dials a server when configured to.
 *
 * It speaks the SAME single, current `celnet.wire` contract the gRPC front speaks,
 * encoded as the type-tagged snake_case JSON the mirror defines (src/data/wsCodec.ts
 * mirrors the server's codec.rs field-for-field). One WS connection IS one
 * multiplexed RFS session AND the request/response channel for pricing / RFQ /
 * surface calls — exactly as the server multiplexes one gRPC `StreamSession`.
 *
 * Robustness parity with the SDK (crates/celnet-client): per-subscription sequence
 * tracking with gap-detect → server-assisted `resync`, snapshot rebaseline, and a
 * transparent auto-reconnect with capped exponential backoff that re-opens every
 * live subscription and re-issues a resync from the last good sequence — so a
 * blue-green cutover or a transient drop is recovered without losing the blotter.
 * Pending request/response calls outstanding across a drop are failed fast (never
 * left awaiting forever), matching the SDK's reconnect-liveness contract.
 */

import type {
  AcceptDeskQuoteRequest,
  AcceptDeskQuoteResponse,
  AggregateRatesRiskRequest,
  AggregateRatesRiskResponse,
  AggregateRiskRequest,
  AggregateRiskResponse,
  CombinedTailRiskRequest,
  CombinedTailRiskResponse,
  BookRatesPositionRequest,
  BookRatesPositionResponse,
  BrokerQuoteSet,
  CcyPair,
  Conventions,
  CreateUserInput,
  DeskDesc,
  EntityDesc,
  EntityInput,
  BookDesc,
  BookInput,
  AggregatedBookDesc,
  AggregatedBookSpec,
  FeaturePipeline,
  PricingGroup,
  PricingMode,
  RiskBook,
  RiskBookRisk,
  RiskRoutingGraph,
  InstrumentDef,
  InstrumentInput,
  BuildCurveRequest,
  CalibratedCurve,
  GetCurveResult,
  MarkedCurve,
  CurveScenarioResult,
  DrillRiskRequest,
  DrillRiskResponse,
  Execution,
  ListDealsRequest,
  ListDealsResponse,
  ListDeskRequestsRequest,
  ListDeskRequestsResponse,
  ListRatesPositionsRequest,
  ListRatesPositionsResponse,
  Notification,
  NotificationScope,
  RespondDeskRequestRequest,
  RespondDeskRequestResponse,
  SubmitDeskRequestRequest,
  SubmitDeskRequestResponse,
  FixConnection,
  FixConnectionSpec,
  FixMessagePage,
  Instrument,
  LoginResult,
  LimitStatusRequest,
  LimitStatusResponse,
  ListPositionsRequest,
  ListPositionsResponse,
  MarkedSurface,
  MarketContext,
  MultiDealerQuote,
  Quote,
  RatesCurveSet,
  RatesInstrument,
  RatesPricingResult,
  RatesQuote,
  RiskBucketRequest,
  ScenarioResult,
  ShockAxis,
  Side,
  Smile,
  SmileModel,
  UpdateUserInput,
  UserCapabilities,
  RoleCapabilities,
  UserRole,
  UserDesc,
  Capability,
  XvaPricingRequest,
  XvaResult,
} from "./contract";
import {
  acceptDeskQuoteToWire,
  acceptDeskQuoteResponseFromWire,
  aggregateRatesRiskRequestToWire,
  aggregateRatesRiskResponseFromWire,
  aggregateRiskRequestToWire,
  aggregateRiskResponseFromWire,
  combinedTailRiskRequestToWire,
  combinedTailRiskResponseFromWire,
  bookRatesPositionToWire,
  bookRatesPositionResponseFromWire,
  ccyPairToWire,
  conventionsToWire,
  listDealsToWire,
  listDealsResponseFromWire,
  listDeskRequestsToWire,
  listDeskRequestsResponseFromWire,
  listRatesPositionsToWire,
  listRatesPositionsResponseFromWire,
  notificationFromWire,
  respondDeskRequestToWire,
  respondDeskRequestResponseFromWire,
  submitDeskRequestToWire,
  submitDeskRequestResponseFromWire,
  subscribeNotificationsToWire,
  brokerQuoteSetToWire,
  createFixConnectionRequestToWire,
  deleteFixConnectionRequestToWire,
  drillRiskRequestToWire,
  drillRiskResponseFromWire,
  executedFromWire,
  executionFromWire,
  fixConnectionResponseFromWire,
  greeksFromWire,
  heartbeatFromWire,
  instrumentToWire,
  limitStatusRequestToWire,
  limitStatusResponseFromWire,
  listFixConnectionsRequestToWire,
  listFixConnectionsResponseFromWire,
  listFixMessagesRequestToWire,
  listFixMessagesResponseFromWire,
  createDeskRequestToWire,
  updateDeskRequestToWire,
  createUserRequestToWire,
  deleteDeskRequestToWire,
  listEntitiesRequestToWire,
  entitiesResponseFromWire,
  createEntityRequestToWire,
  updateEntityRequestToWire,
  entityResponseFromWire,
  deleteEntityRequestToWire,
  listBooksRequestToWire,
  booksResponseFromWire,
  createBookRequestToWire,
  updateBookRequestToWire,
  bookResponseFromWire,
  deleteBookRequestToWire,
  listInstrumentsRequestToWire,
  instrumentsResponseFromWire,
  getInstrumentRequestToWire,
  instrumentResponseFromWire,
  createInstrumentRequestToWire,
  updateInstrumentRequestToWire,
  deleteInstrumentRequestToWire,
  deleteInstrumentResponseFromWire,
  buildCurveRequestToWire,
  calibratedCurveFromWire,
  getCurveRequestToWire,
  getCurveResultFromWire,
  markCurveRequestToWire,
  markedCurveFromWire,
  curveScenarioRequestToWire,
  curveScenarioResultFromWire,
  deleteUserRequestToWire,
  deskResponseFromWire,
  listDesksRequestToWire,
  listDesksResponseFromWire,
  listUsersRequestToWire,
  listUsersResponseFromWire,
  getUserCapabilitiesRequestToWire,
  setUserCapabilitiesRequestToWire,
  userCapabilitiesFromWire,
  getRoleCapabilitiesRequestToWire,
  setRoleCapabilitiesRequestToWire,
  roleCapabilitiesFromWire,
  loginRequestToWire,
  loginResultFromWire,
  logoutRequestToWire,
  resetPasswordRequestToWire,
  updateUserRequestToWire,
  userResponseFromWire,
  listPositionsRequestToWire,
  listPositionsResponseFromWire,
  markedSurfaceFromWire,
  marketSeriesPointFromWire,
  marketSeriesSnapshotFromWire,
  marketSeriesSubscribeToWire,
  marketSeriesUnsubscribeToWire,
  marketToWire,
  multiDealerQuoteFromWire,
  principalOrGrantAllToWire,
  parseFrame,
  quoteAcceptToWire,
  quoteFromWire,
  priceXvaRequestToWire,
  ratesCurveSetToWire,
  ratesInstrumentUnionToWire,
  ratesPricingResultFromWire,
  ratesQuoteFromWire,
  ratesQuoteRequestToWire,
  ratesStreamSnapshotFromWire,
  ratesStreamUpdateFromWire,
  ratesSubscribeToWire,
  listAggregatedBooksRequestToWire,
  aggregatedBooksResponseFromWire,
  createAggregatedBookRequestToWire,
  updateAggregatedBookRequestToWire,
  deleteAggregatedBookRequestToWire,
  aggregatedBookResponseFromWire,
  listPricingGroupsRequestToWire,
  pricingGroupsResponseFromWire,
  createPricingGroupRequestToWire,
  updatePricingGroupRequestToWire,
  deletePricingGroupRequestToWire,
  updatePricingGroupPipelineRequestToWire,
  pricingGroupResponseFromWire,
  pricingModeToWire,
  listRiskBooksRequestToWire,
  riskBooksResponseFromWire,
  createRiskBookRequestToWire,
  updateRiskBookRequestToWire,
  deleteRiskBookRequestToWire,
  riskBookResponseFromWire,
  getRiskRoutingGraphRequestToWire,
  riskRoutingGraphResponseFromWire,
  updateRiskRoutingGraphRequestToWire,
  updateRiskRoutingGraphResponseFromWire,
  listRiskBookRiskRequestToWire,
  riskBookRiskResponseFromWire,
  riskBookRiskSubscribeToWire,
  riskBookRiskStreamSnapshotFromWire,
  riskBookRiskStreamUpdateFromWire,
  aggregatedBookSubscribeToWire,
  aggregatedBookSnapshotFromWire,
  aggregatedBookUpdateFromWire,
  riskBucketRequestToWire,
  xvaResultFromWire,
  scenarioResultFromWire,
  serializeFrame,
  setFixConnectionEnabledRequestToWire,
  shockAxisToWire,
  smileFromWire,
  smileModelToWire,
  updateFixConnectionRequestToWire,
  snapshotFromWire,
  streamRejectFromWire,
  updateFromWire,
  type WireObject,
} from "./wsCodec";
import type {
  CelnetTransport,
  MarketSeriesParams,
  PriceResult,
  StreamEvent,
  StreamSession,
} from "./transport";

/** Live transport tuning (all optional; sane defaults dial localhost). */
export interface WsTransportOptions {
  /** The mirror endpoint, e.g. `ws://127.0.0.1:8081`. */
  readonly url: string;
  /** First reconnect backoff in ms (doubles up to `maxBackoffMs`). */
  readonly baseBackoffMs?: number;
  /** Reconnect backoff ceiling in ms. */
  readonly maxBackoffMs?: number;
  /** Per-request timeout in ms for request/response calls. */
  readonly requestTimeoutMs?: number;
}

const DEFAULT_BASE_BACKOFF_MS = 250;
const DEFAULT_MAX_BACKOFF_MS = 5_000;
const DEFAULT_REQUEST_TIMEOUT_MS = 10_000;

/**
 * Client keepalive cadence (ms). The browser `WebSocket` API cannot emit
 * protocol-level ping frames, so an otherwise-idle GUI session (no live price
 * subscriptions ticking) puts ZERO bytes on the wire — and a reverse proxy in
 * front of the server reaps the idle socket at its idle timeout (commonly 60s).
 * That surfaced as the app flickering to the reconnecting overlay every idle
 * cycle: close → overlay → reconnect → idle → close. We instead send the
 * contract's client→server `heartbeat` stream-control frame (a pure liveness
 * no-op server-side — `services/stream.rs` handles `Message::Heartbeat` by doing
 * nothing) on this interval. 20s stays comfortably under a 60s proxy idle timeout
 * while adding negligible traffic, and keeps the OS TCP write path exercised so a
 * silently-dead peer surfaces as a real `onclose` promptly instead of hanging
 * half-open.
 */
const KEEPALIVE_INTERVAL_MS = 20_000;

/**
 * The per-call deadline for PRICING-class requests (`price`, `request_quote`,
 * `request_multi_dealer_quote`, `scenario`) — the calls whose latency is the
 * server's pricing engines, not the wire. The Monte-Carlo/LSM families (TARF,
 * accumulator, American) legitimately price in tens of seconds on a contended
 * box; at the 10s default the transport abandoned the waiter and DISCARDED the
 * server's later (correct) quote, so the heavy families could never quote from
 * the live ticket at all. 90s matches the per-call pricing deadline of the
 * other clients on the one contract (Excel `wsClient`, the GUI e2e
 * `edgeClient`) — a HANG detector covering the heaviest MC family, not a
 * latency gate (latency budgets are gated in `celnet-bench` on a quiet
 * machine).
 */
const PRICING_REQUEST_TIMEOUT_MS = 90_000;

/** An error surfaced when a request/response call cannot complete. */
export class WsTransportError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "WsTransportError";
  }
}

/** A pending request/response waiter, keyed by its correlation id. */
interface Waiter {
  /** The reply frame `type` this waiter expects (e.g. "quote"). */
  readonly expect: string;
  readonly resolve: (frame: WireObject) => void;
  readonly reject: (err: Error) => void;
  readonly timer: ReturnType<typeof setTimeout>;
}

/**
 * The shared per-connection socket manager. Owns exactly one WebSocket at a time,
 * routes inbound frames either to a request/response waiter (by `correlation_id`)
 * or to the live RFS session, and transparently reconnects with capped backoff.
 * One instance backs both the request/response surface and the single multiplexed
 * stream session, exactly as one WS connection serves both on the server.
 */
class WsConnection {
  private ws: WebSocket | null = null;
  private readonly url: string;
  private readonly baseBackoffMs: number;
  private readonly maxBackoffMs: number;
  private readonly requestTimeoutMs: number;
  private nextCorrelation = 1n;
  private backoff: number;
  private reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  /** The idle-keepalive ticker; live only while the socket is OPEN. */
  private keepaliveTimer: ReturnType<typeof setInterval> | undefined;
  private closed = false;
  private readonly waiters = new Map<bigint, Waiter>();
  /** Frames queued while the socket is not OPEN, flushed on connect. */
  private readonly outbox: string[] = [];
  /** The live RFS session bound to this connection, if any. */
  private session: WsStreamSession | null = null;
  /**
   * Live notification subscriptions, keyed by an opaque token: each holds its
   * desk scope (so a reconnect re-opens it) and the decoded-frame handler the
   * `notification` push frames route to. The notification stream is a dedicated
   * server→client channel (NOT request/response), so it bypasses the waiter map
   * and is dispatched by frame `type`.
   */
  private readonly notificationSubs = new Map<
    symbol,
    { readonly scope: NotificationScope | undefined; readonly onFrame: (frame: WireObject) => void }
  >();
  /** Connection-state listeners (for the status ribbon / debugging). */
  private readonly stateListeners = new Set<(open: boolean) => void>();
  /**
   * The bearer session token from `AuthService.Login`, injected into every
   * request envelope when set (the server authenticates the caller from it and
   * role-gates admin RPCs). `null` ⇒ the anonymous/legacy principal path. A
   * secret — held only in memory, never persisted or logged.
   */
  private sessionToken: string | null = null;

  constructor(opts: WsTransportOptions) {
    this.url = opts.url;
    this.baseBackoffMs = opts.baseBackoffMs ?? DEFAULT_BASE_BACKOFF_MS;
    this.maxBackoffMs = opts.maxBackoffMs ?? DEFAULT_MAX_BACKOFF_MS;
    this.requestTimeoutMs = opts.requestTimeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS;
    this.backoff = this.baseBackoffMs;
    this.open();
  }

  /** True iff the underlying socket is currently OPEN. */
  isOpen(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  onState(listener: (open: boolean) => void): () => void {
    this.stateListeners.add(listener);
    return () => this.stateListeners.delete(listener);
  }

  /** Bind the (single) live RFS session so reconnects can re-open it. */
  bindSession(session: WsStreamSession | null): void {
    this.session = session;
  }

  private open(): void {
    if (this.closed) return;
    let ws: WebSocket;
    try {
      ws = new WebSocket(this.url);
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.ws = ws;
    ws.onopen = () => {
      this.backoff = this.baseBackoffMs;
      // Authenticate the session FIRST — the server pins the caller from this frame
      // before any subscribe/execute (the Enforce posture rejects an un-authenticated
      // session). Carry our session token when we hold one, plus an explicit grant-all
      // principal default (mirroring the SDK's `principal_or_grant_all`) so the headline
      // GUI workflow is admitted under Enforce without relying on the server granting an
      // absent caller. Sent directly (not via the outbox) so it is the literal first
      // frame on every (re)connect — a re-dialed socket is anonymous server-side.
      ws.send(JSON.stringify(this.authenticateFrame()));
      // Then flush anything queued while down, then let the bound session re-establish
      // its subscriptions (fresh subscribe + resync from last good sequence).
      for (const frame of this.outbox.splice(0)) ws.send(frame);
      this.session?.onReconnect();
      // Re-open every live notification subscription from the last good scope so
      // the push stream survives a blue-green cutover / transient drop, exactly
      // as the RFS subscriptions are re-established above.
      for (const sub of this.notificationSubs.values()) this.sendNotificationSubscribe(sub.scope);
      // Arm the idle keepalive so a quiescent socket keeps a trickle of bytes
      // flowing and is not reaped by a proxy idle timeout (the reconnect-flicker).
      this.startKeepalive();
      for (const l of this.stateListeners) l(true);
    };
    ws.onmessage = (ev: MessageEvent<unknown>) => {
      const data = ev.data;
      if (typeof data !== "string") return; // the mirror is text JSON only
      this.dispatch(data);
    };
    ws.onerror = () => {
      // `onclose` follows; reconnect is handled there to avoid a double schedule.
    };
    ws.onclose = () => {
      if (this.ws === ws) this.ws = null;
      this.stopKeepalive();
      for (const l of this.stateListeners) l(false);
      // Fail every outstanding request/response waiter fast so no `await` hangs
      // across a drop (SDK reconnect-liveness parity), then redial.
      this.failAllWaiters(new WsTransportError("connection closed; reconnecting"));
      this.scheduleReconnect();
    };
  }

  private scheduleReconnect(): void {
    if (this.closed || this.reconnectTimer !== undefined) return;
    const delay = this.backoff;
    this.backoff = Math.min(this.maxBackoffMs, this.backoff * 2);
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = undefined;
      this.open();
    }, delay);
  }

  /**
   * Arm the idle keepalive: every {@link KEEPALIVE_INTERVAL_MS} while the socket
   * is OPEN, send the contract's client→server `heartbeat` frame (server-side
   * liveness no-op). Sent directly on the live socket — a keepalive queued into
   * the outbox while down would be pointless (reconnect re-establishes state).
   */
  private startKeepalive(): void {
    this.stopKeepalive();
    this.keepaliveTimer = setInterval(() => {
      if (this.isOpen() && this.ws) this.ws.send('{"type":"heartbeat"}');
    }, KEEPALIVE_INTERVAL_MS);
  }

  /** Disarm the idle keepalive (on close / teardown). Idempotent. */
  private stopKeepalive(): void {
    if (this.keepaliveTimer !== undefined) {
      clearInterval(this.keepaliveTimer);
      this.keepaliveTimer = undefined;
    }
  }

  private dispatch(raw: string): void {
    let frame: WireObject;
    try {
      // Lossless parse: 64-bit identity fields (token / nanos / ids) over the safe
      // integer range are recovered as `bigint` so a tradable `token` is exact.
      const parsed: unknown = parseFrame(raw);
      if (!parsed || typeof parsed !== "object") return;
      frame = parsed as WireObject;
    } catch {
      return;
    }
    const type = typeof frame["type"] === "string" ? (frame["type"] as string) : "";
    const corr = frame["correlation_id"];
    // A reply to a request/response call: route to the waiter by correlation id.
    if (typeof corr === "number" || typeof corr === "bigint") {
      const key = BigInt(corr as number | bigint);
      const waiter = this.waiters.get(key);
      if (waiter) {
        this.waiters.delete(key);
        clearTimeout(waiter.timer);
        if (type === "error") {
          waiter.reject(new WsTransportError(String(frame["message"] ?? "server error")));
        } else {
          waiter.resolve(frame);
        }
        return;
      }
    }
    // A notification push frame (the dedicated server→client stream): route it to
    // every live notification subscriber. It carries no `correlation_id` (it is
    // not a reply), so it never matches a waiter — dispatched purely by `type`.
    if (type === "notification") {
      for (const sub of this.notificationSubs.values()) sub.onFrame(frame);
      return;
    }
    // Some contract reply messages do not carry a `correlation_id` — the `smile`,
    // `mark_surface_response`, `scenario_response` and `reject_ack` proto messages
    // have no correlation field, so the server cannot echo one. Match such a reply
    // to the oldest in-flight waiter that `expect`s this reply `type`: a single
    // connection preserves request/reply order (FIFO) per reply type. This keeps
    // the contract single — we invent no field the server must echo — and is the
    // same resolution the Excel add-in's shared connection uses.
    if (type !== "" && type !== "error") {
      for (const [key, waiter] of this.waiters) {
        if (waiter.expect === type) {
          this.waiters.delete(key);
          clearTimeout(waiter.timer);
          waiter.resolve(frame);
          return;
        }
      }
    }
    // Otherwise it is an RFS server message — hand it to the live session, which
    // routes by subscription id.
    this.session?.onServerFrame(type, frame);
  }

  /** Send a fire-and-forget control frame (queued if the socket is down). */
  send(frame: WireObject): void {
    // `serializeFrame` writes any `bigint` field (notably a tradable `token`) as a
    // bare integer literal so the server sees the exact 64-bit identity it minted.
    const text = serializeFrame(frame);
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
   * `timeoutMs` overrides the connection default for pricing-class calls (see
   * `PRICING_REQUEST_TIMEOUT_MS`).
   */
  request(
    type: string,
    body: WireObject,
    expect: string,
    timeoutMs?: number,
  ): Promise<WireObject> {
    if (this.closed) {
      return Promise.reject(new WsTransportError("transport closed"));
    }
    const correlationId = this.nextCorrelation++;
    return new Promise<WireObject>((resolve, reject) => {
      const timer = setTimeout(() => {
        if (this.waiters.delete(correlationId)) {
          reject(new WsTransportError(`request \`${type}\` timed out`));
        }
      }, timeoutMs ?? this.requestTimeoutMs);
      this.waiters.set(correlationId, { expect, resolve, reject, timer });
      // Inject the bearer session token (when authenticated) into every request
      // envelope, exactly as the correlation id is — the server reads it off the
      // gated RPCs and ignores it on the rest. Anonymous ⇒ omit the field.
      const auth = this.sessionToken ? { session_token: this.sessionToken } : {};
      this.send({ ...body, ...auth, type, correlation_id: Number(correlationId) });
    });
  }

  /**
   * Build the opening `Authenticate` control frame: the bearer token (when held)
   * plus the audited explicit grant-all entitlement principal. The token is what
   * lets the server resolve the signed-in user's CAPABILITIES (a body principal
   * cannot self-grant them — finding #3 — so under Enforce a tokenless stream is
   * refused `Stream·FxOptions` / `Execute·FxOptions`); the grant-all principal
   * keeps the risk plane admitted. Used on every (re)open AND re-sent by
   * {@link setSessionToken} when the token changes mid-session.
   */
  private authenticateFrame(): WireObject {
    return {
      type: "authenticate",
      principal: principalOrGrantAllToWire(undefined),
      ...(this.sessionToken ? { session_token: this.sessionToken } : {}),
    };
  }

  /**
   * Set (or clear, with `null`) the bearer session token injected into every
   * subsequent request envelope. Called by the auth flow after login/logout.
   *
   * The persistent connection opens (and sends its `Authenticate` frame) BEFORE
   * the login round-trip completes — the very `login` call rides this open socket
   * — so at first open the server pins an anonymous (capability-less) caller. When
   * the token then arrives we must RE-AUTHENTICATE the already-open stream so the
   * server re-pins the caller WITH the session and can resolve its capabilities;
   * otherwise the live RFS session stays anonymous and its `Subscribe`/`Execute`
   * frames are denied under Enforce. (A drop/reconnect re-sends the frame anyway
   * via `open`; this covers the steady-state, no-reconnect login.)
   */
  setSessionToken(token: string | null): void {
    this.sessionToken = token;
    if (this.isOpen() && this.ws) {
      // Re-pin the server-side caller with the new credential. Sent directly (not
      // via the outbox) so the live session's pinned identity is updated promptly,
      // before any further subscribe/execute on this connection.
      this.ws.send(JSON.stringify(this.authenticateFrame()));
      // Re-issue every live subscription under the NOW-authenticated caller. The
      // connection opens (and seeds the blotter's default watchlist) BEFORE login —
      // the stream session is bound above the login gate — so those seed
      // `Subscribe` frames went out anonymously and were DENIED under Enforce
      // (`Stream·FxOptions`; finding #3). Re-subscribing here (exactly as a
      // reconnect does) replays them now that the server can resolve the user's
      // capabilities, so the seed watchlist materialises post-login. A no-op when
      // no session is bound or no lines are live.
      this.session?.onReconnect();
    }
  }

  /**
   * Open a notification subscription: register the decoded-frame handler, send the
   * `subscribe_notifications` control frame, and return a disposer. When the LAST
   * subscriber disposes, the `unsubscribe_notifications` frame stops the server
   * push. The handler routes by `type: "notification"` in {@link dispatch}.
   */
  subscribeNotifications(
    scope: NotificationScope | undefined,
    onFrame: (frame: WireObject) => void,
  ): () => void {
    const key = Symbol("notification-sub");
    this.notificationSubs.set(key, { scope, onFrame });
    this.sendNotificationSubscribe(scope);
    return () => {
      if (this.notificationSubs.delete(key) && this.notificationSubs.size === 0) {
        this.send({ type: "unsubscribe_notifications" });
      }
    };
  }

  /** Send the `subscribe_notifications` control frame (injecting the bearer token). */
  private sendNotificationSubscribe(scope: NotificationScope | undefined): void {
    const auth = this.sessionToken ? { session_token: this.sessionToken } : {};
    this.send({ type: "subscribe_notifications", ...subscribeNotificationsToWire(scope), ...auth });
  }

  private failAllWaiters(err: Error): void {
    for (const [, w] of this.waiters) {
      clearTimeout(w.timer);
      w.reject(err);
    }
    this.waiters.clear();
  }

  /** Tear down the connection permanently (no further reconnects). */
  close(): void {
    this.closed = true;
    this.stopKeepalive();
    if (this.reconnectTimer !== undefined) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = undefined;
    }
    this.failAllWaiters(new WsTransportError("transport closed"));
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
        // ignore — already closing
      }
    }
  }
}

/** Per-subscription state the session tracks for gap-detect and reconnect. */
interface WsSub {
  readonly id: bigint;
  readonly instrument: Instrument;
  readonly conventions: Conventions;
  readonly label: string;
  /** The last in-sequence number successfully applied (0 until first snapshot). */
  lastSequence: bigint;
  /** True once the baseline snapshot has been seen (post-subscribe/post-resync). */
  baselined: boolean;
  /** The health last emitted, so we only emit on a transition. */
  health: "HEALTHY" | "RESYNCING" | "STALE";
}

/**
 * A live fixed-income (linear-rates) streaming line — the FI analogue of
 * {@link WsSub}. It carries the originating instrument + baseline curve so a
 * reconnect re-opens the exact line, and its last good sequence to detect gaps.
 */
interface WsRatesSub {
  readonly id: bigint;
  readonly instrument: RatesInstrument;
  readonly curveSet: RatesCurveSet;
  readonly label: string;
  /** The last in-sequence number successfully applied (0 until first snapshot). */
  lastSequence: bigint;
  /** True once the baseline snapshot has been seen (post-subscribe/post-resync). */
  baselined: boolean;
}

/**
 * A live aggregated-book composite line — the consolidated best bid/offer + per-
 * member contribution report for one book. Carries the `bookId` so a reconnect
 * re-opens the exact line; its last good sequence detects gaps (a snapshot
 * re-baselines whole).
 */
interface WsAggSub {
  readonly id: bigint;
  readonly bookId: string;
  readonly throttleNanos: bigint;
  /** The last in-sequence number successfully applied (0 until first snapshot). */
  lastSequence: bigint;
  /** True once the baseline snapshot has been seen. */
  baselined: boolean;
}

/**
 * A live risk-book-risk push line — the per-book rolled-up risk set, delivered as
 * a baseline snapshot then version-gated updates. It carries no per-line key (the
 * server pushes the whole firm's enabled books); its last good sequence detects
 * gaps (a snapshot re-baselines whole) and `lastVersion` drops stale frames.
 */
interface WsRiskSub {
  readonly id: bigint;
  /** The last in-sequence number successfully applied (0 until first snapshot). */
  lastSequence: bigint;
  /** True once the baseline snapshot has been seen. */
  baselined: boolean;
}

/**
 * The live multiplexed RFS session over the WS connection. Implements the same
 * `StreamSession` seam the mock does, so workspaces and the streaming store are
 * transport-agnostic. It assigns each subscription a client `SubscriptionId`
 * (the wire keys routing on it), tracks per-subscription sequence to detect gaps
 * (sending a server-assisted `resync` exactly as the contract prescribes), and on
 * a reconnect re-subscribes every live line and resyncs from its last good
 * sequence — recovering the blotter transparently.
 */
class WsStreamSession implements StreamSession {
  private readonly subs = new Map<bigint, WsSub>();
  /**
   * Live market-series subscriptions, keyed by the SAME id space as price streams
   * (the contract multiplexes both on one `SubscriptionId` space). Held so a
   * reconnect re-opens each series and the snapshot/point frames route by id.
   */
  private readonly series = new Map<bigint, MarketSeriesParams>();
  /**
   * Live fixed-income streaming lines, keyed by the SAME id space as the FX price
   * + market-series streams (the contract multiplexes all three on one
   * `SubscriptionId` space). Held so a reconnect re-opens each line and the
   * rates snapshot/update frames route by id.
   */
  private readonly ratesSubs = new Map<bigint, WsRatesSub>();
  /**
   * Live aggregated-book composite lines, keyed by the SAME id space as the FX /
   * market-series / rates streams (the contract multiplexes all of them on one
   * `SubscriptionId` space). Held so a reconnect re-opens each line and the
   * composite snapshot/update frames route by id.
   */
  private readonly aggBookSubs = new Map<bigint, WsAggSub>();
  /**
   * Live risk-book-risk push lines, keyed by the SAME `SubscriptionId` space. Held
   * so a reconnect re-opens each line and the risk snapshot/update frames route by id.
   */
  private readonly riskSubs = new Map<bigint, WsRiskSub>();
  private readonly listeners = new Set<(e: StreamEvent) => void>();
  private nextSubId = 1n;
  private closed = false;

  constructor(private readonly conn: WsConnection) {
    this.conn.bindSession(this);
  }

  private emit(event: StreamEvent): void {
    for (const l of this.listeners) l(event);
  }

  onEvent(listener: (event: StreamEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  subscribe(instrument: Instrument, conventions: Conventions, label: string): bigint {
    const id = this.nextSubId++;
    this.subs.set(id, {
      id,
      instrument,
      conventions,
      label,
      lastSequence: 0n,
      baselined: false,
      health: "RESYNCING",
    });
    this.sendSubscribe(id, instrument, conventions);
    return id;
  }

  /** Emit a health transition for a subscription (only on an actual change). */
  private setHealth(sub: WsSub, health: "HEALTHY" | "RESYNCING" | "STALE"): void {
    if (sub.health === health) return;
    sub.health = health;
    this.emit({ kind: "health", subscriptionId: sub.id, health });
  }

  private sendSubscribe(id: bigint, instrument: Instrument, conventions: Conventions): void {
    this.conn.send({
      type: "subscribe",
      subscription: { value: Number(id) },
      instrument: instrumentToWire(instrument),
      conventions: conventionsToWire(conventions),
      throttle_nanos: 0,
    });
  }

  unsubscribe(subscriptionId: bigint): void {
    if (!this.subs.delete(subscriptionId)) return;
    this.conn.send({
      type: "unsubscribe",
      subscription: { value: Number(subscriptionId) },
    });
  }

  execute(subscriptionId: bigint, token: bigint, idempotencyKey: string): void {
    this.conn.send({
      type: "execute",
      subscription: { value: Number(subscriptionId) },
      // The token is the maker's exact 64-bit identity; pass it as a `bigint` so
      // `serializeFrame` writes the full-precision integer literal the server
      // minted. (A lossy `Number(token)` would be rejected `UNKNOWN_TOKEN`.)
      token,
      idempotency_key: idempotencyKey,
    });
  }

  subscribeMarketSeries(params: MarketSeriesParams): bigint {
    const id = this.nextSubId++;
    this.series.set(id, params);
    this.sendMarketSeriesSubscribe(id, params);
    return id;
  }

  private sendMarketSeriesSubscribe(id: bigint, params: MarketSeriesParams): void {
    this.conn.send({
      type: "market_series_subscribe",
      ...marketSeriesSubscribeToWire({
        subscriptionId: id,
        pair: params.pair,
        observable: params.observable,
        ...(params.tenor ? { tenor: params.tenor } : {}),
        ...(params.delta !== undefined ? { delta: params.delta } : {}),
        throttleNanos: params.throttleNanos ?? 0n,
        historyLimit: params.historyLimit ?? 0,
      }),
    });
  }

  unsubscribeMarketSeries(subscriptionId: bigint): void {
    if (!this.series.delete(subscriptionId)) return;
    this.conn.send({
      type: "market_series_unsubscribe",
      ...marketSeriesUnsubscribeToWire(subscriptionId),
    });
  }

  subscribeRates(instrument: RatesInstrument, curveSet: RatesCurveSet, label: string): bigint {
    const id = this.nextSubId++;
    this.ratesSubs.set(id, {
      id,
      instrument,
      curveSet,
      label,
      lastSequence: 0n,
      baselined: false,
    });
    this.sendRatesSubscribe(id, instrument, curveSet);
    return id;
  }

  private sendRatesSubscribe(
    id: bigint,
    instrument: RatesInstrument,
    curveSet: RatesCurveSet,
  ): void {
    this.conn.send({
      type: "rates_subscribe",
      ...ratesSubscribeToWire({ subscriptionId: id, instrument, curveSet }),
    });
  }

  unsubscribeRates(subscriptionId: bigint): void {
    if (!this.ratesSubs.delete(subscriptionId)) return;
    // A rates line tears down through the SAME generic `unsubscribe` control
    // frame the FX lines use — the server's handler removes it from either the
    // FX or the fixed-income subscription map (crates/celnet-server/src/services
    // /stream.rs), so the id space is uniform and one verb suffices.
    this.conn.send({
      type: "unsubscribe",
      subscription: { value: Number(subscriptionId) },
    });
  }

  subscribeAggregatedBook(bookId: string, throttleNanos = 0n): bigint {
    const id = this.nextSubId++;
    this.aggBookSubs.set(id, {
      id,
      bookId,
      throttleNanos,
      lastSequence: 0n,
      baselined: false,
    });
    this.sendAggregatedBookSubscribe(id, bookId, throttleNanos);
    return id;
  }

  private sendAggregatedBookSubscribe(id: bigint, bookId: string, throttleNanos: bigint): void {
    this.conn.send({
      type: "aggregated_book_subscribe",
      ...aggregatedBookSubscribeToWire({ subscriptionId: id, bookId, throttleNanos }),
    });
  }

  unsubscribeAggregatedBook(subscriptionId: bigint): void {
    if (!this.aggBookSubs.delete(subscriptionId)) return;
    // The composite line has its OWN unsubscribe verb (unlike the rates line,
    // which shares the generic `unsubscribe`): the server routes it to the
    // aggregated-book subscription map (crates/celnet-server/src/ws/mod.rs).
    this.conn.send({
      type: "aggregated_book_unsubscribe",
      subscription: { value: Number(subscriptionId) },
    });
  }

  subscribeRiskBookRisk(): bigint {
    const id = this.nextSubId++;
    this.riskSubs.set(id, { id, lastSequence: 0n, baselined: false });
    this.sendRiskBookRiskSubscribe(id);
    return id;
  }

  private sendRiskBookRiskSubscribe(id: bigint): void {
    this.conn.send({
      type: "risk_book_risk_subscribe",
      ...riskBookRiskSubscribeToWire({ subscriptionId: id }),
    });
  }

  unsubscribeRiskBookRisk(subscriptionId: bigint): void {
    if (!this.riskSubs.delete(subscriptionId)) return;
    // The risk push line tears down through the generic `unsubscribe` verb (like
    // the rates line): the server removes it from the risk subscription map
    // (crates/celnet-server/src/ws/mod.rs), so one verb suffices.
    this.conn.send({
      type: "unsubscribe",
      subscription: { value: Number(subscriptionId) },
    });
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    for (const id of this.subs.keys()) {
      this.conn.send({ type: "unsubscribe", subscription: { value: Number(id) } });
    }
    for (const id of this.series.keys()) {
      this.conn.send({
        type: "market_series_unsubscribe",
        ...marketSeriesUnsubscribeToWire(id),
      });
    }
    for (const id of this.ratesSubs.keys()) {
      this.conn.send({ type: "unsubscribe", subscription: { value: Number(id) } });
    }
    for (const id of this.aggBookSubs.keys()) {
      this.conn.send({
        type: "aggregated_book_unsubscribe",
        subscription: { value: Number(id) },
      });
    }
    for (const id of this.riskSubs.keys()) {
      this.conn.send({ type: "unsubscribe", subscription: { value: Number(id) } });
    }
    this.subs.clear();
    this.series.clear();
    this.ratesSubs.clear();
    this.aggBookSubs.clear();
    this.riskSubs.clear();
    this.listeners.clear();
    this.conn.bindSession(null);
  }

  /**
   * On a fresh connection (initial open or post-drop), re-issue every live
   * subscription, then resync each from its last good sequence so the server
   * replays anything missed (or re-baselines with a fresh snapshot).
   */
  onReconnect(): void {
    if (this.closed) return;
    for (const sub of this.subs.values()) {
      sub.baselined = false;
      this.sendSubscribe(sub.id, sub.instrument, sub.conventions);
      if (sub.lastSequence > 0n) {
        this.conn.send({
          type: "resync",
          subscription: { value: Number(sub.id) },
          last_sequence: sub.lastSequence,
        });
      }
      this.setHealth(sub, "RESYNCING");
    }
    // Re-open every live market series — the server re-baselines each with a fresh
    // snapshot (the series has no client-side sequence resync; it is conflatable).
    for (const [id, params] of this.series) {
      this.sendMarketSeriesSubscribe(id, params);
    }
    // Re-open every live fixed-income line — the server re-baselines each with a
    // fresh `rates_stream_snapshot` at shift 0 (the FI line is conflatable and
    // re-prices from the baseline curve, so no client-side sequence resync).
    for (const sub of this.ratesSubs.values()) {
      sub.baselined = false;
      this.sendRatesSubscribe(sub.id, sub.instrument, sub.curveSet);
    }
    // Re-open every live aggregated-book composite line — the server re-baselines
    // each with a fresh `aggregated_book_stream_snapshot` (the composite is
    // conflatable and re-consolidates from the members' live quotes, so no
    // client-side sequence resync).
    for (const sub of this.aggBookSubs.values()) {
      sub.baselined = false;
      this.sendAggregatedBookSubscribe(sub.id, sub.bookId, sub.throttleNanos);
    }
    // Re-open every live risk push line — the server re-baselines each with a fresh
    // `risk_book_risk_snapshot` (the line is conflatable and re-rolls the whole book
    // set from live positions, so no client-side sequence resync).
    for (const sub of this.riskSubs.values()) {
      sub.baselined = false;
      this.sendRiskBookRiskSubscribe(sub.id);
    }
  }

  /** Route an inbound RFS server frame (already typed by the connection). */
  onServerFrame(type: string, frame: WireObject): void {
    switch (type) {
      case "snapshot": {
        const snapshot = snapshotFromWire(frame);
        const sub = this.subs.get(snapshot.subscriptionId);
        if (!sub) return;
        // A snapshot is the authoritative baseline: accept its sequence whole.
        sub.lastSequence = snapshot.sequence;
        sub.baselined = true;
        this.emit({ kind: "snapshot", snapshot });
        this.setHealth(sub, "HEALTHY");
        break;
      }
      case "update": {
        const update = updateFromWire(frame);
        const sub = this.subs.get(update.subscriptionId);
        if (!sub || !sub.baselined) return;
        const expected = sub.lastSequence + 1n;
        if (update.sequence > expected) {
          // Detected a gap: ask the server to resync from our last good sequence
          // and mark the row resyncing until a fresh baseline lands. We still
          // apply this update so the price stays live, then reconcile on snapshot.
          this.setHealth(sub, "RESYNCING");
          this.conn.send({
            type: "resync",
            subscription: { value: Number(sub.id) },
            last_sequence: sub.lastSequence,
          });
        } else {
          // An in-sequence update confirms the line is live — clear any prior
          // RESYNCING/STALE so the row's health badge is honest.
          this.setHealth(sub, "HEALTHY");
        }
        if (update.sequence > sub.lastSequence) sub.lastSequence = update.sequence;
        this.emit({ kind: "update", update });
        break;
      }
      case "heartbeat": {
        // A heartbeat carries (1) the current sequence so a silent gap is
        // detectable AND (2) the server's additive observability (ring conflation
        // drops + drain-side price p50/p99/p99.9 + provenance echo). Surface the
        // decoded beat to the store (StatusRibbon reads it), THEN do liveness:
        // a heartbeat ahead of our last applied sequence signals a missed update,
        // so resync to recover (matching the SDK / add-in connection).
        const heartbeat = heartbeatFromWire(frame);
        this.emit({ kind: "heartbeat", heartbeat });
        const sub = this.subs.get(subscriptionIdOf(frame) ?? -1n);
        if (!sub || !sub.baselined) break;
        const seq = bigField(frame, "sequence");
        if (seq > sub.lastSequence) {
          this.setHealth(sub, "RESYNCING");
          this.conn.send({
            type: "resync",
            subscription: { value: Number(sub.id) },
            last_sequence: sub.lastSequence,
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
        // The server ended this subscription (LAGGED / DRAINING / …). Mark it
        // stale; a reconnect (or the next snapshot) re-baselines it.
        const id = subscriptionIdOf(frame);
        if (id !== undefined) {
          const sub = this.subs.get(id);
          if (sub) {
            sub.baselined = false;
            this.setHealth(sub, "STALE");
          }
        }
        break;
      }
      case "market_series_snapshot": {
        const snapshot = marketSeriesSnapshotFromWire(frame);
        if (!this.series.has(snapshot.subscriptionId)) break;
        this.emit({ kind: "marketSeriesSnapshot", snapshot });
        break;
      }
      case "market_series_point": {
        const point = marketSeriesPointFromWire(frame);
        if (!this.series.has(point.subscriptionId)) break;
        this.emit({ kind: "marketSeriesPoint", point });
        break;
      }
      case "rates_stream_snapshot": {
        const snapshot = ratesStreamSnapshotFromWire(frame);
        const sub = this.ratesSubs.get(snapshot.subscriptionId);
        if (!sub) break;
        // The baseline is authoritative: accept its sequence whole.
        sub.lastSequence = snapshot.sequence;
        sub.baselined = true;
        this.emit({ kind: "ratesSnapshot", snapshot });
        break;
      }
      case "rates_stream_update": {
        const update = ratesStreamUpdateFromWire(frame);
        const sub = this.ratesSubs.get(update.subscriptionId);
        // A rates line is conflatable and re-prices from the baseline curve each
        // tick, so a gap needs no client resync — just apply the latest.
        if (!sub || !sub.baselined) break;
        if (update.sequence > sub.lastSequence) sub.lastSequence = update.sequence;
        this.emit({ kind: "ratesUpdate", update });
        break;
      }
      case "aggregated_book_stream_snapshot": {
        const snapshot = aggregatedBookSnapshotFromWire(frame);
        const sub = this.aggBookSubs.get(snapshot.subscriptionId);
        if (!sub) break;
        // The baseline is authoritative: accept its sequence whole.
        sub.lastSequence = snapshot.sequence;
        sub.baselined = true;
        this.emit({ kind: "aggregatedBookSnapshot", snapshot });
        break;
      }
      case "aggregated_book_stream_update": {
        const update = aggregatedBookUpdateFromWire(frame);
        const sub = this.aggBookSubs.get(update.subscriptionId);
        // A composite line is conflatable and re-consolidates from the members'
        // live quotes each tick, so a gap needs no client resync — apply the latest.
        if (!sub || !sub.baselined) break;
        if (update.sequence > sub.lastSequence) sub.lastSequence = update.sequence;
        this.emit({ kind: "aggregatedBookUpdate", update });
        break;
      }
      case "risk_book_risk_snapshot": {
        const snapshot = riskBookRiskStreamSnapshotFromWire(frame);
        const sub = this.riskSubs.get(snapshot.subscriptionId);
        if (!sub) break;
        // The baseline is authoritative: accept its sequence whole.
        sub.lastSequence = snapshot.sequence;
        sub.baselined = true;
        this.emit({ kind: "riskBookRiskSnapshot", snapshot });
        break;
      }
      case "risk_book_risk_update": {
        const update = riskBookRiskStreamUpdateFromWire(frame);
        const sub = this.riskSubs.get(update.subscriptionId);
        // A risk push line is conflatable and re-rolls the whole book set each
        // change, so a gap needs no client resync — apply the latest (the consumer
        // additionally drops any frame whose `version` regressed).
        if (!sub || !sub.baselined) break;
        if (update.sequence > sub.lastSequence) sub.lastSequence = update.sequence;
        this.emit({ kind: "riskBookRiskUpdate", update });
        break;
      }
      default:
        break;
    }
  }
}

/**
 * The live WebSocket transport. Construct with the mirror endpoint and inject at
 * the app root in place of the mock. All request/response calls go over the same
 * connection that carries the RFS session; a single connection is opened lazily
 * on the first stream session and reused for everything.
 */
export class WsTransport implements CelnetTransport {
  readonly label: string;
  private readonly conn: WsConnection;
  /**
   * Quote → originating instrument, recorded when we serve a quote so an
   * `acceptQuote` can return a complete `Execution` (the wire `Execution` omits
   * the instrument — the booking instrument is the one the client quoted, exactly
   * as the SDK/mock pair it). Bounded by eviction on accept/reject.
   */
  private readonly quoteInstruments = new Map<bigint, Instrument>();

  constructor(opts: WsTransportOptions) {
    this.conn = new WsConnection(opts);
    // The ribbon reads "live ws://host:port" so the operator sees the real edge.
    this.label = `live ${normalizeWsUrl(opts.url)}`;
  }

  /** Subscribe to connection-open state (for diagnostics / a future indicator). */
  onConnectionState(listener: (open: boolean) => void): () => void {
    return this.conn.onState(listener);
  }

  /** Snapshot of socket liveness (`true` iff the live socket is currently OPEN). */
  isConnected(): boolean {
    return this.conn.isOpen();
  }

  /**
   * Install (or clear, with `null`) the bearer session token the transport
   * injects into every gated request. The auth flow calls this after a login
   * succeeds and again on logout; it survives reconnects (held on the persistent
   * connection, not the socket).
   */
  setSessionToken(token: string | null): void {
    this.conn.setSessionToken(token);
  }

  async price(
    instrument: Instrument,
    market: MarketContext,
    conventions: Conventions,
  ): Promise<PriceResult> {
    const reply = await this.conn.request(
      "price",
      {
        instrument: instrumentToWire(instrument),
        market: marketToWire(market),
        conventions: conventionsToWire(conventions),
      },
      "price_response",
      PRICING_REQUEST_TIMEOUT_MS,
    );
    const greeks = greeksFromWire(asChild(reply, "greeks"));
    const resolvedStrike = numField(reply, "resolved_strike");
    const surfaceVersion = bigField(reply, "surface_version");
    // The mirror's `PriceResponse` carries Greeks + resolved strike + the echoed
    // conventions (no two-way — `Price` is the mid valuation; the two-way comes
    // from `RequestQuote`). Surface the mid as a degenerate two-way so the ticket
    // shows the priced premium; the trade-able market is fetched via requestQuote.
    return {
      greeks,
      resolvedStrike,
      conventions,
      twoWay: { bid: greeks.price, offer: greeks.price },
      surfaceVersion,
    };
  }

  async priceRates(
    curve: RatesCurveSet,
    instrument: RatesInstrument,
  ): Promise<RatesPricingResult> {
    // The `price_rates` RPC over the WS mirror: send the curve set + the
    // `RatesInstrument` oneof arm (OIS / IRS / FRA / bond) and decode the
    // server-bootstrapped PV + risk. One unversioned contract, so the
    // server-priced result is byte-identical to the offline in-app pricer (which
    // reproduces the same `celnet-rates` / `celnet-bond` math).
    const reply = await this.conn.request(
      "price_rates",
      {
        curve_set: ratesCurveSetToWire(curve),
        instrument: ratesInstrumentUnionToWire(instrument),
      },
      "rates_price_response",
      PRICING_REQUEST_TIMEOUT_MS,
    );
    return ratesPricingResultFromWire(reply);
  }

  async priceXva(request: XvaPricingRequest): Promise<XvaResult> {
    // The `price_xva` RPC over the WS mirror: send the netting set + exposure
    // model + survival curves and decode the four scalar adjustments the server
    // returns. One unversioned contract, so the WS-priced result is byte-identical
    // to the gRPC edge. The exposure profile is a server-internal — the reply
    // carries only { cva, dva, fva, total_adjustment }.
    const reply = await this.conn.request(
      "price_xva",
      priceXvaRequestToWire(request),
      "price_xva_response",
      PRICING_REQUEST_TIMEOUT_MS,
    );
    return xvaResultFromWire(reply);
  }

  async requestQuote(
    instrument: Instrument,
    conventions: Conventions,
    idempotencyKey: string,
  ): Promise<Quote> {
    const reply = await this.conn.request(
      "request_quote",
      {
        idempotency_key: idempotencyKey,
        instrument: instrumentToWire(instrument),
        conventions: conventionsToWire(conventions),
        // Caller-authz (item B §2): the server gates QuoteService under Enforce
        // and binds this requester to a later acceptQuote. The bearer
        // `session_token` is auto-injected by `WsConnection.request` (the same one
        // the stream `authenticate` frame uses); the entitlement `principal`
        // defaults to the audited grant-all so the headline RFQ workflow is
        // admitted under Enforce, exactly as the stream auth frame and the gated
        // risk requests default.
        principal: principalOrGrantAllToWire(undefined),
      },
      "quote",
      PRICING_REQUEST_TIMEOUT_MS,
    );
    const quote = quoteFromWire(reply);
    // Remember the instrument this quote priced so a subsequent acceptQuote can
    // return a complete Execution (the wire Execution omits the instrument).
    this.quoteInstruments.set(quote.quoteId, instrument);
    return quote;
  }

  async requestMultiDealerQuote(
    instrument: Instrument,
    conventions: Conventions,
    idempotencyKey: string,
  ): Promise<MultiDealerQuote> {
    // The SAME QuoteRequest body as `request_quote` — only the frame type selects
    // the multi-dealer fan-out; the reply is the ranked panel frame.
    const reply = await this.conn.request(
      "request_multi_dealer_quote",
      {
        idempotency_key: idempotencyKey,
        instrument: instrumentToWire(instrument),
        conventions: conventionsToWire(conventions),
        // Same caller-authz envelope as `request_quote` (item B §2): grant-all
        // default principal + the auto-injected bearer token. The recording
        // caller is bound to a later acceptQuote on any winning panel row.
        principal: principalOrGrantAllToWire(undefined),
      },
      "multi_dealer_quote",
      PRICING_REQUEST_TIMEOUT_MS,
    );
    const panel = multiDealerQuoteFromWire(reply);
    // Remember the instrument this panel priced so a subsequent acceptQuote (with
    // any row's lpId) can return a complete Execution, exactly as requestQuote.
    this.quoteInstruments.set(panel.quoteId, instrument);
    return panel;
  }

  async requestRatesQuote(
    curve: RatesCurveSet,
    instrument: RatesInstrument,
    notional: number,
    side: Side,
    idempotencyKey: string,
  ): Promise<RatesQuote> {
    // The fixed-income taker RFQ (`QuoteService.RequestRatesQuote`) over the WS
    // mirror: send the curve set + the `RatesInstrument` oneof arm + the RFQ
    // envelope (notional + taker side) and decode the two-way struck around the
    // side-independent fair level plus the full FI risk. A pure price-discovery
    // calculation (the curve set IS the market), so — like `priceRates` — it carries
    // no caller-authz envelope on the wire (the `RatesQuoteRequest` proto has none;
    // the stateful maker/booking flow is `RfqDeskService`). One unversioned
    // contract, so the server-priced two-way is byte-identical to the offline path.
    const reply = await this.conn.request(
      "request_rates_quote",
      ratesQuoteRequestToWire(idempotencyKey, curve, instrument, notional, side),
      "rates_quote",
      PRICING_REQUEST_TIMEOUT_MS,
    );
    return ratesQuoteFromWire(reply);
  }

  async acceptQuote(
    quoteId: bigint,
    side: "BUY" | "SELL",
    idempotencyKey: string,
    lpId?: string,
  ): Promise<Execution> {
    const reply = await this.conn.request(
      "accept_quote",
      // `lp_id` is emitted only when a panel row is named; the single-dealer
      // accept stays byte-identical to the pre-panel frame (quoteAcceptToWire).
      quoteAcceptToWire(quoteId, side, idempotencyKey, lpId),
      "execution",
    );
    const partial = executionFromWire(reply);
    // The wire Execution does not echo the instrument; pair it with the one this
    // client quoted (recorded on requestQuote), exactly as the SDK/mock does.
    const instrument = this.quoteInstruments.get(quoteId);
    if (!instrument) {
      throw new WsTransportError(`no quoted instrument for quote ${quoteId}`);
    }
    this.quoteInstruments.delete(quoteId);
    return { ...partial, instrument };
  }

  async rejectQuote(quoteId: bigint, reason: string): Promise<void> {
    this.quoteInstruments.delete(quoteId);
    await this.conn.request(
      "reject_quote",
      // The exact 64-bit minted quote id, as a `bigint` so `serializeFrame`
      // writes the full-precision literal (a lossy `Number(quoteId)` rounds ids
      // beyond MAX_SAFE and the server refuses them as unknown). Caller-authz
      // (item B §2): grant-all default principal + the auto-injected bearer
      // token, so reject_quote is admitted under Enforce like the other RPCs.
      { quote_id: quoteId, reason, principal: principalOrGrantAllToWire(undefined) },
      "reject_ack",
    );
  }

  openStreamSession(): StreamSession {
    return new WsStreamSession(this.conn);
  }

  async getSmile(pair: CcyPair, tenorYears: number, conventions: Conventions): Promise<Smile> {
    const reply = await this.conn.request(
      "get_smile",
      {
        pair: ccyPairToWire(pair),
        tenor_years: tenorYears,
        conventions: conventionsToWire(conventions),
      },
      "smile",
    );
    return smileFromWire(reply);
  }

  async markSurface(
    pair: CcyPair,
    brokerQuotes: BrokerQuoteSet[],
    conventions: Conventions,
    smileModel?: SmileModel,
  ): Promise<MarkedSurface> {
    const body: Record<string, unknown> = {
      pair: ccyPairToWire(pair),
      broker_quotes: brokerQuotes.map(brokerQuoteSetToWire),
      conventions: conventionsToWire(conventions),
    };
    // Presence-tracked: omit ⇒ the server's default calibration (MARKET_HEDGE).
    if (smileModel) body["smile_model"] = smileModelToWire(smileModel);
    const reply = await this.conn.request("mark_surface", body, "mark_surface_response");
    return markedSurfaceFromWire(reply);
  }

  async scenario(
    instrument: Instrument,
    baseMarket: MarketContext,
    conventions: Conventions,
    axes: ShockAxis[],
    riskBuckets?: RiskBucketRequest,
  ): Promise<ScenarioResult> {
    const body: Record<string, unknown> = {
      instrument: instrumentToWire(instrument),
      base_market: marketToWire(baseMarket),
      conventions: conventionsToWire(conventions),
      axes: axes.map(shockAxisToWire),
      expiry_years: instrument.expiryYears,
    };
    // The book-shaped risk decomposition is computed by the server ONLY when the
    // request carries a `risk_buckets` block — otherwise `bucketed_risk` is null.
    if (riskBuckets) body["risk_buckets"] = riskBucketRequestToWire(riskBuckets);
    // Scenario grids revalue through the same pricing engines — pricing-class deadline.
    const reply = await this.conn.request("scenario", body, "scenario_response", PRICING_REQUEST_TIMEOUT_MS);
    return scenarioResultFromWire(reply);
  }

  async listPositions(request: ListPositionsRequest): Promise<ListPositionsResponse> {
    const reply = await this.conn.request(
      "list_positions",
      listPositionsRequestToWire(request),
      "list_positions_response",
    );
    return listPositionsResponseFromWire(reply);
  }

  async aggregateRisk(request: AggregateRiskRequest): Promise<AggregateRiskResponse> {
    const reply = await this.conn.request(
      "aggregate_risk",
      aggregateRiskRequestToWire(request),
      "aggregate_risk_response",
    );
    return aggregateRiskResponseFromWire(reply);
  }

  async aggregateRatesRisk(
    request: AggregateRatesRiskRequest,
    _conventions: Conventions,
  ): Promise<AggregateRatesRiskResponse> {
    const reply = await this.conn.request(
      "aggregate_rates_risk",
      aggregateRatesRiskRequestToWire(request),
      "aggregate_rates_risk_response",
    );
    return aggregateRatesRiskResponseFromWire(reply);
  }

  async combinedTailRisk(
    request: CombinedTailRiskRequest,
  ): Promise<CombinedTailRiskResponse> {
    const reply = await this.conn.request(
      "combined_tail_risk",
      combinedTailRiskRequestToWire(request),
      "combined_tail_risk_response",
    );
    return combinedTailRiskResponseFromWire(reply);
  }

  async drillRisk(request: DrillRiskRequest): Promise<DrillRiskResponse> {
    const reply = await this.conn.request(
      "drill_risk",
      drillRiskRequestToWire(request),
      "drill_risk_response",
    );
    return drillRiskResponseFromWire(reply);
  }

  async limitStatus(request: LimitStatusRequest): Promise<LimitStatusResponse> {
    const reply = await this.conn.request(
      "limit_status",
      limitStatusRequestToWire(request),
      "limit_status_response",
    );
    return limitStatusResponseFromWire(reply);
  }

  // --- RfqDeskService — dealer-quoting RFQ/IOI desk --------------------------

  async submitDeskRequest(
    request: SubmitDeskRequestRequest,
  ): Promise<SubmitDeskRequestResponse> {
    const reply = await this.conn.request(
      "submit_desk_request",
      submitDeskRequestToWire(request),
      "submit_desk_request_response",
    );
    return submitDeskRequestResponseFromWire(reply);
  }

  async respondDeskRequest(
    request: RespondDeskRequestRequest,
  ): Promise<RespondDeskRequestResponse> {
    const reply = await this.conn.request(
      "respond_desk_request",
      respondDeskRequestToWire(request),
      "respond_desk_request_response",
    );
    return respondDeskRequestResponseFromWire(reply);
  }

  async acceptDeskQuote(request: AcceptDeskQuoteRequest): Promise<AcceptDeskQuoteResponse> {
    const reply = await this.conn.request(
      "accept_desk_quote",
      acceptDeskQuoteToWire(request),
      "accept_desk_quote_response",
    );
    return acceptDeskQuoteResponseFromWire(reply);
  }

  async listDeskRequests(
    request: ListDeskRequestsRequest,
  ): Promise<ListDeskRequestsResponse> {
    const reply = await this.conn.request(
      "list_desk_requests",
      listDeskRequestsToWire(request),
      "list_desk_requests_response",
    );
    return listDeskRequestsResponseFromWire(reply);
  }

  async listDeals(request: ListDealsRequest): Promise<ListDealsResponse> {
    const reply = await this.conn.request(
      "list_deals",
      listDealsToWire(request),
      "list_deals_response",
    );
    return listDealsResponseFromWire(reply);
  }

  // --- RiskService rates Book ------------------------------------------------

  async bookRatesPosition(
    request: BookRatesPositionRequest,
  ): Promise<BookRatesPositionResponse> {
    const reply = await this.conn.request(
      "book_rates_position",
      bookRatesPositionToWire(request),
      "book_rates_position_response",
    );
    return bookRatesPositionResponseFromWire(reply);
  }

  async listRatesPositions(
    request: ListRatesPositionsRequest,
  ): Promise<ListRatesPositionsResponse> {
    const reply = await this.conn.request(
      "list_rates_positions",
      listRatesPositionsToWire(request),
      "list_rates_positions_response",
    );
    return listRatesPositionsResponseFromWire(reply);
  }

  // --- NotificationService — dedicated server→client push stream -------------

  streamNotifications(
    scope: NotificationScope | undefined,
    onNotification: (notification: Notification) => void,
  ): () => void {
    return this.conn.subscribeNotifications(scope, (frame) =>
      onNotification(notificationFromWire(frame)),
    );
  }

  // --- FixAdminService — manage the inbound FIX acceptor connections ---------

  async listFixConnections(): Promise<FixConnection[]> {
    const reply = await this.conn.request(
      "list_fix_connections",
      listFixConnectionsRequestToWire(),
      "fix_connections",
    );
    return listFixConnectionsResponseFromWire(reply);
  }

  async createFixConnection(spec: FixConnectionSpec): Promise<FixConnection> {
    const reply = await this.conn.request(
      "create_fix_connection",
      createFixConnectionRequestToWire(spec),
      "fix_connection_created",
    );
    return fixConnectionResponseFromWire(reply);
  }

  async updateFixConnection(id: string, spec: FixConnectionSpec): Promise<FixConnection> {
    const reply = await this.conn.request(
      "update_fix_connection",
      updateFixConnectionRequestToWire(id, spec),
      "fix_connection_updated",
    );
    return fixConnectionResponseFromWire(reply);
  }

  async deleteFixConnection(id: string): Promise<void> {
    await this.conn.request(
      "delete_fix_connection",
      deleteFixConnectionRequestToWire(id),
      "fix_connection_deleted",
    );
  }

  async setFixConnectionEnabled(id: string, enabled: boolean): Promise<FixConnection> {
    const reply = await this.conn.request(
      "set_fix_connection_enabled",
      setFixConnectionEnabledRequestToWire(id, enabled),
      "fix_connection_enabled",
    );
    return fixConnectionResponseFromWire(reply);
  }

  async listFixMessages(
    connectionId: string | undefined,
    afterSeq: bigint,
    limit = 0,
  ): Promise<FixMessagePage> {
    const reply = await this.conn.request(
      "list_fix_messages",
      listFixMessagesRequestToWire(connectionId, afterSeq, limit),
      "fix_messages",
    );
    return listFixMessagesResponseFromWire(reply);
  }

  // --- AuthService — sessions + user/desk administration ----------------------
  //
  // The bearer token rides on every gated request automatically (the connection
  // injects it once `setSessionToken` is called); `login` is the one anonymous
  // call. The admin RPCs below resolve the server-side identity from that token.

  async login(email: string, password: string): Promise<LoginResult> {
    const reply = await this.conn.request(
      "login",
      loginRequestToWire(email, password),
      "login_result",
    );
    return loginResultFromWire(reply);
  }

  async logout(): Promise<boolean> {
    const reply = await this.conn.request("logout", logoutRequestToWire(), "logout_result");
    return reply["ended"] === true;
  }

  async listUsers(): Promise<UserDesc[]> {
    const reply = await this.conn.request("list_users", listUsersRequestToWire(), "users");
    return listUsersResponseFromWire(reply);
  }

  async createUser(input: CreateUserInput): Promise<UserDesc> {
    const reply = await this.conn.request(
      "create_user",
      createUserRequestToWire(input),
      "user_created",
    );
    return userResponseFromWire(reply);
  }

  async updateUser(id: string, input: UpdateUserInput): Promise<UserDesc> {
    const reply = await this.conn.request(
      "update_user",
      updateUserRequestToWire(id, input),
      "user_updated",
    );
    return userResponseFromWire(reply);
  }

  async deleteUser(id: string): Promise<boolean> {
    const reply = await this.conn.request(
      "delete_user",
      deleteUserRequestToWire(id),
      "user_deleted",
    );
    return reply["removed"] === true;
  }

  async resetPassword(id: string, newPassword: string): Promise<void> {
    await this.conn.request(
      "reset_password",
      resetPasswordRequestToWire(id, newPassword),
      "password_reset",
    );
  }

  async getUserCapabilities(id: string): Promise<UserCapabilities> {
    const reply = await this.conn.request(
      "get_user_capabilities",
      getUserCapabilitiesRequestToWire(id),
      "user_capabilities",
    );
    return userCapabilitiesFromWire(reply);
  }

  async setUserCapabilities(
    id: string,
    grants: readonly Capability[],
    denies: readonly Capability[],
  ): Promise<UserCapabilities> {
    const reply = await this.conn.request(
      "set_user_capabilities",
      setUserCapabilitiesRequestToWire(id, grants, denies),
      "user_capabilities_set",
    );
    return userCapabilitiesFromWire(reply);
  }

  async getRoleCapabilities(role: UserRole): Promise<RoleCapabilities> {
    const reply = await this.conn.request(
      "get_role_capabilities",
      getRoleCapabilitiesRequestToWire(role),
      "role_capabilities",
    );
    return roleCapabilitiesFromWire(reply);
  }

  async setRoleCapabilities(
    role: UserRole,
    capabilities: readonly Capability[],
  ): Promise<RoleCapabilities> {
    const reply = await this.conn.request(
      "set_role_capabilities",
      setRoleCapabilitiesRequestToWire(role, capabilities),
      "role_capabilities_set",
    );
    return roleCapabilitiesFromWire(reply);
  }

  async listDesks(): Promise<DeskDesc[]> {
    const reply = await this.conn.request("list_desks", listDesksRequestToWire(), "desks");
    return listDesksResponseFromWire(reply);
  }

  async createDesk(name: string): Promise<DeskDesc> {
    const reply = await this.conn.request("create_desk", createDeskRequestToWire(name), "desk_created");
    return deskResponseFromWire(reply);
  }

  async updateDesk(id: string, name: string): Promise<DeskDesc> {
    const reply = await this.conn.request(
      "update_desk",
      updateDeskRequestToWire(id, name),
      "desk_updated",
    );
    return deskResponseFromWire(reply);
  }

  async deleteDesk(id: string): Promise<boolean> {
    const reply = await this.conn.request(
      "delete_desk",
      deleteDeskRequestToWire(id),
      "desk_deleted",
    );
    return reply["removed"] === true;
  }

  // --- legal-entity / netting-book registry (entity/book admin) --------------

  async listEntities(): Promise<EntityDesc[]> {
    const reply = await this.conn.request(
      "list_entities",
      listEntitiesRequestToWire(),
      "entities",
    );
    return entitiesResponseFromWire(reply);
  }

  async createEntity(input: EntityInput): Promise<EntityDesc> {
    const reply = await this.conn.request(
      "create_entity",
      createEntityRequestToWire(input),
      "entity_created",
    );
    return entityResponseFromWire(reply);
  }

  async updateEntity(key: number, input: EntityInput): Promise<EntityDesc> {
    const reply = await this.conn.request(
      "update_entity",
      updateEntityRequestToWire(key, input),
      "entity_updated",
    );
    return entityResponseFromWire(reply);
  }

  async deleteEntity(key: number): Promise<boolean> {
    const reply = await this.conn.request(
      "delete_entity",
      deleteEntityRequestToWire(key),
      "entity_deleted",
    );
    return reply["removed"] === true;
  }

  async listBooks(): Promise<BookDesc[]> {
    const reply = await this.conn.request("list_books", listBooksRequestToWire(), "books");
    return booksResponseFromWire(reply);
  }

  async createBook(input: BookInput): Promise<BookDesc> {
    const reply = await this.conn.request(
      "create_book",
      createBookRequestToWire(input),
      "book_created",
    );
    return bookResponseFromWire(reply);
  }

  async updateBook(key: number, input: BookInput): Promise<BookDesc> {
    const reply = await this.conn.request(
      "update_book",
      updateBookRequestToWire(key, input),
      "book_updated",
    );
    return bookResponseFromWire(reply);
  }

  async deleteBook(key: number): Promise<boolean> {
    const reply = await this.conn.request(
      "delete_book",
      deleteBookRequestToWire(key),
      "book_deleted",
    );
    return reply["removed"] === true;
  }

  // --- FI Aggregated Book (ADR-0022) admin CRUD ------------------------------

  async listAggregatedBooks(): Promise<AggregatedBookDesc[]> {
    const reply = await this.conn.request(
      "list_aggregated_books",
      listAggregatedBooksRequestToWire(),
      "aggregated_books",
    );
    return aggregatedBooksResponseFromWire(reply);
  }

  async createAggregatedBook(spec: AggregatedBookSpec): Promise<AggregatedBookDesc> {
    const reply = await this.conn.request(
      "create_aggregated_book",
      createAggregatedBookRequestToWire(spec),
      "aggregated_book_created",
    );
    return aggregatedBookResponseFromWire(reply);
  }

  async updateAggregatedBook(
    id: string,
    spec: AggregatedBookSpec,
  ): Promise<AggregatedBookDesc> {
    const reply = await this.conn.request(
      "update_aggregated_book",
      updateAggregatedBookRequestToWire(id, spec),
      "aggregated_book_updated",
    );
    return aggregatedBookResponseFromWire(reply);
  }

  async deleteAggregatedBook(id: string): Promise<boolean> {
    const reply = await this.conn.request(
      "delete_aggregated_book",
      deleteAggregatedBookRequestToWire(id),
      "aggregated_book_deleted",
    );
    return reply["removed"] === true;
  }

  // --- FI Pricing Groups (server commit 07fc99f) -----------------------------

  async listPricingGroups(): Promise<PricingGroup[]> {
    const reply = await this.conn.request(
      "list_pricing_groups",
      listPricingGroupsRequestToWire(),
      "pricing_groups",
    );
    return pricingGroupsResponseFromWire(reply);
  }

  async createPricingGroup(spec: PricingGroup): Promise<PricingGroup> {
    const reply = await this.conn.request(
      "create_pricing_group",
      createPricingGroupRequestToWire(spec),
      "pricing_group_created",
    );
    return pricingGroupResponseFromWire(reply);
  }

  async updatePricingGroup(id: string, spec: PricingGroup): Promise<PricingGroup> {
    const reply = await this.conn.request(
      "update_pricing_group",
      updatePricingGroupRequestToWire(id, spec),
      "pricing_group_updated",
    );
    return pricingGroupResponseFromWire(reply);
  }

  async deletePricingGroup(id: string): Promise<boolean> {
    const reply = await this.conn.request(
      "delete_pricing_group",
      deletePricingGroupRequestToWire(id),
      "pricing_group_deleted",
    );
    return reply["removed"] === true;
  }

  async updatePricingGroupPipeline(
    groupId: string,
    mode: PricingMode,
    pipeline: FeaturePipeline | null,
    sharePipeline: boolean,
  ): Promise<PricingGroup> {
    const reply = await this.conn.request(
      "update_pricing_group_pipeline",
      updatePricingGroupPipelineRequestToWire(
        groupId,
        pricingModeToWire(mode),
        pipeline,
        sharePipeline,
      ),
      "pricing_group_pipeline_updated",
    );
    return pricingGroupResponseFromWire(reply);
  }

  // --- FI Risk routing & risk books (server phases 4-5) ----------------------

  async listRiskBooks(): Promise<RiskBook[]> {
    const reply = await this.conn.request(
      "list_risk_books",
      listRiskBooksRequestToWire(),
      "risk_books",
    );
    return riskBooksResponseFromWire(reply);
  }

  async createRiskBook(spec: RiskBook): Promise<RiskBook> {
    const reply = await this.conn.request(
      "create_risk_book",
      createRiskBookRequestToWire(spec),
      "risk_book_created",
    );
    return riskBookResponseFromWire(reply);
  }

  async updateRiskBook(id: string, spec: RiskBook): Promise<RiskBook> {
    const reply = await this.conn.request(
      "update_risk_book",
      updateRiskBookRequestToWire(id, spec),
      "risk_book_updated",
    );
    return riskBookResponseFromWire(reply);
  }

  async deleteRiskBook(id: string): Promise<boolean> {
    const reply = await this.conn.request(
      "delete_risk_book",
      deleteRiskBookRequestToWire(id),
      "risk_book_deleted",
    );
    return reply["removed"] === true;
  }

  async getRiskRoutingGraph(): Promise<RiskRoutingGraph | null> {
    const reply = await this.conn.request(
      "get_risk_routing_graph",
      getRiskRoutingGraphRequestToWire(),
      "risk_routing_graph",
    );
    return riskRoutingGraphResponseFromWire(reply);
  }

  async updateRiskRoutingGraph(graph: RiskRoutingGraph): Promise<RiskRoutingGraph> {
    const reply = await this.conn.request(
      "update_risk_routing_graph",
      updateRiskRoutingGraphRequestToWire(graph),
      "risk_routing_graph_updated",
    );
    return updateRiskRoutingGraphResponseFromWire(reply);
  }

  async listRiskBookRisk(): Promise<RiskBookRisk[]> {
    const reply = await this.conn.request(
      "list_risk_book_risk",
      listRiskBookRiskRequestToWire(),
      "risk_book_risk",
    );
    return riskBookRiskResponseFromWire(reply);
  }

  subscribeRiskBookRisk(
    onSnapshot: (books: RiskBookRisk[], version: number) => void,
  ): () => void {
    // Open a dedicated multiplexed session for the live risk line (mirrors the
    // aggregated-book store): subscribe, route the baseline snapshot + version-gated
    // updates to the callback, and tear the whole line down on unsubscribe.
    const session = this.openStreamSession();
    let subId: bigint | null = null;
    const dispose = session.onEvent((event) => {
      if (event.kind === "riskBookRiskSnapshot") {
        if (subId !== null && event.snapshot.subscriptionId !== subId) return;
        onSnapshot(event.snapshot.books, event.snapshot.version);
      } else if (event.kind === "riskBookRiskUpdate") {
        if (subId !== null && event.update.subscriptionId !== subId) return;
        onSnapshot(event.update.books, event.update.version);
      }
    });
    subId = session.subscribeRiskBookRisk();
    return () => {
      dispose();
      if (subId !== null) session.unsubscribeRiskBookRisk(subId);
      session.close();
      subId = null;
    };
  }

  // --- instrument reference-data registry (instrument admin) -----------------

  async listInstruments(): Promise<InstrumentDef[]> {
    const reply = await this.conn.request(
      "list_instruments",
      listInstrumentsRequestToWire(),
      "instruments",
    );
    return instrumentsResponseFromWire(reply);
  }

  async getInstrument(id: string): Promise<InstrumentDef | null> {
    const reply = await this.conn.request(
      "get_instrument",
      getInstrumentRequestToWire(id),
      "instrument",
    );
    return instrumentResponseFromWire(reply);
  }

  async createInstrument(input: InstrumentInput): Promise<InstrumentDef> {
    const reply = await this.conn.request(
      "create_instrument",
      createInstrumentRequestToWire(input),
      "instrument_created",
    );
    return instrumentResponseFromWire(reply) ?? input;
  }

  async updateInstrument(input: InstrumentInput): Promise<InstrumentDef> {
    const reply = await this.conn.request(
      "update_instrument",
      updateInstrumentRequestToWire(input),
      "instrument_updated",
    );
    return instrumentResponseFromWire(reply) ?? input;
  }

  async deleteInstrument(id: string): Promise<boolean> {
    const reply = await this.conn.request(
      "delete_instrument",
      deleteInstrumentRequestToWire(id),
      "instrument_deleted",
    );
    return deleteInstrumentResponseFromWire(reply);
  }

  // --- curve bootstrap from registry-referenced instruments ------------------

  async buildCurve(request: BuildCurveRequest): Promise<CalibratedCurve> {
    const reply = await this.conn.request(
      "build_curve",
      buildCurveRequestToWire(request),
      "calibrated_curve",
    );
    return calibratedCurveFromWire(reply);
  }

  async getCurve(
    curveSet: RatesCurveSet | null,
    queryTenorYears: readonly number[],
    curveVersion?: bigint,
  ): Promise<GetCurveResult> {
    // The `get_curve` RPC over the WS mirror (SurfaceService, ADR-0021): read a
    // discount curve on the tenor axis, either bootstrapping the inline `curveSet`
    // live or reading a pinned `MarkCurve`d version. One unversioned contract, so
    // the read curve is byte-identical to the offline in-app bootstrap.
    const reply = await this.conn.request(
      "get_curve",
      getCurveRequestToWire(curveSet, queryTenorYears, curveVersion),
      "get_curve_response",
      PRICING_REQUEST_TIMEOUT_MS,
    );
    return getCurveResultFromWire(reply);
  }

  async markCurve(curveSet: RatesCurveSet): Promise<MarkedCurve> {
    // The `mark_curve` RPC over the WS mirror: bootstrap + persist the curve under a
    // fresh server-assigned version (the same version authority `mark_surface`
    // stamps), so a later `get_curve` pinned to it reproduces this exact curve.
    const reply = await this.conn.request(
      "mark_curve",
      markCurveRequestToWire(curveSet),
      "mark_curve_response",
      PRICING_REQUEST_TIMEOUT_MS,
    );
    return markedCurveFromWire(reply);
  }

  async curveScenario(
    curveSet: RatesCurveSet,
    parallelShiftBp: number,
    keyRateShiftBp: readonly number[],
    queryTenorYears: readonly number[],
    instrument?: RatesInstrument,
  ): Promise<CurveScenarioResult> {
    // The `curve_scenario` RPC over the WS mirror: parallel (+ optional per-pillar
    // key-rate) shift the par rates, re-bootstrap, return the shifted curve, and —
    // when an `instrument` rides along — reprice it on the base and shifted curves.
    // Scenario re-bootstraps through the same engine — pricing-class deadline.
    const reply = await this.conn.request(
      "curve_scenario",
      curveScenarioRequestToWire(
        curveSet,
        parallelShiftBp,
        keyRateShiftBp,
        queryTenorYears,
        instrument,
      ),
      "curve_scenario_response",
      PRICING_REQUEST_TIMEOUT_MS,
    );
    return curveScenarioResultFromWire(reply);
  }

  /** Permanently close the underlying connection (call on app teardown). */
  close(): void {
    this.conn.close();
  }
}

// ---------------------------------------------------------------------------
// small helpers used only by the transport surface
// ---------------------------------------------------------------------------

function asChild(o: WireObject, key: string): WireObject {
  const v = o[key];
  return v && typeof v === "object" ? (v as WireObject) : {};
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

/** Normalize a ws(s) URL to `scheme://host:port` for the status-ribbon label. */
function normalizeWsUrl(url: string): string {
  try {
    const u = new URL(url);
    const origin = `${u.protocol}//${u.host}`;
    return u.pathname && u.pathname !== "/" ? origin + u.pathname : origin;
  } catch {
    return url;
  }
}
