/**
 * useStreamSession — the streaming RFS store (GUI-DESIGN §4.2, §6.3). Opens the
 * multiplexed StreamService.StreamSession via the transport seam, applies
 * snapshot+delta+resync, and coalesces all updates within a frame into ONE
 * commit (rAF batching) so a burst tape never causes >1 paint/frame — the
 * render budget IS a latency budget (principle 10). Per-row mid history feeds
 * the sparkline; per-row health badges are honest (seq/resync/stale).
 */

import { useEffect, useRef, useState } from "react";
import type {
  AttributionRecord,
  Conventions,
  Greeks,
  Heartbeat,
  Instrument,
  RatesCurveSet,
  RatesInstrument,
  RatesPricingResult,
  Snapshot,
  StreamHealth,
  TradableToken,
  TwoWayPrice,
} from "../data/contract";
import type { CelnetTransport, StreamSession } from "../data/transport";
import type { AssetClass } from "../products/types";
import { underlierAssetClass } from "../lib/assetUniverse";

/** A live row in the blotter, derived from the multiplexed stream. */
export interface StreamRow {
  subscriptionId: bigint;
  instrument: Instrument;
  /**
   * The streamed line's asset class, derived from the instrument's `underlying`
   * arm (FX when the legacy FX instrument carries no `underlying`). It relabels the
   * row's rate-rho Greeks to the class-correct carry identity (e.g. an equity's
   * "rho (dividend yield)") — the streamed edge of the carry seam reaching the
   * blotter. Pure projection of the instrument; never a fabricated class.
   */
  assetClass: AssetClass;
  label: string;
  conventions: Conventions;
  sequence: bigint;
  price: TwoWayPrice;
  greeks: Greeks;
  vol: number;
  resolvedStrike: number;
  tradable: TradableToken[];
  health: StreamHealth;
  /** Recent mids for the sparkline (bounded ring). */
  midHistory: number[];
  gaps: number;
  /**
   * Who quoted/holds this streamed line, when the server emits it. Engine-quoted
   * edge flow is the maker auto-pricer; `undefined` until/unless the contract
   * carries it on the wire (honest empty-state, never a fabricated seat).
   */
  attribution?: AttributionRecord;
}

/**
 * A live fixed-income (linear-rates) line in the Stream workspace — the FI
 * analogue of {@link StreamRow}, multiplexed on the SAME session. It carries the
 * streamed indicative PV + first-order risk (PV / par / PV01 / DV01) re-priced
 * each tick against the baseline curve shifted by `curveShift`. There is NO
 * two-way market and NO click-to-trade token: rates click-to-trade routes through
 * the RFQ/desk path, so this row is honestly indicative-only.
 */
export interface RatesStreamRow {
  subscriptionId: bigint;
  instrument: RatesInstrument;
  /** The subscribed baseline curve (its shift-0 price is the baseline snapshot). */
  curveSet: RatesCurveSet;
  /** The linear-rates arm ("ois" / "irs" / "fra" / "bond") for the row label. */
  kind: RatesInstrument["kind"];
  label: string;
  sequence: bigint;
  /** The streamed PV + par + PV01 / DV01 / key-rate ladder at this sequence. */
  result: RatesPricingResult;
  /** The parallel curve shift (decimal) this tick was priced against (0 baseline). */
  curveShift: number;
  /** Recent PVs for the sparkline (bounded ring). */
  pvHistory: number[];
  epochNanos: bigint;
}

/**
 * The latest server-reported observability, distilled from the most recent
 * [`Heartbeat`] across all live subscriptions. The StatusRibbon renders this so
 * the trader sees the SERVER's real numbers (drain-side price tail, ring
 * conflation drops, surface/correlation provenance), not just a client-side proxy.
 * `received === false` until the first heartbeat lands (honest empty-state — the
 * ribbon shows "—", never a fabricated zero presented as a measurement).
 */
export interface ServerObservability {
  received: boolean;
  /** Sum of `conflationDrops` across live subscriptions (total ticks the rings dropped). */
  conflationDrops: bigint;
  /** Worst (max) drain-side price p99 across live subscriptions, in ns. */
  serverPriceP99Nanos: bigint;
  /** The surface version the most-recently-beating subscription is pinned to (0 ⇒ live). */
  surfaceVersion?: bigint;
  /** The correlation id echoed by the most-recently-beating subscription (0 ⇒ none). */
  correlationId?: bigint;
}

