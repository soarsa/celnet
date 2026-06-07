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
 *   - StreamService.StreamSession     → openStreamSession() (multiplexed)
 *   - SurfaceService.GetSmile/Mark/Scenario → getSmile/markSurface/scenario()
 */

import type {
  AggregateRiskRequest,
  AggregateRiskResponse,
  BrokerQuoteSet,
  CcyPair,
  Conventions,
  DrillRiskRequest,
  DrillRiskResponse,
  Executed,
  Execution,
  Greeks,
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
  Quote,
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

  /** PricingService.Price */
  price(
    instrument: Instrument,
    market: MarketContext,
    conventions: Conventions,
  ): Promise<PriceResult>;

  /** QuoteService.RequestQuote (idempotent on key). */
  requestQuote(
    instrument: Instrument,
    conventions: Conventions,
    idempotencyKey: string,
  ): Promise<Quote>;

  /** QuoteService.AcceptQuote (books an execution on the chosen side). */
  acceptQuote(quoteId: bigint, side: "BUY" | "SELL", idempotencyKey: string): Promise<Execution>;

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
   * RiskService.DrillRisk — drill one node into child sub-nodes at a finer
   * dimension and/or its contributing positions (the Book→Risk drill).
   */
  drillRisk(request: DrillRiskRequest): Promise<DrillRiskResponse>;

  /**
   * RiskService.LimitStatus — the limit tree + per-limit utilization/RAG for a
   * scope node, with the `hardBreach` escalation flag.
   */
  limitStatus(request: LimitStatusRequest): Promise<LimitStatusResponse>;
}
