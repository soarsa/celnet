/**
 * useAggregatedBook — the live composite store for the FI Aggregated Book price
 * view (ADR-0022 D3). It opens its OWN multiplexed StreamSession over the shared
 * transport (the live WS transport multiplexes it on the same connection; the
 * mock ticks it independently), subscribes to ONE aggregated book by id, applies
 * the baseline `aggregatedBookSnapshot` then each `aggregatedBookUpdate`, and
 * exposes the current consolidated instruments the price grid renders. There is
 * no click-to-trade token on this line — it is a READ composite (any authenticated
 * user), so the store carries only market state.
 *
 * Updates are coalesced into ONE commit per animation frame (rAF batching),
 * matching the render-budget-as-latency-budget rule the price blotter uses, so a
 * fast re-quoting book never causes more than one paint per frame. The store is
 * keyed on `bookId`: selecting a different book tears down the old subscription
 * and opens the new one; unmount closes the session cleanly.
 */

import { useEffect, useRef, useState } from "react";

import type { AggregatedInstrument } from "../data/contract";
import type { CelnetTransport, StreamSession } from "../data/transport";

/** The live composite state of one subscribed aggregated book. */
export interface AggregatedBookState {
  /** The book id this state is for (echoes the requested `bookId`). */
  bookId: string;
  /** True once the baseline snapshot has landed (honest empty-state before it). */
  baselined: boolean;
  /** The latest applied per-subscription sequence (0 before the baseline). */
  sequence: bigint;
  /** The consolidated composite for each in-scope instrument (instrument-id order). */
  instruments: AggregatedInstrument[];
  /** The last-applied wall-clock time (nanoseconds since the Unix epoch), or 0. */
  epochNanos: bigint;
  /** How many sequence gaps were observed (a conflatable line still applies latest). */
  gaps: number;
}

const EMPTY_STATE = (bookId: string): AggregatedBookState => ({
  bookId,
  baselined: false,
  sequence: 0n,
  instruments: [],
  epochNanos: 0n,
  gaps: 0,
});

/**
 * Subscribe to the composite of the aggregated book `bookId` (or `null` to
 * subscribe to nothing). Returns the live composite state; `baselined` is false
 * until the first snapshot lands, so the caller renders an honest awaiting state
 * rather than a fabricated empty book.
 */
export function useAggregatedBook(
  transport: CelnetTransport,
  bookId: string | null,
): AggregatedBookState {
  const [state, setState] = useState<AggregatedBookState>(() =>
    EMPTY_STATE(bookId ?? ""),
  );

  // The authoritative state lives in a ref (mutated by the stream callback);
  // React state is committed once per frame from it.
  const store = useRef<AggregatedBookState>(EMPTY_STATE(bookId ?? ""));
  const dirty = useRef(false);
  const raf = useRef(0);
  // The subscription id this session opened (so stray frames from a torn-down
  // subscription are ignored).
  const subId = useRef<bigint | null>(null);

  useEffect(() => {
    if (bookId === null) {
      store.current = EMPTY_STATE("");
      setState(store.current);
      return;
    }

    store.current = EMPTY_STATE(bookId);
    setState(store.current);
    const session: StreamSession = transport.openStreamSession();

    const scheduleCommit = (): void => {
      if (dirty.current) return;
      dirty.current = true;
      raf.current = requestAnimationFrame(() => {
        dirty.current = false;
        // Publish a fresh object so React sees a new reference.
        setState({ ...store.current, instruments: store.current.instruments });
      });
    };

    const dispose = session.onEvent((event) => {
      switch (event.kind) {
        case "aggregatedBookSnapshot": {
          const s = event.snapshot;
          if (subId.current !== null && s.subscriptionId !== subId.current) break;
          // The baseline is authoritative: accept its sequence + instruments whole.
          store.current = {
            bookId: s.book.bookId || bookId,
            baselined: true,
            sequence: s.sequence,
            instruments: s.book.instruments,
            epochNanos: s.epochNanos,
            gaps: store.current.gaps,
          };
          scheduleCommit();
          break;
        }
        case "aggregatedBookUpdate": {
          const u = event.update;
          if (subId.current !== null && u.subscriptionId !== subId.current) break;
          // Ignore an update before the baseline (a conflatable line re-baselines).
          if (!store.current.baselined) break;
          const expected = store.current.sequence + 1n;
          const gap = u.sequence > expected ? 1 : 0;
          store.current = {
            ...store.current,
            sequence: u.sequence > store.current.sequence ? u.sequence : store.current.sequence,
            instruments: u.book.instruments,
            epochNanos: u.epochNanos,
            gaps: store.current.gaps + gap,
          };
          scheduleCommit();
          break;
        }
        default:
          break;
      }
    });

    subId.current = session.subscribeAggregatedBook(bookId);

    return () => {
      dispose();
      cancelAnimationFrame(raf.current);
      if (subId.current !== null) session.unsubscribeAggregatedBook(subId.current);
      session.close();
      subId.current = null;
    };
  }, [transport, bookId]);

  return state;
}
