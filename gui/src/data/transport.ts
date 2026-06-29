/**
 * The transport seam. This is the ONLY interface the application talks to; it is
 * deliberately isolated so the live gRPC-Web/Connect or WebSocket JSON-mirror
 * client (against `celnet-server`) can be dropped in without touching any
 * workspace, component, or store. Today it is satisfied by the deterministic
 * in-app mock/replay source (src/data/mockSource.ts). One contract, two
 * transports (GUI-DESIGN §6.2) — both serialize the SAME `celnet-proto` types,
 * so the GUI cannot drift from the wire contract.
 *
 * The shapes here mirror the contract's RPCs (`celnet.proto` services):
 *   - PricingService.Price            → price()
 *   - QuoteService.RequestQuote/...   → requestQuote/acceptQuote/rejectQuote()
 *   - QuoteService.RequestMultiDealerQuote → requestMultiDealerQuote()
 *   - StreamService.StreamSession     → openStreamSession() (multiplexed)
 *   - SurfaceService.GetSmile/Mark/Scenario → getSmile/markSurface/scenario()
 */

import type {
  AcceptDeskQuoteRequest,
  AcceptDeskQuoteResponse,
  AggregateRatesRiskRequest,
  AggregateRatesRiskResponse,
  AggregateRiskRequest,
  AggregateRiskResponse,
  BookRatesPositionRequest,
  BookRatesPositionResponse,
  BrokerQuoteSet,
  Capability,
  CcyPair,
  Conventions,
  CreateUserInput,
  DeskDesc,
  EntityDesc,
  EntityInput,
  BookDesc,
  BookInput,
  UserCapabilities,
  RoleCapabilities,
  UserRole,
  DrillRiskRequest,
  DrillRiskResponse,
  Executed,
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
  MarketObservable,
  MarketSeriesPoint,
  MarketSeriesSnapshot,
  MultiDealerQuote,
  OisInstrument,
  Quote,
  RatesCurveSet,
  RatesPricingResult,
  RiskBucketRequest,
  ScenarioResult,
  ShockAxis,
  Smile,
  SmileModel,
  Snapshot,
  StreamReject,
  Tenor,
  TwoWayPrice,
  Update,
  UpdateUserInput,
  UserDesc,
} from "./contract";

/** A priced result for a single instrument (PricingService.Price). */
export interface PriceResult {
  greeks: Greeks;
  resolvedStrike: number;
  conventions: Conventions;
  twoWay: TwoWayPrice;
  surfaceVersion: bigint;
  /**
   * The Monte-Carlo standard error of the priced value (`PriceResponse
   * .price_std_error`). Set ONLY for an MC-priced product (a clamped cliquet),
   * `undefined` for every closed-form product.
   */
  priceStdError?: number;
}

/** Events the multiplexed stream session emits to the client (server→client). */
export type StreamEvent =
  | { kind: "snapshot"; snapshot: Snapshot }
  | { kind: "update"; update: Update }
  | { kind: "executed"; executed: Executed }
  | { kind: "reject"; reject: StreamReject }
  | { kind: "health"; subscriptionId: bigint; health: "HEALTHY" | "RESYNCING" | "STALE" }
  | { kind: "heartbeat"; heartbeat: Heartbeat }
  | { kind: "marketSeriesSnapshot"; snapshot: MarketSeriesSnapshot }
  | { kind: "marketSeriesPoint"; point: MarketSeriesPoint };

/** Parameters for opening a market-series subscription on the stream session. */
export interface MarketSeriesParams {
  pair: CcyPair;
  observable: MarketObservable;
  /** Required for tenor-dependent observables (ATM_VOL/RR/BF/FORWARD); absent for SPOT. */
  tenor?: Tenor;
  /** Required for the wing observables (RR/BF); absent otherwise. */
  delta?: number;
  /** Client throttle hint in nanos (0 = none). */
  throttleNanos?: bigint;
  /** Max history points in the opening snapshot (0 = server default). */
  historyLimit?: number;
}

