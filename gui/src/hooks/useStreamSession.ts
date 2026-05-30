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
  Conventions,
  Greeks,
  Instrument,
  Snapshot,
  StreamHealth,
  TradableToken,
  TwoWayPrice,
} from "../data/contract";
import type { CelnetTransport, StreamSession } from "../data/transport";

/** A live row in the blotter, derived from the multiplexed stream. */
export interface StreamRow {
  subscriptionId: bigint;
  instrument: Instrument;
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
  toasts: TradeToast[];
  subscribe: (instrument: Instrument, conventions: Conventions, label: string) => bigint;
  unsubscribe: (subscriptionId: bigint) => void;
  execute: (subscriptionId: bigint, token: bigint) => void;
  dismissToast: (id: number) => void;
  /** Aggregate health for the status ribbon. */
  totalSeq: bigint;
  lpCount: number;
}

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

  const [rows, setRows] = useState<StreamRow[]>([]);
  const [toasts, setToasts] = useState<TradeToast[]>([]);
  const [totalSeq, setTotalSeq] = useState<bigint>(0n);

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
        let max = 0n;
        for (const r of rowMap.current.values()) if (r.sequence > max) max = r.sequence;
        setTotalSeq(max);
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
      rowMap.current.set(s.subscriptionId, {
        subscriptionId: s.subscriptionId,
        instrument: m.instrument,
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
      });
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
        case "health": {
          const row = rowMap.current.get(event.subscriptionId);
          if (!row) break;
          row.health = event.health;
          scheduleCommit();
          break;
        }
        case "executed": {
          const e = event.executed;
          pushToast({
            kind: "executed",
            subscriptionId: e.subscriptionId,
            text: `Filled ${e.side === "BUY" ? "BUY" : "SELL"} @ ${e.tradedPremium.toFixed(3)} · exec #${e.executionId}`,
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
      meta.clear();
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
    metaRef.current.delete(subscriptionId);
    sessionRef.current?.unsubscribe(subscriptionId);
    setRows([...rowMap.current.values()]);
  };
  const execute = (subscriptionId: bigint, token: bigint) => {
    sessionRef.current?.execute(subscriptionId, token, `exec-${subscriptionId}-${token}`);
  };
  const dismissToast = (id: number) => setToasts((ts) => ts.filter((t) => t.id !== id));

  return {
    rows,
    toasts,
    subscribe,
    unsubscribe,
    execute,
    dismissToast,
    totalSeq,
    lpCount: 3,
  };
}

function mid(p: TwoWayPrice): number {
  return (p.bid + p.offer) / 2;
}

function appendHistory(prev: number[] | undefined, v: number): number[] {
  const next = prev ? prev.slice(-(HISTORY_LEN - 1)) : [];
  next.push(v);
  return next;
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
