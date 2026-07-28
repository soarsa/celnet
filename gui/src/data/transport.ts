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
  CombinedTailRiskRequest,
  CombinedTailRiskResponse,
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
  AggregatedBookDesc,
  AggregatedBookSpec,
  AggregatedBookStreamSnapshot,
  AggregatedBookStreamUpdate,
  InstrumentDef,
  InstrumentInput,
  BuildCurveRequest,
  CalibratedCurve,
  GetCurveResult,
  MarkedCurve,
  CurveScenarioResult,
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
  Quote,
  RatesCurveSet,
  RatesInstrument,
  RatesPricingResult,
  RatesQuote,
  RatesStreamSnapshot,
  RatesStreamUpdate,
  RiskBucketRequest,
  ScenarioResult,
  ShockAxis,
  Side,
  Smile,
  SmileModel,
  Snapshot,
  FeaturePipeline,
  PricingGroup,
  PricingMode,
  StreamReject,
  Tenor,
  TwoWayPrice,
  Update,
  UpdateUserInput,
  UserDesc,
  XvaPricingRequest,
  XvaResult,
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
  | { kind: "marketSeriesPoint"; point: MarketSeriesPoint }
  | { kind: "ratesSnapshot"; snapshot: RatesStreamSnapshot }
  | { kind: "ratesUpdate"; update: RatesStreamUpdate }
  | { kind: "aggregatedBookSnapshot"; snapshot: AggregatedBookStreamSnapshot }
  | { kind: "aggregatedBookUpdate"; update: AggregatedBookStreamUpdate };

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
  /**
   * Open a fixed-income (linear-rates) streaming line on the SAME multiplexed
   * session — a {@link RatesInstrument} priced against a baseline
   * {@link RatesCurveSet}. Returns its client subscription id. The server (or
   * offline mock) replies with a baseline `ratesSnapshot` then sequenced
   * `ratesUpdate`s as the curve deterministically ticks. The line is INDICATIVE
   * (PV + first-order risk); rates click-to-trade routes through RFQ/desk, so no
   * click-to-trade token rides this line and there is no `execute` for it.
   */
  subscribeRates(instrument: RatesInstrument, curveSet: RatesCurveSet, label: string): bigint;
  /** Tear down a fixed-income streaming line (the SAME id space as the FX lines). */
  unsubscribeRates(subscriptionId: bigint): void;
  /**
   * Open an aggregated-book composite line on the SAME multiplexed session — the
   * consolidated best bid/offer + per-member contribution report for the book
   * `bookId`. Returns its client subscription id. The server replies with a
   * baseline `aggregatedBookSnapshot` (sequence 1) then `aggregatedBookUpdate`
   * deltas as the book's members re-quote. `throttleNanos` is a client conflation
   * hint (0 = none). This is a READ line (any authenticated user); there is no
   * click-to-trade token on it.
   */
  subscribeAggregatedBook(bookId: string, throttleNanos?: bigint): bigint;
  /** Tear down an aggregated-book composite line (the SAME id space as the FX lines). */
  unsubscribeAggregatedBook(subscriptionId: bigint): void;
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
   * PricingService.PriceRates — price one linear-rates instrument (an OIS, vanilla
   * IRS, FRA, or cash bond — the `RatesInstrument` oneof) against an explicit
   * calibrated `RatesCurveSet`. The linear-rates analogue of {@link price}: pure and
   * market-explicit (the curve set IS the market), it returns the side-signed PV +
   * first-order risk (par/yield, PV01, DV01, key-rate ladder; the bond maps `pv =
   * dirty price`, `parRate = yield to maturity`, empty ladder). Satisfied identically
   * by both transports — the offline source bootstraps the curve and prices
   * in-browser; the live transport issues the `price_rates` RPC to celnet-server.
   */
  priceRates(
    curve: RatesCurveSet,
    instrument: RatesInstrument,
  ): Promise<RatesPricingResult>;

  /**
   * PricingService.PriceXva — price a netting set's all-in counterparty valuation
   * adjustments (CVA / DVA / FVA + their signed total) against the request's
   * exposure-model market, survival curves, LGDs and funding spread. Satisfied
   * identically by both transports — the offline source runs a deterministic
   * quadrature exposure model in-browser; the live transport issues the
   * `price_xva` RPC to celnet-server.
   *
   * Honest contract boundary: the wire result carries ONLY the four scalar
   * adjustments. The simulated exposure PROFILE (EPE/ENE per bucket) is a
   * server-internal of the estimator and is NOT on the contract, so no client can
   * render a LIVE exposure fan from this reply — the workspace draws an explicitly
   * illustrative, seeded profile for the fan and never presents it as a valuation.
   */
  priceXva(request: XvaPricingRequest): Promise<XvaResult>;

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
   * QuoteService.RequestRatesQuote — the fixed-income taker RFQ: request a
   * tradeable two-way on a linear-rates instrument (an OIS / vanilla IRS / FRA /
   * cash bond — the `RatesInstrument` oneof) priced against an explicit calibrated
   * `RatesCurveSet`. The FI analogue of {@link requestQuote}: a pure, market-
   * explicit price-discovery two-way (the curve set IS the market) struck around
   * the side-independent fair level (the par rate for an OIS/IRS/FRA, the clean
   * price for a cash bond) plus the full PV / PV01 / DV01 / key-rate risk. `side`
   * is the taker's directional intent (BUY = pay fixed / long, SELL = receive fixed
   * / short, TWO_WAY = no firm direction) and selects the sign of the returned
   * risk; the price is always the side-independent two-way. Satisfied identically
   * by both transports — the offline source prices in-browser and struts the same
   * two-way, the live transport issues the `request_rates_quote` RPC to
   * celnet-server. There is NO multi-dealer rates path (the panel wire is
   * FX-`Instrument` only), so this single two-way is the whole FI RFQ contract.
   */
  requestRatesQuote(
    curve: RatesCurveSet,
    instrument: RatesInstrument,
    notional: number,
    side: Side,
    idempotencyKey: string,
  ): Promise<RatesQuote>;

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
   * RiskService.CombinedTailRisk — the C2c "true single VaR engine": ONE
   * non-additive tail cube over a portfolio's vanilla FX option legs AND its
   * linear-FI (OIS-swap) legs, by full joint bump-and-revalue over aligned
   * (options-shock, rate-shock) scenarios, plus the FI signed key-rate DV01
   * ladder and signed parallel DV01. Pure of the edge — the whole portfolio +
   * scenario config travels inline — so both transports satisfy it identically:
   * the offline source reprices + reduces in-browser (mirroring
   * `celnet_risk_cube::fi::combined_tail_risk`); the live transport issues the
   * `combined_tail_risk` RPC to celnet-server. Options-only ⇒ the options VaR;
   * FI-only ⇒ the rate VaR; a mixed book shows the joint diversification.
   */
  combinedTailRisk(
    request: CombinedTailRiskRequest,
  ): Promise<CombinedTailRiskResponse>;

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

  /**
   * AuthService.UpdateDesk (admin) — rename a desk. `id` is the stable routing
   * key (immutable — RFQ/deal delivery, `User.deskIds` membership and connection
   * routing all key on it); only the display `name` changes. Rejects on an unknown id
   * (NotFound), a blank name (InvalidArgument), or a name that case-insensitively
   * collides with another desk (AlreadyExists). Resolves to the renamed desk.
   */
  updateDesk(id: string, name: string): Promise<DeskDesc>;

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

  // --- FI Aggregated Book (ADR-0022) admin CRUD ------------------------------
  //
  // The admin-defined composite books that consolidate N inbound liquidity
  // members into ONE best bid/offer per instrument. Listing is open to any
  // authenticated user (the composite is globally readable); create/update/delete
  // are admin-only (server-enforced). The live composite is read over
  // {@link StreamSession.subscribeAggregatedBook}.

  /** AuthService.ListAggregatedBooks — the full aggregated-book roster (any authenticated user). */
  listAggregatedBooks(): Promise<AggregatedBookDesc[]>;

  /** AuthService.CreateAggregatedBook (admin) — define a new book; resolves to the created book. */
  createAggregatedBook(spec: AggregatedBookSpec): Promise<AggregatedBookDesc>;

  /** AuthService.UpdateAggregatedBook (admin) — replace a book's definition (the `id` is immutable). */
  updateAggregatedBook(id: string, spec: AggregatedBookSpec): Promise<AggregatedBookDesc>;

  /** AuthService.DeleteAggregatedBook (admin) — remove a book; resolves to whether one was removed. */
  deleteAggregatedBook(id: string): Promise<boolean>;

  // --- FI Pricing Groups (server commit 07fc99f) -----------------------------
  //
  // Admin-defined pricing groups: many FIX connections / users / desks resolve to
  // ONE group, each carrying an ESP and an RFQ feature pipeline (RAW → ordered
  // features → OUTBOUND). Listing is any-authenticated; create/update/delete are
  // admin-only (server-enforced). `updatePricingGroupPipeline` retunes ONLY one
  // mode's pipeline block and is trader-accessible (gated server-side on
  // `quote_respond·fixed_income`), leaving the group's structure admin-only.

  /** ListPricingGroups — the full pricing-group roster (any authenticated user). */
  listPricingGroups(): Promise<PricingGroup[]>;

  /** CreatePricingGroup (admin) — define a new group; blank `id` ⇒ server mints from name. */
  createPricingGroup(spec: PricingGroup): Promise<PricingGroup>;

  /** UpdatePricingGroup (admin) — replace a group's definition (the `id` is immutable). */
  updatePricingGroup(id: string, spec: PricingGroup): Promise<PricingGroup>;

  /** DeletePricingGroup (admin) — remove a group; resolves to whether one was removed. */
  deletePricingGroup(id: string): Promise<boolean>;

  /**
   * UpdatePricingGroupPipeline — retune ONLY one mode's pipeline (structure stays
   * admin-only). `pipeline === null` ⇒ that mode falls back to the book default;
   * `sharePipeline` makes RFQ mirror ESP. Resolves to the retuned group.
   */
  updatePricingGroupPipeline(
    groupId: string,
    mode: PricingMode,
    pipeline: FeaturePipeline | null,
    sharePipeline: boolean,
  ): Promise<PricingGroup>;

  // --- instrument reference-data registry ------------------------------------

  /** AuthService.ListInstruments (any authenticated) — all instrument defs. */
  listInstruments(): Promise<InstrumentDef[]>;

  /** AuthService.GetInstrument (any authenticated) — one def, or null. */
  getInstrument(id: string): Promise<InstrumentDef | null>;

  /** AuthService.CreateInstrument (admin) — blank id ⇒ server mints from name. */
  createInstrument(input: InstrumentInput): Promise<InstrumentDef>;

  /** AuthService.UpdateInstrument (admin) — replace the identified def. */
  updateInstrument(input: InstrumentInput): Promise<InstrumentDef>;

  /** AuthService.DeleteInstrument (admin) — remove a def; true if it existed. */
  deleteInstrument(id: string): Promise<boolean>;

  // --- curve bootstrap from registry-referenced instruments ------------------

  /**
   * AuthService.BuildCurve (any authenticated) — bootstrap a discount curve from
   * registry-referenced instruments + their calibrating quotes. The server
   * resolves each pillar id against the reference-data registry and returns the
   * per-instrument calibrated points (short→long by resolved maturity).
   */
  buildCurve(request: BuildCurveRequest): Promise<CalibratedCurve>;

  // --- fixed-income curve query (SurfaceService, ADR-0021) -------------------
  //
  // The FI market-data query surface — the discount-curve analogue of the FX vol
  // surface's GetSmile / MarkSurface / Scenario. One asset-class-agnostic seam:
  // read a marked/bootstrapped curve on a tenor axis, pin it under a fresh version,
  // and bump-and-reprice it. The curve source is either an inline `curveSet`
  // (bootstrapped live) or a pinned marked `curveVersion` — exactly one.

  /**
   * SurfaceService.GetCurve — read a discount curve on the `queryTenorYears` axis.
   * Exactly one source: pass a `curveVersion` to read a `MarkCurve`d version (the
   * inline `curveSet` is ignored), else the inline `curveSet` is bootstrapped live
   * (pass `null` when pinning a version). The FI analogue of {@link getSmile}.
   */
  getCurve(
    curveSet: RatesCurveSet | null,
    queryTenorYears: readonly number[],
    curveVersion?: bigint,
  ): Promise<GetCurveResult>;

  /**
   * SurfaceService.MarkCurve — bootstrap + persist `curveSet` under a fresh pinned
   * `curveVersion`, so a later {@link getCurve} pinned to it reproduces this exact
   * curve. The FI analogue of {@link markSurface}.
   */
  markCurve(curveSet: RatesCurveSet): Promise<MarkedCurve>;

  /**
   * SurfaceService.CurveScenario — apply a parallel (and optional per-pillar
   * key-rate) shift to the calibrating par rates, re-bootstrap, and report the
   * shifted curve on `queryTenorYears`. When an `instrument` is supplied it is
   * repriced on the base and shifted curves (PV impact + base-curve DV01). The FI
   * analogue of the vol-surface {@link scenario}. `keyRateShiftBp` is parallel-only
   * when empty; otherwise its length must equal the pillar count.
   */
  curveScenario(
    curveSet: RatesCurveSet,
    parallelShiftBp: number,
    keyRateShiftBp: readonly number[],
    queryTenorYears: readonly number[],
    instrument?: RatesInstrument,
  ): Promise<CurveScenarioResult>;
}