/** A handle to a live multiplexed stream session (StreamService.StreamSession). */
export interface StreamSession {
  /** Open a subscription on an instrument; returns its client subscription id. */
  subscribe(instrument: Instrument, conventions: Conventions, label: string): bigint;
  /** Tear down a subscription. */
  unsubscribe(subscriptionId: bigint): void;
  /** Click-to-trade: present a stamped token to book exactly that streamed price. */
  execute(subscriptionId: bigint, token: bigint, idempotencyKey: string): void;
  /**
   * Open a market-series subscription (ATM-vol/spot/RR/BF/forward time-series),
   * multiplexed on the same session; returns its client subscription id. The
   * server replies with a `marketSeriesSnapshot` then `marketSeriesPoint`s.
   */
  subscribeMarketSeries(params: MarketSeriesParams): bigint;
  /** Tear down a market-series subscription. */
  unsubscribeMarketSeries(subscriptionId: bigint): void;
  /** Subscribe to server→client events; returns an unsubscribe disposer. */
  onEvent(listener: (event: StreamEvent) => void): () => void;
  /** Close the whole session. */
  close(): void;
}

/** The full client surface over the Celnet contract. */
export interface CelnetTransport {
  /** Human-readable transport label for the status ribbon (e.g. "mock/replay"). */
  readonly label: string;

  /**
   * Observe socket liveness — the listener fires `true` when the live connection
   * is OPEN and `false` when it drops (the transport then reconnects with capped
   * backoff). Returns a disposer. Present ONLY on transports with a remote socket
   * (the live WS transport); ABSENT on the in-app mock, which has no connection to
   * lose — a caller treats an absent observer as "permanently connected".
   */
  onConnectionState?(listener: (open: boolean) => void): () => void;

  /**
   * Snapshot of socket liveness at call time (`true` iff currently OPEN). Pairs
   * with {@link onConnectionState} for the initial state a subscriber would
   * otherwise miss (the observer only fires on transitions). Live transports only.
   */
  isConnected?(): boolean;

  /**
   * Install (or clear, with `null`) the bearer session token from
   * `AuthService.Login`. Once set, the transport authenticates every gated RPC
   * with it server-side (and admin RPCs are role-gated on it); cleared on logout.
   * Implemented by every transport so the auth flow is transport-agnostic.
   */
  setSessionToken(token: string | null): void;

  /** PricingService.Price */
  price(
    instrument: Instrument,
    market: MarketContext,
    conventions: Conventions,
  ): Promise<PriceResult>;

  /**
   * PricingService.PriceRates — price one linear-rates instrument (an OIS today)
   * against an explicit calibrated `RatesCurveSet`. The linear-rates analogue of
   * {@link price}: pure and market-explicit (the curve set IS the market), it
   * returns the direction-signed PV + first-order risk (PV01, DV01, key-rate
   * ladder). Satisfied identically by both transports — the offline source
   * bootstraps the curve and prices in-browser; the live transport issues the
   * `price_rates` RPC to celnet-server.
   */
  priceRates(
    curve: RatesCurveSet,
    instrument: OisInstrument,
  ): Promise<RatesPricingResult>;

  /** QuoteService.RequestQuote (idempotent on key). */
  requestQuote(
    instrument: Instrument,
    conventions: Conventions,
    idempotencyKey: string,
  ): Promise<Quote>;

  /**
   * QuoteService.RequestMultiDealerQuote — fan the RFQ across the edge's LP
   * panel and return the ranked lines (one row per responding dealer, the touch
   * winners named). Booking a row goes through `acceptQuote` with the row's
   * `lpId`. Honest boundary: in-repo dealers are the native maker plus
   * deterministic synthetic demo LPs; live bank LP connectivity is
   * environment-provisioned, never claimed in-repo.
   */
  requestMultiDealerQuote(
    instrument: Instrument,
    conventions: Conventions,
    idempotencyKey: string,
  ): Promise<MultiDealerQuote>;

  /**
   * QuoteService.AcceptQuote (books an execution on the chosen side). An absent/
   * empty `lpId` books the single-dealer quote (byte-identical to the pre-panel
   * wire frame); a panel row's `lpId` books exactly that pinned dealer line.
   */
  acceptQuote(
    quoteId: bigint,
    side: "BUY" | "SELL",
    idempotencyKey: string,
    lpId?: string,
  ): Promise<Execution>;