/** A toast surfaced from click-to-trade Executed / StreamReject. */
export interface TradeToast {
  id: number;
  kind: "executed" | "reject";
  subscriptionId: bigint;
  text: string;
  epochNanos: bigint;
}

const HISTORY_LEN = 48;

export interface StreamApi {
  rows: StreamRow[];
  /** The live fixed-income (linear-rates) lines, multiplexed on the same session. */
  ratesRows: RatesStreamRow[];
  toasts: TradeToast[];
  subscribe: (instrument: Instrument, conventions: Conventions, label: string) => bigint;
  unsubscribe: (subscriptionId: bigint) => void;
  /**
   * Open a fixed-income streaming line (an OIS / IRS / FRA / bond priced against a
   * baseline curve). Returns its client subscription id. The line is INDICATIVE
   * (PV + first-order risk); there is no click-to-trade side (rates route to RFQ/
   * desk), so no `execute` counterpart exists for it.
   */
  subscribeRates: (
    instrument: RatesInstrument,
    curveSet: RatesCurveSet,
    label: string,
  ) => bigint;
  /** Tear down a fixed-income streaming line. */
  unsubscribeRates: (subscriptionId: bigint) => void;
  /**
   * Click-to-trade a side. Resolves the FRESHEST live token for that side from the
   * authoritative row map (not the React-committed row, which lags the tape by up
   * to one coalesced frame), so a click trades against the maker's current token
   * rather than a just-superseded one. Returns false if no live token is present.
   */
  execute: (subscriptionId: bigint, side: "BUY" | "SELL") => boolean;
  dismissToast: (id: number) => void;
  /** Aggregate health for the status ribbon. */
  totalSeq: bigint;
  lpCount: number;
  /** Latest server-reported observability, distilled from the live heartbeats. */
  observability: ServerObservability;
}

const EMPTY_OBSERVABILITY: ServerObservability = {
  received: false,
  conflationDrops: 0n,
  serverPriceP99Nanos: 0n,
};

