/**
 * useConnectionStatus — observes the live transport's socket liveness and drives
 * the reconnection window: when the backend connection drops (or never comes up),
 * it gives the transport's auto-reconnect a fixed window to recover, exposing a
 * live countdown for the blocking overlay, and latches to `failed` once the window
 * elapses so the app can fall back to the sign-in screen.
 *
 * The transport itself already reconnects with capped-exponential backoff (≈10
 * attempts inside the window); this hook only OBSERVES that liveness and times the
 * window — it never opens or closes a socket of its own.
 *
 * Transports without a remote (the in-app `?mock` source, which exposes no
 * `onConnectionState`) have no connection to lose, so the hook reports a permanent
 * `connected` and the overlay never appears.
 */

import { useEffect, useState } from "react";
import type { CelnetTransport } from "../data/transport";

/** How long the transport is given to reconnect before we drop to sign-in (ms). */
export const RECONNECT_WINDOW_MS = 30_000;

/**
 * Grace before the overlay is shown after a disconnect (ms). The overlay is shown
 * only when the socket is down *continuously* past this window; a transparent
 * transport reconnect (which re-opens the socket and re-subscribes on its own)
 * resets the timer the instant the socket is OPEN again, so a brief blip never
 * flashes the blocking modal.
 *
 * Sized for a real WAN reconnect, not a localhost one: over TLS + a reverse proxy
 * (and any fronting load balancer), a single drop→reconnect cycle — onclose,
 * backoff, fresh TCP+TLS+HTTP-upgrade — routinely takes 1–3s, which is longer than
 * a loopback reconnect. At the old 1.2s grace that transient cycle tripped the
 * modal, so the desk saw "connection lost" flash and immediately clear even though
 * the transport had already recovered. 4s covers a normal proxied reconnect (even
 * a periodic LB connection-lifetime cut) without a flash, while a genuine outage
 * still surfaces well inside the {@link RECONNECT_WINDOW_MS} hard-fail window.
 */
export const OVERLAY_GRACE_MS = 4_000;

/** Countdown refresh cadence while disconnected (ms). */
const TICK_MS = 250;

/** The phase the connection monitor is in. */
export type ConnectionPhase = "connected" | "reconnecting" | "failed";

export interface ConnectionStatus {
  /**
   * `connected` — socket OPEN (or no remote to monitor), and within the grace
   * window after a fresh drop. `reconnecting` — disconnected past the grace
   * window, still inside the reconnect window (show the overlay). `failed` —
   * the reconnect window elapsed while still down (drop to sign-in). Latches:
   * once `failed`, a later reconnect does not yank the user back.
   */
  readonly phase: ConnectionPhase;
  /** Whole seconds left in the reconnect window (only meaningful while reconnecting). */
  readonly remainingSeconds: number;
}

const FULL_WINDOW_SECONDS = Math.ceil(RECONNECT_WINDOW_MS / 1000);
const CONNECTED: ConnectionStatus = { phase: "connected", remainingSeconds: FULL_WINDOW_SECONDS };

/**
 * Observe `transport` connection liveness and report the reconnection-window phase.
 * `nowMs` defaults to `Date.now`; tests inject a deterministic clock alongside fake
 * timers.
 */
export function useConnectionStatus(
  transport: CelnetTransport,
  nowMs: () => number = Date.now,
): ConnectionStatus {
  const [status, setStatus] = useState<ConnectionStatus>(CONNECTED);

  useEffect(() => {
    // No remote socket to monitor (the in-app mock): permanently connected.
    if (!transport.onConnectionState) {
      setStatus(CONNECTED);
      return;
    }

    let open = transport.isConnected?.() ?? false;
    let downSince: number | null = open ? null : nowMs();
    let failed = false;

    const evaluate = (): void => {
      if (failed) return; // latched — never recover the session in place
      if (open) {
        downSince = null;
        setStatus(CONNECTED);
        return;
      }
      if (downSince === null) downSince = nowMs();
      const elapsed = nowMs() - downSince;
      const remaining = RECONNECT_WINDOW_MS - elapsed;
      if (remaining <= 0) {
        failed = true;
        setStatus({ phase: "failed", remainingSeconds: 0 });
        return;
      }
      if (elapsed < OVERLAY_GRACE_MS) {
        // Brief gap — keep the workspace visible, no overlay yet.
        setStatus(CONNECTED);
        return;
      }
      setStatus({ phase: "reconnecting", remainingSeconds: Math.ceil(remaining / 1000) });
    };

    const dispose = transport.onConnectionState((nowOpen) => {
      open = nowOpen;
      if (nowOpen) downSince = null;
      else if (downSince === null) downSince = nowMs();
      evaluate();
    });

    evaluate();
    const tick = setInterval(evaluate, TICK_MS);

    return () => {
      clearInterval(tick);
      dispose();
    };
  }, [transport, nowMs]);

  return status;
}