  /** QuoteService.RejectQuote (declines to trade). */
  rejectQuote(quoteId: bigint, reason: string): Promise<void>;

  /** StreamService.StreamSession — open the multiplexed RFS channel. */
  openStreamSession(): StreamSession;

  /** SurfaceService.GetSmile */
  getSmile(pair: CcyPair, tenorYears: number, conventions: Conventions): Promise<Smile>;

  /**
   * SurfaceService.MarkSurface — calibrate + publish a fresh `surface_version`.
   * `smileModel` selects the calibration family the server marks under (absent ⇒
   * the server default, `MARKET_HEDGE`); the model used is echoed in each smile's
   * `arbitrage.note` as `model=<family>` (the contract's provenance channel).
   */
  markSurface(
    pair: CcyPair,
    brokerQuotes: BrokerQuoteSet[],
    conventions: Conventions,
    smileModel?: SmileModel,
  ): Promise<MarkedSurface>;

  /**
   * SurfaceService.Scenario — reprice across the Cartesian product of the shock
   * axes. When `riskBuckets` is supplied the server additionally returns the
   * book-shaped risk decomposition (`bucketedRisk`); when omitted, `bucketedRisk`
   * is `null` (the server computes it only on request).
   */
  scenario(
    instrument: Instrument,
    baseMarket: MarketContext,
    conventions: Conventions,
    axes: ShockAxis[],
    riskBuckets?: RiskBucketRequest,
  ): Promise<ScenarioResult>;

  // --- RiskService — server-side hierarchical risk over the org cube ---------
  //
  // Aggregation is owned by the SERVER (API-first parity, CLAUDE.md rule 11): a
  // client never loops positions and sums. It lists positions, asks for a
  // rolled-up node tree over an org dimension, drills a node to its constituents,
  // and reads limit utilization — all behind this one contract.

  /** RiskService.ListPositions — the entitled open book, optionally scoped. */
  listPositions(request: ListPositionsRequest): Promise<ListPositionsResponse>;

  /**
   * RiskService.AggregateRisk — prune by principal BEFORE roll-up, group by the
   * org `dimension`, sum the additive measures + re-derive the non-additive ones
   * per node, and collapse everything into the reporting `numeraire`.
   */
  aggregateRisk(request: AggregateRiskRequest): Promise<AggregateRiskResponse>;

  /**
   * RiskService.AggregateRatesRisk — the linear-rates analogue of
   * {@link aggregateRisk}: price every `RatesPosition` against the request
   * `curveSet`, narrow by the optional `(entity, book, ccy)` scope, then sum
   * additively into one `RatesRiskNode` per settlement currency. Purely additive,
   * per-ccy partitioned, deterministic. Satisfied identically by both transports —
   * the offline source prices + folds in-browser; the live transport issues the
   * `aggregate_rates_risk` RPC to celnet-server.
   */
  aggregateRatesRisk(
    request: AggregateRatesRiskRequest,
    conventions: Conventions,
  ): Promise<AggregateRatesRiskResponse>;

  /**
   * RiskService.DrillRisk — drill one node into child sub-nodes at a finer
   * dimension and/or its contributing positions (the Book→Risk drill).
   */
  drillRisk(request: DrillRiskRequest): Promise<DrillRiskResponse>;

  // --- RfqDeskService — dealer-quoting RFQ/IOI desk --------------------------
  //
  // The desk lifecycle over the single contract: a counterparty SubmitDeskRequest
  // enqueues an inbound RFQ/IOI (PENDING); the desk RespondDeskRequest quotes or
  // rejects it; the counterparty AcceptDeskQuote lifts a quote, booking a Deal (+
  // a RatesPosition). ListDeskRequests/ListDeals read the inbox/blotter. Desk
  // requests price the SAME OisInstrument the `priceRates` seam prices.

  /** RfqDeskService.SubmitDeskRequest — inject an inbound RFQ/IOI (PENDING). */
  submitDeskRequest(request: SubmitDeskRequestRequest): Promise<SubmitDeskRequestResponse>;

