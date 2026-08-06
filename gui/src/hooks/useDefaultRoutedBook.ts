/**
 * useDefaultRoutedBook — the React binding for the startup routing guard. Fetches
 * the firm-wide routing graph + risk-portfolio roster (existing RPCs — no new wire),
 * runs the pure {@link resolveDefaultRoutedBook} predicate, and exposes the result as
 * a small status machine the popup consumes.
 *
 * It RE-EVALUATES live so a warning retires the moment the firm gains a valid default:
 *   - on sign-in (and whenever the signed-in identity changes);
 *   - when any surface dispatches {@link RISK_ROUTING_CHANGED_EVENT} (the wizard Apply
 *     / routing-editor Save);
 *   - when the tab regains focus / visibility (catches an edit made on another surface
 *     or another client).
 *
 * `status: "unknown"` covers both "not signed in" and "the fetch failed / is in
 * flight" — the popup shows ONLY on `"warn"`, so a firm whose state we cannot
 * determine (e.g. an RPC the identity may not call) is never false-alarmed.
 */
import { useCallback, useEffect, useState } from "react";

import { useApp } from "../app/AppContext";
import type { CelnetTransport } from "../data/transport";
import {
  RISK_ROUTING_CHANGED_EVENT,
  resolveDefaultRoutedBook,
  type DefaultRouteResolution,
} from "../lib/routingGuard";

/** The guard status: still resolving/undeterminable, a valid default, or a warning. */
export type RoutingGuardStatus = "unknown" | "ok" | "warn";

export interface DefaultRoutedBookState {
  /** `"warn"` iff there is no valid default routed portfolio (⇒ show the popup). */
  status: RoutingGuardStatus;
  /** The full predicate output on a resolved fetch, else `null`. */
  resolution: DefaultRouteResolution | null;
  /** Force a re-fetch + re-evaluate (used by tests and manual retry). */
  refresh: () => void;
}

/** Fetch both inputs and resolve the predicate; throws propagate to the caller. */
async function evaluate(transport: CelnetTransport): Promise<DefaultRouteResolution> {
  const [graph, books] = await Promise.all([
    transport.getRiskRoutingGraph(),
    transport.listRiskBooks(),
  ]);
  return resolveDefaultRoutedBook(graph, books);
}

export function useDefaultRoutedBook(): DefaultRoutedBookState {
  const app = useApp();
  const { transport } = app;
  const signedIn = app.auth.signedIn;

  const [resolution, setResolution] = useState<DefaultRouteResolution | null>(null);
  const [status, setStatus] = useState<RoutingGuardStatus>("unknown");
  // A monotonically-bumped nonce that both re-runs the effect and lets a stale
  // in-flight fetch detect it was superseded (last-writer-wins, no race).
  const [nonce, setNonce] = useState(0);
  const refresh = useCallback(() => setNonce((n) => n + 1), []);

  useEffect(() => {
    if (!signedIn) {
      setStatus("unknown");
      setResolution(null);
      return;
    }
    let cancelled = false;
    void evaluate(transport)
      .then((r) => {
        if (cancelled) return;
        setResolution(r);
        setStatus(r.ok ? "ok" : "warn");
      })
      .catch(() => {
        if (cancelled) return;
        // Undeterminable (in-flight failure / not permitted) — never false-alarm.
        setResolution(null);
        setStatus("unknown");
      });
    return () => {
      cancelled = true;
    };
  }, [transport, signedIn, nonce]);

  // Live re-evaluation triggers: an explicit config-changed broadcast, plus the tab
  // regaining focus/visibility (an edit made elsewhere). All just bump the nonce.
  useEffect(() => {
    if (!signedIn) return;
    const onChanged = (): void => refresh();
    const onVisible = (): void => {
      if (document.visibilityState === "visible") refresh();
    };
    window.addEventListener(RISK_ROUTING_CHANGED_EVENT, onChanged);
    window.addEventListener("focus", onChanged);
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.removeEventListener(RISK_ROUTING_CHANGED_EVENT, onChanged);
      window.removeEventListener("focus", onChanged);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [signedIn, refresh]);

  return { status, resolution, refresh };
}
