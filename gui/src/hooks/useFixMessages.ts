/**
 * Tails the captured FIX traffic for one managed acceptor session — the data
 * behind the monitor screen.
 *
 * It polls `FixAdminService.ListMessages` on a fixed cadence with a `seq` cursor:
 * the first poll (`afterSeq = 0`) pulls the retained ring buffer, and each
 * subsequent poll passes back the server's `latestSeq` so only new frames arrive.
 * New frames are appended to a bounded local buffer (newest-last) so the view can
 * follow a long-lived session at bounded memory. Polling pauses when no connection
 * is selected or when `paused` is set (so an operator can read without the tail
 * scrolling underneath them).
 */

import { useCallback, useEffect, useRef, useState } from "react";

import type { FixMessage } from "../data/contract";
import type { CelnetTransport } from "../data/transport";
import { appendCapped } from "../lib/fixMessageFilter";

/** The poll cadence (ms) — brisk enough to feel live, light on the edge. */
const POLL_MS = 1200;
/**
 * The local retained-frame cap (older frames drop off the top). Sized for
 * INVESTIGATION, not just a live tail: a few thousand frames give real history
 * to search over while staying bounded (~a few MB of small strings), so an admin
 * can pause and hunt back through the recent flow. Ring-buffered (newest-last)
 * so memory stays flat on a long-lived, high-rate session.
 */
export const FIX_BUFFER_CAP = 5000;

export interface FixMessagesApi {
  /** The retained captured frames for the selected session (oldest-first). */
  messages: FixMessage[];
  /** The last poll error as a display string, or `null`. */
  error: string | null;
  /** Clear the local buffer (does not affect the server ring). */
  clear: () => void;
}

export function useFixMessages(
  transport: CelnetTransport,
  connectionId: string | null,
  paused: boolean,
): FixMessagesApi {
  const [messages, setMessages] = useState<FixMessage[]>([]);
  const [error, setError] = useState<string | null>(null);
  const cursorRef = useRef<bigint>(0n);

  const clear = useCallback((): void => {
    setMessages([]);
    cursorRef.current = 0n;
  }, []);

  // Reset the cursor + buffer whenever the selected session changes.
  useEffect(() => {
    cursorRef.current = 0n;
    setMessages([]);
    setError(null);
  }, [connectionId]);

  useEffect(() => {
    if (connectionId === null || paused) return;
    let cancelled = false;

    const poll = async (): Promise<void> => {
      try {
        const page = await transport.listFixMessages(connectionId, cursorRef.current);
        if (cancelled) return;
        cursorRef.current = page.latestSeq;
        if (page.messages.length > 0) {
          setMessages((prev) => appendCapped(prev, page.messages, FIX_BUFFER_CAP));
        }
        setError(null);
      } catch (e: unknown) {
        if (!cancelled) setError(e instanceof Error ? e.message : "monitor poll failed");
      }
    };

    void poll();
    const id = setInterval(() => void poll(), POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [transport, connectionId, paused]);

  return { messages, error, clear };
}