  /** RfqDeskService.RespondDeskRequest — quote (→ QUOTED) or reject (→ REJECTED). */
  respondDeskRequest(request: RespondDeskRequestRequest): Promise<RespondDeskRequestResponse>;

  /** RfqDeskService.AcceptDeskQuote — lift a QUOTED request, booking a deal + position. */
  acceptDeskQuote(request: AcceptDeskQuoteRequest): Promise<AcceptDeskQuoteResponse>;

  /** RfqDeskService.ListDeskRequests — the desk inbox, optionally scoped. */
  listDeskRequests(request: ListDeskRequestsRequest): Promise<ListDeskRequestsResponse>;

  /** RfqDeskService.ListDeals — the received-deals blotter, optionally scoped. */
  listDeals(request: ListDealsRequest): Promise<ListDealsResponse>;

  // --- RiskService rates Book — book + list linear-rates positions -----------

  /** RiskService.BookRatesPosition — book one open rates position into the book. */
  bookRatesPosition(request: BookRatesPositionRequest): Promise<BookRatesPositionResponse>;

  /** RiskService.ListRatesPositions — the booked rates positions, optionally scoped. */
  listRatesPositions(request: ListRatesPositionsRequest): Promise<ListRatesPositionsResponse>;

  /**
   * NotificationService.StreamNotifications — open the dedicated server→client
   * push stream (RFQ/IOI received, accepted/rejected/expired). `onNotification`
   * fires for each pushed `Notification`; the returned disposer unsubscribes.
   * Re-opens transparently across a reconnect (live transport).
   */
  streamNotifications(
    scope: NotificationScope | undefined,
    onNotification: (notification: Notification) => void,
  ): () => void;

  /**
   * RiskService.LimitStatus — the limit tree + per-limit utilization/RAG for a
   * scope node, with the `hardBreach` escalation flag.
   */
  limitStatus(request: LimitStatusRequest): Promise<LimitStatusResponse>;

  // --- FixAdminService — manage the inbound FIX acceptor connections ---------
  //
  // The admin surface that defines / persists / lists / enables / deletes the
  // inbound FIX acceptors the edge binds (Options today; SPOT FX in phase 2).
  // Entitlement-gated server-side; the GUI asserts an explicit grant-all
  // principal (the same stance the risk calls take).

  /** FixAdminService.ListConnections — all managed connections + live status. */
  listFixConnections(): Promise<FixConnection[]>;

  /** FixAdminService.CreateConnection — define a new acceptor (binds if enabled). */
  createFixConnection(spec: FixConnectionSpec): Promise<FixConnection>;

  /** FixAdminService.UpdateConnection — replace a definition (restarts the acceptor). */
  updateFixConnection(id: string, spec: FixConnectionSpec): Promise<FixConnection>;

  /** FixAdminService.DeleteConnection — remove a connection (stops its acceptor). */
  deleteFixConnection(id: string): Promise<void>;

  /** FixAdminService.SetEnabled — enable/disable a connection (bind/stop its acceptor). */
  setFixConnectionEnabled(id: string, enabled: boolean): Promise<FixConnection>;

  /**
   * FixAdminService.ListMessages — poll the captured inbound/outbound session
   * traffic for the monitor screen. `afterSeq` is the cursor (`0n` ⇒ the whole
   * retained ring buffer); `connectionId` filters to one session; `limit` caps the
   * page (0 ⇒ server default). The response carries `latestSeq` to advance the cursor.
   */
  listFixMessages(
    connectionId: string | undefined,
    afterSeq: bigint,
    limit?: number,
  ): Promise<FixMessagePage>;

  // --- AuthService — server-enforced sessions + user/desk administration -----
  //
  // Login mints a bearer token the transport then installs via
  // {@link setSessionToken}; every gated RPC carries it and the SERVER resolves
  // the identity (admin RPCs are admin-gated on it). The admin calls below assume
  // an installed admin token — they encode no credential themselves.

  /** AuthService.Login — exchange email + password for a session + the user profile. */
  login(email: string, password: string): Promise<LoginResult>;

  /** AuthService.Logout — invalidate the installed bearer token; resolves to whether a live session ended. */
  logout(): Promise<boolean>;