export function useStreamSession(
  transport: CelnetTransport,
  seed: { instrument: Instrument; conventions: Conventions; label: string }[],
): StreamApi {
  const sessionRef = useRef<StreamSession | null>(null);
  // The authoritative row map lives in a ref (mutated by the stream callback);
  // React state is committed once per frame from it.
  const rowMap = useRef(new Map<bigint, StreamRow>());
  // Per-subscription metadata (label + the originating Instrument/Conventions),
  // keyed by SubscriptionId. The wire snapshot/update carry conventions but NOT
  // the Instrument or the human label, so we resolve them here. This map is a
  // ref — NOT a closure-captured array — so subscriptions added at RUNTIME
  // (e.g. "Stream this" from the Ticket) are visible to the long-lived onEvent
  // handler. The previous design closed over the initial seed array, so any
  // runtime subscription's snapshot resolved to `undefined` metadata and was
  // silently dropped (its row never appeared in the blotter).
  const metaRef = useRef(
    new Map<bigint, { label: string; instrument: Instrument; conventions: Conventions }>(),
  );
  // Snapshots that arrived before their metadata was recorded (the mock emits
  // the baseline snapshot synchronously inside subscribe(), i.e. before
  // subscribe() has returned the SubscriptionId we key metadata by). They are
  // materialized into rows the instant metadata lands. This makes both seed and
  // runtime subscriptions appear on frame 0 regardless of emission ordering.
  const orphanRef = useRef(new Map<bigint, Snapshot>());
  // Bridge into the effect closure so the subscribe/seed paths can materialize a
  // parked snapshot once they hold the SubscriptionId. Set inside the effect.
  const materializeOrphanRef = useRef<(id: bigint) => boolean>(() => false);
  const dirty = useRef(false);
  const raf = useRef(0);
  const toastSeq = useRef(0);
  // The latest heartbeat per subscription (the authoritative source for the
  // server observability the ribbon shows). Distilled into one ServerObservability
  // on the same rAF batch as the rows, so a beat burst never causes >1 paint/frame.
  const beatMap = useRef(new Map<bigint, Heartbeat>());
  // The authoritative fixed-income row map (same ref-then-rAF-commit discipline as
  // the FX rows). Per-subscription metadata (label + instrument + baseline curve)
  // the wire snapshot/update do NOT carry is resolved here, keyed by SubscriptionId.
  const ratesRowMap = useRef(new Map<bigint, RatesStreamRow>());
  const ratesMetaRef = useRef(
    new Map<bigint, { label: string; instrument: RatesInstrument; curveSet: RatesCurveSet }>(),
  );
  // Rates snapshots that arrived before their metadata was recorded (the mock
  // emits the baseline snapshot synchronously inside subscribeRates(), before it
  // has returned the id we key metadata by). Materialized once metadata lands.
  const ratesOrphanRef = useRef(
    new Map<
      bigint,
      { sequence: bigint; result: RatesPricingResult; curveShift: number; epochNanos: bigint }
    >(),
  );
  const materializeRatesOrphanRef = useRef<(id: bigint) => boolean>(() => false);

  const [rows, setRows] = useState<StreamRow[]>([]);
  const [ratesRows, setRatesRows] = useState<RatesStreamRow[]>([]);
  const [toasts, setToasts] = useState<TradeToast[]>([]);
  const [totalSeq, setTotalSeq] = useState<bigint>(0n);
  const [observability, setObservability] = useState<ServerObservability>(EMPTY_OBSERVABILITY);

  useEffect(() => {
    const session = transport.openStreamSession();
    sessionRef.current = session;
    const meta = metaRef.current;

    // Register the onEvent handler BEFORE issuing any subscribe() call: the
    // mock/replay session emits the baseline snapshot synchronously from
    // subscribe(), so the listener must already be live to receive it.
    const scheduleCommit = () => {
      if (dirty.current) return;
      dirty.current = true;
      raf.current = requestAnimationFrame(() => {
        dirty.current = false;
        setRows([...rowMap.current.values()]);
        setRatesRows([...ratesRowMap.current.values()]);
        let max = 0n;
        for (const r of rowMap.current.values()) if (r.sequence > max) max = r.sequence;
        setTotalSeq(max);
        setObservability(distillObservability(beatMap.current));
      });
    };

    const pushToast = (toast: Omit<TradeToast, "id">) => {
      const id = (toastSeq.current += 1);
      setToasts((ts) => [...ts, { ...toast, id }]);
      setTimeout(() => setToasts((ts) => ts.filter((t) => t.id !== id)), 5200);
    };

    // Build (or refresh) a blotter row from a snapshot + resolved metadata.
    const applySnapshot = (s: Snapshot, m: { label: string; instrument: Instrument }) => {
      const existing = rowMap.current.get(s.subscriptionId);
      const row: StreamRow = {
        subscriptionId: s.subscriptionId,
        instrument: m.instrument,
        // Class is a pure projection of the streamed instrument's underlying arm
        // (FX when the legacy FX instrument carries no `underlying`).
        assetClass: m.instrument.underlying
          ? underlierAssetClass(m.instrument.underlying)
          : "FX",
        label: m.label,
        conventions: s.conventions,
        sequence: s.sequence,
        price: s.price,
        greeks: s.greeks,
        vol: s.vol,
        resolvedStrike: s.resolvedStrike,
        tradable: s.tradable,
        health: "HEALTHY",
        midHistory: appendHistory(existing?.midHistory, mid(s.price)),
        gaps: existing?.gaps ?? 0,
      };
      // Carry attribution when the wire emits it (else keep any prior, else absent).
      const attribution = s.attribution ?? existing?.attribution;
      if (attribution !== undefined) row.attribution = attribution;
      rowMap.current.set(s.subscriptionId, row);
    };
    // Materialize any orphaned snapshot now that metadata for `id` is known.
    // Called from the subscribe path (the metadata-arrives-after-snapshot case).
    materializeOrphanRef.current = (id: bigint) => {
      const m = meta.get(id);
      const orphan = orphanRef.current.get(id);
      if (m && orphan) {
        applySnapshot(orphan, m);
        orphanRef.current.delete(id);
        scheduleCommit();
        return true;
      }
      return false;
    };

    // --- fixed-income rows (same discipline as the FX rows above) ------------
    const ratesMeta = ratesMetaRef.current;
    // Build (or refresh) a fixed-income row from a baseline snapshot. The wire
    // snapshot carries the priced result + shift + sequence; the display label +
    // instrument + baseline curve are resolved from the recorded metadata.
    const applyRatesSnapshot = (
      subscriptionId: bigint,
      sequence: bigint,
      result: RatesPricingResult,
      curveShift: number,
      epochNanos: bigint,
      m: { label: string; instrument: RatesInstrument; curveSet: RatesCurveSet },
    ): void => {
      const existing = ratesRowMap.current.get(subscriptionId);
      const row: RatesStreamRow = {
        subscriptionId,
        instrument: m.instrument,
        curveSet: m.curveSet,
        kind: m.instrument.kind,
        label: m.label,
        sequence,
        result,
        curveShift,
        pvHistory: appendHistory(existing?.pvHistory, result.pv),
        epochNanos,
      };
      ratesRowMap.current.set(subscriptionId, row);
    };
    // Materialize a parked rates snapshot now that metadata for `id` is known.
    materializeRatesOrphanRef.current = (id: bigint) => {
      const m = ratesMeta.get(id);
      const orphan = ratesOrphanRef.current.get(id);
      if (m && orphan) {
        applyRatesSnapshot(id, orphan.sequence, orphan.result, orphan.curveShift, orphan.epochNanos, m);
        ratesOrphanRef.current.delete(id);
        scheduleCommit();
        return true;
      }
      return false;
    };

    const dispose = session.onEvent((event) => {
      switch (event.kind) {
        case "snapshot": {
          const s = event.snapshot;
          const m = meta.get(s.subscriptionId);
          const existing = rowMap.current.get(s.subscriptionId);
          if (!m && !existing) {
            // Metadata not recorded yet (snapshot emitted synchronously inside
            // subscribe()). Park it; subscribe() will materialize it once it has
            // the id. Keep only the latest orphan per id.
            orphanRef.current.set(s.subscriptionId, s);
            break;
          }
          applySnapshot(s, {
            label: m?.label ?? existing?.label ?? "",
            instrument: (m?.instrument ?? existing?.instrument)!,
          });
          scheduleCommit();
          break;
        }
        case "update": {
          const u = event.update;
          const row = rowMap.current.get(u.subscriptionId);
          if (!row) break;
          const expected = row.sequence + 1n;
          const gap = u.sequence > expected ? 1 : 0;
          row.sequence = u.sequence;
          row.price = u.price;
          row.greeks = u.greeks;
          row.vol = u.vol;
          row.tradable = u.tradable;
          row.midHistory = appendHistory(row.midHistory, mid(u.price));
          row.gaps += gap;
          scheduleCommit();
          break;
        }
        case "ratesSnapshot": {
          const s = event.snapshot;
          const m = ratesMeta.get(s.subscriptionId);
          const existing = ratesRowMap.current.get(s.subscriptionId);
          if (!m && !existing) {
            // Metadata not recorded yet (snapshot emitted synchronously inside
            // subscribeRates()). Park it; subscribeRates() materializes it once
            // it holds the id. Keep only the latest orphan per id.
            ratesOrphanRef.current.set(s.subscriptionId, {
              sequence: s.sequence,
              result: s.result,
              curveShift: s.curveShift,
              epochNanos: s.epochNanos,
            });
            break;
          }
          const resolved = m ?? {
            label: existing!.label,
            instrument: existing!.instrument,
            curveSet: existing!.curveSet,
          };
          applyRatesSnapshot(
            s.subscriptionId,
            s.sequence,
            s.result,
            s.curveShift,
            s.epochNanos,
            resolved,
          );
          scheduleCommit();
          break;
        }
        case "ratesUpdate": {
          const u = event.update;
          const row = ratesRowMap.current.get(u.subscriptionId);
          if (!row) break;
          row.sequence = u.sequence;
          row.result = u.result;
          row.curveShift = u.curveShift;
          row.pvHistory = appendHistory(row.pvHistory, u.result.pv);
          row.epochNanos = u.epochNanos;
          scheduleCommit();
          break;
        }
        case "health": {
          const row = rowMap.current.get(event.subscriptionId);
          if (!row) break;
          row.health = event.health;
          scheduleCommit();
          break;
        }
        case "heartbeat": {
          // Record the latest beat for this subscription. We DON'T gate on a known
          // row: the beat is server observability in its own right (and arrives on
          // every subscription incl. ones whose first snapshot we may not yet hold).
          beatMap.current.set(event.heartbeat.subscriptionId, event.heartbeat);
          scheduleCommit();
          break;
        }
        case "executed": {
          const e = event.executed;
          // Append the maker attribution when the wire carries it (honest: omit
          // entirely when absent rather than inventing a counterparty).
          const maker = ownerLabel(e.attribution?.quotedBy);
          const makerSuffix = maker ? ` · vs ${maker}` : "";
          pushToast({
            kind: "executed",
            subscriptionId: e.subscriptionId,
            text: `Filled ${e.side === "BUY" ? "BUY" : "SELL"} @ ${e.tradedPremium.toFixed(3)} · exec #${e.executionId}${makerSuffix}`,
            epochNanos: e.epochNanos,
          });
          break;
        }
        case "reject": {
          const r = event.reject;
          pushToast({
            kind: "reject",
            subscriptionId: r.subscriptionId,
            text: `Rejected — ${rejectText(r.reason)}`,
            epochNanos: r.epochNanos,
          });
          break;
        }
      }
    });

    // Now that the listener is live, issue the seed subscriptions. The baseline
    // snapshot is emitted synchronously inside subscribe() — before we hold the
    // id — so it lands in the orphan buffer; we record metadata and materialize
    // it immediately. Seeds therefore render on frame 0.
    for (const s of seed) {
      const id = session.subscribe(s.instrument, s.conventions, s.label);
      meta.set(id, { label: s.label, instrument: s.instrument, conventions: s.conventions });
      materializeOrphanRef.current(id);
    }
    if (rowMap.current.size > 0) {
      setRows([...rowMap.current.values()]);
    }

    return () => {
      dispose();
      cancelAnimationFrame(raf.current);
      session.close();
      sessionRef.current = null;
      rowMap.current = new Map();
      ratesRowMap.current = new Map();
      ratesOrphanRef.current = new Map();
      beatMap.current = new Map();
      meta.clear();
      ratesMeta.clear();
    };
    // Seed + transport are stable for the app lifetime; intentional one-time wire.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [transport]);

  const subscribe = (instrument: Instrument, conventions: Conventions, label: string): bigint => {
    const session = sessionRef.current;
    if (!session) return 0n;
    // The session emits the baseline snapshot synchronously inside subscribe()
    // — before this call returns the id we key metadata by — so it lands in the
    // orphan buffer. We record metadata, then materialize the parked snapshot so
    // the new line appears in the blotter immediately. THIS is the runtime-
    // subscription path ("Stream this" from the Ticket); the prior closure over
    // the initial seed array meant these never resolved and never rendered.
    const id = session.subscribe(instrument, conventions, label);
    metaRef.current.set(id, { label, instrument, conventions });
    if (materializeOrphanRef.current(id)) {
      setRows([...rowMap.current.values()]);
    }
    return id;
  };
  const unsubscribe = (subscriptionId: bigint) => {
    rowMap.current.delete(subscriptionId);
    beatMap.current.delete(subscriptionId);
    metaRef.current.delete(subscriptionId);
    sessionRef.current?.unsubscribe(subscriptionId);
    setRows([...rowMap.current.values()]);
    setObservability(distillObservability(beatMap.current));
  };
  const subscribeRates = (
    instrument: RatesInstrument,
    curveSet: RatesCurveSet,
    label: string,
  ): bigint => {
    const session = sessionRef.current;
    if (!session) return 0n;
    // Same synchronous-baseline race as the FX path: the session emits the
    // baseline `ratesSnapshot` inside subscribeRates() before this returns the id,
    // so it lands in the rates orphan buffer. Record metadata, then materialize
    // the parked snapshot so the new fixed-income line appears immediately.
    const id = session.subscribeRates(instrument, curveSet, label);
    ratesMetaRef.current.set(id, { label, instrument, curveSet });
    if (materializeRatesOrphanRef.current(id)) {
      setRatesRows([...ratesRowMap.current.values()]);
    }
    return id;
  };
  const unsubscribeRates = (subscriptionId: bigint) => {
    ratesRowMap.current.delete(subscriptionId);
    ratesMetaRef.current.delete(subscriptionId);
    ratesOrphanRef.current.delete(subscriptionId);
    sessionRef.current?.unsubscribeRates(subscriptionId);
    setRatesRows([...ratesRowMap.current.values()]);
  };
  const execute = (subscriptionId: bigint, side: "BUY" | "SELL"): boolean => {
    const session = sessionRef.current;
    if (!session) return false;
    // Resolve the freshest token for the side straight from the live row map (the
    // ref the stream callback mutates every tick), so the click trades the maker's
    // current token, never a token from a frame the rAF batcher hasn't committed.
    const row = rowMap.current.get(subscriptionId);
    const tradable = row?.tradable.find((t) => t.side === side);
    if (!tradable) return false;
    session.execute(subscriptionId, tradable.token, `exec-${subscriptionId}-${tradable.token}`);
    return true;
  };
  const dismissToast = (id: number) => setToasts((ts) => ts.filter((t) => t.id !== id));

  return {
    rows,
    ratesRows,
    toasts,
    subscribe,
    unsubscribe,
    subscribeRates,
    unsubscribeRates,
    execute,
    dismissToast,
    totalSeq,
    lpCount: 3,
    observability,
  };
}

/**
 * Distil the per-subscription heartbeat map into one ServerObservability for the
 * ribbon. `conflationDrops` SUMS across subscriptions (each is a monotonic
 * per-ring skip counter — the firm-wide total dropped). `serverPriceP99Nanos`
 * takes the MAX (the worst tail any live line is paying — the honest headline).
 * The provenance echo is taken from the most recently-beating subscription.
 * Returns the empty sentinel (`received: false`) when no beat has landed.
 */
export function distillObservability(beats: Map<bigint, Heartbeat>): ServerObservability {
  if (beats.size === 0) return EMPTY_OBSERVABILITY;
  let drops = 0n;
  let worstP99 = 0n;
  let latest: Heartbeat | undefined;
  for (const beat of beats.values()) {
    drops += beat.conflationDrops;
    if (beat.serverPriceP99Nanos > worstP99) worstP99 = beat.serverPriceP99Nanos;
    if (!latest || beat.epochNanos > latest.epochNanos) latest = beat;
  }
  const out: ServerObservability = {
    received: true,
    conflationDrops: drops,
    serverPriceP99Nanos: worstP99,
  };
  if (latest?.surfaceVersion !== undefined) out.surfaceVersion = latest.surfaceVersion;
  if (latest?.correlationId !== undefined) out.correlationId = latest.correlationId;
  return out;
}

function mid(p: TwoWayPrice): number {
  return (p.bid + p.offer) / 2;
}

function appendHistory(prev: number[] | undefined, v: number): number[] {
  const next = prev ? prev.slice(-(HISTORY_LEN - 1)) : [];
  next.push(v);
  return next;
}

/**
 * A human label for a `BookId`'s owner seat, or undefined when absent. Used to
 * attribute a fill/line honestly — never invents a seat when the wire omits it.
 */
export function ownerLabel(book: AttributionRecord["quotedBy"]): string | undefined {
  const owner = book?.owner;
  if (!owner) return book?.book || undefined;
  return owner.kind === "trader" ? owner.trader : owner.autoPricer;
}

function rejectText(reason: "EXPIRED" | "UNKNOWN_TOKEN" | "ALREADY_CONSUMED"): string {
  switch (reason) {
    case "EXPIRED":
      return "last-look window expired";
    case "UNKNOWN_TOKEN":
      return "token no longer live";
    case "ALREADY_CONSUMED":
      return "already traded";
  }
}
