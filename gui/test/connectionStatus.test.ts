/**
 * useConnectionStatus — the reconnection-window state machine that backs the
 * blocking overlay and the sign-in fallback.
 *
 * The properties under test are the lifecycle contract itself:
 *  - a transport with NO remote (the in-app mock: no `onConnectionState`) is
 *    reported permanently `connected` — the overlay never appears offline;
 *  - a live drop stays `connected` through the brief grace window (no modal flash
 *    on a sub-second cutover), then shows `reconnecting` with a whole-second
 *    countdown of the 30s window;
 *  - a reconnect inside the window returns to `connected` and cancels the count;
 *  - the window elapsing while still down latches `failed` (the App then drops to
 *    sign-in), and a late reconnect does NOT un-latch it.
 *
 * Timing is driven by vitest fake timers (which also fake `Date.now`, the hook's
 * default clock), so every assertion is deterministic — no wall-clock waits.
 */

import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { CelnetTransport } from "../src/data/transport";
import {
  OVERLAY_GRACE_MS,
  RECONNECT_WINDOW_MS,
  useConnectionStatus,
} from "../src/hooks/useConnectionStatus";

/** A minimal live-shaped transport whose socket state the test drives by hand. */
function makeLiveTransport(initiallyOpen: boolean): {
  transport: CelnetTransport;
  emit: (open: boolean) => void;
} {
  let open = initiallyOpen;
  const listeners = new Set<(open: boolean) => void>();
  const transport = {
    label: "live ws://test:8081",
    onConnectionState(listener: (open: boolean) => void): () => void {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    isConnected(): boolean {
      return open;
    },
  } as unknown as CelnetTransport;
  const emit = (next: boolean): void => {
    open = next;
    for (const l of listeners) l(next);
  };
  return { transport, emit };
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("useConnectionStatus", () => {
  it("reports a permanent connection for a transport with no remote (mock)", () => {
    const mock = { label: "mock/replay" } as unknown as CelnetTransport;
    const { result } = renderHook(() => useConnectionStatus(mock));
    expect(result.current.phase).toBe("connected");

    act(() => {
      vi.advanceTimersByTime(RECONNECT_WINDOW_MS * 2);
    });
    expect(result.current.phase).toBe("connected");
  });

  it("stays connected through the grace window, then shows reconnecting with a countdown", () => {
    const { transport } = makeLiveTransport(false);
    const { result } = renderHook(() => useConnectionStatus(transport));

    // Down from mount, but inside the grace window → no overlay yet.
    expect(result.current.phase).toBe("connected");
    act(() => {
      vi.advanceTimersByTime(OVERLAY_GRACE_MS - 250);
    });
    expect(result.current.phase).toBe("connected");

    // Past the grace window → reconnecting, counting down the remaining window.
    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(result.current.phase).toBe("reconnecting");
    expect(result.current.remainingSeconds).toBeGreaterThan(0);
    expect(result.current.remainingSeconds).toBeLessThanOrEqual(30);
  });

  it("returns to connected when the socket recovers inside the window", () => {
    const { transport, emit } = makeLiveTransport(false);
    const { result } = renderHook(() => useConnectionStatus(transport));

    act(() => {
      vi.advanceTimersByTime(OVERLAY_GRACE_MS + 500);
    });
    expect(result.current.phase).toBe("reconnecting");

    act(() => {
      emit(true);
    });
    expect(result.current.phase).toBe("connected");

    // And it does not later fail even past the original window.
    act(() => {
      vi.advanceTimersByTime(RECONNECT_WINDOW_MS);
    });
    expect(result.current.phase).toBe("connected");
  });

  it("latches failed once the reconnect window elapses while still down", () => {
    const { transport, emit } = makeLiveTransport(false);
    const { result } = renderHook(() => useConnectionStatus(transport));

    act(() => {
      vi.advanceTimersByTime(RECONNECT_WINDOW_MS + 1_000);
    });
    expect(result.current.phase).toBe("failed");
    expect(result.current.remainingSeconds).toBe(0);

    // A late reconnect must NOT un-latch the signed-out state.
    act(() => {
      emit(true);
    });
    expect(result.current.phase).toBe("failed");
  });

  it("restarts the window on a fresh drop after a recovery", () => {
    const { transport, emit } = makeLiveTransport(true);
    const { result } = renderHook(() => useConnectionStatus(transport));
    expect(result.current.phase).toBe("connected");

    // Drop now; the window is measured from THIS moment.
    act(() => {
      emit(false);
      vi.advanceTimersByTime(OVERLAY_GRACE_MS + 250);
    });
    expect(result.current.phase).toBe("reconnecting");

    act(() => {
      vi.advanceTimersByTime(RECONNECT_WINDOW_MS);
    });
    expect(result.current.phase).toBe("failed");
  });
});