  /** AuthService.ListUsers (admin) — the full user roster. */
  listUsers(): Promise<UserDesc[]>;

  /** AuthService.CreateUser (admin) — create a user; resolves to the created account. */
  createUser(input: CreateUserInput): Promise<UserDesc>;

  /** AuthService.UpdateUser (admin) — update a user's profile/role/desk/disabled flag. */
  updateUser(id: string, input: UpdateUserInput): Promise<UserDesc>;

  /** AuthService.DeleteUser (admin) — remove a user; resolves to whether one was removed. */
  deleteUser(id: string): Promise<boolean>;

  /** AuthService.ResetPassword (admin) — set a user's password (the seeded-admin rotation path). */
  resetPassword(id: string, newPassword: string): Promise<void>;

  /**
   * AuthService.GetUserCapabilities (admin) — read a user's capability overlay
   * (`grants`/`denies`) plus the server-resolved `effective` set (`role bundle ∪
   * grants ∖ denies`, deny-wins), enumerated over every action × asset.
   */
  getUserCapabilities(id: string): Promise<UserCapabilities>;

  /**
   * AuthService.SetUserCapabilities (admin) — replace the overlay wholesale (the
   * full new `grants`/`denies`, not a delta) and return the freshly-resolved set.
   * A successful set revokes the target user's live sessions server-side.
   */
  setUserCapabilities(
    id: string,
    grants: readonly Capability[],
    denies: readonly Capability[],
  ): Promise<UserCapabilities>;

  /**
   * AuthService.GetRoleCapabilities (admin) — read a role's capability bundle (its
   * base authority before any per-user overlay). `ADMIN` resolves to the full
   * grant-all surface; a non-admin role to its admin-editable bundle.
   */
  getRoleCapabilities(role: UserRole): Promise<RoleCapabilities>;

  /**
   * AuthService.SetRoleCapabilities (admin) — replace a non-admin role's bundle
   * wholesale (the full new set, not a delta) and return the freshly-stored bundle.
   * A successful set revokes the live sessions of every user holding the role. The
   * `ADMIN` role is grant-all and immutable — setting it is rejected server-side.
   */
  setRoleCapabilities(
    role: UserRole,
    capabilities: readonly Capability[],
  ): Promise<RoleCapabilities>;

  /** AuthService.ListDesks (admin) — the full desk roster. */
  listDesks(): Promise<DeskDesc[]>;

  /** AuthService.CreateDesk (admin) — create a desk; resolves to the created desk. */
  createDesk(name: string): Promise<DeskDesc>;

  /** AuthService.DeleteDesk (admin) — remove a desk (its members become unassigned). */
  deleteDesk(id: string): Promise<boolean>;

  // --- legal-entity / netting-book registry (entity/book admin) --------------
  //
  // ListEntities/ListBooks are callable by ANY authenticated user (they populate
  // the rates booking form's named dropdowns); Create/Update/Delete are admin-only
  // (server-enforced). DeleteEntity is rejected (FailedPrecondition) while any book
  // still references the entity.

  /** AuthService.ListEntities — the legal-entity registry (any authenticated user). */
  listEntities(): Promise<EntityDesc[]>;

  /** AuthService.CreateEntity (admin) — create an entity; resolves to the created entity. */
  createEntity(input: EntityInput): Promise<EntityDesc>;

  /** AuthService.UpdateEntity (admin) — rename / recode an entity (the key is immutable). */
  updateEntity(key: number, input: EntityInput): Promise<EntityDesc>;

  /** AuthService.DeleteEntity (admin) — remove an entity (rejected if a book references it). */
  deleteEntity(key: number): Promise<boolean>;

  /** AuthService.ListBooks — the netting-book registry (any authenticated user). */
  listBooks(): Promise<BookDesc[]>;

  /** AuthService.CreateBook (admin) — create a book under an entity. */
  createBook(input: BookInput): Promise<BookDesc>;

  /** AuthService.UpdateBook (admin) — rename / re-home a book (the key is immutable). */
  updateBook(key: number, input: BookInput): Promise<BookDesc>;

  /** AuthService.DeleteBook (admin) — remove a book. */
  deleteBook(key: number): Promise<boolean>;
}
